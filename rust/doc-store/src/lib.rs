//! `doc-store` — the SQLite-backed chunk store, and the resolver over it.
//!
//! # What this crate is for
//! A `.dx` file on disk is a one-line [`stub`] pointer, not content. The content lives here,
//! in a local SQLite database, as content-addressed compressed chunks. Every read anyone
//! performs — a person opening the editor, `dx render`, an agent calling `dx_read` — goes
//! through [`Store`] and gets the true document back. That is the whole contract: **the
//! store always resolves to the real thing, for humans and agents alike.**
//!
//! # Why a database rather than text on disk
//! Chunks are shared. A block that appears in two documents, or that survives an edit to its
//! neighbours, is stored once ([`doc_core::chunk`]). Sections and a token index are kept
//! alongside, so asking for one section or searching the corpus never means decompressing
//! every document. Text files can do none of that and cost their full size every copy.
//!
//! # The two artifacts
//! - `.doc/index.db` — this store: the local authority every read goes through. Rebuildable,
//!   and therefore not committed.
//! - `.doc/repo.dxcp` / `.doc/local.dxcp` — [`pack`] exports. The repo pack is the committed
//!   artifact that carries documents to a fresh clone; the local pack holds git-ignored
//!   scratch work. [`Store::sync`] rebuilds the database from them when it is missing.
//!
//! Nothing here is allowed to lose a byte: a document's chunks reassemble to its exact
//! canonical source, and [`Store::source`] verifies the result against the digest recorded in
//! the stub before returning it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod git;
pub mod pack;
mod schema;
mod store;
pub mod stub;

pub use store::{Saved, Stats, Store, Summary, SyncReport};

use core::fmt;

/// Everything that can go wrong talking to the store.
///
/// The split matters to callers: a [`NotFound`](StoreError::NotFound) is a normal answer to
/// a question about a document that is not there, an
/// [`InvalidPath`](StoreError::InvalidPath) is the caller's mistake, and the rest are
/// failures to report with advice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// No document is stored at the given path.
    NotFound(String),
    /// The supplied path was empty or tried to escape the workspace.
    InvalidPath(String),
    /// A document's chunks are missing or do not match the digest recorded for it.
    Corrupt(String),
    /// The database or filesystem refused an operation.
    Backend(String),
}

impl StoreError {
    /// Wrap a backend failure (SQLite, filesystem) as a [`StoreError::Backend`].
    pub(crate) fn backend(error: impl fmt::Display) -> Self {
        Self::Backend(error.to_string())
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => {
                write!(f, "no document at {path}; `dx ls` shows what is stored")
            }
            Self::InvalidPath(path) => write!(
                f,
                "{path} is not a usable document path; give a path inside the workspace"
            ),
            Self::Corrupt(detail) => write!(
                f,
                "{detail}; run `dx sync` to rebuild the store from the packs in .doc/"
            ),
            Self::Backend(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for StoreError {}
