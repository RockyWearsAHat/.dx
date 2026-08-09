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

use std::io::{BufRead, BufReader, Write};
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

/// Serve MCP over this process's stdio until the client disconnects, re-execing in place
/// when the binary on disk is updated so a long-lived session always answers with the
/// current engine.
///
/// Returns the first I/O error; a clean end-of-input is success.
pub fn serve(root: &Path) -> std::io::Result<()> {
    let engine = engine_fingerprint();
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = std::io::stdout().lock();
    serve_buffered(root, &mut input, &mut output, engine.as_ref())
}

/// The serving loop behind [`serve`]: answer requests line by line — and keep the engine
/// current.
///
/// MCP clients hold a server for the life of a session, which outlives any `dx` upgrade:
/// without the drift check, every long-lived agent session keeps running the engine it
/// started with, old bugs included, until a person thinks to restart the assistant. So
/// after each answer, if the binary on disk no longer matches `engine` — the fingerprint
/// recorded at startup — the server re-execs it in place: same arguments, same stdio, and
/// the next request is served by the new engine. The check runs only while the reader's
/// own buffer is empty: bytes still in the kernel pipe survive an exec, bytes already
/// buffered here would not, so a pipelined request is never dropped (drift is caught
/// after a later answer instead).
fn serve_buffered<R: std::io::Read, W: Write>(
    root: &Path,
    input: &mut BufReader<R>,
    output: &mut W,
    engine: Option<&(PathBuf, EngineFingerprint)>,
) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(()); // clean end of input: the client disconnected
        }
        if let Some(encoded) = answer(&line, root) {
            writeln!(output, "{encoded}")?;
            output.flush()?;
        }
        if let Some((exe, recorded)) = engine {
            if input.buffer().is_empty() && engine_drifted(exe, recorded) {
                let _ = writeln!(
                    std::io::stderr(),
                    "dx mcp — binary updated on disk; restarting with the new engine"
                );
                reexec(exe); // returns only if the exec failed; keep serving then
            }
        }
    }
}

/// Answer one wire line: parse, dispatch, encode. `None` for blank lines and
/// notifications, which get no reply.
fn answer(line: &str, root: &Path) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let response = match serde_json::from_str::<Value>(line) {
        Ok(request) => handle(&request, root)?,
        Err(_) => error(Value::Null, code::PARSE_ERROR, "Parse error"),
    };
    Some(serde_json::to_string(&response).unwrap_or_else(|_| "null".to_string()))
}

/// What identifies the engine on disk: its length and modification time. Cheap enough to
/// check after every request, and any reinstall changes it (a rewritten file gets a new
/// mtime even at the same length).
type EngineFingerprint = (u64, std::time::SystemTime);

/// The running binary's path and fingerprint, recorded at startup. `None` when the
/// binary cannot be identified — then the server simply never re-execs.
fn engine_fingerprint() -> Option<(PathBuf, EngineFingerprint)> {
    let exe = std::env::current_exe().ok()?;
    let recorded = fingerprint_of(&exe)?;
    Some((exe, recorded))
}

/// Fingerprint the file at `path`, or `None` when it cannot be read.
fn fingerprint_of(path: &Path) -> Option<EngineFingerprint> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Whether the binary at `exe` no longer matches `recorded`. A file that cannot be
/// fingerprinted right now (mid-replacement, or gone) is not drift — the next answer
/// checks again.
fn engine_drifted(exe: &Path, recorded: &EngineFingerprint) -> bool {
    fingerprint_of(exe).is_some_and(|current| current != *recorded)
}

/// Replace this process with a fresh execution of `exe`, same arguments, same stdio.
/// Returns only when the exec itself failed; the caller keeps serving with the old engine.
#[cfg(unix)]
fn reexec(exe: &Path) {
    use std::os::unix::process::CommandExt;
    let error = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .exec();
    let _ = writeln!(std::io::stderr(), "dx mcp — restart failed: {error}");
}

/// On platforms without `exec`, an updated binary keeps serving until the session ends.
#[cfg(not(unix))]
fn reexec(_exe: &Path) {}

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
        "instructions": "This project works through .dx documents: block documents that \
                         render to pages and execute their own code blocks. dx is not a \
                         sidecar for notes — it is the working method, and it is the \
                         default: every agent connected to this server works in the dx \
                         methodology, always, on every task, unless specifically \
                         requested not to. Falling back to raw exploration or hand-run \
                         builds because it feels quicker is leaving the method; only an \
                         explicit request is. Speak minimally and work constantly: act on \
                         what the task requires and implies — judged from everything \
                         understood so far, not only the latest message — and let the \
                         documents carry the explanation. The documents are \
                         the project's index, its verification harness, its proof of \
                         progress, and your memory between sessions; worked through \
                         properly they replace re-reading the codebase and re-running \
                         builds by hand, which is where the order-of-magnitude token and \
                         time savings live. Whole projects are built this way: orient from \
                         the index, edit through blocks, and let recorded verdicts prove \
                         each claim, so a task finishes in one pass and the next session \
                         inherits proof instead of redoing work.\n\n\
                         ORIENT. Every codebase you work in gets indexed — always. \
                         dx_list first; read index.dx if it exists. If dx_list \
                         finds no documents, run dx_index — it scaffolds index.dx from the \
                         file tree — then read the scaffold whole and improve it as you \
                         learn (replace TODOs, add `::code src=<path>` blocks for the \
                         load-bearing files: they render as the file's current text, never \
                         a stale copy), so orientation costs one read forever after. Find \
                         before reading: dx_search — a hit carries the best block's id and \
                         text, so a search that lands is the read. Map with dx_outline \
                         (one row per block) and read one `section` with dx_source; never \
                         page through a document.\n\n\
                         READ ECONOMY. Prose and code are text: dx_source, a fraction of \
                         what images cost. Spend dx_read's page images only on what text \
                         cannot carry — boards, diagrams, charts, rendered views — one \
                         `block` at a time, never a page sweep. Reads are live: stale \
                         output of approved code re-runs before you see it, so what you \
                         read is what the code does now; only unreviewed code waits, and \
                         the read says so.\n\n\
                         THE HARNESS. Every claim the work depends on — it compiles, the \
                         tests pass, the page renders right — lives as a `::code run` \
                         block declaring what it judges with reads= (files or folders, \
                         reads=src) and granting its build directory with writes= \
                         (writes=target). Its recorded verdict stales mechanically when \
                         any input changes and re-runs on your next read, so a recorded \
                         claim never needs re-verifying by hand. Build the harness before \
                         building the thing, then loop: edit, read the verdict, fix, \
                         repeat — until every claim holds. dx_edit changes one block by \
                         id — an edited runnable \
                         block runs at once, output fresh — its `replace`+`with` params \
                         change an exact string inside the body so a small edit costs \
                         the change instead of a retyped block, and its `header` param \
                         retypes the `::kind attrs` line, so changing an attribute never \
                         costs a document rewrite (dx_write is for new documents). \
                         Planning, creation, and verification are one motion, not three \
                         phases: dx_edit writes the intent and the code in one block, the \
                         edit runs it, dx_read shows the result — three calls, and the \
                         document is already the proof. A task \
                         is complete when its document proves it, not when the code looks \
                         done.\n\n\
                         THE DOCUMENTS ARE THE MEMORY. Everything needed to work on the \
                         project lives in its documents — decisions, constraints, \
                         findings, dead ends — written the moment they form and linked \
                         across documents, so any fact is one search away and nothing is \
                         ever re-derived. Keeping them current is your responsibility, \
                         always: a change that stales a document updates it in the same \
                         sweep, and a claim worth keeping is recorded as a run block whose \
                         verdict proves it — that is what keeps this memory factual \
                         instead of a pile of notes that drift from the code.\n\n\
                         WORK IN THE DOCUMENT, NOT IN CONTEXT. Conversation context is a \
                         cache; the document is the memory. Think by writing: a \
                         hypothesis, a judgment, a dead end goes into the working section \
                         the moment it forms — reading your own two-line note back costs \
                         ~50 tokens, re-deriving it costs thousands, every time. \
                         dx_append adds the lines without resending anything; dx_check \
                         ticks a worklist box. The `now` section (dx_index scaffolds it) \
                         is the program counter: a live worklist — found / fixing / \
                         verified — that makes the task the unit of work, not the \
                         session; it survives compaction, crashes, and handoffs, and a \
                         second agent picks up an unclaimed line. The convention that \
                         makes it hold: the first tool call of a turn reads the working \
                         section, the last one writes it. Between them the conversation \
                         stays thin — read the worklist line, read the one section under \
                         edit, run the verdict, write the result back; context never \
                         holds the project, only the current move. Scratch promotes: \
                         when a note hardens, move it into the durable section it \
                         belongs to and delete the scratch.\n\n\
                         RESULTS LIVE IN THE DOCUMENT. Run output folds in as ::output, \
                         fingerprinted in place. A picture a run produces — a screenshot, \
                         a rendered frame — is an `::image src=<file> for=<run-block-id>` \
                         so the page itself vouches for its freshness: a failed or unrun \
                         producer is called out on the figure. A gallery of hand-managed \
                         images proves nothing and is the smell that you have left the \
                         method; prefer the verdict, and keep any embedded image under \
                         8MB (capture at scale 1).\n\n\
                         PIXELS IN SUBAGENTS. A page image is the costliest thing a \
                         context can carry, so frames are judged in a review subagent \
                         that returns verdicts: the subagent reads the pixels and answers \
                         with what holds and what fails, and the operator's context \
                         carries verdicts, never page images. The operator looks at \
                         pixels only to set design direction — deciding what a page \
                         should be needs the operator's own eyes; checking that it is \
                         stays delegated.\n\n\
                         THE REVIEW BAR IS STANDING. Whenever a session changes a \
                         rendered surface, it runs a reviewer against the mission bar \
                         and iterates until PASS before claiming done — a surface change \
                         nobody reviewed is a claim nobody proved. The document's verify \
                         block is the mechanical twin of that review: it re-runs by \
                         staleness, so the bar holds between sessions with no one \
                         re-invoking it by hand.\n\n\
                         BATCH EVERY CAPTURE. A capture's cost is the browser launch, \
                         not the frame, so captures share one launch: name every block \
                         wanted in one call (comma-separated --block), never one call \
                         per frame."
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
        // A search hit carries its answer, and images are spent one block at a time.
        assert!(instructions.contains("a search that lands is the read"));
        assert!(instructions.contains("never a page sweep"));
        // And the guidance names the method: the verify-block harness, driven in a loop
        // until every claim holds — completion is the document's proof, not an impression.
        assert!(instructions.contains("harness"));
        assert!(instructions.contains("until every claim holds"));
        // The method is the point: dx is the way the work is done, results live in the
        // document, a run-produced picture names its producer, and attributes are one
        // header edit — the gaps a real session fell through when this was left unsaid.
        assert!(instructions.contains("the working method"));
        // The method is the standing default for every connected agent — dropped only on
        // an explicit request, never for convenience. Left unsaid, sessions drifted back
        // to raw grepping and hand-run builds the moment the loop felt slow.
        assert!(instructions.contains("unless specifically requested not to"));
        assert!(instructions.contains("only an explicit request is"));
        // Work is judged from everything understood, not the latest message alone; the
        // reply stays thin because the documents carry the explanation.
        assert!(instructions.contains("not only the latest message"));
        assert!(instructions.contains("Speak minimally and work constantly"));
        // Indexing is unconditional, the documents are the project's factual memory the
        // agent keeps current, and plan/create/verify is one three-call motion.
        assert!(instructions.contains("gets indexed — always"));
        assert!(instructions.contains("THE DOCUMENTS ARE THE MEMORY"));
        assert!(instructions.contains("in the same sweep"));
        assert!(instructions.contains("one search away"));
        assert!(instructions.contains("three calls"));
        // The inversion: work lives in the document while it happens — the `now` section
        // carries the program counter, the first call of a turn reads it and the last
        // writes it, and the cheap writes are what make that the easy path.
        assert!(instructions.contains("WORK IN THE DOCUMENT, NOT IN CONTEXT"));
        assert!(instructions.contains("Think by writing"));
        assert!(instructions.contains("program counter"));
        assert!(instructions.contains("first tool call of a turn reads the working section"));
        assert!(instructions.contains("dx_append"));
        assert!(instructions.contains("dx_check"));
        assert!(instructions.contains("Scratch promotes"));
        assert!(instructions.contains("for=<run-block-id>"));
        assert!(instructions.contains("`header` param"));
        assert!(instructions.contains("hand-managed images proves nothing"));
        // The visual loop: frames are judged in a review subagent so the operator's
        // context holds verdicts rather than page images, pixels reach the operator only
        // to set direction, every rendered-surface change is reviewed to PASS with the
        // verify block as its mechanical twin, and captures batch into one browser
        // launch instead of one call per frame.
        assert!(instructions.contains("PIXELS IN SUBAGENTS"));
        assert!(instructions.contains("review subagent"));
        assert!(instructions.contains("verdicts, never page images"));
        assert!(instructions.contains("only to set design direction"));
        assert!(instructions.contains("THE REVIEW BAR IS STANDING"));
        assert!(instructions.contains("mission bar"));
        assert!(instructions.contains("until PASS before claiming done"));
        assert!(instructions.contains("mechanical twin"));
        assert!(instructions.contains("re-runs by staleness"));
        assert!(instructions.contains("BATCH EVERY CAPTURE"));
        assert!(instructions.contains("share one launch"));
        assert!(instructions.contains("comma-separated --block"));
        assert!(instructions.contains("never one call per frame"));
    }

    #[test]
    fn a_rewritten_binary_is_drift_and_an_untouched_one_is_not() {
        let dir = std::env::temp_dir().join("dx-server-tests-drift");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let exe = dir.join("engine");
        std::fs::write(&exe, b"engine v1").expect("write");
        let recorded = fingerprint_of(&exe).expect("fingerprint");

        assert!(!engine_drifted(&exe, &recorded));
        // A reinstall writes new bytes; the different length alone must register.
        std::fs::write(&exe, b"engine v2, longer").expect("rewrite");
        assert!(engine_drifted(&exe, &recorded));
    }

    #[test]
    fn a_binary_that_cannot_be_read_is_not_drift() {
        // Mid-replacement (or deleted), the file may be unreadable for a moment; the
        // server must keep serving rather than re-exec into nothing.
        let gone = std::env::temp_dir().join("dx-server-tests-drift-gone/engine");
        assert!(fingerprint_of(&gone).is_none());
        assert!(!engine_drifted(
            &gone,
            &(0, std::time::SystemTime::UNIX_EPOCH)
        ));
    }

    #[test]
    fn answers_carry_the_reply_and_skip_blank_lines_and_notifications() {
        let root = project("answer");
        let ping = serde_json::to_string(&request(9, "ping", json!({}))).expect("encode");
        assert!(answer(&ping, &root).expect("reply").contains("\"id\":9"));
        assert!(answer("", &root).is_none());
        assert!(answer("   ", &root).is_none());
        // A notification (no id) gets no reply.
        let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(answer(note, &root).is_none());
        // Garbage still gets a parse error, not silence.
        assert!(answer("not json", &root).expect("reply").contains("-32700"));
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
        serve_buffered(
            &root,
            &mut BufReader::new(input.as_bytes()),
            &mut output,
            None,
        )
        .expect("serve");

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
        serve_buffered(
            &root,
            &mut BufReader::new(input.as_bytes()),
            &mut output,
            None,
        )
        .expect("serve");

        let lines: Vec<&str> = std::str::from_utf8(&output)
            .expect("utf8")
            .lines()
            .collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).expect("json");
        assert_eq!(first["error"]["code"], code::PARSE_ERROR);
    }
}
