//! Attribute-string parsing for the DOCSRC reader.
//!
//! Block headers carry an attribute remainder (`id=…`, `class=…`, bare
//! `hidden`/`module`/`run`/`open`,
//! and per-type keys). This module ports the reference's left-to-right attribute scanner
//! and the small `(key, value)` lookup helpers used by the parser.

use super::util::js_trim;

/// A `(key, value)` attribute pair extracted from a block header.
pub(super) type Attr = (String, String);

/// Parse the leading attributes of a `kind` block's header remainder and return the
/// unconsumed remainder.
///
/// Consumes, left to right, either a bare boolean the kind may carry (see
/// [`bare_booleans`]) or a `key=value` (double-quoted, single-quoted, or bare) pair,
/// stopping at the first non-attribute token. Keys are lowercased. Port of
/// `parseLeadingAttributesAndRemainder`, with the bare booleans scoped per kind.
pub(super) fn parse_leading_attributes(text: &str, kind: &str) -> (Vec<Attr>, String) {
    let allowed = bare_booleans(kind);
    let mut attrs: Vec<Attr> = Vec::new();
    let mut rest = text;

    loop {
        if let Some(consumed) = match_bare_boolean(rest, allowed) {
            let key = js_trim(&rest[..consumed]).to_lowercase();
            set_attr(&mut attrs, &key, "true");
            rest = &rest[consumed..];
            continue;
        }

        match match_key_value(rest) {
            Some((consumed, key, value)) => {
                let key = js_trim(&key).to_lowercase();
                if !key.is_empty() {
                    set_attr(&mut attrs, &key, &value);
                }
                rest = &rest[consumed..];
            }
            None => break,
        }
    }

    (attrs, js_trim(rest).to_string())
}

/// Insert or overwrite an attribute, preserving first-seen order (JS object assignment).
pub(super) fn set_attr(attrs: &mut Vec<Attr>, key: &str, value: &str) {
    if let Some(entry) = attrs.iter_mut().find(|(name, _)| name == key) {
        entry.1 = value.to_string();
    } else {
        attrs.push((key.to_string(), value.to_string()));
    }
}

/// Look up an attribute value, returning empty when absent.
pub(super) fn attr<'a>(attrs: &'a [Attr], key: &str) -> &'a str {
    attrs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}

/// The bare boolean attributes a block of `kind` may carry. Scoped per kind so a bare
/// word opening an inline body — `::heading id=h Open questions ::end` — stays prose
/// instead of being swallowed as an attribute: `run` and `open` belong to code,
/// `module` to script, and `hidden` to every block.
fn bare_booleans(kind: &str) -> &'static [&'static str] {
    match kind.to_ascii_lowercase().as_str() {
        "code" => &["hidden", "run", "open"],
        "script" => &["hidden", "module"],
        _ => &["hidden"],
    }
}

/// Match a leading bare boolean attribute from `allowed` followed by whitespace or
/// end-of-string, after optional leading whitespace. Returns the consumed byte length.
fn match_bare_boolean(text: &str, allowed: &[&str]) -> Option<usize> {
    let leading_ws = text.len() - text.trim_start().len();
    let body = &text[leading_ws..];
    for keyword in allowed {
        if body.len() >= keyword.len() && body[..keyword.len()].eq_ignore_ascii_case(keyword) {
            let after = &body[keyword.len()..];
            if after.is_empty() || after.starts_with(|c: char| c.is_ascii_whitespace()) {
                return Some(leading_ws + keyword.len());
            }
        }
    }
    None
}

/// Match a leading `key=value` attribute (after optional whitespace), where the value is
/// `"…"`, `'…'`, or a run of non-whitespace bytes. Returns `(consumed_len, key, value)`.
/// Port of the `/^\s*([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/` branch.
fn match_key_value(text: &str) -> Option<(usize, String, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    let key_start = i;
    while i < bytes.len() && is_attr_key_byte(bytes[i]) {
        i += 1;
    }
    if i == key_start || i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    let key = text[key_start..i].to_string();
    i += 1; // consume '='

    if i >= bytes.len() {
        return None;
    }

    let quote = bytes[i];
    if quote == b'"' || quote == b'\'' {
        let value_start = i + 1;
        let mut j = value_start;
        while j < bytes.len() && bytes[j] != quote {
            j += 1;
        }
        if j >= bytes.len() {
            // Unterminated quote: the regex's quoted alternatives cannot match, so the
            // engine falls back to the bare-value alternative `[^\s]+` from `i`.
            return match_bare_value(text, key, i);
        }
        let value = text[value_start..j].to_string();
        Some((j + 1, key, value))
    } else {
        match_bare_value(text, key, i)
    }
}

/// Match a bare (`[^\s]+`) attribute value starting at byte `start`.
fn match_bare_value(text: &str, key: String, start: usize) -> Option<(usize, String, String)> {
    let bytes = text.as_bytes();
    let mut j = start;
    while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    if j == start {
        return None;
    }
    Some((j, key, text[start..j].to_string()))
}

/// Whether `byte` is allowed in an attribute key: `[a-zA-Z0-9._-]`.
fn is_attr_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}
