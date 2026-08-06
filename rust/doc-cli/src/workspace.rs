//! The resolver: turning a `.dx` path into the document it stands for.
//!
//! # A `.dx` file is a pointer, and this is what dereferences it
//! On disk a `.dx` file is a one-line stub carrying the digest of its content
//! ([`doc_store::stub`]). The content lives in the workspace store. Every read in the CLI and
//! the MCP server comes through here, and the contract is absolute: **the caller gets the
//! true document, always.** [`read`] tries the store, then the committed packs, and only
//! reports a failure when no source can produce the document — it never returns a pointer as
//! if it were content, and never returns nothing where content exists.
//!
//! # Reading has no side effects
//! Resolving never creates the database, never rewrites a stub, and never executes anything.
//! A workspace that has only stubs and a committed pack — a fresh clone — resolves straight
//! from the pack. Adoption of outside edits happens in [`sync`], which a caller asks for
//! explicitly, or as part of a [`save`].
//!
//! # Writing
//! [`save`] stores the document as chunks, rewrites its stub, and re-exports the packs, so
//! canonical form and the on-disk pointer are always what the store says they are.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use doc_core::format::parse;
use doc_core::model::Document;
use doc_core::resolve::Resolver;
use doc_store::{pack, stub, Stats, Store, StoreError, SyncReport};

/// Markers that identify a workspace root, in the order they are trusted.
const ROOT_MARKERS: &[&str] = &[".doc", ".git"];

/// A document and the path it was resolved from.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Absolute path of the `.dx` file (the stub).
    pub path: PathBuf,
    /// Path relative to the workspace root, for display.
    pub relative: String,
    /// The resolved document.
    pub document: Document,
}

impl Loaded {
    /// The document's display title: its metadata title, else its first heading, else its
    /// file name.
    #[must_use]
    pub fn title(&self) -> String {
        for candidate in [
            self.document.title.trim(),
            self.document.first_heading_text().trim(),
        ] {
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.relative.clone())
    }
}

/// The workspace root governing `start`: the nearest ancestor holding `.doc` or `.git`.
///
/// Falls back to `start` itself (or its parent, when it is a file) so a document outside any
/// project still has a well-defined store beside it.
#[must_use]
pub fn workspace_root(start: &Path) -> PathBuf {
    let absolute = fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut cursor: &Path = if absolute.is_dir() {
        &absolute
    } else {
        absolute.parent().unwrap_or(&absolute)
    };

    loop {
        if ROOT_MARKERS
            .iter()
            .any(|marker| cursor.join(marker).is_dir())
        {
            return cursor.to_path_buf();
        }
        match cursor.parent() {
            Some(parent) if parent != cursor => cursor = parent,
            _ => break,
        }
    }

    if absolute.is_dir() {
        absolute
    } else {
        absolute
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }
}

/// The workspace-relative form of `path`, for keying the store.
fn relative_of(root: &Path, path: &Path) -> String {
    let absolute = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let trimmed = absolute.strip_prefix(root).unwrap_or(&absolute);
    stub::normalize_path(&trimmed.to_string_lossy())
        .unwrap_or_else(|| trimmed.to_string_lossy().into_owned())
}

/// Open the workspace store at `root`, creating it if needed.
///
/// Use this for writes. Reads should prefer [`open_existing`] so that resolving a document
/// never brings a database into being.
pub fn open_store(root: &Path) -> Result<Store, String> {
    Store::open(root).map_err(|error| error.to_string())
}

/// Open the store only if it has already been built, so reads create nothing.
fn open_existing(root: &Path) -> Option<Store> {
    if root.join(".doc").join("index.db").exists() {
        Store::open(root).ok()
    } else {
        None
    }
}

/// Read the canonical source of the document at `path`, or `-` for standard input.
///
/// Resolution order, stopping at the first that answers:
/// 1. Plain document text sitting on disk — something other than `dx` wrote it, and it is the
///    newest truth. Returned as found, so an external edit is never lost or second-guessed.
/// 2. The store, when the file is a stub.
/// 3. The committed packs, for a workspace whose index has not been built yet.
///
/// A stub nothing can resolve is an error that says what to run, never an empty document.
pub fn read(path: &Path) -> Result<String, String> {
    if path == Path::new("-") {
        let mut buffer = String::new();
        return std::io::stdin()
            .read_to_string(&mut buffer)
            .map(|_| buffer)
            .map_err(|error| format!("could not read standard input: {error}"));
    }

    let on_disk = fs::read_to_string(path).ok();
    let is_pointer = on_disk.as_deref().is_some_and(stub::is_stub);

    if let Some(text) = &on_disk {
        if !is_pointer {
            return Ok(text.clone());
        }
    }

    let root = workspace_root(path);
    let relative = relative_of(&root, path);

    if let Some(store) = open_existing(&root) {
        match store.source(&relative) {
            Ok(source) => return Ok(source),
            // Not in the index yet: fall through to the packs.
            Err(StoreError::NotFound(_)) => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    match pack::source(&root, &relative).map_err(|error| error.to_string())? {
        Some(source) => Ok(source),
        None if is_pointer => Err(format!(
            "{} is a dx pointer, but its content is not in this workspace's store or packs; \
             run `dx sync` to rebuild from .doc/, or restore .doc/repo.dxcp",
            path.display()
        )),
        None => Err(format!("could not read {}", path.display())),
    }
}

/// Resolve whatever a `.dx` file *contains*, without trusting where it sits.
///
/// This is what git's diff driver needs. Git extracts a blob to a throwaway temporary file and
/// runs `textconv` on that, so the path carries no information — the digest written inside the
/// pointer is the only usable key. Because the store keeps a manifest for every version it has
/// ever held, this resolves historical revisions too, which is what makes `git log -p` and
/// `git show` render documents rather than digests.
///
/// `search_from` is where the workspace is looked for (git runs the driver from the top of the
/// worktree). Plain document text passes straight through unchanged.
pub fn resolve_contents(text: &str, search_from: &Path) -> Result<String, String> {
    let Some(digest) = stub::digest_in(text) else {
        return Ok(text.to_string());
    };

    let root = workspace_root(search_from);
    if let Some(store) = open_existing(&root) {
        if let Some(source) = store
            .source_of_version(&digest)
            .map_err(|error| error.to_string())?
        {
            return Ok(source);
        }
    }

    // No index, or a version it never held: the packs still carry the current one.
    for source in pack::load_all(&root)
        .map_err(|error| error.to_string())?
        .into_values()
    {
        if stub::digest_of(&source) == digest {
            return Ok(source);
        }
    }

    Err(format!(
        "this dx pointer names version {digest}, which is not in {}'s store or packs; \
         run `dx sync` there, or restore .doc/repo.dxcp",
        root.display()
    ))
}

/// The CLI's [`Resolver`]: a document's own folder on disk.
///
/// `file` reads a sibling file as it is; `document` reads a sibling `.dx` through
/// [`read`], so a pointer resolves to its true content exactly as every other read does.
/// The path law lives in `doc_core::resolve` — by the time a path reaches this struct it
/// is already relative and downward — so this is transport, nothing more.
pub struct FolderResolver {
    /// The folder the document lives in; every reference is joined under it.
    folder: PathBuf,
}

/// The resolver for references made by the document at `path`.
#[must_use]
pub fn resolver_for(path: &Path) -> FolderResolver {
    FolderResolver {
        folder: document_dir(path),
    }
}

impl Resolver for FolderResolver {
    fn file(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.folder.join(path)).ok()
    }

    fn document(&self, path: &str) -> Option<String> {
        read(&self.folder.join(path)).ok()
    }

    fn binary(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(self.folder.join(path)).ok()
    }
}

/// Read and resolve the document at `path`.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let source = read(path)?;
    let root = workspace_root(path);
    Ok(Loaded {
        relative: relative_of(&root, path),
        path: path.to_path_buf(),
        document: parse(&source),
    })
}

/// Store `document` at `path`: chunks into the store, pointer onto disk, packs re-exported.
pub fn save(path: &Path, document: &Document) -> Result<(), String> {
    let root = workspace_root(path);
    let relative = relative_of(&root, path);
    let mut store = open_store(&root)?;
    store
        .save(&relative, document)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Write raw text to `path`, creating parent directories as needed.
///
/// This is the escape hatch for output that is *not* a document — a rendered HTML page, a
/// screenshot, a report redirected by `--out`. Documents go through [`save`].
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
    }
    fs::write(path, text).map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// The directory a document lives in, used as the working directory for its code blocks.
#[must_use]
pub fn document_dir(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Reconcile the workspace at `root`: adopt plain-text documents, restore stubs from packs,
/// and collect unreferenced chunks.
pub fn sync(root: &Path) -> Result<SyncReport, String> {
    open_store(root)?.sync().map_err(|error| error.to_string())
}

/// Storage totals for the workspace at `root`.
pub fn stats(root: &Path) -> Result<Stats, String> {
    match open_existing(root) {
        Some(store) => store.stats().map_err(|error| error.to_string()),
        None => Ok(Stats::default()),
    }
}

/// Find every `.dx` file under `root`, sorted by path.
#[must_use]
pub fn discover(root: &Path) -> Vec<PathBuf> {
    doc_store::discover_documents(root)
}

/// Resolve every document under `root`, skipping any that cannot be read.
///
/// Documents are found from the store when it has them and from disk otherwise, so a listing
/// covers both what has been stored and what has only just been written by something else.
#[must_use]
pub fn load_all(root: &Path) -> Vec<Loaded> {
    let mut loaded: Vec<Loaded> = Vec::new();
    for path in discover(root) {
        if let Ok(document) = load(&path) {
            loaded.push(document);
        }
    }

    // Include stored documents whose stub is absent, so nothing stored is invisible.
    if let Some(store) = open_existing(root) {
        if let Ok(summaries) = store.list() {
            for summary in summaries {
                if loaded.iter().any(|entry| entry.relative == summary.path) {
                    continue;
                }
                if let Ok(document) = store.document(&summary.path) {
                    loaded.push(Loaded {
                        path: root.join(&summary.path),
                        relative: summary.path,
                        document,
                    });
                }
            }
        }
    }

    loaded.sort_by(|a, b| a.relative.cmp(&b.relative));
    loaded
}

/// One search hit: a document and why it matched.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The matching document.
    pub document: Loaded,
    /// Relevance score from the index; higher is a better match.
    pub score: f64,
}

/// Search every document under `root` for `query`, best matches first.
///
/// The store answers when it has been built — it narrows candidates in SQL before ranking, so
/// the cost tracks matches rather than corpus size. Otherwise the documents are resolved and
/// ranked in memory, which keeps search working in a workspace with no index yet.
#[must_use]
pub fn search(root: &Path, query: &str, limit: usize) -> Vec<Hit> {
    if let Some(store) = open_existing(root) {
        if let Ok(summaries) = store.search(query, limit) {
            let hits: Vec<Hit> = summaries
                .into_iter()
                .filter_map(|summary| {
                    let document = store.document(&summary.path).ok()?;
                    Some(Hit {
                        // Ranking order is the store's; the score is not re-derived here.
                        score: 1.0,
                        document: Loaded {
                            path: root.join(&summary.path),
                            relative: summary.path,
                            document,
                        },
                    })
                })
                .collect();
            if !hits.is_empty() {
                return hits;
            }
        }
    }

    let documents = load_all(root);
    let indexed: Vec<(String, Document)> = documents
        .iter()
        .map(|loaded| (loaded.relative.clone(), loaded.document.clone()))
        .collect();
    let index = doc_core::search::build_index(&indexed);

    index
        .search(query)
        .into_iter()
        .take(limit)
        .filter_map(|result| {
            documents
                .iter()
                .find(|loaded| loaded.relative == result.path)
                .map(|loaded| Hit {
                    document: loaded.clone(),
                    score: result.score,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-resolver-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");
        // Mark it a workspace so root detection does not climb into the real repository.
        fs::create_dir_all(root.join(".doc")).expect("marker");
        fs::canonicalize(&root).expect("canonical root")
    }

    const NOTES: &str = "::heading level=1 id=notes\nNotes\n::end\n\n::paragraph id=p\nkubernetes scheduling notes\n::end\n";

    #[test]
    fn a_saved_document_is_a_pointer_on_disk_and_real_content_through_the_resolver() {
        let root = scratch("pointer");
        let path = root.join("notes.dx");
        save(&path, &parse(NOTES)).expect("save");

        let raw = fs::read_to_string(&path).expect("read raw");
        assert!(stub::is_stub(&raw), "expected a pointer, found {raw:?}");
        assert!(!raw.contains("kubernetes"));

        assert_eq!(read(&path).expect("resolve"), NOTES);
        assert_eq!(load(&path).expect("load").title(), "Notes");
    }

    #[test]
    fn plain_text_written_by_anything_else_is_returned_as_the_newest_truth() {
        let root = scratch("external-edit");
        let path = root.join("outside.dx");
        fs::write(&path, NOTES).expect("external write");
        assert_eq!(read(&path).expect("resolve"), NOTES);
    }

    #[test]
    fn a_pointer_resolves_from_the_packs_with_no_database_present() {
        // The fresh-clone case: stubs and repo.dxcp committed, index.db absent.
        let root = scratch("fresh-clone");
        let path = root.join("notes.dx");
        save(&path, &parse(NOTES)).expect("save");
        fs::remove_file(root.join(".doc/index.db")).expect("drop index");

        assert!(stub::is_stub(
            &fs::read_to_string(&path).expect("stub still there")
        ));
        assert_eq!(read(&path).expect("resolve from pack"), NOTES);
    }

    #[test]
    fn reading_creates_nothing() {
        let root = scratch("pure-read");
        let path = root.join("notes.dx");
        fs::write(&path, NOTES).expect("write");

        let _ = read(&path).expect("resolve");
        assert!(
            !root.join(".doc/index.db").exists(),
            "resolving a document must not build a database"
        );
    }

    #[test]
    fn an_unresolvable_pointer_says_what_to_run() {
        let root = scratch("orphan");
        let path = root.join("orphan.dx");
        fs::write(&path, stub::render(NOTES)).expect("write stub");

        let error = read(&path).expect_err("should fail");
        assert!(error.contains("dx sync"), "{error}");
        // And never passes the pointer off as content.
        assert!(!error.contains("~ dx1"));
    }

    #[test]
    fn a_missing_file_explains_which_one() {
        let root = scratch("missing");
        let error = read(&root.join("nope.dx")).expect_err("should fail");
        assert!(error.contains("nope.dx"), "{error}");
    }

    #[test]
    fn saving_canonicalizes_sloppy_input() {
        let root = scratch("canonical");
        let path = root.join("messy.dx");
        save(&path, &parse("::heading level=2 id=h Hello ::end\n")).expect("save");
        assert_eq!(
            read(&path).expect("resolve"),
            "::heading level=2 id=h\nHello\n::end\n"
        );
    }

    #[test]
    fn listing_and_search_find_documents_through_the_store() {
        let root = scratch("search");
        save(&root.join("kubernetes.dx"), &parse(NOTES)).expect("k");
        save(
            &root.join("recipes.dx"),
            &parse("::paragraph id=p\nbread and soup\n::end\n"),
        )
        .expect("r");

        let listed = load_all(&root);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].relative, "kubernetes.dx");

        let hits = search(&root, "kubernetes", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.relative, "kubernetes.dx");
    }

    #[test]
    fn search_works_before_any_index_exists() {
        let root = scratch("search-no-index");
        fs::write(root.join("a.dx"), NOTES).expect("write");
        let hits = search(&root, "kubernetes", 10);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn sync_adopts_plain_text_and_leaves_a_pointer() {
        let root = scratch("sync");
        let path = root.join("adopted.dx");
        fs::write(&path, NOTES).expect("write");

        let report = sync(&root).expect("sync");
        assert_eq!(report.ingested, vec!["adopted.dx".to_string()]);
        assert!(stub::is_stub(&fs::read_to_string(&path).expect("read")));
        assert_eq!(read(&path).expect("resolve"), NOTES);
    }

    #[test]
    fn the_workspace_root_is_the_nearest_marked_ancestor() {
        let root = scratch("root-detect");
        let nested = root.join("a/b");
        fs::create_dir_all(&nested).expect("dirs");
        assert_eq!(workspace_root(&nested.join("c.dx")), root);
        assert_eq!(workspace_root(&root), root);
    }

    #[test]
    fn document_dir_is_where_a_blocks_relative_paths_resolve() {
        assert_eq!(document_dir(Path::new("/a/b/c.dx")), PathBuf::from("/a/b"));
        assert_eq!(document_dir(Path::new("c.dx")), PathBuf::from("."));
    }

    /// Each `doc-core` fixture input and the repository document it mirrors.
    ///
    /// The round-trip corpus in `doc-core/tests/fixtures` is hermetic plain text (the
    /// store's walker never adopts a `fixtures` directory), while the documents it mirrors
    /// live in this repository's store as pointers. This crate is the one that can resolve
    /// them, so the drift guard lives here: refreshing an example is a deliberate
    /// two-file change, never a silent divergence.
    const FIXTURE_MIRRORS: &[(&str, &str)] = &[
        ("welcome.input.dx", "examples/welcome.dx"),
        ("tutorial.input.dx", "examples/tutorial.dx"),
        ("showcase.input.dx", "examples/showcase.dx"),
        ("block-reference.input.dx", "examples/block-reference.dx"),
        (
            "compactness-comparison.input.dx",
            "examples/compactness-comparison.dx",
        ),
        ("footprint-pair.input.dx", "examples/footprint-pair.dx"),
        ("compact-proof.input.dx", "documents/compact-proof.dx"),
    ];

    #[test]
    fn every_fixture_input_still_matches_the_document_it_mirrors() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("the crate sits two levels below the repository root");
        let fixtures = repository.join("rust/doc-core/tests/fixtures");

        for (fixture, source) in FIXTURE_MIRRORS {
            let resolved = read(&repository.join(source))
                .unwrap_or_else(|error| panic!("could not resolve {source}: {error}"));
            let copy = fs::read_to_string(fixtures.join(fixture))
                .unwrap_or_else(|error| panic!("fixture {fixture} is missing: {error}"));
            assert_eq!(
                copy, resolved,
                "rust/doc-core/tests/fixtures/{fixture} has drifted from {source}; copy the \
                 document over it and re-check the .expected.dx output"
            );
        }
    }

    #[test]
    fn write_text_is_for_output_that_is_not_a_document() {
        let root = scratch("raw-write");
        let path = root.join("out/page.html");
        write_text(&path, "<p>hi</p>").expect("write");
        assert_eq!(fs::read_to_string(&path).expect("read"), "<p>hi</p>");
    }
}
