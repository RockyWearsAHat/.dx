//! Request dispatch: the pure, testable core of the MCP server.
//!
//! [`handle_request`] maps one parsed JSON-RPC request value to an optional response
//! value, with no I/O. The stdio loop in [`crate::server`] is a thin wrapper that
//! reads lines, calls this function, and prints the result. Keeping dispatch pure
//! makes the whole protocol exercisable from integration tests by feeding it
//! [`serde_json::Value`]s directly.
//!
//! The return is `Option<Value>`: notifications (requests without an `id`) produce
//! `None` (no reply is written), while every request with an `id` produces `Some`.

use serde_json::{json, Value};

use crate::protocol::{error, error_code, success, PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION};
use crate::store::{CreateSpec, DocStore, DocSummary, Document, StoreError};
use crate::tools::tool_catalogue;

/// Default result cap for `list-documents` when no `limit` is supplied.
const DEFAULT_LIST_LIMIT: usize = 50;
/// Default result cap for `search-documents` when no `limit` is supplied.
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// Handle one JSON-RPC request against `store`, returning the response to write.
///
/// Returns `None` for notifications (no `id` member) — by JSON-RPC 2.0, those get no
/// reply. Every other request yields `Some(response)`, success or error. Unknown
/// methods and bad parameters become well-formed JSON-RPC error responses rather than
/// errors propagated to the caller, so the loop can never crash on bad input.
pub fn handle_request<S: DocStore>(store: &mut S, request: Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let is_notification = request.get("id").is_none();

    // Reject anything that is not a JSON-RPC 2.0 envelope. A missing/garbage
    // jsonrpc field on a notification is silently dropped (no id to answer).
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        if is_notification {
            return None;
        }
        return Some(error(id, error_code::INVALID_REQUEST, "Invalid Request"));
    }

    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    // Notifications carry no id; we run any side effects but emit no response.
    if is_notification {
        return None;
    }

    let response = match method {
        "initialize" => initialize(id),
        "tools/list" => success(id, json!({ "tools": tool_catalogue() })),
        "tools/call" => handle_tool_call(store, id, &params),
        "resources/list" => resources_list(store, id),
        "resources/read" => resources_read(store, id, &params),
        _ => error(id, error_code::METHOD_NOT_FOUND, "Method not found"),
    };
    Some(response)
}

/// Build the `initialize` handshake result (protocol version, capabilities, info).
fn initialize(id: Value) -> Value {
    success(
        id,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {}, "resources": {} },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        }),
    )
}

/// Dispatch a `tools/call` request to the named tool handler.
fn handle_tool_call<S: DocStore>(store: &mut S, id: Value, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let empty = Value::Object(serde_json::Map::new());
    let args = params.get("arguments").unwrap_or(&empty);

    match name {
        "list-documents" => list_documents(store, id, args),
        "search-documents" => search_documents(store, id, args),
        "get-document" => get_document(store, id, args),
        "create-document" => create_document(store, id, args),
        "save-document" => save_document(store, id, args),
        "ingest-workspace" => ingest_workspace(store, id),
        "maintain-database" => maintain_database(id),
        "open-document-viewer"
        | "interact-document-viewer"
        | "capture-document-view"
        | "use-document-viewer" => unsupported_viewer_tool(id, name),
        other => error(
            id,
            error_code::METHOD_NOT_FOUND,
            format!("Unknown tool: {other}"),
        ),
    }
}

/// Wrap a successful tool result as MCP `content` text, like the reference server.
fn tool_text_result(id: Value, value: &Value) -> Value {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    success(id, json!({ "content": [{ "type": "text", "text": text }] }))
}

/// Map a [`StoreError`] to the JSON-RPC error code the reference server would emit.
fn store_error(id: Value, err: &StoreError) -> Value {
    let code = match err {
        // The reference server reports "Document not found" and bad args as -32602.
        StoreError::NotFound | StoreError::InvalidArgument(_) => error_code::INVALID_PARAMS,
        StoreError::Backend(_) => error_code::INTERNAL_ERROR,
    };
    error(id, code, err.to_string())
}

/// Serialize a list of summaries to the JSON array shape the reference server returns.
fn summaries_to_json(summaries: &[DocSummary]) -> Value {
    Value::Array(
        summaries
            .iter()
            .map(|doc| {
                json!({
                    "id": doc.id,
                    "title": doc.title,
                    "relativePath": doc.relative_path,
                    "updatedAt": doc.updated_at,
                })
            })
            .collect(),
    )
}

/// Serialize a full document to the JSON object shape the reference server returns.
fn document_to_json(doc: &Document) -> Value {
    json!({
        "id": doc.id,
        "title": doc.title,
        "relativePath": doc.relative_path,
        "updatedAt": doc.updated_at,
        "source": doc.source,
    })
}

/// Read an optional positive `limit` argument, falling back to `default`.
fn read_limit(args: &Value, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| n.max(1) as usize)
        .unwrap_or(default)
}

/// `list-documents`: list (optionally query-filtered) document summaries.
fn list_documents<S: DocStore>(store: &S, id: Value, args: &Value) -> Value {
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = read_limit(args, DEFAULT_LIST_LIMIT);
    match store.list(query, limit) {
        Ok(rows) => tool_text_result(id, &summaries_to_json(&rows)),
        Err(err) => store_error(id, &err),
    }
}

/// `search-documents`: require a non-empty query, then return summaries.
fn search_documents<S: DocStore>(store: &S, id: Value, args: &Value) -> Value {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if query.is_empty() {
        return error(id, error_code::INVALID_PARAMS, "query is required");
    }
    let limit = read_limit(args, DEFAULT_SEARCH_LIMIT);
    match store.search(query, limit) {
        Ok(rows) => tool_text_result(id, &summaries_to_json(&rows)),
        Err(err) => store_error(id, &err),
    }
}

/// `get-document`: resolve by `path` or numeric `id` (path takes precedence).
fn get_document<S: DocStore>(store: &S, id: Value, args: &Value) -> Value {
    let result = if let Some(path) = args.get("path").and_then(Value::as_str) {
        store.get_by_path(path)
    } else if let Some(doc_id) = args.get("id").and_then(Value::as_i64) {
        store.get_by_id(doc_id)
    } else {
        return error(
            id,
            error_code::INVALID_PARAMS,
            "Either path or id is required",
        );
    };

    match result {
        Ok(doc) => tool_text_result(id, &document_to_json(&doc)),
        Err(err) => store_error(id, &err),
    }
}

/// `create-document`: create from args, optionally seeding source content.
fn create_document<S: DocStore>(store: &mut S, id: Value, args: &Value) -> Value {
    let tags = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let content = args
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    let spec = CreateSpec {
        path: args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        title: args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        summary: args
            .get("summary")
            .and_then(Value::as_str)
            .map(str::to_string),
        tags,
        content,
    };

    match store.create(spec) {
        Ok(doc) => tool_text_result(id, &document_to_json(&doc)),
        Err(err) => store_error(id, &err),
    }
}

/// `save-document`: persist full `content` for a `path`.
fn save_document<S: DocStore>(store: &mut S, id: Value, args: &Value) -> Value {
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    match store.save(path, content) {
        Ok(doc) => tool_text_result(id, &document_to_json(&doc)),
        Err(err) => store_error(id, &err),
    }
}

/// `ingest-workspace`: re-scan and ingest, returning the store's report.
fn ingest_workspace<S: DocStore>(store: &mut S, id: Value) -> Value {
    match store.ingest() {
        Ok(report) => tool_text_result(id, &report),
        Err(err) => store_error(id, &err),
    }
}

/// `maintain-database`: returns the static "removed" notice from the reference server.
///
/// SQLite maintenance no longer applies (the active engine is bundle + dxlite), so the
/// reference server returns a fixed message regardless of state; we mirror it exactly.
fn maintain_database(id: Value) -> Value {
    tool_text_result(
        id,
        &json!({
            "status": "removed",
            "message": "SQLite maintenance is no longer applicable. Active engine is bundle + dxlite.",
        }),
    )
}

/// Reject the live-viewer/capture tools that require a real webview host.
///
/// These tools are advertised for protocol parity, but driving them needs the VS Code
/// webview and Playwright capture pipeline that the native shell's stub store does not
/// provide. They are wired in a later step alongside the real `doc-core` store.
fn unsupported_viewer_tool(id: Value, name: &str) -> Value {
    error(
        id,
        error_code::INTERNAL_ERROR,
        format!("Tool '{name}' requires the live viewer host, which is not wired in the native shell yet"),
    )
}

/// `resources/list`: two resources per document — source and rendered view.
fn resources_list<S: DocStore>(store: &S, id: Value) -> Value {
    match store.list("", usize::MAX) {
        Ok(rows) => {
            let mut resources = Vec::with_capacity(rows.len() * 2);
            for doc in &rows {
                let label = if doc.title.is_empty() {
                    doc.relative_path.clone()
                } else {
                    doc.title.clone()
                };
                resources.push(json!({
                    "uri": format!("doc:///{}", doc.relative_path),
                    "name": format!("{label} (source)"),
                    "mimeType": "text/plain",
                }));
                resources.push(json!({
                    "uri": format!("docview:///{}", doc.relative_path),
                    "name": format!("{label} (view)"),
                    "mimeType": "text/html",
                }));
            }
            success(id, json!({ "resources": resources }))
        }
        Err(err) => store_error(id, &err),
    }
}

/// `resources/read`: return source for `doc:///` or rendered HTML for `docview:///`.
fn resources_read<S: DocStore>(store: &S, id: Value, params: &Value) -> Value {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    let is_view = uri.starts_with("docview:///");
    let relative = uri
        .strip_prefix("docview:///")
        .or_else(|| uri.strip_prefix("doc:///"))
        .unwrap_or(uri);

    match store.get_by_path(relative) {
        Ok(doc) => {
            if is_view {
                match store.render_html(relative) {
                    Ok(html) => success(
                        id,
                        json!({ "contents": [{ "uri": uri, "mimeType": "text/html", "text": html }] }),
                    ),
                    Err(err) => store_error(id, &err),
                }
            } else {
                success(
                    id,
                    json!({ "contents": [{ "uri": uri, "mimeType": "text/plain", "text": doc.source }] }),
                )
            }
        }
        Err(err) => store_error(id, &err),
    }
}
