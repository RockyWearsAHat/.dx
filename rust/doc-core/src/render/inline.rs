//! Inline prose formatting for headings, paragraphs, quotes, and list items.
//!
//! Block structure in `.dx` is explicit, so inline syntax is deliberately tiny — just the
//! four marks people reach for without thinking:
//!
//! | Source              | Renders as            |
//! |---------------------|-----------------------|
//! | `` `code` ``        | `<code>`              |
//! | `**bold**`          | `<strong>`            |
//! | `*italic*`          | `<em>`                |
//! | `[label](target)`   | `<a href>`            |
//!
//! Two rules keep this predictable. Text is HTML-escaped **before** any mark is
//! recognized, so the only tags in the output are the ones this module emits. And the line
//! is split into code spans and prose spans *first*, with marks applied only to prose — so
//! `` `a *b* c` `` renders literally, the way anyone writing about code expects.

use super::escape::escape_html;

/// Render one line of prose to HTML: escape it, then apply the inline marks.
#[must_use]
pub fn inline_html(text: &str) -> String {
    let escaped = escape_html(text);
    let mut out = String::with_capacity(escaped.len());

    for span in split_code_spans(&escaped) {
        match span {
            Span::Code(body) => {
                out.push_str("<code>");
                out.push_str(body);
                out.push_str("</code>");
            }
            Span::Prose(body) => out.push_str(&apply_emphasis(&apply_links(body))),
        }
    }

    out
}

/// A run of already-escaped text, tagged by whether marks apply to it.
///
/// Shared with the field renderer (`super::field`), which draws the same grammar with its
/// markers still visible — one scanner, two views, so the two cannot disagree about where a
/// code span begins.
pub(super) enum Span<'a> {
    /// Text inside backticks: rendered verbatim, never re-scanned.
    Code(&'a str),
    /// Ordinary prose: links and emphasis apply.
    Prose(&'a str),
}

/// Split escaped text into alternating prose and backtick-delimited code spans.
///
/// An unmatched backtick is not a code span: the rest of the line stays prose, so a
/// stray tick in a sentence never swallows the remainder.
pub(super) fn split_code_spans(text: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut rest = text;

    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        if open > 0 {
            spans.push(Span::Prose(&rest[..open]));
        }
        spans.push(Span::Code(&after[..close]));
        rest = &after[close + 1..];
    }

    if !rest.is_empty() {
        spans.push(Span::Prose(rest));
    }
    spans
}

/// Turn `[label](target)` into an anchor. Input and output are already escaped.
fn apply_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find('[') {
        let Some(link) = parse_link(&rest[open..]) else {
            out.push_str(&rest[..open + 1]);
            rest = &rest[open + 1..];
            continue;
        };
        out.push_str(&rest[..open]);
        out.push_str(&format!("<a href=\"{}\">{}</a>", link.target, link.label));
        rest = &rest[open + link.consumed..];
    }

    out.push_str(rest);
    out
}

/// A parsed `[label](target)` span and how many bytes of input it covered.
pub(super) struct Link<'a> {
    pub(super) label: &'a str,
    pub(super) target: &'a str,
    pub(super) consumed: usize,
}

/// Parse a link starting at the `[` in `text`, or `None` when the shape does not match.
///
/// The target must be a single non-empty token; a bracketed phrase followed by an
/// unrelated parenthetical is left as written.
pub(super) fn parse_link(text: &str) -> Option<Link<'_>> {
    let label_end = text.find(']')?;
    if !text[label_end + 1..].starts_with('(') {
        return None;
    }
    let target_start = label_end + 2;
    let target_end = target_start + text[target_start..].find(')')?;
    let target = text[target_start..target_end].trim();
    if target.is_empty() || target.contains(char::is_whitespace) {
        return None;
    }
    Some(Link {
        label: &text[1..label_end],
        target,
        consumed: target_end + 1,
    })
}

/// Apply `**bold**` first, then `*italic*` over what remains.
fn apply_emphasis(text: &str) -> String {
    let bolded = wrap_delimited(text, "**", "strong");
    wrap_delimited(&bolded, "*", "em")
}

/// Wrap runs delimited by `marker` in `<tag>`, ignoring markers inside `<…>` tags and
/// empty or whitespace-led runs (so `2 * 3 * 4` stays arithmetic).
fn wrap_delimited(text: &str, marker: &str, tag: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = find_outside_tags(rest, marker) {
        let after = &rest[open + marker.len()..];
        let Some(close) = find_outside_tags(after, marker) else {
            break;
        };
        let inner = &after[..close];
        if inner.is_empty() || inner.starts_with(char::is_whitespace) {
            out.push_str(&rest[..open + marker.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..open]);
        out.push_str(&format!("<{tag}>{inner}</{tag}>"));
        rest = &after[close + marker.len()..];
    }

    out.push_str(rest);
    out
}

/// Find `marker` in `text`, skipping any occurrence between `<` and `>`.
/// Walking by character rather than by byte is required, not stylistic: prose contains
/// em dashes, accents, and emoji, and slicing `text` at a position inside one of those
/// would panic.
fn find_outside_tags(text: &str, marker: &str) -> Option<usize> {
    let mut in_tag = false;

    for (index, character) in text.char_indices() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag && text[index..].starts_with(marker) => return Some(index),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_before_marking_up() {
        assert_eq!(inline_html("<b>raw</b>"), "&lt;b&gt;raw&lt;/b&gt;");
    }

    #[test]
    fn renders_the_four_marks() {
        assert_eq!(inline_html("`x = 1`"), "<code>x = 1</code>");
        assert_eq!(inline_html("**loud**"), "<strong>loud</strong>");
        assert_eq!(inline_html("*soft*"), "<em>soft</em>");
        assert_eq!(
            inline_html("see [docs](https://x.dev)"),
            "see <a href=\"https://x.dev\">docs</a>"
        );
    }

    #[test]
    fn code_spans_are_not_re_scanned() {
        assert_eq!(inline_html("`a *b* c`"), "<code>a *b* c</code>");
        assert_eq!(
            inline_html("use `[a](b)` here"),
            "use <code>[a](b)</code> here"
        );
    }

    #[test]
    fn an_unmatched_backtick_stays_literal() {
        assert_eq!(inline_html("a ` b *c*"), "a ` b <em>c</em>");
    }

    #[test]
    fn leaves_arithmetic_and_stray_brackets_alone() {
        assert_eq!(inline_html("2 * 3 * 4"), "2 * 3 * 4");
        assert_eq!(inline_html("array[0] then"), "array[0] then");
        assert_eq!(inline_html("[see this] (later)"), "[see this] (later)");
    }

    #[test]
    fn bold_wins_over_italic_for_double_markers() {
        assert_eq!(
            inline_html("**a** and *b*"),
            "<strong>a</strong> and <em>b</em>"
        );
    }

    #[test]
    fn link_targets_stay_escaped_exactly_once() {
        assert_eq!(inline_html("[x](a\"b)"), "<a href=\"a&quot;b\">x</a>");
    }

    #[test]
    fn prose_with_non_ascii_characters_renders_instead_of_panicking() {
        // Every one of these is multi-byte: slicing at a byte offset inside one aborts.
        assert_eq!(inline_html("a — b"), "a — b");
        assert_eq!(inline_html("naïve café — déjà vu"), "naïve café — déjà vu");
        assert_eq!(inline_html("ship it 🚀 — now"), "ship it 🚀 — now");
        assert_eq!(
            inline_html("**é** — *ü*"),
            "<strong>é</strong> — <em>ü</em>"
        );
        assert_eq!(inline_html("`—` — ok"), "<code>—</code> — ok");
        assert_eq!(inline_html("日本語のテキスト"), "日本語のテキスト");
    }

    #[test]
    fn markup_cannot_be_injected_through_a_link() {
        let out = inline_html("[l](\"><script>bad()</script>)");
        assert!(!out.contains("<script"));
        assert!(out.contains("&lt;script&gt;"));
    }
}
