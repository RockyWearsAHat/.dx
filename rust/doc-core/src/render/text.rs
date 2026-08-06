//! Document → Markdown. The text view an agent reads when it wants the words, not the page.
//!
//! Markdown is the target because every model already reads it fluently, and because the
//! result stays diffable. Structure survives the trip: heading levels, list nesting,
//! checklist state, code fences with their language, and captured run output.
//!
//! With [`TextOptions::include_ids`] each block is preceded by a `<!-- block:<id> <kind> -->`
//! comment. That is the handle an agent needs to come back and ask for one section
//! (`dx section <id>`) or edit one block, so it does not have to rewrite a whole file to
//! change a paragraph.

use super::nav::entries as nav_entries;
use super::template::{self, Values};
use crate::model::{Block, Document, Item};

/// How to render a document to Markdown.
#[derive(Debug, Clone, Default)]
pub struct TextOptions {
    /// Emit a `<!-- block:<id> <kind> -->` marker before each block.
    pub include_ids: bool,
    /// Include blocks marked `hidden`.
    pub include_hidden: bool,
    /// Include presentation-only blocks (`style`, `stylesheet`, `script`).
    pub include_presentation: bool,
}

/// Render `document` to Markdown under `options`.
#[must_use]
pub fn text(document: &Document, options: &TextOptions) -> String {
    let values = template::collect(document);
    let mut chunks: Vec<String> = Vec::new();

    for block in &document.blocks {
        if block.hidden && !options.include_hidden {
            continue;
        }
        if is_presentation(block) && !options.include_presentation {
            continue;
        }
        let body = block_text(block, document, &values);
        if body.is_empty() {
            continue;
        }
        if options.include_ids {
            chunks.push(format!(
                "<!-- block:{} {} -->\n{body}",
                block.id, block.kind
            ));
        } else {
            chunks.push(body);
        }
    }

    if chunks.is_empty() {
        return String::new();
    }
    format!("{}\n", chunks.join("\n\n"))
}

/// Whether a block exists only to shape presentation and carries no reading content.
fn is_presentation(block: &Block) -> bool {
    matches!(block.kind.as_str(), "style" | "stylesheet" | "script")
}

/// Render one block to Markdown.
///
/// `document` is what an empty `nav` block reads to list the document's own headings; no
/// other block looks past itself.
fn block_text(block: &Block, document: &Document, values: &Values) -> String {
    let body = template::interpolate(&block.text, values);

    match block.kind.as_str() {
        "heading" => format!("{} {}", "#".repeat(block.level.clamp(1, 4) as usize), body),
        "paragraph" => body,
        "quote" => body
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "bulleted-list" => list_text(&block.items, 0, false, values),
        "numbered-list" => list_text(&block.items, 0, true, values),
        "checklist" => block
            .items
            .iter()
            .map(|item| {
                let mark = if item.checked { "x" } else { " " };
                format!("- [{mark}] {}", template::interpolate(&item.text, values))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        // Nav becomes an ordinary Markdown link list — which is exactly what renders on
        // GitHub, in a diff, and in an agent's reading of the document.
        "nav" => nav_entries(block, document)
            .iter()
            .map(|entry| {
                format!(
                    "{}- [{}]({})",
                    "  ".repeat(entry.depth),
                    template::interpolate(&entry.name, values),
                    entry.target
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "code" => fence(&code_language(block), &body),
        // A view's meaning is the page it renders to — pixels, not words. Folding it to
        // one line is what keeps a guide that frames a whole site from costing every text
        // reader the site's source; the rendered routes (`dx png`, `dx_read`) show the page.
        "view" => view_text(block),
        "output" => format!(
            "{}\n{}",
            output_caption(block),
            fence(output_language(block), &body).trim_end()
        ),
        "image" => format!("![{}]({})", block.alt, block.src),
        "rule" => "---".to_string(),
        "svg" | "html" | "mermaid" | "graph" => fence(&block.kind, &body),
        "script" => fence(script_language(block), &body),
        "style" => fence("css", &body),
        "stylesheet" => format!("<!-- stylesheet: {} -->", block.href),
        _ => body,
    }
}

/// The one-line stand-in for a framed page: what it shows and where to see it rendered.
///
/// A `src=` view names its page; an inline view states its size, which is the honest
/// measure of what the fold is saving the reader.
fn view_text(block: &Block) -> String {
    if block.src.is_empty() {
        format!(
            "*View: an inline page ({} bytes), rendered in a sandboxed frame — read it \
             rendered (`dx png`, `dx_read`).*",
            block.text.len()
        )
    } else {
        format!(
            "*View: `{}` as the page it renders to — read it rendered (`dx png`, `dx_read`).*",
            block.src
        )
    }
}

/// A one-line caption saying which code block an output came from and how it ended.
fn output_caption(block: &Block) -> String {
    let source = if block.for_block.is_empty() {
        "code".to_string()
    } else {
        format!("`{}`", block.for_block)
    };
    let status = if block.status.is_empty() {
        "ok".to_string()
    } else {
        block.status.clone()
    };
    if block.exit == 0 {
        format!("Output of {source} ({status}):")
    } else {
        format!("Output of {source} ({status}, exit {}):", block.exit)
    }
}

/// The fence language for captured output: the format the code drew, or plain text.
fn output_language(block: &Block) -> &str {
    if block.format.is_empty() {
        "text"
    } else {
        &block.format
    }
}

/// The fence language for a code block, including a `run` hint when it is executable.
fn code_language(block: &Block) -> String {
    let language = if block.language.is_empty() {
        "text"
    } else {
        &block.language
    };
    if block.run {
        format!("{language} run")
    } else {
        language.to_string()
    }
}

/// The fence language for a script block, derived from its MIME type.
fn script_language(block: &Block) -> &'static str {
    match block.script_type.trim().to_ascii_lowercase().as_str() {
        "application/json" | "" => "json",
        _ => "javascript",
    }
}

/// Wrap `body` in a fence long enough to contain any backtick run inside it.
fn fence(language: &str, body: &str) -> String {
    let mut ticks = 3;
    for line in body.lines() {
        let run = line.chars().take_while(|c| *c == '`').count();
        if run >= ticks {
            ticks = run + 1;
        }
    }
    let rail = "`".repeat(ticks);
    format!("{rail}{language}\n{body}\n{rail}")
}

/// Render list items with two-space indentation per nesting level.
fn list_text(items: &[Item], depth: usize, numbered: bool, values: &Values) -> String {
    let indent = "  ".repeat(depth);
    let mut lines: Vec<String> = Vec::new();

    for (position, item) in items.iter().enumerate() {
        let marker = if numbered {
            format!("{}.", position + 1)
        } else {
            "-".to_string()
        };
        lines.push(format!(
            "{indent}{marker} {}",
            template::interpolate(&item.text, values)
        ));
        if !item.nested.is_empty() {
            lines.push(list_text(&item.nested, depth + 1, numbered, values));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse;

    fn render(source: &str) -> String {
        text(&parse(source), &TextOptions::default())
    }

    #[test]
    fn renders_structure_as_markdown() {
        let out = render(
            "::heading level=2 id=h\nTitle\n::end\n\n::paragraph id=p\nBody\n::end\n\n::bulleted-list id=l\n- a\n- b\n::end\n",
        );
        assert_eq!(out, "## Title\n\nBody\n\n- a\n- b\n");
    }

    #[test]
    fn nav_renders_as_a_markdown_link_list() {
        let out = render("::nav id=n\n- [Setup](setup.dx)\n  - api.dx#errors\n::end\n");
        assert_eq!(out, "- [Setup](setup.dx)\n  - [errors](api.dx#errors)\n");
    }

    #[test]
    fn an_empty_nav_lists_the_documents_own_headings() {
        let out = render(
            "::heading level=1 id=title\nGuide\n::end\n\n::nav id=n\n::end\n\n\
             ::heading level=2 id=setup\nSetup\n::end\n",
        );
        assert_eq!(out, "# Guide\n\n- [Setup](#setup)\n\n## Setup\n");
    }

    #[test]
    fn code_fences_carry_the_language_and_run_hint() {
        assert!(render("::code id=c lang=python\nx=1\n::end\n").contains("```python\nx=1\n```"));
        assert!(render("::code id=c lang=python run\nx=1\n::end\n").contains("```python run"));
    }

    #[test]
    fn fences_grow_to_contain_inner_backticks() {
        let out = render("::code id=c lang=md\n```js\nx\n```\n::end\n");
        assert!(out.contains("````md"));
    }

    #[test]
    fn a_view_folds_to_one_line_naming_its_page() {
        let mut document = parse("::view id=v src=site/index.html\n::end\n");
        // What hydration would have filled in: the page's whole source.
        document.blocks[0].text = "<!doctype html><html>a whole site</html>".to_string();
        let out = text(&document, &TextOptions::default());
        assert!(out.contains("`site/index.html`"), "names the page: {out}");
        assert!(!out.contains("doctype"), "never inlines the source: {out}");
    }

    #[test]
    fn an_inline_view_folds_to_its_size() {
        let out = render("::view id=v\n<p>framed</p>\n::end\n");
        assert!(out.contains("bytes"), "states what the fold saves: {out}");
        assert!(!out.contains("<p>"), "never inlines the markup: {out}");
    }

    #[test]
    fn output_is_captioned_with_its_source_block() {
        let out = render("::output id=o for=demo status=error exit=1\nboom\n::end\n");
        assert!(out.contains("Output of `demo` (error, exit 1):"));
        assert!(out.contains("```text\nboom\n```"));
    }

    #[test]
    fn presentation_blocks_are_omitted_by_default() {
        let source = "::style id=s\np{}\n::end\n\n::paragraph id=p\nx\n::end\n";
        assert_eq!(render(source), "x\n");
        let full = text(
            &parse(source),
            &TextOptions {
                include_presentation: true,
                ..TextOptions::default()
            },
        );
        assert!(full.contains("```css"));
    }

    #[test]
    fn ids_can_be_emitted_for_addressing() {
        let out = text(
            &parse("::paragraph id=intro\nHello\n::end\n"),
            &TextOptions {
                include_ids: true,
                ..TextOptions::default()
            },
        );
        assert_eq!(out, "<!-- block:intro paragraph -->\nHello\n");
    }

    #[test]
    fn checklist_state_survives() {
        let out = render("::checklist id=c\n[x] done\n[ ] todo\n::end\n");
        assert_eq!(out, "- [x] done\n- [ ] todo\n");
    }
}
