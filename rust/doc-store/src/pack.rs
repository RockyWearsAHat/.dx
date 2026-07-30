//! Pack files: the durable, portable form of the store.
//!
//! The SQLite database is local and rebuildable. These two files are what actually carry
//! documents between machines:
//!
//! - `.doc/repo.dxcp` — repository content. **Commit this.** A fresh clone has the stubs and
//!   this pack, and [`crate::Store::sync`] rebuilds the database from it.
//! - `.doc/local.dxcp` — git-ignored scratch work, so a note in an ignored directory never
//!   reaches a teammate.
//!
//! Both are `DXCP1` containers ([`doc_core::chunk`]): chunk bodies deduplicated across every
//! document in the pack, referenced by index, and `dxz1`-compressed as one stream. Splitting
//! by route rather than writing one pack means committing a document does not rewrite the
//! bytes of unrelated local notes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use doc_core::chunk::{decode_pack, encode_pack, Pack};
use doc_core::model::Document;

use crate::store::STORE_DIR;
use crate::{Store, StoreError};

/// The committed pack, relative to the workspace root.
pub const REPO_PACK: &str = ".doc/repo.dxcp";
/// The git-ignored pack, relative to the workspace root.
pub const LOCAL_PACK: &str = ".doc/local.dxcp";

/// Absolute paths of both packs for the workspace at `root`.
#[must_use]
pub fn paths(root: &Path) -> (PathBuf, PathBuf) {
    (root.join(REPO_PACK), root.join(LOCAL_PACK))
}

/// Write both packs from what `store` holds, splitting documents by git route.
///
/// A pack with no documents is removed rather than written empty, so a workspace with nothing
/// local does not carry a stray file.
pub(crate) fn export(store: &Store) -> Result<(), StoreError> {
    let mut repo: Vec<(String, Document)> = Vec::new();
    let mut local: Vec<(String, Document)> = Vec::new();

    for summary in store.list()? {
        let document = store.document(&summary.path)?;
        if summary.local_only {
            local.push((summary.path, document));
        } else {
            repo.push((summary.path, document));
        }
    }

    let (repo_path, local_path) = paths(store.root());
    write_pack(&repo_path, &repo)?;
    write_pack(&local_path, &local)?;
    Ok(())
}

/// Write one pack file, or remove it when there is nothing to store.
fn write_pack(path: &Path, documents: &[(String, Document)]) -> Result<(), StoreError> {
    if documents.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                StoreError::Backend(format!("could not remove {}: {error}", path.display()))
            })?;
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            StoreError::Backend(format!("could not create {}: {error}", parent.display()))
        })?;
    }

    let pack = Pack::build(
        documents
            .iter()
            .map(|(relative, document)| (relative.as_str(), document)),
    );
    fs::write(path, encode_pack(&pack)).map_err(|error| {
        StoreError::Backend(format!("could not write {}: {error}", path.display()))
    })
}

/// Every document available from either pack, keyed by workspace-relative path.
///
/// The repo pack is read first so a local pack entry wins for the same path — the local copy
/// is the one this machine last worked on. A pack that will not decode is reported rather than
/// skipped: silently ignoring it would look exactly like the document never existing.
pub(crate) fn load_all(root: &Path) -> Result<BTreeMap<String, String>, StoreError> {
    let mut found = BTreeMap::new();
    let (repo_path, local_path) = paths(root);
    for path in [repo_path, local_path] {
        for (relative, source) in read_pack(&path)? {
            found.insert(relative, source);
        }
    }
    Ok(found)
}

/// Read one pack file into `(path, canonical source)` pairs; an absent file yields nothing.
fn read_pack(path: &Path) -> Result<Vec<(String, String)>, StoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(StoreError::Backend(format!(
                "could not read {}: {error}",
                path.display()
            )))
        }
    };

    let pack = decode_pack(&bytes).map_err(|error| {
        StoreError::Corrupt(format!("{} is not readable ({error})", path.display()))
    })?;

    let mut out = Vec::with_capacity(pack.entries.len());
    for entry in &pack.entries {
        let source = pack.source(&entry.path).ok_or_else(|| {
            StoreError::Corrupt(format!(
                "{} is missing chunks for {}",
                path.display(),
                entry.path
            ))
        })?;
        out.push((entry.path.clone(), source));
    }
    Ok(out)
}

/// The `.gitignore` lines a workspace needs so the right things are committed.
///
/// The database and the local pack are machine-local; the repo pack and the stubs are the
/// content and must travel.
#[must_use]
pub fn gitignore_lines() -> String {
    format!(
        "# dx: the local index is rebuildable from {REPO_PACK}\n\
         {STORE_DIR}/index.db\n\
         {STORE_DIR}/index.db-wal\n\
         {STORE_DIR}/index.db-shm\n\
         {LOCAL_PACK}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-pack-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");
        root
    }

    const NOTES: &str = "::heading level=1 id=n\nNotes\n::end\n";

    #[test]
    fn saving_a_document_writes_a_pack_that_round_trips() {
        let root = scratch("export");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");

        let available = load_all(&root).expect("load");
        assert_eq!(available.get("notes.dx").map(String::as_str), Some(NOTES));
    }

    #[test]
    fn an_empty_store_leaves_no_pack_behind() {
        let root = scratch("empty");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");
        store.delete("notes.dx").expect("delete");

        let (repo_path, local_path) = paths(&root);
        assert!(!repo_path.exists(), "empty repo pack should be removed");
        assert!(!local_path.exists());
        assert!(load_all(&root).expect("load").is_empty());
    }

    #[test]
    fn a_damaged_pack_is_reported_rather_than_treated_as_absent() {
        let root = scratch("damaged");
        fs::create_dir_all(root.join(STORE_DIR)).expect("dir");
        fs::write(root.join(REPO_PACK), b"this is not a pack").expect("write");

        let error = load_all(&root).expect_err("should refuse");
        assert!(matches!(error, StoreError::Corrupt(_)), "{error}");
    }

    #[test]
    fn a_missing_pack_is_simply_empty() {
        let root = scratch("absent");
        assert!(load_all(&root).expect("load").is_empty());
    }

    #[test]
    fn the_gitignore_advice_keeps_the_index_out_and_the_content_in() {
        let lines = gitignore_lines();
        assert!(lines.contains("index.db"));
        assert!(lines.contains(LOCAL_PACK));
        assert!(
            !lines.contains(&format!("\n{REPO_PACK}\n")),
            "the committed pack must not be ignored: {lines}"
        );
    }

    #[test]
    fn many_documents_share_chunk_bodies_inside_one_pack() {
        let root = scratch("sharing");
        let mut store = Store::open(&root).expect("open");
        let shared = "::paragraph id=p\nthe very same paragraph text\n::end\n";
        for name in ["a.dx", "b.dx", "c.dx"] {
            store.ingest(name, shared).expect("ingest");
        }

        let bytes = fs::read(root.join(REPO_PACK)).expect("read pack");
        assert!(
            bytes.len() < shared.len() * 3,
            "pack of 3 identical documents ({} bytes) should beat 3 copies ({} bytes)",
            bytes.len(),
            shared.len() * 3
        );
        let available = load_all(&root).expect("load");
        assert_eq!(available.len(), 3);
        assert!(available.values().all(|source| source == shared));
    }

    #[test]
    fn parse_is_reachable_for_pack_sources() {
        // The pack stores canonical text, so a caller can parse it straight back.
        let root = scratch("parseable");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");
        let source = load_all(&root)
            .expect("load")
            .remove("notes.dx")
            .expect("entry");
        assert_eq!(parse(&source).blocks.len(), 1);
    }
}
