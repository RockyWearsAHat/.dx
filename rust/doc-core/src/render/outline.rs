//! Outlines and section slicing — how a reader asks for *part* of a document.
//!
//! A long document should not have to be read whole. [`outline`] gives a cheap table of
//! contents: one row per block, with the id to ask for next. [`section`] then returns a
//! [`Document`] containing just that part, which every renderer already knows how to
//! handle — so text, HTML, and screenshots all support sections without extra code.

use crate::model::{Document, RUNNERS};

/// One row of a document outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Zero-based position of the block in the document.
    pub index: usize,
    /// Block kind (`heading`, `paragraph`, `code`, …).
    pub kind: String,
    /// Block id — the selector to pass to [`section`].
    pub id: String,
    /// Heading level, or `0` for non-heading blocks.
    pub level: u8,
    /// A short single-line preview of the block's content.
    pub preview: String,
    /// Character count of the block's content.
    pub chars: usize,
    /// Whether this block is executable code (`::code … run` with a known runner).
    pub runnable: bool,
}

/// Longest preview emitted for an outline row.
const PREVIEW_LIMIT: usize = 72;

/// Build a one-row-per-block outline of `document`.
#[must_use]
pub fn outline(document: &Document) -> Vec<OutlineEntry> {
    document
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| OutlineEntry {
            index,
            kind: block.kind.clone(),
            id: block.id.clone(),
            level: if block.kind == "heading" {
                block.level
            } else {
                0
            },
            preview: preview_of(&preview_content(document, index)),
            chars: block_content(document, index).chars().count(),
            runnable: is_runnable(document, index),
        })
        .collect()
}

/// Slice the part of `document` addressed by `selector`, or `None` when nothing matches.
///
/// `selector` is a block id. When it names a **heading**, the slice runs from that heading
/// to just before the next heading at the same or a shallower level — the section a reader
/// means when they point at a title. When it names any other block, the slice is that block
/// plus the `::output` that belongs to it, so asking for a code block always shows its result.
#[must_use]
pub fn section(document: &Document, selector: &str) -> Option<Document> {
    let start = find_block(document, selector)?;
    let anchor = &document.blocks[start];

    let end = if anchor.kind == "heading" {
        document.blocks[start + 1..]
            .iter()
            .position(|block| block.kind == "heading" && block.level <= anchor.level)
            .map_or(document.blocks.len(), |offset| start + 1 + offset)
    } else {
        let mut end = start + 1;
        while end < document.blocks.len()
            && document.blocks[end].kind == "output"
            && document.blocks[end].for_block == anchor.id
        {
            end += 1;
        }
        end
    };

    Some(Document {
        title: if anchor.kind == "heading" {
            anchor.text.clone()
        } else {
            document.title.clone()
        },
        summary: document.summary.clone(),
        tags: document.tags.clone(),
        meta: document.meta.clone(),
        blocks: carry_presentation(document, start, end),
    })
}

/// Collect the slice `start..end` plus the document's presentation blocks and any blocks
/// a `::board` inside the slice references.
///
/// `::script` (template values) and `::style` blocks live wherever the author put them, so
/// a slice that omitted them would render placeholders as blanks. A board's node lines name
/// sibling blocks of the same document, so a slice that dropped those would draw every node
/// as a missing-block sentence. Both are carried along, deduplicated against blocks already
/// inside the slice, and always come first; a carried node block is marked `hidden` so it
/// lives on the board, exactly as it does in the whole page, instead of also appearing in
/// the slice's own flow.
fn carry_presentation(document: &Document, start: usize, end: usize) -> Vec<crate::model::Block> {
    let referenced = board_references(&document.blocks[start..end]);
    let mut blocks: Vec<crate::model::Block> = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(index, block)| {
            (*index < start || *index >= end)
                && (matches!(block.kind.as_str(), "script" | "style" | "stylesheet")
                    || referenced.contains(&block.id.to_ascii_lowercase()))
        })
        .map(|(_, block)| {
            let mut carried = block.clone();
            if referenced.contains(&carried.id.to_ascii_lowercase()) {
                carried.hidden = true;
            }
            carried
        })
        .collect();
    blocks.extend(document.blocks[start..end].iter().cloned());
    blocks
}

/// The ids every `::board` in `slice` refers to, lowercased for the id comparison
/// [`find_block`] already performs.
fn board_references(slice: &[crate::model::Block]) -> Vec<String> {
    slice
        .iter()
        .filter(|block| block.kind == "board")
        .flat_map(|block| super::board::nodes(&block.text))
        .map(|node| node.id.to_ascii_lowercase())
        .collect()
}

/// Find the index of the block whose id matches `selector`, case-insensitively.
fn find_block(document: &Document, selector: &str) -> Option<usize> {
    let wanted = selector.trim().trim_start_matches('#').to_ascii_lowercase();
    if wanted.is_empty() {
        return None;
    }
    document
        .blocks
        .iter()
        .position(|block| block.id.to_ascii_lowercase() == wanted)
}

/// Whether the block at `index` is executable code with a runner the platform supports.
fn is_runnable(document: &Document, index: usize) -> bool {
    let block = &document.blocks[index];
    block.kind == "code"
        && block.run
        && crate::model::runner_for_language(&block.language)
            .is_some_and(|runner| RUNNERS.contains(&runner))
}

/// What a row previews: a reference block leads with the path it points at.
///
/// The map exists to tell a reader what to fetch next. A `::view src=` or `::code src=`
/// block's content is the referenced file's text, so nine views of one page would all
/// preview as the same doctype preamble — the reference is the row's identity, and the
/// content, when hydrated, trails it for flavour.
fn preview_content(document: &Document, index: usize) -> String {
    let block = &document.blocks[index];
    let content = block_content(document, index);
    if matches!(block.kind.as_str(), "code" | "view") && !block.src.is_empty() {
        if content.is_empty() {
            return block.src.clone();
        }
        return format!("{} — {content}", block.src);
    }
    content
}

/// The reading content of a block, whichever field carries it for that kind.
fn block_content(document: &Document, index: usize) -> String {
    let block = &document.blocks[index];
    match block.kind.as_str() {
        // A nav's preview is its entries as written; an empty one previews as empty
        // because what it will list depends on the document around it.
        "bulleted-list" | "numbered-list" | "checklist" | "nav" => block
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        "image" => block.alt.clone(),
        "stylesheet" => block.href.clone(),
        _ => block.text.clone(),
    }
}

/// Collapse `content` to a single line no longer than [`PREVIEW_LIMIT`] characters.
fn preview_of(content: &str) -> String {
    let flattened = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= PREVIEW_LIMIT {
        return flattened;
    }
    let head: String = flattened.chars().take(PREVIEW_LIMIT - 1).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{parse, stringify};

    const SAMPLE: &str = "::heading level=1 id=top\nTop\n::end\n\n\
::paragraph id=lead\nLead text\n::end\n\n\
::heading level=2 id=alpha\nAlpha\n::end\n\n\
::paragraph id=a-body\nAlpha body\n::end\n\n\
::code id=a-code lang=python run\nprint(1)\n::end\n\n\
::output id=a-out for=a-code status=ok\n1\n::end\n\n\
::heading level=2 id=beta\nBeta\n::end\n\n\
::paragraph id=b-body\nBeta body\n::end\n";

    #[test]
    fn outline_reports_ids_levels_and_runnability() {
        let entries = outline(&parse(SAMPLE));
        assert_eq!(entries.len(), 8);
        assert_eq!(entries[0].id, "top");
        assert_eq!(entries[0].level, 1);
        assert_eq!(entries[0].preview, "Top");
        assert!(entries[4].runnable);
        assert!(!entries[1].runnable);
    }

    #[test]
    fn a_reference_block_previews_the_path_it_points_at() {
        let source = "::view id=desk src=site/index.html#tonight width=1180\n::end\n\n\
::code id=styles src=site/site.css lang=css\n::end\n";
        let entries = outline(&parse(source));
        // Unhydrated, the reference is the whole preview — the row's identity is what
        // it points at, never an empty string.
        assert_eq!(entries[0].preview, "site/index.html#tonight");
        assert_eq!(entries[1].preview, "site/site.css");
        // Hydrated, the reference still leads and the content trails it.
        let mut hydrated = parse(source);
        hydrated.blocks[1].text = "body { color: red }".to_string();
        let entries = outline(&hydrated);
        assert_eq!(entries[1].preview, "site/site.css — body { color: red }");
    }

    #[test]
    fn outline_previews_are_single_line_and_bounded() {
        let long = format!("::paragraph id=p\n{}\n::end\n", "word ".repeat(60));
        let entries = outline(&parse(&long));
        assert!(entries[0].preview.chars().count() <= PREVIEW_LIMIT);
        assert!(entries[0].preview.ends_with('…'));
    }

    #[test]
    fn heading_section_stops_at_the_next_same_level_heading() {
        let sliced = section(&parse(SAMPLE), "alpha").expect("alpha section");
        let out = stringify(&sliced);
        assert!(out.contains("id=alpha"));
        assert!(out.contains("id=a-code"));
        assert!(out.contains("id=a-out"));
        assert!(!out.contains("id=beta"));
        assert!(!out.contains("id=lead"));
    }

    #[test]
    fn top_level_heading_section_covers_the_whole_document() {
        let sliced = section(&parse(SAMPLE), "top").expect("top section");
        assert_eq!(sliced.blocks.len(), 8);
    }

    #[test]
    fn code_section_brings_its_output_along() {
        let sliced = section(&parse(SAMPLE), "a-code").expect("code section");
        assert_eq!(sliced.blocks.len(), 2);
        assert_eq!(sliced.blocks[1].kind, "output");
    }

    #[test]
    fn selectors_accept_a_leading_hash_and_any_case() {
        assert!(section(&parse(SAMPLE), "#ALPHA").is_some());
        assert!(section(&parse(SAMPLE), "nope").is_none());
        assert!(section(&parse(SAMPLE), "  ").is_none());
    }

    /// The bug this pins: `--section <board-id>` used to drop the rest of the document
    /// before the board resolved its node lines, so every node rendered as a
    /// missing-block sentence. A board is resolved from the document it sits in — nav's
    /// own rule — so its referenced blocks ride along with the slice, hidden.
    #[test]
    fn a_board_section_carries_the_blocks_its_nodes_reference() {
        let source = "::heading level=1 id=top\nTop\n::end\n\n\
::board id=plan height=300\n- idea x=10 y=10 w=200 h=100\n- steps x=260 y=10 w=200 h=100\n::end\n\n\
::paragraph id=idea hidden\nThe idea.\n::end\n\n\
::checklist id=steps\n[ ] build\n::end\n";
        let document = parse(source);

        let sliced = section(&document, "plan").expect("board section");
        let page = crate::render::html(&sliced, &crate::render::HtmlOptions::default());
        assert!(!page.contains("no block named"), "{page}");
        assert!(page.contains("The idea."));
        assert!(page.contains("build"));
        // Carried blocks live on the board, not also in the slice's own flow.
        assert_eq!(page.matches("The idea.").count(), 1);
        // The whole document is untouched: `steps` is still a flow block there.
        assert!(!document.blocks.iter().any(|b| b.id == "steps" && b.hidden));
    }

    #[test]
    fn a_heading_section_holding_a_board_resolves_nodes_outside_the_slice() {
        let source = "::heading level=2 id=plan-sec\nPlan\n::end\n\n\
::board id=plan height=300\n- idea x=10 y=10 w=200 h=100\n::end\n\n\
::heading level=2 id=next\nNext\n::end\n\n\
::paragraph id=idea hidden\nThe idea.\n::end\n";
        let sliced = section(&parse(source), "plan-sec").expect("heading section");
        let page = crate::render::html(&sliced, &crate::render::HtmlOptions::default());
        assert!(!page.contains("no block named"), "{page}");
        assert!(page.contains("The idea."));
    }

    #[test]
    fn sections_carry_template_and_style_blocks_from_outside_the_slice() {
        let source = "::script id=v type=application/json\n{\"k\":\"v\"}\n::end\n\n\
::heading level=2 id=s\nS\n::end\n\n::paragraph id=p\n{{k}}\n::end\n";
        let sliced = section(&parse(source), "s").expect("section");
        assert_eq!(sliced.blocks[0].kind, "script");
        assert_eq!(sliced.blocks.len(), 3);
    }
}
