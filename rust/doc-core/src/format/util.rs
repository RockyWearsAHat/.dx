//! Small leaf helpers shared across the DOCSRC parser, writer, and normalizer.
//!
//! These are ports of the JavaScript reference's string/number primitives and the
//! deterministic id registry. They carry no DOCSRC-specific policy beyond faithfully
//! reproducing JS semantics (`String.trim`, `Number`, slugification, etc.).

// ---------------------------------------------------------------------------
// Small string helpers (port of JS `String(x).trim()` / `trimEnd()` semantics)
// ---------------------------------------------------------------------------

/// Trim ASCII/Unicode whitespace from both ends, matching JS `String.prototype.trim`.
///
/// JavaScript trims a fixed set of whitespace code points; Rust's [`str::trim`] trims the
/// Unicode `White_Space` property, which agrees with JS for every character that appears in
/// DOCSRC. We use [`str::trim`] directly for that reason.
pub(super) fn js_trim(text: &str) -> &str {
    text.trim()
}

/// Trim trailing whitespace only, matching JS `String.prototype.trimEnd`.
pub(super) fn js_trim_end(text: &str) -> &str {
    text.trim_end()
}

/// Remove every leading `\n` (after CRLF normalization), matching `replace(/^\n+/, '')`.
pub(super) fn strip_leading_newlines(text: &str) -> &str {
    text.trim_start_matches('\n')
}

/// Remove every trailing `\n`, matching `replace(/\n+$/, '')`.
pub(super) fn strip_trailing_newlines(text: &str) -> &str {
    text.trim_end_matches('\n')
}

// ---------------------------------------------------------------------------
// Attribute value formatting / heading helpers (writer side)
// ---------------------------------------------------------------------------

/// Format an attribute value for the canonical writer.
///
/// Bare-token values (no whitespace, quotes, or `=`) are emitted unquoted; anything else is
/// double-quoted with embedded `"` stripped. Port of `formatAttributeValue`.
pub(super) fn format_attribute_value(value: &str) -> String {
    let text = js_trim(value);
    if !text.is_empty()
        && text
            .bytes()
            .all(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b'='))
    {
        return text.to_string();
    }
    let escaped: String = text.chars().filter(|&c| c != '"').collect();
    format!("\"{escaped}\"")
}

/// Collapse any run of ASCII whitespace into single spaces, dropping empty tokens.
/// Port of `normalizeClassName`.
pub(super) fn normalize_class_name(value: &str) -> String {
    value
        .split(|c: char| c.is_ascii_whitespace())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase, replace each run of non-`[a-z0-9]` with `-`, trim leading/trailing `-`, and
/// fall back to `block` when the result is empty. Port of `slugifyHeading`.
pub(super) fn slugify_heading(heading: &str) -> String {
    let lowered = heading.to_lowercase();
    let mut slug = String::with_capacity(lowered.len());
    let mut prev_dash = false;
    for ch in lowered.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "block".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Clamp a heading level into `1..=4`, defaulting non-numeric input to `1`.
/// Port of `clampHeadingLevel` for the integer string inputs DOCSRC carries.
pub(super) fn clamp_heading_level(value: &str) -> u8 {
    match parse_js_integer(value) {
        Some(level) => level.clamp(1, 4) as u8,
        None => 1,
    }
}

/// Clamp a numeric heading level already held as a `u8`.
pub(super) fn clamp_heading_level_u8(level: u8) -> u8 {
    level.clamp(1, 4)
}

/// Parse a JS-style finite integer prefix the way `Math.trunc(Number(value))` would for the
/// integer-or-empty strings DOCSRC uses. Returns `None` when `Number(value)` is `NaN`.
fn parse_js_integer(value: &str) -> Option<i64> {
    let text = value.trim();
    if text.is_empty() {
        // Number('') === 0 in JS.
        return Some(0);
    }
    // Number() accepts an optional sign and digits; anything else is NaN for our inputs.
    text.parse::<i64>().ok()
}

// ---------------------------------------------------------------------------
// Unique-id registry (writer + parser both assign deterministic ids)
// ---------------------------------------------------------------------------

/// Tracks how many times each slug base has been emitted, so duplicate seeds become
/// `base`, `base-2`, `base-3`, … exactly like the reference `ensureUniqueId`.
#[derive(Default)]
pub(super) struct IdRegistry {
    seen: Vec<(String, usize)>,
}

impl IdRegistry {
    /// Produce a unique id for `seed`, recording the collision count.
    pub(super) fn ensure_unique(&mut self, seed: &str) -> String {
        let base = slugify_heading(seed);
        let count = match self.seen.iter_mut().find(|(name, _)| name == &base) {
            Some(entry) => {
                entry.1 += 1;
                entry.1 - 1
            }
            None => {
                self.seen.push((base.clone(), 1));
                0
            }
        };
        if count == 0 {
            base
        } else {
            format!("{base}-{}", count + 1)
        }
    }
}

// ---------------------------------------------------------------------------
// Boolean attribute parsing
// ---------------------------------------------------------------------------

/// Recognize the truthy attribute spellings `true`, `1`, `yes`, `on` (case-insensitive).
/// Port of `parseBooleanAttribute`.
pub(super) fn parse_boolean_attribute(value: &str) -> bool {
    let normalized = js_trim(value).to_lowercase();
    matches!(normalized.as_str(), "true" | "1" | "yes" | "on")
}

/// Strip one-or-more leading whitespace characters (the regex `\s+`), returning the
/// remainder, or `None` when there is no leading whitespace.
pub(super) fn strip_leading_inline_ws(text: &str) -> Option<&str> {
    let stripped = text.trim_start();
    if stripped.len() == text.len() {
        None
    } else {
        Some(stripped)
    }
}

// ---------------------------------------------------------------------------
// Frontmatter scalar value conversion
// ---------------------------------------------------------------------------

/// Convert a frontmatter scalar string into the JSON-text representation stored in the
/// model's `meta` values: `true`/`false` booleans, integers, JSON objects/arrays kept
/// verbatim when well-formed-looking, otherwise the trimmed string. Port of `parseValue`,
/// specialized to the JSON-text the binary codec stores.
///
/// Note: object/array values are kept as their trimmed source text rather than being
/// re-serialized through a JSON parser (the crate is dependency-free); for the compact JSON
/// these documents use this is byte-identical, but pretty-printed JSON would differ.
pub(super) fn parse_value(raw: &str) -> String {
    let value = js_trim(raw);
    if value == "true" {
        return "true".to_string();
    }
    if value == "false" {
        return "false".to_string();
    }
    if is_js_integer_literal(value) {
        return value.to_string();
    }
    if (value.starts_with('{') && value.ends_with('}'))
        || (value.starts_with('[') && value.ends_with(']'))
    {
        return value.to_string();
    }
    // JSON string scalar: quote it the way `JSON.stringify(String)` would.
    format!("\"{}\"", json_escape(value))
}

/// Whether `value` matches the reference `/^-?\d+$/` integer test.
fn is_js_integer_literal(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Minimal JSON string escaping for the scalar meta values DOCSRC carries.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}
