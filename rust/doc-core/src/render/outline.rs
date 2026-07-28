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
            preview: preview_of(&block_content(document, index)),
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

/// Collect the slice `start..end` plus the document's presentation blocks.
///
/// `::script` (template values) and `::style` blocks live wherever the author put them, so
/// a slice that omitted them would render placeholders as blanks. They are carried along,
/// deduplicated against blocks already inside the slice, and always come first.
fn carry_presentation(document: &Document, start: usize, end: usize) -> Vec<crate::model::Block> {
    let mut blocks: Vec<crate::model::Block> = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(index, block)| {
            (*index < start || *index >= end)
                && matches!(block.kind.as_str(), "script" | "style" | "stylesheet")
        })
        .map(|(_, block)| block.clone())
        .collect();
    blocks.extend(document.blocks[start..end].iter().cloned());
    blocks
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

/// The reading content of a block, whichever field carries it for that kind.
fn block_content(document: &Document, index: usize) -> String {
    let block = &document.blocks[index];
    match block.kind.as_str() {
        "bulleted-list" | "numbered-list" | "checklist" => block
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

    #[test]
    fn sections_carry_template_and_style_blocks_from_outside_the_slice() {
        let source = "::script id=v type=application/json\n{\"k\":\"v\"}\n::end\n\n\
::heading level=2 id=s\nS\n::end\n\n::paragraph id=p\n{{k}}\n::end\n";
        let sliced = section(&parse(source), "s").expect("section");
        assert_eq!(sliced.blocks[0].kind, "script");
        assert_eq!(sliced.blocks.len(), 3);
    }
}
