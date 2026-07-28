//! `doc-core` — the runtime-agnostic engine for the DOC platform.
//!
//! This crate is pure Rust with no operating-system dependencies, so the exact same
//! code compiles to a native binary (the CLI and MCP server) and to `wasm32` (the
//! in-editor block editor). It owns every deterministic concern of the platform:
//!
//! - [`digest`] — SHA-256 / SHA-1 hex digests, byte-identical to the reference spec.
//! - [`compress`] — the `dxz1` LZSS byte codec used for bundle storage.
//! - [`docbin`] — the DOCB1 binary document codec.
//! - [`format`] — DOCSRC (`.dx`) parsing and canonical stringify.
//! - [`bundle`] — the DXBUN5 archive container that stores many packed documents.
//! - [`search`] — the dxlite-equivalent in-memory token search index.
//! - [`render`] — the document views: themed HTML, Markdown, outlines, and sections.
//!
//! # Design rules
//! The crate forbids `unsafe` code and documents every public item. Fallible
//! operations return [`Result`] rather than panicking, so host shells (native or
//! wasm) can surface errors instead of aborting.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod bundle;
pub mod compress;
pub mod digest;
pub mod docbin;
pub mod format;
pub mod model;
pub mod render;
pub mod search;
