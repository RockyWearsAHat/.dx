//! HTML escaping and markup sanitizing shared by every renderer surface.
//!
//! Two different jobs live here and must not be confused:
//!
//! - [`escape_html`] turns arbitrary text into inert HTML text. Use it for anything the
//!   author wrote as *prose* — headings, paragraphs, list items, code bodies.
//! - [`sanitize_markup`] keeps author-supplied *markup* (`::html`, `::svg`) working while
//!   removing the parts that could execute: `<script>` elements, `on*` event handlers, and
//!   `javascript:` URLs. It is a deny-list over a small, well-understood surface, applied
//!   to content that is already local to the reader's own machine.

/// Escape the five HTML-significant characters so `value` renders as literal text.
#[must_use]
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a value for use inside a `<style>` element, where only `</style` can break out.
#[must_use]
pub fn escape_style(value: &str) -> String {
    replace_case_insensitive(value, "</style", "<\\/style")
}

/// Strip executable constructs from author-supplied markup, leaving presentation intact.
///
/// Removes `<script>…</script>` elements, `on*=` event-handler attributes, and
/// `javascript:` URLs in `href`/`src`/`xlink:href`. Everything else — tables, spans,
/// SVG shapes, inline `style` attributes — passes through unchanged.
#[must_use]
pub fn sanitize_markup(value: &str) -> String {
    let without_scripts = strip_elements(value, "script");
    let without_handlers = strip_event_handlers(&without_scripts);
    strip_javascript_urls(&without_handlers)
}

/// Extract the first `<svg>…</svg>` element from `value`, or the empty string when the
/// text contains no SVG root. Used so an `::svg` block cannot smuggle in sibling markup.
#[must_use]
pub fn extract_svg(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let start = match lower.find("<svg") {
        Some(index) => index,
        None => return String::new(),
    };
    match lower[start..].find("</svg>") {
        Some(offset) => value[start..start + offset + "</svg>".len()].to_string(),
        None => String::new(),
    }
}

/// Remove every `<tag …>…</tag>` element (and any unterminated trailing open tag).
fn strip_elements(value: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find(&open) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        match lower[start..].find(&close) {
            Some(offset) => rest = &rest[start + offset + close.len()..],
            // Unterminated open tag: drop the remainder rather than leaving it live.
            None => return out,
        }
    }
}

/// Remove ` on<name>="…"` / `='…'` / `=bare` event-handler attributes.
fn strip_event_handlers(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            if let Some(end) = match_event_handler(value, index) {
                index = end;
                continue;
            }
        }
        let ch_len = char_len_at(value, index);
        out.push_str(&value[index..index + ch_len]);
        index += ch_len;
    }

    out
}

/// If an event-handler attribute starts at whitespace position `start`, return the byte
/// index just past it.
fn match_event_handler(value: &str, start: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut index = start;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if !value[index..].to_ascii_lowercase().starts_with("on") {
        return None;
    }
    let name_start = index;
    index += 2;
    while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
        index += 1;
    }
    if index == name_start + 2 {
        return None;
    }
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() || bytes[index] != b'=' {
        return None;
    }
    index += 1;
    Some(skip_attribute_value(bytes, index))
}

/// Advance past an attribute value starting at `index`: quoted run or bare token.
fn skip_attribute_value(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() {
        return index;
    }
    let quote = bytes[index];
    if quote == b'"' || quote == b'\'' {
        index += 1;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        return (index + 1).min(bytes.len());
    }
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' {
        index += 1;
    }
    index
}

/// Neutralize `javascript:` URL schemes wherever they appear in markup.
fn strip_javascript_urls(value: &str) -> String {
    replace_case_insensitive(value, "javascript:", "about:blank#blocked-")
}

/// Case-insensitive literal replacement, preserving all other bytes.
fn replace_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    loop {
        let lower = rest.to_ascii_lowercase();
        match lower.find(&lower_needle) {
            Some(index) => {
                out.push_str(&rest[..index]);
                out.push_str(replacement);
                rest = &rest[index + needle.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Byte length of the UTF-8 character starting at `index`.
fn char_len_at(value: &str, index: usize) -> usize {
    value[index..].chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_significant_character() {
        assert_eq!(
            escape_html("<a href=\"x\">&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn removes_script_elements_and_handlers() {
        let dirty = "<p onclick=\"steal()\">hi</p><script>bad()</script><b>ok</b>";
        assert_eq!(sanitize_markup(dirty), "<p>hi</p><b>ok</b>");
    }

    #[test]
    fn removes_unterminated_script_tail() {
        assert_eq!(sanitize_markup("<b>ok</b><script>bad()"), "<b>ok</b>");
    }

    #[test]
    fn neutralizes_javascript_urls() {
        assert_eq!(
            sanitize_markup("<a href=\"JavaScript:evil()\">x</a>"),
            "<a href=\"about:blank#blocked-evil()\">x</a>"
        );
    }

    #[test]
    fn drops_every_on_prefixed_attribute() {
        // HTML reserves the whole `on*` attribute namespace for event handlers, so the
        // deny-list covers all of it rather than tracking a list of event names.
        assert_eq!(sanitize_markup("<p onerror='x' ondrop=y>t</p>"), "<p>t</p>");
        assert_eq!(sanitize_markup("<p data-on=1>t</p>"), "<p data-on=1>t</p>");
    }

    #[test]
    fn extracts_only_the_svg_root() {
        assert_eq!(
            extract_svg("junk <svg><rect/></svg> more"),
            "<svg><rect/></svg>"
        );
        assert_eq!(extract_svg("no svg here"), "");
    }

    #[test]
    fn escapes_style_terminator() {
        assert_eq!(escape_style("a{} </STYLE>"), "a{} <\\/style>");
    }
}
