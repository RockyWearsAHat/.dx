//! `doc-wasm` — WebAssembly bindings around [`doc_core`].
//!
//! This crate compiles the runtime-agnostic [`doc_core`] engine to `wasm32` and exposes a
//! thin, documented JavaScript API via `wasm-bindgen`. The point is that there is one
//! engine: the VS Code editor and the github.com extension call the *same* Rust DOCSRC
//! parser, renderer, and codecs the `dx` binary calls, so no surface can drift from
//! another.
//!
//! # Boundary conventions
//! - Documents cross the boundary as JSON strings shaped by [`dto::DocumentDto`]
//!   (`camelCase`, `type` for block kind — the shape JavaScript hosts expect).
//! - Binary payloads (packed documents, compressed frames, hash inputs) cross as byte
//!   slices (`&[u8]` in, `Vec<u8>` out), which `wasm-bindgen` maps to `Uint8Array`.
//! - Fallible operations return `Result<_, JsValue>`; the error is a JS string. Infallible
//!   operations (`parse`, `sha256_hex`, `compress`, `decompress`-of-trusted-input is still
//!   fallible) return their value directly.
//!
//! All glue here is `unsafe`-free; the only `unsafe` is the code `wasm-bindgen` generates,
//! which is expected and confined to the macro output.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod dto;

use dto::{ChunkDto, DocumentDto};
use wasm_bindgen::prelude::*;

/// Serialize any displayable error into a JS `Error`-friendly string value.
fn js_err(message: impl core::fmt::Display) -> JsValue {
    JsValue::from_str(&message.to_string())
}

/// Parse DOCSRC (`.dx`) source text into a canonical document, returned as JSON.
///
/// `path` is the document's path. Like the core [`doc_core::format::parse`], it does not
/// influence the canonical block output — a filename-derived title is a host concern, not
/// part of the format core — but it is accepted so a host can pass what it already has and
/// keep its call sites uniform. `text` is the raw `.dx` source.
///
/// The returned JSON is a [`dto::DocumentDto`]: canonical blocks (unique ids, clamped
/// heading levels, recovered inline forms) plus any `@doc` header metadata.
#[wasm_bindgen]
pub fn parse(path: &str, text: &str) -> String {
    let _ = path; // Reserved for API parity; the core parser is path-independent.
    let document = doc_core::format::parse(text);
    let dto = DocumentDto::from(&document);
    // Serialization of our own DTO never fails (no maps, no non-string keys).
    serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string())
}

/// Render a document (JSON, [`dto::DocumentDto`] shape) back to canonical DOCSRC text.
///
/// The blocks are re-normalized before stringify, exactly as the core
/// [`doc_core::format::stringify`] does, so the result is the canonical `.dx` serialization
/// regardless of minor input irregularities. Returns an error if `doc_json` is not valid
/// JSON of the expected shape.
#[wasm_bindgen]
pub fn stringify(doc_json: &str) -> Result<String, JsValue> {
    let dto: DocumentDto = serde_json::from_str(doc_json).map_err(js_err)?;
    let document = (&dto).into();
    Ok(doc_core::format::stringify(&document))
}

/// Split a document (JSON, [`dto::DocumentDto`] shape) into its content-addressed chunks,
/// returned as JSON: `[{"hash": "<sha256 hex>", "text": "<canonical block source>"}, …]`.
///
/// This is the same split the native store uses, so an editor and the CLI address a block
/// identically. Returns an error if `doc_json` is not valid JSON of the expected shape.
#[wasm_bindgen]
pub fn split_chunks(doc_json: &str) -> Result<String, JsValue> {
    let dto: DocumentDto = serde_json::from_str(doc_json).map_err(js_err)?;
    let document = (&dto).into();
    let chunks: Vec<ChunkDto> = doc_core::chunk::split(&document)
        .into_iter()
        .map(|chunk| ChunkDto {
            hash: chunk.hash,
            text: chunk.text,
        })
        .collect();
    serde_json::to_string(&chunks).map_err(js_err)
}

/// Rebuild canonical `.dx` source from ordered per-block chunk texts (a JSON array of
/// strings), the inverse of [`split_chunks`].
///
/// Returns an error if `texts_json` is not a JSON array of strings.
#[wasm_bindgen]
pub fn join_chunks(texts_json: &str) -> Result<String, JsValue> {
    let texts: Vec<String> = serde_json::from_str(texts_json).map_err(js_err)?;
    Ok(doc_core::chunk::join(texts.iter().map(String::as_str)))
}

/// Render `.dx` source to a self-contained HTML page.
///
/// This is the same [`doc_core::render::html`] the CLI and the screenshotter call, so a
/// document shown in an editor webview is byte-identical to the one a person opens in a
/// browser or an agent sees as an image. `theme` is `auto`, `light`, or `dark`; `fragment`
/// emits just the document container (for embedding in an existing page) instead of a full
/// document; `document_css` opts into the document's own `::style` blocks.
#[wasm_bindgen]
pub fn render_html(text: &str, theme: &str, fragment: bool, document_css: bool) -> String {
    let document = doc_core::format::parse(text);
    doc_core::render::html(
        &document,
        &doc_core::render::HtmlOptions {
            theme: doc_core::render::Theme::parse(theme),
            fragment,
            document_css,
            ..doc_core::render::HtmlOptions::default()
        },
    )
}

/// Render `.dx` source to Markdown, the text view an agent or a diff reads.
///
/// `include_ids` prefixes each block with a `<!-- block:<id> <kind> -->` marker.
#[wasm_bindgen]
pub fn render_text(text: &str, include_ids: bool) -> String {
    let document = doc_core::format::parse(text);
    doc_core::render::text(
        &document,
        &doc_core::render::TextOptions {
            include_ids,
            ..doc_core::render::TextOptions::default()
        },
    )
}

/// Outline `.dx` source: a JSON array of one entry per block.
///
/// Each entry is `{ id, kind, level, preview, chars, runnable }` — the map a caller needs
/// to jump to, fetch, or edit one part of a document.
#[wasm_bindgen]
pub fn render_outline(text: &str) -> String {
    let document = doc_core::format::parse(text);
    let rows: Vec<serde_json::Value> = doc_core::render::outline(&document)
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "kind": row.kind,
                "level": row.level,
                "preview": row.preview,
                "chars": row.chars,
                "runnable": row.runnable,
            })
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

/// Slice one section out of `.dx` source, returning canonical DOCSRC for that part.
///
/// `selector` is a block id: a heading id yields that whole section, any other id yields
/// that block plus its captured output. Returns an empty string when nothing matches.
#[wasm_bindgen]
pub fn render_section(text: &str, selector: &str) -> String {
    let document = doc_core::format::parse(text);
    doc_core::render::section(&document, selector)
        .map(|slice| doc_core::format::stringify(&slice))
        .unwrap_or_default()
}

/// The canonical `.dx` source of one document held in a `DXCP1` pack.
///
/// This is the whole reason a pack is committed rather than kept in a database: given the
/// pack bytes and a workspace-relative path, any host — a browser extension reading a
/// repository on github.com, an editor, a build — can recover the true document without the
/// `dx` binary, a SQLite file, or a network service. The pack is the content; this is how it
/// is read.
///
/// Returns an error when the bytes are not a pack, or when the pack holds no such path.
#[wasm_bindgen]
pub fn pack_document(pack: &[u8], path: &str) -> Result<String, JsValue> {
    let decoded = doc_core::chunk::decode_pack(pack).map_err(js_err)?;
    decoded
        .source(path)
        .ok_or_else(|| js_err(format!("the pack holds no document at {path}")))
}

/// Every document path a `DXCP1` pack carries, as a JSON array of strings.
///
/// Useful for saying *what is* in a pack when a lookup misses — a reader who asked for the
/// wrong path should be told the right ones, not handed nothing.
#[wasm_bindgen]
pub fn pack_paths(pack: &[u8]) -> Result<String, JsValue> {
    let decoded = doc_core::chunk::decode_pack(pack).map_err(js_err)?;
    let paths: Vec<&str> = decoded
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    serde_json::to_string(&paths).map_err(js_err)
}

/// The stylesheet [`render_html`] pages are styled with.
///
/// A host that embeds a rendered fragment — a webview, a page on github.com — needs the
/// same CSS the standalone page inlines, or the document would read as one thing in one
/// place and another somewhere else.
#[wasm_bindgen]
pub fn stylesheet() -> String {
    doc_core::render::stylesheet().to_string()
}

/// Compress bytes into a `dxz` frame, the store's framed format.
///
/// `doc_core::compress` chooses the smallest encoding (currently DEFLATE, or stored when
/// nothing beats it) and names the codec in the frame's four magic bytes. Infallible; it
/// returns the frame as a `Uint8Array`.
#[wasm_bindgen]
pub fn compress(input: &[u8]) -> Vec<u8> {
    doc_core::compress::compress(input)
}

/// Decompress any `dxz` frame, whichever codec its magic names — including `DXZ1` (LZSS),
/// which is no longer written but is decoded forever.
///
/// Returns the original bytes, or an error if `frame` is not a valid `dxz` frame.
#[wasm_bindgen]
pub fn decompress(frame: &[u8]) -> Result<Vec<u8>, JsValue> {
    doc_core::compress::decompress(frame).map_err(|err| js_err(format!("{err:?}")))
}

/// Compute the lowercase hex SHA-256 digest of `input`, byte-identical to the reference.
#[wasm_bindgen]
pub fn sha256_hex(input: &[u8]) -> String {
    doc_core::digest::sha256_hex(input)
}

/// Compute the lowercase hex SHA-1 digest of `input`, byte-identical to the reference.
#[wasm_bindgen]
pub fn sha1_hex(input: &[u8]) -> String {
    doc_core::digest::sha1_hex(input)
}

/// Build a search index over a set of documents and run a query in one call.
///
/// `docs_json` is a JSON array of `[path, document]` pairs, where each `document` is a
/// [`dto::DocumentDto`]. `query` is the search string. The result is a JSON array of
/// `{ "path": string, "score": number }` hits, sorted by descending score with ascending
/// path as a stable tie-break (exactly [`doc_core::search::SearchIndex::search`]). An empty
/// query yields an empty array. Returns an error if `docs_json` is malformed.
#[wasm_bindgen]
pub fn build_index_and_search(docs_json: &str, query: &str) -> Result<String, JsValue> {
    let pairs: Vec<(String, DocumentDto)> = serde_json::from_str(docs_json).map_err(js_err)?;
    let docs: Vec<(String, doc_core::model::Document)> = pairs
        .iter()
        .map(|(path, dto)| (path.clone(), dto.into()))
        .collect();
    let index = doc_core::search::build_index(&docs);
    let hits = index.search(query);
    let serializable: Vec<HitDto> = hits
        .into_iter()
        .map(|hit| HitDto {
            path: hit.path,
            score: hit.score,
        })
        .collect();
    serde_json::to_string(&serializable).map_err(js_err)
}

/// JSON shape of a single search hit returned by [`build_index_and_search`].
#[derive(serde::Serialize)]
struct HitDto {
    /// The matched document's path.
    path: String,
    /// Relevance score (higher is better).
    score: f64,
}

/// The editable text of one block — what a surface puts in the field when a reader clicks it.
///
/// The same [`doc_core::edit`] the `dx` command line calls, so a block edited in the VS Code
/// webview and the same block edited in DX.app are edited by one implementation.
///
/// Returns an error naming the ids that do exist when `id` names no block.
#[wasm_bindgen]
pub fn block_source(text: &str, id: &str) -> Result<String, JsValue> {
    doc_core::edit::block_source(text, id).map_err(js_err)
}

/// Replace one block's body, returning the whole document's canonical source.
///
/// Every other block comes back byte-identical.
///
/// Returns an error naming the ids that do exist when `id` names no block.
#[wasm_bindgen]
pub fn set_block(text: &str, id: &str, body: &str) -> Result<String, JsValue> {
    doc_core::edit::set_block(text, id, body).map_err(js_err)
}

/// Add a block of `kind` after the block called `after`, or at the top when `after` is empty.
///
/// Returns JSON `{"source": "<canonical .dx>", "id": "<the new block's id>"}` — the id is
/// what lets the caller put the reader's cursor in the block they just created.
///
/// Returns an error when `kind` is not authorable or `after` names no block.
#[wasm_bindgen]
pub fn insert_block(text: &str, after: &str, kind: &str, body: &str) -> Result<String, JsValue> {
    let anchor = (!after.is_empty()).then_some(after);
    let insertion = doc_core::edit::Insertion {
        kind,
        body,
        ..doc_core::edit::Insertion::default()
    };
    let (source, id) = doc_core::edit::insert_after(text, anchor, &insertion).map_err(js_err)?;
    serde_json::to_string(&InsertedDto { source, id }).map_err(js_err)
}

/// Take one block out, returning the document's canonical source without it.
///
/// Returns an error naming the ids that do exist when `id` names no block.
#[wasm_bindgen]
pub fn remove_block(text: &str, id: &str) -> Result<String, JsValue> {
    doc_core::edit::remove_block(text, id).map_err(js_err)
}

/// The HTML one block renders to with `body` in it, saving nothing.
///
/// This is what keeps a page rendered while a reader writes on it: the surface hands over
/// the characters currently in the field and gets back the block as it will be read. The
/// same [`doc_core::edit::preview_block`] DX.app reaches through `dx render --block`, so
/// what a reader sees mid-sentence in an editor and on a Mac is the same markup.
///
/// `theme` is `auto`, `light`, or `dark`; `document_css` opts into the document's own
/// `::style` blocks, matching [`render_html`] so a previewed block is styled like the page
/// it sits in.
///
/// Returns an error naming the ids that do exist when `id` names no block.
#[wasm_bindgen]
pub fn preview_block(
    text: &str,
    id: &str,
    body: &str,
    theme: &str,
    document_css: bool,
) -> Result<String, JsValue> {
    doc_core::edit::preview_block(
        text,
        id,
        body,
        &doc_core::render::HtmlOptions {
            theme: doc_core::render::Theme::parse(theme),
            document_css,
            ..doc_core::render::HtmlOptions::default()
        },
    )
    .map_err(js_err)
}

/// JSON shape returned by [`insert_block`].
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InsertedDto {
    /// The document's canonical source, with the new block in it.
    source: String,
    /// The id the new block was given.
    id: String,
}
