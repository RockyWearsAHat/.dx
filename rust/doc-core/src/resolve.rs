//! References a document makes past its own edge, and how they are filled in.
//!
//! A document may name five things it does not itself carry:
//!
//! - **A sibling file**, on a `::code src=path` block. The file is the source of truth —
//!   the listing a reader sees, the text `dx run` executes — and the document shows it as
//!   it is *now*, so a page reviewing code can never drift from the code.
//! - **A sibling coded page**, on a `::view src=path` block. The same reference shown the
//!   other way up: not the file's text but the page it renders to, framed. Hydration also
//!   inlines the page's own relative stylesheets ([`file_references`] is how a prefetching
//!   host learns about them), because the frame it renders in has no folder to fetch from.
//!   The `src` may carry a fragment (`src=site/index.html#visit`): the file before the
//!   `#` is read, and the frame shows just that element — one screen per section of a
//!   page too tall for one frame.
//! - **One block of a sibling document**, on a board node line (`- plan.dx#step x= y=`).
//!   The block lives once, in its own document, and every board that names it shows the
//!   current content instead of a copy.
//! - **A sibling stylesheet**, on a `::style src=path.css` block. The document's dress is
//!   the file's current text, so one sheet dresses every page of a site instead of being
//!   pasted into each. The fetched CSS passes through the same sanitizer as an inline
//!   body (`render::escape::escape_style`) — a file is not a trust upgrade — and a file
//!   that cannot be read leaves the block's CSS empty: the page renders undressed, and
//!   the report says why, because a sentence injected as CSS would just be a broken rule.
//! - **A sibling picture**, on an `::image src=path` block. The file's current bytes are
//!   embedded as a `data:` URI, so the rendered page carries its own artwork wherever it
//!   is shown — a capture made from a scratch directory, an editor webview, a PNG export.
//!   Raster formats only (SVG is a document that can script), and a remote or `data:` src
//!   travels as written.
//!
//! All of them are filled in by [`hydrate`], which asks an injected [`Resolver`] for bytes and
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
    /// The raw bytes of the file at `path` — how an `::image src=` is embedded.
    ///
    /// Defaulted to `None` so text-only hosts keep working unchanged; a host that can
    /// read bytes (the CLI's folder resolver) overrides it, and where it stays `None`
    /// an image resolves to its missing-file sentence like any other reference.
    fn binary(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    /// Every file under the directory at `path`, as `(path, text)` pairs whose paths
    /// stay relative to the document's own folder, sorted by path — how a `reads=`
    /// naming a folder is expanded into the files that decide staleness.
    ///
    /// Hidden entries and the caches a build regenerates (`target`, `node_modules`) are
    /// left out: an input tree is its sources, and a folder that changes on every run
    /// would keep its own verdict permanently stale. Bytes that are not UTF-8 arrive
    /// lossily decoded, which loses nothing for the one thing this feeds — a fingerprint.
    ///
    /// Defaulted to `None` so hosts that cannot walk a folder keep working unchanged;
    /// where it stays `None`, a directory declared as a read fails with the same
    /// sentence a missing file does.
    fn files_under(&self, _path: &str) -> Option<Vec<(String, String)>> {
        None
    }
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
/// For hosts that cannot read on demand: a browser asks [`Provided::pending`] what is still
/// missing, fetches it — sibling documents from the repository pack, files from wherever
/// the host can honestly get them — adds it here, and asks again until nothing is pending.
/// Anything not gathered resolves to its sentence, exactly as a missing file would.
#[derive(Debug, Default)]
pub struct Provided {
    /// `(path, text)` pairs answering [`Resolver::file`].
    files: Vec<(String, String)>,
    /// `(path, source)` pairs answering [`Resolver::document`].
    documents: Vec<(String, String)>,
    /// `(path, bytes)` pairs answering [`Resolver::binary`].
    binaries: Vec<(String, Vec<u8>)>,
    /// Paths the host tried and could not get, so [`Provided::pending`] stops asking.
    absent: Vec<String>,
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

    /// Supply the raw bytes of the file at `path` — what an `::image src=` embeds.
    pub fn add_binary(&mut self, path: &str, bytes: &[u8]) {
        self.binaries.push((path.to_string(), bytes.to_vec()));
    }

    /// Record that `path` was asked for and could not be got.
    ///
    /// The reference still resolves to its sentence — an absent path and a path that was
    /// never mentioned render the same, which is the honest thing to show. What this adds
    /// is termination: [`Provided::pending`] will not ask for it again.
    pub fn add_absent(&mut self, path: &str) {
        self.absent.push(path.to_string());
    }

    /// What the host still has to fetch before [`hydrate`] will find everything in hand.
    ///
    /// This is the prefetch walk, stated once. A document's own [`references`] open it; each
    /// file already gathered extends it with what that file names in turn ([`file_references`]
    /// — a `::view` page naming its stylesheets); and anything already held, or already
    /// marked absent, is left out. The host's whole job is the loop:
    ///
    /// ```text
    /// while !provided.pending(&document).is_empty() {
    ///     for reference in provided.pending(&document) { fetch it, or mark it absent }
    /// }
    /// ```
    ///
    /// which terminates because every round either fills a path or marks it absent, and the
    /// set of paths a document and its files can name is finite. Hosts differ only in *how*
    /// they fetch — the filesystem in an editor, the repository's raw route in a browser —
    /// and never in what to fetch or when to stop.
    #[must_use]
    pub fn pending(&self, document: &Document) -> Vec<Reference> {
        let mut out: Vec<Reference> = Vec::new();
        let mut consider = |reference: Reference| {
            let held = match &reference {
                Reference::File(path) => self.files.iter().any(|(held, _)| held == path),
                Reference::Document(path) => self.documents.iter().any(|(held, _)| held == path),
            };
            let given_up = self.absent.iter().any(|path| path == reference.path());
            if !held && !given_up && !out.contains(&reference) {
                out.push(reference);
            }
        };
        for reference in references(document) {
            consider(reference);
        }
        for (path, text) in &self.files {
            for reference in file_references(path, text) {
                consider(reference);
            }
        }
        out
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

    fn binary(&self, path: &str) -> Option<Vec<u8>> {
        self.binaries
            .iter()
            .find(|(held, _)| held == path)
            .map(|(_, bytes)| bytes.clone())
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
/// [`Provided::pending`] reports these so a host that cannot fetch synchronously (a browser)
/// can gather everything first and answer the resolver from memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// A sibling file whose text becomes a `::code src=` block's listing.
    File(String),
    /// A sibling document, one block of which a board shows (`path` only — the host
    /// fetches the whole document; the block is picked here).
    Document(String),
}

impl Reference {
    /// The word a host names this kind of reference by — the `kind` field of the JSON the
    /// wasm engine and the daemon both hand back, written once so the two doors cannot
    /// spell it differently.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::File(_) => "file",
            Self::Document(_) => "document",
        }
    }

    /// The path referenced, relative to the document's own folder.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::File(path) | Self::Document(path) => path,
        }
    }
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

/// A view's `src`, split at its `#fragment`: the file to read, and the element the
/// frame shows.
///
/// Only a fragment naming a CSS-addressable id (`[A-Za-z0-9_-]+`) is recognized — that
/// is every id a real page carries, and it is what keeps the fragment inert when it is
/// stamped into a selector. Any other `#` stays part of the filename, which the path
/// law and the resolver then judge as they always have.
fn view_fragment(src: &str) -> (&str, Option<&str>) {
    match src.rsplit_once('#') {
        Some((path, fragment))
            if !fragment.is_empty()
                && fragment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') =>
        {
            (path, Some(fragment))
        }
        _ => (src, None),
    }
}

/// The page with a style appended that shows only the element `#anchor` (its ancestors
/// keep their place; everything else is hidden), so a fragment view is that section's
/// current render. A sandboxed frame has no URL to carry a fragment to, and no render
/// may need a script — a selector is the whole mechanism.
fn opened_at(page: &str, anchor: &str) -> String {
    format!(
        "{page}<style>body :not(#{anchor}):not(#{anchor} *):not(:has(#{anchor}))\
         {{display:none !important}}</style>"
    )
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
/// This opens the prefetch walk. A host that must gather before it can answer a [`Resolver`]
/// asks [`Provided::pending`] instead, which starts here and keeps going through what the
/// gathered files name in turn; a host that can read on demand needs neither.
#[must_use]
pub fn references(document: &Document) -> Vec<Reference> {
    let mut out: Vec<Reference> = Vec::new();
    let mut push = |reference: Reference| {
        if !out.contains(&reference) {
            out.push(reference);
        }
    };
    for block in &document.blocks {
        if matches!(block.kind.as_str(), "code" | "view" | "style") && !block.src.is_empty() {
            let src = match block.kind.as_str() {
                "view" => view_fragment(&block.src).0,
                _ => block.src.as_str(),
            };
            if let Some(path) = confined(src) {
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
/// block. Then every `::code src=` and `::style src=` body — including one a foreign
/// block just brought in, whose path is re-rooted in *its* document's folder — is
/// replaced with the file's current text, and every `::image src=` naming a sibling
/// raster file is embedded as a `data:` URI of its current bytes. A reference that
/// cannot be resolved becomes a sentence in the block's place naming the path — a
/// listing's body, an image's alt; a style block stays empty instead, because a sentence
/// in a style body would be parsed as CSS — and an [`Unresolved`] is returned for it, so
/// a caller that must not proceed on a partial document (running it, say) can tell which
/// blocks are affected.
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
                if matches!(block.kind.as_str(), "code" | "view" | "style") && !block.src.is_empty()
                {
                    // The reference's path is relative to the document that owns it.
                    block.src = joined(folder(path), &block.src).unwrap_or_default();
                }
                // An image's folder reference re-roots the same way; a remote or `data:`
                // src is not a folder reference and travels as written.
                if block.kind == "image" && confined(&block.src).is_some() {
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
        if !matches!(block.kind.as_str(), "code" | "view" | "style") || block.src.is_empty() {
            continue;
        }
        let (path, fragment) = match block.kind.as_str() {
            "view" => view_fragment(&block.src),
            _ => (block.src.as_str(), None),
        };
        let (path, fragment) = (path.to_string(), fragment.map(str::to_string));
        let text = confined(&path).and_then(|path| resolver.file(path));
        match text {
            Some(text) if block.kind == "view" => {
                // The framed page fetches nothing relative — its frame has no folder —
                // so its own stylesheets ride in with it.
                let (page, missing) = inline_stylesheets(folder(&path), &text, resolver);
                block.text = match &fragment {
                    Some(anchor) => opened_at(&page, anchor),
                    None => page,
                };
                for path in missing {
                    unresolved.push(Unresolved {
                        block: block.id.clone(),
                        sentence: format!(
                            "{path} could not be read here — the page names it as a \
                             stylesheet, and the view shows the page without its dress."
                        ),
                        reference: path,
                    });
                }
            }
            Some(text) => {
                block.text = text.trim_end_matches(['\n', '\r']).to_string();
            }
            None if block.kind == "view" => {
                // The renderer shows the sentence for an empty view body; putting it in
                // `text` would frame the sentence as if it were the page.
                unresolved.push(Unresolved {
                    block: block.id.clone(),
                    reference: block.src.clone(),
                    sentence: format!(
                        "{} could not be shown here — the view is the page's current \
                         render, and the file was not found inside this document's folder.",
                        block.src
                    ),
                });
            }
            None if block.kind == "style" => {
                // A sentence written into a style body would be parsed as (broken) CSS;
                // the honest render is the page undressed, with the report saying why.
                unresolved.push(Unresolved {
                    block: block.id.clone(),
                    reference: block.src.clone(),
                    sentence: format!(
                        "{} could not be read here — the document's dress is the file's \
                         current text, and the file was not found inside this document's \
                         folder.",
                        block.src
                    ),
                });
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

    for block in &mut document.blocks {
        if block.kind != "image" {
            continue;
        }
        // Only a confined folder path is a reference to fill; a remote URL, a `data:` URI
        // already carrying its bytes, and an empty src all travel exactly as written.
        let Some(path) = confined(&block.src).map(str::to_string) else {
            continue;
        };
        // SVG is a document that can script, so it is never inlined; a file outside the
        // media allow-list keeps its reference too, rather than being embedded as a type
        // a page must not carry.
        let Some(media) = image_media_type(&path) else {
            continue;
        };
        match resolver.binary(&path) {
            Some(bytes) if bytes.len() > MAX_IMAGE_BYTES => {
                let sentence = format!(
                    "{path} is too large to embed — an image travels inside the rendered \
                     page, and this one is {} bytes against a limit of {MAX_IMAGE_BYTES}. \
                     Export a smaller file.",
                    bytes.len()
                );
                block.alt = sentence.clone();
                unresolved.push(Unresolved {
                    block: block.id.clone(),
                    reference: path,
                    sentence,
                });
            }
            Some(bytes) => {
                block.src = format!("data:{media};base64,{}", crate::base64::encode(&bytes));
            }
            None => {
                let sentence = format!(
                    "{path} could not be read here — the image is the file's current \
                     bytes, and the file was not found inside this document's folder."
                );
                // The page says what happened where the picture would have been.
                block.alt = sentence.clone();
                unresolved.push(Unresolved {
                    block: block.id.clone(),
                    reference: path,
                    sentence,
                });
            }
        }
    }

    unresolved
}

/// Largest image file hydration will embed. A `data:` URI travels inside every render of
/// the page — a capture, an export, a tool result — and an unbounded one is a page nothing
/// can carry ("bound anything sized by untrusted input"; the document names the file).
/// Public so a writing surface can warn at save time instead of leaving the reader to
/// discover the refusal at render.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// The media type an `::image` may embed, judged by the file's extension.
///
/// Every value is one of [`crate::render::escape::RASTER_IMAGE_MEDIA_TYPES`] — the same
/// allow-list author markup is held to, pinned by test. SVG is deliberately absent: it
/// is a document that can script, not a picture.
fn image_media_type(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// The sibling files a fetched file names in turn — a view's page naming its stylesheets.
///
/// `path` is the file's own reference (so its links resolve against *its* folder), `text`
/// its current content. This is what keeps the prefetch walk going past its first round;
/// [`Provided::pending`] applies it to every gathered file, so no host has to. Non-HTML
/// files name nothing.
#[must_use]
pub fn file_references(path: &str, text: &str) -> Vec<Reference> {
    let mut out = Vec::new();
    for href in stylesheet_links(text) {
        if let Some(joined) = joined(folder(path), &href) {
            let reference = Reference::File(joined);
            if !out.contains(&reference) {
                out.push(reference);
            }
        }
    }
    out
}

/// The relative stylesheet hrefs a page's `<link rel="stylesheet">` tags name, in order.
///
/// Absolute and scheme-carrying hrefs are not returned: the frame can fetch those itself,
/// exactly as a browser would. This is a scan, not a parse — it reads the tags a real
/// page writes and leaves anything it does not recognize alone.
fn stylesheet_links(page: &str) -> Vec<String> {
    let lower = page.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(start) = lower[cursor..].find("<link") {
        let start = cursor + start;
        let Some(end) = lower[start..].find('>') else {
            break;
        };
        let end = start + end + 1;
        let tag = &page[start..end];
        cursor = end;
        let rel = tag_attribute(tag, "rel").unwrap_or_default();
        if !rel
            .split_ascii_whitespace()
            .any(|word| word.eq_ignore_ascii_case("stylesheet"))
        {
            continue;
        }
        if let Some(href) = tag_attribute(tag, "href") {
            if !href.contains(':') && !href.starts_with('/') && confined(&href).is_some() {
                out.push(href);
            }
        }
    }
    out
}

/// The value of `name="…"` (or `name='…'`, or unquoted) inside one tag's text.
fn tag_attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut cursor = 0;
    loop {
        let at = lower[cursor..].find(name)?;
        let at = cursor + at;
        cursor = at + name.len();
        // A whole attribute name: preceded by whitespace, followed by `=`.
        let preceded = tag[..at].ends_with(|ch: char| ch.is_ascii_whitespace());
        let rest = tag[cursor..].trim_start();
        if !preceded || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let value = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or(""),
            _ => value
                .split(|ch: char| ch.is_ascii_whitespace() || ch == '>')
                .next()
                .unwrap_or(""),
        };
        return Some(value.to_string());
    }
}

/// Replace each relative `<link rel="stylesheet" href=…>` in `page` with an inline
/// `<style>` holding the file's current text, resolved against `base_folder`.
///
/// Returns the rewritten page and the joined paths that could not be read — those tags
/// are left exactly as written, so the page still says what it meant. Stylesheets the
/// frame can fetch itself (absolute, scheme-carrying) are untouched.
fn inline_stylesheets(
    base_folder: &str,
    page: &str,
    resolver: &dyn Resolver,
) -> (String, Vec<String>) {
    let lower = page.to_ascii_lowercase();
    let mut out = String::with_capacity(page.len());
    let mut missing = Vec::new();
    let mut cursor = 0;
    while let Some(start) = lower[cursor..].find("<link") {
        let start = cursor + start;
        let Some(end) = lower[start..].find('>') else {
            break;
        };
        let end = start + end + 1;
        out.push_str(&page[cursor..start]);
        let tag = &page[start..end];
        cursor = end;

        let rel = tag_attribute(tag, "rel").unwrap_or_default();
        let is_stylesheet = rel
            .split_ascii_whitespace()
            .any(|word| word.eq_ignore_ascii_case("stylesheet"));
        let href = tag_attribute(tag, "href").unwrap_or_default();
        let relative =
            is_stylesheet && !href.contains(':') && !href.starts_with('/') && !href.is_empty();
        let path = relative.then(|| joined(base_folder, &href)).flatten();
        match path {
            Some(path) => match resolver.file(&path) {
                Some(css) => {
                    out.push_str("<style>\n");
                    out.push_str(css.trim_end_matches(['\n', '\r']));
                    out.push_str("\n</style>");
                }
                None => {
                    missing.push(path);
                    out.push_str(tag);
                }
            },
            None => out.push_str(tag),
        }
    }
    out.push_str(&page[cursor..]);
    (out, missing)
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

    /// The walk every gathering host used to carry its own copy of — a queue, a dedupe, and
    /// a second round for what the fetched files name. Two copies drifted apart once; this
    /// is the one implementation both of them now call.
    #[test]
    fn the_prefetch_walk_opens_with_the_documents_own_references() {
        let document = parse(
            "::code id=lib src=src/lib.rs lang=rust\n::end\n\
             ::board id=plan\n- notes.dx#step x=0 y=0 w=10 h=10\n::end\n",
        );
        assert_eq!(
            Provided::new().pending(&document),
            vec![
                Reference::File("src/lib.rs".to_string()),
                Reference::Document("notes.dx".to_string()),
            ]
        );
    }

    #[test]
    fn a_gathered_page_extends_the_walk_with_the_stylesheets_it_names() {
        let document = parse("::view id=site src=site/index.html\n::end\n");
        let held = map(
            &[(
                "site/index.html",
                "<link rel=\"stylesheet\" href=\"look.css\">",
            )],
            &[],
        );
        // Relative to the *page's* folder, so the host fetches site/look.css.
        assert_eq!(
            held.pending(&document),
            vec![Reference::File("site/look.css".to_string())]
        );
    }

    #[test]
    fn the_walk_ends_when_everything_is_held_or_given_up_on() {
        let document = parse(
            "::code id=lib src=src/lib.rs lang=rust\n::end\n\
             ::board id=plan\n- notes.dx#step x=0 y=0 w=10 h=10\n::end\n",
        );
        let mut held = map(&[("src/lib.rs", "fn main() {}\n")], &[]);
        // A path the host could not fetch is not asked for twice — without this the loop
        // would spin forever on a document naming a file that is not there.
        assert_eq!(
            held.pending(&document),
            vec![Reference::Document("notes.dx".to_string())]
        );
        held.add_absent("notes.dx");
        assert!(held.pending(&document).is_empty());
    }

    #[test]
    fn a_reference_names_its_kind_and_path_the_one_way() {
        // Both engine doors write these two words into their JSON; they are stated here.
        assert_eq!(Reference::File("a.css".to_string()).kind(), "file");
        assert_eq!(Reference::Document("a.dx".to_string()).kind(), "document");
        assert_eq!(Reference::File("a.css".to_string()).path(), "a.css");
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

    /// The field-report gap: a page judged by its pictures could not carry them. An
    /// `::image src=` is a reference like any other, and hydration fills it with the
    /// file's current bytes — as a `data:` URI, so the render is self-contained wherever
    /// it is shown: a capture from a scratch directory, an editor webview, a PNG export.
    #[test]
    fn an_image_src_is_filled_into_a_data_uri() {
        let mut document = parse("::image id=shot src=shots/frame.png\nthe frame\n::end\n");
        let mut provided = Provided::new();
        provided.add_binary("shots/frame.png", &[0x89, b'P', b'N', b'G']);
        let notes = hydrate(&mut document, &provided);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(document.blocks[0].src, "data:image/png;base64,iVBORw==");
        assert_eq!(document.blocks[0].alt, "the frame");
    }

    #[test]
    fn a_missing_image_is_reported_and_the_page_says_so() {
        let mut document = parse("::image id=shot src=shots/gone.png\n::end\n");
        let notes = hydrate(&mut document, &Nowhere);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].block, "shot");
        assert!(notes[0].sentence.contains("shots/gone.png"));
        // The page says what happened where the picture would have been.
        assert!(document.blocks[0].alt.contains("shots/gone.png"));
    }

    /// SVG is a document that can script, so it is never inlined — the reference stays
    /// exactly as written, like any src the path law or the media allow-list refuses.
    #[test]
    fn an_svg_image_is_never_inlined_and_a_remote_src_is_left_untouched() {
        let mut document = parse(
            "::image id=vector src=art/logo.svg\n::end\n\n\
             ::image id=remote src=https://example.com/x.png\n::end\n\n\
             ::image id=carried src=\"data:image/png;base64,AAAA\"\n::end\n",
        );
        let mut provided = Provided::new();
        provided.add_binary("art/logo.svg", b"<svg/>");
        let notes = hydrate(&mut document, &provided);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(document.blocks[0].src, "art/logo.svg");
        assert_eq!(document.blocks[1].src, "https://example.com/x.png");
        assert_eq!(document.blocks[2].src, "data:image/png;base64,AAAA");
    }

    /// The drift guard between the two halves of one rule: hydration must never embed a
    /// media type the escape allow-list would refuse in author markup.
    #[test]
    fn every_embeddable_media_type_is_one_the_escape_allow_list_keeps() {
        for extension in ["png", "jpg", "jpeg", "gif", "webp"] {
            let media = image_media_type(&format!("art/file.{extension}"))
                .expect("a raster extension embeds");
            assert!(
                crate::render::escape::RASTER_IMAGE_MEDIA_TYPES.contains(&media),
                "{media} is not on the escape allow-list"
            );
        }
        assert_eq!(image_media_type("art/file.svg"), None);
        assert_eq!(image_media_type("no-extension"), None);
    }

    #[test]
    fn an_image_past_the_size_bound_is_a_sentence_not_an_embed() {
        let mut document = parse("::image id=big src=big.png\n::end\n");
        let mut provided = Provided::new();
        provided.add_binary("big.png", &vec![0u8; MAX_IMAGE_BYTES + 1]);
        let notes = hydrate(&mut document, &provided);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].sentence.contains("too large"));
        assert_eq!(
            document.blocks[0].src, "big.png",
            "src must stay a reference"
        );
        assert!(document.blocks[0].alt.contains("too large"));
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
        let mut document = parse(
            "::code id=x src=../../etc/passwd\n::end\n\n\
             ::style id=dress src=../theme.css\n::end\n",
        );
        let notes = hydrate(&mut document, &Panics);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn a_style_src_is_the_files_current_css() {
        let mut document = parse("::style id=dress src=theme/site.css\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(&[("theme/site.css", "p { color: red }\n")], &[]),
        );
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(document.blocks[0].text, "p { color: red }");
    }

    #[test]
    fn a_missing_style_src_is_a_report_not_injected_css() {
        // A sentence written into a style body would be parsed as broken CSS; the page
        // renders undressed and the report names the path instead.
        let mut document = parse("::style id=dress src=theme/site.css\n::end\n");
        let notes = hydrate(&mut document, &Nowhere);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].block, "dress");
        assert!(notes[0].sentence.contains("theme/site.css"));
        assert_eq!(document.blocks[0].text, "");
    }

    #[test]
    fn a_view_is_the_pages_current_markup_with_its_stylesheet_inlined() {
        let mut document = parse("::view id=shipped src=site/index.html\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(
                &[
                    (
                        "site/index.html",
                        "<html><head><link rel=\"stylesheet\" href=\"site.css\">\
                         </head><body>Hi</body></html>",
                    ),
                    ("site/site.css", "body { color: red }\n"),
                ],
                &[],
            ),
        );
        assert!(notes.is_empty(), "{notes:?}");
        let page = &document.blocks[0].text;
        assert!(page.contains("<style>\nbody { color: red }\n</style>"));
        assert!(!page.contains("<link"), "the link tag is replaced");
        assert!(page.contains("<body>Hi</body>"));
    }

    #[test]
    fn a_fragment_view_reads_the_file_and_shows_its_element() {
        let mut document = parse("::view id=v src=site/index.html#visit\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(
                &[
                    (
                        "site/index.html",
                        "<link rel=\"stylesheet\" href=\"site.css\">\
                         <body><section id=visit>Book</section></body>",
                    ),
                    ("site/site.css", "body { color: red }\n"),
                ],
                &[],
            ),
        );
        assert!(notes.is_empty(), "{notes:?}");
        let page = &document.blocks[0].text;
        assert!(page.contains("<section id=visit>Book</section>"));
        assert!(
            page.contains("<style>\nbody { color: red }\n</style>"),
            "the fragment strips before the file read, so the stylesheet still inlines"
        );
        assert!(
            page.contains(":not(#visit"),
            "the appended style narrows the frame to the named element"
        );
    }

    #[test]
    fn a_fragment_strips_from_the_prefetch_list_but_not_from_an_unsafe_name() {
        let document = parse(
            "::view id=a src=site/index.html#visit\n::end\n\n\
             ::view id=b src=odd#name.html\n::end\n",
        );
        assert_eq!(
            references(&document),
            vec![
                Reference::File("site/index.html".to_string()),
                Reference::File("odd#name.html".to_string()),
            ],
            "a safe id is a fragment; a `#` amid other punctuation stays filename"
        );
    }

    #[test]
    fn a_fragment_never_reaches_a_selector_unvetted() {
        // `#…{}` would close the crop rule and open an author-written one.
        let mut document = parse("::view id=v src=index.html#x{}*{background:url(evil)}\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(&[("index.html", "<body>Hi</body>")], &[]),
        );
        assert_eq!(notes.len(), 1, "treated as a filename, and not found");
        assert!(document.blocks[0].text.is_empty());
    }

    #[test]
    fn a_missing_view_is_reported_and_its_body_stays_empty() {
        // The sentence is the renderer's to show — framing it would draw it as a page.
        let mut document = parse("::view id=shipped src=site/gone.html\n::end\n");
        let notes = hydrate(&mut document, &Nowhere);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].block, "shipped");
        assert!(notes[0].sentence.contains("site/gone.html"));
        assert!(document.blocks[0].text.is_empty());
    }

    #[test]
    fn a_views_missing_stylesheet_keeps_the_tag_and_says_so() {
        let mut document = parse("::view id=shipped src=index.html\n::end\n");
        let notes = hydrate(
            &mut document,
            &map(
                &[(
                    "index.html",
                    "<link rel=stylesheet href=gone.css><body>Hi</body>",
                )],
                &[],
            ),
        );
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].reference, "gone.css");
        assert!(document.blocks[0]
            .text
            .contains("<link rel=stylesheet href=gone.css>"));
    }

    #[test]
    fn a_pages_absolute_stylesheets_are_the_frames_own_to_fetch() {
        let page = "<link rel=\"stylesheet\" href=\"https://fonts.example/f.css\">\
                    <link rel=\"stylesheet\" href=\"/site.css\">\
                    <link rel=\"preconnect\" href=\"other.css\">";
        assert_eq!(file_references("site/index.html", page), Vec::new());
        let mut document = parse("::view id=v src=site/index.html\n::end\n");
        let notes = hydrate(&mut document, &map(&[("site/index.html", page)], &[]));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            document.blocks[0].text, page,
            "nothing to inline, nothing touched"
        );
    }

    #[test]
    fn file_references_lists_a_pages_relative_stylesheets_against_its_own_folder() {
        let page = "<LINK REL=\"Stylesheet\" HREF=\"site.css\">\
                    <link rel=\"stylesheet\" href=\"site.css\">";
        assert_eq!(
            file_references("site/index.html", page),
            vec![Reference::File("site/site.css".to_string())]
        );
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
             ::style id=dress src=theme/site.css\n::end\n\n\
             ::board id=map\n- plan.dx#one x=0 y=0\n- plan.dx#two x=0 y=200\n::end\n",
        );
        assert_eq!(
            references(&document),
            vec![
                Reference::File("src/lib.rs".to_string()),
                Reference::File("theme/site.css".to_string()),
                Reference::Document("plan.dx".to_string()),
            ]
        );
    }
}
