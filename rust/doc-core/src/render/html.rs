//! Document → HTML. One block at a time, no framework, no network.
//!
//! [`html`] produces either a complete standalone page (default) or a bare fragment. A
//! standalone page inlines the stylesheet and every asset it needs, so the file can be
//! opened straight from disk, screenshotted headlessly, or handed to a webview with a
//! strict content-security policy — all from the same bytes.
//!
//! # Safety posture
//! Prose is escaped ([`super::inline`]); author markup (`::html`, `::svg`) is sanitized
//! ([`super::escape::sanitize_markup`]). Document-authored CSS (`::style`, `::stylesheet`)
//! is **inert by default** and only emitted when the caller sets
//! [`HtmlOptions::document_css`] — the format contract's rule that in-document CSS never
//! silently changes how a document reads.

use super::escape::{escape_html, escape_style, extract_svg, sanitize_markup};
use super::inline::inline_html;
use super::template::{self, Values};
use super::theme::{stylesheet, Theme};
use crate::model::{Block, Document, Item};

/// How to render a document to HTML.
#[derive(Debug, Clone)]
pub struct HtmlOptions {
    /// Palette to render with.
    pub theme: Theme,
    /// Emit only the document container instead of a full HTML page.
    pub fragment: bool,
    /// `<title>` for the standalone page; falls back to the document title.
    pub title: String,
    /// Include blocks marked `hidden` (off by default, as the author intended).
    pub include_hidden: bool,
    /// Apply the document's own `::style`/`::stylesheet` blocks. Off by default.
    pub document_css: bool,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            theme: Theme::Auto,
            fragment: false,
            title: String::new(),
            include_hidden: false,
            document_css: false,
        }
    }
}

/// Render `document` to HTML under `options`.
#[must_use]
pub fn html(document: &Document, options: &HtmlOptions) -> String {
    let values = template::collect(document);
    let body = blocks_html(document, options, &values);

    if options.fragment {
        return body;
    }

    let title = pick_title(document, options);
    let theme_attr = options
        .theme
        .attribute()
        .map(|value| format!(" data-theme=\"{value}\""))
        .unwrap_or_default();
    let doc_css = if options.document_css {
        document_css(document)
    } else {
        String::new()
    };

    format!(
        "<!doctype html>\n<html lang=\"en\"{theme_attr}>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{}</style>\n{doc_css}</head>\n<body>\n{body}</body>\n</html>\n",
        escape_html(&title),
        stylesheet(),
    )
}

/// Render just the document container and its blocks.
fn blocks_html(document: &Document, options: &HtmlOptions, values: &Values) -> String {
    let mut out = String::from("<div class=\"dx-doc\">\n");
    for block in &document.blocks {
        if block.hidden && !options.include_hidden {
            continue;
        }
        let rendered = block_html(block, values);
        if !rendered.is_empty() {
            out.push_str(&rendered);
            out.push('\n');
        }
    }
    out.push_str("</div>\n");
    out
}

/// The page title: the caller's override, the document title, then the first heading.
fn pick_title(document: &Document, options: &HtmlOptions) -> String {
    for candidate in [
        options.title.trim(),
        document.title.trim(),
        document.first_heading_text().trim(),
    ] {
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }
    "Document".to_string()
}

/// Collect the document's own CSS into a `<style>` element (opt-in only).
fn document_css(document: &Document) -> String {
    let mut css = String::new();
    for block in &document.blocks {
        match block.kind.as_str() {
            "style" => {
                css.push_str(&escape_style(&block.text));
                css.push('\n');
            }
            "stylesheet" if !block.href.is_empty() => {
                css.push_str(&format!("@import url(\"{}\");\n", escape_html(&block.href)));
            }
            _ => {}
        }
    }
    if css.is_empty() {
        String::new()
    } else {
        format!("<style data-dx-document-css>\n{css}</style>\n")
    }
}

/// Render one block. Returns the empty string for blocks with nothing to show.
fn block_html(block: &Block, values: &Values) -> String {
    let text = template::interpolate(&block.text, values);

    match block.kind.as_str() {
        "heading" => {
            let level = block.level.clamp(1, 4);
            format!(
                "<h{level}{}>{}</h{level}>",
                attributes(block, &[]),
                inline_html(&text)
            )
        }
        "paragraph" => format!("<p{}>{}</p>", attributes(block, &[]), paragraph_body(&text)),
        "quote" => format!(
            "<blockquote{}>{}</blockquote>",
            attributes(block, &[]),
            paragraph_body(&text)
        ),
        "bulleted-list" => list_html(block, "ul", values),
        "numbered-list" => list_html(block, "ol", values),
        "checklist" => checklist_html(block, values),
        "code" => code_html(block, &text),
        "output" => output_html(block, &text),
        "image" => image_html(block, values),
        "rule" => format!("<hr{}>", attributes(block, &[])),
        "svg" => {
            let svg = extract_svg(&text);
            if svg.is_empty() {
                String::new()
            } else {
                format!(
                    "<div{}>{}</div>",
                    attributes(block, &["dx-svg"]),
                    sanitize_markup(&svg)
                )
            }
        }
        "html" => format!(
            "<div{}>{}</div>",
            attributes(block, &["dx-html"]),
            sanitize_markup(&text)
        ),
        "mermaid" | "graph" => format!(
            "<pre{}>{}</pre>",
            attributes(block, &["dx-mermaid"]),
            escape_html(&text)
        ),
        // style/stylesheet/script carry no visible body; their effect (if any) is applied
        // by `document_css` and `template::collect` instead.
        _ => String::new(),
    }
}

/// Render paragraph-like text: blank lines split it into separate visual lines.
fn paragraph_body(text: &str) -> String {
    text.split('\n')
        .map(inline_html)
        .collect::<Vec<_>>()
        .join("<br>")
}

/// Render a bulleted or numbered list, including nested levels.
fn list_html(block: &Block, tag: &str, values: &Values) -> String {
    format!(
        "<{tag}{}>\n{}</{tag}>",
        attributes(block, &[]),
        items_html(&block.items, tag, values)
    )
}

/// Render list items, recursing into nested children.
fn items_html(items: &[Item], tag: &str, values: &Values) -> String {
    let mut out = String::new();
    for item in items {
        let text = inline_html(&template::interpolate(&item.text, values));
        if item.nested.is_empty() {
            out.push_str(&format!("<li>{text}</li>\n"));
        } else {
            out.push_str(&format!(
                "<li>{text}\n<{tag}>\n{}</{tag}>\n</li>\n",
                items_html(&item.nested, tag, values)
            ));
        }
    }
    out
}

/// Render a checklist as a list of ticked/unticked entries.
fn checklist_html(block: &Block, values: &Values) -> String {
    let mut out = format!("<ul{}>\n", attributes(block, &["dx-checklist"]));
    for item in &block.items {
        let text = inline_html(&template::interpolate(&item.text, values));
        let (mark, class) = if item.checked {
            ("[x]", " class=\"dx-done\"")
        } else {
            ("[ ]", "")
        };
        out.push_str(&format!(
            "<li><span class=\"dx-mark\">{mark}</span><span{class}>{text}</span></li>\n"
        ));
    }
    out.push_str("</ul>");
    out
}

/// Render a code block: the verbatim source, and a marginal note saying what it is.
///
/// There is no header bar and no badges. What the block *is* — its language, whether it runs,
/// what it needs — goes into one `data-label` attribute that the stylesheet sets as a faint
/// note in the margin. The code is the content; everything else stays quiet.
fn code_html(block: &Block, text: &str) -> String {
    let language = if block.language.is_empty() {
        "text"
    } else {
        &block.language
    };

    let mut parts = vec![language.to_string()];
    if block.run {
        parts.push("run".to_string());
    }
    if !block.deps.is_empty() {
        parts.push(block.deps.clone());
    }

    format!(
        "<div{} data-label=\"{}\">\n<pre><code class=\"language-{}\">{}</code></pre>\n</div>",
        attributes(block, &["dx-code"]),
        escape_html(&parts.join(" · ")),
        escape_html(language),
        escape_html(text)
    )
}

/// Render a captured run result beneath the code that produced it.
///
/// A block that declared `format=svg` or `format=html` had its code *draw* something, so
/// the output is rendered as markup rather than quoted as text — that is what lets a
/// generated chart appear as a chart. The markup is sanitized like any author markup.
/// A successful run needs no announcement: the result sits under the code that produced it,
/// set apart by the stylesheet, and that is all a reader needs. A *failure* is different — it
/// is the one thing about a run worth saying out loud, so only errors carry a note.
fn output_html(block: &Block, text: &str) -> String {
    let failed = block.status == "error" || block.exit != 0;
    if !failed && !block.format.is_empty() {
        return rendered_output_html(block, text);
    }
    if !failed {
        return format!(
            "<div{}>\n<pre>{}</pre>\n</div>",
            attributes(block, &["dx-output"]),
            escape_html(text)
        );
    }

    let note = if block.exit != 0 {
        format!("error · exit {}", block.exit)
    } else {
        "error".to_string()
    };
    format!(
        "<div{} data-note=\"{}\">\n<pre>{}</pre>\n</div>",
        attributes(block, &["dx-output", "dx-output-error"]),
        escape_html(&note),
        escape_html(text)
    )
}

/// Render output that the code produced as markup (`format=svg` / `format=html`).
fn rendered_output_html(block: &Block, text: &str) -> String {
    let markup = if block.format == "svg" {
        sanitize_markup(&extract_svg(text))
    } else {
        sanitize_markup(text)
    };
    if markup.trim().is_empty() {
        // The block promised markup and produced none; show what it did print instead of
        // silently rendering an empty box.
        return format!(
            "<div{} data-note=\"no {} produced\">\n<pre>{}</pre>\n</div>",
            attributes(block, &["dx-output", "dx-output-error"]),
            escape_html(&block.format),
            escape_html(text)
        );
    }
    format!(
        "<div{}>{markup}</div>",
        attributes(block, &["dx-output", "dx-output-rendered"])
    )
}

/// Render an image, using the alt text as both alt attribute and caption.
fn image_html(block: &Block, values: &Values) -> String {
    if block.src.is_empty() {
        return String::new();
    }
    let alt = template::interpolate(&block.alt, values);
    let caption = if alt.is_empty() {
        String::new()
    } else {
        format!("\n<figcaption>{}</figcaption>", inline_html(&alt))
    };
    format!(
        "<figure{}>\n<img src=\"{}\" alt=\"{}\">{caption}\n</figure>",
        attributes(block, &[]),
        escape_html(&block.src),
        escape_html(&alt)
    )
}

/// Build the shared `id`/`data-block-id`/`class` attribute string for a block.
fn attributes(block: &Block, base_classes: &[&str]) -> String {
    let mut parts = Vec::new();
    if !block.id.is_empty() {
        let id = escape_html(&block.id);
        parts.push(format!("id=\"{id}\""));
        parts.push(format!("data-block-id=\"{id}\""));
    }

    let mut classes: Vec<&str> = base_classes.to_vec();
    for class in block.class_name.split_whitespace() {
        if !classes.contains(&class) {
            classes.push(class);
        }
    }
    if !classes.is_empty() {
        parts.push(format!("class=\"{}\"", escape_html(&classes.join(" "))));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" {}", parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse;

    fn fragment(source: &str) -> String {
        html(
            &parse(source),
            &HtmlOptions {
                fragment: true,
                ..HtmlOptions::default()
            },
        )
    }

    #[test]
    fn standalone_page_is_self_contained() {
        let page = html(
            &parse("::heading level=1 id=h\nTitle\n::end\n"),
            &HtmlOptions::default(),
        );
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<title>Title</title>"));
        assert!(page.contains("--dx-bg"));
        // No external references of any kind.
        assert!(!page.contains("http://"));
        assert!(!page.contains("<link "));
        assert!(!page.contains("<script"));
    }

    #[test]
    fn forced_theme_sets_the_root_attribute() {
        let page = html(
            &parse("::paragraph id=p\nx\n::end\n"),
            &HtmlOptions {
                theme: Theme::Dark,
                ..HtmlOptions::default()
            },
        );
        assert!(page.contains("<html lang=\"en\" data-theme=\"dark\">"));
    }

    #[test]
    fn blocks_carry_their_ids_for_addressing() {
        let out = fragment("::paragraph id=intro\nHello\n::end\n");
        assert!(out.contains("<p id=\"intro\" data-block-id=\"intro\">Hello</p>"));
    }

    #[test]
    fn hidden_blocks_stay_hidden_unless_requested() {
        let source = "::paragraph id=p hidden\nSecret\n::end\n";
        assert!(!fragment(source).contains("Secret"));
        let shown = html(
            &parse(source),
            &HtmlOptions {
                fragment: true,
                include_hidden: true,
                ..HtmlOptions::default()
            },
        );
        assert!(shown.contains("Secret"));
    }

    #[test]
    fn document_css_is_inert_by_default() {
        let source = "::style id=s\np { color: red }\n::end\n\n::paragraph id=p\nx\n::end\n";
        let page = html(&parse(source), &HtmlOptions::default());
        assert!(!page.contains("color: red"));

        let with_css = html(
            &parse(source),
            &HtmlOptions {
                document_css: true,
                ..HtmlOptions::default()
            },
        );
        assert!(with_css.contains("color: red"));
        assert!(with_css.contains("data-dx-document-css"));
    }

    #[test]
    fn a_code_block_names_itself_in_one_marginal_label() {
        let out = fragment(
            "::code id=c lang=python run deps=rich\nprint(1)\n::end\n\n::output id=o for=c status=ok\n1\n::end\n",
        );
        // Language, runnability, and dependencies in one quiet attribute — no header bar and
        // no pills in the markup at all.
        assert!(out.contains("data-label=\"python · run · rich\""), "{out}");
        assert!(!out.contains("dx-badge"), "{out}");
        assert!(!out.contains("dx-code-head"), "{out}");

        // A successful run is not announced; the result simply follows the code.
        assert!(out.contains("<pre>1</pre>"), "{out}");
        assert!(!out.contains("dx-output-head"), "{out}");
        assert!(!out.contains("data-note"), "success needs no note: {out}");
    }

    #[test]
    fn a_plain_code_block_still_says_what_language_it_is() {
        let out = fragment("::code id=c lang=js\nconst x = 1;\n::end\n");
        assert!(out.contains("data-label=\"js\""), "{out}");
    }

    #[test]
    fn failed_output_is_marked_with_its_exit_code() {
        let out = fragment("::output id=o for=c status=error exit=2\nboom\n::end\n");
        assert!(out.contains("dx-output-error"));
        assert!(out.contains("data-note=\"error · exit 2\""), "{out}");
    }

    #[test]
    fn drawn_output_renders_as_a_picture_not_as_quoted_source() {
        let out = fragment(
            "::output id=o for=c status=ok format=svg\n<svg viewBox=\"0 0 10 10\"><rect/></svg>\n::end\n",
        );
        assert!(out.contains("dx-output-rendered"));
        assert!(out.contains("<svg viewBox=\"0 0 10 10\"><rect/></svg>"));
        assert!(!out.contains("&lt;svg"));
    }

    #[test]
    fn drawn_output_is_still_sanitized() {
        let out = fragment(
            "::output id=o for=c status=ok format=html\n<p onclick=\"x()\">hi</p><script>bad()</script>\n::end\n",
        );
        assert!(out.contains("<p>hi</p>"));
        assert!(!out.contains("onclick"));
        assert!(!out.contains("bad()"));
    }

    #[test]
    fn a_drawing_block_that_drew_nothing_shows_what_it_printed_instead() {
        let out = fragment("::output id=o for=c status=ok format=svg\nTraceback: nope\n::end\n");
        assert!(out.contains("no svg produced"));
        assert!(out.contains("Traceback: nope"));
    }

    #[test]
    fn a_failed_drawing_block_shows_its_error_text() {
        let out = fragment("::output id=o for=c status=error exit=1 format=svg\nboom\n::end\n");
        assert!(out.contains("dx-output-error"));
        assert!(out.contains("boom"));
    }

    #[test]
    fn author_markup_is_sanitized_but_preserved() {
        let out = fragment("::html id=h\n<table><tr><td onclick=\"x()\">c</td></tr></table><script>bad()</script>\n::end\n");
        assert!(out.contains("<table><tr><td>c</td></tr></table>"));
        assert!(!out.contains("onclick"));
        assert!(!out.contains("bad()"));
    }

    #[test]
    fn a_document_full_of_non_ascii_prose_renders_end_to_end() {
        let out = fragment(
            "::heading level=1 id=h\nRésumé — 概要 🚀\n::end\n\n\
::paragraph id=p\nEm dash — and **bold é** and `köde`.\n::end\n\n\
::bulleted-list id=l\n- première\n- 二番目\n::end\n",
        );
        assert!(out.contains("Résumé — 概要 🚀"));
        assert!(out.contains("<strong>bold é</strong>"));
        assert!(out.contains("<li>première</li>"));
    }

    #[test]
    fn placeholders_resolve_from_json_scripts() {
        let out = fragment(
            "::script id=v type=application/json\n{\"phase\":\"beta\"}\n::end\n\n::paragraph id=p\nPhase {{phase}}.\n::end\n",
        );
        assert!(out.contains("Phase beta."));
    }

    #[test]
    fn lists_and_checklists_render_their_items() {
        assert!(fragment("::bulleted-list id=l\n- a\n- b\n::end\n").contains("<li>a</li>"));
        let checks = fragment("::checklist id=c\n[x] done\n[ ] todo\n::end\n");
        assert!(checks.contains("dx-checklist"));
        assert!(checks.contains("dx-done"));
    }

    #[test]
    fn every_real_example_renders_without_leaking_block_markers() {
        for source in [
            include_str!("../../../../examples/welcome.dx"),
            include_str!("../../../../examples/tutorial.dx"),
            include_str!("../../../../examples/block-reference.dx"),
        ] {
            let page = html(&parse(source), &HtmlOptions::default());
            assert!(!page.contains("::end"), "raw block marker leaked into HTML");
            assert!(page.contains("<div class=\"dx-doc\">"));
        }
    }
}
