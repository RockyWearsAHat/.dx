//! Arranging a parsed flowchart into board node lines.
//!
//! A board states every box — `x y w h`, never measured — so converting a flowchart means
//! *deciding* those four numbers for each node. This is a layered arrangement, the same shape
//! every flowchart tool draws: nodes are dealt into ranks by how far along the flow they sit,
//! ordered within their rank to keep the links between ranks from crossing, and then given
//! coordinates. The result is fed through [`crate::render::board::node_line`] rather than
//! formatted here, because that module owns the node-line grammar in both directions.
//!
//! Two properties are guaranteed by construction rather than checked afterwards:
//!
//! - **No two boxes overlap.** Ranks are packed along the flow axis by the deepest box in the
//!   rank before them, and boxes within a rank are stacked with a gap. Nothing can land on
//!   anything.
//! - **Crossings are few.** The ordering pass is the barycentre heuristic — repeatedly place
//!   each node at the average position of the nodes it links to in the neighbouring rank —
//!   swept both ways until it stops improving. It does not prove a minimum (that problem is
//!   NP-hard) but it removes the crossings a reader would notice.

use super::mermaid::{Direction, Graph};
use crate::render::board::{node_line, Edge, Node};

/// Clear space between two ranks, in canvas pixels.
const RANK_GAP: i32 = 110;

/// Clear space between two boxes sharing a rank, in canvas pixels.
const SEAT_GAP: i32 = 44;

/// The narrowest and widest a converted node may be drawn.
const MIN_WIDTH: u32 = 140;
/// The widest a converted node may be drawn before its label wraps instead.
const MAX_WIDTH: u32 = 260;

// A box is *stated*, so these numbers decide whether a label fits inside one. They are read
// off `render::theme`'s own rules for a node rather than guessed: a node is
// `padding: 0.55rem 0.8rem` inside a 1px border, and its body is the page's prose. Anything
// too small here is a label clipped by the box that was supposed to hold it, which is the
// defect these replaced.

/// Roughly the width one character of the page's serif takes, in canvas pixels.
const CHAR_WIDTH: u32 = 10;

/// A node's own chrome across: `0.8rem` of padding either side, plus both borders.
const PADDING_X: u32 = 32;
/// A node's own chrome down: `0.55rem` above and below, plus both borders.
const PADDING_Y: u32 = 26;
/// Height of one line of the page's prose, at its own line height.
const LINE_HEIGHT: u32 = 28;
/// The shortest a node is drawn, so a one-word label still reads as a box.
const MIN_HEIGHT: u32 = 64;

/// How many ordering sweeps to run. Barycentre converges fast; past a handful of passes it
/// only shuffles ties.
const SWEEPS: usize = 6;

/// A finished arrangement: the board's body, and the viewport height that shows it at
/// roughly its stated size.
pub(crate) struct Placed {
    /// The board's body — one node line per node.
    pub body: String,
    /// The viewport height in CSS pixels.
    pub height: u32,
}

/// Arrange `graph` into board node lines.
///
/// The flow axis follows the chart's direction, so a `flowchart LR` reads left to right on
/// the board exactly as it did in the source, and every edge is pinned to the pair of sides
/// that direction implies — an edge that leaves the right and arrives at the left cannot
/// double back around its own node.
pub(crate) fn arrange(graph: &Graph, ids: &[String]) -> Placed {
    let ranks = rank(graph);
    let mut seats = seat(graph, &ranks);
    order(graph, &mut seats);

    let sizes: Vec<(u32, u32)> = graph.nodes.iter().map(|node| size(&node.label)).collect();
    let places = place(&seats, &sizes, graph.direction);

    let (from_side, to_side) = graph.direction.sides();
    let mut lines = Vec::with_capacity(graph.nodes.len());
    for index in 0..graph.nodes.len() {
        let (x, y) = places[index];
        let (w, h) = sizes[index];
        let to = graph
            .edges
            .iter()
            .filter(|edge| edge.from == index)
            .map(|edge| Edge {
                id: ids[edge.to].clone(),
                from: crate::render::board::side_named(&from_side.to_string()),
                to: crate::render::board::side_named(&to_side.to_string()),
                label: edge.label.clone(),
            })
            .collect();
        lines.push(node_line(&Node {
            id: ids[index].clone(),
            x,
            y,
            w,
            h,
            to,
            ..Node::default()
        }));
    }

    let height = viewport_height(&places, &sizes, graph.direction);
    Placed {
        body: lines.join("\n"),
        height,
    }
}

/// Which rank each node belongs to: one past the deepest node that links into it.
///
/// Longest-path ranking over the chart's *forward* edges. A flowchart is perfectly entitled
/// to contain a cycle — "if it fails, go back and fix it" is a loop — and a loop has no
/// longest path at all, so the back edges are found first ([`back_edges`]) and left out of the
/// ranking. They are still drawn; they simply point against the flow, which is what a reader
/// means by one.
///
/// Ranking *with* the back edges was the defect this replaced: each pass round the loop
/// pushed every node in it one rank deeper, and a six-node chart with one loop in it came out
/// as a column fifteen hundred pixels tall.
fn rank(graph: &Graph) -> Vec<usize> {
    let backward = back_edges(graph);
    let mut ranks = vec![0usize; graph.nodes.len()];
    for _ in 0..graph.nodes.len() {
        let mut moved = false;
        for (index, edge) in graph.edges.iter().enumerate() {
            if backward[index] {
                continue;
            }
            if ranks[edge.to] < ranks[edge.from] + 1 {
                ranks[edge.to] = ranks[edge.from] + 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    ranks
}

/// Which edges close a loop, by depth-first search: an edge onto a node already on the
/// current path is the one that turns that path into a cycle.
///
/// Nodes are visited in source order, so which edge of a loop is called the back edge is
/// decided by the order the author wrote them — the same chart always arranges the same way.
fn back_edges(graph: &Graph) -> Vec<bool> {
    /// Where a node stands in the search: untouched, on the current path, or finished with.
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Fresh,
        Open,
        Done,
    }

    let mut state = vec![State::Fresh; graph.nodes.len()];
    let mut backward = vec![false; graph.edges.len()];
    // An explicit stack of (node, how many of its edges have been taken), because a chart may
    // be deeper than the call stack is willing to go.
    let mut stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..graph.nodes.len() {
        if state[start] != State::Fresh {
            continue;
        }
        state[start] = State::Open;
        stack.push((start, 0));
        while let Some((node, taken)) = stack.pop() {
            let next = graph
                .edges
                .iter()
                .enumerate()
                .filter(|(_, edge)| edge.from == node)
                .nth(taken);
            let Some((index, edge)) = next else {
                state[node] = State::Done;
                continue;
            };
            stack.push((node, taken + 1));
            match state[edge.to] {
                // Onto a node we are standing on: this edge closes the loop.
                State::Open => backward[index] = true,
                State::Fresh => {
                    state[edge.to] = State::Open;
                    stack.push((edge.to, 0));
                }
                State::Done => {}
            }
        }
    }
    backward
}

/// Group node indices by rank, in first-mention order within each rank.
fn seat(graph: &Graph, ranks: &[usize]) -> Vec<Vec<usize>> {
    let depth = ranks.iter().copied().max().map_or(0, |top| top + 1);
    let mut seats = vec![Vec::new(); depth];
    for index in 0..graph.nodes.len() {
        seats[ranks[index]].push(index);
    }
    seats
}

/// Reorder each rank so the links between neighbouring ranks cross as little as possible.
///
/// One sweep down then one sweep up, repeated: a node moves to the average seat of the nodes
/// it is joined to in the rank being swept from. Nodes with no such neighbour keep their seat,
/// so an unconnected node does not drift to the front of the rank.
fn order(graph: &Graph, seats: &mut [Vec<usize>]) {
    for sweep in 0..SWEEPS {
        let downward = sweep % 2 == 0;
        let indices: Vec<usize> = if downward {
            (1..seats.len()).collect()
        } else {
            (0..seats.len().saturating_sub(1)).rev().collect()
        };
        for rank in indices {
            let neighbour = if downward { rank - 1 } else { rank + 1 };
            let positions = seat_positions(&seats[neighbour]);
            let mut ranked: Vec<(usize, f64, usize)> = seats[rank]
                .iter()
                .enumerate()
                .map(|(seat, &node)| {
                    let centre = barycentre(graph, node, &positions, downward);
                    (node, centre.unwrap_or(seat as f64), seat)
                })
                .collect();
            // The original seat breaks a tie, so an arrangement is stated, not lucky.
            ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
            seats[rank] = ranked.into_iter().map(|(node, _, _)| node).collect();
        }
    }
}

/// Each node's seat within a rank, indexed by node.
fn seat_positions(rank: &[usize]) -> Vec<(usize, f64)> {
    rank.iter()
        .enumerate()
        .map(|(seat, &node)| (node, seat as f64))
        .collect()
}

/// The average seat of the nodes `node` joins in the neighbouring rank, or `None` when it
/// joins none of them.
fn barycentre(
    graph: &Graph,
    node: usize,
    positions: &[(usize, f64)],
    downward: bool,
) -> Option<f64> {
    let seat_of = |wanted: usize| {
        positions
            .iter()
            .find(|(index, _)| *index == wanted)
            .map(|(_, seat)| *seat)
    };
    let seats: Vec<f64> = graph
        .edges
        .iter()
        .filter_map(|edge| {
            // Sweeping down, a node is placed by what points *into* it; sweeping up, by what
            // it points *at*.
            let other = match (downward, edge.from == node, edge.to == node) {
                (true, _, true) => edge.from,
                (false, true, _) => edge.to,
                _ => return None,
            };
            seat_of(other)
        })
        .collect();
    if seats.is_empty() {
        return None;
    }
    Some(seats.iter().sum::<f64>() / seats.len() as f64)
}

/// Give every node its `x`,`y`, packing ranks along the flow axis and stacking within them.
///
/// Each rank is centred on the cross axis, which is what makes a chain of single nodes come
/// out as a straight line rather than a staircase pinned to one margin.
fn place(seats: &[Vec<usize>], sizes: &[(u32, u32)], direction: Direction) -> Vec<(i32, i32)> {
    let horizontal = direction.is_horizontal();
    let mut places = vec![(0i32, 0i32); sizes.len()];

    // Where each rank starts along the flow axis.
    let mut flow = 0i32;
    let mut rank_flow = Vec::with_capacity(seats.len());
    for rank in seats {
        rank_flow.push(flow);
        let deepest = rank
            .iter()
            .map(|&node| {
                if horizontal {
                    sizes[node].0
                } else {
                    sizes[node].1
                }
            })
            .max()
            .unwrap_or(0);
        flow += i32::try_from(deepest).unwrap_or(0) + RANK_GAP;
    }

    // A rank's extent across the flow, so it can be centred against the widest rank.
    let spans: Vec<i32> = seats
        .iter()
        .map(|rank| {
            let boxes: i32 = rank
                .iter()
                .map(|&node| {
                    i32::try_from(if horizontal {
                        sizes[node].1
                    } else {
                        sizes[node].0
                    })
                    .unwrap_or(0)
                })
                .sum();
            boxes + SEAT_GAP * i32::try_from(rank.len().saturating_sub(1)).unwrap_or(0)
        })
        .collect();
    let widest = spans.iter().copied().max().unwrap_or(0);

    for (index, rank) in seats.iter().enumerate() {
        let mut cross = (widest - spans[index]) / 2;
        for &node in rank {
            let (w, h) = sizes[node];
            let along = rank_flow[index];
            // A backwards direction lays the ranks out the same way and mirrors at the end,
            // so the arithmetic above never has to know which way the chart points.
            places[node] = if horizontal {
                (along, cross)
            } else {
                (cross, along)
            };
            cross += i32::try_from(if horizontal { h } else { w }).unwrap_or(0) + SEAT_GAP;
        }
    }

    if !direction.is_forward() {
        let furthest = flow - RANK_GAP;
        for (index, place) in places.iter_mut().enumerate() {
            let (w, h) = sizes[index];
            if horizontal {
                place.0 = furthest - place.0 - i32::try_from(w).unwrap_or(0);
            } else {
                place.1 = furthest - place.1 - i32::try_from(h).unwrap_or(0);
            }
        }
    }

    places
}

/// The box a label needs: wide enough to read, never wider than the page's own column.
///
/// Stated from the character count rather than measured, because a board's boxes are stated
/// — the same rule that lets a browser, a PNG, and a terminal lay one board out identically.
fn size(label: &str) -> (u32, u32) {
    let chars = u32::try_from(label.chars().count()).unwrap_or(u32::MAX / CHAR_WIDTH);
    let wanted = chars.saturating_mul(CHAR_WIDTH).saturating_add(PADDING_X);
    let width = wanted.clamp(MIN_WIDTH, MAX_WIDTH);
    let per_line = (width.saturating_sub(PADDING_X) / CHAR_WIDTH).max(1);
    let lines = chars.div_ceil(per_line).max(1);
    (width, (PADDING_Y + lines * LINE_HEIGHT).max(MIN_HEIGHT))
}

/// A viewport tall enough to hold the arrangement at roughly its stated size.
fn viewport_height(places: &[(i32, i32)], sizes: &[(u32, u32)], direction: Direction) -> u32 {
    /// Room above and below the arrangement, matching the renderer's own fit padding.
    const MARGIN: i32 = 48;
    /// A board is a block on a page, never the whole of one.
    const TALLEST: u32 = 720;
    /// Below this a board reads as a stripe rather than a canvas.
    const SHORTEST: u32 = 220;

    let extent = places
        .iter()
        .zip(sizes)
        .map(|((_, y), (_, h))| y + i32::try_from(*h).unwrap_or(0))
        .max()
        .unwrap_or(0);
    let top = places.iter().map(|(_, y)| *y).min().unwrap_or(0);
    let tall = u32::try_from(extent - top + MARGIN * 2).unwrap_or(SHORTEST);
    // A horizontal chart is short and wide; the fit will scale it down to the column, and a
    // tall viewport under a short chart is a block of blank paper.
    let wanted = if direction.is_horizontal() {
        tall.min(480)
    } else {
        tall
    };
    wanted.clamp(SHORTEST, TALLEST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::mermaid::read;
    use crate::render::board::nodes;

    /// Node ids the way `mermaid::convert` makes them, so a test reads what ships.
    fn ids(board: &str, graph: &crate::format::mermaid::Graph) -> Vec<String> {
        graph
            .nodes
            .iter()
            .map(|node| format!("{board}-{}", node.key).to_ascii_lowercase())
            .collect()
    }

    fn placed(source: &str) -> Vec<crate::render::board::Node> {
        let graph = read(source).expect("a flowchart");
        nodes(&arrange(&graph, &ids("g", &graph)).body)
    }

    /// Whether two placed boxes cover any of the same canvas.
    fn overlap(a: &crate::render::board::Node, b: &crate::render::board::Node) -> bool {
        let right = |n: &crate::render::board::Node| n.x + i32::try_from(n.w).unwrap_or(0);
        let bottom = |n: &crate::render::board::Node| n.y + i32::try_from(n.h).unwrap_or(0);
        a.x < right(b) && b.x < right(a) && a.y < bottom(b) && b.y < bottom(a)
    }

    #[test]
    fn no_two_boxes_of_an_arrangement_ever_overlap() {
        for source in [
            "flowchart LR\nA-->B\nA-->C\nA-->D\nB-->E\nC-->E\nD-->E",
            "flowchart TD\nA-->B\nB-->C\nC-->A",
            "graph TD\nA-->B\nA-->C\nB-->D\nC-->D\nD-->E\nE-->F\nF-->A",
            "flowchart LR\nA[a very long label indeed, wrapping]-->B[b]",
        ] {
            let list = placed(source);
            for (i, one) in list.iter().enumerate() {
                for other in &list[i + 1..] {
                    assert!(
                        !overlap(one, other),
                        "{} and {} overlap in:\n{source}",
                        one.id,
                        other.id
                    );
                }
            }
        }
    }

    /// A loop is an ordinary thing for a flowchart to contain, and must not stretch the
    /// arrangement: ranking with the back edge in it pushed every node round the loop one
    /// rank deeper on every pass, and a six-node chart came out fifteen hundred px tall.
    #[test]
    fn a_loop_does_not_stretch_the_arrangement() {
        let looped = placed(
            "flowchart TD\n\
             A[write] --> B{parses?}\n\
             B -->|yes| C[render]\n\
             B -->|no| D[say why]\n\
             D --> A\n\
             C --> E[read]",
        );
        let deepest = looped
            .iter()
            .map(|node| node.y + i32::try_from(node.h).unwrap_or(0))
            .max()
            .expect("nodes");
        // Four ranks deep: write, parses?, render/say why, read.
        assert!(deepest < 600, "the loop stretched the chart to {deepest}px");

        // And the back edge is still drawn.
        let from_d = looped
            .iter()
            .find(|node| node.id == "g-d")
            .expect("the `say why` node");
        assert_eq!(from_d.to[0].id, "g-a");
    }

    #[test]
    fn a_chain_lays_out_along_the_direction_it_names() {
        let right = placed("flowchart LR\nA-->B\nB-->C");
        assert!(
            right[0].x < right[1].x && right[1].x < right[2].x,
            "{right:?}"
        );
        assert_eq!(right[0].y, right[1].y, "a chain is a straight line");

        let down = placed("flowchart TD\nA-->B\nB-->C");
        assert!(down[0].y < down[1].y && down[1].y < down[2].y, "{down:?}");
        assert_eq!(down[0].x, down[1].x);
    }

    #[test]
    fn a_reversed_direction_mirrors_the_flow() {
        let up = placed("flowchart BT\nA-->B\nB-->C");
        assert!(up[0].y > up[1].y && up[1].y > up[2].y, "{up:?}");
        let left = placed("flowchart RL\nA-->B\nB-->C");
        assert!(left[0].x > left[1].x && left[1].x > left[2].x, "{left:?}");
    }

    #[test]
    fn edges_are_pinned_to_the_pair_of_sides_the_direction_implies() {
        let list = placed("flowchart LR\nA-->B");
        let edge = &list[0].to[0];
        assert_eq!(edge.id, "g-b");
        assert_eq!(
            edge.from
                .map(super::super::super::render::board::Side::letter),
            Some('r')
        );
        assert_eq!(
            edge.to
                .map(super::super::super::render::board::Side::letter),
            Some('l')
        );
    }

    #[test]
    fn an_edge_label_survives_the_arrangement() {
        let list = placed("flowchart TD\nA -->|yes| B");
        assert_eq!(list[0].to[0].label, "yes");
    }

    #[test]
    fn ordering_removes_the_crossing_a_reader_would_notice() {
        // Q and P are declared in the order Q, P, but A links to P and B links to Q. Left in
        // source order the two links cross; the barycentre sweep should swap the second rank
        // so each target sits under the node that points at it.
        let list = placed("flowchart TD\nQ\nP\nA --> P\nB --> Q");
        let across = |id: &str| {
            list.iter()
                .find(|node| node.id == format!("g-{id}").to_ascii_lowercase())
                .map(|node| node.x)
                .expect(id)
        };
        assert_eq!(
            across("A") < across("B"),
            across("P") < across("Q"),
            "the links should not cross: {list:?}"
        );
    }

    #[test]
    fn a_long_label_wraps_into_a_taller_box_rather_than_a_wider_one() {
        let (narrow, short) = size("hi");
        let (wide, tall) = size("a label long enough that it has to wrap onto several lines");
        assert_eq!(narrow, MIN_WIDTH);
        assert_eq!(wide, MAX_WIDTH);
        assert!(tall > short, "{tall} should exceed {short}");
    }
}
