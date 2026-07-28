//! The DOCSRC block scanner: turn a `.dx` block body into raw, un-normalized blocks.
//!
//! This module owns the block-assembly loop (`parseDocsrcBlocks`/`pushBlock`): it walks the
//! body line by line, opens and closes `::type … ::end` blocks, and shapes per-type fields.
//! The per-line matchers it drives live in [`super::lines`]; the top-level strategy
//! selection and public `parse` entry point live in [`super::source`]; attribute scanning in
//! [`super::attrs`].

use super::attrs::{attr, parse_leading_attributes, Attr};
use super::lines::{
    build_nested_list_structure, parse_block_header, parse_checklist_line, parse_inline_block,
    parse_list_items, unwrap_synthetic_paragraph_wrappers,
};
use super::util::{
    clamp_heading_level, js_trim, js_trim_end, normalize_class_name, parse_boolean_attribute,
    strip_leading_newlines, strip_trailing_newlines,
};
use crate::model::{Block, Item};

/// Append a parsed block of `block_type` (with `attrs` and raw `content_lines`) to `blocks`,
/// shaping per-type fields exactly as the reference `pushBlock` closure does.
fn push_block(blocks: &mut Vec<Block>, block_type: &str, attrs: &[Attr], content_lines: &[String]) {
    let content = js_trim(&content_lines.join("\n")).to_string();
    let hidden = parse_boolean_attribute(attr(attrs, "hidden"));
    let id = attr(attrs, "id").to_string();
    let class_name = normalize_class_name(attr(attrs, "class"));

    let mut block = Block {
        kind: block_type.to_string(),
        id,
        class_name,
        hidden,
        ..Block::default()
    };

    // Per-type field shaping: each arm pulls the fields meaningful to that block kind
    // (the rest stay at their `Block::default()` values), mirroring the reference `pushBlock`.
    match block_type {
        // Heading: clamp the level (default 1) and fall back to "Section" for empty bodies.
        "heading" => {
            let level_attr = attr(attrs, "level");
            block.level = clamp_heading_level(if level_attr.is_empty() {
                "1"
            } else {
                level_attr
            });
            block.text = if content.is_empty() {
                "Section".to_string()
            } else {
                content
            };
        }
        // Plain prose blocks: the trimmed content is the whole payload.
        "paragraph" | "quote" => {
            block.text = content;
        }
        // Code: `lang` wins over the `language` alias; body keeps interior whitespace
        // (only trailing newlines stripped) so indentation survives the round-trip.
        "code" => {
            let lang = attr(attrs, "lang");
            block.language = if lang.is_empty() {
                attr(attrs, "language").to_string()
            } else {
                lang.to_string()
            };
            block.run = parse_boolean_attribute(attr(attrs, "run"));
            block.deps = js_trim(attr(attrs, "deps")).to_string();
            block.timeout = attr(attrs, "timeout").trim().parse().unwrap_or(0);
            block.format = js_trim(attr(attrs, "format")).to_lowercase();
            block.text = strip_trailing_newlines(&content_lines.join("\n")).to_string();
        }
        // Output: the captured result of running the `code` block named by `for`. The body
        // is verbatim process output, so only trailing newlines are stripped.
        "output" => {
            block.for_block = js_trim(attr(attrs, "for")).to_string();
            block.status = js_trim(attr(attrs, "status")).to_string();
            block.exit = attr(attrs, "exit").trim().parse().unwrap_or(0);
            block.hash = js_trim(attr(attrs, "hash")).to_string();
            block.format = js_trim(attr(attrs, "format")).to_lowercase();
            block.text = strip_trailing_newlines(&content_lines.join("\n")).to_string();
        }
        // Lists: collapse the `list` alias to `bulleted-list`, then build nested items by
        // indentation from the flat parse.
        "bulleted-list" | "list" | "numbered-list" => {
            let normalized_type = if block_type == "list" {
                "bulleted-list"
            } else {
                block_type
            };
            block.kind = normalized_type.to_string();
            let flat = parse_list_items(content_lines);
            block.items = build_nested_list_structure(&flat);
        }
        // Image: `src` is an attribute; the body is the alt text.
        "image" => {
            block.src = js_trim(attr(attrs, "src")).to_string();
            block.alt = content;
        }
        // Checklist: each non-empty line becomes an item; `[x]`/`[ ]` prefixes set `checked`,
        // bare lines default to unchecked. Empty items are dropped.
        "checklist" => {
            block.items = content_lines
                .iter()
                .filter_map(|line| {
                    let trimmed = js_trim(line);
                    if let Some((checked, text)) = parse_checklist_line(trimmed) {
                        Some(Item {
                            checked,
                            text,
                            ..Item::default()
                        })
                    } else if !trimmed.is_empty() {
                        Some(Item {
                            checked: false,
                            text: trimmed.to_string(),
                            ..Item::default()
                        })
                    } else {
                        None
                    }
                })
                .filter(|item| !item.text.is_empty())
                .collect();
        }
        // Rule: a horizontal divider carries no fields.
        "rule" => {}
        // Style: inline CSS body, trailing-trimmed but otherwise verbatim.
        "style" => {
            block.text = js_trim_end(&content_lines.join("\n")).to_string();
        }
        // Stylesheet: resolve the link from `href`, then `src`, then the body, in that order.
        "stylesheet" => {
            let href = attr(attrs, "href");
            let href = if !href.is_empty() {
                href.to_string()
            } else {
                let src = attr(attrs, "src");
                if !src.is_empty() {
                    src.to_string()
                } else {
                    js_trim(&content_lines.join("\n")).to_string()
                }
            };
            block.href = js_trim(&href).to_string();
            block.media = js_trim(attr(attrs, "media")).to_string();
        }
        // Script: capture type/src/module attributes and the trailing-trimmed body.
        "script" => {
            block.script_type = js_trim(attr(attrs, "type")).to_string();
            block.src = js_trim(attr(attrs, "src")).to_string();
            block.module = parse_boolean_attribute(attr(attrs, "module"));
            block.text = js_trim_end(&content_lines.join("\n")).to_string();
        }
        // Unknown kinds (svg/html/graph/mermaid/…) keep their raw body verbatim.
        _ => {
            block.text = js_trim_end(&content_lines.join("\n")).to_string();
        }
    }

    blocks.push(block);
}

/// Parse a DOCSRC body (block syntax) into raw, un-normalized blocks. Port of
/// `parseDocsrcBlocks`.
pub(super) fn parse_docsrc_blocks(body: &str) -> Vec<Block> {
    let unwrapped = unwrap_synthetic_paragraph_wrappers(body);
    let stripped = strip_leading_newlines(&unwrapped);
    let lines: Vec<String> = stripped.split('\n').map(str::to_string).collect();
    let mut blocks: Vec<Block> = Vec::new();

    let mut cursor = 0;
    while cursor < lines.len() {
        let raw_line = lines[cursor].clone();
        let line = js_trim(&raw_line);

        if line.is_empty() {
            cursor += 1;
            continue;
        }

        // Inline single-line block recovery.
        if let Some((block_type, inner)) = parse_inline_block(line) {
            if block_type != "end" {
                let (attrs, remainder) = parse_leading_attributes(inner);
                let content: Vec<String> = if remainder.is_empty() {
                    Vec::new()
                } else {
                    vec![remainder]
                };
                push_block(&mut blocks, &block_type, &attrs, &content);
            }
            cursor += 1;
            continue;
        }

        // Full-line block header.
        let header = parse_block_header(line);
        let (block_type, _) = match header {
            Some(parsed) => parsed,
            None => {
                cursor += 1;
                continue;
            }
        };
        let block_type = block_type.to_lowercase();
        if block_type == "end" {
            cursor += 1;
            continue;
        }

        // Re-split the opening line from the raw form to capture leading attributes plus a
        // same-line remainder, matching `rawLine.replace(/^::[a-z-]+/i, '')`.
        let opening_remainder = strip_block_type_prefix(&raw_line);
        let (attrs, remainder) = parse_leading_attributes(opening_remainder);
        let mut content_lines: Vec<String> = Vec::new();
        if !remainder.is_empty() {
            content_lines.push(remainder);
        }

        cursor += 1;
        let mut found_end = false;

        while cursor < lines.len() {
            let body_line = &lines[cursor];
            let body_trimmed = js_trim(body_line);
            let end_index = body_line.find("::end");

            if body_trimmed == "::end" {
                found_end = true;
                break;
            }

            if let Some(idx) = end_index {
                let before_end = js_trim_end(&body_line[..idx]);
                if !before_end.is_empty() {
                    content_lines.push(before_end.to_string());
                }
                found_end = true;
                break;
            }

            content_lines.push(body_line.clone());
            cursor += 1;
        }

        push_block(&mut blocks, &block_type, &attrs, &content_lines);

        if cursor < lines.len() && found_end {
            cursor += 1;
        }
    }

    blocks
}

/// Remove a leading `::type` prefix (case-insensitive ASCII letters and `-`) from a raw
/// opening line, matching `rawLine.replace(/^::[a-z-]+/i, '')`.
fn strip_block_type_prefix(raw_line: &str) -> &str {
    let after = match raw_line.strip_prefix("::") {
        Some(rest) => rest,
        None => return raw_line,
    };
    let type_end = after
        .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
        .unwrap_or(after.len());
    if type_end == 0 {
        return raw_line;
    }
    &after[type_end..]
}
