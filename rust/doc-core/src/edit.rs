//! Changing a document, one block at a time.
//!
//! Four operations change a document — read a block's body, replace it, add one, take one
//! out — and a fifth changes nothing: [`preview_block`] says what a block *would* look like
//! with the characters a reader has typed so far, which is how a page keeps rendering while
//! it is being written on. Every editing surface performs all five through here.
//! `dx source`/`dx set`/`dx insert`/`dx remove` are this module with a command line in
//! front; `doc-wasm` exports it to the VS Code webview; DX.app reaches it through the binary
//! it carries.
//!
//! That matters more than the size of the code suggests. "What is the editable text of a
//! checklist?" has exactly one right answer, and a second implementation of it in an editor
//! is a document that changes shape depending on which surface last touched it. The rule is
//! the same one the renderer follows: one engine, no re-implementations.
//!
//! # The body of a block
//! Every block kind carries its content somewhere different — a list in `items`, an image in
//! `alt`, everything else in `text`. [`body`] and [`set_body`] are inverses across all of
//! them, so what a surface shows a reader is exactly what it can hand back unchanged.

use crate::format::{
    build_nested_list_structure, list_lines, parse, parse_checklist_line, parse_list_items,
    slugify_heading, stringify,
};
use crate::model::{Block, Document, Item};
use crate::render::{self, HtmlOptions};

/// Block kinds that may be created by [`insert_after`].
///
/// A subset of the format's kinds (`format::BLOCK_TYPES`) — the ones whose whole content is
/// a body a person types. Each of the rest is excluded for its own reason:
///
/// - `output` is written by running code; a person typing one would be recording a result
///   that was never computed.
/// - `image` lives in its `src` attribute, which a typed body (the alt text) cannot carry —
///   inserting one here would put a picture that points at nothing on the page.
/// - `nav`, `style`, `stylesheet`, and `script` are structural: a nav resolves against the
///   document's own headings, and the style/script kinds carry presentation for the page,
///   not prose. A surface inserts them deliberately, attributes and all, not as freehand
///   text.
/// - `graph` is the older spelling of `mermaid` (they render identically); new blocks get
///   the current name.
pub const AUTHORABLE: &[&str] = &[
    "paragraph",
    "heading",
    "quote",
    "code",
    "bulleted-list",
    "numbered-list",
    "checklist",
    "rule",
    "html",
    "svg",
    "mermaid",
];

/// The editable text of `block`: what a reader sees in the field when they click it.
///
/// A list becomes one line per item, which is how a person edits a list — not as a
/// structure, as the lines it is made of. Those lines are exactly the body lines the
/// canonical writer puts in the file ([`list_lines`]): an indented `- text` per item, so a
/// nested child is visible as its indentation and survives the save. A checklist's lines
/// keep their `[x]`/`[ ]` markers the same way, because the marker is where the checked
/// state lives — a body without it would erase every tick on the next save. (Checklists do
/// not nest; their body is flat by the format's own rule.)
#[must_use]
pub fn body(block: &Block) -> String {
    match block.kind.as_str() {
        "bulleted-list" | "numbered-list" => {
            let mut lines = Vec::new();
            list_lines(&block.items, 0, &mut lines);
            lines.join("\n")
        }
        "checklist" => block
            .items
            .iter()
            .map(|item| {
                let marker = if item.checked { "[x]" } else { "[ ]" };
                format!("{marker} {}", item.text)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "image" => block.alt.clone(),
        _ => block.text.clone(),
    }
}

/// Put `body` back into whichever field carries content for this block's kind.
///
/// The inverse of [`body`]. A list's items are rebuilt by the parser's own grammar
/// ([`parse_list_items`] then [`build_nested_list_structure`]): one leading `-`/`*`
/// **followed by whitespace** is a bullet and comes off, a line that merely starts with
/// those characters — `--verbose`, `*emphasis*` — is content and stays verbatim, and an
/// indented line nests under the item above it, so what the field showed is the tree the
/// save produces. Checklist lines are read the way the parser reads them
/// ([`parse_checklist_line`]): a `[x]`/`[ ]` prefix carries the checked state, and a bare
/// line is an unchecked item.
pub fn set_body(block: &mut Block, body: &str) {
    match block.kind.as_str() {
        "bulleted-list" | "numbered-list" => {
            let lines: Vec<String> = body.lines().map(str::to_string).collect();
            block.items = build_nested_list_structure(&parse_list_items(&lines));
            block.text.clear();
        }
        "checklist" => {
            block.items = body
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| match parse_checklist_line(line) {
                    Some((checked, text)) => Item {
                        checked,
                        text,
                        ..Item::default()
                    },
                    None => Item {
                        text: line.to_string(),
                        ..Item::default()
                    },
                })
                .filter(|item| !item.text.is_empty())
                .collect();
            block.text.clear();
        }
        "image" => block.alt = body.to_string(),
        _ => block.text = body.to_string(),
    }
}

/// Position of the block called `id`, or a message listing the ids that do exist.
///
/// The matching rule is the document's own ([`Document::block_index`]); what this adds is
/// the sentence a person or an agent needs when the id they asked for is not there.
///
/// # Errors
/// Returns a sentence naming the available ids, which is the thing the caller actually needs
/// when the one they asked for is not there.
pub fn find(document: &Document, id: &str) -> Result<usize, String> {
    document.block_index(id).ok_or_else(|| {
        let available = document
            .blocks
            .iter()
            .map(|block| block.id.clone())
            .collect::<Vec<_>>()
            .join(", ");
        format!("no block named `{id}`. Available: {available}")
    })
}

/// The editable text of one block of `source`.
///
/// # Errors
/// Returns a message when no block carries `id`.
pub fn block_source(source: &str, id: &str) -> Result<String, String> {
    let document = parse(source);
    Ok(body(&document.blocks[find(&document, id)?]))
}

/// Replace one block's body, returning the whole document's canonical source.
///
/// Every other block comes back byte-identical: this parses, changes one field, and
/// stringifies, so a surface saving a paragraph cannot disturb the document around it.
///
/// # Errors
/// Returns a message when no block carries `id`.
pub fn set_block(source: &str, id: &str, text: &str) -> Result<String, String> {
    let mut document = parse(source);
    let index = find(&document, id)?;
    set_body(&mut document.blocks[index], text);
    Ok(stringify(&document))
}

/// The HTML one block renders to when its body is `body`, without saving anything.
///
/// This is what "the page keeps rendering while you write on it" is made of: a surface hands
/// over the characters currently in the field and gets back the block as it will be read.
/// Nothing is written — [`set_body`] is applied to a parsed copy and thrown away — so a
/// reader who types and then presses Escape has changed no file.
///
/// It is deliberately one call rather than "set, then render" performed by each host: the
/// two halves have to agree about what a block's body means, and they agree by being the
/// same two functions every time.
///
/// # Errors
/// Returns a message naming the available ids when no block carries `id`.
pub fn preview_block(
    source: &str,
    id: &str,
    body: &str,
    options: &HtmlOptions,
) -> Result<String, String> {
    let mut document = parse(source);
    let index = find(&document, id)?;
    set_body(&mut document.blocks[index], body);
    // `set_body` writes the body and nothing else, so the block still answers to the id it
    // was found by; `render::block` can only miss if that stops being true.
    let block_id = document.blocks[index].id.clone();
    Ok(render::block(&document, &block_id, options).unwrap_or_default())
}

/// A block to be created by [`insert_after`]: its kind, the body a person typed, and — for
/// a `code` block — the execution attributes only that kind carries.
///
/// `language`, `run`, and `deps` mean nothing on any other kind and are dropped by
/// normalization there, exactly as they would be if hand-written in the file; a surface
/// that wants to reject them earlier (as `dx insert` does) checks before calling.
#[derive(Debug, Clone, Copy, Default)]
pub struct Insertion<'a> {
    /// One of [`AUTHORABLE`].
    pub kind: &'a str,
    /// The typed body, read the way [`set_body`] reads it for this kind.
    pub body: &'a str,
    /// The id to give the new block; empty lets the document name it. It is slugified the
    /// way every other id is, and any spelling that lands on an id some block already
    /// answers to is refused, never taken — a silent rename would break every reference
    /// pointing at the block that held it.
    pub id: &'a str,
    /// A heading's level (1–4); 0 means the default, 2.
    pub level: u8,
    /// A code block's `lang` attribute; empty means unset.
    pub language: &'a str,
    /// Whether a code block is marked `run`.
    pub run: bool,
    /// A code block's `deps` attribute; empty means unset.
    pub deps: &'a str,
}

/// Refuse a kind [`insert_after`] cannot create, naming the ones it can.
///
/// The one sentence for every surface that authors blocks — `dx append`, `dx insert`, and
/// the editors — so "what may be written" is answered identically wherever it is asked.
///
/// # Errors
/// Returns that sentence when `kind` is not one of [`AUTHORABLE`].
pub fn authorable(kind: &str) -> Result<(), String> {
    if AUTHORABLE.contains(&kind) {
        return Ok(());
    }
    Err(format!(
        "cannot add a `{kind}` block. Supported: {}",
        AUTHORABLE.join(", ")
    ))
}

/// The existing id an explicit `id` would land on, or `None` when it is free (or empty).
///
/// The check has to be made against the id the writer *will* produce, not the characters the
/// caller typed: an id is slugified on its way into the document (`My Id` becomes `my-id`),
/// so comparing the raw spelling let `--id "My Id"` past the refusal below and the registry
/// then resolved the collision the silent way, by renaming the new block to `my-id-2`. The
/// caller asked for a name and got a different one, which is exactly what refusing exists to
/// prevent.
fn colliding_id(document: &Document, id: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    let wanted = slugify_heading(id.trim().trim_start_matches('#'));
    document
        .blocks
        .iter()
        .find(|block| block.id == wanted)
        .map(|block| block.id.clone())
}

/// Add `block` directly after the block called `after`, returning the new source and the id
/// the new block was given.
///
/// `after` being `None` puts the block at the very top, which is what a reader gets when
/// they ask for something before the first line.
///
/// The new block never takes an id the document already uses: when its natural name (the
/// same rule that named every other id) belongs to an existing block, the new block yields
/// and takes the next free suffix instead. An insert that silently renamed some *other*
/// block would break every nav target and external reference pointing at it.
///
/// An explicit [`Insertion::id`] that some block already answers to is refused with a
/// sentence, never taken: the registry would otherwise resolve the collision by renaming,
/// silently, and whichever block lost the name would lose its references with it. The
/// refusal is against the id the writer would produce, so every spelling that slugifies onto
/// a taken name is refused too — see [`colliding_id`].
///
/// # Errors
/// Returns a message when the kind is not authorable, when no block carries `after`, or
/// when an explicit id is already in use.
pub fn insert_after(
    source: &str,
    after: Option<&str>,
    block: &Insertion,
) -> Result<(String, String), String> {
    authorable(block.kind)?;

    let mut document = parse(source);
    if let Some(taken) = colliding_id(&document, block.id) {
        return Err(format!(
            "a block named `{taken}` already exists — pick an unused id, or leave the id out \
             and the document will choose one",
        ));
    }
    let existing_ids: Vec<String> = document
        .blocks
        .iter()
        .map(|block| block.id.clone())
        .collect();
    let at = match after {
        Some(id) => find(&document, id)? + 1,
        None => 0,
    };

    let mut new_block = Block {
        kind: block.kind.to_string(),
        id: block.id.to_string(),
        level: if block.level == 0 { 2 } else { block.level },
        language: block.language.to_string(),
        run: block.run,
        deps: block.deps.to_string(),
        ..Block::default()
    };
    set_body(&mut new_block, block.body);
    document.blocks.insert(at, new_block);

    // Parsing the result is what names the new block, by the same rule that named every
    // other id in the document — rather than a second id scheme invented here that could
    // collide with the first.
    let named = parse(&stringify(&document));
    let natural = named
        .blocks
        .get(at)
        .map(|block| block.id.clone())
        .unwrap_or_default();
    if !existing_ids.contains(&natural) {
        return Ok((stringify(&named), natural));
    }

    // The natural name belongs to an existing block, which the naming pass would have
    // renamed out from under its references. Give the new block the next free suffix —
    // the registry's own collision rule — explicitly, so every existing id stays put.
    let mut count = 2;
    let fresh = loop {
        let candidate = format!("{natural}-{count}");
        if !existing_ids.contains(&candidate) {
            break candidate;
        }
        count += 1;
    };
    document.blocks[at].id = fresh.clone();
    Ok((stringify(&document), fresh))
}

/// Take one block out, returning the document's canonical source without it.
///
/// # Errors
/// Returns a message when no block carries `id`.
pub fn remove_block(source: &str, id: &str) -> Result<String, String> {
    let mut document = parse(source);
    let index = find(&document, id)?;
    document.blocks.remove(index);
    Ok(stringify(&document))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "::heading level=1 id=title\nGuide\n::end\n\n\
::paragraph id=intro\nThe opening line.\n::end\n\n\
::bulleted-list id=points\n- first\n- second\n::end\n";

    /// What a reader sees while they are typing is the block they will have when they stop.
    #[test]
    fn a_preview_is_the_block_the_save_would_produce() {
        let options = HtmlOptions::default();
        let typed = "A *new* opening line.";

        let previewed = preview_block(SAMPLE, "intro", typed, &options).expect("preview");
        let saved = set_block(SAMPLE, "intro", typed).expect("set");
        let rendered = render::block(&parse(&saved), "intro", &options).expect("the block");

        assert_eq!(previewed, rendered);
        assert!(previewed.contains("<em>new</em>"), "{previewed}");
    }

    /// A preview is a *read*: it renders characters that have not been saved, and saves none
    /// of them. Escape after typing has to leave the document exactly as it was.
    #[test]
    fn a_preview_changes_nothing() {
        let options = HtmlOptions::default();
        preview_block(SAMPLE, "intro", "something else entirely", &options).expect("preview");
        assert_eq!(
            block_source(SAMPLE, "intro").expect("body"),
            "The opening line."
        );
    }

    /// A list previews as a list — the block's kind decides what its lines mean, which is the
    /// whole reason this goes through `set_body` rather than through the text.
    #[test]
    fn a_preview_reads_the_body_the_way_the_block_does() {
        let previewed = preview_block(SAMPLE, "points", "one\ntwo\nthree", &HtmlOptions::default())
            .expect("preview");
        assert_eq!(previewed.matches("<li>").count(), 3, "{previewed}");
    }

    #[test]
    fn previewing_an_unknown_block_names_the_ones_that_exist() {
        let failure = preview_block(SAMPLE, "nope", "text", &HtmlOptions::default())
            .expect_err("no such block");
        assert!(failure.contains("intro"), "{failure}");
    }

    #[test]
    fn a_blocks_body_is_what_a_reader_would_type() {
        assert_eq!(
            block_source(SAMPLE, "intro").expect("intro"),
            "The opening line."
        );
        assert_eq!(block_source(SAMPLE, "title").expect("title"), "Guide");
        // A list edits as its body lines — the same `- text` lines the writer puts in the
        // file, so nesting is visible and nothing depends on a second grammar.
        assert_eq!(
            block_source(SAMPLE, "points").expect("points"),
            "- first\n- second"
        );
    }

    /// The property the whole module rests on: what a surface shows is what it can hand
    /// straight back, for every kind of block.
    #[test]
    fn showing_a_body_and_saving_it_unchanged_changes_nothing() {
        let canonical = stringify(&parse(SAMPLE));
        for id in ["title", "intro", "points"] {
            let shown = block_source(&canonical, id).expect("body");
            let saved = set_block(&canonical, id, &shown).expect("set");
            assert_eq!(
                saved, canonical,
                "round trip changed the document at `{id}`"
            );
        }
    }

    #[test]
    fn setting_one_block_leaves_every_other_alone() {
        let after = set_block(SAMPLE, "intro", "A replacement line.").expect("set");
        assert!(after.contains("A replacement line."));
        assert!(!after.contains("The opening line."));
        assert!(after.contains("Guide"));
        assert!(after.contains("first"));
    }

    /// The un-marking rule is the parser's rule: one `-`/`*` **followed by whitespace** is
    /// a bullet. Text that merely starts with those characters — a `--flag`, `*emphasis*` —
    /// is content, and an unchanged save must not eat it.
    #[test]
    fn a_list_item_that_starts_with_marker_characters_survives_an_unchanged_save() {
        let source = "::bulleted-list id=flags\n\
            - --verbose enables logging\n\
            - *emphasis*\n\
            - item\n::end\n";
        let canonical = stringify(&parse(source));
        let shown = block_source(&canonical, "flags").expect("body");
        assert_eq!(shown, "- --verbose enables logging\n- *emphasis*\n- item");
        let saved = set_block(&canonical, "flags", &shown).expect("set");
        assert_eq!(saved, canonical, "an unchanged save changed the list");
    }

    /// Nesting is part of the list: the body shows children as the indented `- ` lines the
    /// writer puts in the file, and an unchanged save keeps the whole tree.
    #[test]
    fn a_nested_list_survives_an_unchanged_save() {
        let source = "::bulleted-list id=tree\n\
            - parent\n\
            \x20 - child\n\
            \x20 - second child\n\
            - sibling\n::end\n";
        let canonical = stringify(&parse(source));
        let shown = block_source(&canonical, "tree").expect("body");
        assert_eq!(shown, "- parent\n  - child\n  - second child\n- sibling");
        let saved = set_block(&canonical, "tree", &shown).expect("set");
        assert_eq!(saved, canonical, "an unchanged save changed the nesting");
    }

    /// An indented line a reader types nests under the item above it — the same rule the
    /// parser applies to a document's own body lines.
    #[test]
    fn a_typed_indented_line_becomes_a_nested_child() {
        let saved = set_block(SAMPLE, "points", "- one\n  - one-a\n- two").expect("set");
        let document = parse(&saved);
        let items = &document.blocks[find(&document, "points").expect("points")].items;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "one");
        assert_eq!(items[0].nested.len(), 1);
        assert_eq!(items[0].nested[0].text, "one-a");
        assert_eq!(items[1].text, "two");
        assert!(items[1].nested.is_empty());
    }

    /// A checklist's checked state lives in its `[x]`/`[ ]` markers, so the editable text
    /// carries them — and an unchanged save keeps every tick, and every literal `-`/`*`.
    #[test]
    fn a_checklist_body_carries_its_marks_and_survives_an_unchanged_save() {
        let source = "::checklist id=todo\n[x] ship it\n[ ] --dry-run first\n::end\n";
        let canonical = stringify(&parse(source));
        let shown = block_source(&canonical, "todo").expect("body");
        assert_eq!(shown, "[x] ship it\n[ ] --dry-run first");
        let saved = set_block(&canonical, "todo", &shown).expect("set");
        assert_eq!(saved, canonical, "an unchanged save changed the checklist");
    }

    /// Typed checklist lines mean what they would mean in the file: `[x]` ticks the item,
    /// and a bare line is an unchecked item — the same reading `parse` gives those lines.
    #[test]
    fn typed_checklist_lines_set_the_checked_state_the_way_the_parser_would() {
        let source = "::checklist id=todo\n[ ] a\n::end\n";
        let saved = set_block(source, "todo", "[x] done\nplain").expect("set");
        let document = parse(&saved);
        let items = &document.blocks[find(&document, "todo").expect("todo")].items;
        assert_eq!(items.len(), 2);
        assert!(items[0].checked);
        assert_eq!(items[0].text, "done");
        assert!(!items[1].checked);
        assert_eq!(items[1].text, "plain");
    }

    #[test]
    fn a_list_rebuilt_from_lines_drops_the_bullets_a_reader_typed() {
        let after = set_block(SAMPLE, "points", "- one\n* two\nthree").expect("set");
        let document = parse(&after);
        let items = &document.blocks[find(&document, "points").expect("points")].items;
        assert_eq!(
            items
                .iter()
                .map(|item| item.text.clone())
                .collect::<Vec<_>>(),
            vec!["one", "two", "three"]
        );
    }

    /// A prose insertion: the kind and the typed body, nothing else.
    fn prose<'a>(kind: &'a str, body: &'a str) -> Insertion<'a> {
        Insertion {
            kind,
            body,
            ..Insertion::default()
        }
    }

    #[test]
    fn inserting_puts_the_block_where_it_was_asked_for_and_names_it() {
        let (after, id) =
            insert_after(SAMPLE, Some("intro"), &prose("paragraph", "New.")).expect("insert");
        assert!(!id.is_empty(), "the new block was not named");
        let document = parse(&after);
        let at = find(&document, &id).expect("the new block");
        assert_eq!(at, find(&document, "intro").expect("intro") + 1);
        assert_eq!(document.blocks[at].text, "New.");
        assert_eq!(document.blocks.len(), parse(SAMPLE).blocks.len() + 1);
    }

    #[test]
    fn inserting_with_no_anchor_puts_the_block_first() {
        let (after, id) = insert_after(SAMPLE, None, &prose("paragraph", "Top.")).expect("insert");
        let document = parse(&after);
        assert_eq!(find(&document, &id).expect("new"), 0);
    }

    /// The attributes a runnable block carries arrive with it — an insert that dropped
    /// `lang`/`run`/`deps` could only author code that cannot execute.
    #[test]
    fn inserting_a_code_block_carries_its_execution_attributes() {
        let insertion = Insertion {
            kind: "code",
            body: "print(1)",
            language: "python",
            run: true,
            deps: "requests",
            ..Insertion::default()
        };
        let (after, id) = insert_after(SAMPLE, Some("intro"), &insertion).expect("insert");
        let document = parse(&after);
        let block = &document.blocks[find(&document, &id).expect("new")];
        assert_eq!(block.language, "python");
        assert!(block.run);
        assert_eq!(block.deps, "requests");
        assert_eq!(block.text, "print(1)");
    }

    /// The bug this pins down: a new block whose natural name collided with an existing id
    /// used to *take* that id, silently renaming the existing block — which broke every nav
    /// target and reference pointing at it. The new block yields; every existing id stays.
    #[test]
    fn inserting_never_steals_an_existing_blocks_id() {
        let source = "::paragraph id=intro\nHello.\n::end\n\n::code id=code-2 lang=python\nprint(1)\n::end\n";
        let (after, id) =
            insert_after(source, Some("intro"), &prose("code", "x = 2")).expect("insert");
        let document = parse(&after);

        let kept = &document.blocks[find(&document, "code-2").expect("the original block")];
        assert_eq!(kept.language, "python", "the existing block was renamed");
        assert_ne!(id, "code-2", "the new block took an existing id");
        assert_eq!(
            document.blocks[find(&document, &id).expect("new")].text,
            "x = 2"
        );
    }

    /// The same rule holds when the collision comes from a heading's text, where the
    /// natural name is the slug readers link to: the anchor stays with the original.
    #[test]
    fn inserting_a_duplicate_heading_leaves_the_original_anchor_in_place() {
        let source =
            "::paragraph id=intro\nHello.\n::end\n\n::heading level=2 id=guide\nGuide\n::end\n";
        let (after, id) =
            insert_after(source, Some("intro"), &prose("heading", "Guide")).expect("insert");
        let document = parse(&after);

        let original = find(&document, "guide").expect("the original heading");
        assert_eq!(
            original,
            document.blocks.len() - 1,
            "`guide` moved to a different block"
        );
        assert_ne!(id, "guide", "the new heading took an existing anchor");
        assert!(
            find(&document, &id).is_ok(),
            "the new heading was not named"
        );
    }

    #[test]
    fn removing_takes_out_one_block_and_only_that_one() {
        let after = remove_block(SAMPLE, "intro").expect("remove");
        assert!(!after.contains("The opening line."));
        assert!(after.contains("Guide"));
        assert_eq!(parse(&after).blocks.len(), parse(SAMPLE).blocks.len() - 1);
    }

    #[test]
    fn an_id_matches_however_the_reader_wrote_it() {
        assert!(block_source(SAMPLE, "#Intro").is_ok());
        assert!(block_source(SAMPLE, " intro ").is_ok());
    }

    #[test]
    fn a_missing_block_is_answered_with_the_ones_that_exist() {
        let error = block_source(SAMPLE, "nope").expect_err("should fail");
        assert!(error.contains("no block named `nope`"));
        assert!(error.contains("intro"), "{error}");
    }

    #[test]
    fn an_output_block_cannot_be_written_by_hand() {
        let error =
            insert_after(SAMPLE, Some("intro"), &prose("output", "5")).expect_err("should fail");
        assert!(error.contains("cannot add"));
    }

    /// An explicit id names the new block — the caller who chose it can reference it.
    #[test]
    fn inserting_with_an_explicit_id_uses_it() {
        let insertion = Insertion {
            kind: "paragraph",
            body: "Named.",
            id: "callout",
            ..Insertion::default()
        };
        let (after, id) = insert_after(SAMPLE, Some("intro"), &insertion).expect("insert");
        assert_eq!(id, "callout");
        let document = parse(&after);
        assert_eq!(
            document.blocks[find(&document, "callout").expect("callout")].text,
            "Named."
        );
    }

    /// The defect this pins down: an explicit id a block already answered to used to be
    /// resolved by the registry renaming somebody, silently. It is refused with a sentence,
    /// and however the caller spelled the id, since matching is the document's own rule.
    #[test]
    fn inserting_refuses_an_explicit_id_that_is_already_taken() {
        for taken in ["intro", "#Intro", " intro "] {
            let insertion = Insertion {
                kind: "paragraph",
                body: "x",
                id: taken,
                ..Insertion::default()
            };
            let error = insert_after(SAMPLE, Some("title"), &insertion).expect_err("should fail");
            assert!(error.contains("already exists"), "{error}");
        }
    }

    /// The defect this pins down: the refusal compared the characters the caller typed,
    /// while the writer slugifies them — so `My Id` slipped past the guard and came back
    /// renamed to `my-id-2`, which is the silent rename refusing exists to prevent. Every
    /// spelling that lands on a taken id is refused, and it names the id that is taken.
    #[test]
    fn inserting_refuses_an_id_that_slugifies_onto_a_taken_one() {
        let source = "::heading level=1 id=title\nGuide\n::end\n\n\
                      ::paragraph id=my-id\nOriginal.\n::end\n";
        for spelling in ["My Id", "my id", "My  Id!", "MY-ID"] {
            let insertion = Insertion {
                kind: "paragraph",
                body: "x",
                id: spelling,
                ..Insertion::default()
            };
            let error =
                insert_after(source, Some("title"), &insertion).expect_err("should be refused");
            assert!(error.contains("`my-id` already exists"), "{error}");
        }

        // A spelling that lands somewhere free is still accepted, under its slugified name.
        let insertion = Insertion {
            kind: "paragraph",
            body: "x",
            id: "Another Note",
            ..Insertion::default()
        };
        let (after, id) = insert_after(source, Some("title"), &insertion).expect("insert");
        assert_eq!(id, "another-note");
        assert!(after.contains("::paragraph id=another-note"), "{after}");
    }

    /// An explicit level reaches the block; 0 (the default) means level 2.
    #[test]
    fn inserting_a_heading_carries_its_level() {
        let insertion = Insertion {
            kind: "heading",
            body: "Deep",
            level: 3,
            ..Insertion::default()
        };
        let (after, id) = insert_after(SAMPLE, Some("intro"), &insertion).expect("insert");
        let document = parse(&after);
        assert_eq!(document.blocks[find(&document, &id).expect("new")].level, 3);

        let (defaulted, id) =
            insert_after(SAMPLE, Some("intro"), &prose("heading", "Plain")).expect("insert");
        let document = parse(&defaulted);
        assert_eq!(document.blocks[find(&document, &id).expect("new")].level, 2);
    }
}
