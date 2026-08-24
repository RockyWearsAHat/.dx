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
//! document in the pack, referenced by index, and wrapped in one `dxz` frame whose magic
//! names its codec (`DXZ1` frames from older builds are decoded forever). Splitting by route
//! rather than writing one pack means committing a document does not rewrite the bytes of
//! unrelated local notes.
//!
//! The two packs are written under different policies, because they have different readers —
//! and the policy follows each file's **git status, never its name** ([`storage_for`]). A pack
//! git carries in history is written plain, because git deltas plain bytes between revisions
//! while it cannot delta a compressed stream; so the committed pack is deliberately the larger
//! file on disk and the smaller one in history. A pack git ignores, or any pack in a directory
//! that is not a repository, has no history to delta and is simply compressed.
//! [`doc_core::chunk::PackStorage`] carries the reasoning and `validation.dx#git-cost-holds`
//! carries the measurement.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use doc_core::chunk::{decode_pack, encode_pack_for, Pack, PackStorage};
use doc_core::model::Document;

use crate::store::STORE_DIR;
use crate::{git, Store, StoreError};

/// The committed pack, relative to the workspace root.
pub const REPO_PACK: &str = ".doc/repo.dxcp";
/// The git-ignored pack, relative to the workspace root.
pub const LOCAL_PACK: &str = ".doc/local.dxcp";

/// Absolute paths of both packs for the workspace at `root`.
#[must_use]
pub fn paths(root: &Path) -> (PathBuf, PathBuf) {
    (root.join(REPO_PACK), root.join(LOCAL_PACK))
}

/// Whether an export may write packs that no longer carry a document the tree still points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loss {
    /// Normal operation. An export takes the index as the whole truth, so an index that has
    /// lost documents writes a pack that has lost them too — which is exactly how a
    /// half-built index turns into missing content. Refuse, and name the repair.
    Refuse,
    /// A repair in progress ([`Store::sync`]). There, a pointer with no index row is the
    /// thing being repaired and the packs are what it is repaired *from*, so an export that
    /// lands mid-restore is expected to be short.
    Expected,
}

/// How a pack file must be encoded, decided by what git will do with it.
///
/// The names `repo.dxcp` and `local.dxcp` are a convention, and a convention is not a policy:
/// a workspace that commits its local pack would otherwise keep paying for a compressed stream
/// git cannot delta, silently. So the question asked is [`git::tracks`] — will history carry
/// these bytes? — and the answer picks the encoding.
fn storage_for(root: &Path, relative: &str) -> PackStorage {
    if git::tracks(root, relative) {
        PackStorage::ForVersionControl
    } else {
        PackStorage::Compressed
    }
}

/// Write both packs from what `store` holds, splitting documents by git route.
///
/// A pack with no documents is removed rather than written empty, so a workspace with nothing
/// local does not carry a stray file. A pack whose bytes would be unchanged is left alone.
pub(crate) fn export(store: &Store, loss: Loss) -> Result<(), StoreError> {
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

    let root = store.root();
    let (repo_path, local_path) = paths(root);
    if loss == Loss::Refuse {
        let keeping: BTreeSet<&str> = repo
            .iter()
            .chain(local.iter())
            .map(|(relative, _)| relative.as_str())
            .collect();
        refuse_to_drop(store, &[&repo_path, &local_path], &keeping)?;
    }

    write_pack(&repo_path, &repo, storage_for(root, REPO_PACK))?;
    write_pack(&local_path, &local, storage_for(root, LOCAL_PACK))?;
    Ok(())
}

/// Fail if any pack carries a document `keeping` does not, while its `.dx` file is still there.
///
/// Both packs are checked against the *whole* store rather than pack by pack, because a
/// document whose git status changed legitimately moves from one pack to the other.
fn refuse_to_drop(
    store: &Store,
    packs: &[&PathBuf],
    keeping: &BTreeSet<&str>,
) -> Result<(), StoreError> {
    for path in packs {
        for (relative, _) in read_pack(path)? {
            if !keeping.contains(relative.as_str()) && store.stub_path(&relative).exists() {
                return Err(StoreError::WouldLose(relative));
            }
        }
    }
    Ok(())
}

/// Write one pack file, or remove it when there is nothing to store; `true` when the bytes on
/// disk changed.
///
/// Rewriting a file with the bytes it already holds is not free — it churns mtimes, which is
/// what a watcher and a build cache key off — so the encoded bytes are compared first. That
/// comparison is also what lets [`Store::sync`] notice a pack whose *encoding policy* changed
/// while no document did.
fn write_pack(
    path: &Path,
    documents: &[(String, Document)],
    storage: PackStorage,
) -> Result<bool, StoreError> {
    if documents.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                StoreError::Backend(format!("could not remove {}: {error}", path.display()))
            })?;
            return Ok(true);
        }
        return Ok(false);
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
    let bytes = encode_pack_for(&pack, storage);
    if fs::read(path).is_ok_and(|found| found == bytes) {
        return Ok(false);
    }
    fs::write(path, bytes)
        .map_err(|error| {
            StoreError::Backend(format!("could not write {}: {error}", path.display()))
        })
        .map(|()| true)
}

/// The bytes both packs hold right now, in the order [`paths`] names them.
///
/// [`Store::sync`] takes this before and after reconciling: comparing the two is the only
/// honest way to report *the packs were rewritten*, since the writes happen in the middle of
/// the repair rather than at the end of it.
pub(crate) fn snapshot(root: &Path) -> [Option<Vec<u8>>; 2] {
    let (repo_path, local_path) = paths(root);
    [fs::read(repo_path).ok(), fs::read(local_path).ok()]
}

/// The pack names [`snapshot`] reports, in the same order.
pub(crate) const NAMES: [&str; 2] = [REPO_PACK, LOCAL_PACK];

/// Every document available from either pack, keyed by workspace-relative path.
///
/// The repo pack is read first so a local pack entry wins for the same path — the local copy
/// is the one this machine last worked on. A pack that will not decode is reported rather than
/// skipped: silently ignoring it would look exactly like the document never existing.
pub fn load_all(root: &Path) -> Result<BTreeMap<String, String>, StoreError> {
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
    decode(&bytes, &path.display().to_string())
}

/// Decode pack `bytes` into `(path, canonical source)` pairs, naming `label` in any failure.
///
/// The bytes are the unit rather than a file because a merge reads two of the three sides
/// out of git rather than off disk ([`crate::merge`]) — a pack that only ever exists as a
/// blob still has to decode by exactly the same rules as one on disk.
///
/// # Errors
/// [`StoreError::Corrupt`] when the container will not decode, or when it names a document
/// whose chunks it does not carry.
pub fn decode(bytes: &[u8], label: &str) -> Result<Vec<(String, String)>, StoreError> {
    let pack = decode_pack(bytes)
        .map_err(|error| StoreError::Corrupt(format!("{label} is not readable ({error})")))?;

    let mut out = Vec::with_capacity(pack.entries.len());
    for entry in &pack.entries {
        let source = pack.source(&entry.path).ok_or_else(|| {
            StoreError::Corrupt(format!("{label} is missing chunks for {}", entry.path))
        })?;
        out.push((entry.path.clone(), source));
    }
    Ok(out)
}

/// The canonical source of one document as the packs hold it, or `None` when neither pack
/// carries it.
///
/// This is the read path that needs no database: it lets a reader resolve a stub in a fresh
/// clone — or any workspace whose index has not been built — without creating anything on
/// disk. Reading a document must never have a side effect.
pub fn source(root: &Path, relative: &str) -> Result<Option<String>, StoreError> {
    Ok(load_all(root)?.remove(relative))
}

/// The `.gitignore` lines a workspace needs so the right things are committed.
///
/// The database, its write-ahead log, the local pack, and the search-coverage log are all
/// machine-local; the repo pack and the stubs are the content and must travel. Committing a
/// machine-local file is not a tidiness problem but a merge one: `.doc/index.db` is binary and
/// every branch that wrote a document has its own, so a tracked index conflicts on every
/// single merge and no resolution of it can be right.
#[must_use]
pub fn gitignore_lines() -> String {
    format!(
        "# dx: the local index is rebuildable from {REPO_PACK}\n\
         {STORE_DIR}/index.db\n\
         {STORE_DIR}/index.db-wal\n\
         {STORE_DIR}/index.db-shm\n\
         {STORE_DIR}/coverage.jsonl\n\
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
    fn a_document_in_a_git_repository_lands_in_the_committed_pack() {
        // The whole point of the repo pack: commit the pointers and the content travels with
        // them. A document whose content only reached the ignored pack would arrive at a clone
        // as a pointer with nothing behind it.
        let root = scratch("committed");
        if !std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .is_ok_and(|status| status.success())
        {
            return; // No git available; the routing test covers the decision itself.
        }

        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");

        let (repo_path, _) = paths(&root);
        assert!(
            repo_path.exists(),
            "a new document must reach the pack that gets committed"
        );
        let pack = decode_pack(&fs::read(&repo_path).expect("read")).expect("decode");
        assert_eq!(pack.source("notes.dx").as_deref(), Some(NOTES));
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
    fn a_pack_git_does_not_carry_is_compressed_and_one_it_carries_is_not() {
        // The policy follows git status, not the file's name: a workspace with no git has no
        // history to delta, so both packs take the smaller form.
        let root = scratch("policy");
        assert_eq!(storage_for(&root, REPO_PACK), PackStorage::Compressed);
        assert_eq!(storage_for(&root, LOCAL_PACK), PackStorage::Compressed);

        if !std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .is_ok_and(|status| status.success())
        {
            return;
        }
        fs::write(root.join(".gitignore"), format!("{LOCAL_PACK}\n")).expect("ignore");
        assert_eq!(
            storage_for(&root, REPO_PACK),
            PackStorage::ForVersionControl,
            "a committed pack must stay delta-able"
        );
        assert_eq!(
            storage_for(&root, LOCAL_PACK),
            PackStorage::Compressed,
            "an ignored pack has no history to delta"
        );
    }

    #[test]
    fn writing_a_pack_whose_bytes_are_unchanged_touches_nothing() {
        // The property `dx sync` leans on: re-exporting is free, so it can export every time
        // and still notice the one case where the bytes genuinely differ.
        let root = scratch("idempotent");
        // Long enough that the two policies genuinely differ: a 35-byte pack compresses to
        // itself, and a test that cannot tell the policies apart proves nothing.
        let long = format!(
            "::paragraph id=p\n{}\n::end\n",
            "the very same sentence, over and over. ".repeat(60)
        );
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", &long).expect("ingest");

        let documents = vec![("notes.dx".to_string(), parse(&long))];
        let (repo_path, _) = paths(&root);
        let storage = storage_for(&root, REPO_PACK);
        assert_eq!(storage, PackStorage::Compressed, "no git here");
        assert!(
            !write_pack(&repo_path, &documents, storage).expect("write"),
            "identical bytes must not be rewritten"
        );
        assert!(
            write_pack(&repo_path, &documents, PackStorage::ForVersionControl).expect("write"),
            "a changed encoding policy must reach the file"
        );
        assert!(
            load_all(&root).expect("load").contains_key("notes.dx"),
            "and the rewritten pack still decodes"
        );
    }

    #[test]
    fn an_export_refuses_to_drop_a_document_the_tree_still_points_at() {
        // Part 66's failure, made impossible: the index lost a document while its pointer
        // stayed on disk, and the next export wrote a pack without it.
        let root = scratch("wouldlose");
        let mut store = Store::open(&root).expect("open");
        store.ingest("keep.dx", NOTES).expect("keep");
        store.ingest("lost.dx", NOTES).expect("lost");

        // Simulate an index that has lost a row while the pointer file survives — exactly
        // what a half-applied migration left behind.
        rusqlite::Connection::open(root.join(".doc/index.db"))
            .expect("index")
            .execute("DELETE FROM documents WHERE path = 'lost.dx'", [])
            .expect("forget");
        assert!(store.stub_path("lost.dx").exists());

        let error = export(&store, Loss::Refuse).expect_err("must refuse");
        assert!(
            matches!(&error, StoreError::WouldLose(path) if path == "lost.dx"),
            "{error}"
        );
        assert!(
            error.to_string().contains("dx sync"),
            "the refusal must name the repair: {error}"
        );

        // The pack is untouched, so the document is still there to be restored.
        assert!(load_all(&root).expect("load").contains_key("lost.dx"));
        // And a repair may export freely — that is what puts the row back.
        export(&store, Loss::Expected).expect("a repair exports");
    }

    #[test]
    fn a_deliberate_deletion_is_not_a_loss() {
        // `dx rm` removes the pointer first, so nothing points at the document any more and
        // the export must go through. The guard has to tell the two apart.
        let root = scratch("deleted");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");
        store.delete("notes.dx").expect("delete");
        assert!(load_all(&root).expect("load").is_empty());
    }

    #[test]
    fn a_document_that_changes_pack_is_not_read_as_a_loss() {
        // A file that becomes git-ignored moves from the repo pack to the local one. It left
        // one pack, but it did not leave the store, so the guard must not fire.
        let root = scratch("moved");
        let mut store = Store::open(&root).expect("open");
        store.ingest("notes.dx", NOTES).expect("ingest");
        let (repo_path, _) = paths(&root);
        assert!(repo_path.exists());

        // Whichever pack it is in now, exporting again with the guard on must be fine.
        export(&store, Loss::Refuse).expect("no loss");
        assert!(load_all(&root).expect("load").contains_key("notes.dx"));
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
