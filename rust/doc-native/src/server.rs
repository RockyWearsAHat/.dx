//! The newline-delimited stdio JSON-RPC loop.
//!
//! [`run`] reads one JSON object per line from a [`BufRead`] source, dispatches each
//! through [`crate::dispatch::handle_request`], and writes one response object per
//! line to a [`Write`] sink. This is the thin I/O wrapper around the pure dispatcher;
//! all protocol logic lives in [`crate::dispatch`].
//!
//! Framing matches the reference server's `readline`-over-stdin model exactly: lines
//! are the message boundary, and unparseable lines yield a JSON-RPC parse error with a
//! null id. The loop ends cleanly at end-of-input.

use std::io::{BufRead, Write};

use serde_json::Value;

use crate::dispatch::handle_request;
use crate::protocol::{error, error_code};
use crate::store::DocStore;

/// Run the stdio JSON-RPC loop until `input` reaches end-of-stream.
///
/// Each input line is parsed as JSON and dispatched against `store`; responses (and
/// parse errors) are written to `output`, one JSON object per line, and flushed so a
/// peer reading line-by-line sees each reply immediately. Returns the first I/O error
/// encountered while reading or writing; a clean end-of-input returns `Ok(())`.
pub fn run<S, R, W>(store: &mut S, input: R, output: &mut W) -> std::io::Result<()>
where
    S: DocStore,
    R: BufRead,
    W: Write,
{
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle_request(store, request),
            Err(_) => Some(error(Value::Null, error_code::PARSE_ERROR, "Parse error")),
        };

        if let Some(response) = response {
            // Serialization of a well-formed serde_json::Value cannot fail; if it
            // somehow did we fall back to a literal null so the loop stays alive.
            let encoded = serde_json::to_string(&response).unwrap_or_else(|_| "null".to_string());
            writeln!(output, "{encoded}")?;
            output.flush()?;
        }
    }

    Ok(())
}
