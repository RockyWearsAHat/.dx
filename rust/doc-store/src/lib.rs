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

pub use store::{
    discover as discover_documents, stale_index, timestamp, Saved, Stats, Store, Summary,
    SyncReport,
};

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
    /// The path enters a directory document discovery never walks (`build`, `fixtures`,
    /// a dotted name, …), so a document stored there would exist but never be listed —
    /// the ghost-document symptom by another door.
    Unlistable {
        /// The refused document path.
        path: String,
        /// The path segment discovery would never walk into.
        directory: String,
    },
    /// A document's chunks are missing or do not match the digest recorded for it.
    Corrupt(String),
    /// Writing the packs from what the index holds would drop a document some `.dx` pointer
    /// still names — the index has lost it, and exporting would make that loss durable.
    WouldLose(String),
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
            Self::Unlistable { path, directory } => write!(
                f,
                "{path} sits under `{directory}/`, which document discovery never walks — \
                 stored there it would never appear in a listing again; save it outside \
                 `{directory}/`"
            ),
            Self::Corrupt(detail) => write!(
                f,
                "{detail}; run `dx sync` to rebuild the store from the packs in .doc/"
            ),
            Self::WouldLose(path) => write!(
                f,
                "the index no longer holds {path}, but the file still points at it — writing \
                 the packs now would lose the document; run `dx sync` to restore it from \
                 .doc/repo.dxcp first"
            ),
            Self::Backend(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for StoreError {}
