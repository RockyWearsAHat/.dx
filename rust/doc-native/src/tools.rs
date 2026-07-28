//! The static MCP tool catalogue.
//!
//! [`tool_catalogue`] returns the full `tools/list` payload, matching the `TOOLS`
//! array in the reference TypeScript server name-for-name and schema-for-schema. The
//! viewer/capture/maintenance tools are advertised for protocol parity even though
//! this shell's stub store does not yet drive a live webview — see the crate report
//! for the parity matrix.

use serde_json::{json, Value};

/// The complete set of tool names this server advertises, in catalogue order.
///
/// Exposed for tests and for callers that need the names without the full schemas.
pub const TOOL_NAMES: &[&str] = &[
    "list-documents",
    "get-document",
    "search-documents",
    "create-document",
    "save-document",
    "open-document-viewer",
    "interact-document-viewer",
    "capture-document-view",
    "use-document-viewer",
    "maintain-database",
    "ingest-workspace",
];

/// The viewer action verbs shared by the interactive viewer tools.
const VIEWER_ACTIONS: &[&str] = &[
    "inspect",
    "click-block",
    "scroll-to",
    "set-block-text",
    "append-paragraph",
    "save",
    "set-view-settings",
    "reset-view-settings",
    "close",
    "undo-state",
    "redo-state",
    "undo-document",
    "redo-document",
];

/// Standard `workspacePath` property shared by most tool schemas.
fn workspace_path() -> Value {
    json!({ "type": "string", "description": "Optional workspace root path" })
}

/// The `oneOf` clause requiring either a `path` or an `id`, shared by lookup tools.
fn path_id_one_of() -> Value {
    json!([
        { "required": ["path"] },
        { "required": ["id"] },
    ])
}

/// Build the `tools/list` result: an array of tool descriptors with input schemas.
///
/// The returned value is stable and self-contained, suitable for direct inclusion in
/// a JSON-RPC success response. Each entry is assembled by a small named builder so the
/// catalogue reads as a list of tools rather than one large literal.
#[must_use]
pub fn tool_catalogue() -> Value {
    json!([
        list_documents_tool(),
        get_document_tool(),
        search_documents_tool(),
        create_document_tool(),
        save_document_tool(),
        open_document_viewer_tool(),
        interact_document_viewer_tool(),
        capture_document_view_tool(),
        use_document_viewer_tool(),
        maintain_database_tool(),
        ingest_workspace_tool(),
    ])
}

/// `list-documents` — list workspace documents, optionally filtered by a query.
fn list_documents_tool() -> Value {
    json!({
        "name": "list-documents",
        "description": "List documents in the workspace database",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "query": { "type": "string", "description": "Optional search query" },
                "limit": { "type": "number", "description": "Maximum results (default 50)" }
            },
            "required": []
        }
    })
}

/// `get-document` — fetch a single document by relative path or numeric id.
fn get_document_tool() -> Value {
    json!({
        "name": "get-document",
        "description": "Get a document by relative path or ID",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "path": { "type": "string", "description": "Workspace-relative .dx path" },
                "id": { "type": "number", "description": "Document ID" }
            },
            "required": [],
            "oneOf": path_id_one_of()
        }
    })
}

/// `search-documents` — full-text search over titles, content, and tags.
fn search_documents_tool() -> Value {
    json!({
        "name": "search-documents",
        "description": "Search documents by title/content/tags",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "number", "description": "Maximum results (default 20)" }
            },
            "required": ["query"]
        }
    })
}

/// `create-document` — create a new document, optionally seeding source content.
fn create_document_tool() -> Value {
    json!({
        "name": "create-document",
        "description": "Create a new document and optionally seed source content",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "path": { "type": "string", "description": "Workspace-relative .dx path (default under documents/)" },
                "title": { "type": "string", "description": "Document title" },
                "summary": { "type": "string", "description": "Document summary" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Document tags" },
                "content": { "type": "string", "description": "Optional DOC source text to save after create" }
            },
            "required": []
        }
    })
}

/// `save-document` — overwrite a document's full source text by path.
fn save_document_tool() -> Value {
    json!({
        "name": "save-document",
        "description": "Save full source text for a document path",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "path": { "type": "string", "description": "Workspace-relative .dx path" },
                "content": { "type": "string", "description": "Full document source text" }
            },
            "required": ["path", "content"]
        }
    })
}

/// `open-document-viewer` — open an interactive viewer session and return its state.
fn open_document_viewer_tool() -> Value {
    json!({
        "name": "open-document-viewer",
        "description": "Open a built-in interactive viewer session for a document and return current view state.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "path": { "type": "string", "description": "Workspace-relative .dx path" },
                "id": { "type": "number", "description": "Document ID" }
            },
            "required": [],
            "oneOf": path_id_one_of()
        }
    })
}

/// `interact-document-viewer` — apply one interaction action to an open viewer session.
fn interact_document_viewer_tool() -> Value {
    json!({
        "name": "interact-document-viewer",
        "description": "Interact with an active document viewer session: inspect, click, scroll, edit block text, append paragraph, and save.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sessionId": { "type": "string", "description": "Viewer session id returned by open-document-viewer" },
                "action": {
                    "type": "string",
                    "enum": VIEWER_ACTIONS,
                    "description": "Interaction action to apply"
                },
                "index": { "type": "number", "description": "Block index for click/set/scroll actions" },
                "text": { "type": "string", "description": "Text payload for set-block-text or append-paragraph" },
                "settings": {
                    "type": "object",
                    "description": "Optional viewer settings patch for set-view-settings",
                    "properties": {
                        "theme": { "type": "string" },
                        "resolvedTheme": { "type": "string" },
                        "appearance": { "type": "object" },
                        "viewport": { "type": "object" },
                        "effectiveCss": { "type": "string" },
                        "sourceHash": { "type": "string" },
                        "editBuffer": { "type": "string" }
                    }
                }
            },
            "required": ["sessionId", "action"]
        }
    })
}

/// `capture-document-view` — render a `.dx` document to a PNG screenshot.
fn capture_document_view_tool() -> Value {
    json!({
        "name": "capture-document-view",
        "description": "Capture a real PNG screenshot for a .dx document rendered in the simple browser workflow.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "path": { "type": "string", "description": "Workspace-relative .dx path" },
                "id": { "type": "number", "description": "Document ID" },
                "size": { "type": "number", "description": "Capture width hint in pixels (default 1492)" }
            },
            "required": [],
            "oneOf": path_id_one_of()
        }
    })
}

/// `use-document-viewer` — single-call open/resume + apply actions + return screenshot.
fn use_document_viewer_tool() -> Value {
    json!({
        "name": "use-document-viewer",
        "description": "Single-call document viewer operation: open/resume a session, apply interactions, and return updated state with rendered screenshot.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path(),
                "path": { "type": "string", "description": "Workspace-relative .dx path" },
                "id": { "type": "number", "description": "Document ID" },
                "sessionId": { "type": "string", "description": "Existing viewer session id. If omitted, a new session opens from path or id." },
                "size": { "type": "number", "description": "Screenshot width hint in pixels (default 1492)" },
                "actions": {
                    "type": "array",
                    "description": "Optional list of actions to run in order. If omitted, defaults to inspect.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "enum": VIEWER_ACTIONS },
                            "index": { "type": "number" },
                            "text": { "type": "string" },
                            "settings": { "type": "object" }
                        },
                        "required": ["action"]
                    }
                }
            },
            "required": []
        }
    })
}

/// `maintain-database` — run storage compaction/maintenance for the workspace.
fn maintain_database_tool() -> Value {
    json!({
        "name": "maintain-database",
        "description": "Run WAL checkpoint(TRUNCATE) + VACUUM maintenance for SQLite compaction.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path()
            },
            "required": []
        }
    })
}

/// `ingest-workspace` — scan workspace `.dx` files and ingest them into the index.
fn ingest_workspace_tool() -> Value {
    json!({
        "name": "ingest-workspace",
        "description": "Scan workspace .dx files and ingest them into SQLite",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspacePath": workspace_path()
            },
            "required": []
        }
    })
}
