//! JSON-RPC 2.0 framing for the MCP server.
//!
//! This module defines the protocol-version constant, the standard error codes, and
//! constructors for the two response shapes (success and error). Requests are parsed
//! loosely from [`serde_json::Value`] rather than a rigid struct so that malformed or
//! partial input can be rejected with a precise JSON-RPC error rather than a parse
//! failure — matching the permissive behaviour of the reference TypeScript server.

use serde_json::{json, Value};

/// MCP protocol version advertised in the `initialize` handshake.
///
/// Must stay byte-identical to `PROTOCOL_VERSION` in `src/mcp-server.ts`.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Server name reported in the `initialize` `serverInfo` block.
pub const SERVER_NAME: &str = "doc-platform-mcp";

/// Server version reported in the `initialize` `serverInfo` block.
pub const SERVER_VERSION: &str = "0.1.0";

/// Standard JSON-RPC 2.0 error codes used by the server.
///
/// The numeric values match the JSON-RPC specification and the reference server.
pub mod error_code {
    /// Invalid JSON was received (the payload could not be parsed).
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON sent is not a valid Request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist or is not available.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameters (also used for not-found documents, per the reference).
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal server error (e.g. a store operation failed unexpectedly).
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// Build a JSON-RPC 2.0 success response carrying `result` for request `id`.
///
/// `id` is echoed verbatim per the specification; pass [`Value::Null`] when the
/// originating request had no id.
#[must_use]
pub fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Build a JSON-RPC 2.0 error response for request `id`.
///
/// The `code`/`message` pair is placed under the `error` member exactly as the
/// reference server emits it.
#[must_use]
pub fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}
