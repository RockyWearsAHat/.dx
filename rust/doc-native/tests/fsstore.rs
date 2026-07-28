//! Integration tests for the filesystem-backed [`FsDocStore`].
//!
//! Each test builds a throwaway workspace under [`std::env::temp_dir`], drives the store
//! through create / get / save / search / ingest, and asserts the real on-disk effects:
//! a `.dx` stub file and a `.doc/*.bin` bundle appear, source round-trips through
//! parse/stringify, search finds a document by a body term, and ingest reports a count.
//! A `git init` exercises the repo-vs-local routing; git being unavailable is tolerated.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use doc_native::store::{CreateSpec, DocStore};
use doc_native::FsDocStore;

/// Monotonic counter so concurrently-run tests never share a workspace directory.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp workspace directory that removes itself on drop.
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    /// Create a unique empty workspace under the system temp directory.
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("doc-native-fsstore-{nanos}-{unique}"));
        fs::create_dir_all(&root).expect("create temp workspace");
        TempWorkspace { root }
    }

    /// Path to the workspace root.
    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Initialize a git repo in `root`; return whether git was available and succeeded.
fn try_git_init(root: &Path) -> bool {
    let init = Command::new("git").arg("-C").arg(root).arg("init").output();
    let Ok(init) = init else {
        return false;
    };
    if !init.status.success() {
        return false;
    }
    // Configure an identity so commits in routing tests do not fail.
    let _ = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.email", "test@example.com"])
        .output();
    let _ = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name", "Test"])
        .output();
    true
}

#[test]
fn create_writes_stub_and_bundle_then_round_trips() {
    let workspace = TempWorkspace::new();
    let git = try_git_init(workspace.path());
    let mut store = FsDocStore::new(workspace.path());

    let created = store
        .create(CreateSpec {
            path: "documents/notes.dx".to_string(),
            title: "My Notes".to_string(),
            content: Some("::heading level=1 id=h\nMy Notes\n::end\n\n::paragraph id=p\nThe quick brown fox jumps.\n::end\n".to_string()),
            ..CreateSpec::default()
        })
        .expect("create document");

    assert_eq!(created.relative_path, "documents/notes.dx");
    assert_eq!(created.title, "My Notes");

    // The on-disk `.dx` is a tiny stub pointer, never the content.
    let stub_path = workspace.path().join("documents/notes.dx");
    let stub = fs::read_to_string(&stub_path).expect("read stub");
    assert_eq!(stub, "~", "the .dx file must be a tiny stub, not content");

    // A bundle archive exists. Without git the doc routes to the repo bundle; with git in
    // a fresh repo the file is untracked, so it routes to the local bundle.
    let repo_bundle = workspace.path().join(".doc/.repo-docs.bin");
    let local_bundle = workspace.path().join(".doc/.local-docs.bin");
    if git {
        assert!(
            local_bundle.exists(),
            "untracked doc in a git repo must land in the local bundle"
        );
        assert!(
            !repo_bundle.exists(),
            "untracked doc must not land in the repo bundle"
        );
    } else {
        assert!(
            repo_bundle.exists(),
            "without git the doc must land in the repo bundle"
        );
    }

    // get-by-path returns the document; source round-trips through parse/stringify.
    let fetched = store
        .get_by_path("documents/notes.dx")
        .expect("get document");
    assert_eq!(fetched.id, created.id);
    assert!(fetched.source.contains("The quick brown fox jumps."));
    // The binary codec re-derives block ids from text on unpack (it stores no ids), so
    // the heading id is regenerated from its text rather than preserved verbatim.
    assert!(fetched.source.contains("::heading level=1 id=my-notes"));
    assert!(fetched.source.contains("My Notes"));
    assert!(fetched.source.ends_with("::end\n"));

    // get-by-id resolves to the same document.
    let by_id = store.get_by_id(created.id).expect("get by id");
    assert_eq!(by_id.relative_path, "documents/notes.dx");
}

#[test]
fn save_edit_then_search_finds_by_body_term() {
    let workspace = TempWorkspace::new();
    try_git_init(workspace.path());
    let mut store = FsDocStore::new(workspace.path());

    store
        .create(CreateSpec {
            path: "documents/topic.dx".to_string(),
            title: "Topic".to_string(),
            content: Some("::paragraph id=p\noriginal text\n::end\n".to_string()),
            ..CreateSpec::default()
        })
        .expect("create document");

    // Save an edit introducing a distinctive body term.
    let saved = store
        .save(
            "documents/topic.dx",
            "::heading level=1 id=h\nTopic\n::end\n\n::paragraph id=p\nphotosynthesis is fascinating\n::end\n",
        )
        .expect("save edit");
    assert!(saved.source.contains("photosynthesis is fascinating"));

    // The edited body is now searchable by its distinctive term.
    let hits = store.search("photosynthesis", 10).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].relative_path, "documents/topic.dx");

    // A term that does not appear yields no hits.
    let empty = store.search("xyzzy-nonexistent", 10).expect("search");
    assert!(empty.is_empty());

    // list with no query returns the document.
    let listed = store.list("", 50).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].relative_path, "documents/topic.dx");
}

#[test]
fn ingest_reports_count_over_workspace() {
    let workspace = TempWorkspace::new();
    try_git_init(workspace.path());
    let mut store = FsDocStore::new(workspace.path());

    store
        .create(CreateSpec {
            path: "a.dx".to_string(),
            title: "Alpha".to_string(),
            content: Some("::paragraph id=p\nalpha body\n::end\n".to_string()),
            ..CreateSpec::default()
        })
        .expect("create a");
    store
        .create(CreateSpec {
            path: "nested/b.dx".to_string(),
            title: "Beta".to_string(),
            content: Some("::paragraph id=p\nbeta body\n::end\n".to_string()),
            ..CreateSpec::default()
        })
        .expect("create b");

    // A loose full-source `.dx` (not a stub) is also picked up and parsed by ingest.
    fs::write(
        workspace.path().join("loose.dx"),
        "::heading level=1 id=h\nLoose Doc\n::end\n\n::paragraph id=p\ngamma body\n::end\n",
    )
    .expect("write loose doc");

    // Files inside ignored directories must be skipped.
    fs::create_dir_all(workspace.path().join("node_modules")).expect("mkdir node_modules");
    fs::write(
        workspace.path().join("node_modules/ignored.dx"),
        "::paragraph id=p\nshould be skipped\n::end\n",
    )
    .expect("write ignored doc");

    let report = store.ingest().expect("ingest");
    assert_eq!(report["ingested"], 3, "report = {report}");
    assert_eq!(report["engine"], "fs-bundle-dxlite");

    // After ingest, the loose doc is retrievable and the skipped one is not.
    let loose = store.get_by_path("loose.dx").expect("get loose");
    assert!(loose.source.contains("gamma body"));
    assert!(store.get_by_path("node_modules/ignored.dx").is_err());

    // render_html produces a structural HTML page for the docview:// resource.
    let html = store.render_html("loose.dx").expect("render html");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<h1>Loose Doc</h1>"));
    assert!(html.contains("<p>gamma body</p>"));
}

#[test]
fn tracked_document_routes_to_repo_bundle() {
    let workspace = TempWorkspace::new();
    if !try_git_init(workspace.path()) {
        // Git unavailable: routing-by-tracking cannot be exercised; skip cleanly.
        return;
    }
    let mut store = FsDocStore::new(workspace.path());

    store
        .create(CreateSpec {
            path: "tracked.dx".to_string(),
            title: "Tracked".to_string(),
            content: Some("::paragraph id=p\ntracked body\n::end\n".to_string()),
            ..CreateSpec::default()
        })
        .expect("create tracked");

    // Stage and commit the stub so git now reports it as tracked.
    let add = Command::new("git")
        .arg("-C")
        .arg(workspace.path())
        .args(["add", "tracked.dx"])
        .output()
        .expect("git add");
    assert!(add.status.success());
    let commit = Command::new("git")
        .arg("-C")
        .arg(workspace.path())
        .args(["commit", "-m", "add tracked"])
        .output()
        .expect("git commit");
    assert!(commit.status.success());

    // Re-saving now routes to the repo bundle (tracked, not ignored, not removed).
    store
        .save(
            "tracked.dx",
            "::paragraph id=p\ntracked body updated\n::end\n",
        )
        .expect("save tracked");

    assert!(
        workspace.path().join(".doc/.repo-docs.bin").exists(),
        "a committed, tracked doc must route to the repo bundle"
    );
    // And it must have been pruned from the local bundle if it was ever there.
    let local = workspace.path().join(".doc/.local-docs.bin");
    if local.exists() {
        let bytes = fs::read(&local).expect("read local bundle");
        let entries = doc_core::bundle::decode_bundle(&bytes).expect("decode local");
        assert!(
            entries.iter().all(|e| e.path != "tracked.dx"),
            "tracked doc must be pruned from the local bundle"
        );
    }
}
