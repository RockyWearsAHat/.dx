//! Legacy Markdown parsing — the fallback for input that is neither DOCSRC nor `@doc`.
//!
//! This port of `parseLegacyBlocks` recognizes the small Markdown subset the reference
//! ingests (ATX headings, `-`/`*` and `N.` lists, `>` quotes, fenced code, paragraphs) and
//! emits raw blocks that the normal normalization pass then canonicalizes.

use super::lines::build_nested_list_structure;
use super::util::{js_trim, js_trim_end, strip_leading_inline_ws, strip_leading_newlines};
use crate::model::Block;

/// Parse legacy Markdown into raw blocks (headings, lists, quotes, code fences, paragraphs).
/// Port of `parseLegacyBlocks`; used only when the input is neither DOCSRC nor `@doc`.
pub(super) fn parse_legacy_blocks(body: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut paragraph_lines: Vec<String> = Vec::new();
    let mut quote_lines: Vec<String> = Vec::new();
    let mut list_kind: Option<&'static str> = None;
    let mut list_items: Vec<(String, usize)> = Vec::new();
    let mut code_fence: Option<String> = None;
    let mut code_lines: Vec<String> = Vec::new();

    let stripped = strip_leading_newlines(body);
    for line in stripped.split('\n') {
        let trimmed_for_fence = js_trim(line);
        let is_fence = trimmed_for_fence.starts_with("```");

        if let Some(fence_lang) = code_fence.clone() {
            if is_fence {
                blocks.push(Block {
                    kind: "code".to_string(),
                    language: fence_lang,
                    text: js_trim_end(&code_lines.join("\n")).to_string(),
                    ..Block::default()
                });
                code_fence = None;
                code_lines.clear();
            } else {
                code_lines.push(line.to_string());
            }
            continue;
        }

        if is_fence {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            flush_list(&mut list_kind, &mut list_items, &mut blocks);
            flush_quote(&mut quote_lines, &mut blocks);
            code_fence = Some(js_trim(&trimmed_for_fence["```".len()..]).to_string());
            code_lines.clear();
            continue;
        }

        if let Some((level, text)) = match_markdown_heading(line) {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            flush_list(&mut list_kind, &mut list_items, &mut blocks);
            flush_quote(&mut quote_lines, &mut blocks);
            blocks.push(Block {
                kind: "heading".to_string(),
                level,
                text: js_trim(text).to_string(),
                ..Block::default()
            });
            continue;
        }

        if let Some((indent, text)) = match_markdown_bullet(line) {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            flush_quote(&mut quote_lines, &mut blocks);
            if list_kind.is_some() && list_kind != Some("bulleted-list") {
                flush_list(&mut list_kind, &mut list_items, &mut blocks);
            }
            list_kind = Some("bulleted-list");
            list_items.push((js_trim(text).to_string(), indent));
            continue;
        }

        if let Some((indent, text)) = match_markdown_numbered(line) {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            flush_quote(&mut quote_lines, &mut blocks);
            if list_kind.is_some() && list_kind != Some("numbered-list") {
                flush_list(&mut list_kind, &mut list_items, &mut blocks);
            }
            list_kind = Some("numbered-list");
            list_items.push((js_trim(text).to_string(), indent));
            continue;
        }

        if let Some(text) = match_markdown_quote(line) {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            flush_list(&mut list_kind, &mut list_items, &mut blocks);
            quote_lines.push(text.to_string());
            continue;
        }

        if js_trim(line).is_empty() {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            flush_list(&mut list_kind, &mut list_items, &mut blocks);
            flush_quote(&mut quote_lines, &mut blocks);
            continue;
        }

        paragraph_lines.push(js_trim(line).to_string());
    }

    flush_paragraph(&mut paragraph_lines, &mut blocks);
    flush_list(&mut list_kind, &mut list_items, &mut blocks);
    flush_quote(&mut quote_lines, &mut blocks);

    if let Some(fence_lang) = code_fence {
        blocks.push(Block {
            kind: "code".to_string(),
            language: fence_lang,
            text: js_trim_end(&code_lines.join("\n")).to_string(),
            ..Block::default()
        });
    }

    blocks
}

/// Flush buffered paragraph lines (joined by spaces) into a paragraph block.
fn flush_paragraph(lines: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if lines.is_empty() {
        return;
    }
    blocks.push(Block {
        kind: "paragraph".to_string(),
        text: js_trim(&lines.join(" ")).to_string(),
        ..Block::default()
    });
    lines.clear();
}

/// Flush a buffered Markdown list into a list block via the nested-structure builder.
fn flush_list(
    list_kind: &mut Option<&'static str>,
    items: &mut Vec<(String, usize)>,
    blocks: &mut Vec<Block>,
) {
    match *list_kind {
        Some(kind) if !items.is_empty() => {
            let nested = build_nested_list_structure(items);
            blocks.push(Block {
                kind: kind.to_string(),
                items: nested,
                ..Block::default()
            });
        }
        _ => {}
    }
    *list_kind = None;
    items.clear();
}

/// Flush buffered quote lines (joined by newlines) into a quote block.
fn flush_quote(lines: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if lines.is_empty() {
        return;
    }
    blocks.push(Block {
        kind: "quote".to_string(),
        text: js_trim(&lines.join("\n")).to_string(),
        ..Block::default()
    });
    lines.clear();
}

/// Match `# … #### ` Markdown headings. Returns `(level, text)`.
fn match_markdown_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 4 {
        return None;
    }
    let rest = &line[hashes..];
    let after_ws = strip_leading_inline_ws(rest)?;
    Some((hashes as u8, after_ws))
}

/// Match a Markdown bullet `[-*]\s+`. Returns `(indent, text)`.
fn match_markdown_bullet(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let body = &line[indent..];
    let rest = body.strip_prefix(['-', '*'])?;
    let text = strip_leading_inline_ws(rest)?;
    Some((indent, text))
}

/// Match a Markdown numbered item `\d+\.\s+`. Returns `(indent, text)`.
fn match_markdown_numbered(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let body = &line[indent..];
    let digits_end = body
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(body.len());
    if digits_end == 0 {
        return None;
    }
    let rest = body[digits_end..].strip_prefix('.')?;
    let text = strip_leading_inline_ws(rest)?;
    Some((indent, text))
}

/// Match a Markdown quote `>\s?…`. Returns the quoted text.
fn match_markdown_quote(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('>')?;
    // `>\s?` consumes at most one leading whitespace character.
    let trimmed = rest.strip_prefix([' ', '\t']).unwrap_or(rest);
    Some(trimmed)
}
