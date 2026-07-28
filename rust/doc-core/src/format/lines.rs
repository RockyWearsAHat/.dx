//! Line-level matchers for the DOCSRC block scanner.
//!
//! These ports of the reference's per-line regexes recognize block headers, inline
//! `::type … ::end` blocks, synthetic-wrapper triples, list markers, and checklist items.
//! They are pure functions over a single line (or the whole body, for the wrapper unwrap)
//! and carry no block-assembly state — that lives in [`super::parse`].

use super::util::{js_trim, strip_leading_inline_ws};
use crate::model::Item;

/// Unwrap the broken `::paragraph id=paragraph-N` / line / `::end` triples that an earlier
/// renderer could persist, replacing each with just its wrapped line. Port of
/// `unwrapSyntheticParagraphWrappers`.
pub(super) fn unwrap_synthetic_paragraph_wrappers(source: &str) -> String {
    let normalized = source.replace("\r\n", "\n");
    let input: Vec<&str> = normalized.split('\n').collect();
    let mut output: Vec<String> = Vec::with_capacity(input.len());

    let mut i = 0;
    while i < input.len() {
        let line = input[i];
        let trimmed = js_trim(line);
        if is_synthetic_paragraph_open(trimmed) && i + 2 < input.len() {
            let wrapped = input[i + 1];
            let close = js_trim(input[i + 2]);
            if close == "::end" {
                output.push(wrapped.to_string());
                i += 3;
                continue;
            }
        }
        output.push(line.to_string());
        i += 1;
    }

    output.join("\n")
}

/// Match `::paragraph id=paragraph-<digits>` (case-insensitive, optional trailing spaces).
/// Port of `/^::paragraph\s+id=paragraph-\d+\s*$/i`.
fn is_synthetic_paragraph_open(trimmed: &str) -> bool {
    let lower = trimmed.to_lowercase();
    let rest = match lower.strip_prefix("::paragraph") {
        Some(rest) => rest,
        None => return false,
    };
    let rest = rest.trim_start_matches([' ', '\t']);
    if rest.len() == lower.len() - "::paragraph".len() {
        // No whitespace separated `::paragraph` from the rest: `\s+` failed.
        return false;
    }
    let rest = match rest.strip_prefix("id=paragraph-") {
        Some(rest) => rest,
        None => return false,
    };
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits_end == 0 {
        return false;
    }
    rest[digits_end..].chars().all(|c| c == ' ' || c == '\t')
}

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

/// Build the nested list structure from flat `(text, indent)` items, faithfully reproducing
/// the reference `buildNestedListStructure` — including its object-aliasing quirk where
/// `nested` children attach to discarded stack copies, so the returned items never actually
/// carry nesting. The result is the sequence of items at which the working stack emptied.
pub(super) fn build_nested_list_structure(flat: &[(String, usize)]) -> Vec<Item> {
    let mut result: Vec<Item> = Vec::new();
    // The stack tracks only indents; the reference pushes copies, so child attachment is
    // never observable in `result`. We model exactly that observable behavior.
    let mut stack_indents: Vec<usize> = Vec::new();

    for (text, indent) in flat {
        while let Some(&top) = stack_indents.last() {
            if top < *indent {
                break;
            }
            stack_indents.pop();
        }

        if stack_indents.is_empty() {
            result.push(Item {
                text: text.clone(),
                ..Item::default()
            });
        }

        stack_indents.push(*indent);
    }

    result
}

/// Split a list body's lines into `(text, indent)` pairs, recognizing `-`/`*` and `N.`
/// markers, with an indentation-preserving fallback for unmarked lines. Port of the
/// `itemsWithIndent` mapping inside `parseDocsrcBlocks`.
pub(super) fn parse_list_items(lines: &[String]) -> Vec<(String, usize)> {
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
    if let Some(rest) = body.strip_prefix(['-', '*']) {
        if let Some(after_ws) = strip_leading_inline_ws(rest) {
            return Some((indent, after_ws));
        }
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

/// Match a checklist line `[x]`/`[ ]` + text (after trimming). Returns `(checked, text)`.
/// Port of `/^\s*\[(x| )\]\s*(.*)$/i` applied to the trimmed line.
pub(super) fn parse_checklist_line(trimmed: &str) -> Option<(bool, String)> {
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
