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
mod daemon;
mod desktop;
mod extension;
mod home;
mod install;
mod mcp;
mod policies;
mod reports;
mod service;
mod state;
mod workspace;

use std::io::{self, Write};
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
        return write_out(commands::setup::HELP);
    };

    let parsed = Args::parse(&raw[1..]);
    // Both servers run until they are stopped, so neither returns an `Output` to print —
    // which is exactly why `--help` has to be answered here, before either starts.
    if (command == "mcp" || command == "serve") && parsed.present("help") {
        return write_out(commands::setup::HELP);
    }
    // The servers cannot go through the dispatch table (they never return an `Output`),
    // but a flag they do not read is refused by the same formatter, never swallowed.
    if command == "mcp" {
        if let Some(refused) = commands::refuse_unknown_flags("mcp", &parsed, &[]) {
            let _ = writeln!(io::stderr(), "{refused}");
            return ExitCode::FAILURE;
        }
        return serve_mcp();
    }
    if command == "serve" {
        if let Some(refused) = commands::refuse_unknown_flags("serve", &parsed, &["port"]) {
            let _ = writeln!(io::stderr(), "{refused}");
            return ExitCode::FAILURE;
        }
        return serve_documents(&parsed);
    }
    match commands::dispatch(&command, &parsed) {
        Ok(output) => emit(&output, &parsed),
        Err(message) => {
            let _ = writeln!(io::stderr(), "{}", message.trim_end());
            ExitCode::FAILURE
        }
    }
}

/// Write `text` to stdout, treating a reader that walked away as a finished job.
///
/// `print!` panics when the pipe it writes into has closed, which is what `dx source big.dx
/// | head` does the moment `head` has its lines — a normal shell gesture answered with a
/// Rust backtrace. The reader asked for less than there was and got it; that is a success,
/// not a crash. Any other write failure is a real one and says so.
fn write_out(text: &str) -> ExitCode {
    let mut stdout = io::stdout();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "cannot write to standard output: {error}");
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
            Ok(()) => write_out(&format!("wrote {path}\n")),
            Err(message) => {
                let _ = writeln!(io::stderr(), "{message}");
                ExitCode::FAILURE
            }
        },
        None => write_out(output.text()),
    }
}

/// Serve MCP over stdio until the client disconnects, re-execing when the binary on disk
/// is updated so a long-lived session always answers with the current engine.
///
/// The banner goes to stderr: stdout carries the JSON-RPC stream and nothing else.
fn serve_mcp() -> ExitCode {
    let root = workspace_root();
    let _ = writeln!(
        io::stderr(),
        "dx mcp — serving .dx documents from {}",
        root.display()
    );

    match mcp::serve(&root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "dx mcp — fatal I/O error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run `dx serve`, the local rendering service, until the process is stopped.
///
/// The banner goes to stdout: unlike `dx mcp` there is no protocol stream to keep clean, and
/// a person who started a server wants to see which port it took.
fn serve_documents(args: &Args) -> ExitCode {
    let port = match args.value("port") {
        None => None,
        Some(value) => match value.parse::<u16>() {
            Ok(port) => Some(port),
            Err(_) => {
                let _ = writeln!(io::stderr(), "`{value}` is not a port number");
                return ExitCode::FAILURE;
            }
        },
    };

    let mut stdout = io::stdout().lock();
    match daemon::serve(port, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = writeln!(io::stderr(), "{}", message.trim_end());
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
