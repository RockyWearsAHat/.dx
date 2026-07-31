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
use super::nav::{entries as nav_entries, NavEntry};
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
    /// Start every code block closed, so a document opens as what it *says* rather than
    /// how it was written. On by default.
    ///
    /// The reveal is the block's own label, and it is CSS only — no page ever needs a script
    /// to open one, which is what lets DX.app keep JavaScript off entirely.
    ///
    /// A caller rendering to something nobody can click — a PNG, a printed sheet, the images
    /// `dx_read` hands an agent — sets this to `false`. There is no "to start" in a static
    /// image, and a reader who cannot expand a block must not be shown a label where the
    /// code should be.
    pub collapse_code: bool,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            theme: Theme::Auto,
            fragment: false,
            title: String::new(),
            include_hidden: false,
            document_css: false,
            collapse_code: true,
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

/// Render the single block called `id`, exactly as the whole page would carry it.
///
/// This is what lets a surface change one block without redrawing the document around it —
/// and, while a reader is writing, show what they are writing as it will be read. It is the
/// same [`block_html`] the page calls, with the same options and the same
/// `{{placeholder}}` values, so a block rendered on its own and the same block rendered in
/// its page are byte-identical. A second implementation of "what does this block look like"
/// is a page that disagrees with itself the moment anyone touches it.
///
/// Returns `None` when no block carries `id`, and `Some("")` for a block with nothing to
/// show (`::style`, `::script`) — the caller needs to tell "there is no such block" from
/// "that block draws nothing".
#[must_use]
pub fn block(document: &Document, id: &str, options: &HtmlOptions) -> Option<String> {
    let index = document.block_index(id)?;
    let values = template::collect(document);
    Some(block_html(
        &document.blocks[index],
        document,
        options,
        &values,
    ))
}

/// Render just the document container and its blocks.
fn blocks_html(document: &Document, options: &HtmlOptions, values: &Values) -> String {
    let mut out = String::from("<div class=\"dx-doc\">\n");
    for block in &document.blocks {
        if block.hidden && !options.include_hidden {
            continue;
        }
        let rendered = block_html(block, document, options, values);
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
///
/// `document` is needed because a `nav` block is defined partly by what surrounds it — an
/// empty one lists the document's headings — and no other block reads beyond itself.
fn block_html(
    block: &Block,
    document: &Document,
    options: &HtmlOptions,
    values: &Values,
) -> String {
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
        "nav" => nav_html(block, document, values),
        "code" => code_html(block, &text, options.collapse_code),
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

/// Render a nav block: a `<nav>` holding one list of links, nested by entry depth.
///
/// No panel, no box, no highlight — navigation is a list of names on the same sheet as
/// everything else, and the author's `class` is the hook for anything more. A nav that
/// resolves to nothing renders nothing rather than an empty list.
fn nav_html(block: &Block, document: &Document, values: &Values) -> String {
    let entries = nav_entries(block, document);
    if entries.is_empty() {
        return String::new();
    }
    format!(
        "<nav{}>\n{}</nav>",
        attributes(block, &["dx-nav"]),
        nav_list_html(&entries, 0, values)
    )
}

/// Render the entries at `depth` as one `<ul>`, recursing where the next entry is deeper.
///
/// Complexity: `O(n)` — each entry is emitted once, however deep the nesting goes.
fn nav_list_html(entries: &[NavEntry], depth: usize, values: &Values) -> String {
    let mut out = String::from("<ul>\n");
    let mut index = 0;

    while index < entries.len() {
        let entry = &entries[index];
        if entry.depth < depth {
            break;
        }
        // A deeper run belongs to the item just written, so it is emitted inside it.
        if entry.depth > depth {
            let run_end = entries[index..]
                .iter()
                .position(|next| next.depth < entry.depth)
                .map_or(entries.len(), |offset| index + offset);
            let nested = nav_list_html(&entries[index..run_end], entry.depth, values);
            // Reopen the last item to hold its children, keeping the list well-formed.
            if out.ends_with("</li>\n") {
                out.truncate(out.len() - "</li>\n".len());
                out.push('\n');
                out.push_str(&nested);
                out.push_str("</li>\n");
            } else {
                out.push_str(&nested);
            }
            index = run_end;
            continue;
        }
        out.push_str(&format!(
            "<li><a href=\"{}\">{}</a></li>\n",
            escape_html(&entry.target),
            inline_html(&template::interpolate(&entry.name, values))
        ));
        index += 1;
    }

    out.push_str("</ul>\n");
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

/// Render a code block: what it is, and — once asked for — the verbatim source.
///
/// A document is what it says, not how it was made, so the source starts folded away behind
/// one faint line naming it: its language, whether it runs, what it needs. What the code
/// *produced* is a separate `output` block and stays on the page; the reader sees the result
/// and can ask for the recipe.
///
/// There is no header bar and no badges. Folded or open, the label is the same faint pencil
/// note in the same margin — a `summary` is a line of type, not a control with a surface.
///
/// The fold is `details`/`summary`, so opening one is the browser's own behavior and needs no
/// script on a page that is not allowed to carry one. `collapsed` is false for renders nobody
/// can click, where the code is simply shown.
fn code_html(block: &Block, text: &str, collapsed: bool) -> String {
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
    let label = escape_html(&parts.join(" · "));

    let listing = format!(
        "<pre><code class=\"language-{}\">{}</code></pre>",
        escape_html(language),
        hanging_lines(text)
    );

    if !collapsed {
        return format!(
            "<div{} data-label=\"{label}\">\n{listing}\n</div>",
            attributes(block, &["dx-code"]),
        );
    }

    format!(
        "<details{}>\n<summary>{label}</summary>\n{listing}\n</details>",
        attributes(block, &["dx-code", "dx-code-folded"]),
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
            hanging_lines(text)
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
        hanging_lines(text)
    )
}

/// Escape preformatted text, wrapping each source line in its own element.
///
/// A long line has to wrap — a rendered document is also read as a static image, where a
/// horizontal scrollbar silently truncates code. But a continuation that restarts at column 0
/// reads as the next statement and destroys the indentation the code is structured by. The
/// stylesheet hangs each `dx-line` so a wrapped remainder sits *under* its own line.
///
/// The wrapper has to be per line: `text-indent` applies to the first line box of a block, so
/// setting it on the `pre` indents everything after the first line instead of the wraps.
fn hanging_lines(text: &str) -> String {
    // Escaping introduces no newlines, so splitting the escaped text is the same split.
    //
    // The pieces are joined with nothing between them. Each one is a block box and breaks the
    // line by itself, so keeping the original newline inside a `pre` would render a blank line
    // between every pair — the whole listing double-spaced.
    escape_html(text)
        .split('\n')
        .map(|line| format!("<span class=\"dx-line\">{line}</span>"))
        .collect()
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

    /// The property a live page rests on: rendering one block on its own and rendering the
    /// whole document produce the *same* markup for that block. A surface that redraws one
    /// block mid-sentence must not be able to draw it differently from the page around it.
    #[test]
    fn one_block_renders_exactly_as_the_page_carries_it() {
        const SOURCE: &str = "::heading level=2 id=head\nA **strong** title\n::end\n\n\
::paragraph id=intro\nSome *emphasis* and a [link](https://example.com).\n::end\n\n\
::bulleted-list id=points\n- first\n- second\n::end\n\n\
::svg id=drawing\n<svg viewBox=\"0 0 10 10\"><rect width=\"10\" height=\"10\"/></svg>\n::end\n";

        let document = parse(SOURCE);
        let page = fragment(SOURCE);
        for id in ["head", "intro", "points", "drawing"] {
            let one = block(&document, id, &HtmlOptions::default()).expect("the block");
            assert!(!one.is_empty(), "{id} rendered nothing");
            assert!(
                page.contains(&one),
                "{id} renders differently alone:\n{one}\n\nnot found in\n{page}"
            );
        }
    }

    #[test]
    fn an_unknown_block_is_none_and_an_invisible_one_is_empty() {
        let document = parse("::paragraph id=p\nText.\n::end\n\n::style id=s\np{}\n::end\n");
        assert!(block(&document, "nope", &HtmlOptions::default()).is_none());
        // A `::style` block exists and draws nothing — a caller has to tell that from
        // "there is no such block", which is why one is `Some("")` and the other `None`.
        assert_eq!(
            block(&document, "s", &HtmlOptions::default()).as_deref(),
            Some("")
        );
    }

    #[test]
    fn a_block_is_rendered_with_the_documents_placeholder_values() {
        let document = parse(
            "::script id=vars type=application/json\n{\"phase\":\"authoring\"}\n::end\n\n\
::paragraph id=line\nPhase: {{phase}}.\n::end\n",
        );
        let one = block(&document, "line", &HtmlOptions::default()).expect("the block");
        assert!(one.contains("Phase: authoring."), "{one}");
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
    fn nav_is_a_list_of_links_and_nothing_else() {
        let out = fragment("::nav id=side class=sidebar\n- [Setup](setup.dx)\n::end\n");
        assert!(out.contains("<nav id=\"side\" data-block-id=\"side\" class=\"dx-nav sidebar\">"));
        assert!(out.contains("<li><a href=\"setup.dx\">Setup</a></li>"));
        // Navigation is type on the page, not a widget: no panel, no state, no script.
        let nav = &out[out.find("<nav").expect("nav")..out.find("</nav>").expect("nav end")];
        assert!(!nav.contains("<div"));
        assert!(!nav.contains("aria-current"));
        assert!(!nav.contains("style="));
    }

    #[test]
    fn a_nested_nav_entry_renders_inside_its_parent_item() {
        let out = fragment("::nav id=n\n- a.dx\n  - b.dx\n::end\n");
        let a = out.find("a.dx").expect("parent");
        let nested_open = out.find("<ul>\n<li><a href=\"b.dx\"").expect("nested list");
        assert!(a < nested_open, "child list must open after its parent");
        assert_eq!(out.matches("</li>").count(), 2);
        assert_eq!(out.matches("<ul>").count(), 2);
    }

    #[test]
    fn a_nav_with_nothing_to_point_at_renders_nothing() {
        // An empty nav in a document with no headings must not leave an empty list behind.
        assert_eq!(
            fragment("::nav id=n\n::end\n").trim(),
            "<div class=\"dx-doc\">\n</div>"
        );
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
        // Language, runnability, and dependencies in one quiet line — no header bar and no
        // pills in the markup at all.
        assert!(
            out.contains("<summary>python · run · rich</summary>"),
            "{out}"
        );
        assert!(!out.contains("dx-badge"), "{out}");
        assert!(!out.contains("dx-code-head"), "{out}");

        // A successful run is not announced; the result simply follows the code. Each line is
        // its own element so a wrapped one hangs under itself rather than restarting at the
        // margin — see `hanging_lines`.
        assert!(
            out.contains("<pre><span class=\"dx-line\">1</span></pre>"),
            "{out}"
        );
        assert!(!out.contains("dx-output-head"), "{out}");
        assert!(!out.contains("data-note"), "success needs no note: {out}");
    }

    #[test]
    fn a_plain_code_block_still_says_what_language_it_is() {
        let out = fragment("::code id=c lang=js\nconst x = 1;\n::end\n");
        assert!(out.contains("<summary>js</summary>"), "{out}");
    }

    /// A document opens as what it says. The source is behind the label, and — this is the
    /// part that has to hold — the fold is the browser's own, so the page still carries no
    /// script for DX.app to have to allow.
    #[test]
    fn code_starts_folded_and_opening_it_needs_no_script() {
        let out = fragment("::code id=c lang=python\nsecret = 1\n::end\n");
        assert!(out.contains("<details"), "{out}");
        assert!(out.contains("dx-code-folded"), "{out}");
        assert!(
            !out.contains(" open"),
            "a fold that starts open is not a fold: {out}"
        );
        assert!(!out.to_lowercase().contains("<script"), "{out}");
        assert!(!out.contains("onclick"), "{out}");

        // Folded is not deleted: the listing is in the markup, one click away.
        assert!(out.contains("secret = 1"), "{out}");
    }

    /// What the code *produced* is the document's content and is never folded — the reader
    /// sees the result and asks for the recipe, not the other way round.
    #[test]
    fn output_stays_on_the_page_when_its_code_is_folded() {
        let out = fragment(
            "::code id=c lang=python run\nprint(1)\n::end\n\n::output id=o for=c status=ok\n1\n::end\n",
        );
        let output_at = out.find("dx-output").expect("output block");
        assert!(!out[output_at..].contains("<details"), "{out}");
    }

    /// Nobody can click a PNG, so a caller rendering one asks for the code itself.
    #[test]
    fn a_render_nobody_can_click_shows_the_code_instead_of_a_label() {
        let out = html(
            &parse("::code id=c lang=python\nprint(1)\n::end\n"),
            &HtmlOptions {
                fragment: true,
                collapse_code: false,
                ..HtmlOptions::default()
            },
        );
        assert!(!out.contains("<details"), "{out}");
        assert!(out.contains("data-label=\"python\""), "{out}");
        assert!(out.contains("print(1)"), "{out}");
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
            include_str!("../../tests/fixtures/welcome.input.dx"),
            include_str!("../../tests/fixtures/tutorial.input.dx"),
            include_str!("../../tests/fixtures/block-reference.input.dx"),
        ] {
            let page = html(&parse(source), &HtmlOptions::default());
            // A marker leaks when the parser fails to consume it and it lands in the page as
            // content of its own. The characters simply *appearing* is not that: a document
            // that explains the format writes `::end` in a sentence, and the block reference
            // draws one in a diagram. So the check is for an element that is nothing but a
            // marker, which is what an unconsumed one would produce.
            for tag in ["<p>", "<li>", "<h1>", "<h2>", "<h3>", "<h4>"] {
                assert!(
                    !page.contains(&format!("{tag}::")),
                    "raw block marker leaked into HTML as a {tag} of its own"
                );
            }
            assert!(page.contains("<div class=\"dx-doc\">"));
        }
    }
}
