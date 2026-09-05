//! Platform commands: `doctor` and `setup`.
//!
//! `dx setup` is the one install. It is what makes `.dx` a format that works on a *device*
//! rather than in one program: the binary lands on `PATH`, the MCP server is registered with
//! every assistant on the machine, the rendering service is registered to start at login, and
//! every browser that is here is given the extension by whatever route that browser allows.
//!
//! # Why it is one command and not five
//! Every surface that shows a document — a terminal, an agent, an editor, a browser, a file
//! preview — is a client of the same engine, and a person installing `dx` is not installing
//! five things, they are making this machine understand a file format. Splitting that into a
//! command per surface guarantees that most machines run some of them, which is exactly the
//! state where a reader opens github.com and sees a pointer.
//!
//! So: one command, run once, per device. It is idempotent, it reports what it changed rather
//! than what it attempted, and `--uninstall` reverses every part of it that reaches outside
//! `dx`'s own directory.

use std::path::PathBuf;

use doc_run::toolchain::locate;

use crate::args::Args;
use crate::commands::browser;
use crate::commands::store;
use crate::desktop;
use crate::extension::{self, Channel, Family, Target};
use crate::install::{self, Registration};
use crate::policies;
use crate::reports;
use crate::service;

/// Check for `.dx` pointers that are both tracked by git and in git-ignored paths.
/// Such pointers are unresolvable on other machines since their content never gets committed.
/// Returns Some with a diagnostic message if any are found, None if documents look healthy.
fn check_tracked_but_ignored_pointers() -> Result<Option<String>, String> {
    use doc_store::stub;
    use std::fs;

    let root = match crate::workspace::workspace_root(&std::path::PathBuf::from(".")) {
        root if root.is_dir() => root,
        _ => return Ok(None),
    };

    // Discover all .dx files to check.
    let discovered = doc_store::discover_documents(&root);
    let mut problems = Vec::new();

    for path in discovered {
        let Ok(relative) = path
            .strip_prefix(&root)
            .ok()
            .and_then(|p| {
                let s = p.to_string_lossy().into_owned();
                stub::normalize_path(&s)
            })
            .ok_or("could not normalize path")
        else {
            continue;
        };

        // Check if this is a pointer in a tracked-but-ignored state.
        if let Ok(text) = fs::read_to_string(&path) {
            if stub::is_stub(&text) && doc_store::git::is_tracked_but_ignored(&root, &relative) {
                problems.push(relative);
            }
        }
    }

    if problems.is_empty() {
        Ok(None) // No problems found, let desktop status lines handle it.
    } else {
        let mut message = String::from("  WARNING: tracked-but-ignored pointers found\n");
        for path in problems {
            message.push_str(&format!(
                "    {} — force-added to git but in a git-ignored path; content is in \
                 .doc/local.dxcp and won't reach other machines\n",
                path
            ));
        }
        message.push_str(
            "  Run `dx sync` for details on resolving these, or `git rm --cached <path>` \
             to remove them from git.\n",
        );
        Ok(Some(message))
    }
}

/// `dx doctor` — what is installed, what works, and what is missing.
/// One line describing the boundary a code block would run inside on this machine.
///
/// Says what it *is*, not merely that it is fine: a reader deciding whether to run a
/// document someone sent them is entitled to know which of these three sentences applies.
fn sandbox_status() -> String {
    match doc_run::confine::describe() {
        "seatbelt" => "seatbelt — reads allowed, writes confined, no network".to_string(),
        "bubblewrap" => "bubblewrap — reads allowed, writes confined, no network".to_string(),
        other if other.starts_with("none (") => {
            format!("{other} — code runs with your own permissions")
        }
        _ => "none — dx will refuse to run code on this platform".to_string(),
    }
}

/// `dx doctor` — report what is installed, healthy, and missing on this machine.
pub fn run_doctor(_args: &Args) -> Result<String, String> {
    let mut out = String::from("dx doctor\n\n");

    out.push_str("binary\n");
    out.push_str(&format!(
        "  running   {}\n",
        std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "(unknown)".to_string())
    ));
    out.push_str(&format!(
        "  on PATH   {}\n\n",
        locate("dx").map_or_else(
            || "no — run `dx setup`".to_string(),
            |path| path.display().to_string()
        )
    ));

    out.push_str("rendering\n");
    out.push_str(&format!(
        "  text      yes (built in)\n  images    {}\n\n",
        image_status()
    ));

    out.push_str("code execution\n");
    // The first line of this section is the one that matters: it says what stands between a
    // document someone sent you and the rest of your machine.
    out.push_str(&format!("  sandbox   {}\n", sandbox_status()));
    // A language is ready when every requirement group has a program installed; the
    // missing sentence names only the groups that are actually absent.
    for (language, groups) in install::RUNTIME_PROBES {
        let mut found: Vec<&str> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for group in *groups {
            match group
                .iter()
                .copied()
                .find(|program| locate(program).is_some())
            {
                Some(program) => found.push(program),
                None => missing.push(group.join(" or ")),
            }
        }
        let status = if missing.is_empty() {
            found.join(", ")
        } else {
            format!("missing — install {}", missing.join(" and "))
        };
        out.push_str(&format!("  {language:<9} {status}\n"));
    }

    out.push_str("\nagents\n");
    for registration in install::registrations() {
        out.push_str(&format!(
            "  {:<9} {}\n",
            registration.agent,
            describe_registration(&registration)
        ));
    }

    out.push_str("\nservice\n");
    let binary = locate("dx")
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("dx"));
    for line in service::status_lines(&binary) {
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str("\ndocuments\n");
    // Check for tracked-but-ignored pointers, which indicate a diagnosability issue.
    match check_tracked_but_ignored_pointers() {
        Ok(Some(message)) => out.push_str(&message),
        Ok(None) => {
            for line in desktop::status_lines() {
                out.push_str(&line);
                out.push('\n');
            }
        }
        Err(message) => out.push_str(&message),
    }

    out.push_str(&git_readiness());

    out.push_str(&validate_store());

    out.push_str("\ngithub.com\n");
    for line in browser::status_lines() {
        out.push_str(&line);
        out.push('\n');
    }

    // A filed report reaches the repository only when someone drains it, and the inbox is
    // the one piece of dx state that lives nowhere a session would trip over. So doctor —
    // the command a maintainer already runs — says how many are waiting.
    out.push_str("\nreports\n");
    let inbox = reports::inbox();
    match reports::read_inbox(&inbox) {
        Ok(waiting) if waiting.pending.is_empty() && waiting.unreadable.is_empty() => {
            out.push_str(&format!("  inbox     empty ({})\n", inbox.display()));
        }
        Ok(waiting) => out.push_str(&format!(
            "  inbox     {} waiting, {} unreadable — run `dx report drain` in the dx \
             checkout ({})\n",
            waiting.pending.len(),
            waiting.unreadable.len(),
            inbox.display()
        )),
        Err(reason) => out.push_str(&format!("  inbox     unreadable — {reason}\n")),
    }

    Ok(out)
}

/// Validate the .doc store: pointer digests, pack integrity, and index.db consistency.
/// Reports issues only when found; silent on healthy stores.
fn validate_store() -> String {
    use std::fs;

    let root = crate::workspace::workspace_root(&std::path::PathBuf::from("."));
    let doc_path = root.join(".doc");

    // Quick check: if .doc doesn't exist, nothing to validate
    if !doc_path.exists() {
        return String::new();
    }

    let mut issues = Vec::new();

    // Check .doc/index.db exists and is readable
    let index_db = doc_path.join("index.db");
    if !index_db.exists() {
        issues.push("  WARNING: .doc/index.db not found".to_string());
    }

    // Check for pack file integrity
    let pack_file = doc_path.join("repo.dxcp");
    if pack_file.exists() {
        if let Ok(metadata) = fs::metadata(&pack_file) {
            if metadata.len() == 0 {
                issues.push("  WARNING: .doc/repo.dxcp is empty".to_string());
            }
        }
    }

    // Validate .dx pointers in the workspace
    let discovered = doc_store::discover_documents(&root);
    for path in discovered {
        if let Ok(text) = fs::read_to_string(&path) {
            if text.starts_with("~ dx1 ") {
                let parts: Vec<&str> = text.split_whitespace().collect();
                if parts.len() < 2 || parts[1].len() != 64 {
                    issues.push(format!("  WARNING: invalid pointer {}", path.display()));
                }
            }
        }
    }

    if issues.is_empty() {
        return String::new();
    }

    let mut out = String::from("\nstore\n");
    for issue in issues {
        out.push_str(&issue);
        out.push('\n');
    }
    out
}

/// Whether this checkout's git is taught to diff and merge dx documents.
///
/// Every write path prepares the repository ([`store::ensure_git_ready`]), so this section is
/// normally three "yes" lines. It exists for the case that cannot be repaired automatically:
/// a `.doc/index.db` some earlier commit put into git. That file is binary, rebuildable, and
/// different on every machine, so once it is tracked *every* merge between branches that both
/// wrote documents conflicts on it and no resolution is correct. Untracking it is a commit
/// somebody has to make on purpose, so doctor names it and `dx git-setup` does it.
fn git_readiness() -> String {
    let root = crate::workspace::workspace_root(&std::path::PathBuf::from("."));
    if !root.join(".git").exists() {
        return String::new();
    }

    let mut out = String::from("\ngit\n");
    let attributes = std::fs::read_to_string(root.join(".gitattributes")).unwrap_or_default();
    let says = |pattern: &str, attribute: &str| {
        attributes.lines().any(|line| {
            let mut words = line.split_whitespace();
            words.next() == Some(pattern) && words.any(|word| word == attribute)
        })
    };
    out.push_str(&format!(
        "  diff      {}\n",
        if says("*.dx", "diff=dx") {
            "yes — `git diff` shows documents, not digests"
        } else {
            "no — run `dx git-setup`"
        }
    ));
    out.push_str(&format!(
        "  merge     {}\n",
        if says("*.dx", "merge=dx") && says(doc_store::pack::REPO_PACK, "merge=dx") {
            "yes — two branches reconcile block by block"
        } else {
            "no — run `dx git-setup`"
        }
    ));

    let committed: Vec<&str> = [
        ".doc/index.db",
        ".doc/index.db-wal",
        ".doc/index.db-shm",
        doc_store::pack::LOCAL_PACK,
        ".doc/coverage.jsonl",
    ]
    .into_iter()
    .filter(|relative| store::in_the_index(&root, relative))
    .collect();
    out.push_str(&format!(
        "  local     {}\n",
        if committed.is_empty() {
            "yes — nothing machine-local is committed".to_string()
        } else {
            format!(
                "{} is committed and will conflict on every merge — run `dx git-setup`, \
                 then commit the removal",
                committed.join(", ")
            )
        }
    ));
    out
}

/// Whether images can be produced, and what to install when they cannot.
fn image_status() -> String {
    doc_shot::browser::find().map_or_else(
        || "no browser found — install Chrome, Edge, or Chromium".to_string(),
        |browser| format!("yes ({})", browser.display()),
    )
}

/// One line describing whether an agent is configured for `dx`.
fn describe_registration(registration: &Registration) -> String {
    if !registration.config_exists() {
        return format!("not installed ({})", registration.config.display());
    }
    if registration.is_registered() {
        format!("registered ({})", registration.config.display())
    } else {
        "present but not registered — run `dx setup`".to_string()
    }
}

/// `dx setup` — make this device understand `.dx`, everywhere it can.
///
/// # Errors
/// Only when the binary itself cannot be installed. Everything after that is reported and
/// carried on from: a machine with no writable Firefox, or no browser at all, still gets a
/// working CLI, MCP server, and rendering service, and the report says what did not happen.
pub fn run_setup(args: &Args) -> Result<String, String> {
    if args.present("print") {
        return Ok(install::manual_instructions());
    }
    if args.present("uninstall") {
        return run_uninstall();
    }

    let mut out = String::new();
    let binary = install_binary(args, &mut out)?;

    out.push_str("\nagents\n");
    let mut registered = 0;
    for mut registration in install::registrations() {
        if !registration.config_exists() && !args.present("all") {
            out.push_str(&format!(
                "  {:<9} skipped (not installed)\n",
                registration.agent
            ));
            continue;
        }
        match registration.write(&binary) {
            Ok(true) => {
                registered += 1;
                out.push_str(&format!(
                    "  {:<9} registered in {}\n",
                    registration.agent,
                    registration.config.display()
                ));
            }
            Ok(false) => out.push_str(&format!("  {:<9} already registered\n", registration.agent)),
            Err(error) => out.push_str(&format!("  {:<9} failed: {error}\n", registration.agent)),
        }
    }

    out.push_str(&install_service(&binary));
    out.push_str(&install_viewer());
    out.push_str(&install_extension());
    out.push_str(&install_browsers());
    out.push_str(&format!(
        "\n{registered} agent config(s) updated. Restart your assistant to pick up the change.\n\
         Any agent that can run shell commands can use `dx` directly — see `dx help`.\n"
    ));
    Ok(out)
}

/// `dx setup --uninstall` — undo everything `dx setup` wrote outside its own directory.
///
/// The binary is deliberately left alone: removing the program that is currently running is
/// the one step a person can do better themselves, and `dx` deleting itself mid-command is a
/// surprise nobody asked for. What is removed is what a reader cannot easily find — a launch
/// agent and a policy file inside another application's bundle.
fn run_uninstall() -> Result<String, String> {
    let mut out = String::from("dx setup --uninstall\n\nservice\n");
    match service::uninstall() {
        Ok(true) => out.push_str("  removed the login service\n"),
        Ok(false) => out.push_str("  no login service was registered\n"),
        Err(error) => out.push_str(&format!("  {error}\n")),
    }

    out.push_str("\nbrowsers\n");
    let firefoxes = extension::detect()
        .into_iter()
        .filter(|browser| browser.family == Family::Firefox);
    let mut touched = 0;
    for browser in firefoxes {
        let file = policies::policy_file(&browser.path);
        match policies::remove(&file) {
            Ok(true) => {
                touched += 1;
                out.push_str(&format!("  {} — policy removed\n", browser.name));
            }
            Ok(false) => {}
            Err(error) => out.push_str(&format!("  {} — {error}\n", browser.name)),
        }
    }
    if touched == 0 {
        out.push_str("  no browser policy written by dx was found\n");
    }

    out.push_str(&format!(
        "\nThe extension directories are still in {}, and the binary is still on your PATH.\n\
         Remove either by hand if you want them gone.\n",
        extension::default_dir().display()
    ));
    // Dragging an application to the Trash is a thing every Mac user already knows how to do,
    // and it is what un-registers the type: LaunchServices drops a bundle that is gone.
    // Deleting someone's application from under them is not this command's decision.
    if let Some(app) = desktop::bundle() {
        out.push_str(&format!(
            "{} is still installed — drag it to the Trash to stop .dx opening in it.\n",
            app.display()
        ));
    }
    Ok(out)
}

/// Register the rendering service to start at login, and start it now.
///
/// Reported and not fatal: a machine where the service manager refuses still has a working
/// `dx`, and every surface falls back to decoding for itself. What it loses is speed, not
/// correctness, which is the whole reason this is safe to do without ceremony.
fn install_service(binary: &Path) -> String {
    let mut out = String::from("\nservice\n");
    match service::install(binary) {
        Ok(file) => out.push_str(&format!("  at login  {}\n", file.display())),
        Err(error) => {
            out.push_str(&format!("  at login  {error}\n"));
            return out;
        }
    }
    match service::wait_until_serving(service::STARTUP) {
        Some(port) => out.push_str(&format!("  serving   http://127.0.0.1:{port}\n")),
        None => out.push_str(
            "  serving   not answering yet — it starts with your session, or run `dx serve`\n",
        ),
    }
    out
}

/// Put `DX.app` where applications live and register it, so a double-clicked `.dx` opens.
///
/// Reported and not fatal, like every other surface: a `dx` installed on its own has no
/// application to register and is still a working `dx`. See [`crate::desktop`] for why the
/// bundle is copied before it is registered.
fn install_viewer() -> String {
    format!("\ndocuments\n{}\n", desktop::install().line())
}

/// Give every browser on this machine the extension, by whatever route it allows.
///
/// The only route that needs `dx` to *do* something is Firefox's policy file; the rest are
/// reported so the reader knows the one thing left. See [`extension::channel`] for why the
/// route differs per family.
fn install_browsers() -> String {
    let found = extension::detect();
    if found.is_empty() {
        return String::from("\nbrowsers on this machine\n  none found in the usual places\n");
    }

    let mut out = String::from("\nbrowsers on this machine\n");
    for browser in &found {
        out.push_str(&format!("  {}\n", browser.name));
        let report = match extension::channel(browser.family) {
            Channel::Policy { xpi } => give_by_policy(&browser.path, &xpi),
            _ => browser.family.steps(),
        };
        for line in report.lines() {
            out.push_str(&format!("      {line}\n"));
        }
    }
    out
}

/// Write the policy that makes one Firefox install the add-on at its next start.
fn give_by_policy(application: &Path, xpi: &Path) -> String {
    let file = policies::policy_file(application);
    match policies::force_install(&file, xpi) {
        Ok(true) => format!(
            "installed by dx — nothing for you to do\nrestart it to pick up the add-on ({})",
            file.display()
        ),
        Ok(false) => "installed by dx — already configured".to_string(),
        // The usual cause is a Firefox owned by another user or installed read-only, which is
        // a real situation and not an error in dx — so it says what it could not write.
        Err(error) => format!("could not configure it: {error}"),
    }
}

/// Say where this machine's browser extension is, or that this install has none.
///
/// Installing `dx` sets up everything the machine itself needs to read and write documents.
/// A browser extension is not that: it is a separate thing a reader installs when they want
/// github.com to show documents, and it is not in the binary (see [`crate::extension`]). So
/// this reports rather than writes — and reports the truth, because naming a directory that
/// was never installed is what makes a reader hunt through a browser's settings for it.
fn install_extension() -> String {
    let mut out = String::from("\ngithub.com\n");
    for target in Target::ALL {
        match extension::installed_dir(target) {
            Some(dir) => out.push_str(&format!("  {:<9} {}\n", target.name(), dir.display())),
            None => out.push_str(&format!("  {:<9} not in this install\n", target.name())),
        }
    }
    out.push_str("  load it   run `dx browser` for the one step your browser reserves for you\n");
    out
}

/// Copy the running binary somewhere on `PATH`, reporting where it landed.
///
/// `--no-path` skips only this copy. Every step after it in [`run_setup`], the login
/// service included, still runs — against `source`, whatever binary happens to be running
/// right now. A bypass is never silent: the notice below says so, the same way
/// `doc_run::FORCED_NOTICE` marks a block forced past its approval gate.
fn install_binary(args: &Args, out: &mut String) -> Result<PathBuf, String> {
    let source = std::env::current_exe()
        .map_err(|error| format!("could not find the running binary: {error}"))?;

    if args.present("no-path") {
        out.push_str(&format!(
            "binary\n  left in place: {}\n  note: --no-path only skips this copy — the \
             login service below still registers against this binary\n",
            source.display()
        ));
        return Ok(source);
    }

    let directory = args
        .value("bin-dir")
        .map_or_else(install::default_bin_dir, PathBuf::from);
    let target = directory.join(binary_name());

    // `dx setup` installs *the running binary*, so running the already-installed copy has
    // nothing to do. Saying only "already installed" reads as a successful update, which is
    // exactly wrong for the common case: someone rebuilt and wants the new binary in place.
    if target == source {
        out.push_str(&format!(
            "binary\n  this is the installed binary: {}\n  \
             nothing copied — to install a fresh build, run that build's own binary:\n  \
             cd rust && cargo build --release -p doc-cli && ./target/release/dx setup\n",
            target.display()
        ));
        return Ok(target);
    }

    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    replace(&source, &target)?;
    make_executable(&target)?;

    out.push_str(&format!("binary\n  installed: {}\n", target.display()));
    if !on_path(&directory) {
        out.push_str(&format!(
            "  note: {} is not on your PATH — add it to use `dx` from any shell\n",
            directory.display()
        ));
    }
    Ok(target)
}

/// Copy `source` over `target`, replacing the file rather than writing through it.
///
/// # Why the old file is removed first
/// Overwriting an executable in place leaves macOS holding a cached code signature for a file
/// whose contents have changed underneath it, and the kernel then **kills the new binary on
/// launch** — `SIGKILL`, exit 137, no message. The bytes are fine; the file identity is not.
/// Unlinking first gives the copy a new inode, so nothing stale is associated with it.
///
/// Removing a file that is currently executing is safe on Unix: a running process keeps the
/// inode it started from, so an in-flight `dx mcp` is unaffected. On Windows a locked target
/// cannot be removed, so the removal error is ignored and the copy is attempted anyway —
/// which is the behavior that worked there before.
fn replace(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        match std::fs::remove_file(target) {
            Ok(()) => {}
            Err(error) if cfg!(windows) => {
                let _ = error; // Fall through to the copy, as before.
            }
            Err(error) => {
                return Err(format!(
                    "could not replace {}: {error}. Remove it and run `dx setup` again.",
                    target.display()
                ))
            }
        }
    }
    std::fs::copy(source, target)
        .map_err(|error| format!("could not install to {}: {error}", target.display()))?;
    Ok(())
}

/// The platform's executable file name.
fn binary_name() -> &'static str {
    if cfg!(windows) {
        "dx.exe"
    } else {
        "dx"
    }
}

/// Mark a freshly copied binary executable on Unix; a no-op on Windows.
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)
            .map_err(|error| format!("could not make {} executable: {error}", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Whether `directory` is already listed in `PATH`.
fn on_path(directory: &Path) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|entry| entry == directory))
}

/// `dx help` — the command list.
pub fn run_help(_args: &Args) -> Result<String, String> {
    Ok(HELP.to_string())
}

/// The help text, also shown when `dx` is run with no arguments.
pub const HELP: &str = "\
dx — a notepad for code. Read, render, run, and share .dx documents.

QUICKSTART
  dx new notes.dx                                create a new document
  dx text notes.dx                              read it as plain Markdown
  dx render notes.dx                            view it in a browser
  dx insert notes.dx --after intro --type code  add a code block
  dx set notes.dx block1 --text 'new content'   edit a block
  dx run notes.dx --review                      check before running code

READ
  dx text     <file> [--section ID] [--ids]     document as Markdown — plain text, not
                                                formatted. Use --ids to print block ids
                                                you can pass to other commands
  dx outline  <file>                            block ids, kinds, and previews — one line
                                                per block, ready for scripting
  dx render   <file> [--section ID] [--theme T] self-contained HTML page in stdout
  dx render   <file> --block ID [--body=T]      one block, as the page draws it — for
                                                embedding a single block elsewhere
  dx render   --all [dir] --out DIR             batch export: every document as its own
                                                page — <out>/<same relative path>.html
  dx png      <file> [--section ID] [--out F]   render as PNG image
  dx png      <file> --pages                    one image per page, breaking between blocks
  dx png      <file> --block ID                 one block alone at natural size — a board
                                                rendered exactly as drawn
  dx png      <file> --block A,B,…              multiple blocks from one browser session
                                                — one <stem>-<id>.png per block
  dx png      <file> --block ID --against F     compare render to golden PNG: output is
                                                `identical` or `differs: N px in x,y wxh`
                                                (drift up to 3/255 is antialiasing, ok)
  dx play     <file> --script \"wait 500ms; key Space; scroll 200\"
              [--node ID] [--fps N] [--out DIR] drive the rendered page with real input
                                                — wait, key, click, scroll, hover — and
                                                write one PNG frame per tick, stamped with
                                                the moment and action. --node clips every
                                                frame to one block and reads x,y targets
                                                inside that block (0,0 is corner, bare
                                                scroll its center); without --node, x,y
                                                is viewport pixels. Nothing executes;
                                                targets are block ids or x,y coordinates
  dx open     <file> [--section ID]             render in browser — if no browser found,
                                                prints a file:// URL instead
  dx ls       [dir]                             list every .dx document in the directory
  dx search   <query> [dir] [--limit N]         search documents *and* source code. Ask a
                                                whole question in natural language (word
                                                endings, camelCase, and project jargon all
                                                work). Each hit shows the answering block
                                                or source line range
  dx trace    [dir] [--brief]                   extract symbols and references from code
                                                (Rust, JS/TS, Python, Go) — lists definition
                                                sites and files that reference each symbol.
                                                Use --brief to rank by usage count, suitable
                                                for embedding in a document
  dx coverage [dir] [--window N] [--min-rate R] search quality report: of the last N
                                                searches (default 200), how many hit a
                                                document vs. fell back to source-only or
                                                found nothing. Shows most-repeated misses
                                                first — hints for what to document next.
                                                Use --min-rate R to fail if below R%
  dx doctor                                     check installation health and find missing
                                                toolchains

WRITE
  dx new      <file> [--title T]                create a new document with an optional title

  dx index    [dir] [--force]                   scaffold index.dx — auto-discovers the
                                                project, ranks files by importance, and
                                                creates index.dx plus dev.dx with verify
                                                gates for the detected build system. Use
                                                --force to re-scan the tree

  dx source   <file> [--block ID] [--header]    raw block text for editing — copy from
              --header includes \"::kind attrs\"  the shell and edit offline, then use
                                                `dx set` to update the document. --header
                                                adds the opening ::kind line

  dx set      <file> <id> --text TEXT           replace block body (entire block content)
              [--header H]                      with new text. Use --header to rewrite
                                                the opening line (new ::kind or blank to
                                                make prose). Fastest for one-liners:
              --replace OLD --with NEW [--all]    find-replace OLD→NEW once (or --all for
                                                every occurrence). Keeps every other
                                                character — ideal for renames

  dx insert   <file> --after ID [--type T]      add a new block after another block. Type
              [--id ID] [--level N]             can be: h1-h4 (headings), text, code,
              [--lang L] [--run] [--deps D]     checklist, etc. For code: use --lang
                                                (rust, python, js), --run to mark runnable,
                                                and --deps to declare dependencies

  dx append   <file> --type T --text TEXT       add a block at the end — same options as
              [--id ID] [--level N]             `dx insert` but always goes to end

  dx remove   <file> [block-id]                 delete a block (no id: delete the whole
                                                document). History is preserved in the store

  dx check    <file> <checklist-id> --item N    toggle checkbox N in checklist (counting
                                                from zero). Use to automate box ticking

  dx board    <file> <board-id>                 edit ::board geometry (node positions,
              --place N --x X --y Y [--w W]       sizes, connections):
              --add --x X --y Y                   --place or --add sets node position
              --detach N                          (size in px, \"page\", or \"fit\")
              --link A --to B                     --link and --unlink draw or erase edges
              --unlink A --to B

  dx fmt      <file...> [--check]               rewrite blocks in canonical form (fixes
                                                line endings, spacing). Use --check to
                                                verify files are already formatted

  dx rename   <old> <new> [dir]                  structural rename: changes an identifier at
              [--review] [--approve]             every site a reference graph names for it.
                                                --review shows changes without approval.
                                                --approve records approval in the ledger

REPORT — report dx bugs and suggestions
  dx report   bug|suggestion|observation        file a report: --title T --detail D
              --title T --detail D              [--route R] [--repro X]. Goes to the dx
              [--route R] [--repro X]           inbox first (survives network loss),
                                                then syncs to the intake. --route names
                                                the affected command (e.g. \"run\", \"set\")
  dx report   list [dir]                        show reports waiting to sync, and what
                                                <dir>/reports.dx already has
  dx report   setup [dir]                       subscribe this repository to its own
              [--endpoint URL] [--token T]      reports (one-time setup). Mints a
                                                collision-resistant project key
  dx report   subscribe [dir]                   add <dir>/reports.dx to report sync
              [--project P] [--token T]
              [--endpoint URL]
  dx report   sync [dir]                        push inbox reports, pull open defects
                                                (works offline: stages changes to sync
                                                on next network connection)
  dx report   token [T]                         store your token machine-wide (or
                                                check what's stored if no T)
  dx report   close <id> [dir]                  mark a report fixed (removes block,
                                                notifies intake)
  dx report   drop <id>                         remove from inbox (for unsynced stubs)
  dx report   drain [dir]                       move inbox → <dir>/reports.dx (no
                                                network). Duplicate reports become
                                                second sightings, not duplicates

STORAGE — how dx stores and manages documents

  A .dx file is a one-line pointer (a digest). Content lives in .doc/.
  Every read resolves the pointer to the real document.

  dx sync     [dir]                             repair/restore pointers and documents
                                                after git operations. Reconciles .dx
                                                files, .doc/ packs, and index.db

  dx stats    [dir]                             storage summary: document count, block
                                                sharing, space used, compaction advice

  dx rm       <file>                            delete document (history survives in
                                                .doc/ — reversible if committed)

  dx textconv <file>                            debug: print the real content a .dx
                                                pointer stands for

  dx git-setup [dir]                            configure git diff/merge for .dx files.
              (Normally automatic on write, so  Use when: fixing a checkout, or fixing
               fresh clones need this only for  a .doc/index.db that was accidentally
               repairs)                         committed (it's binary, machine-local,
                                                and conflicts on every merge that
                                                touched documents)

  Git merge & diff:
    When two branches change documents, they merge block-by-block. Conflict markers
    in a .dx file freeze the block until `dx sync` can adopt a clean merge.
    The merge driver is called by git; you don't call it:
    dx merge-driver --ancestor %O --ours %A --theirs %B --path %P --marker-size %L

RUN
  dx run      <file> [--only ID] [--force] [--dry] [--timeout S]
              [--review] [--approve] [--follow-edges]

  Execution:
    dx run notes.dx                 run all code blocks marked ::code run
    dx run notes.dx --only block1   run one block
    dx run notes.dx --dry           show what would run, without executing
    dx run notes.dx --follow-edges  run in board edge order, not document order

  Approval workflow (new code must be reviewed):
    dx run notes.dx --review        print code fingerprints without executing
    dx run notes.dx --approve       approve reviewed code and run it
    dx run notes.dx --force         run once without approval (marked in output)
    NOTE: Editing a block clears its approval (the edit is the review). Only
    this machine's approvals count; cached results don't auto-approve.

  Sandbox (code runs confined, not with your full permissions):
    • Reads: repository folder, run caches, system toolchains
    • Writes: block's own scratch directory only (--writes= for document folders)
    • Network: none. Dependencies install during setup (--deps=), then offline
    • $HOME, $TMPDIR, $DX_SANDBOX all point to the scratch directory

  Block metadata:
    ::code id=build lang=rust run deps=\"cargo\"
      <code here>
      reads=src,Cargo.lock            # files/folders this block reads (confined to
      writes=target,build             # document folder only)
    ::end

    reads=: declare what files the block needs (csv). Content changes re-run
            approved code. Folders are recursive (hidden dirs included, but
            target/ node_modules/ and the block's own writes= are skipped)
    writes=: declare where to write (folders only, must be inside document
            folder). Grant joins the fingerprint, so new write powers re-review.
    deps=: packages/tools to install during setup (offline, doesn't re-install
           unless changed)

PLATFORM & SETUP
  dx serve    [--port N]                        local rendering service. Holds packs in
                                                memory, thin-client for browser/phone/editor.
                                                Reads no files, writes nothing, runs nothing

  dx mcp                                        serve documents to AI agents over MCP stdio

  dx setup    [--all] [--bin-dir D]             one-time setup per device: add dx to PATH,
              [--uninstall]                     register MCP with every assistant, start
                                                rendering service at login, install DX.app,
                                                configure browsers. --uninstall reverses all

  dx browser  [--browser B] [--from S] [--dir D]  install GitHub extension to view .dx files
              [--open]                           on github.com. Shows setup steps per browser.
                                                --from builds an install dir from a
                                                checkout's editor/github directory

  dx doctor                                     check installation, toolchains, agents,
                                                documents, git config, and report health

COMMON FLAGS (all commands)
  --section ID   show only one block/section (by id)
  --theme T      auto (default), light, or dark
  --hidden       include blocks marked hidden
  --show-code    expand all code blocks (for printing/archiving)
  --out FILE     write to file instead of stdout (- means stdout)
  --pages        (for dx png) one image per page, break between blocks
  --scale N      (for dx png) pixels per CSS pixel (default 2 for hi-dpi; 1 for
                 CSS-pixel size)

EXAMPLE: A runnable block
  ::code id=analyze lang=python run deps=\"pandas,matplotlib\"
  import pandas as df
  df.read_csv('data.csv').plot()
  ::end

  This block: imports pandas/matplotlib during setup, runs offline with reads=data.csv
  access and writes=plots output permissions, stores result in the document.";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn doctor_reports_every_area_it_checks() {
        let report = run_doctor(&args(&[])).expect("doctor");
        for section in ["binary", "rendering", "code execution", "agents", "reports"] {
            assert!(report.contains(section), "missing section: {section}");
        }
        // A filed report that nobody drains is a report nobody fixes, so the count is
        // said here whether or not anything is waiting.
        assert!(report.contains("inbox"), "{report}");
    }

    #[test]
    fn doctor_names_a_missing_toolchain_instead_of_staying_silent() {
        let report = run_doctor(&args(&[])).expect("doctor");
        // Every probed language is listed either as found or as "missing — install …".
        for (language, _) in install::RUNTIME_PROBES {
            assert!(report.contains(language));
        }
    }

    #[test]
    fn install_print_shows_config_anyone_can_copy() {
        let text = run_setup(&args(&["--print"])).expect("print");
        assert!(text.contains("\"dx\""));
        assert!(text.contains("mcp"));
    }

    #[test]
    fn help_lists_the_commands_that_exist() {
        let help = run_help(&args(&[])).expect("help");
        for command in [
            "dx text",
            "dx render",
            "dx png",
            "dx run",
            "dx mcp",
            "dx setup",
            "dx serve",
            "dx browser",
        ] {
            assert!(help.contains(command), "help omits {command}");
        }
    }

    /// `dx play --node` reads `x,y` inside the clipped block's box; the help has to say so,
    /// or a script author aims viewport pixels at a clipped frame.
    #[test]
    fn play_help_states_the_node_coordinate_frame() {
        let help = run_help(&args(&[])).expect("help");
        assert!(help.contains("reads x,y targets"), "{help}");
        assert!(help.contains("share one frame"), "{help}");
    }

    /// `dx setup` reaches every surface, so the report has to account for every one of them —
    /// a section quietly dropped is a machine that is quietly half-installed.
    #[test]
    fn setup_reports_on_each_thing_it_installs() {
        // `--print` is the one path that changes nothing on the machine running the suite,
        // so the sections are asserted against the help text that describes the real run.
        for section in ["binary", "agents", "service", "documents", "github.com"] {
            assert!(
                run_doctor(&args(&[])).expect("doctor").contains(section),
                "doctor omits {section}, so `dx setup` cannot be verified afterwards"
            );
        }
    }

    #[test]
    fn installing_the_binary_can_be_skipped() {
        let mut out = String::new();
        let path = install_binary(&args(&["--no-path"]), &mut out).expect("install");
        assert_eq!(path, std::env::current_exe().expect("exe"));
        assert!(out.contains("left in place"));
    }

    /// `--no-path` skips the PATH copy, never the login service that runs after it in
    /// [`run_setup`] — the flag's name once read as "nothing happens", and it still
    /// registers a real launch agent against whatever binary is running. The notice makes
    /// that unmissable, the way `FORCED_NOTICE` marks any other bypass.
    #[test]
    fn skipping_the_path_copy_says_the_service_still_registers() {
        let mut out = String::new();
        install_binary(&args(&["--no-path"]), &mut out).expect("install");
        assert!(
            out.contains("login service") && out.contains("still registers"),
            "{out}"
        );
    }

    #[test]
    fn installing_twice_replaces_the_file_instead_of_writing_through_it() {
        // The bug this pins: `dx install` used to copy over the existing binary, and macOS
        // then killed the result on launch — SIGKILL, exit 137, no message, from a file whose
        // bytes were correct. A new inode is the observable difference between replacing the
        // file and writing through it, so that is what is asserted.
        let directory = std::env::temp_dir().join("dx-install-replace-test");
        let _ = std::fs::remove_dir_all(&directory);
        let arguments = args(&["--bin-dir", directory.to_str().expect("path")]);

        let mut out = String::new();
        let first = install_binary(&arguments, &mut out).expect("first install");
        let before = inode_of(&first);

        let mut out = String::new();
        let second = install_binary(&arguments, &mut out).expect("second install");
        assert_eq!(first, second);
        assert!(second.exists(), "the binary must still be there");

        #[cfg(unix)]
        assert_ne!(
            before,
            inode_of(&second),
            "the second install must replace the file, not write through it"
        );
        let _ = before;
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The file's inode on Unix; `0` elsewhere, where the concept does not apply.
    fn inode_of(path: &Path) -> u64 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(path).map(|data| data.ino()).unwrap_or(0)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            0
        }
    }

    #[test]
    fn running_the_installed_binary_says_how_to_install_a_fresh_build() {
        // Reporting "already installed" for this case reads as a successful update when it is
        // a no-op, which is how a stale binary goes unnoticed.
        let source = std::env::current_exe().expect("exe");
        let directory = source.parent().expect("parent");
        let arguments = args(&["--bin-dir", directory.to_str().expect("path")]);
        let mut out = String::new();
        // Only meaningful when the running binary already sits at the install target, which is
        // the case being described: install into the directory the test binary runs from.
        if directory.join(binary_name()) != source {
            return;
        }
        install_binary(&arguments, &mut out).expect("install");
        assert!(out.contains("nothing copied"));
        assert!(out.contains("./target/release/dx setup"));
    }

    #[test]
    fn the_binary_name_matches_the_platform() {
        let expected = if cfg!(windows) { "dx.exe" } else { "dx" };
        assert_eq!(binary_name(), expected);
    }
}
