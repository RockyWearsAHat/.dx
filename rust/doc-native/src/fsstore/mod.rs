//! A filesystem-backed [`DocStore`] wiring the `doc-core` engine into the MCP shell.
//!
//! [`FsDocStore`] is the real, OS-backed store: it persists documents as packed blocks
//! inside two bundle archives under `.doc/` and writes tiny `.dx` stub pointers on disk,
//! exactly like the reference TypeScript service (`src/doc-service.ts`,
//! `src/doc-archive.ts`). Because this is the *native host* layer, using [`std::fs`] and
//! [`std::process::Command`] (for git) is appropriate here; `doc-core` itself stays free
//! of OS dependencies so it can also target `wasm32`.
//!
//! # Module layout
//! - [`git`] — git-driven routing (repo vs local bundle) and the `Route` archive map.
//! - [`render`] — best-effort HTML rendering for the `docview://` resource.
//! - [`stub`] — stub recognition, path normalization, identity, and timestamp helpers.
//!
//! # Storage model (mirrors the reference)
//! - `.dx` files on disk are **stubs/pointers** (the tiny `~` form), never content.
//! - Canonical content is packed [`doc_core::model::Document`] blocks held inside two
//!   bundle archives:
//!   - `.doc/.repo-docs.bin` — repo-tracked documents (commit this).
//!   - `.doc/.local-docs.bin` — local-only documents (ignored/untracked/de-tracked).
//! - A document's bundle is chosen from its git state (see [`FsDocStore::route`]).
//!
//! # Git routing
//! A document is **local-only** when it is ignored, not tracked, or removed from the
//! index — matching `localOnlyArchive = ignored || !tracked || removedFromIndex` in
//! `src/git-doc-state.ts`. Otherwise it is repo-tracked. When git is unavailable the
//! store defaults to the repo bundle, again matching the reference.
//!
//! # Identity
//! A document's numeric id is derived from its workspace-relative path the same way the
//! reference `toStableDocumentId` does: the first 8 hex chars of `sha1(relativePath)`
//! masked to 31 bits (and floored to `1`), so ids are stable across runs without a
//! counter.

mod api;
mod git;
mod render;
mod stub;

use std::fs;
use std::path::{Component, Path, PathBuf};

use doc_core::bundle::{decode_bundle, encode_bundle, BundleEntry};
use doc_core::docbin::{pack, unpack};
use doc_core::format::stringify;
use doc_core::model::Document as CoreDocument;

use crate::store::{Document, StoreError};

use stub::{derive_title, iso8601_from_system_time, stable_document_id};

/// Workspace-relative path of the repo-tracked bundle archive.
const REPO_ARCHIVE_RELATIVE: &str = ".doc/.repo-docs.bin";
/// Workspace-relative path of the local-only bundle archive.
const LOCAL_ARCHIVE_RELATIVE: &str = ".doc/.local-docs.bin";
/// Tiny stub marker written into on-disk `.dx` files (content lives in the bundle).
const STUB_TINY_PREFIX: &str = "~";
/// Directory names skipped while walking for `.dx` files, matching `file-discovery.ts`.
const IGNORED_DIRECTORIES: &[&str] = &[".git", ".github", ".vscode", "data", "node_modules"];
/// Deterministic placeholder timestamp used when a file's mtime is unavailable.
const FALLBACK_TIMESTAMP: &str = "1970-01-01T00:00:00.000Z";

/// Which bundle archive a document belongs in, decided by git state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// The repo-tracked archive (`.doc/.repo-docs.bin`).
    Repo,
    /// The local-only archive (`.doc/.local-docs.bin`).
    Local,
}

/// A document resolved out of a bundle: its parsed core document plus its path key.
pub(super) struct LoadedDoc {
    /// Workspace-relative `.dx` path (the bundle entry key).
    pub(super) relative_path: String,
    /// The parsed `doc-core` document (canonical blocks).
    pub(super) document: CoreDocument,
}

/// A filesystem-backed [`DocStore`] persisting to `.doc/*.bin` bundles and `.dx` stubs.
///
/// Construct with [`FsDocStore::new`] over a workspace root directory. All paths handed
/// to and from this store are workspace-relative; absolute paths are derived internally.
pub struct FsDocStore {
    /// Absolute workspace root directory.
    pub(super) root: PathBuf,
}

impl FsDocStore {
    /// Create a store rooted at `root` (the workspace directory). The directory need not
    /// exist yet; it is created on first write.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Absolute path of the bundle archive for `route`.
    fn archive_path(&self, route: Route) -> PathBuf {
        self.root.join(route.archive_relative())
    }

    /// Read and decode a bundle archive, returning its entries (empty when absent).
    ///
    /// A missing archive is normal (a fresh workspace), so it yields no entries rather
    /// than an error; a present-but-corrupt archive surfaces as [`StoreError::Backend`].
    fn read_bundle(&self, route: Route) -> Result<Vec<BundleEntry>, StoreError> {
        let path = self.archive_path(route);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(StoreError::Backend(format!(
                    "failed to read {}: {err}",
                    path.display()
                )))
            }
        };
        decode_bundle(&bytes)
            .map_err(|err| StoreError::Backend(format!("failed to decode bundle: {err}")))
    }

    /// Encode `entries` and write them to the archive for `route`, creating `.doc/`.
    fn write_bundle(&self, route: Route, entries: &[BundleEntry]) -> Result<(), StoreError> {
        let path = self.archive_path(route);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                StoreError::Backend(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        let bytes = encode_bundle(entries);
        fs::write(&path, bytes).map_err(|err| {
            StoreError::Backend(format!("failed to write {}: {err}", path.display()))
        })
    }

    /// Load every document from both bundles, unpacking each entry.
    ///
    /// Entries that fail to unpack are skipped rather than aborting the whole load, so a
    /// single corrupt blob never blocks listing the rest of the workspace. Ordering is
    /// repo entries first, then local, each in bundle order.
    pub(super) fn load_all(&self) -> Result<Vec<LoadedDoc>, StoreError> {
        let mut loaded = Vec::new();
        for route in [Route::Repo, Route::Local] {
            for entry in self.read_bundle(route)? {
                if let Ok(document) = unpack(&entry.packed) {
                    loaded.push(LoadedDoc {
                        relative_path: entry.path,
                        document,
                    });
                }
            }
        }
        Ok(loaded)
    }

    /// Project a [`LoadedDoc`] into the protocol [`Document`] (full record with source).
    pub(super) fn to_full(&self, loaded: &LoadedDoc) -> Document {
        Document {
            id: stable_document_id(&loaded.relative_path),
            title: derive_title(&loaded.document, &loaded.relative_path),
            relative_path: loaded.relative_path.clone(),
            updated_at: self.updated_at(&loaded.relative_path),
            source: stringify(&loaded.document),
        }
    }

    /// Read the on-disk `.dx` stub's mtime as an ISO-8601 timestamp, or a fixed fallback.
    ///
    /// The reference derives `updatedAt` from the stub file's modification time; when the
    /// file is absent or its time is unreadable we fall back to a fixed value so the field
    /// is always populated.
    fn updated_at(&self, relative_path: &str) -> String {
        let path = self.root.join(relative_path);
        match fs::metadata(&path).and_then(|meta| meta.modified()) {
            Ok(time) => iso8601_from_system_time(time),
            Err(_) => FALLBACK_TIMESTAMP.to_string(),
        }
    }

    /// Persist `document` at `relative_path`: route via git, write its bundle entry, prune
    /// any stale copy from the other bundle, and write the on-disk `.dx` stub.
    pub(super) fn persist(
        &self,
        relative_path: &str,
        document: &CoreDocument,
    ) -> Result<Document, StoreError> {
        let (route, git) = self.route(relative_path);
        let packed = pack(document);

        // Write/overwrite the entry in the chosen bundle, preserving other entries' order.
        let mut entries = self.read_bundle(route)?;
        let new_entry = BundleEntry {
            path: relative_path.to_string(),
            git,
            packed,
        };
        match entries.iter_mut().find(|e| e.path == relative_path) {
            Some(existing) => *existing = new_entry,
            None => entries.push(new_entry),
        }
        self.write_bundle(route, &entries)?;

        // Prune any stale copy from the *other* bundle so a document lives in exactly one.
        let other = match route {
            Route::Repo => Route::Local,
            Route::Local => Route::Repo,
        };
        let mut other_entries = self.read_bundle(other)?;
        let before = other_entries.len();
        other_entries.retain(|e| e.path != relative_path);
        if other_entries.len() != before {
            self.write_bundle(other, &other_entries)?;
        }

        // Write the tiny on-disk stub pointer (the `.dx` file never holds content).
        self.write_stub(relative_path)?;

        Ok(Document {
            id: stable_document_id(relative_path),
            title: derive_title(document, relative_path),
            relative_path: relative_path.to_string(),
            updated_at: self.updated_at(relative_path),
            source: stringify(document),
        })
    }

    /// Write the tiny `~` stub to the on-disk `.dx` file, creating parent directories.
    fn write_stub(&self, relative_path: &str) -> Result<(), StoreError> {
        let path = self.root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                StoreError::Backend(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        fs::write(&path, STUB_TINY_PREFIX).map_err(|err| {
            StoreError::Backend(format!("failed to write stub {}: {err}", path.display()))
        })
    }

    /// Recursively collect workspace-relative `.dx` paths, skipping ignored directories.
    pub(super) fn collect_dx_files(&self) -> Result<Vec<String>, StoreError> {
        let mut out = Vec::new();
        self.walk_dx(&self.root, &mut out)?;
        out.sort();
        Ok(out)
    }

    /// Depth-first walk helper for [`Self::collect_dx_files`].
    fn walk_dx(&self, dir: &Path, out: &mut Vec<String>) -> Result<(), StoreError> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(StoreError::Backend(format!(
                    "failed to read dir {}: {err}",
                    dir.display()
                )))
            }
        };
        for entry in entries {
            let entry = entry
                .map_err(|err| StoreError::Backend(format!("failed to read dir entry: {err}")))?;
            let file_type = entry
                .file_type()
                .map_err(|err| StoreError::Backend(format!("failed to stat entry: {err}")))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if IGNORED_DIRECTORIES.contains(&name.as_ref()) {
                    continue;
                }
                self.walk_dx(&entry.path(), out)?;
            } else if file_type.is_file() && name.ends_with(".dx") {
                if let Some(relative) = self.to_relative(&entry.path()) {
                    out.push(relative);
                }
            }
        }
        Ok(())
    }

    /// Convert an absolute path under the root to a `/`-separated workspace-relative path.
    fn to_relative(&self, absolute: &Path) -> Option<String> {
        let relative = absolute.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                // Reject any traversal that would escape the root.
                Component::ParentDir => return None,
                _ => {}
            }
        }
        Some(parts.join("/"))
    }

    /// Resolve a full [`Document`] for `relative_path` from whichever bundle holds it.
    pub(super) fn find_full(&self, relative_path: &str) -> Result<Document, StoreError> {
        let loaded = self.load_all()?;
        loaded
            .iter()
            .find(|doc| doc.relative_path == relative_path)
            .map(|doc| self.to_full(doc))
            .ok_or(StoreError::NotFound)
    }

    /// Resolve the *core* document at `relative_path` from either bundle, if present.
    pub(super) fn find_core(
        &self,
        relative_path: &str,
    ) -> Result<Option<CoreDocument>, StoreError> {
        let loaded = self.load_all()?;
        Ok(loaded
            .into_iter()
            .find(|doc| doc.relative_path == relative_path)
            .map(|doc| doc.document))
    }
}
