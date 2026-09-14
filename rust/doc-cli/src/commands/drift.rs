//! `dx drift [dir] [--session ID]` — which sessions in a project worked through dx.
//!
//! With a Claude Code hook event on stdin this is the hook entry: it records the tool use
//! against the project the call was made in and prints the nudge when one is due
//! ([`crate::drift`] is the authority). At a terminal it prints the project's sessions, most
//! recent first, with the off-method ones marked — the per-project view that says which
//! agents to tune the method against.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use crate::args::Args;
use crate::drift;

/// `dx drift [dir] [--session ID]` — record a hook event from stdin, or report a project.
///
/// Never fails on hook input: a hook that failed would block the tool call it wraps.
pub fn run(args: &Args) -> Result<String, String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() || args.positional(0).is_some() || args.value("session").is_some() {
        return Ok(report(args));
    }
    let mut input = String::new();
    let _ = stdin.lock().read_to_string(&mut input);
    if input.trim().is_empty() {
        return Ok(report(args));
    }
    drift::record(&input);
    Ok(String::new())
}

/// The project report, narrowed to one session when `--session` names it.
fn report(args: &Args) -> String {
    let directory = args
        .positional(0)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match args.value("session") {
        None => drift::summary(&directory),
        Some(wanted) => match drift::report(&directory)
            .into_iter()
            .find(|s| s.id == wanted || s.id.starts_with(wanted))
        {
            Some(s) => format!(
                "drift: {} raw commands, {} dx calls ({}% raw){} — session {}\n",
                s.raw,
                s.dx,
                s.raw_percent(),
                if s.off_method() { ", OFF METHOD" } else { "" },
                s.id
            ),
            None => format!(
                "drift: no session {wanted} recorded in {}\n",
                directory.display()
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::setup::run_help;

    #[test]
    fn help_text_describes_the_command() {
        let help = run_help(&crate::args::Args::parse(&[])).expect("help");
        assert!(help.contains("dx drift"), "{help}");
    }
}
