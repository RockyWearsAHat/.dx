//! The MCP server: `dx mcp`.
//!
//! A newline-delimited JSON-RPC 2.0 loop over stdin/stdout, which is how MCP clients start
//! a local server. [`handle`] is a pure function from one request to one response, so the
//! whole protocol is testable without spawning a process; [`serve`] is the thin I/O
//! wrapper around it.
//!
//! Documents are also exposed as MCP *resources* under `dx://<path>`, so a client that
//! browses resources sees the project's documentation without calling a tool first.

mod handlers;
mod tools;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// MCP protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Server name reported during the handshake.
pub const SERVER_NAME: &str = "dx";

/// Server version reported during the handshake.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// URI scheme for document resources.
const RESOURCE_SCHEME: &str = "dx://";

/// JSON-RPC error codes used by this server.
mod code {
    /// The payload was not valid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The payload was not a JSON-RPC request.
    pub const INVALID_REQUEST: i64 = -32600;
    /// No such method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The arguments were missing or wrong.
    pub const INVALID_PARAMS: i64 = -32602;
}

/// Serve MCP over `input`/`output` until the client disconnects.
///
/// Returns the first I/O error; a clean end-of-input is success.
pub fn serve<R: BufRead, W: Write>(root: &Path, input: R, output: &mut W) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(&request, root),
            Err(_) => Some(error(Value::Null, code::PARSE_ERROR, "Parse error")),
        };

        if let Some(response) = response {
            let encoded = serde_json::to_string(&response).unwrap_or_else(|_| "null".to_string());
            writeln!(output, "{encoded}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// Handle one request, returning the response — or `None` for a notification.
#[must_use]
pub fn handle(request: &Value, root: &Path) -> Option<Value> {
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        let id = request.get("id")?.clone();
        return Some(error(id, code::INVALID_REQUEST, "Invalid Request"));
    }
    // Notifications carry no id and get no reply.
    let id = request.get("id")?.clone();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    Some(match method {
        "initialize" => success(id, initialize()),
        "tools/list" => success(id, json!({ "tools": tools::catalogue() })),
        "tools/call" => tool_call(id, &params, root),
        "resources/list" => success(id, resources(root)),
        "resources/read" => resource_read(id, &params, root),
        "ping" => success(id, json!({})),
        _ => error(id, code::METHOD_NOT_FOUND, "Method not found"),
    })
}

/// The handshake result: what this server is and what it can do.
fn initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {}, "resources": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "instructions": "This project uses .dx documents: block documents that render to \
                         pages and can execute their own code blocks. Treat them as durable, \
                         token-cheap memory: read recorded results instead of re-deriving \
                         them. Find before reading: dx_search or dx_list, then dx_outline to \
                         map a document as one row per block, and always index into the \
                         document — read one `section` (any block id), never page through \
                         the rest. Prose and code are cheapest as text: dx_source, a \
                         fraction of what page images cost. Spend dx_read's images only on \
                         what text cannot carry — boards, diagrams, charts, rendered views. \
                         Reads are live: stale output of approved code re-runs before you \
                         see it, so what you read is what the code does now; only \
                         unreviewed code waits, and the read says so. Edit with dx_edit, \
                         one block by id — an edited runnable block runs at once, output \
                         fresh. Save what you learn as documents: `::code src=<path>` \
                         indexes a file as its current text, never a stale copy, and run \
                         output is fingerprinted in place, so an index costs one section \
                         read to consult, forever. If dx_list finds no documents, offer \
                         dx_index: it scaffolds index.dx from the tree — read it whole and \
                         improve it before other work."
    })
}

/// Run one tool and wrap its content items in a `tools/call` result.
fn tool_call(id: Value, params: &Value, root: &Path) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);

    match handlers::call(name, args, root) {
        Ok(content) => success(id, json!({ "content": content })),
        // A tool failure is reported inside the result, not as a protocol error, so the
        // agent sees the message and can correct itself rather than losing the call.
        Err(message) => success(
            id,
            json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true,
            }),
        ),
    }
}

/// Every document in the workspace, as MCP resources.
fn resources(root: &Path) -> Value {
    let entries: Vec<Value> = crate::workspace::load_all(root)
        .into_iter()
        .map(|loaded| {
            json!({
                "uri": format!("{RESOURCE_SCHEME}{}", loaded.relative),
                "name": loaded.title(),
                "description": format!("{} ({} blocks)", loaded.relative, loaded.document.blocks.len()),
                "mimeType": "text/markdown",
            })
        })
        .collect();
    json!({ "resources": entries })
}

/// Read one document resource as Markdown.
fn resource_read(id: Value, params: &Value, root: &Path) -> Value {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return error(id, code::INVALID_PARAMS, "`uri` is required");
    };
    let relative = uri.strip_prefix(RESOURCE_SCHEME).unwrap_or(uri);
    let path: PathBuf = root.join(relative);

    match crate::workspace::read(&path) {
        Ok(source) => {
            let document = doc_core::format::parse(&source);
            let body = doc_core::render::text(&document, &doc_core::render::TextOptions::default());
            success(
                id,
                json!({
                    "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": body }]
                }),
            )
        }
        Err(message) => error(id, code::INVALID_PARAMS, message),
    }
}

/// A JSON-RPC success response.
fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC error response.
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace;

    fn project(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-server-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        workspace::write_text(
            &root.join("guide.dx"),
            "::heading level=1 id=title\nGuide\n::end\n\n::paragraph id=p\nBody.\n::end\n",
        )
        .expect("seed");
        root
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn the_handshake_advertises_tools_resources_and_guidance() {
        let root = project("handshake");
        let response = handle(&request(1, "initialize", json!({})), &root).expect("response");
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], "dx");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        let instructions = response["result"]["instructions"].as_str().expect("text");
        // The guidance is a token economy: text before images, one section at a time,
        // live output on every read, one block per edit, durable results in documents —
        // and a scaffolded index for a project that has none yet.
        assert!(instructions.contains("dx_source"));
        assert!(instructions.contains("dx_read"));
        assert!(instructions.contains("images"));
        assert!(instructions.contains("section"));
        assert!(instructions.contains("live"));
        assert!(instructions.contains("dx_edit"));
        assert!(instructions.contains("::code src="));
        assert!(instructions.contains("dx_index"));
    }

    #[test]
    fn tools_list_returns_the_catalogue() {
        let root = project("tools");
        let response = handle(&request(2, "tools/list", json!({})), &root).expect("response");
        let tools = response["result"]["tools"].as_array().expect("array");
        assert_eq!(tools.len(), tools::TOOL_NAMES.len());
    }

    #[test]
    fn a_tool_call_returns_content() {
        let root = project("call");
        let response = handle(
            &request(
                3,
                "tools/call",
                json!({ "name": "dx_source", "arguments": { "path": "guide.dx" } }),
            ),
            &root,
        )
        .expect("response");
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("# Guide"));
        assert!(response["result"]["isError"].is_null());
    }

    #[test]
    fn a_tool_failure_is_reported_in_the_result_so_the_agent_can_retry() {
        let root = project("call-fail");
        let response = handle(
            &request(
                4,
                "tools/call",
                json!({ "name": "dx_read", "arguments": {} }),
            ),
            &root,
        )
        .expect("response");
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("required"));
        assert!(response["error"].is_null());
    }

    #[test]
    fn documents_are_listed_and_readable_as_resources() {
        let root = project("resources");
        let listed = handle(&request(5, "resources/list", json!({})), &root).expect("response");
        let uri = listed["result"]["resources"][0]["uri"]
            .as_str()
            .expect("uri")
            .to_string();
        assert_eq!(uri, "dx://guide.dx");

        let read =
            handle(&request(6, "resources/read", json!({ "uri": uri })), &root).expect("response");
        assert!(read["result"]["contents"][0]["text"]
            .as_str()
            .expect("text")
            .contains("Guide"));
    }

    #[test]
    fn notifications_get_no_reply() {
        let root = project("notify");
        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(&notification, &root).is_none());
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let root = project("unknown-method");
        let response = handle(&request(7, "nope", json!({})), &root).expect("response");
        assert_eq!(response["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn a_bad_envelope_is_rejected_without_crashing() {
        let root = project("bad-envelope");
        let response =
            handle(&json!({ "id": 8, "method": "initialize" }), &root).expect("response");
        assert_eq!(response["error"]["code"], code::INVALID_REQUEST);
    }

    #[test]
    fn the_stdio_loop_answers_requests_line_by_line() {
        let root = project("loop");
        let input = format!(
            "{}\n{}\n\n",
            serde_json::to_string(&request(1, "initialize", json!({}))).expect("json"),
            serde_json::to_string(&request(2, "tools/list", json!({}))).expect("json"),
        );
        let mut output = Vec::new();
        serve(&root, input.as_bytes(), &mut output).expect("serve");

        let lines: Vec<&str> = std::str::from_utf8(&output)
            .expect("utf8")
            .lines()
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(lines.len(), 2);
        let second: Value = serde_json::from_str(lines[1]).expect("json");
        assert_eq!(second["id"], 2);
    }

    #[test]
    fn unparseable_input_gets_a_parse_error_and_the_loop_continues() {
        let root = project("parse-error");
        let input = format!(
            "not json\n{}\n",
            serde_json::to_string(&request(9, "ping", json!({}))).expect("json")
        );
        let mut output = Vec::new();
        serve(&root, input.as_bytes(), &mut output).expect("serve");

        let lines: Vec<&str> = std::str::from_utf8(&output)
            .expect("utf8")
            .lines()
            .collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).expect("json");
        assert_eq!(first["error"]["code"], code::PARSE_ERROR);
    }
}
