//! `doc-wasm` — WebAssembly bindings around [`doc_core`].
//!
//! This crate compiles the runtime-agnostic [`doc_core`] engine to `wasm32` and exposes a
//! thin, documented JavaScript API via `wasm-bindgen`. The point is parser parity: the
//! in-editor block editor calls the *same* Rust DOCSRC parser and codecs that the native
//! backend uses, eliminating the historical dual-parser drift between the webview and the
//! server.
//!
//! # Boundary conventions
//! - Documents cross the boundary as JSON strings shaped by [`dto::DocumentDto`]
//!   (`camelCase`, `type` for block kind), matching the TypeScript reference shape.
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
/// `path` is the document's path; it is accepted for API symmetry with the TypeScript
/// reference (`parseDocFile(path, text)`) but, like the core [`doc_core::format::parse`],
/// it does not influence the canonical block output — filename-derived title fallback is a
/// host concern, not part of the format core. `text` is the raw `.dx` source.
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

/// Compress bytes with the `dxz1` LZSS codec used for bundle storage.
///
/// This is infallible; it returns the compressed frame as a `Uint8Array`.
#[wasm_bindgen]
pub fn compress(input: &[u8]) -> Vec<u8> {
    doc_core::compress::compress(input)
}

/// Decompress a `dxz1` frame produced by [`compress`].
///
/// Returns the original bytes, or an error if `frame` is not a valid `dxz1` frame.
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
