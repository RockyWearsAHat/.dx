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
//! is sanitized by [`super::escape::escape_style`] and then **always applied** — a
//! document that dresses itself reads the same in every surface, and a `::style` a reader
//! has to know a flag to switch on is a block that silently does nothing.
//!
//! The CSS travels *inside* `.dx-doc`, not in the page `<head>`, because that container is
//! the unit every surface swaps: an editing host replaces `.dx-doc` on each save, and a
//! stylesheet left behind in a `<head>` the host never re-reads is a document that loses its
//! own dress the moment anyone touches it. One position, one code path, page and fragment
//! alike.

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

    page_shell(&pick_title(document, options), options.theme, &body)
}

/// Wrap a rendered body in the standalone page: doctype, charset, theme, stylesheet.
///
/// One shell for every full page this module produces, so a document photographed one
/// block at a time is dressed by exactly the bytes the whole page carries.
fn page_shell(title: &str, theme: Theme, body: &str) -> String {
    let theme_attr = theme
        .attribute()
        .map(|value| format!(" data-theme=\"{value}\""))
        .unwrap_or_default();

    format!(
        "<!doctype html>\n<html lang=\"en\"{theme_attr}>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape_html(title),
        stylesheet(),
    )
}

/// Size limits for a [`block_page`] capture, in CSS pixels.
#[derive(Debug, Clone, Copy)]
pub struct PageBounds {
    /// Column width for a block that flows with the page — everything but a board.
    pub width: u32,
    /// Longest edge a self-sized page (a board's natural viewport) may reach.
    pub max_edge: u32,
    /// Most pixels (width × height) a self-sized page may cover.
    pub max_pixels: u32,
}

/// One block, alone, on a standalone page sized for photographing it.
#[derive(Debug, Clone)]
pub struct BlockPage {
    /// The complete HTML page.
    pub html: String,
    /// The width to open the capture window at, in CSS pixels.
    pub width: u32,
    /// The page's exact height, when the block states its own size — a board's natural
    /// viewport. `None` means the height is the content's, and is the capturer's to
    /// measure.
    pub height: Option<u32>,
}

/// Render the single block called `id` as a standalone page, sized to be photographed.
///
/// A `::board` gets its **natural viewport** — the canvas at scale 1, every node exactly
/// the size its line states — shrunk uniformly only when `bounds` says the picture would
/// be too large to deliver, and never enlarged. In the page flow the same board is fitted
/// into the column, which is right for reading in context and wrong for reading the board:
/// a 1200px canvas fitted into a 680px column is 10px type drawn at 5px. Any other block
/// renders in the ordinary column at `bounds.width`, exactly as the page carries it.
///
/// A block marked `hidden` renders like any other: hidden means "lives on the board, not
/// in the flow", and photographing one node is precisely when it must show itself.
///
/// Returns `None` when no block carries `id`.
#[must_use]
pub fn block_page(
    document: &Document,
    id: &str,
    options: &HtmlOptions,
    bounds: &PageBounds,
) -> Option<BlockPage> {
    let index = document.block_index(id)?;
    let block = &document.blocks[index];
    let values = template::collect(document);
    let title = format!("{} — {id}", pick_title(document, options));

    if block.kind == "board" {
        let (nw, nh) = super::board::natural_viewport(document, block);
        let shrink = (f64::from(bounds.max_edge) / nw.max(nh))
            .min((f64::from(bounds.max_pixels) / (nw * nh)).sqrt())
            .min(1.0);
        let (vw, vh) = (nw * shrink, nh * shrink);
        let board = super::board::board_html_in(block, document, options, &values, vw, vh);
        let body = format!(
            "<div class=\"dx-doc\" style=\"max-width:none;margin:0;padding:0\">\n{}{board}\n</div>\n",
            document_css(document),
        );
        // Rounded, not ceiled: the shrink factor is a quotient, and `ceil` on its
        // floating-point remainder handed back a page one pixel past the stated bound.
        return Some(BlockPage {
            html: page_shell(&title, options.theme, &body),
            width: (vw.round() as u32).min(bounds.max_edge),
            height: Some((vh.round() as u32).min(bounds.max_edge)),
        });
    }

    let rendered = block_html(block, document, options, &values);
    let body = format!(
        "<div class=\"dx-doc\">\n{}{rendered}\n</div>\n",
        document_css(document),
    );
    Some(BlockPage {
        html: page_shell(&title, options.theme, &body),
        width: bounds.width,
        height: None,
    })
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
///
/// The document's own CSS leads, inside the container, so a host that swaps `.dx-doc` swaps
/// the dress along with the content it dresses.
fn blocks_html(document: &Document, options: &HtmlOptions, values: &Values) -> String {
    let mut out = String::from("<div class=\"dx-doc\">\n");
    out.push_str(&document_css(document));
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

/// Collect the document's own CSS into a `<style>` element, or nothing when it declares none.
///
/// Declarations are sanitized ([`escape_style`]) rather than trusted: author CSS may dress a
/// document, and may not carry a payload out of it. A block naming `media` is wrapped in the
/// query it names, which is how a document says "this dress is for print".
///
/// `@import`s lead, because CSS requires every import to precede the first rule — a
/// `::stylesheet` written below a `::style` would otherwise be dropped by the browser for
/// being in the wrong place, which reads as the import silently not working.
fn document_css(document: &Document) -> String {
    let mut imports = String::new();
    let mut rules = String::new();
    for block in &document.blocks {
        let media = block.media.trim();
        match block.kind.as_str() {
            "style" if !block.text.trim().is_empty() => {
                let css = escape_style(&block.text);
                if media.is_empty() {
                    rules.push_str(&css);
                } else {
                    rules.push_str(&format!("@media {} {{\n{css}\n}}", escape_style(media)));
                }
                rules.push('\n');
            }
            // A remote sheet is a fetch the author asked for by name, so it is honoured —
            // but only for a scheme that names a document to fetch. `javascript:` in an
            // `@import` is script, not style.
            "stylesheet" if !block.href.is_empty() && safe_import(&block.href) => {
                let query = if media.is_empty() {
                    String::new()
                } else {
                    format!(" {}", escape_style(media))
                };
                imports.push_str(&format!(
                    "@import url(\"{}\"){query};\n",
                    escape_style(&escape_html(&block.href))
                ));
            }
            _ => {}
        }
    }
    if imports.is_empty() && rules.is_empty() {
        String::new()
    } else {
        format!("<style data-dx-document-css>\n{imports}{rules}</style>\n")
    }
}

/// Whether a `::stylesheet` href names something that is a stylesheet rather than a script.
///
/// Relative paths and `http(s)` only. Everything else — `javascript:`, `data:`, a
/// scheme-relative `//host` — is refused, because an `@import` runs whatever it resolves to
/// with the document's own privileges.
fn safe_import(href: &str) -> bool {
    let flattened: String = href
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && !ch.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    if flattened.is_empty() || flattened.starts_with("//") || flattened.contains('"') {
        return false;
    }
    match flattened.split_once(':') {
        None => true,
        Some((scheme, _)) => scheme.contains('/') || scheme == "http" || scheme == "https",
    }
}

/// Render one block. Returns the empty string for blocks with nothing to show.
///
/// `document` is needed because a `nav` block is defined partly by what surrounds it — an
/// empty one lists the document's headings — and a `board` arranges the document's own
/// blocks as its nodes; no other block reads beyond itself.
pub(super) fn block_html(
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
        "image" => image_html(block, document, values),
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
        "board" => super::board::board_html(block, document, options, values),
        "view" => view_html(block, f64::from(super::board::PAGE_NODE_WIDTH), None),
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
///
/// Each mark carries `data-check`, its item's position in the block. That is the renderer
/// stating a fact — the same fact `edit::toggle_check` takes as an argument — so a surface
/// can turn the box a reader clicked into the edit that ticks it without reading the source
/// or counting anything itself. It is a statement, not a control: a render nobody can click
/// carries the attribute and no affordance whatsoever, exactly like `dx-runnable`.
fn checklist_html(block: &Block, values: &Values) -> String {
    let mut out = format!("<ul{}>\n", attributes(block, &["dx-checklist"]));
    for (position, item) in block.items.iter().enumerate() {
        let text = inline_html(&template::interpolate(&item.text, values));
        let (mark, class) = if item.checked {
            ("[x]", " class=\"dx-done\"")
        } else {
            ("[ ]", "")
        };
        out.push_str(&format!(
            "<li><span class=\"dx-mark\" data-check=\"{position}\">{mark}</span>\
             <span{class}>{text}</span></li>\n"
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

    // `dx-runnable` marks the block an editing surface may offer to run. The class is the
    // renderer's statement of fact — the block carries `run` — so no surface has to re-read
    // the source to know which blocks the reader can execute.
    let mut classes = vec!["dx-code"];
    if block.run {
        classes.push("dx-runnable");
    }

    if !collapsed {
        return format!(
            "<div{} data-label=\"{label}\">\n{listing}\n</div>",
            attributes(block, &classes),
        );
    }

    classes.push("dx-code-folded");
    format!(
        "<details{}>\n<summary>{label}</summary>\n{listing}\n</details>",
        attributes(block, &classes),
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
///
/// An image carrying `for=` claims a runnable block's run produced its file. The claim is
/// checked here, from nothing but the document: while that block's recorded `::output`
/// says `ok`, the picture is vouched for and shown plain; a missing producer, a producer
/// that has not run, or a failed run is called out on the figure itself — the reader must
/// never take stale pixels as proof two blocks after a red verdict.
fn image_html(block: &Block, document: &Document, values: &Values) -> String {
    if block.src.is_empty() {
        return String::new();
    }
    let alt = template::interpolate(&block.alt, values);
    let caption = if alt.is_empty() {
        String::new()
    } else {
        format!("\n<figcaption>{}</figcaption>", inline_html(&alt))
    };
    let (classes, note) = match image_doubt(block, document) {
        Some(doubt) => (
            vec!["dx-image-doubt"],
            format!(" data-note=\"{}\"", escape_html(&doubt)),
        ),
        None => (Vec::new(), String::new()),
    };
    format!(
        "<figure{}{note}>\n<img src=\"{}\" alt=\"{}\">{caption}\n</figure>",
        attributes(block, &classes),
        escape_html(&block.src),
        escape_html(&alt)
    )
}

/// Why an image's `for=` claim cannot be vouched for, if it cannot — `None` means the
/// producing block's recorded output says `ok` and the picture stands proven.
fn image_doubt(block: &Block, document: &Document) -> Option<String> {
    if block.for_block.is_empty() {
        return None;
    }
    let producer = &block.for_block;
    if !document.blocks.iter().any(|other| other.id == *producer) {
        return Some(format!("{producer} is not in this document"));
    }
    let Some(output) = document
        .blocks
        .iter()
        .find(|other| other.kind == "output" && other.for_block == *producer)
    else {
        return Some(format!("{producer} has not run"));
    };
    if output.status == "error" || output.exit != 0 {
        return Some(format!("{producer} failed — picture may be stale"));
    }
    None
}

/// The viewport width a `::view` frames its page at when the block states none.
pub(crate) const VIEW_DEFAULT_WIDTH: u32 = 1180;
/// The viewport height a `::view` frames when neither the block nor a board box states one.
pub(crate) const VIEW_DEFAULT_HEIGHT: u32 = 760;

/// Render a `::view`: the referenced page itself, framed, at its stated viewport.
///
/// The page goes into an `<iframe sandbox="">` — an **opaque origin with nothing
/// allowed**, which is the boundary that lets a whole coded page render with its own
/// stylesheet, `<body>` and all, where `render::escape` could only mangle it: nothing in
/// the frame can run script, submit a form, navigate this page, or read its origin, so
/// showing a page is still a read that executes nothing. The frame is laid out at the
/// block's stated `width`/`height` and scaled *uniformly* into the space it is shown in —
/// `target_w` (the page column, or a board node's stated inner box) and, when the box
/// states one, `target_h`. In the page flow a frame narrower than the column keeps its
/// natural size; in a stated box it fills the box, because the box is the box.
///
/// A view with a `src` and no hydrated body renders the resolver's sentence — never an
/// empty frame — and a view with neither renders nothing.
pub(super) fn view_html(block: &Block, target_w: f64, target_h: Option<f64>) -> String {
    if block.text.is_empty() {
        if block.src.is_empty() {
            return String::new();
        }
        return format!(
            "<div{}><p>{} could not be shown here — the view is the page's current \
             render, and the file was not found inside this document's folder.</p></div>",
            attributes(block, &["dx-view", "dx-view-missing"]),
            escape_html(&block.src)
        );
    }
    let frame_w = f64::from(if block.width > 0 {
        block.width
    } else {
        VIEW_DEFAULT_WIDTH
    });
    let frame_h = f64::from(if block.height > 0 {
        block.height
    } else {
        VIEW_DEFAULT_HEIGHT
    });
    let scale = match target_h {
        Some(_) => target_w / frame_w,
        None => (target_w / frame_w).min(1.0),
    };
    let frame_h = target_h.map_or(frame_h, |box_h| box_h / scale);
    let title = if block.src.is_empty() {
        block.id.clone()
    } else {
        block.src.clone()
    };
    format!(
        "<div{} style=\"width:{}px;height:{}px\">\n\
         <iframe sandbox=\"\" title=\"{}\" \
         style=\"width:{}px;height:{}px;transform:scale({})\" \
         srcdoc=\"{}\"></iframe>\n</div>",
        attributes(block, &["dx-view"]),
        round(frame_w * scale),
        round(frame_h * scale),
        escape_html(&title),
        round(frame_w),
        round(frame_h),
        trim_number(scale),
        escape_html(&block.text)
    )
}

/// A CSS pixel count: whole numbers stay whole, fractions keep two places.
fn round(value: f64) -> String {
    trim_number((value * 100.0).round() / 100.0)
}

/// A number without trailing zeros, so `1` is `1` and a scale stays short.
fn trim_number(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// Build the shared `id`/`data-block-id`/`class` attribute string for a block.
pub(super) fn attributes(block: &Block, base_classes: &[&str]) -> String {
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
    fn a_view_is_a_sandboxed_frame_scaled_into_the_column() {
        // A hydrated view frames the page in an iframe whose sandbox allows *nothing* —
        // that boundary, not the escape allow-list, is what lets a whole coded page show.
        let mut document = parse("::view id=shipped src=site/index.html width=1360\n::end\n");
        document.blocks[0].text = "<body class=\"x\">\"Hi\"</body>".to_string();
        let rendered = block_html(
            &document.blocks[0],
            &document,
            &HtmlOptions::default(),
            &Values::default(),
        );
        assert!(rendered.contains("sandbox=\"\""), "{rendered}");
        assert!(
            rendered
                .contains("srcdoc=\"&lt;body class=&quot;x&quot;&gt;&quot;Hi&quot;&lt;/body&gt;\""),
            "the page rides escaped inside the attribute: {rendered}"
        );
        // 1360 wide framed into the 680 column: half scale, and the frame keeps its own
        // stated viewport while the wrapper takes the scaled size.
        assert!(
            rendered.contains("style=\"width:680px;height:380px\""),
            "{rendered}"
        );
        assert!(rendered.contains("width:1360px;height:760px;transform:scale(0.5)"));
    }

    #[test]
    fn a_narrow_view_keeps_its_natural_size_in_the_flow() {
        let mut document =
            parse("::view id=phone src=site/index.html width=340 height=600\n::end\n");
        document.blocks[0].text = "<body>Hi</body>".to_string();
        let rendered = block_html(
            &document.blocks[0],
            &document,
            &HtmlOptions::default(),
            &Values::default(),
        );
        // 340 is narrower than the column: no upscaling — a phone view stays phone-sized.
        assert!(
            rendered.contains("style=\"width:340px;height:600px\""),
            "{rendered}"
        );
        assert!(rendered.contains("transform:scale(1)"), "{rendered}");
    }

    #[test]
    fn an_unresolved_view_is_a_sentence_never_an_empty_frame() {
        let rendered = fragment("::view id=shipped src=site/gone.html\n::end\n");
        assert!(rendered.contains("site/gone.html"), "{rendered}");
        assert!(rendered.contains("could not be shown"), "{rendered}");
        assert!(!rendered.contains("<iframe"), "{rendered}");
    }

    #[test]
    fn a_views_page_is_not_template_interpolated() {
        // A coded page is full of braces that are its own; the document's placeholder
        // values must not rewrite it.
        let mut document = parse(
            "::script id=vars type=application/json\n{\"phase\":\"authoring\"}\n::end\n\n\
             ::view id=v src=site/index.html\n::end\n",
        );
        let index = document.block_index("v").unwrap();
        document.blocks[index].text = "<style>p {{phase}: x}</style>".to_string();
        let rendered = block_html(
            &document.blocks[index],
            &document,
            &HtmlOptions::default(),
            &Values::default(),
        );
        assert!(rendered.contains("{{phase}"), "{rendered}");
    }

    #[test]
    fn a_view_node_fills_its_stated_box_edge_to_edge() {
        let mut document = parse(
            "::view id=screen src=site/index.html width=1180 height=760 hidden\n::end\n\n\
             ::board id=plan\n- screen x=0 y=0 w=592 h=402\n::end\n",
        );
        let index = document.block_index("screen").unwrap();
        document.blocks[index].text = "<body>Hi</body>".to_string();
        let rendered = fragment_of(&document);
        // The node drops its padding, and the frame scales into the stated box minus the
        // hairline: (592-2)/1180 = 0.5.
        assert!(
            rendered.contains("dx-board-node dx-board-node-view"),
            "{rendered}"
        );
        assert!(
            rendered.contains("width:1180px;height:800px;transform:scale(0.5)"),
            "{rendered}"
        );
    }

    fn fragment_of(document: &crate::model::Document) -> String {
        html(
            document,
            &HtmlOptions {
                fragment: true,
                include_hidden: false,
                ..HtmlOptions::default()
            },
        )
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
    fn runnable_code_is_classed_for_the_editing_surface() {
        let out = fragment("::code id=c lang=bash run\necho hi\n::end\n");
        assert!(out.contains("dx-runnable"), "{out}");

        // A block that does not run must not be offered as one to run.
        let inert = fragment("::code id=c lang=bash\necho hi\n::end\n");
        assert!(!inert.contains("dx-runnable"), "{inert}");
    }

    /// A board arranges the document's own blocks: hidden or not, each node renders its
    /// block through the page's per-block renderer, inside a clipping viewport.
    #[test]
    fn a_board_renders_its_nodes_from_the_documents_own_blocks() {
        const SOURCE: &str = "::board id=plan height=520\n- ideas x=40 y=40 w=280 h=160\n\
- steps x=380 y=60 w=300 h=200 to=ideas\n::end\n\n\
::paragraph id=ideas hidden\nCollect the rough ideas here.\n::end\n\n\
::checklist id=steps hidden\n[x] sketch\n[ ] build\n::end\n";
        let out = fragment(SOURCE);
        assert!(out.contains("class=\"dx-board\""), "{out}");
        assert!(out.contains("style=\"height:520px\""), "{out}");
        assert!(out.contains("dx-board-canvas"), "{out}");
        // A node is drawn as the box its line states — every one of the four numbers, so
        // the rectangle the engine did its geometry against is the one a browser lays out —
        // and its block renders as the page would draw it: the checklist is a checklist,
        // not quoted source.
        assert!(
            out.contains(
                "data-node-id=\"ideas\" style=\"left:40px;top:40px;width:280px;height:160px\""
            ),
            "{out}"
        );
        assert!(out.contains("Collect the rough ideas here."), "{out}");
        assert!(out.contains("dx-checklist"), "{out}");
        // One edge, from `steps` back to `ideas`.
        assert!(
            out.contains("data-from=\"steps\" data-to=\"ideas\""),
            "{out}"
        );
    }

    /// A custom-sized node is drawn at exactly the box its line states — numbers that are
    /// nobody's default, so this cannot pass by the defaults being applied — and the canvas
    /// it sits on is scaled uniformly, never per-node: the stated proportions are the drawn
    /// proportions on every surface.
    #[test]
    fn a_custom_sized_node_is_drawn_at_exactly_its_stated_box() {
        const SOURCE: &str = "::board id=plan\n- odd x=-17 y=3 w=333 h=127\n::end\n\n\
::paragraph id=odd hidden\nAn oddly sized node.\n::end\n";
        let out = fragment(SOURCE);
        assert!(
            out.contains(
                "data-node-id=\"odd\" style=\"left:-17px;top:3px;width:333px;height:127px\""
            ),
            "{out}"
        );
        // The only sizing above the node is the canvas's own uniform fit.
        let canvas = out.split("dx-board-canvas").nth(1).expect("a canvas");
        assert!(canvas.contains("scale("), "{canvas}");
    }

    const TWO_NODE_BOARD: &str = "::board id=plan height=400\n- a x=0 y=0 w=400 h=200\n\
- b x=800 y=500 w=400 h=200\n::end\n\n\
::paragraph id=a hidden\nFirst node.\n::end\n\n\
::paragraph id=b hidden\nSecond node.\n::end\n\n\
::paragraph id=aside\nIn the flow.\n::end\n";

    fn wide_bounds() -> PageBounds {
        PageBounds {
            width: 860,
            max_edge: 4000,
            max_pixels: 16_000_000,
        }
    }

    /// A board photographed alone arrives at its natural size: the canvas at scale 1,
    /// every node exactly the box its line states. In the flow the same board is fitted
    /// into the column — right for context, unreadable as a picture.
    #[test]
    fn a_board_page_is_the_canvas_at_scale_one() {
        let document = parse(TWO_NODE_BOARD);
        let page = block_page(&document, "plan", &HtmlOptions::default(), &wide_bounds())
            .expect("the board exists");
        // Nodes span 0..1200 × 0..700; the fit margin is 24 on every side.
        assert_eq!((page.width, page.height), (1248, Some(748)));
        assert!(
            page.html.contains("width:1248px;height:748px"),
            "{}",
            page.html
        );
        assert!(page.html.contains("scale(1)"), "{}", page.html);
        assert!(page.html.contains("First node."), "{}", page.html);
        // One block means one block: the flow around the board stays off its page.
        assert!(!page.html.contains("In the flow."), "{}", page.html);
    }

    /// The natural size honours the caller's ceiling: shrunk uniformly, never enlarged.
    #[test]
    fn a_board_page_shrinks_to_the_stated_bounds_and_never_grows() {
        let document = parse(TWO_NODE_BOARD);
        let small = PageBounds {
            max_edge: 624,
            ..wide_bounds()
        };
        let page = block_page(&document, "plan", &HtmlOptions::default(), &small)
            .expect("the board exists");
        assert_eq!(page.width, 624);
        assert!(!page.html.contains("scale(1)"), "{}", page.html);

        let pixel_capped = PageBounds {
            max_pixels: 100_000,
            ..wide_bounds()
        };
        let capped = block_page(&document, "plan", &HtmlOptions::default(), &pixel_capped)
            .expect("the board exists");
        let area = capped.width * capped.height.expect("a board states its height");
        assert!(
            area <= 100_000 + capped.width,
            "{area} pixels passes the cap"
        );
    }

    /// Any other block — a board node's hidden block included — photographs in the
    /// ordinary column, exactly as the page carries it.
    #[test]
    fn a_block_page_holds_one_block_even_a_hidden_one() {
        let document = parse(TWO_NODE_BOARD);
        let page = block_page(&document, "a", &HtmlOptions::default(), &wide_bounds())
            .expect("hidden blocks photograph too");
        assert_eq!((page.width, page.height), (860, None));
        assert!(page.html.starts_with("<!doctype html>"), "{}", page.html);
        assert!(page.html.contains("First node."), "{}", page.html);
        assert!(!page.html.contains("Second node."), "{}", page.html);

        assert!(
            block_page(&document, "ghost", &HtmlOptions::default(), &wide_bounds()).is_none(),
            "a missing block is a None, not an empty page"
        );
    }

    /// A node holding markup renders it live — sanitized HTML with its classes and inline
    /// style kept — and the document's own `::style` travels in the same fragment, so CSS
    /// written for a wireframe dresses it inside the node exactly as it dresses the page.
    #[test]
    fn a_node_renders_html_with_its_classes_and_the_documents_css_dresses_it() {
        const SOURCE: &str = "::style id=dress\n.card { border: 1px solid; }\n::end\n\n\
::board id=plan\n- mock x=0 y=0 w=280 h=200\n::end\n\n\
::html id=mock hidden\n<div class=\"card\" style=\"text-align:center\"><b>Hi</b></div>\n::end\n";
        let out = fragment(SOURCE);
        let node = out.find("data-node-id=\"mock\"").expect("the node");
        let markup = out
            .find("<div class=\"card\" style=\"text-align:center\"><b>Hi</b></div>")
            .expect("live markup, sanitized but intact");
        assert!(node < markup, "the markup rendered outside its node: {out}");
        assert!(out.contains(".card { border: 1px solid; }"), "{out}");
    }

    #[test]
    fn a_hidden_node_block_stays_out_of_the_page_flow() {
        let out = fragment(
            "::board id=plan\n- note x=0 y=0\n::end\n\n::paragraph id=note hidden\nOnly on the board.\n::end\n",
        );
        // The paragraph appears exactly once: inside the board, not again beneath it.
        assert_eq!(out.matches("Only on the board.").count(), 1, "{out}");
        let node_at = out.find("data-node-id=\"note\"").expect("the node");
        let text_at = out.find("Only on the board.").expect("the text");
        assert!(
            node_at < text_at,
            "the text rendered outside its node: {out}"
        );
    }

    /// The static fit shows the whole arrangement — on both axes, which is the difference
    /// between a plan a reader can see and one whose lower half is below the fold.
    #[test]
    fn a_board_is_fitted_on_both_axes_so_all_of_it_shows() {
        let scale_of = |out: &str| -> f64 {
            out.split("scale(")
                .nth(1)
                .and_then(|rest| rest.split(')').next())
                .expect("a static scale")
                .parse()
                .expect("a number")
        };

        let wide = fragment("::board id=plan\n- a x=0 y=0 w=400\n- b x=1200 y=0 w=400\n::end\n");
        assert!(
            scale_of(&wide) < 1.0,
            "1648px of nodes did not scale into the column: {wide}"
        );

        // The axis that used to be ignored: a column of nodes taller than the viewport.
        let tall = fragment("::board id=plan\n- a x=0 y=0\n- b x=0 y=1400\n::end\n");
        assert!(
            scale_of(&tall) < 0.4,
            "a 1450px column did not scale into a 480px viewport: {tall}"
        );

        // And a board with room to spare uses it rather than huddling in the middle.
        let small = fragment("::board id=plan\n- a x=0 y=0 w=200\n::end\n");
        assert!(scale_of(&small) > 1.0, "{small}");
    }

    #[test]
    fn a_dangling_node_says_so_and_a_board_cannot_hold_a_board() {
        let out = fragment(
            "::board id=plan\n- ghost x=0 y=0\n- inner x=300 y=0\n::end\n\n::board id=inner hidden\n\n::end\n",
        );
        assert!(out.contains("no block named `ghost`"), "{out}");
        assert!(out.contains("a board cannot hold a board"), "{out}");
    }

    #[test]
    fn a_board_alone_renders_exactly_as_the_page_carries_it() {
        const SOURCE: &str =
            "::board id=plan\n- note x=10 y=10\n::end\n\n::paragraph id=note hidden\nx\n::end\n";
        let document = parse(SOURCE);
        let page = fragment(SOURCE);
        let alone = block(&document, "plan", &HtmlOptions::default()).expect("the board");
        assert!(page.contains(&alone), "{alone}\n\nnot found in\n{page}");
    }

    #[test]
    fn a_document_dresses_itself_with_no_flag_to_forget() {
        let source = "::style id=s\np { color: red }\n::end\n\n::paragraph id=p\nx\n::end\n";
        let page = html(&parse(source), &HtmlOptions::default());
        assert!(page.contains("color: red"), "{page}");
        assert!(page.contains("data-dx-document-css"), "{page}");
    }

    #[test]
    fn a_fragment_carries_the_document_css_a_page_carries() {
        // The regression this pins: the CSS used to live in the page `<head>`, so every
        // fragment render — which is what an editing host swaps in on each save — dropped
        // the document's own dress and the page went bare mid-sentence.
        let source = "::style id=s\np { color: red }\n::end\n\n::paragraph id=p\nx\n::end\n";
        let piece = fragment(source);
        assert!(piece.contains("color: red"), "{piece}");
        assert!(
            piece.find("data-dx-document-css") < piece.find("<p"),
            "the dress leads the content it dresses: {piece}"
        );
    }

    #[test]
    fn a_document_with_no_css_of_its_own_grows_no_empty_style_element() {
        let bare = fragment("::paragraph id=p\nx\n::end\n");
        assert!(!bare.contains("data-dx-document-css"), "{bare}");
    }

    #[test]
    fn document_css_may_dress_a_page_but_not_fetch_from_it() {
        let source = "::style id=s\n\
                      a { background: url(https://tracker.example/beacon.png) }\n\
                      b { background: url(data:image/png;base64,AAAA) }\n\
                      @import url(\"https://tracker.example/x.css\");\n\
                      c { width: expression(alert(1)) }\n\
                      ::end\n";
        let page = fragment(source);
        // The beacon is defused, and the self-contained artwork beside it still works.
        assert!(!page.contains("tracker.example"), "{page}");
        assert!(page.contains("url(data:image/png;base64,AAAA)"), "{page}");
        assert!(!page.contains("@import url"), "{page}");
        assert!(!page.contains("expression("), "{page}");
        // Neutralized, not deleted: the rules around a bad one still parse.
        assert!(page.contains("a { background: url() }"), "{page}");
    }

    #[test]
    fn a_stylesheet_block_imports_a_sheet_and_refuses_a_script() {
        let sheet = fragment("::stylesheet id=a href=./print.css media=print\n::end\n");
        assert!(
            sheet.contains("@import url(\"./print.css\") print;"),
            "{sheet}"
        );

        let script = fragment("::stylesheet id=a href=javascript:alert(1)\n::end\n");
        assert!(!script.contains("javascript:"), "{script}");
    }

    #[test]
    fn imports_lead_the_rules_so_a_browser_does_not_drop_them() {
        // CSS requires every `@import` to precede the first rule. A `::stylesheet` written
        // below a `::style` in the document must still end up above it in the output.
        let page = fragment(
            "::style id=s\np { color: red }\n::end\n\n::stylesheet id=a href=./x.css\n::end\n",
        );
        assert!(page.find("@import") < page.find("color: red"), "{page}");
    }

    #[test]
    fn a_style_block_naming_media_is_wrapped_in_the_query_it_names() {
        let page = fragment("::style id=s media=print\np { color: red }\n::end\n");
        assert!(page.contains("@media print {"), "{page}");
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
    fn an_image_vouched_for_by_a_green_run_renders_plain() {
        let out = fragment(
            "::code id=frames lang=sh run\nmake frames\n::end\n\n\
             ::output id=frames-output for=frames status=ok\ndone\n::end\n\n\
             ::image id=shot src=frames/one.png for=frames\n::end\n",
        );
        assert!(!out.contains("dx-image-doubt"), "{out}");
        assert!(!out.contains("data-note=\"frames"), "{out}");
    }

    #[test]
    fn an_image_whose_producer_failed_is_called_out_on_the_figure() {
        let out = fragment(
            "::code id=frames lang=sh run\nmake frames\n::end\n\n\
             ::output id=frames-output for=frames status=error exit=1\nboom\n::end\n\n\
             ::image id=shot src=frames/one.png for=frames\n::end\n",
        );
        assert!(out.contains("dx-image-doubt"), "{out}");
        assert!(
            out.contains("data-note=\"frames failed — picture may be stale\""),
            "{out}"
        );
    }

    #[test]
    fn an_image_whose_producer_never_ran_or_is_missing_is_called_out() {
        let unrun = fragment(
            "::code id=frames lang=sh run\nmake frames\n::end\n\n\
             ::image id=shot src=frames/one.png for=frames\n::end\n",
        );
        assert!(
            unrun.contains("data-note=\"frames has not run\""),
            "{unrun}"
        );

        let missing = fragment("::image id=shot src=frames/one.png for=ghost\n::end\n");
        assert!(
            missing.contains("data-note=\"ghost is not in this document\""),
            "{missing}"
        );
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

    /// Every box says which item it is, counting from zero — the argument `toggle_check`
    /// takes. A surface that has to count the boxes itself is a surface that can miscount.
    #[test]
    fn every_checklist_box_names_its_own_position() {
        let checks = fragment("::checklist id=c\n[x] done\n[ ] todo\n[ ] later\n::end\n");
        for position in 0..3 {
            assert!(
                checks.contains(&format!("data-check=\"{position}\"")),
                "item {position} has no position on its mark: {checks}"
            );
        }
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
