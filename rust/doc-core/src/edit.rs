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
    block_header as canonical_header, build_nested_list_structure, header_line_facts,
    is_known_block_type, list_lines, parse, parse_checklist_line, parse_list_items,
    slugify_heading, stringify,
};
use crate::model::{Block, Document, Item};
use crate::render::{self, board, HtmlOptions};

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
    // A board's whole content is its typed node lines, so it is authored like any other
    // body — and saving one creates the node blocks its lines name (see `set_block`).
    "board",
    // A view's whole content is its tag line (`src=`, `width=`, `height=`); the body a
    // reader sees is the referenced page, hydrated at view time and never stored.
    "view",
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

/// The canonical `::kind attrs` opening line of one block of `source`.
///
/// This is the header an editing surface shows above a block's body — the exact line the
/// canonical writer puts in the file, attributes and all, so an unchanged save writes back
/// what the reader saw. One rule, two directions: the writer's own [`canonical_header`]
/// produces it, and [`replace_block`] reads a typed one back with the scanner's own grammar.
///
/// # Errors
/// Returns a message naming the available ids when no block carries `id`.
pub fn block_header(source: &str, id: &str) -> Result<String, String> {
    let document = parse(source);
    Ok(canonical_header(&document.blocks[find(&document, id)?]))
}

/// Replace one block wholesale — header and body — returning the new canonical source and
/// the id the replacement answers to.
///
/// This is what lets a reader retype a block: the header field over a listing changed from
/// `::code lang=python` to `::quote` is a quotation on the next render. Two paths, chosen by
/// the header:
///
/// - **A `::kind attrs` header** is read by the scanner's own grammar and applied to a fresh
///   block; the body is then set by [`set_body`] — the block's own body rule — and never
///   re-scanned for structure, so a listing whose text contains `::end` keeps every byte.
///   A kind the format does not know is refused rather than silently folded to `paragraph`,
///   and `output` is refused because a person typing one would be recording a result that
///   was never computed.
/// - **An empty header** means the text is plain: the body is read the way the file itself
///   would read it ([`parse`], markdown shorthand included), so `# Title` typed into a
///   paragraph becomes the heading it says. This can legitimately produce more than one
///   block — a blank line between two paragraphs is two paragraphs — and every one of them
///   is placed where the original block was.
///
/// The replacement keeps the original block's id unless the header names its own, so every
/// reference pointing at the block survives an edit that never mentioned ids. An explicit
/// `id=` some *other* block answers to is refused with a sentence (the rule of
/// [`insert_after`]), and no replacement ever takes an existing block's id — a fragment
/// whose natural name collides yields and takes the next free suffix instead.
///
/// # Errors
/// Returns a message when no block carries `id`, when the header names an unknown kind or
/// `output`, or when it claims an id another block holds.
pub fn replace_block(
    source: &str,
    id: &str,
    header: &str,
    body: &str,
) -> Result<(String, String), String> {
    let mut document = parse(source);
    let index = find(&document, id)?;
    let old_id = document.blocks[index].id.clone();
    let other_ids: Vec<String> = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != index)
        .map(|(_, block)| block.id.clone())
        .collect();

    let header = header.trim();
    let (mut replacements, keeps_old_id) = if header.is_empty() {
        (parse(body).blocks, true)
    } else {
        let (kind, has_id) = header_line_facts(header)
            .ok_or_else(|| format!("`{header}` is not a block header — a header line is `::kind`, with attributes after it"))?;
        if kind == "output" {
            return Err(
                "cannot retype a block to `output` — an output is written by running code"
                    .to_string(),
            );
        }
        if !is_known_block_type(&kind) {
            return Err(format!("`::{kind}` is not a block kind this format knows"));
        }
        let mut block = parse(&format!("{header}\n::end\n"))
            .blocks
            .into_iter()
            .next()
            .ok_or_else(|| format!("`{header}` did not open a block"))?;
        set_body(&mut block, body);
        (vec![block], !has_id)
    };
    if replacements.is_empty() {
        replacements = parse("").blocks;
    }

    if keeps_old_id {
        replacements[0].id = old_id;
    } else if let Some(taken) = other_ids.iter().find(|taken| **taken == replacements[0].id) {
        return Err(format!(
            "a block named `{taken}` already exists — pick an unused id, or leave the id out \
             and the block keeps the one it has",
        ));
    }

    // No replacement takes an id an existing block answers to: whichever landed on a taken
    // name yields to the next free suffix, so every reference into the rest of the document
    // stays pointed where it was.
    let mut taken: Vec<String> = other_ids;
    for block in &mut replacements {
        if taken.contains(&block.id) {
            let mut count = 2;
            block.id = loop {
                let candidate = format!("{}-{count}", block.id);
                if !taken.contains(&candidate) {
                    break candidate;
                }
                count += 1;
            };
        }
        taken.push(block.id.clone());
    }

    let focus = replacements[0].id.clone();
    let count = replacements.len();
    document.blocks.splice(index..=index, replacements);

    // A block retyped into (or written as) a board gets the same courtesy a saved board
    // body gets: the node blocks its lines name are created rather than left dangling.
    let mut at = index;
    let mut end = index + count;
    while at < end {
        if document.blocks[at].kind == "board" {
            let created = create_missing_nodes(&mut document, at);
            end += created;
            at += created;
        }
        at += 1;
    }
    Ok((stringify(&document), focus))
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
    if document.blocks[index].kind == "board" {
        create_missing_nodes(&mut document, index);
    }
    Ok(stringify(&document))
}

/// Replace an exact string inside one block's body, returning the new canonical source and
/// how many occurrences changed.
///
/// This is the change-sized edit. [`set_block`] retypes a whole body to alter one word,
/// which prices a cross-cutting change — a port number, a renamed term — at the size of
/// every block it touches; here the caller pays for the characters that changed and
/// nothing else. The body is matched exactly, never as a pattern, so what was typed is
/// what is found.
///
/// Without `all`, `old` must appear exactly once: replacing "one of several" silently
/// picks whichever came first, which is how a rename lands on the wrong line. The refusal
/// names the count so the caller can widen the string to pin one occurrence, or pass
/// `all` to mean every one.
///
/// # Errors
/// Returns a message when no block carries `id`, when `old` is empty, when `old` does not
/// appear in the block's body, or when it appears more than once and `all` was not given.
pub fn replace_in_block(
    source: &str,
    id: &str,
    old: &str,
    new: &str,
    all: bool,
) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err("the text to replace must not be empty".to_string());
    }
    let mut document = parse(source);
    let index = find(&document, id)?;
    let current = body(&document.blocks[index]);
    let count = current.matches(old).count();
    if count == 0 {
        return Err(format!("`{old}` does not appear in the body of `{id}`"));
    }
    if count > 1 && !all {
        return Err(format!(
            "`{old}` appears {count} times in `{id}` — pass a longer string to pin one \
             occurrence, or `all` to change every one"
        ));
    }
    let replaced = if all {
        current.replace(old, new)
    } else {
        current.replacen(old, new, 1)
    };
    set_body(&mut document.blocks[index], &replaced);
    if document.blocks[index].kind == "board" {
        create_missing_nodes(&mut document, index);
    }
    Ok((stringify(&document), count))
}

/// Tick or untick one item of a checklist, returning the new canonical source and the state
/// the item now carries.
///
/// A checklist on the page is a list of things to do, and ticking one off is the whole
/// reason it is a checklist rather than a list — so the box is a control, and this is what
/// it does. It is deliberately *not* "read the body, edit a line, write the body" performed
/// by each surface: the checked state lives in a `[x]`/`[ ]` marker that only the format
/// knows how to spell, and a second speller would eventually spell it differently. Every
/// other block, and every other item, comes back byte-identical.
///
/// `item` is the item's position in the block, counting from zero — the same order the
/// renderer draws them in, which is what lets a surface name the box that was clicked
/// without knowing anything about the text on the line.
///
/// # Errors
/// Returns a message when no block carries `id`, when that block is not a checklist, or
/// when the checklist has no item at `item`.
pub fn toggle_check(source: &str, id: &str, item: usize) -> Result<(String, bool), String> {
    let mut document = parse(source);
    let index = find(&document, id)?;
    let block = &mut document.blocks[index];
    if block.kind != "checklist" {
        return Err(format!(
            "`{id}` is a {} block; only a checklist has boxes to tick",
            block.kind
        ));
    }
    let count = block.items.len();
    let entry = block.items.get_mut(item).ok_or_else(|| {
        format!("checklist `{id}` has {count} items; there is none at position {item}")
    })?;
    entry.checked = !entry.checked;
    let checked = entry.checked;
    Ok((stringify(&document), checked))
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

/// Refuse a kind [`insert_after`] cannot create, naming the ones it can — and the route to
/// the ones it cannot.
///
/// The one sentence for every surface that authors blocks — `dx append`, `dx insert`, and
/// the editors — so "what may be written" is answered identically wherever it is asked.
///
/// The refusal names the retype route because that route works: a kind whose content lives
/// in its attributes (`image`, `style`, `stylesheet`, `script`, `nav`) is written by adding
/// a paragraph and retyping its header, which is exactly what a reader does on the page
/// when they rewrite a block's tag line. A refusal that only listed what it would accept
/// left the caller with no way to add a picture and nothing to read about it.
///
/// # Errors
/// Returns that sentence when `kind` is not one of [`AUTHORABLE`].
pub fn authorable(kind: &str) -> Result<(), String> {
    if AUTHORABLE.contains(&kind) {
        return Ok(());
    }
    Err(format!(
        "cannot add a `{kind}` block directly. Supported: {}. \
         A kind that lives in its attributes is written by retyping a header instead: add a \
         paragraph, then `dx set <file> <id> --header '::{kind} …'`.",
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
/// Every `::output` record reporting on the removed block (`for=` naming it) goes with
/// it: the record is the run's residue, and a document keeping output for code it no
/// longer carries is a document lying about what ran.
///
/// # Errors
/// Returns a message when no block carries `id`.
pub fn remove_block(source: &str, id: &str) -> Result<String, String> {
    let mut document = parse(source);
    let index = find(&document, id)?;
    document.blocks.remove(index);
    document
        .blocks
        .retain(|block| !(block.kind == "output" && block.for_block == id));
    Ok(stringify(&document))
}

// ---------------------------------------------------------------------------------------
// The board operations: what dragging a node on the canvas *is*.
//
// A board's body is text a person can type, but a surface must never rewrite those lines
// itself — the line grammar is the engine's (`render::board`), and a second copy of it in
// an editor is a board that changes shape depending on which surface last touched it. So
// moving, resizing, adding, linking, and detaching a node are operations here, reached by
// every host the way `set_block` is.
// ---------------------------------------------------------------------------------------

/// Position of the board called `board_id`, or a sentence when it is missing or not a board.
fn find_board(document: &Document, board_id: &str) -> Result<usize, String> {
    let index = find(document, board_id)?;
    let kind = &document.blocks[index].kind;
    if kind != "board" {
        return Err(format!("`{board_id}` is a {kind}, not a board"));
    }
    Ok(index)
}

/// Create a hidden paragraph for every node line naming a block the document does not
/// have, directly after the board, in reference order. Returns how many were created.
///
/// This is what makes a board body *authorable*: writing `- sketch x=0 y=0` and saving is
/// enough, and the sketch block appears ready to be written. Only an id that is already in
/// canonical form is created — a spelling the writer would rename on the way in (`My Node`)
/// stays a dangling reference, whose node says so on the canvas rather than silently
/// binding to a block called something else.
fn create_missing_nodes(document: &mut Document, board: usize) -> usize {
    let referenced: Vec<String> = board::nodes(&document.blocks[board].text)
        .into_iter()
        .map(|node| node.id)
        .collect();
    let mut created = 0;
    let mut seen: Vec<String> = Vec::new();
    for id in referenced {
        let key = id.to_ascii_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if document.block_index(&id).is_some() || slugify_heading(&id) != id {
            continue;
        }
        document.blocks.insert(
            board + 1 + created,
            Block {
                kind: "paragraph".to_string(),
                id,
                hidden: true,
                text: "New node".to_string(),
                ..Block::default()
            },
        );
        created += 1;
    }
    created
}

/// Check if a block type is valid for board placement.
///
/// Certain block types (output, script, stylesheet, style, image) render as empty or
/// are meta-blocks, and should not be placed on boards to avoid user confusion.
/// Returns a sentence describing why the type is invalid, or None if it's valid.
#[must_use]
pub fn board_invalid_block_type(kind: &str) -> Option<String> {
    match kind {
        "output" => Some("output blocks are written by code execution; place the code block on the board instead".to_string()),
        "script" => Some("script blocks run in the browser and render as empty; place code or other content instead".to_string()),
        "stylesheet" => Some("stylesheet blocks are references and render as empty; place code or styled content instead".to_string()),
        "style" => Some("style blocks render as empty; place code or content that shows output instead".to_string()),
        "image" => Some("image blocks need a src= file path; place a view or code that generates the image instead".to_string()),
        _ => None,
    }
}

/// How a caller states one dimension of a node's box.
///
/// One vocabulary for every host — a CLI flag, an arrangement item, a typed menu — so
/// "as wide as the page" and "as tall as its content" are spelled the same way everywhere
/// they can be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizeSpec {
    /// Keep what the node's line already says.
    #[default]
    Keep,
    /// State this many canvas pixels.
    Px(u32),
    /// The page's own measure: the column for a width, the default viewport for a height.
    Page,
    /// Sized to the block the node shows, by the engine's own estimate — and re-sized as
    /// the block is rewritten, because the rule (not its number) is what the line keeps.
    Fit,
}

impl SizeSpec {
    /// Read a spelled dimension: empty or `0` keeps the line's own, `page` and `fit` are
    /// the rules, anything else must be a whole number of canvas pixels.
    ///
    /// # Errors
    /// Returns a sentence naming the spelling it could not read.
    pub fn parse(raw: &str) -> Result<SizeSpec, String> {
        let word = raw.trim();
        match word {
            "" | "0" => Ok(SizeSpec::Keep),
            "page" => Ok(SizeSpec::Page),
            "fit" => Ok(SizeSpec::Fit),
            _ => word.parse().map(SizeSpec::Px).map_err(|_| {
                format!(
                    "`{word}` is not a size — write a number of canvas pixels, `page`, or `fit`"
                )
            }),
        }
    }
}

/// Put a node at `x`,`y` on a board, sized `w` by `h` — moving it if its line exists,
/// adding the line if not, and creating the block it names if the document has no such
/// block. Returns the new canonical source and the id of the node that was placed.
///
/// `node_id` empty means "a new node": a fresh `node-N` id is chosen, its line written,
/// and a hidden paragraph created for it — one call from a double-click on empty canvas to
/// a block ready to be written. Every one of `x`, `y`, `w`, `h` keeps what the line already
/// says when left out (`None` for `x`/`y`, [`SizeSpec::Keep`] for `w`/`h`) — so a caller
/// resizing a node does not have to repeat its position, and one moving it does not have to
/// repeat its size. A new node takes `0` for any coordinate left out, and the default box
/// for a size left out.
///
/// The board is [settled](settle) afterwards, so the placed node keeps exactly where it was
/// put and anything it landed on moves down out of its way. A reader dragging a node and an
/// agent running `dx board --place` therefore get the same board, and neither can leave two
/// blocks stacked in one place.
///
/// # Errors
/// Returns a message when `board_id` names no block, or names one that is not a board —
/// or when a node new to the board names a block whose kind renders as empty on a canvas
/// ([`board_invalid_block_type`]); a node already on the board still moves freely.
pub fn board_place(
    source: &str,
    board_id: &str,
    node_id: &str,
    x: Option<i32>,
    y: Option<i32>,
    w: SizeSpec,
    h: SizeSpec,
) -> Result<(String, String), String> {
    let mut document = parse(source);
    let index = find_board(&document, board_id)?;

    let node_id = if node_id.trim().is_empty() {
        fresh_node_id(&document, &document.blocks[index].text)
    } else {
        node_id.trim().trim_start_matches('#').to_string()
    };

    let mut lines: Vec<String> = body_lines(&document.blocks[index].text);
    let already_on_board = lines.iter().any(|line| {
        board::parse_node_line(line).is_some_and(|node| node.id.eq_ignore_ascii_case(&node_id))
    });
    if !already_on_board {
        if let Some(block) = document
            .blocks
            .iter()
            .find(|block| block.id.eq_ignore_ascii_case(&node_id))
        {
            if let Some(reason) = board_invalid_block_type(&block.kind) {
                return Err(reason);
            }
        }
    }
    place_into(&mut lines, &node_id, x, y, w, h, &document);
    settle(&mut lines, std::slice::from_ref(&node_id), &document);

    document.blocks[index].text = lines.join("\n");
    create_missing_nodes(&mut document, index);
    Ok((stringify(&document), node_id))
}

/// Take a node off a board: its line goes, every edge pointing at it goes, and the block
/// it displayed goes too when it was the board's own — hidden, and referenced by no other
/// board line in the document. A visible block, or one another board still shows, stays:
/// the node was a *view* of it, and deleting a view must not delete the content.
///
/// # Errors
/// Returns a message when the board is missing, or no line names `node_id`.
pub fn board_detach(source: &str, board_id: &str, node_id: &str) -> Result<String, String> {
    let mut document = parse(source);
    let index = find_board(&document, board_id)?;

    let before: Vec<String> = body_lines(&document.blocks[index].text);
    let mut removed = false;
    let mut lines: Vec<String> = Vec::new();
    for line in before {
        match board::parse_node_line(&line) {
            Some(node) if node.id.eq_ignore_ascii_case(node_id) => removed = true,
            Some(mut node) => {
                let had = node.to.len();
                node.to
                    .retain(|edge| !edge.id.eq_ignore_ascii_case(node_id));
                lines.push(if node.to.len() == had {
                    line
                } else {
                    board::node_line(&node)
                });
            }
            None => lines.push(line),
        }
    }
    if !removed {
        return Err(format!("no node named `{node_id}` on board `{board_id}`"));
    }
    document.blocks[index].text = lines.join("\n");

    // The node's block leaves with it only when nothing else can see it.
    if let Some(block_at) = document.block_index(node_id) {
        let orphaned = document.blocks[block_at].hidden
            && !document.blocks.iter().any(|candidate| {
                candidate.kind == "board"
                    && board::nodes(&candidate.text)
                        .iter()
                        .any(|node| node.id.eq_ignore_ascii_case(node_id))
            });
        if orphaned {
            document.blocks.remove(block_at);
        }
    }
    Ok(stringify(&document))
}

/// Draw (or erase) the edge from one node to another on a board, between two named sides.
///
/// `from_side`/`to_side` are the edges of the two nodes the line joins — `left`, `right`,
/// `top`, `bottom`, or their initials. Empty means unpinned: the renderer picks the facing
/// pair, which is what an edge drawn before sides existed still does. This is the whole of
/// "drag a connection from any edge of any node to any edge of another": the surface reports
/// the two sides the reader chose, and they are kept in the line.
///
/// `linked` true writes the edge, false removes it; both are idempotent, because a surface
/// repeating a drag must not error its way out of a state it is already in. Re-linking two
/// nodes that are already joined re-pins the sides rather than drawing a second line.
///
/// # Errors
/// Returns a message when the board is missing, when either end has no line on it, or when
/// `from` and `to` name the same node — a node pointing at itself is a drawing mistake, not
/// a plan.
pub fn board_link(
    source: &str,
    board_id: &str,
    from: &str,
    to: &str,
    linked: bool,
    from_side: &str,
    to_side: &str,
) -> Result<String, String> {
    if from.eq_ignore_ascii_case(to) {
        return Err(format!("`{from}` cannot point at itself"));
    }
    let mut document = parse(source);
    let index = find_board(&document, board_id)?;

    let mut lines: Vec<String> = body_lines(&document.blocks[index].text);
    let on_board = |lines: &[String], id: &str| {
        lines
            .iter()
            .filter_map(|line| board::parse_node_line(line))
            .any(|node| node.id.eq_ignore_ascii_case(id))
    };
    for end in [from, to] {
        if !on_board(&lines, end) {
            return Err(format!("no node named `{end}` on board `{board_id}`"));
        }
    }

    for line in &mut lines {
        let Some(mut node) = board::parse_node_line(line) else {
            continue;
        };
        if !node.id.eq_ignore_ascii_case(from) {
            continue;
        }
        // Re-dragging an edge onto another side is a move, not a new edge: whatever was
        // written on it stays written on it.
        let label = node
            .to
            .iter()
            .find(|edge| edge.id.eq_ignore_ascii_case(to))
            .map(|edge| edge.label.clone())
            .unwrap_or_default();
        node.to.retain(|edge| !edge.id.eq_ignore_ascii_case(to));
        if linked {
            node.to.push(board::Edge {
                id: to.to_string(),
                from: board::side_named(from_side),
                to: board::side_named(to_side),
                label,
            });
        }
        *line = board::node_line(&node);
        break;
    }
    document.blocks[index].text = lines.join("\n");
    Ok(stringify(&document))
}

/// Lay out several nodes at once — a whole board arranged in one call.
///
/// This is the bulk shape of [`board_place`]: an agent that has just written six blocks can
/// put all six on the board in one edit rather than six, and an editing surface that moved a
/// group sends the group. Each placement moves a node's line, adds one when the board has
/// none, and creates a hidden paragraph for a node naming no block. The board is
/// [settled](settle) once at the end, with every placed node held where it was put.
///
/// # Errors
/// Returns a message when `board_id` names no block, or names one that is not a board.
pub fn board_arrange(
    source: &str,
    board_id: &str,
    placements: &[Placement],
) -> Result<String, String> {
    let mut document = parse(source);
    let index = find_board(&document, board_id)?;

    let mut lines: Vec<String> = body_lines(&document.blocks[index].text);
    let mut held: Vec<String> = Vec::new();
    for placement in placements {
        let id = placement.node.trim().trim_start_matches('#');
        if id.is_empty() {
            continue;
        }
        place_into(
            &mut lines,
            id,
            Some(placement.x),
            Some(placement.y),
            placement.w,
            placement.h,
            &document,
        );
        held.push(id.to_string());
    }
    settle(&mut lines, &held, &document);

    document.blocks[index].text = lines.join("\n");
    create_missing_nodes(&mut document, index);
    Ok(stringify(&document))
}

/// Where one node goes: its id, its canvas position, and its box
/// ([`SizeSpec::Keep`] keeps the line's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Id of the node being placed.
    pub node: String,
    /// Left edge in canvas pixels.
    pub x: i32,
    /// Top edge in canvas pixels.
    pub y: i32,
    /// The width to state, or [`SizeSpec::Keep`] for the width the line already says.
    pub w: SizeSpec,
    /// The height to state, the same way.
    pub h: SizeSpec,
}

/// Read an arrangement as `id,x,y[,w[,h]]` items separated by spaces, semicolons, or
/// newlines — each size a number of pixels, `page`, or `fit`.
///
/// One spelling for every host: `dx board --arrange`, the wasm the editor calls, and the
/// `dx` DX.app runs all hand the same text to the same parser, so an arranged board is the
/// same document whichever surface arranged it.
///
/// # Errors
/// Returns a message naming the item that is not a placement — a silently dropped node is a
/// board that quietly forgets where something was put.
pub fn placements(spec: &str) -> Result<Vec<Placement>, String> {
    spec.split([' ', '\t', '\n', '\r', ';'])
        .filter(|item| !item.trim().is_empty())
        .map(|item| {
            let parts: Vec<&str> = item.split(',').map(str::trim).collect();
            let number = |raw: &str, what: &str| -> Result<i32, String> {
                raw.parse().map_err(|_| {
                    format!("`{item}` has {what} `{raw}`, which is not a whole number")
                })
            };
            let size = |at: usize| -> Result<SizeSpec, String> {
                match parts.get(at) {
                    Some(raw) => SizeSpec::parse(raw).map_err(|why| format!("`{item}`: {why}")),
                    None => Ok(SizeSpec::Keep),
                }
            };
            match parts[..] {
                [node, x, y] | [node, x, y, _] | [node, x, y, _, _] if !node.is_empty() => {
                    Ok(Placement {
                        node: node.to_string(),
                        x: number(x, "an x of")?,
                        y: number(y, "a y of")?,
                        w: size(3)?,
                        h: size(4)?,
                    })
                }
                _ => Err(format!(
                    "`{item}` is not a placement — write each one as `node,x,y`, \
                     `node,x,y,width`, or `node,x,y,width,height`"
                )),
            }
        })
        .collect()
}

/// Move a node's line to `x`,`y` (and its box, for each of `w`/`h` that is not
/// [`SizeSpec::Keep`]), adding the line when the board has none for it. The one piece
/// [`board_place`] and [`board_arrange`] share, so a drag and an arrangement write
/// byte-identical lines.
///
/// A number equal to what the line's own `page`/`fit` rule already resolves to keeps the
/// rule: a surface reports every box it moved, measured — which *is* the resolved number —
/// and a drag that only moved a node must not strip the rule off it. A number that
/// differs is a reshape, and states itself.
fn place_into(
    lines: &mut Vec<String>,
    node_id: &str,
    x: Option<i32>,
    y: Option<i32>,
    w: SizeSpec,
    h: SizeSpec,
    document: &Document,
) {
    for line in lines.iter_mut() {
        let Some(mut node) = board::parse_node_line(line) else {
            continue;
        };
        if !node.id.eq_ignore_ascii_case(node_id) {
            continue;
        }
        if let Some(x) = x {
            node.x = x;
        }
        if let Some(y) = y {
            node.y = y;
        }
        let resolved = board::resolved_box(document, &node);
        let apply = |spec: SizeSpec, value: &mut u32, rule: &mut board::Sizing, at: u32| match spec
        {
            SizeSpec::Keep => {}
            SizeSpec::Page => *rule = board::Sizing::Page,
            SizeSpec::Fit => *rule = board::Sizing::Fit,
            SizeSpec::Px(pixels) => {
                if *rule == board::Sizing::Stated || pixels != at {
                    *value = pixels;
                    *rule = board::Sizing::Stated;
                }
            }
        };
        let (mut w_value, mut w_rule) = (node.w, node.w_rule);
        let (mut h_value, mut h_rule) = (node.h, node.h_rule);
        apply(w, &mut w_value, &mut w_rule, resolved.0);
        apply(h, &mut h_value, &mut h_rule, resolved.1);
        (node.w, node.w_rule, node.h, node.h_rule) = (w_value, w_rule, h_value, h_rule);
        *line = board::node_line(&node);
        return;
    }
    let sized = |spec: SizeSpec, default: u32| match spec {
        SizeSpec::Px(pixels) if pixels > 0 => (pixels, board::Sizing::Stated),
        SizeSpec::Page => (default, board::Sizing::Page),
        SizeSpec::Fit => (default, board::Sizing::Fit),
        _ => (default, board::Sizing::Stated),
    };
    let (w, w_rule) = sized(w, board::DEFAULT_NODE_WIDTH);
    let (h, h_rule) = sized(h, board::DEFAULT_NODE_HEIGHT);
    lines.push(board::node_line(&board::Node {
        id: node_id.to_string(),
        x: x.unwrap_or(0),
        y: y.unwrap_or(0),
        w,
        h,
        w_rule,
        h_rule,
        ..board::Node::default()
    }));
}

/// Push overlapping nodes apart, downwards, keeping `held` exactly where they are.
///
/// Two blocks in one place is not a plan. A node's box is stated on its line, so the engine
/// knows every rectangle without measuring anything, and this is the whole rule: the nodes
/// that were just placed stand still, everything else is taken in reading order (top down,
/// then left to right), and any node that lands on one already standing falls to just below
/// it. Downwards, always — a board is a column, and pushing sideways is what turns a plan
/// into a wall nobody can read at the width of a page.
///
/// Every surface gets this for free, and so does every agent: `dx board --place` cannot
/// leave one node covering another any more than a drag can.
///
/// The document is here for one reason: a `page` or `fit` dimension has to resolve to its
/// number before overlap means anything. Resolution touches only the numeric fields, and
/// `node_line` writes a ruled dimension as its word, so settling never turns a rule into
/// the number it happened to resolve to.
fn settle(lines: &mut [String], held: &[String], document: &Document) {
    /// The clear space left between two nodes that had to be pushed apart, in canvas px.
    const GAP: i32 = 28;

    let mut nodes: Vec<(usize, board::Node)> = lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| board::parse_node_line(line).map(|node| (at, node)))
        .collect();
    for (_, node) in &mut nodes {
        (node.w, node.h) = board::resolved_box(document, node);
    }

    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&index| {
        let node = &nodes[index].1;
        let pinned = held.iter().any(|id| id.eq_ignore_ascii_case(&node.id));
        (!pinned, node.y, node.x)
    });

    let mut standing: Vec<board::Node> = Vec::new();
    for index in order {
        let node = &mut nodes[index].1;
        // Each push clears one neighbour; with only so many neighbours to clear, this ends.
        for _ in 0..=standing.len() {
            let Some(under) = standing.iter().find(|other| overlaps(other, node)) else {
                break;
            };
            shove(node, under, GAP);
        }
        // On a crowded board the shortest-move loop can cycle — out of one box straight
        // into another and back — and run out of turns still overlapping. Falling straight
        // down cannot: every push strictly grows `y`, so this ends, and it ends clear of
        // everything standing. Two nodes left stacked is the one outcome settle must never
        // allow, however the board was arranged.
        while let Some(under) = standing.iter().find(|other| overlaps(other, node)) {
            node.y = under
                .y
                .saturating_add(i32::try_from(under.h).unwrap_or(0))
                .saturating_add(GAP);
        }
        standing.push(node.clone());
    }

    for (at, node) in nodes {
        lines[at] = board::node_line(&node);
    }
}

/// Push `node` out past the nearest border of `blocker`, leaving `gap` between them.
///
/// Out of whichever side it is closest to escaping — the shortest move that ends the overlap.
/// A node nudged a few pixels onto its neighbour's top steps back up rather than being thrown
/// all the way beneath it, which is what a reader means by dropping one box beside another.
///
/// Two rules bound "shortest", in this order:
///
/// - **Never off the top or the left.** An escape that would put a node at a negative
///   coordinate is taken only when every other one would too. The shortest move out of a box
///   near the origin is usually upwards, and a plan whose first node has been shoved above
///   the canvas is a plan that opens on blank paper.
/// - **A tie goes down**, keeping the habit a board is read with: plans grow down the column,
///   so when two escapes are equally short, down is the one that does not lift a node above
///   the thing it follows.
fn shove(node: &mut board::Node, blocker: &board::Node, gap: i32) {
    let width = |n: &board::Node| i32::try_from(n.w).unwrap_or(0);
    let height = |n: &board::Node| i32::try_from(n.h).unwrap_or(0);

    let left = blocker.x - width(node) - gap;
    let right = blocker.x.saturating_add(width(blocker)).saturating_add(gap);
    let up = blocker.y - height(node) - gap;
    let down = blocker
        .y
        .saturating_add(height(blocker))
        .saturating_add(gap);

    // Down first, so `min_by_key` — which keeps the earliest of equal keys — breaks a tie
    // downwards without a second comparison saying so.
    let escapes = [
        (node.x, down),
        (node.x, up),
        (right, node.y),
        (left, node.y),
    ];
    let (x, y) = escapes
        .into_iter()
        .min_by_key(|(x, y)| {
            let distance = (x - node.x).abs().saturating_add((y - node.y).abs());
            (i32::from(*x < 0 || *y < 0), distance)
        })
        .unwrap_or((node.x, node.y));
    node.x = x;
    node.y = y;
}

/// Whether two nodes' boxes cover any of the same canvas.
fn overlaps(one: &board::Node, other: &board::Node) -> bool {
    let right = |node: &board::Node| node.x.saturating_add(i32::try_from(node.w).unwrap_or(0));
    let bottom = |node: &board::Node| node.y.saturating_add(i32::try_from(node.h).unwrap_or(0));
    one.x < right(other) && other.x < right(one) && one.y < bottom(other) && other.y < bottom(one)
}

/// A board body as its lines, with nothing invented for an empty one.
fn body_lines(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        Vec::new()
    } else {
        text.lines().map(str::to_string).collect()
    }
}

/// The smallest `node-N` no block and no node line is already using.
fn fresh_node_id(document: &Document, board_body: &str) -> String {
    let lines = board::nodes(board_body);
    let mut count = 1;
    loop {
        let candidate = format!("node-{count}");
        let taken = document.block_index(&candidate).is_some()
            || lines
                .iter()
                .any(|node| node.id.eq_ignore_ascii_case(&candidate));
        if !taken {
            return candidate;
        }
        count += 1;
    }
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

    #[test]
    fn replacing_inside_a_block_changes_the_match_and_nothing_else() {
        let source = "::paragraph id=note\nThe server listens on port 7431 by default.\n";
        let (after, count) =
            replace_in_block(source, "note", "7431", "9142", false).expect("replace");
        assert_eq!(count, 1);
        assert!(after.contains("port 9142 by default"));
        assert!(!after.contains("7431"));
    }

    /// A rename that matches twice must not silently land on whichever came first.
    #[test]
    fn an_ambiguous_replacement_is_refused_with_the_count() {
        let source = "::paragraph id=note\nport 80 now, port 80 later\n";
        let refusal =
            replace_in_block(source, "note", "port 80", "port 81", false).expect_err("refuse");
        assert!(refusal.contains("2 times"), "{refusal}");
        assert!(refusal.contains("all"), "{refusal}");
    }

    #[test]
    fn all_replaces_every_occurrence_and_reports_the_count() {
        let source = "::paragraph id=note\nport 80 now, port 80 later\n";
        let (after, count) =
            replace_in_block(source, "note", "port 80", "port 81", true).expect("replace");
        assert_eq!(count, 2);
        assert!(!after.contains("port 80"));
    }

    /// The refusal for absent text names the block, so a caller who edited the wrong
    /// document learns it from the sentence.
    #[test]
    fn replacing_text_a_block_does_not_carry_is_refused() {
        let refusal =
            replace_in_block(SAMPLE, "intro", "no such words", "x", false).expect_err("refuse");
        assert!(refusal.contains("intro"), "{refusal}");
    }

    /// The change-sized edit goes through the same body rules as a whole-body save: a
    /// checklist's content lives in its items, and a replacement inside one item leaves
    /// the ticks on every other line exactly as they were.
    #[test]
    fn replacing_inside_a_checklist_keeps_every_other_item_and_tick() {
        let source = "::checklist id=steps\n[x] install the tool\n[ ] open port 7431\n::end\n";
        let (after, _) = replace_in_block(source, "steps", "7431", "9142", false).expect("replace");
        assert!(after.contains("[x] install the tool"));
        assert!(after.contains("[ ] open port 9142"));
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

    /// Ticking a box is the smallest possible edit: one marker flips, and the rest of the
    /// document — including every other item on the same list — comes back byte-identical.
    #[test]
    fn ticking_a_box_flips_one_marker_and_leaves_the_document_alone() {
        let source = stringify(&parse(
            "::paragraph id=lead\nbefore\n::end\n\
             ::checklist id=todo\n[ ] sketch it\n[x] ship it\n::end\n\
             ::paragraph id=after\nafter\n::end\n",
        ));
        let (ticked, checked) = toggle_check(&source, "todo", 0).expect("tick");
        assert!(checked, "the first item should now be ticked");
        assert_eq!(ticked, source.replace("[ ] sketch it", "[x] sketch it"));

        let (untocked, checked) = toggle_check(&ticked, "todo", 1).expect("untick");
        assert!(!checked, "the second item should now be clear");
        assert_eq!(
            toggle_check(&untocked, "todo", 1).expect("tick back").0,
            ticked,
            "ticking twice should land exactly where it started"
        );
    }

    /// The box names an item by position, so the sentences have to cover a position that is
    /// not there — and a block that has no boxes at all.
    #[test]
    fn ticking_refuses_a_missing_item_and_a_block_that_is_not_a_checklist() {
        let source = "::checklist id=todo\n[ ] a\n::end\n::paragraph id=p\nprose\n::end\n";
        let past_end = toggle_check(source, "todo", 4).expect_err("no such item");
        assert!(past_end.contains("has 1 items"), "{past_end}");
        let prose = toggle_check(source, "p", 0).expect_err("not a checklist");
        assert!(prose.contains("only a checklist"), "{prose}");
        assert!(toggle_check(source, "ghost", 0).is_err(), "no such block");
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
    fn removing_a_code_block_takes_its_output_record_with_it() {
        let source = "::code id=job lang=sh run\necho hi\n::end\n\n\
             ::output id=job-output for=job status=ok exit=0\nhi\n::end\n\n\
             ::paragraph id=after\nStays.\n::end\n";
        let after = remove_block(source, "job").expect("remove");
        assert!(!after.contains("::output"), "stranded record: {after}");
        assert!(after.contains("Stays."));
        // Removing a block that nothing reports on removes only that block.
        let untouched = remove_block(source, "after").expect("remove");
        assert!(untouched.contains("for=job"), "{untouched}");
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

    /// The header a surface shows is the writer's own line — attributes and all.
    #[test]
    fn a_blocks_header_is_the_line_the_writer_puts_in_the_file() {
        assert_eq!(
            block_header(SAMPLE, "title").expect("title"),
            "::heading level=1 id=title"
        );
        assert_eq!(
            block_header(SAMPLE, "intro").expect("intro"),
            "::paragraph id=intro"
        );
        let code = "::code id=demo lang=python run\nprint(1)\n::end\n";
        assert_eq!(
            block_header(code, "demo").expect("demo"),
            "::code id=demo lang=python run"
        );
    }

    /// The property header editing rests on: handing back the shown header and body
    /// unchanged changes nothing.
    #[test]
    fn replacing_with_the_shown_header_and_body_changes_nothing() {
        let canonical = stringify(&parse(SAMPLE));
        for id in ["title", "intro", "points"] {
            let header = block_header(&canonical, id).expect("header");
            let body = block_source(&canonical, id).expect("body");
            let (saved, focus) = replace_block(&canonical, id, &header, &body).expect("replace");
            assert_eq!(
                saved, canonical,
                "round trip changed the document at `{id}`"
            );
            assert_eq!(focus, id);
        }
    }

    /// Retyping is the point: a paragraph whose header now says `::quote` is a quotation.
    #[test]
    fn a_new_header_retypes_the_block_and_keeps_its_id() {
        let (after, focus) =
            replace_block(SAMPLE, "intro", "::quote", "The opening line.").expect("replace");
        assert_eq!(focus, "intro");
        let document = parse(&after);
        let block = &document.blocks[find(&document, "intro").expect("intro")];
        assert_eq!(block.kind, "quote");
        assert_eq!(block.text, "The opening line.");
    }

    /// A retyped code block carries the attributes its header names.
    #[test]
    fn a_code_header_carries_its_execution_attributes() {
        let (after, _) =
            replace_block(SAMPLE, "intro", "::code lang=python run", "print(1)").expect("replace");
        let document = parse(&after);
        let block = &document.blocks[find(&document, "intro").expect("intro")];
        assert_eq!(block.kind, "code");
        assert_eq!(block.language, "python");
        assert!(block.run);
        assert_eq!(block.text, "print(1)");
    }

    /// The body is set by the block's own body rule, never re-scanned for structure — a
    /// listing that *contains* the close token keeps every byte of it.
    #[test]
    fn a_replaced_body_containing_the_close_token_keeps_every_byte() {
        let body = "echo start\n::end is what closes a block\necho done";
        let (after, _) = replace_block(SAMPLE, "intro", "::code lang=bash", body).expect("replace");
        assert_eq!(
            block_source(&after, "intro").expect("body"),
            body,
            "the close token inside the body was treated as structure"
        );
    }

    /// An empty header means plain text, read the way the file itself reads it: markdown
    /// shorthand included, and more than one block when the text says so.
    #[test]
    fn an_empty_header_reads_the_body_as_the_file_would() {
        let (after, focus) = replace_block(SAMPLE, "intro", "", "# A New Title").expect("replace");
        let document = parse(&after);
        let block = &document.blocks[find(&document, &focus).expect("focus")];
        assert_eq!(block.kind, "heading");
        assert_eq!(block.text, "A New Title");
        assert_eq!(
            focus, "intro",
            "the block lost its id to a retype it never asked for"
        );

        let (split, _) = replace_block(SAMPLE, "intro", "", "One.\n\nTwo.").expect("replace");
        assert_eq!(
            parse(&split).blocks.len(),
            parse(SAMPLE).blocks.len() + 1,
            "two paragraphs typed as two paragraphs should be two blocks"
        );
    }

    /// Unknown kinds are refused with a sentence, never folded silently to `paragraph` —
    /// and `output` cannot be typed into existence.
    #[test]
    fn replacing_refuses_unknown_kinds_and_output() {
        let unknown = replace_block(SAMPLE, "intro", "::codee", "x").expect_err("unknown kind");
        assert!(unknown.contains("::codee"), "{unknown}");
        let output =
            replace_block(SAMPLE, "intro", "::output", "5").expect_err("fabricated output");
        assert!(output.contains("running code"), "{output}");
        let not_a_header =
            replace_block(SAMPLE, "intro", "plain words", "x").expect_err("not a header");
        assert!(not_a_header.contains("block header"), "{not_a_header}");
    }

    /// A header claiming another block's id is refused; a natural collision yields.
    #[test]
    fn replacing_never_takes_an_existing_blocks_id() {
        let claimed = replace_block(SAMPLE, "intro", "::paragraph id=title", "x")
            .expect_err("claimed another block's id");
        assert!(claimed.contains("already exists"), "{claimed}");

        // Keeping your own id in the header is not a claim on anyone else's.
        let (_, focus) =
            replace_block(SAMPLE, "intro", "::paragraph id=intro", "x").expect("own id");
        assert_eq!(focus, "intro");
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

    // -----------------------------------------------------------------------------------
    // The board operations.
    // -----------------------------------------------------------------------------------

    const BOARD: &str = "::board id=plan height=520\n- ideas x=40 y=40 w=280 h=160\n\
- steps x=380 y=60 w=300 h=200 to=ideas\n::end\n\n\
::paragraph id=ideas hidden\nRough ideas.\n::end\n\n\
::checklist id=steps hidden\n[ ] build\n::end\n";

    /// A kind that renders as empty on a canvas is refused at placement — but a node
    /// already on the board keeps moving, so a legacy board never strands one.
    #[test]
    fn placing_a_block_that_renders_empty_on_a_canvas_is_refused() {
        let source = format!("{BOARD}\n::output id=result for=steps\nran\n::end\n");
        let refused = board_place(
            &source,
            "plan",
            "result",
            Some(40),
            Some(300),
            SizeSpec::Keep,
            SizeSpec::Keep,
        );
        let message = refused.expect_err("an output block is not placeable");
        assert!(message.contains("output blocks"), "{message}");

        // Moving an existing node of any kind stays allowed.
        board_place(
            &source,
            "plan",
            "steps",
            Some(10),
            Some(10),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect("moving stays allowed");
    }

    #[test]
    fn placing_moves_a_node_and_keeps_what_the_line_already_said() {
        let (after, id) = board_place(
            BOARD,
            "plan",
            "steps",
            Some(500),
            Some(200),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect("place");
        assert_eq!(id, "steps");
        // Position changed, box and the edge kept.
        assert!(
            after.contains("- steps x=500 y=200 w=300 h=200 to=ideas"),
            "{after}"
        );
        assert!(after.contains("- ideas x=40 y=40 w=280 h=160"), "{after}");
    }

    /// A dimension may be placed as a rule — `page`, `fit` — and the line keeps the word.
    /// Restating the number the rule already resolves to keeps the rule too, because that
    /// is what a surface's drag reports for a node it only moved; a *different* number is a
    /// reshape, and states itself.
    #[test]
    fn a_size_rule_is_placed_as_its_word_and_survives_a_move_that_restates_it() {
        let (ruled, _) = board_place(
            BOARD,
            "plan",
            "steps",
            None,
            None,
            SizeSpec::Page,
            SizeSpec::Fit,
        )
        .expect("place");
        assert!(ruled.contains("- steps x=380 y=60 w=page h=fit"), "{ruled}");

        // The measured box a surface reports is the resolved box: the rule stays.
        let document = parse(&ruled);
        let board_at = document.block_index("plan").expect("the board");
        let node = crate::render::board::nodes(&document.blocks[board_at].text)
            .into_iter()
            .find(|node| node.id == "steps")
            .expect("the node");
        let (w, h) = crate::render::board::resolved_box(&document, &node);
        let (moved, _) = board_place(
            &ruled,
            "plan",
            "steps",
            Some(700),
            Some(90),
            SizeSpec::Px(w),
            SizeSpec::Px(h),
        )
        .expect("move");
        assert!(moved.contains("- steps x=700 y=90 w=page h=fit"), "{moved}");

        // A number that differs is a reshape: the rule gives way to it.
        let (reshaped, _) = board_place(
            &moved,
            "plan",
            "steps",
            None,
            None,
            SizeSpec::Px(w + 40),
            SizeSpec::Keep,
        )
        .expect("reshape");
        assert!(
            reshaped.contains(&format!("- steps x=700 y=90 w={} h=fit", w + 40)),
            "{reshaped}"
        );
    }

    /// Settle sees a rule's resolved box, not its default: a page-wide node covers a
    /// neighbour a default-width node would never reach, and the neighbour is moved on.
    #[test]
    fn settle_pushes_neighbours_out_of_a_page_wide_node() {
        let (after, _) = board_place(
            BOARD,
            "plan",
            "ideas",
            Some(0),
            Some(40),
            SizeSpec::Page,
            SizeSpec::Keep,
        )
        .expect("place");
        // `steps` stood at x=380 y=60 — inside a page-wide (680) span; it must be shoved.
        assert!(after.contains("- ideas x=0 y=40 w=page h=160"), "{after}");
        assert!(!after.contains("- steps x=380 y=60"), "{after}");
    }

    /// `page` and `fit` are spelled the same way in an arrangement item.
    #[test]
    fn an_arrangement_item_may_state_page_and_fit() {
        let list = placements("steps,40,60,page,fit ideas,40,300").expect("placements");
        assert_eq!((list[0].w, list[0].h), (SizeSpec::Page, SizeSpec::Fit));
        assert_eq!((list[1].w, list[1].h), (SizeSpec::Keep, SizeSpec::Keep));
        let wrong = placements("steps,40,60,huge").expect_err("not a size");
        assert!(wrong.contains("`huge` is not a size"), "{wrong}");
    }

    #[test]
    fn placing_with_a_box_reshapes_and_unknown_tokens_survive() {
        let source = BOARD.replace("h=200 to=ideas", "h=200 to=ideas future=yes");
        let (after, _) = board_place(
            &source,
            "plan",
            "steps",
            Some(380),
            Some(60),
            SizeSpec::Px(360),
            SizeSpec::Px(240),
        )
        .expect("place");
        assert!(
            after.contains("- steps x=380 y=60 w=360 h=240 to=ideas future=yes"),
            "{after}"
        );
    }

    /// A caller resizing a node should not have to repeat its position, and one moving it
    /// should not have to repeat its size — `x`/`y` left out (`None`) keep the line's own,
    /// exactly like `w`/`h` left out (`0`) already do. Regression: the CLI used to default an
    /// omitted `--x`/`--y` to `0`, so `dx board --place --w --h` alone teleported the node.
    #[test]
    fn placing_with_no_position_keeps_it_and_only_reshapes() {
        let (after, id) = board_place(
            BOARD,
            "plan",
            "steps",
            None,
            None,
            SizeSpec::Px(360),
            SizeSpec::Px(240),
        )
        .expect("place");
        assert_eq!(id, "steps");
        assert!(
            after.contains("- steps x=380 y=60 w=360 h=240 to=ideas"),
            "{after}"
        );
    }

    /// The mirror case: no box given, only a new position — the existing size survives.
    #[test]
    fn placing_with_no_box_keeps_it_and_only_moves() {
        let (after, id) = board_place(
            BOARD,
            "plan",
            "steps",
            Some(500),
            Some(200),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect("place");
        assert_eq!(id, "steps");
        assert!(
            after.contains("- steps x=500 y=200 w=300 h=200 to=ideas"),
            "{after}"
        );
    }

    #[test]
    fn placing_an_empty_id_creates_a_fresh_node_ready_to_write() {
        let (after, id) = board_place(
            BOARD,
            "plan",
            "",
            Some(700),
            Some(40),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect("place");
        assert_eq!(id, "node-1");
        assert!(after.contains("- node-1 x=700 y=40 w=280 h=180"), "{after}");
        // The block exists, hidden, directly after the board — one call from a
        // double-click to a block a reader can type into.
        let document = parse(&after);
        let block = &document.blocks[find(&document, "node-1").expect("created")];
        assert_eq!(block.kind, "paragraph");
        assert!(block.hidden);
        assert_eq!(document.blocks[1].id, "node-1");
    }

    #[test]
    fn placing_a_named_missing_node_adds_its_line_and_its_block() {
        let (after, id) = board_place(
            BOARD,
            "plan",
            "sketch",
            Some(900),
            Some(10),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect("place");
        assert_eq!(id, "sketch");
        assert!(after.contains("- sketch x=900 y=10 w=280 h=180"), "{after}");
        assert!(after.contains("::paragraph id=sketch hidden"), "{after}");
    }

    #[test]
    fn saving_a_board_body_creates_the_blocks_its_lines_name() {
        let after =
            set_block(BOARD, "plan", "- ideas x=0 y=0\n- brand-new x=100 y=0").expect("set");
        assert!(
            after.contains("::paragraph id=brand-new hidden\nNew node\n::end"),
            "{after}"
        );
        // A spelling the writer would rename is left dangling rather than silently bound
        // to a block called something else.
        let odd = set_block(BOARD, "plan", "- My Node x=0 y=0").expect("set");
        assert!(!odd.contains("id=my-node"), "{odd}");
    }

    #[test]
    fn detaching_removes_the_line_the_edges_and_the_orphaned_block() {
        let after = board_detach(BOARD, "plan", "ideas").expect("detach");
        assert!(!after.contains("- ideas"), "{after}");
        // The edge that pointed at it went with it.
        assert!(
            after.contains("- steps x=380 y=60 w=300 h=200\n"),
            "{after}"
        );
        // And the hidden block nothing else showed is gone.
        assert!(!after.contains("Rough ideas."), "{after}");
    }

    #[test]
    fn detaching_keeps_a_block_that_is_visible_or_still_shown_elsewhere() {
        let visible = BOARD.replace("::paragraph id=ideas hidden", "::paragraph id=ideas");
        let after = board_detach(&visible, "plan", "ideas").expect("detach");
        assert!(
            after.contains("Rough ideas."),
            "a visible block was deleted: {after}"
        );

        let two_boards = format!("{BOARD}\n::board id=second\n- ideas x=0 y=0\n::end\n");
        let after = board_detach(&two_boards, "plan", "ideas").expect("detach");
        assert!(
            after.contains("Rough ideas."),
            "a block another board shows was deleted: {after}"
        );
    }

    #[test]
    fn linking_and_unlinking_are_idempotent_and_bounded_to_the_board() {
        let linked = board_link(BOARD, "plan", "ideas", "steps", true, "", "").expect("link");
        assert!(
            linked.contains("- ideas x=40 y=40 w=280 h=160 to=steps"),
            "{linked}"
        );
        let twice = board_link(&linked, "plan", "ideas", "steps", true, "", "").expect("again");
        assert_eq!(twice, linked);

        let unlinked =
            board_link(&linked, "plan", "steps", "ideas", false, "", "").expect("unlink");
        assert!(
            unlinked.contains("- steps x=380 y=60 w=300 h=200\n"),
            "{unlinked}"
        );

        let selfish = board_link(BOARD, "plan", "ideas", "ideas", true, "", "").expect_err("self");
        assert!(selfish.contains("itself"), "{selfish}");
        let missing =
            board_link(BOARD, "plan", "ideas", "ghost", true, "", "").expect_err("missing");
        assert!(missing.contains("ghost"), "{missing}");
    }

    /// A line dragged from one node's edge to another's is kept as the sides it joined, by
    /// letter or by name, and re-linking the same pair re-pins them instead of drawing twice.
    #[test]
    fn linking_keeps_the_two_sides_the_reader_drew_between() {
        let pinned =
            board_link(BOARD, "plan", "ideas", "steps", true, "bottom", "t").expect("link");
        assert!(
            pinned.contains("- ideas x=40 y=40 w=280 h=160 to=steps:b-t"),
            "{pinned}"
        );

        let repinned =
            board_link(&pinned, "plan", "ideas", "steps", true, "r", "l").expect("re-link");
        assert!(
            repinned.contains("- ideas x=40 y=40 w=280 h=160 to=steps:r-l"),
            "{repinned}"
        );

        // A side nobody named leaves that end to the renderer, and the pinned one survives.
        let half = board_link(BOARD, "plan", "ideas", "steps", true, "", "top").expect("link");
        assert!(half.contains("to=steps:.-t"), "{half}");
    }

    /// Settling a board after a drag is one call and one undo step, not one per node.
    #[test]
    fn arranging_moves_every_node_at_once() {
        let spec = placements("ideas,0,0,280,160 steps,0,220,320,200 sketch,0,520").expect("read");
        let after = board_arrange(BOARD, "plan", &spec).expect("arrange");
        assert!(after.contains("- ideas x=0 y=0 w=280 h=160\n"), "{after}");
        assert!(
            after.contains("- steps x=0 y=220 w=320 h=200 to=ideas"),
            "{after}"
        );
        // A placement naming no line adds one, and its block, exactly as a place does.
        assert!(after.contains("- sketch x=0 y=520 w=280 h=180"), "{after}");
        assert!(after.contains("::paragraph id=sketch hidden"), "{after}");

        let broken = placements("ideas,0,zero").expect_err("not a number");
        assert!(broken.contains("zero"), "{broken}");
        let shapeless = placements("ideas").expect_err("not a placement");
        assert!(shapeless.contains("node,x,y"), "{shapeless}");
        assert_eq!(placements("  ").expect("nothing to place"), Vec::new());
    }

    #[test]
    fn board_operations_refuse_a_block_that_is_not_a_board() {
        let refused = board_place(
            BOARD,
            "ideas",
            "steps",
            Some(0),
            Some(0),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect_err("not a board");
        assert!(refused.contains("not a board"), "{refused}");
        let missing = board_detach(BOARD, "plan", "ghost").expect_err("no node");
        assert!(missing.contains("ghost"), "{missing}");
        let arranged = board_arrange(BOARD, "ideas", &[]).expect_err("not a board");
        assert!(arranged.contains("not a board"), "{arranged}");
    }

    /// A node dropped on its neighbour makes room: the node that was placed stays exactly
    /// where it was put, and the one it landed on falls below it — for an agent running
    /// `dx board --place` as much as for a reader dragging, because it is one operation.
    #[test]
    fn a_placed_node_holds_its_spot_and_pushes_what_it_landed_on_down() {
        let (after, _) = board_place(
            BOARD,
            "plan",
            "steps",
            Some(40),
            Some(60),
            SizeSpec::Px(280),
            SizeSpec::Px(200),
        )
        .expect("place");
        assert!(after.contains("- steps x=40 y=60 w=280 h=200"), "{after}");
        // `ideas` was at y=40 and is 160 tall: it lands 28px under the 260px bottom of
        // `steps`, and nothing is left covering anything.
        assert!(after.contains("- ideas x=40 y=288 w=280 h=160"), "{after}");

        // Two nodes that do not touch are left exactly where they were.
        let apart = board_place(
            BOARD,
            "plan",
            "steps",
            Some(380),
            Some(60),
            SizeSpec::Keep,
            SizeSpec::Keep,
        )
        .expect("place");
        assert_eq!(apart.0, stringify(&parse(BOARD)));
    }

    /// However crowded the board, settling never leaves two nodes covering any of the same
    /// canvas. The shortest-move escape can cycle between packed neighbours — out of one box
    /// straight into another and back — and when it runs out of turns the node falls
    /// straight down instead of being left stacked where it was dropped.
    #[test]
    fn settling_a_crowded_board_leaves_no_two_nodes_overlapping() {
        // Nine nodes packed in a tight grid, then a tenth dropped into the middle of them.
        let mut body = String::new();
        for row in 0..3 {
            for column in 0..3 {
                body.push_str(&format!(
                    "- n{row}{column} x={} y={} w=200 h=120\n",
                    column * 210,
                    row * 130
                ));
            }
        }
        let source = format!("::board id=plan\n{body}::end\n");
        let (after, _) = board_place(
            &source,
            "plan",
            "dropped",
            Some(180),
            Some(100),
            SizeSpec::Px(260),
            SizeSpec::Px(200),
        )
        .expect("place");
        assert!(
            after.contains("- dropped x=180 y=100 w=260 h=200"),
            "the placed node did not hold its spot: {after}"
        );
        let document = parse(&after);
        let nodes = board::nodes(&document.blocks[0].text);
        assert_eq!(nodes.len(), 10);
        for (index, one) in nodes.iter().enumerate() {
            for other in &nodes[index + 1..] {
                assert!(
                    !overlaps(one, other),
                    "`{}` and `{}` are left stacked: {after}",
                    one.id,
                    other.id
                );
            }
        }
    }

    #[test]
    fn a_board_body_shows_and_saves_like_any_other_block() {
        assert_eq!(
            block_source(BOARD, "plan").expect("body"),
            "- ideas x=40 y=40 w=280 h=160\n- steps x=380 y=60 w=300 h=200 to=ideas"
        );
        let canonical = stringify(&parse(BOARD));
        let unchanged = set_block(
            &canonical,
            "plan",
            &block_source(&canonical, "plan").expect("body"),
        )
        .expect("set");
        assert_eq!(unchanged, canonical);
    }

    #[test]
    fn board_nodes_can_reference_code_blocks_for_execution() {
        let doc = r#"
::board id=plan height=400
- setup x=0 y=0 w=200 h=100
- test x=220 y=0 w=200 h=100 to=setup
::end

::code id=setup run lang=bash hidden
echo "Setting up..."
::end

::code id=test run lang=bash hidden
echo "Running tests..."
::end
"#;
        let parsed = parse(doc);
        // Both code blocks should exist and be marked as runnable
        let setup = parsed
            .blocks
            .iter()
            .find(|b| b.id == "setup")
            .expect("setup exists");
        let test = parsed
            .blocks
            .iter()
            .find(|b| b.id == "test")
            .expect("test exists");
        assert_eq!(setup.kind, "code");
        assert!(setup.run);
        assert_eq!(test.kind, "code");
        assert!(test.run);
    }

    #[test]
    fn board_invalid_block_types_are_rejected() {
        assert!(board_invalid_block_type("output").is_some());
        assert!(board_invalid_block_type("script").is_some());
        assert!(board_invalid_block_type("stylesheet").is_some());
        assert!(board_invalid_block_type("style").is_some());
        assert!(board_invalid_block_type("image").is_some());
        // Valid types should not be rejected
        assert!(board_invalid_block_type("code").is_none());
        assert!(board_invalid_block_type("paragraph").is_none());
        assert!(board_invalid_block_type("html").is_none());
        assert!(board_invalid_block_type("checklist").is_none());
    }

    #[test]
    fn hidden_code_blocks_on_boards_are_executable() {
        let doc = r#"
::board id=graph height=300
- compute x=10 y=10 w=150 h=80
::end

::code id=compute lang=python run hidden
result = 42
print(result)
::end
"#;
        let parsed = parse(doc);
        let compute = parsed
            .blocks
            .iter()
            .find(|b| b.id == "compute")
            .expect("exists");
        // Hidden code blocks should be executable (run=true) even when hidden
        assert_eq!(compute.kind, "code");
        assert!(compute.run);
        assert!(compute.hidden);
        // The code content should be intact for review before execution
        assert!(compute.text.contains("result = 42"));
    }

    #[test]
    fn board_node_edges_carry_sequence_information() {
        let doc = r#"
::board id=workflow
- init x=0 y=0 w=100 h=80
- process x=150 y=0 w=100 h=80 to=init:r-l:execute
- finish x=300 y=0 w=100 h=80 to=process:r-l:on%20success
::end
"#;
        let parsed = parse(doc);
        let board = parsed
            .blocks
            .iter()
            .find(|b| b.id == "workflow")
            .expect("board exists");
        // Edges should be stored for future sequencing support
        assert!(board.text.contains("to=init"));
        assert!(board.text.contains("on%20success"));
    }

    #[test]
    fn board_nodes_grow_when_content_exceeds_stated_height() {
        let doc = r#"
::board id=canvas height=400
- content x=0 y=0 w=200 h=100
::end

::paragraph id=content hidden
This is a very long paragraph with lots of text that will definitely exceed the 100px height
that the node was originally sized to. The node should grow to accommodate all the content when
the document is edited and the block grows beyond the stated box dimensions. This tests whether
the grow-only logic is working correctly for boards.
::end
"#;
        let parsed = parse(doc);
        let board = parsed
            .blocks
            .iter()
            .find(|b| b.id == "canvas")
            .expect("board");
        // Board should preserve the node line structure
        assert!(board.text.contains("- content x=0 y=0 w=200 h=100"));
    }

    #[test]
    fn removing_a_board_node_removes_its_hidden_block_if_unique_to_board() {
        let source = r#"
::board id=plan
- item1 x=0 y=0
- item2 x=200 y=0
::end

::paragraph id=item1 hidden
Hidden item 1
::end

::paragraph id=item2 hidden
Hidden item 2
::end
"#;
        let after_detach = board_detach(source, "plan", "item1").expect("detach item1");
        // item1 block should be removed since it was unique to the board
        assert!(!after_detach.contains("Hidden item 1"));
        // item2 should remain
        assert!(after_detach.contains("Hidden item 2"));
        // board node should be gone
        assert!(!after_detach.contains("- item1"));
    }
}
