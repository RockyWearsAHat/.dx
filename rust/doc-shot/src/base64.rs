//! Base64 for this crate's callers — a re-export of the platform's one copy.
//!
//! The codec itself lives in [`doc_core::base64`], where hydration uses it to embed
//! `::image` files as `data:` URIs. This module keeps the paths the MCP server (encoding
//! captured images) and the DevTools client (decoding browser frames) have always used.

pub use doc_core::base64::{decode, encode};
