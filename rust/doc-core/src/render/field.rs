//! The editing-field view of one block's source: every character kept, marks worn openly.
//!
//! When a reader clicks a block, the field replaces the rendered content with the block's
//! exact source — and this module is what keeps that source *looking* like the block. The
//! text is decorated, never rewritten: `**bold**` is set in bold with its `**` still on the
//! line, a code span sits in the mono with its backticks beside it, a link's label wears the
//! link ink with its `[](…)` machinery in the margin ink. The reader edits the real
//! characters and sees what they mean at the same time.
//!
//! This lives in `doc-core` for the same reason every view does: the editing surface must
//! never re-implement the format. The scanners are shared with [`super::inline`] — one
//! grammar, two emitters — so the field and the page cannot disagree about where a code
//! span or a link begins.
//!
//! # The one invariant
//! Stripping every tag from the output and unescaping the entities yields the input,
//! byte for byte. The surface maps caret offsets between the field's text and the block's
//! source on that promise; a decoration that dropped or invented a character would move
//! the reader's caret mid-word.
//!
//! # A known, accepted difference
//! The page applies emphasis *after* links, so `**see [x](y)**` bolds across the anchor.
//! Here emphasis is applied within each prose segment, so a mark pair that straddles a link
//! shows its markers undecorated. The characters are untouched either way; only the styling
//! of that one straddle differs, and saving is unaffected.

use super::escape::escape_html;
use super::inline::{parse_link, split_code_spans, Span};

/// Render one field's text as decorated HTML: the source, with its marks styled in place.
///
/// Lines are decorated independently — the same boundary the page's own inline renderer
/// works in — and newlines pass through untouched.
#[must_use]
pub fn field_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        line_html(line, &mut out);
    }
    out
}

/// Decorate one line: a structural lead if the line carries one, then the inline marks.
fn line_html(line: &str, out: &mut String) {
    let lead = lead_length(line);
    if lead > 0 {
        mark(&escape_html(&line[..lead]), out);
    }
    let escaped = escape_html(&line[lead..]);
    for span in split_code_spans(&escaped) {
        match span {
            Span::Code(body) => {
                mark("`", out);
                out.push_str("<code>");
                out.push_str(body);
                out.push_str("</code>");
                mark("`", out);
            }
            Span::Prose(body) => links(body, out),
        }
    }
}

/// How many bytes of `line` are a list or checklist lead: `- `, `1. `, `[ ] `, `[x] `.
///
/// The lead is machinery — the page renders it as a bullet, a number, or a checkbox — so
/// the field sets it in the margin ink rather than pretending it is prose. Only the shapes
/// the parser itself recognizes count; anything else is a sentence that happens to open
/// with a dash.
fn lead_length(line: &str) -> usize {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];

    if let Some(after) = rest.strip_prefix("- ") {
        // A checklist item inside a list keeps both leads: `- [x] done`.
        return indent + 2 + checkbox_length(after);
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && rest[digits..].starts_with(". ") {
        return indent + digits + 2;
    }
    indent + checkbox_length(rest)
}

/// The length of a `[ ] ` / `[x] ` checkbox at the start of `text`, or zero.
fn checkbox_length(text: &str) -> usize {
    let mut chars = text.chars();
    if chars.next() != Some('[') {
        return 0;
    }
    if !matches!(chars.next(), Some(' ' | 'x' | 'X')) {
        return 0;
    }
    if chars.next() != Some(']') {
        return 0;
    }
    match chars.next() {
        Some(' ') => 4,
        _ => 0,
    }
}

/// Decorate `[label](target)` spans in escaped prose, emphasis applied around and inside.
fn links(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(link) = parse_link(&rest[open..]) else {
            // Not a link: emit through the bracket and keep scanning after it, exactly the
            // way the page's renderer leaves a stray bracket in the sentence.
            emphasis(&rest[..=open], out);
            rest = &rest[open + 1..];
            continue;
        };
        emphasis(&rest[..open], out);
        mark("[", out);
        out.push_str("<span class=\"dx-link-label\">");
        emphasis(link.label, out);
        out.push_str("</span>");
        mark("](", out);
        out.push_str("<span class=\"dx-link-target\">");
        out.push_str(link.target);
        out.push_str("</span>");
        mark(")", out);
        rest = &rest[open + link.consumed..];
    }
    emphasis(rest, out);
}

/// Decorate `**bold**` runs, then `*italic*` over what remains — the page's own precedence.
fn emphasis(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some((before, inner, after)) = delimited(rest, "**") {
        italics(before, out);
        mark("**", out);
        out.push_str("<strong>");
        italics(inner, out);
        out.push_str("</strong>");
        mark("**", out);
        rest = after;
    }
    italics(rest, out);
}

/// Decorate `*italic*` runs in text the bold pass has already spoken for.
fn italics(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some((before, inner, after)) = delimited(rest, "*") {
        out.push_str(before);
        mark("*", out);
        out.push_str("<em>");
        out.push_str(inner);
        out.push_str("</em>");
        mark("*", out);
        rest = after;
    }
    out.push_str(rest);
}

/// The first valid `marker…marker` run in `text`: the prose before it, the run's inner
/// text, and everything after the closing marker.
///
/// Validity is the page's rule: an empty or whitespace-led inner is not emphasis, so
/// `2 * 3 * 4` stays arithmetic. An invalid opener is skipped and the scan continues,
/// matching `inline::wrap_delimited`.
fn delimited<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let mut from = 0;
    loop {
        let open = from + text[from..].find(marker)?;
        let inner_start = open + marker.len();
        let close = inner_start + text[inner_start..].find(marker)?;
        let inner = &text[inner_start..close];
        if inner.is_empty() || inner.starts_with(char::is_whitespace) {
            from = inner_start;
            continue;
        }
        return Some((&text[..open], inner, &text[close + marker.len()..]));
    }
}

/// Emit already-escaped marker text in the margin ink.
fn mark(text: &str, out: &mut String) {
    out.push_str("<span class=\"dx-mark\">");
    out.push_str(text);
    out.push_str("</span>");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strip tags and unescape — the text a browser's `textContent` would report.
    fn text_of(html: &str) -> String {
        let mut out = String::new();
        let mut in_tag = false;
        for character in html.chars() {
            match character {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(character),
                _ => {}
            }
        }
        out.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    #[test]
    fn every_character_of_the_source_survives_decoration() {
        for text in [
            "plain prose",
            "**bold** and *italic* and `code`",
            "a [link](https://x.dev) mid-sentence",
            "- first\n- second\n  - nested",
            "[ ] open\n[x] done",
            "1. one\n2. two",
            "2 * 3 * 4 stays arithmetic",
            "stray ` tick and [bracket",
            "<b>raw markup</b> & entities",
            "naïve café — déjà vu 🚀\n日本語",
            "",
            "\n\n",
        ] {
            assert_eq!(text_of(&field_html(text)), text, "lost bytes in {text:?}");
        }
    }

    #[test]
    fn marks_are_visible_and_the_content_is_styled() {
        let out = field_html("**loud**");
        assert_eq!(
            out,
            "<span class=\"dx-mark\">**</span><strong>loud</strong><span class=\"dx-mark\">**</span>"
        );
        assert!(field_html("*soft*").contains("<em>soft</em>"));
        assert!(field_html("`x = 1`").contains("<code>x = 1</code>"));
    }

    #[test]
    fn a_link_wears_its_machinery_in_the_margin_ink() {
        let out = field_html("[docs](https://x.dev)");
        assert!(
            out.contains("<span class=\"dx-link-label\">docs</span>"),
            "{out}"
        );
        assert!(
            out.contains("<span class=\"dx-link-target\">https://x.dev</span>"),
            "{out}"
        );
        assert!(out.contains("<span class=\"dx-mark\">](</span>"), "{out}");
    }

    #[test]
    fn code_spans_are_not_re_scanned_for_marks() {
        let out = field_html("`a *b* c`");
        assert!(out.contains("<code>a *b* c</code>"), "{out}");
        assert!(!out.contains("<em>"), "{out}");
    }

    #[test]
    fn list_and_checklist_leads_are_machinery() {
        assert!(field_html("- item").starts_with("<span class=\"dx-mark\">- </span>"));
        assert!(field_html("12. item").starts_with("<span class=\"dx-mark\">12. </span>"));
        assert!(field_html("[x] done").starts_with("<span class=\"dx-mark\">[x] </span>"));
        assert!(field_html("- [ ] todo").starts_with("<span class=\"dx-mark\">- [ ] </span>"));
        // A sentence that happens to open with a dash is prose, not a list.
        assert!(!field_html("-dash word").contains("dx-mark"));
    }

    #[test]
    fn emphasis_rules_match_the_page() {
        // Whitespace-led and empty runs are not emphasis — the page's own rule.
        assert!(!field_html("2 * 3 * 4").contains("<em>"));
        assert!(!field_html("a ** b ** c").contains("<strong>"));
        // Italic nests inside bold, as it does on the page.
        let nested = field_html("**a *b* c**");
        assert!(nested.contains("<strong>"), "{nested}");
        assert!(nested.contains("<em>b</em>"), "{nested}");
    }

    #[test]
    fn author_markup_is_escaped_before_decoration() {
        let out = field_html("<script>bad()</script>");
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
    }
}
