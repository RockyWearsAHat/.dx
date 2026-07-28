//! Best-effort HTML rendering of a core document for the `docview://` resource.
//!
//! This is a **structural** render of the common block kinds. The full theming /
//! appearance / scoped-CSS fidelity of the reference renderer (`src/doc-view.ts`) is
//! intentionally **out of scope** for the native shell.

use doc_core::model::{Document as CoreDocument, Item};

/// Escape the five XML/HTML special characters for safe inclusion in markup.
pub(super) fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Render a core document to a readable, standalone HTML page.
///
/// This is a **structural, best-effort** render of the common block kinds (heading,
/// paragraph, list, checklist, quote, code, image, rule). In-document
/// `style`/`stylesheet`/`script` blocks are presentation/behaviour and are deliberately
/// omitted, and no per-document theme is applied. The output is escaped, semantic HTML
/// suitable for the `docview://` resource.
pub(super) fn render_document_html(document: &CoreDocument, title: &str) -> String {
    let mut body = String::new();
    for block in &document.blocks {
        if block.hidden {
            continue;
        }
        match block.kind.as_str() {
            "heading" => {
                let level = block.level.clamp(1, 4);
                body.push_str(&format!(
                    "<h{level}>{}</h{level}>\n",
                    escape_html(&block.text)
                ));
            }
            "paragraph" => {
                body.push_str(&format!("<p>{}</p>\n", escape_html(&block.text)));
            }
            "quote" => {
                body.push_str(&format!(
                    "<blockquote>{}</blockquote>\n",
                    escape_html(&block.text)
                ));
            }
            "code" => {
                body.push_str(&format!(
                    "<pre><code>{}</code></pre>\n",
                    escape_html(&block.text)
                ));
            }
            "bulleted-list" => body.push_str(&render_list(&block.items, "ul")),
            "numbered-list" => body.push_str(&render_list(&block.items, "ol")),
            "checklist" => {
                body.push_str("<ul class=\"checklist\">\n");
                for item in &block.items {
                    let mark = if item.checked { "[x]" } else { "[ ]" };
                    body.push_str(&format!("<li>{mark} {}</li>\n", escape_html(&item.text)));
                }
                body.push_str("</ul>\n");
            }
            "image" => {
                body.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\">\n",
                    escape_html(&block.src),
                    escape_html(&block.alt)
                ));
            }
            "rule" => body.push_str("<hr>\n"),
            // style/stylesheet/script and any unknown kind are intentionally not rendered.
            _ => {}
        }
    }

    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n</head>\n<body>\n{}</body>\n</html>\n",
        escape_html(title),
        body
    )
}

/// Render list `items` inside a `<ul>`/`<ol>` (selected by `tag`), recursing into nests.
fn render_list(items: &[Item], tag: &str) -> String {
    let mut out = format!("<{tag}>\n");
    for item in items {
        out.push_str(&format!("<li>{}", escape_html(&item.text)));
        if !item.nested.is_empty() {
            out.push('\n');
            out.push_str(&render_list(&item.nested, tag));
        }
        out.push_str("</li>\n");
    }
    out.push_str(&format!("</{tag}>\n"));
    out
}
