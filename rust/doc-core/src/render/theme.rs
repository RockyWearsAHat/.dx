//! The built-in document stylesheet.
//!
//! One stylesheet serves every surface — CLI preview, MCP render, screenshot capture, and
//! the VS Code webview — so a document looks the same everywhere it is opened. It is
//! embedded in the binary rather than fetched, which is what lets a rendered `.dx` page be
//! a single self-contained file with no network access.
//!
//! Colors are declared once as custom properties on `:root` and overridden in a
//! `prefers-color-scheme: dark` block and by an explicit `[data-theme]` attribute, so the
//! same markup honors the reader's system setting or a caller-forced theme.
//!
//! # The look: a blank sheet, written on
//! This is a scratch pad. The page is one plain paper tone with one ink — **no second
//! surface**: no panels, no fills, no tints, no grid, no rounded boxes, no shadows. Structure
//! comes from type, whitespace, and single hairline rules, the marks you would actually make
//! on paper. Prose is set in a serif at one comfortable measure; code is monospace writing on
//! the same sheet, set in from a margin rule rather than boxed off in a widget.
//!
//! Nothing is announced unless it earns the space. A code block has no header bar — its
//! language sits in the margin as a faint pencil note. A *successful* run says nothing at all;
//! its result simply follows the code it came from. Only a **failure** is called out, because
//! that is the one thing about a run a reader has to know.

/// Which palette a rendered page should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Follow the reader's operating-system preference (the default).
    #[default]
    Auto,
    /// Always render the light palette.
    Light,
    /// Always render the dark palette.
    Dark,
}

impl Theme {
    /// The `data-theme` attribute value for this theme, or `None` for [`Theme::Auto`],
    /// which leaves the choice to `prefers-color-scheme`.
    #[must_use]
    pub fn attribute(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Light => Some("light"),
            Self::Dark => Some("dark"),
        }
    }

    /// Parse a theme name, falling back to [`Theme::Auto`] for anything unrecognized.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Auto,
        }
    }
}

/// The complete document stylesheet, ready to inline into a `<style>` element.
#[must_use]
pub fn stylesheet() -> &'static str {
    STYLESHEET
}

const STYLESHEET: &str = r#":root {
  color-scheme: light dark;
  /* One paper tone, one ink. There is no second surface: no panels, no fills, no tints. */
  --dx-bg: #fdfcf9;
  --dx-text: #1c1a17;
  --dx-muted: #6b665c;
  --dx-faint: #a8a08f;
  --dx-rule: #e2ded2;
  --dx-accent: #35618e;
  --dx-error: #a1382a;
  --dx-select: rgba(53, 97, 142, 0.16);
  --dx-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
  --dx-serif: ui-serif, "Iowan Old Style", "Palatino Linotype", Palatino, "Book Antiqua", Georgia, serif;
}

@media (prefers-color-scheme: dark) {
  :root {
    --dx-bg: #1a1917;
    --dx-text: #ece7dd;
    --dx-muted: #a39c8f;
    --dx-faint: #6d665a;
    --dx-rule: #322e28;
    --dx-accent: #8fb6e6;
    --dx-error: #e5897b;
    --dx-select: rgba(143, 182, 230, 0.22);
  }
}

:root[data-theme="light"] {
  color-scheme: light;
  --dx-bg: #fdfcf9;
  --dx-text: #1c1a17;
  --dx-muted: #6b665c;
  --dx-faint: #a8a08f;
  --dx-rule: #e2ded2;
  --dx-accent: #35618e;
  --dx-error: #a1382a;
  --dx-select: rgba(53, 97, 142, 0.16);
}

:root[data-theme="dark"] {
  color-scheme: dark;
  --dx-bg: #1a1917;
  --dx-text: #ece7dd;
  --dx-muted: #a39c8f;
  --dx-faint: #6d665a;
  --dx-rule: #322e28;
  --dx-accent: #8fb6e6;
  --dx-error: #e5897b;
  --dx-select: rgba(143, 182, 230, 0.22);
}

* { box-sizing: border-box; }

body {
  margin: 0;
  background: var(--dx-bg);
  color: var(--dx-text);
  font-family: var(--dx-serif);
  font-size: 17px;
  line-height: 1.7;
  -webkit-font-smoothing: antialiased;
  text-rendering: optimizeLegibility;
}

::selection { background: var(--dx-select); }

/* The sheet: one comfortable measure, wide margins, nothing framing it. */
.dx-doc {
  max-width: 46rem;
  margin: 0 auto;
  padding: 4rem 1.75rem 6rem;
}

.dx-doc > * { margin: 0 0 1.15rem; }
.dx-doc > *:last-child { margin-bottom: 0; }

h1, h2, h3, h4 {
  font-family: var(--dx-serif);
  font-weight: 600;
  line-height: 1.3;
  letter-spacing: -0.008em;
  margin: 2.4rem 0 0.9rem;
}
.dx-doc > h1:first-child,
.dx-doc > h2:first-child,
.dx-doc > h3:first-child { margin-top: 0; }

h1 { font-size: 1.95rem; letter-spacing: -0.015em; }
h2 { font-size: 1.4rem; }
h3 { font-size: 1.15rem; }
/* A fourth level is a quiet lead-in, not a shouted label. */
h4 { font-size: 1rem; font-weight: 600; color: var(--dx-muted); }

p { margin: 0 0 1.15rem; }

a {
  color: var(--dx-accent);
  text-decoration-thickness: 1px;
  text-underline-offset: 2.5px;
}

ul, ol { margin: 0 0 1.15rem; padding-left: 1.35rem; }
li { margin: 0.3rem 0; }
li > ul, li > ol { margin: 0.3rem 0 0; }

/* Checklists read as ticked boxes written in the margin, not as a styled widget. */
.dx-checklist { list-style: none; padding-left: 0; }
.dx-checklist li { display: flex; gap: 0.6rem; align-items: baseline; }
.dx-checklist .dx-mark { font-family: var(--dx-mono); font-size: 0.85em; color: var(--dx-muted); }
.dx-checklist .dx-done { color: var(--dx-muted); }

/* A quotation is set in from the margin by a rule. No fill. */
blockquote {
  margin: 0 0 1.15rem;
  padding: 0.1rem 0 0.1rem 1.15rem;
  border-left: 1px solid var(--dx-rule);
  color: var(--dx-muted);
  font-style: italic;
}
blockquote p:last-child { margin-bottom: 0; }

hr {
  border: 0;
  border-top: 1px solid var(--dx-rule);
  margin: 2.6rem auto;
}

/* Code is writing on the same sheet: monospace, set in from a single margin rule. No panel,
   no border box, no shadow, no header bar. What the block is goes in the margin as a faint
   pencil note and stays out of the way. */
.dx-code {
  position: relative;
  margin: 0 0 1.15rem;
  padding-left: 1.15rem;
  border-left: 1px solid var(--dx-rule);
  background: none;
}

.dx-code::after {
  content: attr(data-label);
  position: absolute;
  top: 0.1rem;
  right: 0;
  font: 400 0.68rem/1 var(--dx-mono);
  letter-spacing: 0.02em;
  color: var(--dx-faint);
  opacity: 0.7;
  transition: opacity 0.15s ease;
  pointer-events: none;
  user-select: none;
}
@media (hover: hover) {
  .dx-code:hover::after { opacity: 1; }
}

/* Long lines wrap rather than scroll out of view. A rendered document is also read as a
   static image, where a horizontal scrollbar silently truncates code and output. */
.dx-code pre, .dx-output pre {
  margin: 0;
  padding: 0;
  background: none;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-family: var(--dx-mono);
  font-size: 0.83rem;
  line-height: 1.65;
  tab-size: 2;
}

/* Inline code is monospace and nothing else — a tint or a border mid-sentence marks the page
   and breaks the line. */
code {
  font-family: var(--dx-mono);
  font-size: 0.86em;
  background: none;
  padding: 0;
}

/* What a run produced continues the same column, one step quieter than the code. */
.dx-output {
  margin: 0 0 1.15rem;
  padding-left: 1.15rem;
  border-left: 1px solid var(--dx-rule);
  background: none;
}
.dx-output pre { color: var(--dx-muted); }
/* A result sits close to the code it came from, so the pairing is obvious without a box. */
.dx-code + .dx-output,
.dx-code + .dx-output-rendered { margin-top: -0.6rem; }

/* A failed run is the one thing worth saying out loud. */
.dx-output-error {
  position: relative;
  border-left-color: var(--dx-error);
}
.dx-output-error pre { color: var(--dx-error); }
.dx-output-error::after {
  content: attr(data-note);
  position: absolute;
  top: 0.1rem;
  right: 0;
  font: 400 0.68rem/1 var(--dx-mono);
  color: var(--dx-error);
  pointer-events: none;
  user-select: none;
}

/* A drawing the code produced is just the drawing, on the page. */
.dx-output-rendered {
  margin: 0 0 1.15rem;
  padding: 0.25rem 0 0;
  border: 0;
  background: none;
}
.dx-output-rendered svg { width: 100%; max-width: 100%; height: auto; display: block; }
.dx-output-rendered > *:last-child { margin-bottom: 0; }

/* Tables are ruled, not filled: no striping, no shaded header. */
table {
  width: 100%;
  border-collapse: collapse;
  margin: 0 0 1.15rem;
  font-size: 0.92rem;
  line-height: 1.5;
}
th, td {
  padding: 0.55rem 0.7rem 0.55rem 0;
  text-align: left;
  vertical-align: top;
  border-bottom: 1px solid var(--dx-rule);
}
thead th { font-weight: 600; color: var(--dx-muted); }
tbody tr:last-child td { border-bottom: 0; }

/* Author markup can be genuinely wider than the page (a table with many columns). It
   scrolls inside its own box so the page body never scrolls sideways. */
.dx-html { margin: 0 0 1.15rem; overflow-x: auto; }
.dx-svg { margin: 0 0 1.15rem; }
.dx-svg svg, img { max-width: 100%; height: auto; }

figure { margin: 0 0 1.15rem; }
figcaption {
  color: var(--dx-muted);
  font-size: 0.86rem;
  font-style: italic;
  margin-top: 0.5rem;
}

.dx-mermaid {
  margin: 0 0 1.15rem;
  padding-left: 1.15rem;
  border-left: 1px solid var(--dx-rule);
  background: none;
  font-family: var(--dx-mono);
  font-size: 0.8rem;
  color: var(--dx-muted);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/* On paper: real paper. The marginal notes are a screen affordance. */
@media print {
  :root {
    --dx-bg: #fff;
    --dx-text: #000;
    --dx-rule: #ccc;
  }
  body { background: #fff; font-size: 11pt; }
  .dx-doc { max-width: none; padding: 0; }
  .dx-code::after { display: none; }
  h1, h2, h3, h4 { break-after: avoid; }
  .dx-code, .dx-output, table, figure { break-inside: avoid; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_names_and_falls_back_to_auto() {
        assert_eq!(Theme::parse("Dark"), Theme::Dark);
        assert_eq!(Theme::parse(" light "), Theme::Light);
        assert_eq!(Theme::parse("nonsense"), Theme::Auto);
    }

    #[test]
    fn auto_theme_sets_no_attribute() {
        assert_eq!(Theme::Auto.attribute(), None);
        assert_eq!(Theme::Dark.attribute(), Some("dark"));
    }

    #[test]
    fn stylesheet_defines_both_palettes() {
        let css = stylesheet();
        assert!(css.contains("prefers-color-scheme: dark"));
        assert!(css.contains(":root[data-theme=\"light\"]"));
        assert!(css.contains(":root[data-theme=\"dark\"]"));
    }

    #[test]
    fn code_and_output_wrap_so_a_screenshot_never_truncates_them() {
        let css = stylesheet();
        assert!(css.contains("white-space: pre-wrap"));
        assert!(css.contains("overflow-wrap: anywhere"));
    }

    #[test]
    fn wide_author_markup_scrolls_in_its_own_box_rather_than_the_page() {
        let css = stylesheet();
        let html_rule = css
            .split(".dx-html {")
            .nth(1)
            .expect("a .dx-html rule")
            .split('}')
            .next()
            .expect("its body");
        assert!(
            html_rule.contains("overflow-x: auto"),
            "wide markup must scroll in its own box: {html_rule}"
        );
    }

    #[test]
    fn the_page_carries_no_interface_chrome() {
        // The look is a blank sheet: structure comes from type, whitespace, and hairlines —
        // not boxes, shadows, and pills. Any of these creeping back is the clutter this
        // replaced.
        let css = stylesheet();
        assert!(
            !css.contains("box-shadow"),
            "shadows are interface, not paper"
        );
        assert!(!css.contains("dx-badge"), "badges were removed");
        assert!(!css.contains("dx-code-head"), "the header bar was removed");
        assert!(
            !css.contains("dx-output-head"),
            "the header bar was removed"
        );
        assert!(
            !css.contains("text-transform: uppercase"),
            "nothing on the page shouts"
        );
        assert!(
            !css.contains("border-radius"),
            "a scratch pad has no rounded widgets"
        );
    }

    #[test]
    fn there_is_exactly_one_paper_tone_and_nothing_is_filled() {
        // A second surface colour is what turns a sheet of paper into a control panel, so the
        // palette has no panel/surface tone at all and no rule paints a background.
        let css = stylesheet();
        assert!(!css.contains("--dx-panel"), "no second surface");
        assert!(!css.contains("--dx-surface"), "no second surface");
        for fill in ["background: var(--dx-panel", "background: var(--dx-surface"] {
            assert!(!css.contains(fill), "nothing may be filled: {fill}");
        }
        // Every block that a UI would have boxed refuses a background outright.
        for selector in [".dx-code {", ".dx-output {", ".dx-mermaid {", "code {"] {
            let rule = css
                .split(selector)
                .nth(1)
                .expect(selector)
                .split('}')
                .next()
                .expect("body");
            assert!(
                rule.contains("background: none"),
                "{selector} must stay unfilled: {rule}"
            );
        }
    }

    #[test]
    fn blocks_are_set_in_from_a_margin_rule_rather_than_boxed() {
        let css = stylesheet();
        for selector in [".dx-code {", ".dx-output {", ".dx-mermaid {"] {
            let rule = css
                .split(selector)
                .nth(1)
                .expect(selector)
                .split('}')
                .next()
                .expect("body");
            assert!(
                rule.contains("border-left: 1px solid var(--dx-rule)"),
                "{selector} should be set in by a single rule: {rule}"
            );
        }
    }

    #[test]
    fn a_code_block_labels_itself_in_the_margin_instead_of_a_header_bar() {
        let css = stylesheet();
        assert!(css.contains("content: attr(data-label)"));
        assert!(
            css.contains("--dx-faint"),
            "the note is faint, not prominent"
        );
    }

    #[test]
    fn only_a_failed_run_is_called_out() {
        let css = stylesheet();
        assert!(css.contains(".dx-output-error"));
        assert!(css.contains("content: attr(data-note)"));
    }

    #[test]
    fn prose_is_set_in_a_serif_and_code_in_a_monospace() {
        let css = stylesheet();
        assert!(css.contains("--dx-serif"));
        assert!(css.contains("font-family: var(--dx-serif)"));
        assert!(css.contains("font-family: var(--dx-mono)"));
    }

    #[test]
    fn printing_drops_the_screen_washes_and_marginal_notes() {
        let css = stylesheet();
        let print = css.split("@media print").nth(1).expect("a print block");
        assert!(print.contains("--dx-bg: #fff"));
        assert!(print.contains(".dx-code::after { display: none; }"));
        assert!(print.contains("break-inside: avoid"));
    }
}
