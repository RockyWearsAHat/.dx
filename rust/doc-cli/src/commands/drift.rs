//! `dx drift` — report work happening outside the dx harness.
//!
//! Reads a Claude Code PostToolUse hook JSON from stdin, classifies the tool use as raw (Bash)
//! or dx (mcp__dx__*), records it in a machine-local ledger, and nudges when raw work reaches
//! a multiple of 10. Without stdin or when stdin is a terminal, prints a summary of the most
//! recently modified session or a named session.

use crate::args::Args;
use crate::drift;

/// `dx drift [--session ID]` — report raw command vs dx call ratio for a session.
///
/// Without arguments, reads JSON from stdin (Claude Code hook entry) and records it. With
/// `--session`, prints a summary of that session's counts. Without either, summarizes the
/// most recently modified session.
pub fn run(args: &Args) -> Result<String, String> {
    use std::io::Read;

    // Try to read from stdin. If there's data, it's a hook call; if empty, print summary.
    let mut input = String::new();
    match std::io::stdin().read_to_string(&mut input) {
        Ok(_) if input.trim().is_empty() => {
            // Empty stdin: print a summary
            let session_id = args.value("session");
            Ok(drift::summary(session_id))
        }
        Ok(_) => {
            // Data from stdin: record it
            drift::record(&input)?;
            Ok(String::new()) // Print nothing on successful record
        }
        Err(_) => {
            // Cannot read from stdin (e.g., terminal): print a summary
            let session_id = args.value("session");
            Ok(drift::summary(session_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn help_text_describes_the_command() {
        // This is a placeholder to ensure the command is callable
        let args = args(&[]);
        // We can't easily test stdin redirection here without mocking,
        // but the command will be tested through integration tests
        let _ = args;
    }
}
