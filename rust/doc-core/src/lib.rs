//! `doc-core` — the runtime-agnostic engine for the DOC platform.
//!
//! This crate is pure Rust with no operating-system dependencies, so the exact same
//! code compiles to a native binary (the CLI and MCP server) and to `wasm32` (the
//! in-editor block editor). It owns every deterministic concern of the platform:
//!
//! - [`digest`] — SHA-256 / SHA-1 hex digests, byte-identical to the reference spec.
//! - [`compress`] — the `dxz1` LZSS byte codec used for chunk storage.
//! - [`format`] — DOCSRC (`.dx`) parsing and canonical stringify.
//! - [`chunk`] — content-addressed per-block chunks and the `DXCP1` pack container.
//! - [`merge`] — the block-keyed three-way merge two branches of a document reconcile through.
//! - [`search`] — the in-memory token search index.
//! - [`trace`] — the heuristic symbol + reference index.
//! - [`render`] — the document views: themed HTML, Markdown, outlines, and sections.
//!
//! # Storage is lossless by construction
//! A chunk holds the exact canonical text [`format::stringify`] would write for one block,
//! so reassembly returns the file byte-for-byte and no field can be dropped by a codec
//! that had not heard of it. Any encoding that understands only *some* block attributes
//! has no place here.
//!
//! # Design rules
//! The crate forbids `unsafe` code and documents every public item. Fallible
//! operations return [`Result`] rather than panicking, so host shells (native or
//! wasm) can surface errors instead of aborting.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod base64;
pub mod chunk;
pub mod compress;
pub mod digest;
pub mod edit;
pub mod format;
pub mod merge;
pub mod model;
pub mod pointer;
pub mod render;
pub mod resolve;
pub mod search;
pub mod source_index;
pub mod surface;
pub mod trace;
