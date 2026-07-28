//! Binary entry point for the DOC native MCP server.
//!
//! Constructs the real filesystem-backed [`FsDocStore`] rooted at the workspace
//! (`$DOC_ROOT`, or the current working directory) and runs the stdio JSON-RPC loop over
//! the process's standard streams. The protocol layer is store-agnostic, so swapping the
//! store constructed here is the only change needed to back the server with real storage.

use std::env;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use doc_native::server;
use doc_native::FsDocStore;

/// Resolve the workspace root from `$DOC_ROOT`, falling back to the current directory.
fn workspace_root() -> PathBuf {
    if let Some(root) = env::var_os("DOC_ROOT") {
        return PathBuf::from(root);
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Start the server, returning a non-zero exit code on a fatal I/O error.
fn main() -> ExitCode {
    // Startup banner goes to stderr so it never corrupts the stdout JSON-RPC stream.
    let _ = writeln!(
        io::stderr(),
        "[MCP Server] Document Platform MCP Server started"
    );

    let mut store = FsDocStore::new(workspace_root());
    let stdin = BufReader::new(io::stdin().lock());
    let mut stdout = io::stdout().lock();

    match server::run(&mut store, stdin, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "[MCP Server] fatal I/O error: {err}");
            ExitCode::FAILURE
        }
    }
}
