//! Line-level matchers for the DOCSRC block scanner.
//!
//! These matchers recognize block headers, inline `::type … ::end` blocks, list markers,
//! and checklist items. They are pure functions over a single line and carry no
//! block-assembly state — that lives in [`super::parse`].

use super::util::{js_trim, strip_leading_inline_ws};
use crate::model::Item;

/// Match a full-line block header `::type` with an optional attribute remainder.
/// Returns `(type, remainder)`; `None` when the trimmed line is not a header.
/// Port of `parseBlockHeader` combined with the opening-line remainder split.
pub(super) fn parse_block_header(trimmed: &str) -> Option<(String, &str)> {
    let after = trimmed.strip_prefix("::")?;
    let type_end = after
        .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
        .unwrap_or(after.len());
    if type_end == 0 {
        return None;
    }
    let block_type = &after[..type_end];
    let remainder = &after[type_end..];
    // The reference regex requires either end-of-line or whitespace before attributes.
    if !remainder.is_empty() && !remainder.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    Some((block_type.to_string(), remainder))
}

/// Match an inline single-line block `::type … ::end` (case-insensitive `::end`).
/// Returns `(type_lowercased, inner)`. Port of `/^::([a-z-]+)(.*)\s+::end\s*$/i`.
pub(super) fn parse_inline_block(trimmed: &str) -> Option<(String, &str)> {
    let after = trimmed.strip_prefix("::")?;
    let type_end = after
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_uppercase() || c == '-'))
        .unwrap_or(after.len());
    if type_end == 0 {
        return None;
    }
    let block_type = after[..type_end].to_lowercase();
    let rest = &after[type_end..];

    // Greedy `(.*)\s+::end\s*$`: find the last `::end` preceded by whitespace, with only
    // trailing whitespace after it.
    let lower_rest = rest.to_lowercase();
    let mut search_from = lower_rest.len();
    while let Some(pos) = lower_rest[..search_from].rfind("::end") {
        let after_end = &rest[pos + "::end".len()..];
        if !after_end.chars().all(|c| c.is_ascii_whitespace()) {
            search_from = pos;
            continue;
        }
        // Require at least one whitespace char immediately before `::end`.
        let before = &rest[..pos];
        if before.ends_with(|c: char| c.is_ascii_whitespace()) {
            let inner = &before[..before.trim_end().len()];
            return Some((block_type, inner));
        }
        search_from = pos;
    }
    None
}

/// The list of children at `depth` levels below `items`, following the last item at each
/// level. When a level has no item to nest under, descent stops there and the shallowest
/// available list is returned, so an over-indented first item still lands somewhere.
fn child_list_at_depth(items: &mut Vec<Item>, depth: usize) -> &mut Vec<Item> {
    let mut current = items;
    for _ in 0..depth {
        if current.is_empty() {
            break;
        }
        let last = current.len() - 1;
        current = &mut current[last].nested;
    }
    current
}

/// Build a nested list tree from flat `(text, indent)` pairs.
///
/// An item indented further than the item before it becomes that item's child; an item at
/// the same or a shallower indent closes the deeper levels and continues at its own. Every
/// input item appears exactly once in the result — nothing is dropped, whatever the
/// indentation, which is what keeps a saved list identical to the one that was written.
///
/// Complexity: `O(n · d)` for `n` items nested `d` deep (`d` is the descent per item).
pub(crate) fn build_nested_list_structure(flat: &[(String, usize)]) -> Vec<Item> {
    let mut roots: Vec<Item> = Vec::new();
    // Indents of the ancestors currently open, outermost first.
    let mut open: Vec<usize> = Vec::new();

    for (text, indent) in flat {
        while open.last().is_some_and(|&top| *indent <= top) {
            open.pop();
        }
        child_list_at_depth(&mut roots, open.len()).push(Item {
            text: text.clone(),
            ..Item::default()
        });
        open.push(*indent);
    }

    roots
}

/// Split a list body's lines into `(text, indent)` pairs, recognizing `-`/`*` and `N.`
/// markers, with an indentation-preserving fallback for unmarked lines. Port of the
/// `itemsWithIndent` mapping inside `parseDocsrcBlocks`.
///
/// Also the reading half of [`crate::edit::set_body`] for list blocks, so a surface's lines
/// are read by exactly the grammar a document's own body lines are.
pub(crate) fn parse_list_items(lines: &[String]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for line in lines {
        if let Some((indent, text)) = match_list_marker(line) {
            out.push((js_trim(text).to_string(), indent));
        } else {
            // Fallback `/^(\s*)(.+)$/`: requires at least one non-newline char after indent.
            let indent = line.len() - line.trim_start().len();
            let rest = &line[indent..];
            if !rest.is_empty() {
                out.push((js_trim(rest).to_string(), indent));
            }
        }
    }
    out
}

/// Match a leading list marker: indentation, then `-`/`*` + space, or `N.` + space.
/// Returns `(indent_byte_len, text_after_marker)`.
fn match_list_marker(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let body = &line[indent..];

    // Bulleted: `[-*]\s+`.
    if let Some(after_ws) = strip_bullet_marker(body) {
        return Some((indent, after_ws));
    }

    // Numbered: `\d+\.\s+`.
    let digits_end = body
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(body.len());
    if digits_end > 0 {
        let after_digits = &body[digits_end..];
        if let Some(rest) = after_digits.strip_prefix('.') {
            if let Some(after_ws) = strip_leading_inline_ws(rest) {
                return Some((indent, after_ws));
            }
        }
    }

    None
}

/// Strip one leading `-`/`*` bullet marker — the marker plus at least one whitespace
/// character — returning the text after it.
///
/// This is the single definition of what counts as a bullet, used by [`match_list_marker`]
/// (and through it by every reader of list lines, including [`crate::edit::set_body`]):
/// `None` for a line like `--verbose` or `*emphasis*`, which is text that merely starts
/// with a marker character, not a marked item.
pub(crate) fn strip_bullet_marker(text: &str) -> Option<&str> {
    strip_leading_inline_ws(text.strip_prefix(['-', '*'])?)
}

/// Match a checklist line `[x]`/`[ ]` + text (after trimming). Returns `(checked, text)`.
/// Port of `/^\s*\[(x| )\]\s*(.*)$/i` applied to the trimmed line.
pub(crate) fn parse_checklist_line(trimmed: &str) -> Option<(bool, String)> {
    let rest = trimmed.strip_prefix('[')?;
    let token = rest.chars().next()?;
    let checked = match token {
        'x' | 'X' => true,
        ' ' => false,
        _ => return None,
    };
    let after_token = &rest[token.len_utf8()..];
    let after_bracket = after_token.strip_prefix(']')?;
    Some((checked, js_trim(after_bracket).to_string()))
}
