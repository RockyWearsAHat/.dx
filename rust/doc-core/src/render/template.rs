//! `{{placeholder}}` substitution driven by JSON `::script` blocks.
//!
//! A document can declare data once and refer to it from prose:
//!
//! ```text
//! ::script id=vars type=application/json
//! {"phase":"authoring","target":"clean examples"}
//! ::end
//!
//! ::paragraph id=intro
//! Current phase: {{phase}}.
//! ::end
//! ```
//!
//! Only top-level scalar members participate — strings, numbers, and booleans. Nested
//! objects and arrays are skipped rather than stringified, because a rendered `{"a":1}`
//! in the middle of a sentence is never what the author meant. An unknown placeholder
//! renders as the empty string, matching the reference renderer.

use crate::model::{Block, Document};

/// Ordered `(key, value)` pairs harvested from a document's JSON script blocks.
pub type Values = Vec<(String, String)>;

/// Collect placeholder values from every JSON `::script` block in `document`, in order.
///
/// A script block participates when its `type` is absent or `application/json`. Later
/// blocks override earlier ones for the same key.
#[must_use]
pub fn collect(document: &Document) -> Values {
    let mut values: Values = Vec::new();
    for block in &document.blocks {
        if block.kind != "script" || !is_json_script(block) {
            continue;
        }
        for (key, value) in scan_flat_object(&block.text) {
            match values.iter_mut().find(|(name, _)| *name == key) {
                Some(entry) => entry.1 = value,
                None => values.push((key, value)),
            }
        }
    }
    values
}

/// Replace every `{{ key }}` in `text` with its value, or the empty string when unknown.
#[must_use]
pub fn interpolate(text: &str, values: &[(String, String)]) -> String {
    if !text.contains("{{") {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            break;
        };
        let key = after[..close].trim();
        if !is_placeholder_key(key) {
            out.push_str(&rest[..open + 2]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..open]);
        if let Some((_, value)) = values.iter().find(|(name, _)| name == key) {
            out.push_str(value);
        }
        rest = &after[close + 2..];
    }

    out.push_str(rest);
    out
}

/// Whether a script block supplies template values (no type, or `application/json`).
fn is_json_script(block: &Block) -> bool {
    let script_type = block.script_type.trim().to_ascii_lowercase();
    script_type.is_empty() || script_type == "application/json"
}

/// Whether `key` is a well-formed placeholder name: `[A-Za-z0-9_.-]+`.
fn is_placeholder_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Extract top-level scalar members from a JSON object literal.
///
/// This is a deliberately small scanner rather than a JSON parser: it walks the text once,
/// reads `"key": value` pairs at nesting depth 1, and skips any value that opens a nested
/// object or array. Malformed input yields whatever prefix parsed cleanly.
fn scan_flat_object(text: &str) -> Values {
    let bytes = text.as_bytes();
    let mut values = Values::new();
    let mut index = skip_whitespace(bytes, 0);

    if index >= bytes.len() || bytes[index] != b'{' {
        return values;
    }
    index += 1;

    loop {
        index = skip_whitespace(bytes, index);
        if index >= bytes.len() || bytes[index] == b'}' {
            return values;
        }
        if bytes[index] == b',' {
            index += 1;
            continue;
        }
        let Some((key, next)) = read_string(text, index) else {
            return values;
        };
        index = skip_whitespace(bytes, next);
        if index >= bytes.len() || bytes[index] != b':' {
            return values;
        }
        index = skip_whitespace(bytes, index + 1);

        match read_scalar(text, index) {
            Some((value, next)) => {
                values.push((key, value));
                index = next;
            }
            None => index = skip_composite(bytes, index),
        }
    }
}

/// Read a JSON string starting at the opening quote; returns the unescaped value.
fn read_string(text: &str, index: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if index >= bytes.len() || bytes[index] != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut cursor = index + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => return Some((out, cursor + 1)),
            b'\\' if cursor + 1 < bytes.len() => {
                out.push(unescape(bytes[cursor + 1]));
                cursor += 2;
            }
            _ => {
                let len = text[cursor..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&text[cursor..cursor + len]);
                cursor += len;
            }
        }
    }
    None
}

/// Translate a JSON escape character to the byte it stands for.
fn unescape(escape: u8) -> char {
    match escape {
        b'n' => '\n',
        b't' => '\t',
        b'r' => '\r',
        other => other as char,
    }
}

/// Read a scalar value (string, number, `true`, `false`, `null`) starting at `index`.
/// Returns `None` when the value opens a nested object or array.
fn read_scalar(text: &str, index: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if index >= bytes.len() {
        return None;
    }
    if bytes[index] == b'"' {
        return read_string(text, index);
    }
    if bytes[index] == b'{' || bytes[index] == b'[' {
        return None;
    }
    let mut cursor = index;
    while cursor < bytes.len() && !matches!(bytes[cursor], b',' | b'}' | b']') {
        cursor += 1;
    }
    let raw = text[index..cursor].trim();
    if raw.is_empty() || raw == "null" {
        return Some((String::new(), cursor));
    }
    Some((raw.to_string(), cursor))
}

/// Skip a balanced nested object or array beginning at `index`.
fn skip_composite(bytes: &[u8], index: usize) -> usize {
    let mut depth = 0usize;
    let mut cursor = index;
    let mut in_string = false;

    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' if in_string => cursor += 1,
            b'"' => in_string = !in_string,
            b'{' | b'[' if !in_string => depth += 1,
            b'}' | b']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return cursor + 1;
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    cursor
}

/// Advance past ASCII whitespace starting at `index`.
fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse;

    fn values_of(source: &str) -> Values {
        collect(&parse(source))
    }

    #[test]
    fn collects_scalars_and_skips_composites() {
        let values = values_of(
            "::script id=v type=application/json\n{\"a\":\"one\",\"b\":2,\"c\":true,\"d\":{\"x\":1},\"e\":[1,2],\"f\":\"last\"}\n::end\n",
        );
        assert_eq!(
            values,
            vec![
                ("a".into(), "one".into()),
                ("b".into(), "2".into()),
                ("c".into(), "true".into()),
                ("f".into(), "last".into()),
            ]
        );
    }

    #[test]
    fn substitutes_known_keys_and_blanks_unknown_ones() {
        let values = vec![("phase".to_string(), "authoring".to_string())];
        assert_eq!(
            interpolate("Phase: {{ phase }} / {{missing}}.", &values),
            "Phase: authoring / ."
        );
    }

    #[test]
    fn leaves_non_placeholder_braces_intact() {
        assert_eq!(interpolate("{{ not a key }}", &[]), "{{ not a key }}");
        assert_eq!(interpolate("fn() {{ body }", &[]), "fn() {{ body }");
    }

    #[test]
    fn ignores_scripts_that_are_not_json() {
        let values = values_of("::script id=v type=text/javascript\n{\"a\":\"one\"}\n::end\n");
        assert!(values.is_empty());
    }

    #[test]
    fn later_blocks_override_earlier_keys() {
        let values = values_of(
            "::script id=a type=application/json\n{\"k\":\"first\"}\n::end\n\n::script id=b type=application/json\n{\"k\":\"second\"}\n::end\n",
        );
        assert_eq!(values, vec![("k".to_string(), "second".to_string())]);
    }
}
