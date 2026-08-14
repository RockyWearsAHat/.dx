//! Rendering — one document, three views, identical everywhere.
//!
//! A `.dx` file is one canonical source. Which view you get depends on who is looking:
//!
//! | View | Entry point | For |
//! |------|-------------|-----|
//! | Page | [`html`] | Humans in an editor or browser, and screenshots for agents |
//! | Markdown | [`text`] | Agents that want the words, and diffs |
//! | Structure | [`outline`] / [`section`] | Finding and fetching *part* of a document |
//!
//! Every view is produced by this crate, which compiles to both native and `wasm32`. That
//! is the point: the VS Code webview, the CLI, the MCP server, and the screenshotter all
//! render through the same code, so a document cannot look like one thing to a person and
//! another thing to an agent.
//!
//! Rendering is a pure function of the document and the options — no clock, no filesystem,
//! no network — so the same input always yields the same bytes.

pub(crate) mod board;
pub(crate) mod escape;
mod field;
mod html;
mod inline;
mod nav;
mod outline;
mod template;
mod text;
mod theme;

pub use board::{
    board_edges, drag_path, edge_layout, side_named, DragEnd, EdgeLayout, EdgeSpec, Label,
    Measured, Rect, Side,
};
pub use escape::{escape_html, sanitize_markup};
pub use field::field_html;
pub use html::{block, block_page, html, BlockPage, HtmlOptions, PageBounds};
pub use nav::{entries as nav_entries, NavEntry};
pub use outline::{outline, section, OutlineEntry};
pub use text::{text, TextOptions};
pub use theme::{stylesheet, Theme};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::parse;

    const SAMPLE: &str = "::heading level=1 id=title\nGuide\n::end\n\n\
::heading level=2 id=setup\nSetup\n::end\n\n\
::paragraph id=setup-body\nRun the installer.\n::end\n\n\
::heading level=2 id=usage\nUsage\n::end\n";

    #[test]
    fn a_section_renders_through_every_view() {
        let document = parse(SAMPLE);
        let slice = section(&document, "setup").expect("setup section");

        let markdown = text(&slice, &TextOptions::default());
        assert_eq!(markdown, "## Setup\n\nRun the installer.\n");

        let page = html(&slice, &HtmlOptions::default());
        assert!(page.contains("Run the installer."));
        assert!(!page.contains("Usage"));

        let rows = outline(&slice);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn rendering_is_deterministic() {
        let document = parse(SAMPLE);
        let first = html(&document, &HtmlOptions::default());
        let second = html(&document, &HtmlOptions::default());
        assert_eq!(first, second);
    }
}
