//! `doc-native` — the native MCP server shell for the DOC platform.
//!
//! This crate implements the stdio [JSON-RPC 2.0] protocol layer that exposes DOC
//! document operations to AI agents, mirroring the behaviour of the reference
//! TypeScript server (`src/mcp-server.ts`). It is the *host* layer: it owns the
//! wire protocol, request dispatch, and the tool/resource catalogue, but delegates
//! every document operation to a [`DocStore`] implementation.
//!
//! # Layering
//! - [`protocol`] — JSON-RPC 2.0 request/response framing and error codes.
//! - [`tools`] — the static MCP tool catalogue (names + input schemas).
//! - [`store`] — the [`DocStore`] trait plus an in-memory stub implementation.
//! - [`fsstore`] — [`FsDocStore`], the real filesystem-backed store wiring `doc-core`
//!   (bundle archives + dxlite-equivalent search + `.dx` stubs) into the protocol.
//! - [`dispatch`] — [`dispatch::handle_request`], the pure, testable core that maps
//!   one parsed JSON-RPC value to an optional response value.
//! - [`server`] — the newline-delimited stdio read/dispatch/write loop.
//!
//! The wire framing is **newline-delimited JSON-RPC**: exactly one JSON object per
//! line on stdin, exactly one response object per line on stdout, matching the
//! reference server's `readline`-over-stdin model.
//!
//! [JSON-RPC 2.0]: https://www.jsonrpc.org/specification
//!
//! # Design rules
//! The crate forbids `unsafe` code, documents every public item, and never panics in
//! request handling: malformed input and store failures surface as JSON-RPC error
//! responses, not aborts.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod dispatch;
pub mod fsstore;
pub mod protocol;
pub mod server;
pub mod store;
pub mod tools;

pub use dispatch::handle_request;
pub use fsstore::FsDocStore;
pub use store::{DocStore, DocSummary, Document, MemoryStore, StoreError};
