//! The DOCSRC block scanner: turn a `.dx` block body into raw, un-normalized blocks.
//!
//! This module owns the block-assembly loop (`parseDocsrcBlocks`/`pushBlock`): it walks the
//! body line by line, opens and closes `::type … ::end` blocks, and shapes per-type fields.
//! The per-line matchers it drives live in [`super::lines`]; the top-level strategy
//! selection and public `parse` entry point live in [`super::source`]; attribute scanning in
//! [`super::attrs`].

use super::attrs::{attr, parse_leading_attributes, Attr};
use super::legacy::parse_legacy_blocks;
use super::lines::{
    build_nested_list_structure, parse_block_header, parse_checklist_line, parse_inline_block,
    parse_list_items,
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
        // `src` names a sibling file whose current text is the listing — the stored body
        // stays as written, and `resolve::hydrate` fills it in at view time.
        "code" => {
            let lang = attr(attrs, "lang");
            block.language = if lang.is_empty() {
                attr(attrs, "language").to_string()
            } else {
                lang.to_string()
            };
            block.src = js_trim(attr(attrs, "src")).to_string();
            block.run = parse_boolean_attribute(attr(attrs, "run"));
            block.open = parse_boolean_attribute(attr(attrs, "open"));
            block.actions = parse_boolean_attribute(attr(attrs, "actions"));
            block.deps = js_trim(attr(attrs, "deps")).to_string();
            block.reads = js_trim(attr(attrs, "reads")).to_string();
            block.writes = js_trim(attr(attrs, "writes")).to_string();
            block.target = js_trim(attr(attrs, "target")).to_string();
            block.setup = js_trim(attr(attrs, "setup")).to_string();
            block.width = attr(attrs, "width").trim().parse().unwrap_or(0);
            block.height = attr(attrs, "height").trim().parse().unwrap_or(0);
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
        // Image: `src` is an attribute; the body is the alt text. `for` names the
        // runnable block whose run produces the pictured file, tying the picture's
        // freshness to that block's recorded output.
        "image" => {
            block.src = js_trim(attr(attrs, "src")).to_string();
            block.for_block = js_trim(attr(attrs, "for")).to_string();
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
        // Nav: one navigation target per line, parsed exactly like list items so an
        // indented entry nests. An **empty body is meaningful** — it asks for this
        // document's own contents — so no default entry is invented here or in
        // normalization.
        "nav" => {
            block.label = js_trim(attr(attrs, "label")).to_string();
            let flat = parse_list_items(content_lines);
            block.items = build_nested_list_structure(&flat);
        }
        // Rule: a horizontal divider carries no fields.
        "rule" => {}
        // Style: inline CSS body, trailing-trimmed but otherwise verbatim. `media` scopes it
        // the same way it scopes a `::stylesheet`, so "this dress is for print" is one word.
        // `src` names a sibling stylesheet whose current text is the block's CSS — the
        // stored body stays as written, and `resolve::hydrate` fills it in at view time.
        "style" => {
            block.text = js_trim_end(&content_lines.join("\n")).to_string();
            block.src = js_trim(attr(attrs, "src")).to_string();
            block.media = js_trim(attr(attrs, "media")).to_string();
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
        // View: a sibling coded page (`src=`), framed and shown as the page it renders to —
        // the stored body stays as written (usually empty) and `resolve::hydrate` fills it
        // with the page's current markup at view time. `width`/`height` are the framed
        // viewport in its own CSS pixels; 0 means the renderer's default.
        "view" => {
            block.src = js_trim(attr(attrs, "src")).to_string();
            block.width = attr(attrs, "width").trim().parse().unwrap_or(0);
            block.height = attr(attrs, "height").trim().parse().unwrap_or(0);
            block.text = js_trim_end(&content_lines.join("\n")).to_string();
        }
        // Board: a canvas that arranges other blocks of this document as nodes. The body is
        // one reference line per node (`- id x=.. y=.. w=.. to=..`) and is kept verbatim —
        // the line grammar belongs to `render::board`, not the scanner. `height` is the
        // viewport's height in CSS pixels; 0 means the renderer's default.
        "board" => {
            block.height = attr(attrs, "height").trim().parse().unwrap_or(0);
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
///
/// Returns an error if a block opener (e.g. `::heading`, `::paragraph`) is found but has no
/// matching `::end` marker before the document ends.
pub(super) fn parse_docsrc_blocks(body: &str) -> Result<Vec<Block>, String> {
    let stripped = strip_leading_newlines(body);
    let lines: Vec<String> = stripped.split('\n').map(str::to_string).collect();
    let mut blocks: Vec<Block> = Vec::new();

    // Prose written between `::` blocks — a Markdown heading above a listing, a paragraph
    // under it. Buffered here and adopted through the Markdown parser when the next block
    // header (or the end of input) arrives. A mixed document is something people actually
    // write, and a parser that skipped these lines destroyed them on the next save.
    let mut loose: Vec<String> = Vec::new();

    let mut cursor = 0;
    while cursor < lines.len() {
        let raw_line = lines[cursor].clone();
        let line = js_trim(&raw_line);

        if line.is_empty() {
            // A blank inside a loose run separates its paragraphs; outside one it is
            // nothing at all.
            if !loose.is_empty() {
                loose.push(String::new());
            }
            cursor += 1;
            continue;
        }

        // Inline single-line block recovery.
        if let Some((block_type, inner)) = parse_inline_block(line) {
            adopt_loose(&mut loose, &mut blocks);
            if block_type != "end" {
                let (attrs, remainder) = parse_leading_attributes(inner, &block_type);
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
                loose.push(raw_line);
                cursor += 1;
                continue;
            }
        };
        adopt_loose(&mut loose, &mut blocks);
        let block_type = block_type.to_lowercase();
        if block_type == "end" {
            cursor += 1;
            continue;
        }

        // Re-split the opening line from the raw form to capture leading attributes plus a
        // same-line remainder, matching `rawLine.replace(/^::[a-z-]+/i, '')`.
        let opening_remainder = strip_block_type_prefix(&raw_line);
        let (attrs, remainder) = parse_leading_attributes(opening_remainder, &block_type);
        let mut content_lines: Vec<String> = Vec::new();
        if !remainder.is_empty() {
            content_lines.push(remainder);
        }

        cursor += 1;
        let mut found_end = false;
        let block_start_line = cursor; // Track line number for error reporting

        while cursor < lines.len() {
            let body_line = &lines[cursor];

            match close_token(body_line) {
                // An escaped close token is content: the writer put that backslash there so a
                // document could show dx syntax, and reading takes exactly one back off.
                Some(token) if token.escaped() => {
                    content_lines.push(token.unescape(body_line));
                }
                Some(token) => {
                    let before_end = js_trim_end(&body_line[..token.start]);
                    if !before_end.is_empty() {
                        content_lines.push(before_end.to_string());
                    }
                    found_end = true;
                    break;
                }
                None => content_lines.push(body_line.clone()),
            }
            cursor += 1;
        }

        // Check for missing ::end marker
        if !found_end {
            return Err(format!(
                "block opener '::{block_type}' at line {} has no matching '::end'",
                block_start_line
            ));
        }

        push_block(&mut blocks, &block_type, &attrs, &content_lines);

        if cursor < lines.len() && found_end {
            cursor += 1;
        }
    }

    adopt_loose(&mut loose, &mut blocks);
    Ok(blocks)
}

/// Adopt a buffered run of loose prose lines as blocks, through the Markdown parser —
/// so `# heading`, list items, and paragraphs between `::` blocks arrive as what they
/// say instead of being discarded. Clears the buffer; a blank or empty run adopts nothing.
fn adopt_loose(loose: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if loose.is_empty() {
        return;
    }
    let segment = loose.join("\n");
    loose.clear();
    if js_trim(&segment).is_empty() {
        return;
    }
    blocks.extend(parse_legacy_blocks(&segment));
}

/// A body line's close token: where `::end` sits, and how many backslashes escape it.
#[derive(Debug, Clone, Copy)]
pub(super) struct CloseToken {
    /// Byte index where `::end` starts.
    pub(super) start: usize,
    /// Consecutive backslashes immediately before the token — one level per escape.
    backslashes: usize,
}

impl CloseToken {
    /// Whether this token is escaped, and so is content rather than the block's end.
    fn escaped(self) -> bool {
        self.backslashes > 0
    }

    /// The line as content, with exactly one escaping backslash taken off.
    fn unescape(self, line: &str) -> String {
        let cut = self.start - 1;
        format!("{}{}", &line[..cut], &line[self.start..])
    }
}

/// Where a body line's close token starts, if the line has the shape of one at all.
///
/// The format's recovery rule is "close token alone on a line, or trailing after content" — a
/// body line like `    } ::end` closes its block. The rule is deliberately narrow: `::end`
/// counts only when whitespace (or nothing, or the escaping backslashes) precedes it and
/// nothing but whitespace follows, which is the same shape [`parse_inline_block`] matches for
/// a whole single-line block.
///
/// It has to stay narrow, because the alternative destroys prose. Matching `::end` anywhere in
/// a line meant that a document *describing* the format lost content: a paragraph reading
/// "a block ends with `::end` on its own line" was cut at the backtick, and an SVG label
/// containing `::end` truncated the drawing and everything under it. A block terminates on a
/// line of its own, or at the end of a line — never in the middle of a sentence.
///
/// # The escape, and why it exists
/// A close token directly preceded by a backslash is *content*: the line is kept, one backslash
/// lighter. Without it a document could not show dx syntax at all — the format contract, the
/// block reference, and every tutorial that prints a block would be cut off at their first
/// example — and the writer ([`super::stringify`]) puts the backslash there, so the escape is
/// something documents round-trip through rather than something an author maintains. The ladder
/// is one level per backslash, which is what keeps it reversible: `\::end` reads as `::end`,
/// `\\::end` reads as `\::end`, and a line whose backslashes do not sit against a close-shaped
/// token (`foo\::end`) is ordinary content, untouched.
pub(super) fn close_token(body_line: &str) -> Option<CloseToken> {
    let lower = body_line.to_ascii_lowercase();
    let start = lower.rfind("::end")?;
    if !body_line[start + "::end".len()..]
        .chars()
        .all(char::is_whitespace)
    {
        return None;
    }
    let before = &body_line[..start];
    let unescaped = before.trim_end_matches('\\');
    if !(unescaped.is_empty() || unescaped.ends_with(char::is_whitespace)) {
        return None;
    }
    Some(CloseToken {
        start,
        backslashes: before.len() - unescaped.len(),
    })
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

/// What a `::kind attrs` opening line says, read by the scanner's own grammar: the kind it
/// names, and whether it carries an explicit `id=` attribute.
///
/// This exists for [`crate::edit::replace_block`], which has to know both facts *before*
/// trusting the line to a full parse — an unknown kind must be refused rather than silently
/// folded to `paragraph`, and a header that names no id keeps the id the block already has.
/// Reading the line here, with [`parse_block_header`] and [`parse_leading_attributes`], is
/// what keeps editing surfaces from growing a second header grammar that could disagree
/// with this one.
///
/// Returns `None` when the line is not a block opening at all (no `::kind` shape, or the
/// close token `::end`).
pub(crate) fn header_line_facts(line: &str) -> Option<(String, bool)> {
    let trimmed = js_trim(line);
    let (block_type, _) = parse_block_header(trimmed)?;
    let block_type = block_type.to_lowercase();
    if block_type == "end" {
        return None;
    }
    let (attrs, _) = parse_leading_attributes(strip_block_type_prefix(trimmed), &block_type);
    let has_id = attrs.iter().any(|(key, _)| key == "id");
    Some((block_type, has_id))
}
