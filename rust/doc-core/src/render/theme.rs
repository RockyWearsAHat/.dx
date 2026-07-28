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
  --dx-bg: #ffffff;
  --dx-surface: #f6f8fb;
  --dx-surface-2: #eef2f8;
  --dx-border: #d9e0ea;
  --dx-text: #10161f;
  --dx-muted: #5a6675;
  --dx-accent: #1f6feb;
  --dx-accent-soft: rgba(31, 111, 235, 0.12);
  --dx-ok: #16794c;
  --dx-error: #c0362c;
  --dx-code-bg: #f4f6fa;
  --dx-shadow: 0 1px 2px rgba(16, 22, 31, 0.06);
  --dx-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
  --dx-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, "Helvetica Neue", Arial, sans-serif;
}

@media (prefers-color-scheme: dark) {
  :root {
    --dx-bg: #0d1117;
    --dx-surface: #151b23;
    --dx-surface-2: #1b232d;
    --dx-border: #2b3440;
    --dx-text: #e6edf3;
    --dx-muted: #9aa7b4;
    --dx-accent: #6cb0ff;
    --dx-accent-soft: rgba(108, 176, 255, 0.14);
    --dx-ok: #4ec98a;
    --dx-error: #ff7b72;
    --dx-code-bg: #11171f;
    --dx-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }
}

:root[data-theme="light"] {
  color-scheme: light;
  --dx-bg: #ffffff;
  --dx-surface: #f6f8fb;
  --dx-surface-2: #eef2f8;
  --dx-border: #d9e0ea;
  --dx-text: #10161f;
  --dx-muted: #5a6675;
  --dx-accent: #1f6feb;
  --dx-accent-soft: rgba(31, 111, 235, 0.12);
  --dx-ok: #16794c;
  --dx-error: #c0362c;
  --dx-code-bg: #f4f6fa;
  --dx-shadow: 0 1px 2px rgba(16, 22, 31, 0.06);
}

:root[data-theme="dark"] {
  color-scheme: dark;
  --dx-bg: #0d1117;
  --dx-surface: #151b23;
  --dx-surface-2: #1b232d;
  --dx-border: #2b3440;
  --dx-text: #e6edf3;
  --dx-muted: #9aa7b4;
  --dx-accent: #6cb0ff;
  --dx-accent-soft: rgba(108, 176, 255, 0.14);
  --dx-ok: #4ec98a;
  --dx-error: #ff7b72;
  --dx-code-bg: #11171f;
  --dx-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
}

* { box-sizing: border-box; }

body {
  margin: 0;
  background: var(--dx-bg);
  color: var(--dx-text);
  font-family: var(--dx-sans);
  font-size: 16px;
  line-height: 1.65;
  -webkit-font-smoothing: antialiased;
}

.dx-doc {
  max-width: 54rem;
  margin: 0 auto;
  padding: 2.5rem 1.5rem 4rem;
}

.dx-doc > * { margin: 0 0 1.05rem; }
.dx-doc > *:last-child { margin-bottom: 0; }

h1, h2, h3, h4 {
  line-height: 1.25;
  font-weight: 650;
  letter-spacing: -0.011em;
  margin: 2rem 0 0.85rem;
}
.dx-doc > h1:first-child,
.dx-doc > h2:first-child,
.dx-doc > h3:first-child { margin-top: 0; }

h1 { font-size: 2rem; }
h2 { font-size: 1.5rem; }
h3 { font-size: 1.2rem; }
h4 { font-size: 1.02rem; color: var(--dx-muted); text-transform: uppercase; letter-spacing: 0.06em; }

p { margin: 0 0 1.05rem; }

a { color: var(--dx-accent); text-decoration-thickness: 1px; text-underline-offset: 2px; }

ul, ol { margin: 0 0 1.05rem; padding-left: 1.45rem; }
li { margin: 0.22rem 0; }
li > ul, li > ol { margin: 0.22rem 0 0; }

.dx-checklist { list-style: none; padding-left: 0.1rem; }
.dx-checklist li { display: flex; gap: 0.55rem; align-items: baseline; }
.dx-checklist .dx-mark { color: var(--dx-accent); font-family: var(--dx-mono); }
.dx-checklist .dx-done { color: var(--dx-muted); text-decoration: line-through; }

blockquote {
  margin: 0 0 1.05rem;
  padding: 0.15rem 0 0.15rem 1rem;
  border-left: 3px solid var(--dx-accent);
  background: var(--dx-accent-soft);
  border-radius: 0 6px 6px 0;
  color: var(--dx-text);
}
blockquote p:last-child { margin-bottom: 0; }

hr { border: 0; border-top: 1px solid var(--dx-border); margin: 2rem 0; }

.dx-code {
  margin: 0 0 1.05rem;
  border: 1px solid var(--dx-border);
  border-radius: 10px;
  background: var(--dx-code-bg);
  overflow: hidden;
  box-shadow: var(--dx-shadow);
}

.dx-code-head {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.75rem;
  background: var(--dx-surface-2);
  border-bottom: 1px solid var(--dx-border);
  font: 600 0.72rem/1.4 var(--dx-mono);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--dx-muted);
}

.dx-badge {
  padding: 0.05rem 0.42rem;
  border-radius: 999px;
  border: 1px solid currentColor;
  font-size: 0.66rem;
  letter-spacing: 0.04em;
}
.dx-badge-run { color: var(--dx-accent); }
.dx-badge-ok { color: var(--dx-ok); }
.dx-badge-error { color: var(--dx-error); }

.dx-code pre, .dx-output pre {
  margin: 0;
  padding: 0.85rem 1rem;
  overflow-x: auto;
  font-family: var(--dx-mono);
  font-size: 0.855rem;
  line-height: 1.55;
  tab-size: 2;
}

code { font-family: var(--dx-mono); font-size: 0.9em; }
p > code, li > code, td > code {
  background: var(--dx-surface-2);
  border: 1px solid var(--dx-border);
  border-radius: 5px;
  padding: 0.08em 0.34em;
}

.dx-output {
  margin: -1.05rem 0 1.05rem;
  border: 1px solid var(--dx-border);
  border-top: 0;
  border-radius: 0 0 10px 10px;
  background: var(--dx-surface);
  overflow: hidden;
}
.dx-output-head {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.75rem;
  border-bottom: 1px solid var(--dx-border);
  font: 600 0.7rem/1.4 var(--dx-mono);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--dx-muted);
}
.dx-output-error pre { color: var(--dx-error); }

.dx-output-rendered {
  padding: 1rem;
  background: var(--dx-bg);
  border-top: 1px solid var(--dx-border);
}
.dx-output-rendered svg { width: 100%; max-width: 100%; height: auto; display: block; }
.dx-output-rendered > *:last-child { margin-bottom: 0; }

table {
  width: 100%;
  border-collapse: collapse;
  margin: 0 0 1.05rem;
  font-size: 0.94rem;
}
th, td {
  padding: 0.5rem 0.7rem;
  text-align: left;
  border-bottom: 1px solid var(--dx-border);
  vertical-align: top;
}
thead th {
  background: var(--dx-surface-2);
  font-weight: 640;
  border-bottom: 2px solid var(--dx-border);
}
tbody tr:nth-child(even) { background: var(--dx-surface); }

.dx-html, .dx-svg { margin: 0 0 1.05rem; }
.dx-svg svg, img { max-width: 100%; height: auto; }

figure { margin: 0 0 1.05rem; }
figcaption { color: var(--dx-muted); font-size: 0.86rem; margin-top: 0.4rem; }

.dx-mermaid {
  margin: 0 0 1.05rem;
  padding: 0.85rem 1rem;
  border: 1px dashed var(--dx-border);
  border-radius: 10px;
  background: var(--dx-surface);
  font-family: var(--dx-mono);
  font-size: 0.85rem;
  white-space: pre-wrap;
}

@media print {
  body { background: #fff; }
  .dx-doc { max-width: none; padding: 0; }
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
}
