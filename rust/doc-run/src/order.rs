//! Execution order: the waves of independent blocks a default run overlaps, and, for
//! `--follow-edges`, the document's boards as a dependency graph.
//!
//! A board's edge says *this, then that*, and [`RunOptions::follow_board_edges`] takes it
//! at its word: a runnable block waits for every runnable block with an edge path into it.
//! Non-runnable nodes conduct order without joining it — `setup -> note -> test` still
//! runs `setup` before `test` — and edges naming blocks the document does not have are
//! ignored, exactly as the renderer tolerates them by not drawing them.
//!
//! **The rule, stated once:** at every step the earliest *ready* runnable block (by
//! document position) — one whose edge prerequisites have all run — goes next. An edge
//! that defers a block therefore lets every later ready block run before it, whether
//! those later blocks sit on a board or not. Ties always break by document order, so the
//! result is deterministic. Document-order side-effect dependencies between blocks the
//! boards leave unrelated are **not** preserved — state an edge if you need an order.
//!
//! A cycle among runnable blocks has no order to give, so it is an error sentence naming
//! the blocks in the cycle — never a hang, never a silent fall back to document order.
//!
//! [`RunOptions::follow_board_edges`]: crate::RunOptions::follow_board_edges

use std::collections::{BTreeSet, HashMap, HashSet};

use doc_core::model::Document;
use doc_core::render::board_edges;

/// The order `--follow-edges` runs the blocks at `runnable` (indices into
/// `document.blocks`, in document order), or the cycle sentence when the boards state no
/// order at all.
#[cfg(test)]
pub(crate) fn edge_order(document: &Document, runnable: &[usize]) -> Result<Vec<usize>, String> {
    edge_schedule(document, runnable).map(|(order, _)| order)
}

/// The run order and the direct `(before, after)` constraints between runnable blocks.
pub(crate) type EdgeSchedule = (Vec<usize>, HashSet<(usize, usize)>);

/// The `--follow-edges` order plus the direct `(before, after)` constraints behind it, so a
/// parallel run can start a block the moment its own edge predecessors finish.
pub(crate) fn edge_schedule(
    document: &Document,
    runnable: &[usize],
) -> Result<EdgeSchedule, String> {
    let constraints = constraints(document, runnable);
    let order = topological(document, runnable, &constraints)?;
    Ok((order, constraints))
}

/// What a block declares it touches, for deciding what may run beside it: its `reads=`
/// paths and its `writes=` folders, as validated by the header parsers.
#[derive(Debug, Clone, Default)]
pub(crate) struct Footprint {
    /// Declared `reads=` paths.
    pub reads: Vec<String>,
    /// Declared `writes=` folders.
    pub writes: Vec<String>,
}

/// Group blocks, already in run order, into waves that may run concurrently.
///
/// **The rule:** consecutive blocks share a wave unless a later block reads a path an
/// earlier block in the wave writes, or either block declares no `reads=` at all (`None`
/// here). A block with no declared reads could depend on anything, so it keeps document
/// order semantics: it runs alone, after everything before it and before everything after.
/// Each wave holds the indices it was given, in the order given.
pub(crate) fn waves(blocks: &[(usize, Option<Footprint>)]) -> Vec<Vec<usize>> {
    let mut waves: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_writes: Vec<&str> = Vec::new();
    for (index, footprint) in blocks {
        let Some(footprint) = footprint.as_ref().filter(|f| !f.reads.is_empty()) else {
            if !current.is_empty() {
                waves.push(std::mem::take(&mut current));
            }
            waves.push(vec![*index]);
            current_writes.clear();
            continue;
        };
        let conflicts = footprint
            .reads
            .iter()
            .any(|read| current_writes.iter().any(|write| overlaps(read, write)));
        if conflicts {
            waves.push(std::mem::take(&mut current));
            current_writes.clear();
        }
        current.push(*index);
        current_writes.extend(footprint.writes.iter().map(String::as_str));
    }
    if !current.is_empty() {
        waves.push(current);
    }
    waves
}

/// Whether reading `read` could observe what writing `write` changes: the same path, or
/// one inside the other. A read that climbs with `..` cannot be compared by spelling, so
/// it is taken to overlap every write.
fn overlaps(read: &str, write: &str) -> bool {
    let read = normalized(read);
    let write = normalized(write);
    if read.split('/').any(|part| part == "..") || read.is_empty() || write.is_empty() {
        return true;
    }
    read == write
        || read.starts_with(&format!("{write}/"))
        || write.starts_with(&format!("{read}/"))
}

/// A declared path without `./` prefixes, `.` segments, or trailing slashes.
fn normalized(path: &str) -> String {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// The direct order constraints between runnable blocks: `(before, after)` index pairs.
///
/// Each pair is a board edge, or a chain of board edges whose interior nodes are all
/// non-runnable — the conduction that keeps `setup -> note -> test` an order even though
/// a note runs nothing.
fn constraints(document: &Document, runnable: &[usize]) -> HashSet<(usize, usize)> {
    let position: HashMap<&str, usize> = document
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.id.as_str(), index))
        .collect();
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    for (from, to) in board_edges(document) {
        if let (Some(&from), Some(&to)) = (position.get(from.as_str()), position.get(to.as_str())) {
            successors.entry(from).or_default().push(to);
        }
    }

    let runnable_set: HashSet<usize> = runnable.iter().copied().collect();
    let mut pairs = HashSet::new();
    for &start in runnable {
        // Walk forward from one runnable block, passing through non-runnable nodes and
        // stopping at the first runnable one on each path — the transitive closure past
        // that point is the topological sort's own job.
        let mut stack: Vec<usize> = successors.get(&start).cloned().unwrap_or_default();
        let mut seen: HashSet<usize> = HashSet::new();
        while let Some(next) = stack.pop() {
            if !seen.insert(next) {
                continue;
            }
            if runnable_set.contains(&next) {
                pairs.insert((start, next));
                continue;
            }
            stack.extend(successors.get(&next).iter().flat_map(|edges| edges.iter()));
        }
    }
    pairs
}

/// Kahn's algorithm over the constraint pairs, always emitting the earliest ready block
/// by document position — which is what keeps unconstrained blocks in document order.
fn topological(
    document: &Document,
    runnable: &[usize],
    pairs: &HashSet<(usize, usize)>,
) -> Result<Vec<usize>, String> {
    let mut indegree: HashMap<usize, usize> = runnable.iter().map(|&index| (index, 0)).collect();
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(before, after) in pairs {
        *indegree.entry(after).or_default() += 1;
        successors.entry(before).or_default().push(after);
    }

    let mut ready: BTreeSet<usize> = runnable
        .iter()
        .copied()
        .filter(|index| indegree[index] == 0)
        .collect();
    let mut order = Vec::with_capacity(runnable.len());
    while let Some(&next) = ready.iter().next() {
        ready.remove(&next);
        order.push(next);
        for after in successors.remove(&next).unwrap_or_default() {
            if let Some(waiting) = indegree.get_mut(&after) {
                *waiting -= 1;
                if *waiting == 0 {
                    ready.insert(after);
                }
            }
        }
    }

    if order.len() == runnable.len() {
        Ok(order)
    } else {
        let placed: HashSet<usize> = order.into_iter().collect();
        let stuck: BTreeSet<usize> = runnable
            .iter()
            .copied()
            .filter(|index| !placed.contains(index))
            .collect();
        Err(cycle_sentence(document, &stuck, pairs))
    }
}

/// The error naming one cycle among the blocks the topological sort could not place.
///
/// Walked over predecessors: every stuck block has at least one stuck predecessor (that
/// is what kept its indegree above zero), so following them must revisit a block — and
/// the revisited stretch, reversed, is a cycle in edge direction.
fn cycle_sentence(
    document: &Document,
    stuck: &BTreeSet<usize>,
    pairs: &HashSet<(usize, usize)>,
) -> String {
    let fallback = "blocks on this document's boards form a cycle; \
                    --follow-edges needs an order"
        .to_string();
    let Some(&start) = stuck.iter().next() else {
        return fallback;
    };
    let mut trail = vec![start];
    let mut here = start;
    loop {
        let Some(previous) = stuck
            .iter()
            .copied()
            .find(|&candidate| pairs.contains(&(candidate, here)))
        else {
            return fallback;
        };
        if let Some(seen) = trail.iter().position(|&step| step == previous) {
            // The revisited stretch, reversed, follows edge direction; rotate it to open
            // on the block earliest in the document, so the sentence is deterministic.
            let mut cycle: Vec<usize> = trail[seen..].iter().rev().copied().collect();
            let earliest = cycle
                .iter()
                .enumerate()
                .min_by_key(|(_, &index)| index)
                .map_or(0, |(position, _)| position);
            cycle.rotate_left(earliest);
            let mut ids: Vec<&str> = cycle
                .iter()
                .map(|&index| document.blocks[index].id.as_str())
                .collect();
            ids.push(ids[0]);
            return format!(
                "blocks {} form a cycle; --follow-edges needs an order",
                ids.join(" -> ")
            );
        }
        trail.push(previous);
        here = previous;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    /// Indices of the runnable-looking blocks in these fixtures: every `::code … run`.
    fn runnable(document: &Document) -> Vec<usize> {
        document
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.kind == "code" && block.run)
            .map(|(index, _)| index)
            .collect()
    }

    fn code(id: &str) -> String {
        format!("::code id={id} lang=bash run hidden\necho {id}\n::end\n\n")
    }

    fn footprint(reads: &[&str], writes: &[&str]) -> Option<Footprint> {
        Some(Footprint {
            reads: reads.iter().map(|path| path.to_string()).collect(),
            writes: writes.iter().map(|path| path.to_string()).collect(),
        })
    }

    #[test]
    fn disjoint_declared_blocks_share_one_wave() {
        let blocks = vec![
            (0, footprint(&["a"], &["target"])),
            (2, footprint(&["b"], &["target"])),
            (4, footprint(&["c"], &[])),
        ];
        assert_eq!(waves(&blocks), vec![vec![0, 2, 4]]);
    }

    #[test]
    fn a_reader_of_an_earlier_write_starts_the_next_wave() {
        let blocks = vec![
            (0, footprint(&["src"], &["out"])),
            (1, footprint(&["other"], &[])),
            (2, footprint(&["out/made.txt"], &[])),
            (3, footprint(&["./out/"], &[])),
        ];
        assert_eq!(waves(&blocks), vec![vec![0, 1], vec![2, 3]]);
        // A folder read containing the written folder conflicts too.
        let blocks = vec![
            (0, footprint(&["a"], &["rust/target"])),
            (1, footprint(&["rust"], &[])),
        ];
        assert_eq!(waves(&blocks), vec![vec![0], vec![1]]);
    }

    #[test]
    fn a_block_without_reads_runs_alone_in_order() {
        let blocks = vec![
            (0, footprint(&["a"], &[])),
            (1, None),
            (2, footprint(&[], &["x"])),
            (3, footprint(&["b"], &[])),
            (4, footprint(&["c"], &[])),
        ];
        assert_eq!(waves(&blocks), vec![vec![0], vec![1], vec![2], vec![3, 4]]);
    }

    #[test]
    fn edges_order_execution_against_document_order() {
        // Document order is test, note, setup; the board says setup -> note -> test.
        let source = format!(
            "{}{}::paragraph id=note hidden\nbetween\n::end\n\n\
             ::board id=plan\n- setup x=0 y=0 to=note\n- note x=0 y=200 to=test\n- test x=0 y=400\n::end\n",
            code("test"),
            code("setup"),
        );
        let document = parse(&source);
        let order = edge_order(&document, &runnable(&document)).expect("acyclic");
        let ids: Vec<&str> = order
            .iter()
            .map(|&index| document.blocks[index].id.as_str())
            .collect();
        // The non-runnable note conducted the order: setup still precedes test.
        assert_eq!(ids, vec!["setup", "test"]);
    }

    #[test]
    fn an_unconstrained_block_runs_at_its_earliest_ready_moment() {
        // Document order is test, aside, setup; only setup -> test is stated, so aside —
        // on no board, hence never deferred — runs first among the blocks that are ready.
        let source = format!(
            "{}{}{}::board id=plan\n- setup x=0 y=0 to=test\n- test x=0 y=200\n::end\n",
            code("test"),
            code("aside"),
            code("setup"),
        );
        let document = parse(&source);
        let order = edge_order(&document, &runnable(&document)).expect("acyclic");
        let ids: Vec<&str> = order
            .iter()
            .map(|&index| document.blocks[index].id.as_str())
            .collect();
        assert_eq!(ids, vec!["aside", "setup", "test"]);
    }

    #[test]
    fn a_deferred_block_is_overtaken_by_later_blocks_on_no_board() {
        // Document order is a, b, c; the board states only c -> a, and b is on no board.
        // Deferring a makes b and c ready first, so b — a block the boards never
        // mention — overtakes it. This is the stated rule, pinned: an edge that defers a
        // block lets later ready blocks run before it, on-board or off.
        let source = format!(
            "{}{}{}::board id=plan\n- c x=0 y=0 to=a\n- a x=0 y=200\n::end\n",
            code("a"),
            code("b"),
            code("c"),
        );
        let document = parse(&source);
        let order = edge_order(&document, &runnable(&document)).expect("acyclic");
        let ids: Vec<&str> = order
            .iter()
            .map(|&index| document.blocks[index].id.as_str())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn independent_chains_tie_break_by_document_order() {
        let source = format!(
            "{}{}{}{}::board id=plan\n- a x=0 y=0 to=c\n- b x=0 y=100 to=d\n- c x=0 y=200\n- d x=0 y=300\n::end\n",
            code("a"),
            code("b"),
            code("c"),
            code("d"),
        );
        let document = parse(&source);
        let first = edge_order(&document, &runnable(&document)).expect("acyclic");
        let ids: Vec<&str> = first
            .iter()
            .map(|&index| document.blocks[index].id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
        // Deterministic: the same document always yields the same order.
        assert_eq!(
            first,
            edge_order(&document, &runnable(&document)).expect("acyclic")
        );
    }

    #[test]
    fn a_cycle_is_an_error_sentence_naming_its_blocks() {
        let source = format!(
            "{}{}::board id=plan\n- a x=0 y=0 to=b\n- b x=0 y=200 to=a\n::end\n",
            code("a"),
            code("b"),
        );
        let document = parse(&source);
        let sentence = edge_order(&document, &runnable(&document)).expect_err("a cycle");
        assert_eq!(
            sentence,
            "blocks a -> b -> a form a cycle; --follow-edges needs an order"
        );
    }

    #[test]
    fn an_edge_naming_a_missing_block_is_ignored() {
        let source = format!(
            "{}::board id=plan\n- a x=0 y=0 to=ghost\n- ghost x=0 y=200 to=a\n::end\n",
            code("a"),
        );
        let document = parse(&source);
        let order = edge_order(&document, &runnable(&document)).expect("no ghost cycle");
        assert_eq!(order.len(), 1);
    }

    #[test]
    fn a_cycle_among_non_runnable_nodes_neither_hangs_nor_errors() {
        let source = format!(
            "{}::paragraph id=p hidden\none\n::end\n\n::paragraph id=q hidden\ntwo\n::end\n\n\
             ::board id=plan\n- p x=0 y=0 to=q\n- q x=0 y=200 to=p,a\n- a x=0 y=400\n::end\n",
            code("a"),
        );
        let document = parse(&source);
        let order = edge_order(&document, &runnable(&document)).expect("prose imposes no order");
        assert_eq!(order.len(), 1);
    }
}
