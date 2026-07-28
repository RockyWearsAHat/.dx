//! Platform commands: `doctor` and `install`.
//!
//! `dx install` is what makes `.dx` a format *any* agent can use rather than a trick that
//! works in one repository. It puts the `dx` binary on `PATH` and registers the MCP server
//! with every assistant it finds on the machine, so a fresh agent in an unfamiliar project
//! can read, render, and run documents without being told how.

use std::path::{Path, PathBuf};

use doc_run::toolchain::locate;

use crate::args::Args;
use crate::install::{self, Registration};

/// `dx doctor` — what is installed, what works, and what is missing.
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
            || "no — run `dx install`".to_string(),
            |path| path.display().to_string()
        )
    ));

    out.push_str("rendering\n");
    out.push_str(&format!(
        "  text      yes (built in)\n  images    {}\n\n",
        image_status()
    ));

    out.push_str("code execution\n");
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
        "present but not registered — run `dx install`".to_string()
    }
}

/// `dx install` — put `dx` on `PATH` and register it with every agent found.
pub fn run_install(args: &Args) -> Result<String, String> {
    if args.present("print") {
        return Ok(install::manual_instructions());
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

    out.push_str(&format!(
        "\n{registered} agent config(s) updated. Restart your assistant to pick up the change.\n\
         Any agent that can run shell commands can use `dx` directly — see `dx help`.\n"
    ));
    Ok(out)
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

    if target == source {
        out.push_str(&format!(
            "binary\n  already installed: {}\n",
            target.display()
        ));
        return Ok(target);
    }

    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    std::fs::copy(&source, &target)
        .map_err(|error| format!("could not install to {}: {error}", target.display()))?;
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
  dx png      <file> [--section ID] [--out F]   render to an image
  dx open     <file> [--section ID]             open the rendered page in a browser
  dx ls       [dir]                             every .dx document in a project
  dx search   <query> [dir]                     find documents by content

WRITE
  dx new      <file> [--title T]                create a document
  dx set      <file> <block-id> --text T        replace one block
  dx append   <file> --type T --text T          add a block at the end
  dx fmt      <file...> [--check]               rewrite in canonical form

RUN
  dx run      <file> [--only ID] [--force] [--dry] [--timeout S]
              Executes code blocks marked `run` and stores their output in the
              document. Nothing else in dx ever executes code.

PLATFORM
  dx mcp                                        serve documents to AI agents over MCP
  dx install  [--all] [--bin-dir D] [--print]   put dx on PATH and register with agents
  dx doctor                                     what is installed and what is missing
  dx help                                       this text

COMMON FLAGS
  --section ID   render only one block or heading section
  --theme T      auto (default), light, or dark
  --doc-css      apply the document's own ::style blocks
  --hidden       include blocks marked hidden
  --out FILE     write to a file instead of standard output ('-' means stdout)

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
        let text = run_install(&args(&["--print"])).expect("print");
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
            "dx install",
        ] {
            assert!(help.contains(command), "help omits {command}");
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
    fn the_binary_name_matches_the_platform() {
        let expected = if cfg!(windows) { "dx.exe" } else { "dx" };
        assert_eq!(binary_name(), expected);
    }
}
