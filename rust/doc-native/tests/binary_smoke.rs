//! Real-binary smoke test: proves the compiled, Node-free `doc-native` binary speaks
//! MCP over stdio against a real filesystem workspace.
//!
//! Unlike the dispatcher tests (which call the pure handler in-process), this test
//! launches the *actual compiled executable* — the same binary shipped to agents — feeds
//! it newline-delimited JSON-RPC on stdin, and asserts the stdout responses. Cargo builds
//! the binary before the test and exposes its path via `CARGO_BIN_EXE_doc-native`, so no
//! Node, npm, or TypeScript runtime is involved anywhere in the path.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Monotonic counter so the smoke test's workspace is unique per run.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a unique temp workspace directory for the smoke test.
fn temp_workspace() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("doc-native-smoke-{nanos}-{unique}"));
    fs::create_dir_all(&root).expect("create smoke workspace");
    root
}

/// Run the built binary with `DOC_ROOT` set to `root`, feeding `requests` on stdin and
/// returning the parsed JSON-RPC responses (one per non-empty stdout line).
fn run_binary(root: &Path, requests: &[Value]) -> Vec<Value> {
    let exe = env!("CARGO_BIN_EXE_doc-native");
    let mut input = String::new();
    for request in requests {
        input.push_str(&serde_json::to_string(request).expect("serialize request"));
        input.push('\n');
    }

    let mut child = Command::new(exe)
        .env("DOC_ROOT", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn doc-native binary");

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    // Dropping stdin (taken above) closes it, so the binary's read loop reaches EOF.

    let output = child.wait_with_output().expect("await binary");
    assert!(
        output.status.success(),
        "binary exited with failure: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("stdout is utf-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each stdout line is JSON-RPC"))
        .collect()
}

#[test]
fn compiled_binary_speaks_mcp_over_stdio() {
    let root = temp_workspace();

    let requests = vec![
        serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "create-document",
                "arguments": {
                    "path": "documents/smoke.dx",
                    "title": "Smoke Test",
                    "content": "::heading level=1 id=h\nSmoke Test\n::end\n\n::paragraph id=p\nhello from the native binary\n::end\n"
                }
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "get-document",
                "arguments": { "path": "documents/smoke.dx" }
            }
        }),
    ];

    let responses = run_binary(&root, &requests);
    assert_eq!(responses.len(), 3, "expected one response per request");

    // initialize handshake.
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(
        responses[0]["result"]["serverInfo"]["name"],
        "doc-platform-mcp"
    );

    // create-document: the embedded text payload is a JSON document record.
    assert_eq!(responses[1]["id"], 2);
    let created_text = responses[1]["result"]["content"][0]["text"]
        .as_str()
        .expect("create result carries text content");
    let created: Value = serde_json::from_str(created_text).expect("create payload is JSON");
    assert_eq!(created["relativePath"], "documents/smoke.dx");
    assert_eq!(created["title"], "Smoke Test");

    // The compiled binary really wrote to disk: a stub file and a bundle now exist.
    assert!(
        root.join("documents/smoke.dx").exists(),
        "binary must write the .dx stub"
    );
    let wrote_a_bundle =
        root.join(".doc/.repo-docs.bin").exists() || root.join(".doc/.local-docs.bin").exists();
    assert!(wrote_a_bundle, "binary must write a .doc/*.bin bundle");

    // get-document round-trips the body through the binary's parse/stringify.
    assert_eq!(responses[2]["id"], 3);
    let fetched_text = responses[2]["result"]["content"][0]["text"]
        .as_str()
        .expect("get result carries text content");
    let fetched: Value = serde_json::from_str(fetched_text).expect("get payload is JSON");
    assert_eq!(fetched["relativePath"], "documents/smoke.dx");
    assert!(fetched["source"]
        .as_str()
        .expect("source is a string")
        .contains("hello from the native binary"));

    let _ = fs::remove_dir_all(&root);
}
