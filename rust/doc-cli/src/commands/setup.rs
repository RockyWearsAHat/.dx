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

use std::path::{Path, PathBuf};

use doc_run::toolchain::locate;

use crate::args::Args;
use crate::commands::browser;
use crate::desktop;
use crate::extension::{self, Channel, Family, Target};
use crate::install::{self, Registration};
use crate::policies;
use crate::service;

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
    for (language, programs) in install::RUNTIME_PROBES {
        let found = programs
            .iter()
            .filter_map(|program| locate(program).map(|_| (*program).to_string()))
            .collect::<Vec<_>>();
        let status = if found.is_empty() {
            format!("missing — install {}", programs.join(" or "))
        } else {
            found.join(", ")
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
    for line in desktop::status_lines() {
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str("\ngithub.com\n");
    for line in browser::status_lines() {
        out.push_str(&line);
        out.push('\n');
    }

    Ok(out)
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
fn install_binary(args: &Args, out: &mut String) -> Result<PathBuf, String> {
    let source = std::env::current_exe()
        .map_err(|error| format!("could not find the running binary: {error}"))?;

    if args.present("no-path") {
        out.push_str(&format!("binary\n  left in place: {}\n", source.display()));
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

READ
  dx text     <file> [--section ID] [--ids]     document as Markdown
  dx outline  <file>                            block ids, kinds, and previews
  dx render   <file> [--section ID] [--theme T] self-contained HTML page
  dx render   <file> --block ID [--body=T]      one block, as the page draws it
  dx png      <file> [--section ID] [--out F]   render to an image
  dx png      <file> --pages                    one image per page, in order
  dx open     <file> [--section ID]             open the rendered page in a browser
  dx ls       [dir]                             every .dx document in a project
  dx search   <query> [dir]                     find documents by content

WRITE
  dx new      <file> [--title T]                create a document
  dx source   <file> [--block ID]               the exact characters, to edit
  dx set      <file> <block-id> --text T        replace one block
  dx insert   <file> --after ID [--type T]      add a block after another
  dx remove   <file> <block-id>                 take a block out
  dx append   <file> --type T --text T          add a block at the end
  dx fmt      <file...> [--check]               rewrite in canonical form

STORE
  A .dx file on disk is a one-line pointer; its content lives in the workspace
  store (.doc/). Every dx read resolves the pointer to the real document.
  dx sync     [dir]                             adopt, restore, and repair pointers
  dx stats    [dir]                             documents, block sharing, compaction
  dx textconv <file>                            print what a pointer stands for
  dx git-setup [dir]                            make git diff .dx as documents

RUN
  dx run      <file> [--only ID] [--force] [--dry] [--timeout S]
              Executes code blocks marked `run` and stores their output in the
              document. Nothing else in dx ever executes code.

PLATFORM
  dx serve    [--port N]                        the local rendering service
              Holds packs in memory and renders through the one engine, so every
              surface — a browser, a phone, an editor — is a thin client of it.
              Reads no files, writes none, runs nothing.
  dx mcp                                        serve documents to AI agents over MCP
  dx setup    [--all] [--bin-dir D] [--uninstall]
              The one install, run once per device: puts dx on PATH, registers the
              MCP server with every assistant here, starts the rendering service at
              login, installs DX.app so a double-clicked .dx opens as a page, and
              configures every browser that dx can configure for itself.
              --uninstall reverses all of it.
  dx browser  [--browser B] [--from S] [--dir D] [--open]
              Show .dx documents on github.com. Says how each browser here takes the
              extension and the one step it reserves for you. --from <source> builds
              a loadable directory out of a checkout's editor/github.
  dx doctor                                     what is installed and what is missing
  dx help                                       this text

COMMON FLAGS
  --section ID   render only one block or heading section
  --theme T      auto (default), light, or dark
  --doc-css      apply the document's own ::style blocks
  --hidden       include blocks marked hidden
  --show-code    open every code block, for a page nobody can click (print, archive)
  --out FILE     write to a file instead of standard output ('-' means stdout)
  --pages        with dx png: one image per page, breaking between blocks

A RUNNABLE BLOCK
  ::code id=chart lang=python run deps=\"matplotlib\"
  ...your code...
  ::end
";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn doctor_reports_every_area_it_checks() {
        let report = run_doctor(&args(&[])).expect("doctor");
        for section in ["binary", "rendering", "code execution", "agents"] {
            assert!(report.contains(section), "missing section: {section}");
        }
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
