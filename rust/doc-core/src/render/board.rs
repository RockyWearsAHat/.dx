//! The board: this document's own blocks, arranged on a canvas instead of down the page.
//!
//! A `::board` block is a planning surface — a node editor in the Blender sense, drawn on
//! the sheet. Its body is one reference line per node:
//!
//! ```text
//! - ideas x=40 y=40 w=280 h=160
//! - sketch x=40 y=260 w=320 h=220 to=ideas:t-b
//! ```
//!
//! Each line names a sibling block of the *same document* (usually one marked `hidden`, so
//! it appears on the board and not in the page flow) and says where it sits. Like a `nav`,
//! a board is resolved from the document it sits in and never by reading another file —
//! rendering stays a pure function of the document.
//!
//! An edge names the two sides it joins (`to=sketch:b-t` — out of the bottom, into the
//! top), because on a board a reader drags a line from the edge of one node to the edge of
//! another and that choice is part of the plan. A bare `to=sketch` is unpinned: the edge
//! takes whichever pair of facing sides reads best, so every board written before sides
//! existed still draws.
//!
//! **A node's box is stated, never guessed.** `x y w h` are the whole of where a node is
//! and how big it is, the node is drawn at exactly that size, and every consumer — this
//! renderer, `crate::edit`, an editing surface, an agent running `dx board` — works from the
//! same four numbers. A board is therefore laid out identically by a browser, a PNG, and a
//! terminal, and nothing has to measure content to know where an edge meets a node or
//! whether two nodes overlap.
//!
//! The board renders as a fixed-height viewport that clips its canvas, so however large the
//! arrangement grows it can never push past the side of the document. The canvas is fitted
//! — scaled on **both** axes so the whole arrangement is visible from the first glance — and
//! an editing surface adds pan and zoom on top of the same fit. A render nobody can click (a
//! PNG, a page on github.com) already shows everything there is.
//!
//! This module owns the node-line grammar in both directions ([`nodes`] to read,
//! [`node_line`] to write) so `crate::edit`'s board operations and this renderer can never
//! disagree about what a line says — and no editing surface has to grow a copy of it. It
//! owns the *geometry* the same way: which side an edge leaves, how several edges sharing
//! one side spread along it, and the curve between them are stated here once and mirrored
//! by the surface against measured boxes.

use super::escape::escape_html;
use super::html::{block_html, HtmlOptions};
use super::template::Values;
use crate::model::{Block, Document};

/// Width in canvas pixels of a node whose line names none.
pub(crate) const DEFAULT_NODE_WIDTH: u32 = 280;

/// Height in canvas pixels of a node whose line names none.
pub(crate) const DEFAULT_NODE_HEIGHT: u32 = 180;

/// Viewport height in CSS pixels of a board whose header names none.
pub(crate) const DEFAULT_BOARD_HEIGHT: u32 = 480;

/// What `w=page` resolves to: the page column, so a node written at page width spans the
/// sheet the way a full-bleed figure does.
pub(crate) const PAGE_NODE_WIDTH: u32 = 680;

/// What `h=page` resolves to: the board's own default viewport, so a node written at page
/// height stands as tall as the frame a reader first sees the board through.
pub(crate) const PAGE_NODE_HEIGHT: u32 = DEFAULT_BOARD_HEIGHT;

/// The page column a static render fits to, in CSS pixels: the sheet's `46rem` measure
/// minus its padding, at the browser-default 16px rem. An interactive surface measures the
/// real viewport instead; this only has to be close enough that a static page never clips
/// content sideways. The same measure `w=page` states, so the two cannot disagree.
const STATIC_COLUMN: f64 = PAGE_NODE_WIDTH as f64;

/// Breathing room between the nodes' bounding box and the viewport's edge, in canvas px.
const FIT_PADDING: f64 = 24.0;

/// The most a fit may magnify. A board holding three small nodes should use the room it
/// was given rather than huddle in the middle of it, but past this the type stops looking
/// like the page's type and starts looking like a zoom.
const FIT_MAX: f64 = 1.5;

/// A side of a node: where an edge leaves it, and where one arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    /// The left edge; an edge here reaches out to the left.
    Left,
    /// The right edge; an edge here reaches out to the right.
    Right,
    /// The top edge; an edge here reaches upwards.
    Top,
    /// The bottom edge; an edge here reaches downwards.
    Bottom,
}

impl Side {
    /// The letter this side is written as in a node line.
    pub(crate) fn letter(self) -> char {
        match self {
            Side::Left => 'l',
            Side::Right => 'r',
            Side::Top => 't',
            Side::Bottom => 'b',
        }
    }

    /// The side a letter names, or `None` for anything else — including `.`, which is how a
    /// line says "this end is not pinned" while pinning the other.
    fn from_letter(letter: char) -> Option<Side> {
        match letter.to_ascii_lowercase() {
            'l' => Some(Side::Left),
            'r' => Some(Side::Right),
            't' => Some(Side::Top),
            'b' => Some(Side::Bottom),
            _ => None,
        }
    }

    /// True for the sides an edge spreads along horizontally (top and bottom).
    fn horizontal_span(self) -> bool {
        matches!(self, Side::Top | Side::Bottom)
    }

    /// Which way an edge's handle reaches out of this side, as a unit vector.
    fn reach(self) -> (f64, f64) {
        match self {
            Side::Left => (-1.0, 0.0),
            Side::Right => (1.0, 0.0),
            Side::Top => (0.0, -1.0),
            Side::Bottom => (0.0, 1.0),
        }
    }

    /// This side's index, for grouping the endpoints that share it.
    fn slot(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
            Side::Top => 2,
            Side::Bottom => 3,
        }
    }
}

/// One edge out of a node: the node it points at, and the sides it is pinned to.
///
/// `from`/`to` are the sides a reader drew the line between. `None` means unpinned — the
/// renderer picks the facing pair — which is what a bare `to=id` has always meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edge {
    /// Id of the node this edge points at.
    pub id: String,
    /// The side of the source it leaves, when the line pins one.
    pub from: Option<Side>,
    /// The side of the target it arrives on, when the line pins one.
    pub to: Option<Side>,
    /// Words written on the edge — "yes", "on failure" — empty when it carries none.
    ///
    /// A plan's edges are often the part carrying the meaning: a decision with two unlabelled
    /// arrows out of it says nothing about which is which. Written last in the token and
    /// percent-encoded, so an edge without one is byte-for-byte the line it always was.
    pub label: String,
}

impl Edge {
    /// An unpinned edge to `id`: the renderer chooses both of its sides.
    pub(crate) fn unpinned(id: &str) -> Edge {
        Edge {
            id: id.to_string(),
            from: None,
            to: None,
            label: String::new(),
        }
    }
}

/// How one dimension of a node's box is stated on its line.
///
/// A dimension is usually a number, but a line may also say `page` — the page's own measure
/// — or `fit` — sized to the block the node shows. Both resolve to a number by
/// [`resolve_sizes`], deterministically, from nothing but the document: the box is still
/// *stated*, never measured, so every surface draws the same rectangle. The rule survives
/// the round trip ([`node_line`] writes the word back), which is what lets a `fit` node
/// keep tracking its block as the block is rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Sizing {
    /// The line states the number itself.
    #[default]
    Stated,
    /// `page`: the page column for a width, the board's default viewport for a height.
    Page,
    /// `fit`: the engine's estimate of the block's own rendered size.
    Fit,
}

/// One node line of a board body: the block it shows, where it sits, and what it points at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Node {
    /// Id of the block this node displays.
    pub id: String,
    /// Left edge in canvas pixels.
    pub x: i32,
    /// Top edge in canvas pixels.
    pub y: i32,
    /// Width in canvas pixels. When [`Node::w_rule`] is not [`Sizing::Stated`] this holds
    /// the default until [`resolve_sizes`] states the rule's number.
    pub w: u32,
    /// Height in canvas pixels. The node is drawn exactly this tall, so where its edges
    /// meet it — and whether it overlaps its neighbour — is known without measuring
    /// anything.
    pub h: u32,
    /// How the width is stated: a number, the page's measure, or a fit to the block.
    pub w_rule: Sizing,
    /// How the height is stated, the same way.
    pub h_rule: Sizing,
    /// The edges leaving this node, in the order the line writes them.
    pub to: Vec<Edge>,
    /// Tokens this version does not understand, kept verbatim so a rewrite preserves them.
    pub rest: Vec<String>,
}

/// Read a board body into its nodes, in line order. Lines that name no block id are
/// skipped; a malformed coordinate falls back to its default rather than killing the line.
pub(crate) fn nodes(body: &str) -> Vec<Node> {
    body.lines().filter_map(parse_node_line).collect()
}

/// The directed edges every `::board` in `document` states, as `(from, to)` block-id
/// pairs, in board order and then line order.
///
/// An edge on a board says *this, then that* — `- a … to=b` points `a` at `b` — and this
/// is the one place that meaning is read out for consumers beyond the renderer, so a
/// second parser of the node-line grammar never exists (dx run's `--follow-edges` orders
/// execution by these pairs). Targets are returned exactly as the lines state them,
/// including ids the document does not have; a caller decides what a dangling edge means,
/// the same way the renderer tolerates one by not drawing it.
#[must_use]
pub fn board_edges(document: &Document) -> Vec<(String, String)> {
    document
        .blocks
        .iter()
        .filter(|block| block.kind == "board")
        .flat_map(|board| nodes(&board.text))
        .flat_map(|node| {
            node.to
                .iter()
                .map(|edge| (node.id.clone(), edge.id.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Read one node line, or `None` when the line is blank or carries no id.
pub(crate) fn parse_node_line(line: &str) -> Option<Node> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed).trim_start();
    let mut tokens = trimmed.split_whitespace();
    let id = tokens.next()?.to_string();
    if id.is_empty() || id.starts_with('-') {
        return None;
    }

    let mut node = Node {
        id,
        w: DEFAULT_NODE_WIDTH,
        h: DEFAULT_NODE_HEIGHT,
        ..Node::default()
    };
    for token in tokens {
        match token.split_once('=') {
            Some(("x", value)) => node.x = value.parse().unwrap_or(0),
            Some(("y", value)) => node.y = value.parse().unwrap_or(0),
            Some(("w", "page")) => node.w_rule = Sizing::Page,
            Some(("w", "fit")) => node.w_rule = Sizing::Fit,
            Some(("h", "page")) => node.h_rule = Sizing::Page,
            Some(("h", "fit")) => node.h_rule = Sizing::Fit,
            Some(("w", value)) => node.w = value.parse().unwrap_or(DEFAULT_NODE_WIDTH),
            Some(("h", value)) => node.h = value.parse().unwrap_or(DEFAULT_NODE_HEIGHT),
            Some(("to", value)) => {
                node.to = value
                    .split(',')
                    .map(str::trim)
                    .filter(|target| !target.is_empty())
                    .map(parse_edge)
                    .collect();
            }
            _ => node.rest.push(token.to_string()),
        }
    }
    Some(node)
}

/// Read one `to=` target: an id, optionally followed by `:<from>-<to>` naming the sides.
///
/// A side that is not a side letter (or `.`, which pins neither end) is dropped rather than
/// taken literally — a typo in a side leaves an edge that still draws, which is the same
/// courtesy a malformed coordinate gets.
fn parse_edge(token: &str) -> Edge {
    let Some((id, rest)) = token.split_once(':') else {
        return Edge::unpinned(token);
    };
    // `id:<sides>` or `id:<sides>:<label>`. The label is last so a line written before labels
    // existed reads identically.
    let (sides, label) = rest.split_once(':').unwrap_or((rest, ""));
    let mut letters = sides.split('-');
    let from = letters.next().and_then(one_side);
    let to = letters.next().and_then(one_side);
    Edge {
        id: id.to_string(),
        from,
        to,
        label: decode_label(label),
    }
}

/// Read a percent-encoded edge label back into its text.
///
/// Anything that is not a well-formed `%XX` pair is kept as written: a label is prose, and a
/// stray `%` in it must survive rather than swallow the two characters after it.
fn decode_label(encoded: &str) -> String {
    let mut out = String::with_capacity(encoded.len());
    let bytes = encoded.as_bytes();
    let mut index = 0;
    let mut pending: Vec<u8> = Vec::new();
    while index < bytes.len() {
        let decoded = (bytes[index] == b'%' && index + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[index + 1..index + 3]).ok())
            .flatten()
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match decoded {
            Some(byte) => {
                pending.push(byte);
                index += 3;
            }
            None => {
                flush(&mut pending, &mut out);
                out.push_str(&encoded[index..index + char_width(bytes[index])]);
                index += char_width(bytes[index]);
            }
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Turn the bytes gathered from a `%XX` run into text, keeping them literal if they are not
/// valid UTF-8 — a label is never worth losing to an encoding mistake.
fn flush(pending: &mut Vec<u8>, out: &mut String) {
    if pending.is_empty() {
        return;
    }
    match std::str::from_utf8(pending) {
        Ok(text) => out.push_str(text),
        Err(_) => {
            for byte in pending.iter() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    pending.clear();
}

/// How many bytes the UTF-8 character starting with `lead` occupies.
fn char_width(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Write an edge label so it survives a node line: no whitespace, no `,`, and no `:`.
///
/// Percent-encoding, because those three characters are the line's own grammar and a label
/// that contained one would silently become a second edge, a second side, or a truncated
/// line.
fn encode_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for byte in label.as_bytes() {
        let plain = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if plain {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The side a one-letter word names, or `None` for `.`, an empty word, or anything longer.
fn one_side(word: &str) -> Option<Side> {
    let mut letters = word.chars();
    let letter = letters.next()?;
    if letters.next().is_some() {
        return None;
    }
    Side::from_letter(letter)
}

/// The side a caller names, by letter or by word — `l`, `left`, `t`, `top`, and so on.
///
/// This is the spelling every *host* uses: a link drawn in an editor, a `dx board --link`
/// run by an agent, and a node line in the document all end up as the same [`Side`], so
/// there is one vocabulary for which edge of a node a line meets.
pub(crate) fn side_named(word: &str) -> Option<Side> {
    match word.trim().to_ascii_lowercase().as_str() {
        "l" | "left" => Some(Side::Left),
        "r" | "right" => Some(Side::Right),
        "t" | "top" => Some(Side::Top),
        "b" | "bottom" => Some(Side::Bottom),
        _ => None,
    }
}

/// Write one edge as its `to=` target: the id, and the sides when the edge pins any.
///
/// An unpinned end inside a pinned edge is written `.`, so the pinned one survives the
/// round trip — `to=steps:.-t` is "into the top of steps, out of wherever suits".
fn edge_token(edge: &Edge) -> String {
    let label = edge.label.trim();
    match (edge.from, edge.to, label.is_empty()) {
        (None, None, true) => edge.id.clone(),
        (from, to, _) => {
            let sides = format!(
                "{}:{}-{}",
                edge.id,
                from.map_or('.', Side::letter),
                to.map_or('.', Side::letter),
            );
            if label.is_empty() {
                sides
            } else {
                format!("{sides}:{}", encode_label(label))
            }
        }
    }
}

/// Write one node as its canonical body line — the one [`parse_node_line`] reads back.
///
/// Geometry is always explicit (a rewritten line should say where the node is, not make a
/// reader remember the defaults), edges only when there are any, and tokens this version
/// did not understand come along at the end, untouched. A dimension stated as a rule is
/// written as its word — `w=page`, `h=fit` — never as the number it happened to resolve to,
/// so the rule keeps applying as the document changes around it.
pub(crate) fn node_line(node: &Node) -> String {
    let dimension = |rule: Sizing, value: u32| match rule {
        Sizing::Stated => value.to_string(),
        Sizing::Page => "page".to_string(),
        Sizing::Fit => "fit".to_string(),
    };
    let mut line = format!(
        "- {} x={} y={} w={} h={}",
        node.id,
        node.x,
        node.y,
        dimension(node.w_rule, node.w),
        dimension(node.h_rule, node.h),
    );
    if !node.to.is_empty() {
        let targets: Vec<String> = node.to.iter().map(edge_token).collect();
        line.push_str(&format!(" to={}", targets.join(",")));
    }
    for token in &node.rest {
        line.push(' ');
        line.push_str(token);
    }
    line
}

// ---------------------------------------------------------------------------------------
// Resolving `page` and `fit` into the numbers every consumer works from.
//
// A rule is still a *stated* box: it resolves from nothing but the document — the theme's
// own metrics for `fit`, a named constant for `page` — so the renderer, `crate::edit`, an
// editing surface, and an agent running `dx board` all arrive at the identical rectangle
// without measuring anything. `fit` is an estimate of the render, not a measurement of it:
// content the estimate undershoots scrolls inside the node exactly as it would in any
// stated box, and a surface that *can* measure corrects the number through the same
// operations a drag uses.
// ---------------------------------------------------------------------------------------

/// Resolve every node's `page`/`fit` rule into its number, in place.
///
/// Width resolves before height, because a `fit` height is the height of the block wrapped
/// at the node's own width. The numeric fields are safe to overwrite: [`node_line`] writes
/// a ruled dimension as its word, so resolution never leaks into the document.
pub(crate) fn resolve_sizes(document: &Document, nodes: &mut [Node]) {
    for node in nodes {
        let block = document
            .block_index(&node.id)
            .map(|index| &document.blocks[index]);
        match node.w_rule {
            Sizing::Stated => {}
            Sizing::Page => node.w = PAGE_NODE_WIDTH,
            Sizing::Fit => node.w = block.map_or(DEFAULT_NODE_WIDTH, fit_width),
        }
        match node.h_rule {
            Sizing::Stated => {}
            Sizing::Page => node.h = PAGE_NODE_HEIGHT,
            Sizing::Fit => {
                node.h = block.map_or(DEFAULT_NODE_HEIGHT, |block| fit_height(block, node.w));
            }
        }
    }
}

/// One node's resolved box, without touching the node itself — what `w`/`h` would be after
/// [`resolve_sizes`]. This is how `crate::edit` asks whether a stated number is merely a
/// rule's own value restated (a drag that moved a node without reshaping it), so the rule
/// survives edits that did not mean to remove it.
pub(crate) fn resolved_box(document: &Document, node: &Node) -> (u32, u32) {
    let mut copy = [node.clone()];
    resolve_sizes(document, &mut copy);
    (copy[0].w, copy[0].h)
}

// The estimate's metrics, taken from `render::theme`'s own numbers rather than invented:
// the sheet sets 17px type at 1.7 line height, a node pads 0.55rem/0.8rem, and listings run
// 0.83rem mono at 1.65. Coarse on purpose — an average character width stands in for a font
// the engine will never load — and biased slightly tall, because a fit that clips a line is
// a failure while a fit with a breath of room under it is a fit.

/// One wrapped line of body prose, in canvas pixels (17px × 1.7).
const FIT_LINE: f64 = 29.0;
/// One line of a listing (0.83rem mono × 1.65).
const FIT_CODE_LINE: f64 = 23.0;
/// The average width of a body character at 17px serif.
const FIT_CHAR: f64 = 8.2;
/// The width of a listing character at 0.83rem mono.
const FIT_CODE_CHAR: f64 = 8.6;
/// A node's own horizontal padding (0.8rem each side), plus its hairline borders.
const FIT_PAD_X: f64 = 28.0;
/// A node's own vertical padding (0.55rem each side), plus its hairline borders.
const FIT_PAD_Y: f64 = 20.0;
/// The indent a list item's marker takes before its text begins.
const FIT_INDENT: f64 = 26.0;
/// The wider indent of a checklist item: its three-character mono mark plus the gap.
const FIT_CHECK_INDENT: f64 = 40.0;
/// The lead each list item adds over its bare lines (`li { margin: 0.3rem 0 }`), sized to
/// cover the engine with the widest metrics rather than the narrowest.
const FIT_ITEM_LEAD: f64 = 10.0;
/// The narrowest a `w=fit` node may come out: a sliver cannot hold a grip or a sentence.
const FIT_MIN_WIDTH: u32 = 120;
/// The shortest an `h=fit` node may come out: one line under the node's own padding.
const FIT_MIN_HEIGHT: u32 = 56;
/// The tallest an `h=fit` estimate may claim, so a pasted novel cannot stretch a board.
const FIT_MAX_HEIGHT: u32 = 2400;

/// A `::view`'s framed viewport, with the renderer's defaults where the block states none.
fn view_frame(block: &Block) -> (u32, u32) {
    let width = if block.width > 0 {
        block.width
    } else {
        super::html::VIEW_DEFAULT_WIDTH
    };
    let height = if block.height > 0 {
        block.height
    } else {
        super::html::VIEW_DEFAULT_HEIGHT
    };
    (width, height)
}

/// The width `w=fit` states for a block: its longest natural line, capped at the page.
fn fit_width(block: &Block) -> u32 {
    let longest = |lines: &[String], char_width: f64, indent: f64| {
        lines
            .iter()
            .map(|line| line.chars().count() as f64 * char_width + indent)
            .fold(0.0_f64, f64::max)
    };
    let text_lines = |text: &str| -> Vec<String> { text.lines().map(str::to_string).collect() };
    let natural = match block.kind.as_str() {
        "svg" => view_box(&block.text).map_or_else(
            || longest(&text_lines(&block.text), FIT_CHAR, 0.0),
            |(w, _)| w,
        ),
        "code" => longest(&text_lines(&block.text), FIT_CODE_CHAR, 0.0),
        "html" => longest(&stripped_lines(&block.text), FIT_CHAR, 0.0),
        // A view's natural width is its framed viewport, which the page cap below trims.
        "view" => f64::from(view_frame(block).0),
        "bulleted-list" | "numbered-list" | "checklist" => {
            longest(&item_lines(block), FIT_CHAR, FIT_INDENT)
        }
        _ => longest(&text_lines(&block.text), FIT_CHAR, 0.0),
    };
    ((natural + FIT_PAD_X) as u32).clamp(FIT_MIN_WIDTH, PAGE_NODE_WIDTH)
}

/// The height `h=fit` states for a block wrapped at `width` canvas pixels.
fn fit_height(block: &Block, width: u32) -> u32 {
    let inner = (f64::from(width) - FIT_PAD_X).max(FIT_CHAR);
    // How many lines a run of text takes when wrapped at this node's width.
    let wrapped = |text: &str, char_width: f64, indent: f64| -> f64 {
        let columns = ((inner - indent) / char_width).max(8.0);
        text.lines()
            .map(|line| (line.chars().count() as f64 / columns).ceil().max(1.0))
            .sum()
    };
    let estimate = match block.kind.as_str() {
        // A heading is one line of display type; level changes its size, not its count.
        "heading" => 46.0,
        "svg" => {
            view_box(&block.text).map_or(f64::from(DEFAULT_NODE_HEIGHT), |(w, h)| {
                inner * h / w.max(1.0)
            }) + FIT_PAD_Y
        }
        "code" => wrapped(&block.text, FIT_CODE_CHAR, 0.0) * FIT_CODE_LINE + FIT_LINE + FIT_PAD_Y,
        // A view keeps its frame's aspect: the node is the screen, scaled, no padding.
        "view" => {
            let (frame_w, frame_h) = view_frame(block);
            f64::from(width) * f64::from(frame_h) / f64::from(frame_w)
        }
        "html" => {
            let text = stripped_lines(&block.text).join("\n");
            wrapped(&text, FIT_CHAR, 0.0) * FIT_LINE + FIT_PAD_Y
        }
        "bulleted-list" | "numbered-list" | "checklist" => {
            let indent = if block.kind == "checklist" {
                FIT_CHECK_INDENT
            } else {
                FIT_INDENT
            };
            let lines = item_lines(block);
            wrapped(&lines.join("\n"), FIT_CHAR, indent) * FIT_LINE
                + lines.len() as f64 * FIT_ITEM_LEAD
                + FIT_PAD_Y
        }
        _ => wrapped(&block.text, FIT_CHAR, 0.0) * FIT_LINE + FIT_PAD_Y,
    };
    (estimate as u32).clamp(FIT_MIN_HEIGHT, FIT_MAX_HEIGHT)
}

/// A list block's items as one text line each, nested items included — a list's words live
/// in [`Block::items`], not its `text`, and a fit that read the empty text sized every
/// list to the minimum.
fn item_lines(block: &Block) -> Vec<String> {
    fn collect(items: &[crate::model::Item], into: &mut Vec<String>) {
        for item in items {
            into.push(item.text.clone());
            collect(&item.nested, into);
        }
    }
    let mut lines = Vec::new();
    collect(&block.items, &mut lines);
    lines
}

/// The `viewBox` of an `svg` block, as (width, height), when it declares one.
fn view_box(text: &str) -> Option<(f64, f64)> {
    let after = text.split("viewBox=").nth(1)?;
    let quote = after.chars().next()?;
    let inside = after.get(1..)?.split(quote).next()?;
    let numbers: Vec<f64> = inside
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect();
    match numbers[..] {
        [_, _, w, h] if w > 0.0 && h > 0.0 => Some((w, h)),
        _ => None,
    }
}

/// An `html` block's visible text, one entry per source line, tags dropped.
///
/// Coarse by design: markup height depends on CSS the engine will not lay out, so the
/// estimate reads the words a line carries and lets the clamp bound the rest. Art whose
/// height is not its text — a gradient panel, a spacer — is better stated as numbers.
fn stripped_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            let mut visible = String::new();
            let mut in_tag = false;
            for character in line.chars() {
                match character {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    kept if !in_tag => visible.push(kept),
                    _ => {}
                }
            }
            visible.trim().to_string()
        })
        .collect()
}

/// A node's rectangle on the canvas, in canvas pixels: exactly what its line says.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    /// The point an edge meets this rectangle at, `fraction` of the way along `side`.
    fn anchor(self, side: Side, fraction: f64) -> Anchor {
        let (x, y) = match side {
            Side::Left => (self.x, self.y + self.h * fraction),
            Side::Right => (self.x + self.w, self.y + self.h * fraction),
            Side::Top => (self.x + self.w * fraction, self.y),
            Side::Bottom => (self.x + self.w * fraction, self.y + self.h),
        };
        let (dx, dy) = side.reach();
        Anchor { x, y, dx, dy }
    }

    /// The centre, which is what decides the sides of an unpinned edge.
    fn centre(self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Whether a point is inside this rectangle, with `slack` of margin around it.
    ///
    /// The margin is what keeps a line from grazing a box: a path that runs along a node's
    /// exact border reads as touching it, and a reader cannot tell a line that passes behind
    /// a box from one that stops at it.
    fn holds(self, x: f64, y: f64, slack: f64) -> bool {
        x > self.x - slack
            && x < self.x + self.w + slack
            && y > self.y - slack
            && y < self.y + self.h + slack
    }
}

/// How far a path keeps away from a box it is not attached to, in canvas pixels.
const CLEARANCE: f64 = 10.0;

/// How many points along a curve are tested against the boxes.
///
/// A curve is checked by sampling rather than solved analytically: a cubic against a
/// rectangle has no closed form worth writing, and thirty-two points along a path a few
/// hundred pixels long is finer than the boxes it could hide behind.
const SAMPLES: usize = 32;

/// Whether the straight segment between two points passes through `rect`.
///
/// Sampled for the same reason curves are, and used to judge a *candidate* route before the
/// curve exists: the choice of which sides an edge leaves and arrives on is made against the
/// straight line between them, which is the line the curve stays close to.
fn segment_hits(from: (f64, f64), to: (f64, f64), rect: Rect) -> bool {
    (0..=SAMPLES).any(|step| {
        let along = step as f64 / SAMPLES as f64;
        let x = from.0 + (to.0 - from.0) * along;
        let y = from.1 + (to.1 - from.1) * along;
        rect.holds(x, y, CLEARANCE)
    })
}

/// Where two segments cross, as the sine of the angle they meet at, or `None` if they do not.
///
/// The sine rather than a bare yes/no because not every crossing is equally bad. Two lines
/// meeting at a right angle read as one passing over the other; two meeting at a shallow
/// angle read as one line that forks, and a reader has to trace them to tell which is which.
/// So a crossing is scored by how square it is, and the router prefers the arrangement whose
/// unavoidable crossings are the most perpendicular.
fn crossing_sine(a: Run, b: Run) -> Option<f64> {
    let (p, r) = (a.0, (a.1 .0 - a.0 .0, a.1 .1 - a.0 .1));
    let (q, s) = (b.0, (b.1 .0 - b.0 .0, b.1 .1 - b.0 .1));
    let denominator = r.0 * s.1 - r.1 * s.0;
    if denominator.abs() < f64::EPSILON {
        // Parallel: either never meeting, or overlapping along their whole length. Neither is
        // a crossing a reader has to untangle.
        return None;
    }
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let along = (dx * s.1 - dy * s.0) / denominator;
    let across = (dx * r.1 - dy * r.0) / denominator;
    // Strictly inside both segments: two edges that merely share an endpoint meet there
    // because they are joined, not because they cross.
    if !(0.02..=0.98).contains(&along) || !(0.02..=0.98).contains(&across) {
        return None;
    }
    let length = (r.0.hypot(r.1) * s.0.hypot(s.1)).max(f64::EPSILON);
    Some((denominator / length).abs())
}

/// A straight run between two points: the line a route is judged against before it is drawn.
type Run = ((f64, f64), (f64, f64));

/// Where one end of an edge meets a node, and which way its handle reaches from there.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
}

/// One edge that will actually be drawn: the two nodes it joins, the sides it uses, and
/// which of those sides the line *pinned* rather than left to the renderer.
///
/// The pins are what reaches the page (`data-from-side`, absent when unpinned), because an
/// editing surface has to tell the two apart: a pinned end stays on the edge the reader
/// dragged it to however the board is rearranged, and an unpinned one re-routes to whatever
/// faces the other node now.
struct Drawn {
    from: usize,
    to: usize,
    from_side: Side,
    to_side: Side,
    from_pin: Option<Side>,
    to_pin: Option<Side>,
}

/// Render a board block: the clipped viewport, its fitted canvas, edge curves beneath the
/// nodes, and each node's block rendered by the page's own per-block renderer.
///
/// A node that names no block renders a sentence saying so — a dangling reference must send
/// the reader to fix the line, never vanish. A node naming another board renders a refusal
/// instead of recursing: a board inside a board is a cycle waiting to happen, and the
/// planning surface does not need one.
pub(super) fn board_html(
    block: &Block,
    document: &Document,
    options: &HtmlOptions,
    values: &Values,
) -> String {
    let height = if block.height > 0 {
        block.height
    } else {
        DEFAULT_BOARD_HEIGHT
    };
    board_html_in(
        block,
        document,
        options,
        values,
        STATIC_COLUMN,
        f64::from(height),
    )
}

/// Render a board into an explicit viewport, stated in the wrapper's own style.
///
/// This is [`board_html`] with the frame chosen by the caller instead of by the page
/// column — how a board is photographed at its natural size ([`natural_viewport`]) rather
/// than squeezed into the flow's 680px measure. The width is written into the wrapper only
/// here: in flow the board spans the column, and a stated width would fight it.
pub(super) fn board_html_in(
    block: &Block,
    document: &Document,
    options: &HtmlOptions,
    values: &Values,
    viewport_width: f64,
    viewport_height: f64,
) -> String {
    let mut node_list = deduped(nodes(&block.text));
    resolve_sizes(document, &mut node_list);

    let rects = rects(&node_list);
    let (scale, tx, ty) = fit(&rects, viewport_width, viewport_height);

    let size = if (viewport_width - STATIC_COLUMN).abs() < f64::EPSILON {
        format!("height:{}px", viewport_height.round())
    } else {
        format!(
            "width:{}px;height:{}px",
            viewport_width.round(),
            viewport_height.round()
        )
    };
    let mut out = format!(
        "<div{} style=\"{size}\">\n<div class=\"dx-board-canvas\" \
         style=\"transform:translate({tx}px,{ty}px) scale({scale})\">\n",
        super::html::attributes(block, &["dx-board"]),
    );
    out.push_str(&edges_svg(&block.id, &node_list, &rects));
    for node in &node_list {
        out.push_str(&node_html(node, document, options, values));
    }
    out.push_str("</div>\n</div>");
    out
}

/// First line wins when two lines claim one id: a duplicate is a typo, not two nodes.
fn deduped(list: Vec<Node>) -> Vec<Node> {
    let mut seen: Vec<String> = Vec::new();
    list.into_iter()
        .filter(|node| {
            let key = node.id.to_ascii_lowercase();
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        })
        .collect()
}

/// Every node's rectangle, in line order — the boxes the lines state, nothing measured.
fn rects(list: &[Node]) -> Vec<Rect> {
    list.iter()
        .map(|node| Rect {
            x: f64::from(node.x),
            y: f64::from(node.y),
            w: f64::from(node.w),
            h: f64::from(node.h),
        })
        .collect()
}

/// The viewport that shows a board's whole arrangement at its drawn size: the nodes'
/// bounding box plus the fit margin on every side, so [`fit`] lands on scale 1 exactly and
/// every node renders at the size its line states. A board with no nodes gets the flow
/// frame, which is also what it shows there: an empty canvas.
pub(super) fn natural_viewport(document: &Document, block: &Block) -> (f64, f64) {
    let mut node_list = deduped(nodes(&block.text));
    resolve_sizes(document, &mut node_list);
    match extent(&rects(&node_list)) {
        None => (STATIC_COLUMN, f64::from(DEFAULT_BOARD_HEIGHT)),
        Some((x0, y0, x1, y1)) => ((x1 - x0) + 2.0 * FIT_PADDING, (y1 - y0) + 2.0 * FIT_PADDING),
    }
}

/// Fit an arrangement into a viewport: the scale and translation that show all of it.
///
/// Both axes, always — a plan whose lower half is below the fold is a plan nobody read. The
/// arrangement is centred horizontally, and centred vertically only when it fits; a tall
/// board pins to the top instead, because a plan is read from its first node down.
fn fit(rects: &[Rect], viewport_width: f64, viewport_height: f64) -> (f64, f64, f64) {
    let Some((x0, y0, x1, y1)) = extent(rects) else {
        return (1.0, FIT_PADDING, FIT_PADDING);
    };
    let scale = ((viewport_width - 2.0 * FIT_PADDING) / (x1 - x0))
        .min((viewport_height - 2.0 * FIT_PADDING) / (y1 - y0))
        .clamp(0.1, FIT_MAX);
    let tx = (viewport_width - scale * (x0 + x1)) / 2.0;
    let ty = ((viewport_height - scale * (y0 + y1)) / 2.0).min(FIT_PADDING - scale * y0);
    (round(scale, 1000.0), round(tx, 100.0), round(ty, 100.0))
}

/// The nodes' bounding box, or `None` when there are no nodes to bound.
fn extent(rects: &[Rect]) -> Option<(f64, f64, f64, f64)> {
    let first = rects.first()?;
    let mut box_ = (first.x, first.y, first.x + first.w, first.y + first.h);
    for rect in rects {
        box_.0 = box_.0.min(rect.x);
        box_.1 = box_.1.min(rect.y);
        box_.2 = box_.2.max(rect.x + rect.w);
        box_.3 = box_.3.max(rect.y + rect.h);
    }
    // A single point is not a box: never divide by nothing.
    Some((
        box_.0,
        box_.1,
        box_.2.max(box_.0 + 1.0),
        box_.3.max(box_.1 + 1.0),
    ))
}

/// Trim a computed number to a few decimals, so the HTML stays readable and stable.
fn round(value: f64, places: f64) -> f64 {
    (value * places).round() / places
}

/// The sides an unpinned edge joins: the facing pair on the axis the two nodes are furthest
/// apart along.
///
/// Vertical wins a tie, because a board grows downwards — nodes are laid out in a column and
/// pushed apart vertically, so two boxes that overlap horizontally are almost always one
/// above the other. Choosing the horizontal pair for those made every edge on a stacked plan
/// hook out of a side and loop back around the node beneath it.
fn auto_sides(from: Rect, to: Rect) -> (Side, Side) {
    let (fx, fy) = from.centre();
    let (tx, ty) = to.centre();
    let (dx, dy) = (tx - fx, ty - fy);
    if dy.abs() >= dx.abs() {
        if dy >= 0.0 {
            (Side::Bottom, Side::Top)
        } else {
            (Side::Top, Side::Bottom)
        }
    } else if dx >= 0.0 {
        (Side::Right, Side::Left)
    } else {
        (Side::Left, Side::Right)
    }
}

/// Every edge that has both ends on this board, with the sides it joins resolved.
///
/// An end the line pinned is used as written — a reader who dragged an edge onto a
/// particular side meant that side. An end left unpinned is *chosen*, by [`clearest_sides`],
/// against the boxes and the edges already settled.
fn drawn_edges(list: &[Node], rects: &[Rect]) -> Vec<Drawn> {
    let mut drawn: Vec<Drawn> = Vec::new();
    let mut settled: Vec<Run> = Vec::new();
    for (from, node) in list.iter().enumerate() {
        for edge in &node.to {
            let Some(to) = list
                .iter()
                .position(|other| other.id.eq_ignore_ascii_case(&edge.id))
            else {
                continue;
            };
            if to == from {
                continue;
            }
            let (from_side, to_side) =
                clearest_sides(from, to, rects, edge.from, edge.to, &settled);
            settled.push((
                midpoint(rects[from], from_side),
                midpoint(rects[to], to_side),
            ));
            drawn.push(Drawn {
                from,
                to,
                from_side,
                to_side,
                from_pin: edge.from,
                to_pin: edge.to,
            });
        }
    }
    drawn
}

/// The middle of one side of a box — where a candidate route is measured from.
fn midpoint(rect: Rect, side: Side) -> (f64, f64) {
    let anchor = rect.anchor(side, 0.5);
    (anchor.x, anchor.y)
}

/// The pair of sides an edge should join: the one whose straight run is clearest.
///
/// Every combination the line leaves free is scored, and the cheapest wins. The three terms,
/// in the order they matter:
///
/// 1. **A box in the way** is the worst thing that can happen to an edge, because the box
///    is drawn over the line and the reader simply loses it. Weighted so that no number of
///    crossings is worth one occlusion.
/// 2. **A crossing** costs, and a *shallow* crossing costs more than a square one — two lines
///    meeting at a right angle read as one passing over the other, while two meeting at a
///    narrow angle read as a fork.
/// 3. **Leaving the way it is going** breaks the tie, which is what keeps the facing pair —
///    [`auto_sides`], the rule this board has always followed — as the answer whenever the
///    straight run between two nodes is already clear.
fn clearest_sides(
    from: usize,
    to: usize,
    rects: &[Rect],
    from_pin: Option<Side>,
    to_pin: Option<Side>,
    settled: &[Run],
) -> (Side, Side) {
    const SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];
    /// A box over a line beats any number of crossings.
    const OCCLUSION: f64 = 100.0;
    /// A crossing costs this at its worst — parallel — and a fraction of it when square.
    const CROSSING: f64 = 4.0;

    let (preferred_from, preferred_to) = auto_sides(rects[from], rects[to]);
    let from_options: Vec<Side> = from_pin.map_or_else(|| SIDES.to_vec(), |side| vec![side]);
    let to_options: Vec<Side> = to_pin.map_or_else(|| SIDES.to_vec(), |side| vec![side]);

    let mut best = (f64::MAX, preferred_from, preferred_to);
    for &out in &from_options {
        for &into in &to_options {
            let start = midpoint(rects[from], out);
            let end = midpoint(rects[to], into);

            let blocked = rects
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != from && *index != to)
                .filter(|(_, rect)| segment_hits(start, end, **rect))
                .count() as f64;

            let tangle: f64 = settled
                .iter()
                .filter_map(|other| crossing_sine((start, end), *other))
                // A square crossing (sine 1) costs a quarter of a parallel one.
                .map(|square| CROSSING * (1.25 - square * 0.75))
                .sum();

            // The facing pair is what a board has always used; only a real obstacle or a
            // real tangle should talk it out of that.
            let habit = f64::from(u8::from(out != preferred_from))
                + f64::from(u8::from(into != preferred_to));

            let cost = blocked * OCCLUSION + tangle + habit;
            if cost < best.0 {
                best = (cost, out, into);
            }
        }
    }
    (best.1, best.2)
}

/// Spread the edges over the sides they meet, and give each end its anchor.
///
/// A side is a whole connection edge, not a point: every edge landing on it takes an even
/// share of its length, in the order the other ends are arranged along that side's own axis,
/// so lines fan out instead of piling onto one dot and crossing each other on the way in.
/// With `n` edges on a side, the `k`th sits `(k+1)/(n+1)` of the way along it.
fn anchors(drawn: &[Drawn], rects: &[Rect]) -> Vec<(Anchor, Anchor)> {
    // One bucket per node side, holding (edge, which end, where the other end is).
    let mut buckets: Vec<Vec<(usize, bool, f64)>> = vec![Vec::new(); rects.len() * 4];
    for (index, edge) in drawn.iter().enumerate() {
        let (from_centre_x, from_centre_y) = rects[edge.from].centre();
        let (to_centre_x, to_centre_y) = rects[edge.to].centre();
        buckets[edge.from * 4 + edge.from_side.slot()].push((
            index,
            true,
            if edge.from_side.horizontal_span() {
                to_centre_x
            } else {
                to_centre_y
            },
        ));
        buckets[edge.to * 4 + edge.to_side.slot()].push((
            index,
            false,
            if edge.to_side.horizontal_span() {
                from_centre_x
            } else {
                from_centre_y
            },
        ));
    }

    let mut placed: Vec<(Option<Anchor>, Option<Anchor>)> = vec![(None, None); drawn.len()];
    for (slot, bucket) in buckets.iter_mut().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        // The other end's position decides the order; the edge's own index breaks a tie, so
        // two edges to the same place still land in a stated order rather than a lucky one.
        bucket.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        let count = bucket.len() as f64;
        let rect = rects[slot / 4];
        for (position, (index, is_from, _)) in bucket.iter().enumerate() {
            let side = if *is_from {
                drawn[*index].from_side
            } else {
                drawn[*index].to_side
            };
            let anchor = rect.anchor(side, (position as f64 + 1.0) / (count + 1.0));
            if *is_from {
                placed[*index].0 = Some(anchor);
            } else {
                placed[*index].1 = Some(anchor);
            }
        }
    }

    placed
        .into_iter()
        .enumerate()
        .map(|(index, ends)| {
            // Every drawn edge put both of its ends in a bucket above, so both are here.
            let fallback = || rects[drawn[index].from].anchor(drawn[index].from_side, 0.5);
            (
                ends.0.unwrap_or_else(fallback),
                ends.1.unwrap_or_else(fallback),
            )
        })
        .collect()
}

/// How long the straight run into an arrowhead is, in canvas pixels — a little more than
/// the head itself (a 9-unit marker on a 1.5px stroke), so the whole head sits on line
/// that is not curving under it.
const HEAD_RUN: f64 = 16.0;

/// The point the curve hands over to the arrowhead: [`HEAD_RUN`] out from the arrival
/// anchor, along the arrival side's own normal, shortened on a hop too small to hold it.
///
/// The cubic ends here and a straight segment carries the line the rest of the way in.
/// That last segment is what makes the head honest: an arrowhead orients to the path's
/// direction at its very tip, and a cubic bent around an obstacle is still turning as it
/// arrives — so the head would point one way while the visible line came in another, and
/// the line met the head off its centre. A straight tail pins both: the head sits square
/// to the side it arrives on, and the line enters it dead down its axis.
fn lead(from: Anchor, to: Anchor) -> Anchor {
    let span = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
    let run = (span * 0.25).clamp(4.0, HEAD_RUN);
    Anchor {
        x: to.x + run * to.dx,
        y: to.y + run * to.dy,
        dx: to.dx,
        dy: to.dy,
    }
}

/// The path an edge is drawn as: a cubic from the source to the [`lead`] point, then the
/// straight run into the arrowhead.
///
/// The cubic's handles reach out of each end along its own side's normal, a share of the
/// distance between them, bounded at both ends — short enough that neighbours do not bulge
/// into each other, long enough that a short hop still reads as a curve. The same numbers
/// the editing surface uses against measured boxes, so a board does not change shape when a
/// reader can touch it.
fn curve(from: Anchor, to: Anchor, obstacles: &[Rect]) -> String {
    let handover = lead(from, to);
    let (first, second) = controls(from, handover, obstacles);
    format!(
        "M {} {} C {} {}, {} {}, {} {} L {} {}",
        round(from.x, 100.0),
        round(from.y, 100.0),
        round(first.0, 100.0),
        round(first.1, 100.0),
        round(second.0, 100.0),
        round(second.1, 100.0),
        round(handover.x, 100.0),
        round(handover.y, 100.0),
        round(to.x, 100.0),
        round(to.y, 100.0),
    )
}

/// The two control points of an edge's cubic, bent aside if the plain curve would hide
/// behind a box.
///
/// The plain curve — a handle out of each end along its own side's normal — is tried first,
/// and is what almost every edge gets. Each handle's reach is a share of the span, but
/// never more than half the distance the run actually advances along that handle's own
/// normal: two nodes a short gap apart used to get handles that overshot past each other,
/// and the cubic between them wobbled into an S-hook where a reader expected a quiet curve.
/// A run that starts out *against* its own normal keeps the full reach, because that curve
/// has to loop out and around before it can come back. When the plain curve is not clear,
/// the pair of handles is pushed sideways, square to the line between the ends, in steadily
/// larger steps and both directions, and the first arrangement that clears every box wins.
/// The steps run far past the span itself, because a wall of a node can stand taller than
/// the whole run and the curve has to reach around it. A board so dense that *nothing*
/// clears keeps the attempt that hid the fewest points of the line — never the last one
/// tried — because an edge that dips behind one corner still reads, and one that runs the
/// length of a box is lost.
///
/// Bending the handles rather than inserting a waypoint is what keeps this mirrorable: the
/// edge is still one cubic with the same two endpoints, so an editing surface re-lays it by
/// the same rule against the boxes it has measured.
fn controls(from: Anchor, to: Anchor, obstacles: &[Rect]) -> Run {
    /// How far each attempt pushes the handles, as a share of the reach.
    const STEPS: [f64; 10] = [0.0, 0.35, 0.7, 1.1, 1.6, 2.2, 3.0, 4.0, 5.5, 7.5];

    let span = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
    let reach = (span * 0.42).clamp(30.0, 160.0);
    let capped = |end: Anchor, other: Anchor| {
        let advance = (other.x - end.x) * end.dx + (other.y - end.y) * end.dy;
        if advance > 0.0 {
            reach.min((advance * 0.5).max(10.0))
        } else {
            reach
        }
    };
    let (from_reach, to_reach) = (capped(from, to), capped(to, from));
    let base = (
        (from.x + from_reach * from.dx, from.y + from_reach * from.dy),
        (to.x + to_reach * to.dx, to.y + to_reach * to.dy),
    );
    if obstacles.is_empty() {
        return base;
    }

    // Square to the line between the ends, which is the direction that moves the middle of
    // the curve without moving where it meets either node.
    let length = span.max(f64::EPSILON);
    let square = (-(to.y - from.y) / length, (to.x - from.x) / length);

    let mut least = (usize::MAX, base);
    for step in STEPS {
        for direction in [1.0, -1.0] {
            let push = step * reach * direction;
            let candidate = (
                (base.0 .0 + square.0 * push, base.0 .1 + square.1 * push),
                (base.1 .0 + square.0 * push, base.1 .1 + square.1 * push),
            );
            let hidden = hidden_samples(from, to, candidate, obstacles);
            if hidden == 0 {
                return candidate;
            }
            if hidden < least.0 {
                least = (hidden, candidate);
            }
            if step == 0.0 {
                // Both directions are the same curve when nothing is pushed.
                break;
            }
        }
    }
    least.1
}

/// How many sampled points of a cubic with these control points sit inside an obstacle.
///
/// Zero means the curve is clear; anything else says how much of the line a reader would
/// lose, which is what ranks the fallback when no attempt clears everything.
fn hidden_samples(from: Anchor, to: Anchor, controls: Run, obstacles: &[Rect]) -> usize {
    // The ends sit *on* their own nodes' borders, so the sampling skips the very ends: an
    // edge is attached to its two nodes, and touching them is the point.
    (2..SAMPLES - 1)
        .filter(|step| {
            let along = *step as f64 / SAMPLES as f64;
            let (x, y) = cubic_at(from, to, controls, along);
            obstacles.iter().any(|rect| rect.holds(x, y, CLEARANCE))
        })
        .count()
}

/// A point `along` a cubic, `along` running 0 at `from` to 1 at `to`.
fn cubic_at(from: Anchor, to: Anchor, controls: Run, along: f64) -> (f64, f64) {
    let rest = 1.0 - along;
    let (a, b, c, d) = (
        rest * rest * rest,
        3.0 * rest * rest * along,
        3.0 * rest * along * along,
        along * along * along,
    );
    (
        a * from.x + b * controls.0 .0 + c * controls.1 .0 + d * to.x,
        a * from.y + b * controls.0 .1 + c * controls.1 .1 + d * to.y,
    )
}

/// The edges, drawn once beneath every node as one SVG of cubic curves.
///
/// The anchors stand on guessed heights (a render cannot measure content); an editing
/// surface re-lays the same paths, by the same rules, against the measured boxes. An edge
/// naming a node that is not on the board is skipped — the missing *node* already renders
/// its own sentence.
///
/// Every edge ends in an arrowhead, because an edge on a plan is not a connection but a
/// direction: "this, then that" — and begins in a small tether, the dot that says the line
/// is fastened to *that* point of *that* edge rather than floating near it. Both are
/// `marker`s, so they stay on the ends of the path wherever an editing surface moves them,
/// and both take their colour from the stroke they sit on (`context-stroke`), so a picked
/// edge is picked end to end. Their ids carry the board's own id — a page may hold several
/// boards, and two markers sharing one id is one marker.
fn edges_svg(board_id: &str, list: &[Node], rects: &[Rect]) -> String {
    let drawn = drawn_edges(list, rects);
    if drawn.is_empty() {
        return String::new();
    }
    let anchored = anchors(&drawn, rects);
    let arrow = format!("dx-edge-arrow-{board_id}");
    let tether = format!("dx-edge-tether-{board_id}");
    let mut paths = String::new();
    let mut labels = String::new();
    for (edge, (from, to)) in drawn.iter().zip(anchored) {
        // Only a *pinned* side reaches the page: an editing surface re-routes the rest.
        let pin = |side: Option<Side>, name: &str| match side {
            Some(side) => format!(" data-{name}-side=\"{}\"", side.letter()),
            None => String::new(),
        };
        // A node is an obstacle to every edge except the two it joins, which it is attached
        // to. Passing the whole list and filtering here keeps the rule in one place.
        let obstacles: Vec<Rect> = rects
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != edge.from && *index != edge.to)
            .map(|(_, rect)| *rect)
            .collect();
        let handover = lead(from, to);
        let bent = controls(from, handover, &obstacles);
        paths.push_str(&format!(
            "<path data-from=\"{}\" data-to=\"{}\"{}{} marker-start=\"url(#{})\" \
             marker-end=\"url(#{})\" d=\"{}\"/>\n",
            escape_html(&list[edge.from].id),
            escape_html(&list[edge.to].id),
            pin(edge.from_pin, "from"),
            pin(edge.to_pin, "to"),
            escape_html(&tether),
            escape_html(&arrow),
            curve(from, to, &obstacles),
        ));
        let text = edge_label(&list[edge.from], &list[edge.to]);
        if !text.is_empty() {
            let (x, y) = cubic_at(from, handover, bent, 0.5);
            // Named with the pair it belongs to, because only *labelled* edges get a `text`
            // and an editing surface re-placing them cannot count its way to the right one.
            labels.push_str(&format!(
                "<text class=\"dx-board-edge-label\" data-from=\"{}\" data-to=\"{}\" \
                 x=\"{}\" y=\"{}\">{}</text>\n",
                escape_html(&list[edge.from].id),
                escape_html(&list[edge.to].id),
                round(x, 100.0),
                round(y, 100.0),
                escape_html(&text),
            ));
        }
    }
    // The head is a dart — swept back and slightly concave — rather than a squat full
    // triangle: it reads as a point on a hairline stroke instead of a wedge sitting on it.
    format!(
        "<svg class=\"dx-board-edges\" width=\"1\" height=\"1\">\n\
         <defs><marker id=\"{}\" viewBox=\"0 0 10 10\" refX=\"8.5\" refY=\"5\" \
         markerWidth=\"9\" markerHeight=\"9\" orient=\"auto-start-reverse\">\
         <path d=\"M 0 0.8 L 9.6 5 L 0 9.2 C 2.6 7.4, 2.6 2.6, 0 0.8 z\" \
         fill=\"context-stroke\"/></marker>\n\
         <marker id=\"{}\" viewBox=\"0 0 6 6\" refX=\"3\" refY=\"3\" markerWidth=\"5\" \
         markerHeight=\"5\"><circle cx=\"3\" cy=\"3\" r=\"2.4\" fill=\"context-stroke\"/>\
         </marker></defs>\n\
         {paths}{labels}</svg>\n",
        escape_html(&arrow),
        escape_html(&tether),
    )
}

/// The words written on the edge from `from` to `to`, or nothing when it carries none.
fn edge_label(from: &Node, to: &Node) -> String {
    from.to
        .iter()
        .find(|edge| edge.id.eq_ignore_ascii_case(&to.id))
        .map(|edge| edge.label.clone())
        .unwrap_or_default()
}

/// One node: the box its line states, and the referenced block exactly as the page draws it.
///
/// The box is the box — `left/top/width/height` in the node's own style — so the rectangle
/// this renderer did its geometry against is the rectangle a browser lays out, and a board
/// cannot be one shape in the engine and another on the page. Content longer than the box
/// scrolls inside it rather than being lost: a node is a frame onto a block, and the reader
/// decides how big the frame is.
fn node_html(node: &Node, document: &Document, options: &HtmlOptions, values: &Values) -> String {
    let mut node_class = "dx-board-node";
    let inner = match document.block_index(&node.id) {
        None => missing(&format!("no block named `{}`", node.id)),
        Some(index) => {
            let referenced = &document.blocks[index];
            if referenced.kind == "board" {
                missing("a board cannot hold a board")
            } else if referenced.kind == "view" && !referenced.text.is_empty() {
                // A view fills its stated box edge to edge — it is a screen, and a screen
                // has no margin. The node drops its padding (`dx-board-node-view`), so the
                // inner box the frame scales into is the stated box minus the hairline.
                node_class = "dx-board-node dx-board-node-view";
                super::html::view_html(
                    referenced,
                    f64::from(node.w.saturating_sub(2)),
                    Some(f64::from(node.h.saturating_sub(2))),
                )
            } else {
                let rendered = block_html(referenced, document, options, values);
                if rendered.is_empty() {
                    missing(&format!("`{}` has nothing to show", node.id))
                } else {
                    rendered
                }
            }
        }
    };
    format!(
        "<section class=\"{node_class}\" data-node-id=\"{}\" \
         style=\"left:{}px;top:{}px;width:{}px;height:{}px\">\n\
         <div class=\"dx-board-node-body\">\n{inner}\n</div>\n</section>\n",
        escape_html(&node.id),
        node.x,
        node.y,
        node.w,
        node.h
    )
}

/// The sentence a node shows when it cannot show a block.
fn missing(sentence: &str) -> String {
    format!(
        "<div class=\"dx-board-missing\">{}</div>",
        escape_html(sentence)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_line_reads_back_exactly_what_the_writer_writes() {
        let node = Node {
            id: "sketch".into(),
            x: -20,
            y: 380,
            w: 320,
            h: 240,
            to: vec![
                Edge {
                    id: "ideas".into(),
                    from: Some(Side::Bottom),
                    to: Some(Side::Top),
                    label: String::new(),
                },
                Edge::unpinned("notes"),
            ],
            rest: vec!["future=yes".into()],
            ..Node::default()
        };
        let line = node_line(&node);
        assert_eq!(
            line,
            "- sketch x=-20 y=380 w=320 h=240 to=ideas:b-t,notes future=yes"
        );
        assert_eq!(parse_node_line(&line), Some(node));
    }

    #[test]
    fn board_edges_reads_every_boards_stated_pairs_in_order() {
        let document = crate::format::parse(
            "::board id=plan\n\
             - setup x=0 y=0 to=note:b-t\n\
             - note x=0 y=200 to=test\n\
             - test x=0 y=400\n\
             ::end\n\n\
             ::board id=aside\n\
             - test x=0 y=0 to=ghost:r-l:label\n\
             ::end\n\n\
             ::paragraph id=prose\nNo edges here.\n::end\n",
        );
        assert_eq!(
            board_edges(&document),
            vec![
                ("setup".to_string(), "note".to_string()),
                ("note".to_string(), "test".to_string()),
                // A dangling target is returned as stated; the caller decides its meaning.
                ("test".to_string(), "ghost".to_string()),
            ]
        );
    }

    #[test]
    fn a_bare_id_takes_every_default_and_a_blank_line_is_no_node() {
        let node = parse_node_line("ideas").expect("a bare id is a node");
        assert_eq!(
            (node.x, node.y, node.w, node.h),
            (0, 0, DEFAULT_NODE_WIDTH, DEFAULT_NODE_HEIGHT)
        );
        assert!(node.to.is_empty());
        assert_eq!(parse_node_line("   "), None);
    }

    /// One end may be pinned and the other left to the renderer, and a side that is not a
    /// side leaves an edge that still draws.
    #[test]
    fn an_edge_may_pin_one_end_and_survives_a_misspelt_side() {
        let node = parse_node_line("- a x=0 y=0 to=b:-r,c:xx-t,d:l-").expect("a node");
        assert_eq!(
            node.to,
            vec![
                Edge {
                    id: "b".into(),
                    from: None,
                    to: Some(Side::Right),
                    label: String::new(),
                },
                Edge {
                    id: "c".into(),
                    from: None,
                    to: Some(Side::Top),
                    label: String::new(),
                },
                Edge {
                    id: "d".into(),
                    from: Some(Side::Left),
                    to: None,
                    label: String::new(),
                },
            ]
        );
        assert_eq!(
            node_line(&node),
            "- a x=0 y=0 w=280 h=180 to=b:.-r,c:.-t,d:l-."
        );
    }

    /// A 200×100 node at `x`,`y`, pointing wherever the test says — a small helper, so the
    /// geometry rules read at a glance.
    fn node_at(id: &str, x: i32, y: i32, to: Vec<Edge>) -> Node {
        Node {
            id: id.into(),
            x,
            y,
            w: 200,
            h: 100,
            to,
            ..Node::default()
        }
    }

    /// That node's rectangle — the box its line states, which is the only box there is.
    fn rect(x: f64, y: f64) -> Rect {
        Rect {
            x,
            y,
            w: 200.0,
            h: 100.0,
        }
    }

    /// Every point a path visits — the cubic and the straight run into the arrowhead —
    /// for asking whether a box is sitting on top of it.
    fn samples_of(path: &str) -> Vec<(f64, f64)> {
        let numbers = path_numbers(path);
        let ends = |x, y| Anchor {
            x,
            y,
            dx: 0.0,
            dy: 0.0,
        };
        let mut samples: Vec<(f64, f64)> = (0..=SAMPLES)
            .map(|step| {
                cubic_at(
                    ends(numbers[0], numbers[1]),
                    ends(numbers[6], numbers[7]),
                    ((numbers[2], numbers[3]), (numbers[4], numbers[5])),
                    step as f64 / SAMPLES as f64,
                )
            })
            .collect();
        samples.push((numbers[8], numbers[9]));
        samples
    }

    /// The ten numbers of an edge path: `M` start, `C` two handles and the lead point,
    /// `L` the arrival anchor.
    fn path_numbers(path: &str) -> Vec<f64> {
        let numbers: Vec<f64> = path
            .split(['M', 'C', 'L', ',', ' '])
            .filter(|word| !word.is_empty())
            .filter_map(|word| word.parse().ok())
            .collect();
        assert_eq!(
            numbers.len(),
            10,
            "an edge is a cubic and its straight tail: {path}"
        );
        numbers
    }

    /// A node standing directly between two linked nodes must not end up sitting on the
    /// line that joins them: the box is painted over the edge, so an edge that runs under
    /// one is an edge the reader simply cannot see.
    #[test]
    fn an_edge_bends_around_a_box_standing_in_its_way() {
        let list = vec![
            node_at("left", 0, 0, vec![Edge::unpinned("right")]),
            node_at("right", 700, 0, vec![]),
            // Squarely between them, on the straight run from one to the other.
            node_at("wall", 330, 0, vec![]),
        ];
        let boxes = rects(&list);
        let svg = edges_svg("plan", &list, &boxes);
        // From the edge itself, not the arrowhead in `defs`, which also carries a `d`.
        let path = svg
            .split("<path data-from=")
            .nth(1)
            .and_then(|edge| edge.split(" d=\"").nth(1))
            .and_then(|rest| rest.split('"').next())
            .expect("one edge");

        let wall = boxes[2];
        for (x, y) in samples_of(path) {
            assert!(
                !wall.holds(x, y, 0.0),
                "the edge runs under `wall` at {x},{y}: {path}"
            );
        }
    }

    /// A wall taller than the whole run between two nodes: the curve has to reach right
    /// around it, not merely bulge, and must never be left running underneath — the widest
    /// bend the router will try runs far past the span itself for exactly this shape.
    #[test]
    fn an_edge_reaches_around_a_wall_taller_than_its_run() {
        let from = rect(0.0, 0.0).anchor(Side::Right, 0.5);
        let far = Rect {
            x: 1500.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        };
        let to = far.anchor(Side::Left, 0.5);
        let wall = Rect {
            x: 750.0,
            y: -500.0,
            w: 200.0,
            h: 1200.0,
        };
        let path = curve(from, to, &[wall]);
        for (x, y) in samples_of(&path) {
            assert!(
                !wall.holds(x, y, 0.0),
                "the edge runs under the wall at {x},{y}: {path}"
            );
        }
    }

    /// With nothing in the way, the plain curve is what is drawn — the bend is a response to
    /// an obstacle, not a permanent detour every edge pays for.
    #[test]
    fn an_unobstructed_edge_is_drawn_as_the_plain_curve_it_always_was() {
        let list = vec![
            node_at("a", 0, 0, vec![Edge::unpinned("b")]),
            node_at("b", 600, 0, vec![]),
        ];
        let boxes = rects(&list);
        let from = boxes[0].anchor(Side::Right, 0.5);
        let to = boxes[1].anchor(Side::Left, 0.5);
        assert_eq!(
            curve(from, to, &[]),
            curve(from, to, &[boxes[0], boxes[1]]),
            "an edge's own two nodes are never obstacles to it"
        );
    }

    /// Two nodes side by side, each pointing at the other's neighbour, would cross if both
    /// edges took the facing pair. The router should pick sides that keep them apart.
    #[test]
    fn unpinned_edges_take_the_sides_that_keep_them_from_tangling() {
        let list = vec![
            node_at("a", 0, 0, vec![Edge::unpinned("target")]),
            node_at("b", 0, 200, vec![Edge::unpinned("target")]),
            node_at("target", 500, 100, vec![]),
        ];
        let boxes = rects(&list);
        let drawn = drawn_edges(&list, &boxes);
        // Both arrive on the side of `target` that faces them, rather than one of them
        // wrapping around it.
        assert!(
            drawn.iter().all(|edge| edge.to_side == Side::Left),
            "edges should arrive on the facing side"
        );
    }

    /// An edge label survives the node line and is drawn on the curve, in the page's ink.
    #[test]
    fn an_edge_carries_its_words_onto_the_page() {
        let labelled = Edge {
            id: "b".into(),
            from: Some(Side::Right),
            to: Some(Side::Left),
            label: "on failure".into(),
        };
        let line = node_line(&node_at("a", 0, 0, vec![labelled]));
        // Written without a space, so the label cannot be read as another token…
        assert!(line.contains("to=b:r-l:on%20failure"), "{line}");
        // …and read back as the words it was.
        assert_eq!(
            parse_node_line(&line).expect("a node").to[0].label,
            "on failure"
        );

        let list = vec![
            parse_node_line(&line).expect("a node"),
            node_at("b", 500, 0, vec![]),
        ];
        let svg = edges_svg("plan", &list, &rects(&list));
        assert!(svg.contains("dx-board-edge-label"), "{svg}");
        assert!(svg.contains(">on failure</text>"), "{svg}");
    }

    /// An unpinned edge takes the facing pair on the axis the nodes are furthest apart
    /// along, and a tie goes vertical — a board grows downwards.
    #[test]
    fn an_unpinned_edge_joins_the_facing_sides_of_the_longer_axis() {
        assert_eq!(
            auto_sides(rect(0.0, 0.0), rect(600.0, 20.0)),
            (Side::Right, Side::Left)
        );
        assert_eq!(
            auto_sides(rect(600.0, 20.0), rect(0.0, 0.0)),
            (Side::Left, Side::Right)
        );
        assert_eq!(
            auto_sides(rect(0.0, 0.0), rect(40.0, 400.0)),
            (Side::Bottom, Side::Top)
        );
        assert_eq!(
            auto_sides(rect(0.0, 400.0), rect(0.0, 0.0)),
            (Side::Top, Side::Bottom)
        );
    }

    /// Three edges leaving one side stand at a third, a half, and two thirds of it — never
    /// three lines out of one dot — and they are ordered by where they are going, so the
    /// fan does not cross itself.
    #[test]
    fn edges_sharing_a_side_spread_along_it_in_the_order_they_are_going() {
        let down = |id: &str| Edge {
            id: id.into(),
            from: Some(Side::Bottom),
            to: Some(Side::Top),
            label: String::new(),
        };
        let list = vec![
            node_at("hub", 0, 0, vec![down("right"), down("left")]),
            node_at("left", -300, 400, Vec::new()),
            node_at("right", 300, 400, Vec::new()),
        ];
        let rects = rects(&list);
        let drawn = drawn_edges(&list, &rects);
        let anchored = anchors(&drawn, &rects);
        // The edge to `left` leaves at a third of the hub's bottom, the one to `right` at
        // two thirds — the order they arrive in, not the order they were written in.
        let to_right = anchored[0].0;
        let to_left = anchored[1].0;
        assert!(to_left.x < to_right.x, "{to_left:?} {to_right:?}");
        assert!((to_left.x - 200.0 / 3.0).abs() < 0.01, "{to_left:?}");
        assert!((to_right.x - 400.0 / 3.0).abs() < 0.01, "{to_right:?}");
        assert_eq!(to_left.y, 100.0);
        assert_eq!((to_left.dx, to_left.dy), (0.0, 1.0));
    }

    /// Every edge is tethered at one end and pointed at the other — a plan's edge says
    /// "this, then that" — and the markers holding both are named for their own board, so
    /// two boards on one page keep two. Only a *pinned* side reaches the page: the surface
    /// re-routes the rest against boxes it can measure.
    #[test]
    fn every_edge_is_tethered_and_pointed_and_says_only_what_it_pinned() {
        let list = vec![
            node_at(
                "a",
                0,
                0,
                vec![Edge {
                    id: "b".into(),
                    from: Some(Side::Right),
                    to: None,
                    label: String::new(),
                }],
            ),
            node_at("b", 0, 400, Vec::new()),
        ];
        let svg = edges_svg("plan", &list, &rects(&list));
        assert!(svg.contains("<marker id=\"dx-edge-arrow-plan\""), "{svg}");
        assert!(svg.contains("<marker id=\"dx-edge-tether-plan\""), "{svg}");
        assert!(
            svg.contains("marker-end=\"url(#dx-edge-arrow-plan)\""),
            "{svg}"
        );
        assert!(
            svg.contains("marker-start=\"url(#dx-edge-tether-plan)\""),
            "{svg}"
        );
        assert!(svg.contains("fill=\"context-stroke\""), "{svg}");
        // The pinned side is stated; the unpinned one is left for the surface to decide.
        assert!(svg.contains("data-from-side=\"r\""), "{svg}");
        assert!(!svg.contains("data-to-side"), "{svg}");
        // An edge whose target is not on the board draws nothing at all.
        let ghost = vec![node_at("a", 0, 0, vec![Edge::unpinned("ghost")])];
        assert!(edges_svg("plan", &ghost, &rects(&ghost)).is_empty());
    }

    /// The fit shows everything: an arrangement taller than the viewport is scaled down on
    /// the axis that needs it most, and a small one is magnified into the room it has.
    #[test]
    fn the_fit_scales_on_both_axes_and_never_past_the_bound() {
        let tall = vec![rect(0.0, 0.0), rect(0.0, 2000.0)];
        let (scale, _, ty) = fit(&tall, 680.0, 480.0);
        assert!(scale < 0.25, "a 2100px column must fit 480px: {scale}");
        // Everything is inside: the top of the content sits at the padding or below it.
        assert!(ty <= FIT_PADDING + 0.01, "{ty}");
        assert!(ty + scale * 2100.0 <= 480.0 + 0.01, "{ty} {scale}");

        let small = vec![rect(0.0, 0.0)];
        let (magnified, _, _) = fit(&small, 680.0, 480.0);
        assert_eq!(magnified, FIT_MAX);

        // No nodes is a viewport with nothing in it, not a division by zero.
        assert_eq!(fit(&[], 680.0, 480.0), (1.0, FIT_PADDING, FIT_PADDING));
    }

    #[test]
    fn malformed_coordinates_fall_back_instead_of_killing_the_line() {
        let node = parse_node_line("- a x=abc y=12 w=oops h=tall").expect("still a node");
        assert_eq!(
            (node.x, node.y, node.w, node.h),
            (0, 12, DEFAULT_NODE_WIDTH, DEFAULT_NODE_HEIGHT)
        );
        assert_eq!((node.w_rule, node.h_rule), (Sizing::Stated, Sizing::Stated));
    }

    /// Even bent around an obstacle, an edge's last stretch is straight, square to the side
    /// it arrives on — so the line runs dead down the arrowhead's own axis, never into its
    /// corner.
    #[test]
    fn the_line_enters_the_arrowhead_straight_along_its_axis() {
        let from = rect(0.0, 0.0).anchor(Side::Right, 0.5);
        let to = rect(600.0, 0.0).anchor(Side::Left, 0.5);
        let wall = Rect {
            x: 300.0,
            y: -50.0,
            w: 120.0,
            h: 200.0,
        };
        let numbers = path_numbers(&curve(from, to, &[wall]));
        // The path ends at the anchor itself…
        assert_eq!((numbers[8], numbers[9]), (to.x, to.y));
        // …reached by a straight run along the left side's own normal: same y, HEAD_RUN out.
        assert_eq!(numbers[7], to.y, "the lead point is on the arrival axis");
        assert_eq!(numbers[6], to.x + HEAD_RUN * to.dx, "one head-run out");
    }

    /// `w=page` and `h=fit` are rules, not numbers: they read back as the words they are,
    /// so the rule keeps applying as the document changes.
    #[test]
    fn page_and_fit_survive_the_round_trip_as_words() {
        let node = parse_node_line("- hero x=0 y=40 w=page h=fit").expect("a node");
        assert_eq!((node.w_rule, node.h_rule), (Sizing::Page, Sizing::Fit));
        assert_eq!(node_line(&node), "- hero x=0 y=40 w=page h=fit");

        let half = parse_node_line("- hero x=0 y=40 w=320 h=fit").expect("a node");
        assert_eq!(node_line(&half), "- hero x=0 y=40 w=320 h=fit");
    }

    /// The rules resolve from nothing but the document: `page` to the page's own measures,
    /// `fit` to an estimate of the block that grows with its text — and a node naming no
    /// block takes the defaults rather than failing.
    #[test]
    fn page_and_fit_resolve_deterministically_from_the_document() {
        let document = crate::format::parse(
            "::paragraph id=short hidden\nOne line.\n::end\n\n\
             ::paragraph id=long hidden\nA paragraph that says a great deal more than the \
             short one does, and keeps saying it until wrapping at any sensible width takes \
             several lines of the page to hold what it says.\n::end\n",
        );
        let mut nodes = nodes(
            "- short x=0 y=0 w=page h=fit\n- long x=0 y=300 w=280 h=fit\n- ghost x=0 y=600 w=fit h=fit",
        );
        resolve_sizes(&document, &mut nodes);
        assert_eq!(nodes[0].w, PAGE_NODE_WIDTH);
        assert!(nodes[0].h >= FIT_MIN_HEIGHT);
        assert!(
            nodes[1].h > nodes[0].h,
            "more text is a taller fit: {} vs {}",
            nodes[1].h,
            nodes[0].h
        );
        assert_eq!(
            (nodes[2].w, nodes[2].h),
            (DEFAULT_NODE_WIDTH, DEFAULT_NODE_HEIGHT),
            "a dangling reference fits to the defaults"
        );
        // Resolution never leaks into the document: the lines still say the words.
        assert_eq!(node_line(&nodes[0]), "- short x=0 y=0 w=page h=fit");
    }

    /// A list's words live in its items, not its `text` — a fit that read the text sized
    /// every checklist to the minimum, which is the bug this pins shut.
    #[test]
    fn a_list_fits_to_its_items() {
        let document = crate::format::parse(
            "::checklist id=queue hidden\n\
             [x] a first item long enough to wrap once at a node's width, all of it counted\n\
             [ ] a second item of the same kind of length, so the list stands several lines\n\
             [ ] a third, so three items cannot fit inside the minimum height\n::end\n",
        );
        let mut nodes = nodes("- queue x=0 y=0 w=280 h=fit");
        resolve_sizes(&document, &mut nodes);
        assert!(
            nodes[0].h > FIT_MIN_HEIGHT,
            "three items are taller than the floor: {}",
            nodes[0].h
        );
    }
}
