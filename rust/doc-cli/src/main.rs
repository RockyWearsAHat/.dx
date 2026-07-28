//! `dx` — the command line for `.dx` documents.
//!
//! A `.dx` file is a notepad for code: plain-text blocks that render to a page, and code
//! blocks that actually run, with their output stored back in the document. One binary
//! serves every consumer of that idea — a person at a terminal, an editor, and an AI agent
//! (`dx mcp`) — so all of them see the same document rendered the same way.
//!
//! Everything here is a thin shell. Parsing and rendering live in `doc-core`, execution in
//! `doc-run`, screenshots in `doc-shot`; this crate resolves paths, formats output, and
//! chooses an exit code.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

mod args;
mod commands;
mod install;
mod mcp;
mod workspace;

use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use args::Args;
use commands::Output;

/// Environment variable that overrides the workspace root the MCP server serves.
const ROOT_ENV: &str = "DX_ROOT";

/// Parse the command line, run the command, and report the outcome.
fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = raw.first().cloned() else {
        print!("{}", commands::setup::HELP);
        return ExitCode::SUCCESS;
    };

    if command == "mcp" {
        return serve_mcp();
    }

    let parsed = Args::parse(&raw[1..]);
    match commands::dispatch(&command, &parsed) {
        Ok(output) => emit(&output, &parsed),
        Err(message) => {
            let _ = writeln!(io::stderr(), "{}", message.trim_end());
            ExitCode::FAILURE
        }
    }
}

/// Print a command's output, honoring `--out` for content the caller asked for.
///
/// A report is always printed: it describes files the command already wrote, so
/// redirecting it into `--out` would overwrite exactly those files.
fn emit(output: &Output, args: &Args) -> ExitCode {
    let target = match output {
        Output::Document(_) => args.value("out").filter(|value| *value != "-"),
        Output::Report(_) => None,
    };

    match target {
        Some(path) => match workspace::write_text(&PathBuf::from(path), output.text()) {
            Ok(()) => {
                println!("wrote {path}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                let _ = writeln!(io::stderr(), "{message}");
                ExitCode::FAILURE
            }
        },
        None => {
            print!("{}", output.text());
            let _ = io::stdout().flush();
            ExitCode::SUCCESS
        }
    }
}

/// Serve MCP over stdio until the client disconnects.
///
/// The banner goes to stderr: stdout carries the JSON-RPC stream and nothing else.
fn serve_mcp() -> ExitCode {
    let root = workspace_root();
    let _ = writeln!(
        io::stderr(),
        "dx mcp — serving .dx documents from {}",
        root.display()
    );

    let stdin = BufReader::new(io::stdin().lock());
    let mut stdout = io::stdout().lock();
    match mcp::serve(&root, stdin, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "dx mcp — fatal I/O error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The directory the MCP server treats as the project root.
fn workspace_root() -> PathBuf {
    std::env::var_os(ROOT_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both root cases live in one test on purpose: they share one process-wide
    /// environment variable, and as separate tests they would race under the parallel
    /// test runner.
    #[test]
    fn the_workspace_root_prefers_the_override_and_otherwise_exists() {
        std::env::set_var(ROOT_ENV, "/dx/explicit");
        assert_eq!(workspace_root(), PathBuf::from("/dx/explicit"));

        std::env::remove_var(ROOT_ENV);
        assert!(workspace_root().exists());
    }
}
