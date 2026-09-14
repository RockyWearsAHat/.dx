//! `dx drift` — work outside the harness, counted where it happens.
//!
//! With a Claude Code hook event on stdin this is the hook entry: it records the tool use and
//! prints the nudge when one is due ([`crate::drift`] is the authority). At a terminal, or
//! with `--session ID`, it prints the one-line summary a person reads.

use std::io::{IsTerminal, Read};

use crate::args::Args;
use crate::drift;

/// `dx drift [--session ID]` — record a hook event from stdin, or summarize a session.
///
/// Never fails on hook input: a hook that failed would block the tool call it wraps.
pub fn run(args: &Args) -> Result<String, String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() || args.value("session").is_some() {
        return Ok(drift::summary(args.value("session")));
    }
    let mut input = String::new();
    let _ = stdin.lock().read_to_string(&mut input);
    if input.trim().is_empty() {
        return Ok(drift::summary(None));
    }
    drift::record(&input);
    Ok(String::new())
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
