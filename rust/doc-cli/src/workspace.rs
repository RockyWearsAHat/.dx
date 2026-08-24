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

use std::collections::{BTreeMap, HashMap};
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
    /// The document's display title — the one derivation in [`Document::display_title`].
    #[must_use]
    pub fn title(&self) -> String {
        self.document.display_title(&self.relative)
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
///
/// A store on a medium this process cannot write — a read-only checkout, a build
/// sandbox — still answers: it opens without the ability to write. A store that will not
/// open either way is an error, never "no store" — falling through to the packs would
/// quietly answer from a stale copy while the true store sits broken.
///
/// An index written by an *older* dx is the one case that is not an error and not a store:
/// opening it would migrate it, and migrating rebuilds the derived tables — a read that
/// silently rewrites the index is exactly what this project says reading never does. The
/// packs carry the content, so the read falls through to them and the next write (or
/// `dx sync`) does the upgrade, deliberately.
fn open_existing(root: &Path) -> Result<Option<Store>, String> {
    if !root.join(".doc").join("index.db").exists() {
        return Ok(None);
    }
    if doc_store::stale_index(root).map_err(|error| error.to_string())? {
        return Ok(None);
    }
    match Store::open(root) {
        Ok(store) => Ok(Some(store)),
        Err(writable_error) => Store::open_read_only(root).map(Some).map_err(|_| {
            format!(
                "the store at {} exists but would not open: {writable_error}",
                root.join(".doc").display()
            )
        }),
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
    Sources::open(&workspace_root(path))?.source(path)
}

/// A workspace's content, opened once: the store, and the packs behind it.
///
/// One document resolved on its own is one store open, and that is fine. A *listing* resolves
/// every document in the tree, and opening SQLite per document is most of what `dx ls` and
/// `dx search` spend — a quarter of a search's wall clock here, for nineteen documents. The
/// packs are worse: reading one decodes the whole file, so falling back to them per document
/// is quadratic in a workspace with no index at all.
///
/// So both are opened at most once and shared. The resolution order is [`read`]'s and is
/// stated there; this type is where it is implemented, so there is one answer to "what is the
/// text of this document" rather than a fast one and a careful one.
struct Sources {
    root: PathBuf,
    store: Option<Store>,
    /// Filled on the first pointer the store cannot answer, and never if it always can.
    packs: std::cell::OnceCell<BTreeMap<String, String>>,
}

impl Sources {
    /// Open the store for the workspace at `root`, if it has one.
    fn open(root: &Path) -> Result<Self, String> {
        Ok(Self {
            root: root.to_path_buf(),
            store: open_existing(root)?,
            packs: std::cell::OnceCell::new(),
        })
    }

    /// The canonical source of the document at `path` — see [`read`] for the order.
    fn source(&self, path: &Path) -> Result<String, String> {
        let on_disk = fs::read_to_string(path).ok();
        let is_pointer = on_disk.as_deref().is_some_and(stub::is_stub);
        // What the file actually names, read before the text is given away below.
        let named = on_disk.as_deref().and_then(stub::digest_in);

        if let Some(text) = on_disk {
            if !is_pointer {
                return Ok(text);
            }
        }

        let relative = relative_of(&self.root, path);
        // Every source is held to the digest the file names. The store is keyed by path, so
        // it answers for this document even when it holds a *different* version of it — and a
        // git operation moves a pointer without telling the index: a merge writes the merged
        // pack and the merged stub and deliberately writes no store, so between the merge and
        // the next `dx sync` the index still holds whatever version this branch happened to
        // have. Handing that back would be the resolver answering with a document the file
        // does not name, which is the one thing it must never do. So the packs — which the
        // merge did rewrite — get their turn, and a disagreement is a sentence, never a
        // plausible wrong answer.
        let names_it = |source: &str| named.as_ref().is_none_or(|d| stub::digest_of(source) == *d);

        if let Some(store) = &self.store {
            match store.source(&relative) {
                Ok(source) if names_it(&source) => return Ok(source),
                // A version of this document, but not this one: the packs may have it.
                Ok(_) => {}
                // Not in the index yet: fall through to the packs.
                Err(StoreError::NotFound(_)) => {}
                Err(error) => return Err(error.to_string()),
            }
        }

        match self.packed()?.get(&relative) {
            Some(source) if names_it(source) => Ok(source.clone()),
            Some(_) => Err(format!(
                "{} names version {}, and neither this workspace's store nor its packs holds \
                 that version — they hold a different one. Run `dx sync` to reconcile them, or \
                 restore .doc/repo.dxcp",
                path.display(),
                named.unwrap_or_default()
            )),
            None if is_pointer => Err(format!(
                "{} is a dx pointer, but its content is not in this workspace's store or packs; \
                 run `dx sync` to rebuild from .doc/, or restore .doc/repo.dxcp",
                path.display()
            )),
            // The plain "no such file" case, and the one a person hits most: say what is
            // missing and what makes it exist, rather than restating the path back.
            None if !path.exists() => Err(format!(
                "no file at {}; `dx ls` lists the documents in this project, and \
                 `dx new {}` creates one",
                path.display(),
                path.display()
            )),
            None => Err(format!(
                "could not read {} — it exists but is not text this dx can read",
                path.display()
            )),
        }
    }

    /// Every document the packs carry, decoded once.
    fn packed(&self) -> Result<&BTreeMap<String, String>, String> {
        if let Some(loaded) = self.packs.get() {
            return Ok(loaded);
        }
        let loaded = pack::load_all(&self.root).map_err(|error| error.to_string())?;
        Ok(self.packs.get_or_init(|| loaded))
    }

    /// The document at `path`, parsed, with the paths a caller reports it by.
    fn load(&self, path: &Path) -> Result<Loaded, String> {
        Ok(Loaded {
            relative: relative_of(&self.root, path),
            path: path.to_path_buf(),
            document: parse(&self.source(path)?),
        })
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
    if let Some(store) = open_existing(&root)? {
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
/// Both `file` and `document` resolve `.dx` pointers to their true content through
/// [`read`], so every reference in the document gets the actual text it refers to, never
/// a pointer. The path law lives in `doc_core::resolve` — by the time a path reaches this
/// struct it is already relative and downward — so this is transport, nothing more.
/// The contract the resolver enforces is absolute: a caller gets the true document,
/// always, and a pointer never passes off as content.
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
    /// Read a file the document references, resolving `.dx` pointers to their true content.
    ///
    /// For `.dx` files (pointers), this resolves the pointer through the workspace store
    /// or packs, exactly as every other read does — the caller never sees a pointer where
    /// a document belongs. For ordinary files, this returns the file's text as-is. See
    /// [`read`] for the resolution order.
    fn file(&self, path: &str) -> Option<String> {
        read(&self.folder.join(path)).ok()
    }

    /// Read a document (alias for `file`, for compatibility with the `Resolver` trait).
    fn document(&self, path: &str) -> Option<String> {
        read(&self.folder.join(path)).ok()
    }

    fn binary(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(self.folder.join(path)).ok()
    }

    fn files_under(&self, path: &str) -> Option<Vec<(String, String)>> {
        let root = self.folder.join(path);
        if !root.is_dir() {
            return None;
        }
        let mut files = Vec::new();
        collect_tree(&root, path, &mut files);
        files.sort();
        Some(files)
    }
}

/// Gather every file under `dir` into `files` as `(path, content)` pairs, `prefix`
/// carrying the path back to the document's folder. Hidden entries and regenerated build
/// caches (`target`, `node_modules`) are skipped — the contract [`Resolver::files_under`]
/// states. Unreadable entries are skipped rather than failed: a vanished file simply
/// stops contributing to the fingerprint, which is itself a change.
///
/// A text file contributes its exact text; a binary file contributes its bytes' digest.
/// It must never contribute *lossy* text: two different builds of a binary differ mostly
/// inside invalid-UTF-8 runs that lossy conversion collapses to identical replacement
/// characters — which is how a rebuilt wasm engine once slipped past a gate that
/// declared its folder with `reads=`.
fn collect_tree(dir: &Path, prefix: &str, files: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let held = format!("{prefix}/{name}");
        if path.is_dir() {
            if name != "target" && name != "node_modules" {
                collect_tree(&path, &held, files);
            }
        } else if let Ok(bytes) = fs::read(&path) {
            let content = String::from_utf8(bytes)
                .unwrap_or_else(|raw| doc_core::digest::sha256_hex(raw.as_bytes()));
            files.push((held, content));
        }
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

/// Fill an index a workspace has not built yet from the packs committed beside it.
///
/// A fresh clone and a new `git worktree` arrive the same way: `.doc/repo.dxcp` is there
/// because it is committed, and `.doc/index.db` is not because it never is. The first write
/// into such a workspace was the moment the loss happened. The store came into being empty,
/// took the one document being saved, and exported the packs from what it held — which was
/// that document alone. Every other document's pointer then named content the pack no longer
/// carried, and the older versions the pack keeps so `git diff` can show a pointer's history
/// went with them. The export guard ([`StoreError::WouldLose`]) caught the case where another
/// `.dx` file was standing in the tree and refused the write, which is right, and is still a
/// stop sign in front of somebody doing ordinary work in a worktree they just made.
///
/// So the write path restores first. This is [`Store::sync`]'s own fresh-clone rule, run at
/// the moment it is needed instead of waiting for a person to know to run it. It fires only
/// when there is no index at all — a workspace that has one is already the authority — and it
/// is on the write path only, so resolving a pointer still brings no database into being.
fn ensure_store_ready(root: &Path) -> Result<(), String> {
    if root.join(".doc").join("index.db").exists() {
        return Ok(());
    }
    let (repo, local) = pack::paths(root);
    if !repo.exists() && !local.exists() {
        return Ok(());
    }
    let mut store = open_store(root)?;
    store.sync().map(|_| ()).map_err(|error| error.to_string())
}

/// Store `document` at `path`: chunks into the store, pointer onto disk, packs re-exported.
///
/// Writing a document is also the moment the repository is taught to handle one, so this is
/// where [`ensure_git_ready`](crate::commands::store::ensure_git_ready) runs: a checkout
/// nobody prepared by hand still diffs and merges its documents. It writes configuration and
/// two text files, never git's index, and is a no-op every time after the first.
/// [`ensure_store_ready`] runs beside it for the same reason, and states why.
pub fn save(path: &Path, document: &Document) -> Result<(), String> {
    let root = workspace_root(path);
    let relative = relative_of(&root, path);
    let _ = crate::commands::store::ensure_git_ready(&root);
    ensure_store_ready(&root)?;
    let mut store = open_store(&root)?;
    store
        .save(&relative, document)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Save a document whose new content arrived as source text — a run's report, a format
/// pass — keeping the storage form the file already has.
///
/// A pointer file's content goes to the store and the pointer stays a pointer; plain text
/// is rewritten as plain text, and no store is brought into being for it. Whether a
/// document lives in the store was decided by whoever adopted it (`dx sync`, `dx_write`,
/// `dx_edit`), and refreshing its output is not that decision. Writing a stored document
/// out as resolved text — which is what routing these saves through [`write_text`] did —
/// left the store stale behind a working tree full of text git saw as a giant diff, and
/// the next `dx sync` would have adopted that text *back*, silently rewinding anything
/// saved to the store in between.
///
/// # Errors
/// Returns a sentence when neither the store nor the file could take the save.
pub fn save_source(path: &Path, source: &str) -> Result<(), String> {
    let is_pointer = fs::read_to_string(path)
        .ok()
        .is_some_and(|text| stub::is_stub(&text));
    if is_pointer {
        save(path, &parse(source))
    } else {
        write_text(path, source)
    }
}

/// Write raw text to `path`, creating parent directories as needed.
///
/// This is the escape hatch for output that is *not* a document — a rendered HTML page, a
/// screenshot, a report redirected by `--out`. Documents go through [`save`] (block edits)
/// or [`save_source`] (whole-source rewrites that must keep the file's storage form).
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
    }
    fs::write(path, text).map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// One sentence per `::image` in `document` whose local file already exceeds the embed
/// limit — said at save time, so the writer hears about the bound now instead of the
/// reader meeting hydration's refusal at render. A file that does not exist yet (a run
/// will produce it) earns no warning; hydration has its own sentence for a missing file.
#[must_use]
pub fn oversized_image_warnings(document: &Document, path: &Path) -> Vec<String> {
    let dir = document_dir(path);
    document
        .blocks
        .iter()
        .filter(|block| block.kind == "image")
        .filter_map(|block| {
            let relative = doc_core::resolve::confined(&block.src)?;
            let bytes = fs::metadata(dir.join(relative)).ok()?.len() as usize;
            (bytes > doc_core::resolve::MAX_IMAGE_BYTES).then(|| {
                format!(
                    "note: image `{}` names {} — {bytes} bytes against the {} byte embed \
                     limit, so the render will refuse to embed it. Export a smaller \
                     capture (`dx png --scale 1` stays within it).",
                    block.id,
                    block.src,
                    doc_core::resolve::MAX_IMAGE_BYTES
                )
            })
        })
        .collect()
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
    let _ = crate::commands::store::ensure_git_ready(root);
    open_store(root)?.sync().map_err(|error| error.to_string())
}

/// What [`remove`] deleted, for the caller's report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed {
    /// The store forgot the document: its row and stub went, chunks it no longer shares
    /// were collected, and the packs were rewritten so the deletion sticks.
    pub stored: bool,
    /// A file still on disk (a plain-text document nothing had adopted) was deleted.
    pub file: bool,
}

/// Delete the document at `path`, deliberately.
///
/// Deleting the pointer file and running `dx sync` prunes too, but that route reads as
/// repair; this one names the intent. The store forgets the document when it holds it, and
/// whatever file remains on disk is deleted either way. Version history (the manifests)
/// deliberately survives, exactly as a sync prune leaves it, so `git show` on an old commit
/// still renders the document; `git checkout` + `dx sync` restores it whole.
///
/// # Errors
/// Returns a sentence when there is nothing at `path` to remove, or when the store refuses.
pub fn remove(path: &Path) -> Result<Removed, String> {
    let root = workspace_root(path);
    let relative = relative_of(&root, path);

    let mut removed = Removed {
        stored: false,
        file: false,
    };
    if let Some(mut store) = open_existing(&root)? {
        match store.delete(&relative) {
            Ok(()) => removed.stored = true,
            Err(StoreError::NotFound(_)) => {}
            Err(error) => return Err(error.to_string()),
        }
    } else if pack::source(&root, &relative)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        // A fresh clone: stubs and packs, no index. Deleting only the file would let the
        // next `dx sync` restore the document from the pack — build the store first so
        // the deletion registers there and the packs are rewritten without it.
        let mut store = open_store(&root)?;
        store.sync().map_err(|error| error.to_string())?;
        store.delete(&relative).map_err(|error| error.to_string())?;
        removed.stored = true;
    }
    // The store deletes the stub it wrote; anything still here is a plain-text document
    // (or a stub in a workspace with no index), and `rm` means: gone.
    if path.is_file() {
        fs::remove_file(path)
            .map_err(|error| format!("could not delete {}: {error}", path.display()))?;
        removed.file = true;
    }

    if removed.stored || removed.file {
        Ok(removed)
    } else {
        Err(format!(
            "nothing to remove: {} is not on disk and not in this workspace's store",
            path.display()
        ))
    }
}

/// Storage totals for the workspace at `root`.
pub fn stats(root: &Path) -> Result<Stats, String> {
    match open_existing(root)? {
        Some(store) => store.stats().map_err(|error| error.to_string()),
        None => Ok(Stats::default()),
    }
}

/// Find every `.dx` file under `root`, sorted by path.
#[must_use]
pub fn discover(root: &Path) -> Vec<PathBuf> {
    doc_store::discover_documents(root)
}

/// Every document a directory holds — and every one it holds that would not resolve.
///
/// The resolver's contract is "the caller gets the true document, always"; when it cannot,
/// the failure is part of the answer. A listing that silently dropped an unresolvable
/// document would hide exactly the thing its caller needs to hear about.
#[derive(Debug, Default)]
pub struct Listing {
    /// The resolved documents, sorted by workspace-relative path.
    pub documents: Vec<Loaded>,
    /// Documents that exist but did not resolve: each path with the error saying what to run.
    pub unresolved: Vec<(String, String)>,
}

/// The workspace-relative form of `directory`, `""` when it is the workspace root itself.
///
/// Not [`relative_of`]: that one keys *documents* and appends `.dx` to anything unnamed,
/// which would turn the scope `docs` into `docs.dx` and match nothing.
fn scope_of(root: &Path, directory: &Path) -> String {
    let absolute = fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    absolute
        .strip_prefix(root)
        .map(|scoped| scoped.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// Whether the workspace-relative `path` sits under the workspace-relative `scope`
/// (`""` is the root: everything is in scope).
fn in_scope(path: &str, scope: &str) -> bool {
    scope.is_empty()
        || path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Resolve every document under `directory`.
///
/// Documents are found from the tree and from the workspace store, so a listing covers both
/// what has been stored and what has only just been written by something else. The store
/// lives at the *workspace root* even when `directory` is a subdirectory, so a scoped
/// listing still sees stored documents whose stub is absent. A document that will not
/// resolve is reported in [`Listing::unresolved`], never silently dropped; a store that
/// will not open is an error.
pub fn load_all(directory: &Path) -> Result<Listing, String> {
    let root = workspace_root(directory);
    let scope = scope_of(&root, directory);

    // One store open for the whole listing, not one per document (`Sources`).
    let sources = Sources::open(&root)?;
    let mut listing = Listing::default();
    for path in discover(directory) {
        match sources.load(&path) {
            Ok(document) => listing.documents.push(document),
            Err(error) => listing.unresolved.push((relative_of(&root, &path), error)),
        }
    }

    // Include stored documents whose stub is absent, so nothing stored is invisible.
    if let Some(store) = &sources.store {
        for summary in store.list().map_err(|error| error.to_string())? {
            let seen = listing
                .documents
                .iter()
                .any(|entry| entry.relative == summary.path)
                || listing
                    .unresolved
                    .iter()
                    .any(|(path, _)| *path == summary.path);
            if seen || !in_scope(&summary.path, &scope) {
                continue;
            }
            match store.document(&summary.path) {
                Ok(document) => listing.documents.push(Loaded {
                    path: root.join(&summary.path),
                    relative: summary.path,
                    document,
                }),
                Err(error) => listing.unresolved.push((summary.path, error.to_string())),
            }
        }
    }

    listing
        .documents
        .sort_by(|a, b| a.relative.cmp(&b.relative));
    listing.unresolved.sort();
    Ok(listing)
}

/// File extensions offered to search as source, beyond the `.dx` documents themselves.
///
/// An allow-list rather than "everything that decodes as UTF-8": a lock file, a minified
/// bundle, or a generated blob answers no question anyone asks, and indexing it only dilutes
/// the ranking of the files that do.
///
/// What belongs here is a language someone writes by hand. The list was once short enough to
/// be a bug: an Objective-C project's `.m` files and a Metal project's shaders were invisible,
/// so a question whose only answer was in them could not be reached by the cheap route at all —
/// on a first-contact repository that is the difference between landing and sending the caller
/// back to grep. When a project speaks a language not listed here, adding it is the fix.
const SEARCHABLE_SOURCE: &[&str] = &[
    // Systems and application languages.
    "rs", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "m", "mm", "metal", "cu", "go", "zig", "swift",
    "java", "kt", "kts", "scala", "cs", "fs", "dart", "hs", "ml", "ex", "exs", "erl", "clj", "lua",
    "nim", "d", "pas", "asm", "s", // Scripting and the web.
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "vue", "svelte", "py", "pyi", "rb", "php", "pl", "pm",
    "r", "jl", "sh", "bash", "zsh", "fish", "ps1",
    // Markup, style, data, and configuration a person writes.
    "md", "mdx", "rst", "adoc", "txt", "html", "htm", "css", "scss", "sass", "less", "sql",
    "graphql", "gql", "proto", "toml", "yaml", "yml", "json", "jsonc", "ini", "cfg", "conf", "env",
    "tf", "hcl", "cmake", "mk", "gradle", "bzl", "nix",
];

/// Extension-less filenames that are source all the same.
///
/// A C project's whole build is a `Makefile`, and a container's whole environment is a
/// `Dockerfile`; an extension-only allow-list cannot see either.
const SEARCHABLE_NAMES: &[&str] = &[
    "Makefile",
    "makefile",
    "GNUmakefile",
    "Dockerfile",
    "Containerfile",
    "Justfile",
    "justfile",
    "Rakefile",
    "Gemfile",
    "Procfile",
    "CMakeLists.txt",
];

/// Largest source file offered to the index.
///
/// Past this a file is a generated artifact or a data blob, not something a person wrote to be
/// read — and reading it costs the whole search its latency.
const MAX_SOURCE_BYTES: u64 = 512 * 1024;

/// Every source file under `directory`, read as a searchable document.
///
/// This is what makes "where is this decided?" answerable by the cheap route when the answer
/// lives in code rather than prose: a miss is what sends a session back to grepping and reading
/// whole files, which is the expensive route the index exists to replace. Unreadable files,
/// oversized ones, and anything the walker already refuses (build output, `node_modules`, the
/// store, hidden directories) are skipped in silence — a search must never fail because some
/// unrelated file in the tree is a binary.
///
/// Reading and chunking a file is per-file work with nothing shared, so it is spread across
/// the machine's cores ([`in_parallel`]). Output order is the sorted file order regardless of
/// how the work was split — a search that reordered itself by how busy the machine was would
/// not be a search anybody could test.
///
/// Complexity: `O(n)` in the bytes of the files offered, over the available cores.
fn source_corpus(directory: &Path) -> Vec<Loaded> {
    let mut files = Vec::new();
    collect_source_files(directory, &mut files);
    files.sort();
    in_parallel(&files, |path| {
        let relative = path
            .strip_prefix(directory)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(path).ok()?;
        let document = doc_core::search::source_document(&relative, &text);
        (!document.blocks.is_empty()).then(|| Loaded {
            path: path.clone(),
            relative,
            document,
        })
    })
    .into_iter()
    .flatten()
    .collect()
}

/// Apply `work` to every item across the machine's cores, keeping the input's order.
///
/// The one concurrency primitive in this crate, and deliberately the plainest one: scoped
/// threads over contiguous slices, each writing its own `Vec`, joined in slice order. No
/// channel, no shared mutable state, no dependency — and no reordering, so a parallel run and
/// a serial one produce the same bytes. A single item, or a machine reporting one core, runs
/// on the calling thread rather than paying to start one.
fn in_parallel<T, R>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let chunk = items.len().div_ceil(cores.max(1)).max(1);
    if cores <= 1 || items.len() <= 1 {
        return items.iter().map(work).collect();
    }

    let work = &work;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|slice| scope.spawn(move || slice.iter().map(work).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

/// Recursively collect source files, skipping what the document walker skips.
fn collect_source_files(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if !matches!(
                name,
                "node_modules" | "target" | "build" | "dist" | "__pycache__"
            ) {
                collect_source_files(&path, found);
            }
            continue;
        }
        let searchable = SEARCHABLE_NAMES.contains(&name)
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| SEARCHABLE_SOURCE.contains(&extension));
        let sized = entry
            .metadata()
            .is_ok_and(|metadata| metadata.len() <= MAX_SOURCE_BYTES);
        if searchable && sized {
            found.push(path);
        }
    }
}

/// One search hit: a document, why it matched, and where the answer is.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The matching document.
    pub document: Loaded,
    /// Relevance score from the index; higher is a better match.
    pub score: f64,
    /// Id of the block that best matches the query — the hit's answer, so a caller can
    /// hand its text over (or read its section) without a second search.
    pub block: Option<String>,
}

/// Search a project for `query`, best matches first — its documents *and* its source files.
///
/// Documents and source rank in one index, which is why the store's own narrowed search is not
/// consulted here: two indexes would mean two scores with no honest way to order one against
/// the other, and the answer to "where is this decided?" is as often a `.rs` file as a
/// document. A miss is the expensive case — it sends the caller back to grepping and reading
/// whole files — so covering the tree matters more than narrowing it. Narrowing becomes the
/// store's to offer again once it indexes source too.
///
/// `limit` is the caller's whole budget — "Maximum hits", not one per corpus — so the result
/// never holds more than `limit` entries. Within that budget the ranking decides order; the
/// one thing it does not decide is whether a corpus is heard from at all, so a corpus holding
/// a real (above-[`drop_noise`]) answer is never shut out completely by the other — see
/// [`cap_without_shutout`] for how a seat is found for it without disturbing anyone's score
/// order.
///
/// Complexity: `O(n)` in the bytes of the project's documents and source files.
pub fn search(directory: &Path, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
    let mut documents = load_all(directory)?.documents;
    documents.extend(source_corpus(directory));
    // Tokenising is the expensive half of a search, and it is per-document work: index the
    // slices in parallel and join them (`SearchIndex::merge`). The join is order-preserving,
    // so the ranking is the ranking a single thread would have produced.
    let index = doc_core::search::SearchIndex::merge(in_parallel(&documents, |loaded| {
        doc_core::search::build_index([(loaded.relative.as_str(), &loaded.document)])
    }));
    let by_path: HashMap<&str, &Loaded> = documents
        .iter()
        .map(|loaded| (loaded.relative.as_str(), loaded))
        .collect();

    // Each corpus offers at most `limit` *candidates* here — plenty to fill the caller's
    // budget from either side, and enough to hold a rescue candidate in reserve for
    // `cap_without_shutout` — without letting a single sprawling corpus (a hundred source
    // files each mentioning the query once) cost more ranking work than the caller asked for.
    // This walk only skips over-cap results; it never reorders what `index.search` already
    // sorted, so `ranked` stays best-score-first across both corpora, interleaved.
    let mut documents_taken = 0;
    let mut source_taken = 0;
    let ranked: Vec<Hit> = index
        .search(query)
        .into_iter()
        .filter_map(|result| {
            let is_document = result.path.ends_with(".dx");
            let taken = if is_document {
                &mut documents_taken
            } else {
                &mut source_taken
            };
            if *taken >= limit {
                return None;
            }
            *taken += 1;
            by_path.get(result.path.as_str()).map(|loaded| Hit {
                block: doc_core::search::best_block_id(&loaded.document, query),
                document: (*loaded).clone(),
                score: result.score,
            })
        })
        .collect();

    let hits = cap_without_shutout(drop_noise(ranked), limit);
    crate::coverage::record(directory, query, &hits);
    Ok(hits)
}

/// Truncate `ranked` (already best-score-first) to `limit` hits, without letting that cut
/// erase one corpus entirely when it holds a surviving, above-noise answer.
///
/// The naive top slice is score-honest but can still repeat the bug this replaced: a corpus
/// with only mediocre hits can still hold every one of the first `limit` seats when the other
/// corpus's one real answer sits at position `limit + 1` — a long file that mentions the query
/// everywhere used to crowd out the document that explains it, and a question only the code
/// answers had no seat left by the time documents were done filling the list. So: take the top
/// slice, and if it wholly excludes a corpus that has a real hit waiting just past the cut,
/// that hit takes the weakest seat held by a corpus that is not down to its own last one —
/// never the sole seat of the *other* minority corpus, or the trade would just relocate the
/// shutout instead of fixing it.
fn cap_without_shutout(ranked: Vec<Hit>, limit: usize) -> Vec<Hit> {
    if limit == 0 || ranked.len() <= limit {
        return ranked;
    }
    let is_document = |hit: &Hit| hit.document.relative.ends_with(".dx");
    let mut kept: Vec<Hit> = ranked[..limit].to_vec();
    let rest = &ranked[limit..];

    for wanted in [true, false] {
        if kept.iter().any(|hit| is_document(hit) == wanted) {
            continue;
        }
        let Some(rescue) = rest.iter().find(|hit| is_document(hit) == wanted) else {
            continue;
        };
        let victim = kept.iter().rposition(|hit| {
            is_document(hit) != wanted
                && kept
                    .iter()
                    .filter(|other| is_document(other) == is_document(hit))
                    .count()
                    > 1
        });
        if let Some(position) = victim {
            kept[position] = rescue.clone();
        }
    }

    kept.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then_with(|| a.document.relative.cmp(&b.document.relative))
    });
    kept
}

/// Share of the best hit's score a hit must reach to be worth a caller's attention.
///
/// The engine returns everything that matched at all, which is right for a ranking and wrong
/// for an answer: a hit two orders of magnitude below the best one matched an incidental word,
/// and printing it costs the caller a paragraph to conclude what the score already said. This
/// was visible the moment `dx index` scaffolded a project — a placeholder line reading "TODO:
/// what docs/ is for" came back against every question, because a fresh index has few documents
/// and something in them always matches faintly.
const NOISE_FLOOR: f64 = 0.10;

/// Drop hits so far below the best one that they answer nothing.
///
/// Deliberately conservative: the cut is relative to the best hit, so a query where everything
/// scores low keeps its whole ranking, and a single strong answer is never trimmed away by the
/// weak ones around it. Ordering is untouched.
fn drop_noise(ranked: Vec<Hit>) -> Vec<Hit> {
    let Some(best) = ranked
        .iter()
        .map(|hit| hit.score)
        .fold(None::<f64>, |best, score| {
            Some(best.map_or(score, |b| b.max(score)))
        })
    else {
        return ranked;
    };
    let floor = best * NOISE_FLOOR;
    ranked
        .into_iter()
        .filter(|hit| hit.score >= floor)
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
    fn a_missing_file_is_told_apart_from_an_unresolvable_pointer() {
        // Two failures that look alike from the outside and want opposite advice: one
        // wants `dx new`, the other wants `dx sync`. A message that says neither costs
        // the reader the whole diagnosis.
        let root = scratch("missing");
        let missing = read(&root.join("nothing-here.dx")).expect_err("no file");
        assert!(missing.contains("dx new"), "{missing}");
        assert!(!missing.contains("dx sync"), "{missing}");

        fs::write(
            root.join("orphan.dx"),
            format!("~ dx1 {}\n", "0".repeat(64)),
        )
        .expect("pointer");
        let orphan = read(&root.join("orphan.dx")).expect_err("nothing behind it");
        assert!(orphan.contains("dx sync"), "{orphan}");
    }

    #[test]
    fn parallel_work_comes_back_in_the_order_it_went_in() {
        // The whole safety of the parallel corpus build: a search that reordered itself by
        // how busy the machine was would not be a search anybody could test.
        let items: Vec<usize> = (0..1000).collect();
        assert_eq!(
            in_parallel(&items, |n| n * 2),
            items.iter().map(|n| n * 2).collect::<Vec<_>>()
        );
        assert_eq!(in_parallel(&items[..1], |n| *n), vec![0]);
        assert!(in_parallel::<usize, usize>(&[], |n| *n).is_empty());
    }

    #[test]
    fn a_listing_and_a_single_read_resolve_a_pointer_the_same_way() {
        // `Sources` exists to open the store once for a listing; the one thing it must not
        // do is answer differently from the single-document path.
        let root = scratch("one-answer");
        save(&root.join("notes.dx"), &parse(NOTES)).expect("save");
        assert!(stub::is_stub(
            &fs::read_to_string(root.join("notes.dx")).expect("pointer")
        ));

        let listed = load_all(&root).expect("listing");
        assert_eq!(listed.documents.len(), 1);
        assert_eq!(
            listed.documents[0].document,
            parse(&read(&root.join("notes.dx")).expect("read"))
        );
    }

    #[test]
    fn a_folder_read_walks_sorted_and_skips_hidden_entries_and_build_caches() {
        let root = scratch("folder-read");
        fs::create_dir_all(root.join("src/deep")).expect("dirs");
        fs::create_dir_all(root.join("src/target")).expect("dirs");
        fs::create_dir_all(root.join("src/node_modules")).expect("dirs");
        fs::write(root.join("src/b.rs"), "b").expect("write");
        fs::write(root.join("src/a.rs"), "a").expect("write");
        fs::write(root.join("src/deep/c.rs"), "c").expect("write");
        fs::write(root.join("src/.hidden"), "no").expect("write");
        fs::write(root.join("src/target/junk"), "no").expect("write");
        fs::write(root.join("src/node_modules/junk"), "no").expect("write");

        let resolver = FolderResolver {
            folder: root.clone(),
        };
        let files = resolver.files_under("src").expect("a real folder answers");
        let paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "src/b.rs", "src/deep/c.rs"]);
        assert!(
            resolver.files_under("src/a.rs").is_none(),
            "a file is not a folder"
        );
        assert!(resolver.files_under("gone").is_none());
    }

    #[test]
    fn two_different_binaries_never_share_a_fingerprint_contribution() {
        // Lossy conversion collapses invalid-UTF-8 runs to identical replacement
        // characters, which once let a rebuilt wasm engine slip past a `reads=` gate.
        // A binary contributes its bytes' digest, so any byte change is a change.
        let root = scratch("binary-read");
        fs::create_dir_all(root.join("built")).expect("dirs");
        let resolver = FolderResolver {
            folder: root.clone(),
        };
        fs::write(root.join("built/engine.wasm"), [0xFF, 0xFE]).expect("write");
        let first = resolver.files_under("built").expect("folder");
        fs::write(root.join("built/engine.wasm"), [0xFE, 0xFF]).expect("write");
        let second = resolver.files_under("built").expect("folder");
        assert_ne!(first, second, "a changed binary must change the read");
        assert!(
            String::from_utf8_lossy(&[0xFF, 0xFE]) == String::from_utf8_lossy(&[0xFE, 0xFF]),
            "the lossy views really do collide — that is the trap this pins"
        );
    }

    #[test]
    fn resolver_file_resolves_dx_pointers_to_their_true_content() {
        // A .dx file is a one-line pointer on disk; FolderResolver::file() must hand back
        // the resolved document, never the pointer bytes. This is especially critical for
        // blocks declaring reads=some.dx, which use FolderResolver to fingerprint their
        // inputs — fingerprinting pointer bytes instead of content means changing a document
        // through the store (its true home) while the pointer stub stays syntactically
        // identical would not stale the block's verdict, violating the foundational rule
        // that "the resolver always hands back the true document."
        let root = scratch("resolver-pointer-resolution");

        // Create and store a document through save(), so it becomes a pointer on disk.
        let source_path = root.join("data.dx");
        let initial_content = "::paragraph id=p\nfirst version\n::end\n";
        save(&source_path, &parse(initial_content)).expect("save source");

        // Verify it is a pointer on disk.
        let raw_pointer = fs::read_to_string(&source_path).expect("read pointer");
        assert!(
            stub::is_stub(&raw_pointer),
            "save() must create a pointer, got {raw_pointer:?}"
        );
        assert!(
            !raw_pointer.contains("first version"),
            "pointer must not contain content"
        );

        // Create a resolver in the root directory and read the pointer through it.
        let resolver = FolderResolver {
            folder: root.clone(),
        };
        let resolved = resolver
            .file("data.dx")
            .expect("resolver must hand back the content");
        assert_eq!(
            resolved, initial_content,
            "FolderResolver::file() must resolve .dx pointers to their true content"
        );
        assert!(
            !resolved.starts_with("~ dx1 "),
            "resolved content must not be a pointer"
        );

        // Now update the document through the store while leaving the pointer on disk.
        let updated_content = "::paragraph id=p\nsecond version\n::end\n";
        save(&source_path, &parse(updated_content)).expect("save update");

        // Resolve through the resolver again — it must now hand back the updated content.
        let resolved_updated = resolver
            .file("data.dx")
            .expect("resolver must hand back the updated content");
        assert_eq!(
            resolved_updated, updated_content,
            "FolderResolver::file() must reflect updated content from the store"
        );
        assert_ne!(
            resolved, resolved_updated,
            "changing stored content must change what the resolver hands back"
        );
    }

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

        let listed = load_all(&root).expect("list");
        assert_eq!(listed.documents.len(), 2);
        assert!(listed.unresolved.is_empty());
        assert_eq!(listed.documents[0].relative, "kubernetes.dx");

        let hits = search(&root, "kubernetes", 10).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.relative, "kubernetes.dx");
    }

    #[test]
    fn search_works_before_any_index_exists() {
        let root = scratch("search-no-index");
        fs::write(root.join("a.dx"), NOTES).expect("write");
        let hits = search(&root, "kubernetes", 10).expect("search");
        assert_eq!(hits.len(), 1);
    }

    /// A question whose answer lives in code used to miss entirely, and a miss is what sends a
    /// session back to grepping and reading whole files — the expensive route search exists to
    /// replace.
    #[test]
    fn search_reaches_source_files_and_names_the_lines_to_read() {
        let root = scratch("search-source");
        fs::write(root.join("a.dx"), NOTES).expect("write");
        fs::create_dir_all(root.join("src")).expect("dirs");
        fs::write(
            root.join("src/confine.rs"),
            "/// Wrap the command in a deny-by-default seatbelt profile.\npub fn confine() {}\n",
        )
        .expect("write");

        let hits = search(&root, "seatbelt profile", 5).expect("search");
        let hit = hits
            .iter()
            .find(|hit| hit.document.relative == "src/confine.rs")
            .expect("the code answers, so the code is found");
        assert!(
            hit.block.as_deref().is_some_and(|id| id.starts_with('L')),
            "a source hit names the lines to read, got {:?}",
            hit.block
        );
    }

    /// Reach is the whole value of the cheap route, so the languages a project is actually
    /// written in have to be in it. An Objective-C and Metal project was invisible to search
    /// until this list grew; a C project's Makefile still is, unless a name can be source too.
    /// First contact, measured: a reader asks in their own words and the project only
    /// speaks its own. The question has to reach the file that holds the answer even when
    /// no line in the repository is written the way the question was.
    #[test]
    fn a_question_asked_in_the_readers_words_reaches_the_file_written_in_the_projects() {
        let root = scratch("search-vocabulary");
        fs::write(root.join("a.dx"), NOTES).expect("write");
        fs::create_dir_all(root.join("src")).expect("dirs");
        fs::write(
            root.join("src/bitpack.c"),
            "uint64_t bitpack_getu(const uint64_t *w, int i) {\n    return w[i];\n}\n",
        )
        .expect("write");
        fs::write(
            root.join("src/audio.c"),
            "void render(float *samples) {\n    mix(samples);\n}\n",
        )
        .expect("write");

        let hits = search(&root, "how are weights packed", 5).expect("search");
        assert!(
            hits.iter()
                .any(|hit| hit.document.relative == "src/bitpack.c"),
            "the project writes `bitpack`, never `packed`: {:?}",
            hits.iter()
                .map(|hit| &hit.document.relative)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_reaches_the_languages_a_project_is_written_in() {
        let root = scratch("search-languages");
        fs::write(root.join("a.dx"), NOTES).expect("write");
        fs::create_dir_all(root.join("engine")).expect("dirs");
        fs::write(
            root.join("engine/backend_metal.m"),
            "// The Metal compute backend dispatches the xorpopcount kernel.\n",
        )
        .expect("write");
        fs::write(
            root.join("engine/Makefile"),
            "# Building needs the Xcode command line tools.\nall:\n\tclang -O3 backend_metal.m\n",
        )
        .expect("write");

        let metal = search(&root, "metal compute backend", 5).expect("search");
        assert!(
            metal
                .iter()
                .any(|hit| hit.document.relative == "engine/backend_metal.m"),
            "an .m file answers, so it must be reachable: {:?}",
            metal
                .iter()
                .map(|hit| &hit.document.relative)
                .collect::<Vec<_>>()
        );

        let build = search(&root, "Xcode command line tools", 5).expect("search");
        assert!(
            build
                .iter()
                .any(|hit| hit.document.relative == "engine/Makefile"),
            "a Makefile has no extension and is still source: {:?}",
            build
                .iter()
                .map(|hit| &hit.document.relative)
                .collect::<Vec<_>>()
        );
    }

    /// The parameter's contract is "Maximum hits" (`rust/doc-cli/src/mcp/tools.rs`), not
    /// "maximum hits per corpus" — `search` used to return up to `2 * limit` (`limit`
    /// documents plus `limit` source hits) whenever both corpora had enough matches, which
    /// silently broke that contract for any caller who trusted `--limit N` to mean N results.
    #[test]
    fn search_never_returns_more_than_the_limit_across_both_corpora() {
        let root = scratch("search-total-cap");
        for name in ["a", "b", "c"] {
            fs::write(
                root.join(format!("{name}.dx")),
                format!("# {name}\n\nrollout notes\n"),
            )
            .expect("doc");
        }
        fs::create_dir_all(root.join("src")).expect("dirs");
        for name in ["x", "y", "z"] {
            fs::write(
                root.join(format!("src/{name}.rs")),
                "// rollout notes\nfn rollout() {}\n",
            )
            .expect("source");
        }

        let hits = search(&root, "rollout", 2).expect("search");
        assert!(
            hits.len() <= 2,
            "limit is the caller's whole budget, not one per corpus: {hits:?}"
        );
    }

    /// Reported (`reports.dx`): grouping every document ahead of every source hit, regardless
    /// of score, buried a source file that answered a question none of the documents did
    /// behind several documents that only shared the question's ordinary words — past
    /// `--limit 5` in the report, so the answer never reached the caller. `cap_without_shutout`
    /// is what replaced the grouping; pinned directly with synthetic hits, the way
    /// `search_drops_hits_far_below_the_best_one` pins `drop_noise`, so the test states the
    /// rule rather than depending on a real corpus's exact BM25 arithmetic.
    #[test]
    fn cap_without_shutout_rescues_a_shut_out_corpus_without_moving_the_best_hits() {
        let hit = |relative: &str, score: f64| Hit {
            document: Loaded {
                path: PathBuf::from(relative),
                relative: relative.to_string(),
                document: parse("# A document\n"),
            },
            score,
            block: None,
        };

        let ranked = vec![
            hit("doc-1.dx", 9.0),
            hit("doc-2.dx", 8.0),
            hit("doc-3.dx", 7.0),
            hit("answer.rs", 6.0),
        ];

        let kept = cap_without_shutout(ranked, 3);
        let names: Vec<&str> = kept
            .iter()
            .map(|hit| hit.document.relative.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["doc-1.dx", "doc-2.dx", "answer.rs"],
            "the weakest document gives up its seat to the only code hit, which a naive \
             top-3-by-score slice would otherwise have shut out entirely"
        );
    }

    /// When the naive top slice already includes both corpora — the ordinary, common case —
    /// nothing should move: the rescue only ever fires on an actual shutout.
    #[test]
    fn cap_without_shutout_leaves_an_already_mixed_slice_alone() {
        let hit = |relative: &str, score: f64| Hit {
            document: Loaded {
                path: PathBuf::from(relative),
                relative: relative.to_string(),
                document: parse("# A document\n"),
            },
            score,
            block: None,
        };

        let ranked = vec![
            hit("answer.rs", 20.0),
            hit("doc-1.dx", 9.0),
            hit("doc-2.dx", 8.0),
            hit("doc-3.dx", 7.0),
        ];

        let kept = cap_without_shutout(ranked, 2);
        let names: Vec<&str> = kept
            .iter()
            .map(|hit| hit.document.relative.as_str())
            .collect();
        assert_eq!(names, vec!["answer.rs", "doc-1.dx"]);
    }

    /// A budget of one seat cannot represent two corpora, so the rescue must not fire — it
    /// would only empty the seat the better hit already earned.
    #[test]
    fn cap_without_shutout_never_empties_a_single_seat_budget() {
        let hit = |relative: &str, score: f64| Hit {
            document: Loaded {
                path: PathBuf::from(relative),
                relative: relative.to_string(),
                document: parse("# A document\n"),
            },
            score,
            block: None,
        };

        let kept = cap_without_shutout(vec![hit("doc-1.dx", 9.0), hit("answer.rs", 8.0)], 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].document.relative, "doc-1.dx");
    }

    /// A hit far below the best one matched an incidental word. Printing it costs the caller a
    /// paragraph to conclude what the score already said. The rule is tested on the ranking it
    /// governs rather than through BM25, so it pins the contract and not a scorer's arithmetic.
    #[test]
    fn search_drops_hits_far_below_the_best_one() {
        let hit = |relative: &str, score: f64| Hit {
            document: Loaded {
                path: PathBuf::from(relative),
                relative: relative.to_string(),
                document: parse("# A document\n"),
            },
            score,
            block: None,
        };

        let kept = drop_noise(vec![
            hit("answer.dx", 8.9),
            hit("related.dx", 6.2),
            hit("incidental.dx", 0.061),
        ]);

        assert_eq!(
            kept.iter()
                .map(|hit| hit.document.relative.as_str())
                .collect::<Vec<_>>(),
            vec!["answer.dx", "related.dx"],
            "a hit at 0.7% of the best answers nothing; one at 70% still might"
        );
    }

    /// The floor is a share of the best hit, so it can never empty a result set: whatever the
    /// scores are, the best one is always at 100% of itself.
    #[test]
    fn search_never_drops_the_best_hit() {
        let only = Hit {
            document: Loaded {
                path: PathBuf::from("only.dx"),
                relative: "only.dx".to_string(),
                document: parse("# A document\n"),
            },
            score: 0.0001,
            block: None,
        };
        assert_eq!(drop_noise(vec![only]).len(), 1);
    }

    /// The floor is relative, so a query whose every hit is weak keeps its whole ranking —
    /// a weak best answer is still the answer, and trimming it would leave the caller nothing.
    #[test]
    fn search_keeps_every_hit_when_they_are_all_equally_weak() {
        let root = scratch("search-weak");
        fs::write(
            root.join("one.dx"),
            "# One\n\nA note mentioning chunks once.\n",
        )
        .expect("write");
        fs::write(
            root.join("two.dx"),
            "# Two\n\nA note mentioning chunks once.\n",
        )
        .expect("write");

        let hits = search(&root, "chunks", 5).expect("search");
        assert_eq!(hits.len(), 2, "equal scores are all above a relative floor");
    }

    /// Documents and source are two bounded answers, documents first: a project's documents are
    /// written to answer questions, and letting a long source file crowd them out loses the
    /// better answer to the noisier one.
    #[test]
    fn search_caps_the_limit_and_still_rescues_a_corpus_it_would_otherwise_shut_out() {
        // Four source files each repeat the query three times — enough BM25 saturation to
        // outscore the document's single, plainer mention — so a pure top-2-by-score slice
        // would fill both seats from source and lose the document's only candidate entirely.
        let root = scratch("search-grouping");
        fs::write(root.join("a.dx"), NOTES).expect("write");
        fs::create_dir_all(root.join("src")).expect("dirs");
        for index in 0..4 {
            fs::write(
                root.join(format!("src/n{index}.rs")),
                "// kubernetes scheduling kubernetes scheduling kubernetes scheduling\n",
            )
            .expect("write");
        }

        let hits = search(&root, "kubernetes scheduling", 2).expect("search");
        assert_eq!(
            hits.len(),
            2,
            "limit is the caller's whole budget, not one per corpus: {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h.document.relative == "a.dx"),
            "the document is its corpus's only candidate; a shutout would lose it: {hits:?}"
        );
        assert!(
            hits.iter().any(|h| !h.document.relative.ends_with(".dx")),
            "source is not crowded out either — it is what earned the best score here: {hits:?}"
        );
    }

    #[test]
    fn documents_still_resolve_when_the_medium_is_read_only() {
        // The dev.dx sandbox mounts the repository read-only; so does a checkout on
        // mounted media. A store that cannot be *written* must still answer reads —
        // this once fell through to the packs silently, then (part 57) errored instead;
        // both were wrong.
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("read-only-medium");
        let path = root.join("notes.dx");
        save(&path, &parse(NOTES)).expect("save");

        let mut locked = Vec::new();
        for entry in [root.join(".doc"), root.join(".doc/index.db")] {
            let mut permissions = fs::metadata(&entry).expect("meta").permissions();
            permissions.set_mode(if entry.is_dir() { 0o555 } else { 0o444 });
            fs::set_permissions(&entry, permissions).expect("lock");
            locked.push(entry);
        }

        let resolved = read(&path);
        let listed = load_all(&root);

        // Unlock before asserting so a failure does not leave scratch undeletable.
        for entry in locked {
            let mut permissions = fs::metadata(&entry).expect("meta").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&entry, permissions).expect("unlock");
        }

        assert_eq!(resolved.expect("a read-only store still answers"), NOTES);
        assert_eq!(
            listed.expect("a read-only listing works").documents.len(),
            1
        );
    }

    #[test]
    fn remove_forgets_a_stored_document_and_sync_does_not_bring_it_back() {
        let root = scratch("rm-stored");
        let path = root.join("notes.dx");
        save(&path, &parse(NOTES)).expect("save");

        let removed = remove(&path).expect("remove");
        assert!(removed.stored, "the store held it");
        assert!(!path.exists(), "the stub went with it");
        assert!(read(&path).is_err(), "nothing resolves any more");

        let report = sync(&root).expect("sync");
        assert!(report.restored.is_empty(), "a deliberate delete sticks");
        assert!(load_all(&root).expect("list").documents.is_empty());
    }

    #[test]
    fn remove_deletes_a_plain_file_nothing_adopted() {
        let root = scratch("rm-plain");
        let path = root.join("loose.dx");
        fs::write(&path, NOTES).expect("write");

        let removed = remove(&path).expect("remove");
        assert!(!removed.stored);
        assert!(removed.file);
        assert!(!path.exists());
    }

    #[test]
    fn remove_of_nothing_says_there_is_nothing() {
        let root = scratch("rm-nothing");
        let error = remove(&root.join("ghost.dx")).expect_err("nothing there");
        assert!(error.contains("nothing to remove"), "{error}");
    }

    #[test]
    fn remove_in_a_fresh_clone_deletes_from_the_packs_too() {
        // Stubs and packs committed, index.db absent: rm must register the deletion in
        // the store it builds, or the next sync would restore the document.
        let root = scratch("rm-fresh-clone");
        let path = root.join("notes.dx");
        save(&path, &parse(NOTES)).expect("save");
        fs::remove_file(root.join(".doc/index.db")).expect("drop index");

        let removed = remove(&path).expect("remove");
        assert!(removed.stored, "the packs held it");
        assert!(!path.exists());

        let report = sync(&root).expect("sync");
        assert!(report.restored.is_empty(), "the deletion held through sync");
        assert!(load_all(&root).expect("list").documents.is_empty());
    }

    #[test]
    fn a_listing_reports_what_it_cannot_resolve_instead_of_hiding_it() {
        let root = scratch("listing-unresolved");
        save(&root.join("good.dx"), &parse(NOTES)).expect("save");
        // A pointer whose content is in no store or pack: the ghost a silent skip hides.
        fs::write(
            root.join("orphan.dx"),
            stub::render("::paragraph id=p\nelsewhere\n::end\n"),
        )
        .expect("write stub");

        let listing = load_all(&root).expect("list");
        assert_eq!(listing.documents.len(), 1);
        assert_eq!(listing.documents[0].relative, "good.dx");
        assert_eq!(listing.unresolved.len(), 1);
        assert_eq!(listing.unresolved[0].0, "orphan.dx");
        assert!(
            listing.unresolved[0].1.contains("dx sync"),
            "the error says what to run"
        );
    }

    #[test]
    fn a_subdirectory_listing_still_answers_from_the_workspace_store() {
        let root = scratch("scoped-listing");
        save(&root.join("docs/inside.dx"), &parse(NOTES)).expect("save");
        save(&root.join("outside.dx"), &parse(NOTES)).expect("save");
        // Drop the stub: only the workspace store at the root knows this document now.
        fs::remove_file(root.join("docs/inside.dx")).expect("drop stub");

        let listing = load_all(&root.join("docs")).expect("list");
        let paths: Vec<&str> = listing
            .documents
            .iter()
            .map(|entry| entry.relative.as_str())
            .collect();
        assert_eq!(
            paths,
            ["docs/inside.dx"],
            "the store answers, the scope holds"
        );
    }

    #[test]
    fn a_subdirectory_search_scopes_the_store_answer() {
        let root = scratch("scoped-search");
        save(&root.join("docs/inside.dx"), &parse(NOTES)).expect("save");
        save(&root.join("outside.dx"), &parse(NOTES)).expect("save");

        let hits = search(&root.join("docs"), "kubernetes", 10).expect("search");
        let paths: Vec<&str> = hits
            .iter()
            .map(|hit| hit.document.relative.as_str())
            .collect();
        assert_eq!(paths, ["docs/inside.dx"]);
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

    #[test]
    fn the_first_write_in_a_fresh_worktree_restores_the_index_instead_of_emptying_the_pack() {
        // What a `git worktree add` hands you: the committed pack, the pointers, and no
        // index. The write used to build an empty store and export it over the pack, which
        // is how a second document became a pointer naming content nothing held.
        let root = scratch("fresh-worktree");
        save(&root.join("kept.dx"), &parse(NOTES)).expect("first document");
        save(&root.join("edited.dx"), &parse(NOTES)).expect("second document");
        let (pack_path, _) = pack::paths(&root);
        let committed = fs::read(&pack_path).expect("the pack is committed");

        fs::remove_file(root.join(".doc/index.db")).expect("worktrees carry no index");
        for leftover in [".doc/index.db-wal", ".doc/index.db-shm"] {
            let _ = fs::remove_file(root.join(leftover));
        }
        fs::write(&pack_path, &committed).expect("restore the committed pack");

        save(
            &root.join("edited.dx"),
            &parse(&NOTES.replace("kubernetes scheduling notes", "rewritten in the worktree")),
        )
        .expect("the write lands rather than refusing");

        assert!(
            read(&root.join("kept.dx"))
                .expect("kept.dx still resolves")
                .contains("kubernetes"),
            "the document nobody touched must survive the first write"
        );
        assert!(
            read(&root.join("edited.dx"))
                .expect("edited.dx resolves")
                .contains("rewritten in the worktree"),
            "the edit must be what the pointer names"
        );
    }
}
