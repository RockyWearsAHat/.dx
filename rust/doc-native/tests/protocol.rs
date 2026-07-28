//! Integration tests driving the MCP server through its pure dispatcher.
//!
//! These feed JSON-RPC request values to [`doc_native::handle_request`] against a
//! fresh [`MemoryStore`] and assert protocol behaviour without touching real stdio:
//! the `initialize` handshake, the full `tools/list` catalogue, a `create-document`
//! then `get-document` round-trip, `list`/`search` results, and error handling for
//! unknown methods and bad requests.

use std::io::Cursor;

use doc_native::server;
use doc_native::store::MemoryStore;
use doc_native::tools::TOOL_NAMES;
use doc_native::{handle_request, DocStore};
use serde_json::{json, Value};

/// Send one request and require a response (tests never send notifications).
fn call(store: &mut MemoryStore, request: Value) -> Value {
    handle_request(store, request).expect("request with id must yield a response")
}

/// Extract the parsed JSON object embedded in a `tools/call` text-content result.
fn tool_payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result must carry text content");
    serde_json::from_str(text).expect("tool text content must be valid JSON")
}

#[test]
fn initialize_handshake_matches_reference() {
    let mut store = MemoryStore::new();
    let response = call(
        &mut store,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
    );

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(response["result"]["serverInfo"]["name"], "doc-platform-mcp");
    assert_eq!(response["result"]["serverInfo"]["version"], "0.1.0");
    assert!(response["result"]["capabilities"]["tools"].is_object());
    assert!(response["result"]["capabilities"]["resources"].is_object());
}

#[test]
fn tools_list_returns_every_expected_tool() {
    let mut store = MemoryStore::new();
    let response = call(
        &mut store,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );

    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools must be an array");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("each tool has a name"))
        .collect();

    assert_eq!(names, TOOL_NAMES);
    // Every tool advertises an input schema object.
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[test]
fn create_then_get_round_trips_through_store() {
    let mut store = MemoryStore::new();

    let created = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "create-document",
                "arguments": {
                    "path": "documents/hello.dx",
                    "title": "Hello",
                    "content": "::heading\nHello\n::end\n"
                }
            }
        }),
    );
    let created_doc = tool_payload(&created);
    assert_eq!(created_doc["relativePath"], "documents/hello.dx");
    assert_eq!(created_doc["title"], "Hello");
    assert_eq!(created_doc["source"], "::heading\nHello\n::end\n");

    let fetched = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "get-document",
                "arguments": { "path": "documents/hello.dx" }
            }
        }),
    );
    let fetched_doc = tool_payload(&fetched);
    assert_eq!(fetched_doc, created_doc);

    // The same document is reachable by its assigned numeric id.
    let by_id = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "get-document",
                "arguments": { "id": created_doc["id"] }
            }
        }),
    );
    assert_eq!(tool_payload(&by_id)["relativePath"], "documents/hello.dx");
}

#[test]
fn list_and_search_return_stub_results() {
    let mut store = MemoryStore::new();
    store
        .create(doc_native::store::CreateSpec {
            path: "documents/alpha.dx".to_string(),
            title: "Alpha".to_string(),
            content: Some("alpha body".to_string()),
            ..Default::default()
        })
        .unwrap();
    store
        .create(doc_native::store::CreateSpec {
            path: "documents/beta.dx".to_string(),
            title: "Beta".to_string(),
            content: Some("beta body".to_string()),
            ..Default::default()
        })
        .unwrap();

    let listed = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": { "name": "list-documents", "arguments": {} }
        }),
    );
    let listed_rows = tool_payload(&listed);
    assert_eq!(listed_rows.as_array().unwrap().len(), 2);

    let searched = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "search-documents", "arguments": { "query": "beta" } }
        }),
    );
    let searched_rows = tool_payload(&searched);
    let rows = searched_rows.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["title"], "Beta");
}

#[test]
fn search_without_query_is_invalid_params() {
    let mut store = MemoryStore::new();
    let response = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": { "name": "search-documents", "arguments": {} }
        }),
    );
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn get_missing_document_is_invalid_params() {
    let mut store = MemoryStore::new();
    let response = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": { "name": "get-document", "arguments": { "path": "nope.dx" } }
        }),
    );
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn unknown_method_returns_method_not_found() {
    let mut store = MemoryStore::new();
    let response = call(
        &mut store,
        json!({ "jsonrpc": "2.0", "id": 10, "method": "does/not/exist" }),
    );
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], "Method not found");
}

#[test]
fn unknown_tool_returns_method_not_found() {
    let mut store = MemoryStore::new();
    let response = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": { "name": "frobnicate", "arguments": {} }
        }),
    );
    assert_eq!(response["error"]["code"], -32601);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown tool"));
}

#[test]
fn missing_jsonrpc_version_is_invalid_request() {
    let mut store = MemoryStore::new();
    let response = call(&mut store, json!({ "id": 12, "method": "initialize" }));
    assert_eq!(response["error"]["code"], -32600);
    assert_eq!(response["error"]["message"], "Invalid Request");
}

#[test]
fn notification_without_id_yields_no_response() {
    let mut store = MemoryStore::new();
    let response = handle_request(
        &mut store,
        json!({ "jsonrpc": "2.0", "method": "initialized" }),
    );
    assert!(response.is_none());
}

#[test]
fn resources_list_and_read_round_trip() {
    let mut store = MemoryStore::new();
    store
        .create(doc_native::store::CreateSpec {
            path: "documents/res.dx".to_string(),
            title: "Res".to_string(),
            content: Some("body text".to_string()),
            ..Default::default()
        })
        .unwrap();

    let listed = call(
        &mut store,
        json!({ "jsonrpc": "2.0", "id": 13, "method": "resources/list" }),
    );
    let resources = listed["result"]["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0]["uri"], "doc:///documents/res.dx");
    assert_eq!(resources[1]["uri"], "docview:///documents/res.dx");

    let source = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "resources/read",
            "params": { "uri": "doc:///documents/res.dx" }
        }),
    );
    assert_eq!(source["result"]["contents"][0]["mimeType"], "text/plain");
    assert_eq!(source["result"]["contents"][0]["text"], "body text");

    let view = call(
        &mut store,
        json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "resources/read",
            "params": { "uri": "docview:///documents/res.dx" }
        }),
    );
    assert_eq!(view["result"]["contents"][0]["mimeType"], "text/html");
    assert!(view["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("<!doctype html>"));
}

#[test]
fn stdio_loop_processes_newline_delimited_requests() {
    let mut store = MemoryStore::new();
    let input = Cursor::new(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n\
         not json\n\
         {\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}\n",
    );
    let mut output: Vec<u8> = Vec::new();
    server::run(&mut store, input, &mut output).expect("loop runs to end of input");

    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // Two responses: the initialize reply and a parse error. The notification is silent.
    assert_eq!(lines.len(), 2);

    let init: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");

    let parse_err: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(parse_err["error"]["code"], -32700);
    assert_eq!(parse_err["id"], Value::Null);
}
