//! References a document makes past its own edge, and how they are filled in.
//!
//! A document may name two things it does not itself carry:
//!
//! - **A sibling file**, on a `::code src=path` block. The file is the source of truth —
//!   the listing a reader sees, the text `dx run` executes — and the document shows it as
//!   it is *now*, so a page reviewing code can never drift from the code.
//! - **One block of a sibling document**, on a board node line (`- plan.dx#step x= y=`).
//!   The block lives once, in its own document, and every board that names it shows the
//!   current content instead of a copy.
//!
//! Both are filled in by [`hydrate`], which asks an injected [`Resolver`] for bytes and
//! nothing else. The resolver is the host's — the filesystem in the CLI, workspace files
//! in an editor, the repository origin in a browser — and it is transport only: the path
//! law ([`confined`]), the reference grammar, and what happens to the fetched bytes all
//! live here, so a reference resolves identically on every surface. Hydration mutates a
//! parsed, in-memory copy and is never serialized: the stored document keeps the
//! reference, not the referent.
//!
//! Hydration is a read. It executes nothing, writes nothing, and a reference that cannot
//! be resolved becomes a sentence saying so in the block's place — never silence, never
//! an empty block.

use crate::format::parse;
use crate::model::{Block, Document};
use crate::render::board;

/// What a document may reach: sibling files and sibling documents, by relative path.
///
/// Implementations supply bytes and make no decisions. `document` must hand back the
/// **true document source** — a host whose `.dx` files are store pointers resolves them
/// before answering, exactly as every other read does. Both return `None` when the path
/// names nothing; [`hydrate`] turns that into a sentence in the document.
pub trait Resolver {
    /// The text of the file at `path`, relative to the document's own folder.
    fn file(&self, path: &str) -> Option<String>;
    /// The DOCSRC source of the sibling document at `path` (a `.dx` path, resolved
    /// through the store when the file on disk is a pointer).
    fn document(&self, path: &str) -> Option<String>;
}

/// A resolver that holds nothing, for surfaces that render without a workspace.
///
/// Every reference hydrated against it becomes its "could not be resolved" sentence,
/// which is the honest render of a document taken away from its folder.
pub struct Nowhere;

impl Resolver for Nowhere {
    fn file(&self, _path: &str) -> Option<String> {
        None
    }
    fn document(&self, _path: &str) -> Option<String> {
        None
    }
}

/// A resolver answering from content gathered ahead of time.
///
/// For hosts that cannot read on demand: a browser asks [`references`] what a document
/// needs, fetches it — sibling documents from the repository pack, files from wherever
/// the host can honestly get them — and renders against the gathered set. Anything not
/// gathered resolves to its sentence, exactly as a missing file would.
#[derive(Debug, Default)]
pub struct Provided {
    /// `(path, text)` pairs answering [`Resolver::file`].
    files: Vec<(String, String)>,
    /// `(path, source)` pairs answering [`Resolver::document`].
    documents: Vec<(String, String)>,
}

impl Provided {
    /// An empty set: every reference resolves to its sentence.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply the text of the file at `path`.
    pub fn add_file(&mut self, path: &str, text: &str) {
        self.files.push((path.to_string(), text.to_string()));
    }

    /// Supply the DOCSRC source of the document at `path`.
    pub fn add_document(&mut self, path: &str, source: &str) {
        self.documents.push((path.to_string(), source.to_string()));
    }
}

impl Resolver for Provided {
    fn file(&self, path: &str) -> Option<String> {
        self.files
            .iter()
            .find(|(held, _)| held == path)
            .map(|(_, text)| text.clone())
    }

    fn document(&self, path: &str) -> Option<String> {
        self.documents
            .iter()
            .find(|(held, _)| held == path)
            .map(|(_, source)| source.clone())
    }
}

/// One reference hydration could not fill, and the sentence now standing in its place.
///
/// `block` is the id whose content is affected — the `::code` block for a listing, or
/// the reference id itself for a board node — which is what lets `dx run` refuse to
/// execute a listing that is a sentence about a missing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    /// Id of the affected block.
    pub block: String,
    /// The reference as the document wrote it.
    pub reference: String,
    /// The sentence reported, and put in the block's body for a listing.
    pub sentence: String,
}

/// One reference a document makes, as [`hydrate`] will resolve it.
///
/// [`references`] reports these so a host that cannot fetch synchronously (a browser)
/// can gather everything first and answer the resolver from memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// A sibling file whose text becomes a `::code src=` block's listing.
    File(String),
    /// A sibling document, one block of which a board shows (`path` only — the host
    /// fetches the whole document; the block is picked here).
    Document(String),
}

/// The path law: a reference stays inside the document's own folder, or it is nothing.
///
/// Accepts a relative path walking downward (`src/main.rs`, `notes/plan.dx`) and returns
/// it with any leading `./` removed. Refuses everything else — absolute paths, `~`,
/// drive letters and URL schemes (anything with `:`), backslashes, and any `.` or `..`
/// segment — because a document is something you were handed, and "render this page"
/// must not be a way to read a file the page has no claim to.
#[must_use]
pub fn confined(path: &str) -> Option<&str> {
    let path = path.trim();
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.is_empty() || path.starts_with(['/', '\\', '~']) {
        return None;
    }
    if path.contains(':') || path.contains('\\') {
        return None;
    }
    let clean = path
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    clean.then_some(path)
}

/// Read a board node id as a cross-document reference: `path.dx#block-id`.
///
/// A plain id (no `#`, or a path that is not a `.dx`, or one [`confined`] refuses) is not
/// a reference — it stays a name for this document's own block, exactly as boards have
/// always read it.
fn document_ref(id: &str) -> Option<(&str, &str)> {
    let (path, block) = id.split_once('#')?;
    if block.is_empty() || !path.to_ascii_lowercase().ends_with(".dx") {
        return None;
    }
    confined(path).map(|path| (path, block))
}

/// The folder of `path`, as a prefix for joining — empty for a bare filename.
fn folder(path: &str) -> &str {
    path.rfind('/').map_or("", |index| &path[..index])
}

/// A path relative to `base_folder`, re-checked against the path law.
fn joined(base_folder: &str, path: &str) -> Option<String> {
    let relative = confined(path)?;
    if base_folder.is_empty() {
        return Some(relative.to_string());
    }
    confined(&format!("{base_folder}/{relative}")).map(str::to_string)
}

/// Every reference `document` makes, in reading order, deduplicated.
///
/// This is the prefetch list for hosts that must gather before they can answer a
/// [`Resolver`]; a host that can read on demand never needs it.
#[must_use]
pub fn references(document: &Document) -> Vec<Reference> {
    let mut out: Vec<Reference> = Vec::new();
    let mut push = |reference: Reference| {
        if !out.contains(&reference) {
            out.push(reference);
        }
    };
    for block in &document.blocks {
        if block.kind == "code" && !block.src.is_empty() {
            if let Some(path) = confined(&block.src) {
                push(Reference::File(path.to_string()));
            }
        }
        if block.kind == "board" {
            for node in board::nodes(&block.text) {
                if let Some((path, _)) = document_ref(&node.id) {
                    push(Reference::Document(path.to_string()));
                }
            }
        }
    }
    out
}

/// Fill in everything `document` references, in place, and report what could not be.
///
/// Board node references are resolved first: each `path.dx#block` line gains a hidden
/// copy of the named block under exactly that id, so the board draws it like any sibling
/// block. Then every `::code src=` body — including one a foreign block just brought in,
/// whose path is re-rooted in *its* document's folder — is replaced with the file's
/// current text. A reference that cannot be resolved becomes a sentence in the block's
/// body naming the path, and an [`Unresolved`] is returned for it, so a caller that
/// must not proceed on a partial document (running it, say) can tell which blocks are
/// affected.
///
/// The result is for viewing and running, never for saving: nothing here is canonical
/// text, and serializing a hydrated document would turn references back into copies.
pub fn hydrate(document: &mut Document, resolver: &dyn Resolver) -> Vec<Unresolved> {
    let mut unresolved = Vec::new();

    for reference in foreign_nodes(document) {
        let Some((path, block_id)) = document_ref(&reference) else {
            continue;
        };
        match foreign_block(resolver, path, block_id) {
            Some(mut block) => {
                if block.kind == "code" && !block.src.is_empty() {
                    // The listing's path is relative to the document that owns it.
                    block.src = joined(folder(path), &block.src).unwrap_or_default();
                }
                block.id = reference.clone();
                block.hidden = true;
                document.blocks.push(block);
            }
            None => unresolved.push(Unresolved {
                block: reference.clone(),
                sentence: format!(
                    "{reference} could not be resolved here — the document or its block \
                     was not found. Check the path against the document's folder, and \
                     run `dx sync` if its content has not been adopted."
                ),
                reference,
            }),
        }
    }

    for block in &mut document.blocks {
        if block.kind != "code" || block.src.is_empty() {
            continue;
        }
        let text = confined(&block.src).and_then(|path| resolver.file(path));
        match text {
            Some(text) => {
                block.text = text.trim_end_matches(['\n', '\r']).to_string();
            }
            None => {
                let sentence = format!(
                    "{} could not be read here — the listing is the file's current \
                     content, and the file was not found inside this document's folder.",
                    block.src
                );
                block.text = sentence.clone();
                unresolved.push(Unresolved {
                    block: block.id.clone(),
                    reference: block.src.clone(),
                    sentence,
                });
            }
        }
    }

    unresolved
}

/// The cross-document node ids every board in `document` names, minus any already
/// present as blocks, deduplicated in board order.
fn foreign_nodes(document: &Document) -> Vec<String> {
    let mut wanted: Vec<String> = Vec::new();
    for block in &document.blocks {
        if block.kind != "board" {
            continue;
        }
        for node in board::nodes(&block.text) {
            if document_ref(&node.id).is_some()
                && document.block_index(&node.id).is_none()
                && !wanted.contains(&node.id)
            {
                wanted.push(node.id);
            }
        }
    }
    wanted
}

/// One block of the sibling document at `path`, parsed from the source the resolver
/// hands back.
fn foreign_block(resolver: &dyn Resolver, path: &str, block_id: &str) -> Option<Block> {
    let source = resolver.document(path)?;
    let document = parse(&source);
    let index = document.block_index(block_id)?;
    Some(document.blocks[index].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(files: &[(&str, &str)], documents: &[(&str, &str)]) -> Provided {
        let mut provided = Provided::new();
        for (path, text) in files {
            provided.add_file(path, text);
        }
        for (path, source) in documents {
            provided.add_document(path, source);
        }
        provided
    }

    #[test]
    fn the_path_law_keeps_references_inside_the_folder() {
        assert_eq!(confined("src/main.rs"), Some("src/main.rs"));
        assert_eq!(confined("./notes.dx"), Some("notes.dx"));
        for refused in [
            "",
            "/etc/passwd",
            "../secrets",
            "src/../../up",
            "~/.ssh/id_rsa",
            "C:\\windows",
            "https://example.com/x",
            "a//b",
            "a/./b",
            "..",
        ] {
            assert_eq!(confined(refused), None, "{refused} must be refused");
        }
    }

    #[test]
    fn a_code_listing_is_the_files_current_text() {
        let mut document = parse("::code id=lib src=src/lib.rs lang=rust\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(&[("src/lib.rs", "pub fn answer() -> u32 { 42 }\n")], &[]),
        );
        assert!(notes.is_empty());
        assert_eq!(document.blocks[0].text, "pub fn answer() -> u32 { 42 }");
    }

    #[test]
    fn a_missing_file_is_a_sentence_in_the_blocks_place_never_silence() {
        let mut document = parse("::code id=lib src=src/gone.rs lang=rust\n::end\n");
        let notes = hydrate(&mut document, &Nowhere);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].block, "lib");
        assert!(document.blocks[0].text.contains("src/gone.rs"));
        assert!(document.blocks[0].text.contains("not found"));
    }

    #[test]
    fn an_unconfined_src_never_reaches_the_resolver() {
        struct Panics;
        impl Resolver for Panics {
            fn file(&self, path: &str) -> Option<String> {
                panic!("asked for {path}");
            }
            fn document(&self, path: &str) -> Option<String> {
                panic!("asked for {path}");
            }
        }
        let mut document = parse("::code id=x src=../../etc/passwd\n::end\n");
        let notes = hydrate(&mut document, &Panics);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn a_board_node_naming_a_sibling_document_gains_its_block() {
        let mut document = parse("::board id=map\n- plan.dx#step x=10 y=10 w=200 h=90\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(
                &[],
                &[(
                    "plan.dx",
                    "::paragraph id=step\nShip the resolver.\n::end\n",
                )],
            ),
        );
        assert!(notes.is_empty());
        let index = document
            .block_index("plan.dx#step")
            .expect("block appended");
        let block = &document.blocks[index];
        assert_eq!(block.text, "Ship the resolver.");
        assert!(
            block.hidden,
            "a foreign block lives on the board, not in the page flow"
        );
    }

    #[test]
    fn a_foreign_listings_path_is_rooted_in_its_own_document() {
        let mut document =
            parse("::board id=map\n- sub/plan.dx#listing x=0 y=0 w=200 h=90\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(
                &[("sub/main.rs", "fn main() {}\n")],
                &[(
                    "sub/plan.dx",
                    "::code id=listing src=main.rs lang=rust\n::end\n",
                )],
            ),
        );
        assert!(notes.is_empty(), "{notes:?}");
        let index = document.block_index("sub/plan.dx#listing").unwrap();
        assert_eq!(document.blocks[index].text, "fn main() {}");
    }

    #[test]
    fn a_dangling_document_reference_is_reported_and_the_board_still_renders() {
        let mut document = parse("::board id=map\n- gone.dx#step x=0 y=0\n::end\n");
        let notes = hydrate(&mut document, &Nowhere);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].block, "gone.dx#step");
        assert!(notes[0].sentence.contains("gone.dx#step"));
        assert!(document.block_index("gone.dx#step").is_none());
    }

    #[test]
    fn plain_node_ids_are_not_references() {
        let document = parse(
            "::paragraph id=here hidden\nLocal.\n::end\n\n\
             ::board id=map\n- here x=0 y=0\n::end\n",
        );
        assert_eq!(references(&document), Vec::new());
    }

    #[test]
    fn references_lists_each_path_once_for_prefetching_hosts() {
        let document = parse(
            "::code id=a src=src/lib.rs\n::end\n\n\
             ::code id=b src=src/lib.rs\n::end\n\n\
             ::board id=map\n- plan.dx#one x=0 y=0\n- plan.dx#two x=0 y=200\n::end\n",
        );
        assert_eq!(
            references(&document),
            vec![
                Reference::File("src/lib.rs".to_string()),
                Reference::Document("plan.dx".to_string()),
            ]
        );
    }
}
