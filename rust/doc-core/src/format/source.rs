//! Top-level DOCSRC source parsing: `@doc` frontmatter, strategy selection, and the
//! public [`parse`] entry point.
//!
//! This module sits above the block scanner ([`super::parse`]) and the legacy-Markdown
//! fallback ([`super::legacy`]). It chooses a parse strategy for the input, normalizes the
//! resulting blocks, and derives the document title/summary the reference would.

use super::attrs::set_attr;
use super::legacy::parse_legacy_blocks;
use super::normalize::normalize_blocks;
use super::parse::parse_docsrc_blocks;
use super::util::{js_trim, parse_value};
use crate::model::{Block, Document};

// ---------------------------------------------------------------------------
// Frontmatter (`@doc`) header parsing
// ---------------------------------------------------------------------------

/// Parsed `@doc` header: document metadata plus the remaining block body.
struct DocsrcHeader {
    title: String,
    summary: String,
    tags: Vec<String>,
    meta: Vec<(String, String)>,
    body: String,
}

/// Parse a leading `@doc` … `\n---\n` header, returning `None` when the text does not start
/// with one. Port of `parseDocsrcHeader`.
fn parse_docsrc_header(text: &str) -> Option<DocsrcHeader> {
    let marker = "\n---\n";
    let separator = text.find(marker)?;
    let header_text = &text[..separator];
    let mut header_lines = header_text.split('\n');

    let first = header_lines.next().unwrap_or("");
    if first.is_empty() || !first.starts_with("@doc") {
        return None;
    }

    let mut payload = DocsrcHeader {
        title: String::new(),
        summary: String::new(),
        tags: Vec::new(),
        meta: Vec::new(),
        body: String::new(),
    };

    for line in header_lines {
        let trimmed = js_trim(line);
        if trimmed.is_empty() {
            continue;
        }
        let colon = match trimmed.find(':') {
            Some(idx) => idx,
            None => continue,
        };
        let key = js_trim(&trimmed[..colon]);
        let value = js_trim(&trimmed[colon + 1..]);

        match key {
            "title" => payload.title = value.to_string(),
            "summary" => payload.summary = value.to_string(),
            "tags" => {
                payload.tags = value
                    .split(',')
                    .map(js_trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            _ if key.starts_with("meta.") => {
                let meta_key = js_trim(&key["meta.".len()..]);
                if !meta_key.is_empty() {
                    set_attr(&mut payload.meta, meta_key, &parse_value(value));
                }
            }
            _ => {}
        }
    }

    payload.body = text[separator + marker.len()..].to_string();
    Some(payload)
}

// ---------------------------------------------------------------------------
// Top-level parse
// ---------------------------------------------------------------------------

/// The raw, pre-normalization parse result: blocks plus optional document metadata.
struct ParsedSource {
    title: String,
    summary: String,
    tags: Vec<String>,
    meta: Vec<(String, String)>,
    blocks: Vec<Block>,
}

/// Choose a parse strategy for `text`: `@doc` frontmatter, bare DOCSRC block syntax, or
/// legacy Markdown. Port of `parseDocSource` (the JSON-object input path is intentionally
/// not ported — see [`parse`]).
fn parse_doc_source(text: &str) -> Result<ParsedSource, String> {
    if let Some(header) = parse_docsrc_header(text) {
        let blocks = parse_docsrc_blocks(&header.body)?;
        return Ok(ParsedSource {
            title: header.title,
            summary: header.summary,
            tags: header.tags,
            meta: header.meta,
            blocks,
        });
    }

    let trimmed = js_trim(text);
    if starts_with_block_line(trimmed) {
        let blocks = parse_docsrc_blocks(text)?;
        return Ok(ParsedSource {
            title: String::new(),
            summary: String::new(),
            tags: Vec::new(),
            meta: Vec::new(),
            blocks,
        });
    }

    // Legacy Markdown / plain text. (Front-matter `---` and JSON object inputs are not
    // ported; they are non-DOCSRC ingestion shapes.)
    Ok(ParsedSource {
        title: String::new(),
        summary: String::new(),
        tags: Vec::new(),
        meta: Vec::new(),
        blocks: parse_legacy_blocks(text),
    })
}

/// Whether any line of `trimmed` begins with a `::type` block opener (`/^::[a-z-]+(?:\s|$)/m`).
fn starts_with_block_line(trimmed: &str) -> bool {
    trimmed.split('\n').any(|line| {
        let after = match line.strip_prefix("::") {
            Some(rest) => rest,
            None => return false,
        };
        let type_end = after
            .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
            .unwrap_or(after.len());
        if type_end == 0 {
            return false;
        }
        let rest = &after[type_end..];
        rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_whitespace())
    })
}

/// Parse DOCSRC (or `@doc` frontmatter, or legacy Markdown) into a normalized [`Document`].
///
/// The returned document has canonical blocks (unique ids, clamped heading levels, recovered
/// inline forms) and metadata from any `@doc` header. The document `title` is left as the
/// parsed/derived value; when neither a header title nor a first heading is present it is
/// empty (the reference's filename-derived fallback is a host concern, not part of the
/// format core).
///
/// The non-canonical JSON-object input path (`text` starting with `{`) is **not** handled
/// here; pass such inputs through a JSON layer before constructing a [`Document`].
///
/// Returns a parse error document (with an error block) if DOCSRC parsing fails due to
/// structural problems like a block opener with no matching `::end`.
pub fn parse(text: &str) -> Document {
    let parsed = match parse_doc_source(text) {
        Ok(source) => source,
        Err(error_msg) => {
            // Return a document with an error block instead of panicking
            let error_block = Block {
                kind: "paragraph".to_string(),
                text: format!("Parse error: {}", error_msg),
                ..Block::default()
            };
            return Document {
                title: "Parse Error".to_string(),
                summary: error_msg,
                tags: vec![],
                meta: vec![],
                blocks: vec![error_block],
            };
        }
    };

    let blocks = if parsed.blocks.is_empty() {
        normalize_blocks(&[])
    } else {
        normalize_blocks(&parsed.blocks)
    };
    let blocks = convert_flowcharts(blocks);

    let title = if !parsed.title.is_empty() {
        parsed.title
    } else {
        first_heading_text(&blocks).to_string()
    };

    let summary = if !parsed.summary.is_empty() {
        parsed.summary
    } else {
        extract_summary(&blocks)
    };

    Document {
        title,
        summary,
        tags: parsed.tags,
        meta: parsed.meta,
        blocks,
    }
}

/// Replace every readable `::mermaid`/`::graph` block with the board it describes.
///
/// A mermaid block is not a kind this format keeps: a board is the same drawing a reader can
/// take hold of, and one interactive diagram beats two renderers that have to agree. The node
/// blocks are inserted *before* the board, so a document read by an older `dx` — which does
/// not know they are a board's nodes — still shows the labels rather than nothing.
///
/// A block whose source is not a flowchart this converter reads is left exactly as written;
/// see [`super::mermaid::convert`] for why that is the safe answer rather than an empty board.
fn convert_flowcharts(blocks: Vec<Block>) -> Vec<Block> {
    if !blocks
        .iter()
        .any(|block| matches!(block.kind.as_str(), "mermaid" | "graph"))
    {
        return blocks;
    }
    blocks
        .into_iter()
        .flat_map(|block| match block.kind.as_str() {
            "mermaid" | "graph" => super::mermaid::convert(&block).unwrap_or_else(|| vec![block]),
            _ => vec![block],
        })
        .collect()
}

/// Text of the first heading block, or empty when there is none.
fn first_heading_text(blocks: &[Block]) -> &str {
    blocks
        .iter()
        .find(|block| block.kind == "heading")
        .map(|block| block.text.as_str())
        .unwrap_or("")
}

/// First non-heading block's first content line, mirroring the reference `extractSummary`
/// over normalized blocks. Style/stylesheet/script/rule blocks contribute no text.
fn extract_summary(blocks: &[Block]) -> String {
    for block in blocks {
        if block.kind == "heading" {
            continue;
        }
        let content = block_search_text(block);
        let content = js_trim(&content);
        if !content.is_empty() {
            return content.split('\n').next().unwrap_or("").to_string();
        }
    }
    String::new()
}

/// Searchable text of a block, matching the reference `blockText` (presentation-only blocks
/// and rules yield empty; lists/checklists join item text with newlines).
fn block_search_text(block: &Block) -> String {
    match block.kind.as_str() {
        "style" | "stylesheet" | "script" | "rule" => String::new(),
        "image" => {
            let alt = js_trim(&block.alt);
            if alt.is_empty() {
                js_trim(&block.src).to_string()
            } else {
                alt.to_string()
            }
        }
        "checklist" | "bulleted-list" | "numbered-list" => block
            .items
            .iter()
            .map(|item| js_trim(&item.text).to_string())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => block.text.clone(),
    }
}
