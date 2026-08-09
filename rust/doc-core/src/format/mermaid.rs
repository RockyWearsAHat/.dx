//! Mermaid flowchart source → a `::board`.
//!
//! A `::mermaid` (or its older spelling, `::graph`) block is not a kind this format keeps. A
//! board is the same drawing with a reader's hands on it — nodes that can be dragged,
//! reshaped, linked, and *written in*, because every node is an ordinary block of the same
//! document — so a mermaid block is converted the moment it is parsed and the document goes
//! on as a board. There is no second diagram renderer to keep in step with the first.
//!
//! The conversion is a read, not a migration pass someone has to run: [`convert`] happens
//! inside [`crate::format::parse`], so every consumer — the renderer, `crate::edit`, the
//! store, an agent calling `dx_source` — sees the board. The document is written back as a
//! board the next time it is saved.
//!
//! # What survives
//! Node ids, their labels, edge direction, and edge labels. Mermaid's shapes (`[]`, `()`,
//! `{}`, `(())`, …) do not: a board node is a block of the document, and blocks are dressed
//! by the document's own CSS rather than by a punctuation code in a diagram language. The
//! label text inside every shape is kept.
//!
//! Anything this module cannot read — `subgraph`, `click`, `classDef`, a statement with no
//! link in it — is skipped rather than guessed at, and a source with no readable edges at
//! all converts to nothing so the block stays exactly as it was written.

use super::layout;
use crate::model::Block;

/// The direction a flowchart flows, which becomes the sides its edges are pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// `LR` — layers march right; edges leave the right side and arrive at the left.
    Right,
    /// `RL` — layers march left.
    Left,
    /// `TD`/`TB` — layers march down; edges leave the bottom and arrive at the top.
    Down,
    /// `BT` — layers march up.
    Up,
}

impl Direction {
    /// The direction a `graph`/`flowchart` header names, defaulting to top-down as mermaid
    /// itself does.
    fn parse(word: &str) -> Direction {
        match word.trim().to_ascii_uppercase().as_str() {
            "LR" => Direction::Right,
            "RL" => Direction::Left,
            "BT" => Direction::Up,
            _ => Direction::Down,
        }
    }

    /// Whether layers advance along the x axis.
    pub(crate) fn is_horizontal(self) -> bool {
        matches!(self, Direction::Right | Direction::Left)
    }

    /// Whether layers advance towards increasing coordinates.
    pub(crate) fn is_forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }

    /// The `(from, to)` side letters an edge takes when it follows this direction.
    pub(crate) fn sides(self) -> (char, char) {
        match self {
            Direction::Right => ('r', 'l'),
            Direction::Left => ('l', 'r'),
            Direction::Down => ('b', 't'),
            Direction::Up => ('t', 'b'),
        }
    }
}

/// One node of a parsed flowchart: the key the source calls it, and the text it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphNode {
    /// The identifier the mermaid source uses, e.g. `A`.
    pub key: String,
    /// The text inside the node's shape, or the key when the source gave none.
    pub label: String,
}

/// One link of a parsed flowchart, as indices into the node list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphEdge {
    /// Index of the node the link leaves.
    pub from: usize,
    /// Index of the node the link arrives at.
    pub to: usize,
    /// The text written on the link, empty when it carries none.
    pub label: String,
}

/// A flowchart read out of mermaid source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Graph {
    /// Which way the chart flows.
    pub direction: Direction,
    /// Nodes in the order the source first mentions them.
    pub nodes: Vec<GraphNode>,
    /// Links in source order.
    pub edges: Vec<GraphEdge>,
}

/// Convert a `mermaid`/`graph` block into the board it describes, plus one hidden block per
/// node, or `None` when the source holds no flowchart this module can read.
///
/// Returning `None` rather than an empty board is the whole safety of this: a sequence
/// diagram, a Gantt chart, or a flowchart written in a dialect not read here stays the block
/// the author wrote instead of being replaced by an empty canvas.
///
/// The board keeps the original block's id, so a `nav` entry, a link, or a board node
/// pointing at the diagram still resolves.
pub(crate) fn convert(block: &Block) -> Option<Vec<Block>> {
    let graph = read(&block.text)?;
    if graph.edges.is_empty() && graph.nodes.len() < 2 {
        return None;
    }

    let ids = node_ids(&block.id, &graph);
    let body = layout::arrange(&graph, &ids);
    let mut blocks: Vec<Block> = graph
        .nodes
        .iter()
        .zip(&ids)
        .map(|(node, id)| Block {
            kind: "paragraph".to_string(),
            id: id.clone(),
            text: node.label.clone(),
            hidden: true,
            ..Block::default()
        })
        .collect();

    // No height is stated: a computed number would be true only for this arrangement and
    // stay on the header while edits move the nodes under it. Left unstated, the frame is
    // re-sized from the nodes on every render, so it cannot go stale.
    blocks.push(Block {
        kind: "board".to_string(),
        id: block.id.clone(),
        text: body,
        hidden: block.hidden,
        ..Block::default()
    });
    Some(blocks)
}

/// The document id each of the graph's nodes becomes, in node order.
///
/// Three things have to hold at once. Ids are **prefixed with the board's own id**, because
/// mermaid keys are as short as `A` and a document holding two converted charts would
/// otherwise have both of their `A`s claim one block. They are **lowercased**, because block
/// ids in this format are matched without regard to case and an id written in one case and
/// looked up in another is a node that only appears to work. And they are **disambiguated**,
/// because mermaid keys *are* case-sensitive: a chart with both `A` and `a` is two nodes, and
/// folding them onto one block would silently merge two boxes into one.
fn node_ids(board_id: &str, graph: &Graph) -> Vec<String> {
    let mut ids: Vec<String> = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let wanted = format!("{board_id}-{}", node.key).to_ascii_lowercase();
        let mut id = wanted.clone();
        let mut suffix = 2;
        while ids.contains(&id) {
            id = format!("{wanted}-{suffix}");
            suffix += 1;
        }
        ids.push(id);
    }
    ids
}

/// Read mermaid flowchart source into a [`Graph`], or `None` when it is not one.
///
/// The header line decides: only `graph` and `flowchart` are flowcharts. Every other mermaid
/// diagram type is left alone, because converting one into a box-and-arrow board would be
/// inventing a drawing the author did not make.
pub(crate) fn read(text: &str) -> Option<Graph> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("%%"));

    let header = lines.next()?;
    let (keyword, rest) = header
        .split_once(char::is_whitespace)
        .unwrap_or((header, ""));
    if !matches!(keyword.trim(), "graph" | "flowchart") {
        return None;
    }

    let mut graph = Graph {
        direction: Direction::parse(rest.split(';').next().unwrap_or("")),
        nodes: Vec::new(),
        edges: Vec::new(),
    };

    for line in lines {
        // A statement may be terminated by `;`, and a line may hold several.
        for statement in line.split(';') {
            read_statement(statement.trim(), &mut graph);
        }
    }
    Some(graph)
}

/// Read one statement into `graph`: a link, a bare node declaration, or nothing.
fn read_statement(statement: &str, graph: &mut Graph) {
    if statement.is_empty() || is_directive(statement) {
        return;
    }
    let Some((left, label, right)) = split_link(statement) else {
        // No link: a lone `A[Label]` still declares a node, and its label still counts.
        if let Some(node) = read_endpoint(left_of(statement)) {
            intern(graph, node);
        }
        return;
    };
    let (Some(from), Some(to)) = (read_endpoint(&left), read_endpoint(&right)) else {
        return;
    };
    let from = intern(graph, from);
    let to = intern(graph, to);
    if from == to {
        return;
    }
    // A link stated twice is one link — drawing both put a duplicate, unlabelled arrow on
    // the board. As with [`intern`]'s nodes, a later mention carrying a label wins over an
    // earlier bare one, because the words are the part a reader wrote.
    if let Some(held) = graph
        .edges
        .iter_mut()
        .find(|edge| edge.from == from && edge.to == to)
    {
        if held.label.is_empty() && !label.is_empty() {
            held.label = label;
        }
        return;
    }
    graph.edges.push(GraphEdge { from, to, label });
}

/// Whether a statement is one of the mermaid directives this converter skips.
fn is_directive(statement: &str) -> bool {
    let word = statement
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        word.as_str(),
        "subgraph" | "end" | "click" | "classdef" | "class" | "linkstyle" | "style" | "direction"
    )
}

/// The part of a statement before any trailing junk — used only for a bare node declaration.
fn left_of(statement: &str) -> &str {
    statement.trim()
}

/// Add `node` to the graph if its key is new, and return the index it lives at.
///
/// A later mention carrying a label wins over an earlier bare one: mermaid lets a node be
/// named first and described afterwards, and the description is the part a reader wrote.
fn intern(graph: &mut Graph, node: GraphNode) -> usize {
    if let Some(at) = graph.nodes.iter().position(|held| held.key == node.key) {
        if graph.nodes[at].label == graph.nodes[at].key && node.label != node.key {
            graph.nodes[at].label = node.label;
        }
        return at;
    }
    graph.nodes.push(node);
    graph.nodes.len() - 1
}

/// Split a statement into its left endpoint, the link's label, and its right endpoint.
///
/// Mermaid writes a link's label two ways — `A -->|yes| B` and `A -- yes --> B` — and both
/// arrive here as the same label. The scan skips over bracketed regions, because a node's own
/// label may contain the very characters a link is made of (`A[a -- b] --> C` is one link,
/// not two).
fn split_link(statement: &str) -> Option<(String, String, String)> {
    let bytes = statement.as_bytes();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut index = 0;

    while index < bytes.len() {
        let ch = bytes[index];
        if ch == b'"' {
            quoted = !quoted;
            index += 1;
            continue;
        }
        if !quoted {
            match ch {
                b'[' | b'(' | b'{' => depth += 1,
                b']' | b')' | b'}' => depth -= 1,
                _ => {}
            }
            if depth <= 0 && is_link_byte(ch) && run_len(bytes, index) >= 2 {
                let start = index;
                let end = run_end(bytes, index);
                let (link_end, label) = finish_link(statement, start, end);
                let left = statement[..start].trim().to_string();
                let (right, piped) = strip_pipe_label(statement[link_end..].trim());
                if left.is_empty() || right.is_empty() {
                    return None;
                }
                let label = if piped.is_empty() { label } else { piped };
                return Some((left, label, right));
            }
        }
        index += 1;
    }
    None
}

/// Where the link that starts at `start` ends, and the label written inside it.
///
/// A run that already ends in an arrowhead (`-->`, `--o`, `--x`) is the whole link. Otherwise
/// the source may be the `A -- yes --> B` form, so the next run is looked for; if there is
/// none, the first run was an open link (`A --- B`) and everything after it is the endpoint.
fn finish_link(statement: &str, start: usize, end: usize) -> (usize, String) {
    let bytes = statement.as_bytes();
    if end > start && matches!(bytes[end - 1], b'>' | b'o' | b'x') {
        return (end, String::new());
    }
    let mut index = end;
    while index < bytes.len() {
        if is_link_byte(bytes[index]) && run_len(bytes, index) >= 2 {
            let second = run_end(bytes, index);
            return (second, statement[end..index].trim().to_string());
        }
        index += 1;
    }
    (end, String::new())
}

/// Take a `|label|` written immediately after a link, returning the rest and the label.
fn strip_pipe_label(rest: &str) -> (String, String) {
    let Some(tail) = rest.strip_prefix('|') else {
        return (rest.to_string(), String::new());
    };
    match tail.split_once('|') {
        Some((label, remainder)) => (
            remainder.trim().to_string(),
            unquote(label.trim()).to_string(),
        ),
        None => (rest.to_string(), String::new()),
    }
}

/// Whether a byte can be part of a mermaid link (`-->`, `===`, `-.->`, `--o`, `--x`).
fn is_link_byte(ch: u8) -> bool {
    matches!(ch, b'-' | b'=' | b'.' | b'>' | b'o' | b'x')
}

/// How many link bytes run from `index`, counting only the connector characters.
///
/// `o` and `x` are arrowheads, not connectors, so they never start or lengthen a run — `A -->
/// box` must not read `box` as part of the link.
fn run_len(bytes: &[u8], index: usize) -> usize {
    let mut length = 0;
    while index + length < bytes.len() && matches!(bytes[index + length], b'-' | b'=' | b'.') {
        length += 1;
    }
    length
}

/// The index just past the link run beginning at `index`, arrowhead included.
fn run_end(bytes: &[u8], index: usize) -> usize {
    let mut end = index + run_len(bytes, index);
    if end < bytes.len() && matches!(bytes[end], b'>' | b'o' | b'x') {
        end += 1;
    }
    end
}

/// Read one endpoint — `A`, `A[Label]`, `A(Label)`, `A((Label))`, `A{Label}`, `A>Label]` —
/// into a node, or `None` when it names nothing.
fn read_endpoint(text: &str) -> Option<GraphNode> {
    let text = text.trim();
    let key_len = text.find(['[', '(', '{', '>']).unwrap_or(text.len());
    let key = text[..key_len].trim().to_string();
    if key.is_empty() || !key.chars().all(is_key_char) {
        return None;
    }
    let shape = text[key_len..].trim();
    let label = shape
        .trim_matches(|ch| matches!(ch, '[' | ']' | '(' | ')' | '{' | '}' | '>' | '\\' | '/'))
        .trim();
    let label = unquote(label);
    Some(GraphNode {
        label: if label.is_empty() {
            key.clone()
        } else {
            label.to_string()
        },
        key,
    })
}

/// Whether a character may appear in a mermaid node key.
fn is_key_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

/// Strip one layer of matching quotes from a label.
fn unquote(text: &str) -> &str {
    let trimmed = text.trim();
    for quote in ['"', '\''] {
        if trimmed.len() >= 2 && trimmed.starts_with(quote) && trimmed.ends_with(quote) {
            return trimmed[1..trimmed.len() - 1].trim();
        }
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(source: &str) -> Graph {
        read(source).expect("a flowchart")
    }

    /// A link stated twice is one link — a duplicate drew a second, unlabelled arrow into
    /// the same node — and the mention carrying words wins, whichever order they came in.
    #[test]
    fn a_repeated_link_is_read_once_and_keeps_its_label() {
        let read = graph("flowchart TD\nA-->B\nA-->|yes|B");
        assert_eq!(read.edges.len(), 1, "{:?}", read.edges);
        assert_eq!(read.edges[0].label, "yes");

        let reversed = graph("flowchart TD\nA-->|yes|B\nA-->B");
        assert_eq!(reversed.edges.len(), 1);
        assert_eq!(
            reversed.edges[0].label, "yes",
            "the words survive a bare repeat"
        );
    }

    #[test]
    fn a_header_names_the_direction_and_only_flowcharts_convert() {
        assert_eq!(graph("flowchart LR\nA-->B").direction, Direction::Right);
        assert_eq!(graph("graph TD\nA-->B").direction, Direction::Down);
        assert_eq!(graph("graph BT\nA-->B").direction, Direction::Up);
        // No direction named is mermaid's own default.
        assert_eq!(graph("flowchart\nA-->B").direction, Direction::Down);
        assert!(read("sequenceDiagram\nAlice->>Bob: hi").is_none());
        assert!(read("gantt\ntitle A").is_none());
    }

    #[test]
    fn every_shape_keeps_the_text_inside_it() {
        let read = graph(
            "flowchart LR\n\
             A[square] --> B(round)\n\
             B --> C((circle))\n\
             C --> D{diamond}\n\
             D --> E[[framed]]\n",
        );
        let labels: Vec<&str> = read.nodes.iter().map(|node| node.label.as_str()).collect();
        assert_eq!(labels, ["square", "round", "circle", "diamond", "framed"]);
    }

    #[test]
    fn both_spellings_of_an_edge_label_survive() {
        let piped = graph("flowchart TD\nA -->|yes| B");
        assert_eq!(piped.edges[0].label, "yes");
        let inline = graph("flowchart TD\nA -- no --> B");
        assert_eq!(inline.edges[0].label, "no");
        let dotted = graph("flowchart TD\nA -. maybe .-> B");
        assert_eq!(dotted.edges[0].label, "maybe");
        let thick = graph("flowchart TD\nA == sure ==> B");
        assert_eq!(thick.edges[0].label, "sure");
    }

    #[test]
    fn an_open_link_joins_its_two_nodes_and_carries_no_label() {
        let read = graph("flowchart LR\nA --- B");
        assert_eq!(read.edges.len(), 1);
        assert_eq!(read.edges[0].label, "");
        assert_eq!(read.nodes.len(), 2);
    }

    #[test]
    fn a_label_containing_dashes_is_not_read_as_a_link() {
        // The bug this pins: scanning for `--` without tracking brackets split the node.
        let read = graph("flowchart LR\nA[before -- after] --> B");
        assert_eq!(read.nodes[0].label, "before -- after");
        assert_eq!(read.edges.len(), 1);
        assert_eq!(read.nodes.len(), 2);
    }

    #[test]
    fn a_node_named_bare_and_described_later_keeps_the_description() {
        let read = graph("flowchart LR\nA --> B\nB[the label]");
        assert_eq!(read.nodes[1].label, "the label");
    }

    #[test]
    fn an_arrowhead_letter_is_not_eaten_from_the_next_word() {
        // `--o` and `--x` are arrowheads; `-->` followed by `oven` is not one.
        let read = graph("flowchart LR\nA --> oven");
        assert_eq!(read.nodes[1].key, "oven");
    }

    #[test]
    fn directives_and_comments_are_skipped_rather_than_guessed_at() {
        let read = graph(
            "flowchart LR\n\
             %% a comment\n\
             subgraph one\n\
             A --> B\n\
             end\n\
             classDef big fill:#f00\n\
             click A \"https://example.com\"\n",
        );
        assert_eq!(read.edges.len(), 1);
        assert_eq!(read.nodes.len(), 2);
    }

    #[test]
    fn a_self_link_is_dropped_rather_than_drawn_as_a_loop() {
        let read = graph("flowchart LR\nA --> A");
        assert!(read.edges.is_empty());
        assert_eq!(read.nodes.len(), 1);
    }

    #[test]
    fn a_source_with_nothing_to_draw_converts_to_nothing() {
        let block = Block {
            kind: "graph".to_string(),
            id: "g".to_string(),
            text: "flowchart LR\nA[only me]".to_string(),
            ..Block::default()
        };
        assert!(convert(&block).is_none());
    }

    #[test]
    fn a_converted_board_keeps_the_blocks_id_and_hides_its_nodes() {
        let block = Block {
            kind: "graph".to_string(),
            id: "flow".to_string(),
            text: "flowchart LR\nA[start] --> B[finish]".to_string(),
            ..Block::default()
        };
        let blocks = convert(&block).expect("a board");
        let board = blocks.last().expect("the board is last");
        assert_eq!(board.kind, "board");
        assert_eq!(board.id, "flow");
        assert!(blocks[..blocks.len() - 1]
            .iter()
            .all(|node| node.hidden && node.kind == "paragraph"));
        assert_eq!(blocks[0].id, "flow-a");
        assert_eq!(blocks[0].text, "start");
    }
}
