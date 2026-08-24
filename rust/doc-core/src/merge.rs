//! Three-way merge of two revisions of one document against their common ancestor.
//!
//! # Why a document merges by block and not by line
//! A `.dx` file on disk is a pointer, so git's line merge never sees the document at all —
//! it sees one digest line and calls two different digests a conflict. The content lives in
//! `.doc/repo.dxcp`, which git reads as an opaque binary blob and refuses outright. The
//! result was that *any* two branches that both touched documents produced an unresolvable
//! conflict on the one file every dx project must commit.
//!
//! The document, though, is already a sequence of independently addressed pieces: one chunk
//! per block, keyed by the block's id ([`crate::chunk::split`]). That is the natural merge
//! unit, and it is a better one than a line — ticking a checkbox on `master` while a branch
//! rewrites a paragraph two blocks down are edits to two different blocks, and a block-keyed
//! merge takes both without ever asking a human. Only two edits to the *same* block conflict.
//!
//! # Ids are the key, and the format guarantees them
//! [`crate::format::stringify_blocks`] normalizes a document before writing it, and
//! normalization makes every id unique within the document. So the map from id to canonical
//! block text is total and injective on both sides, which is what makes the three-way
//! comparison below a comparison of *the same block* rather than of whatever happened to
//! land at the same index.
//!
//! # A conflict is written the way git writes one
//! When both sides changed one block differently there is no answer, so the block is emitted
//! between `<<<<<<<` / `=======` / `>>>>>>>` markers exactly as git's own drivers do. The
//! result is no longer canonical DOCSRC — deliberately. It is plain text a person edits and
//! `dx sync` adopts, and [`has_conflict_markers`] is what stops anything adopting it before
//! the person has.
//!
//! This module is host-free (no filesystem, no clock, no git), so it compiles to `wasm32`
//! with the rest of `doc-core` and can be tested without a repository.

use std::collections::{BTreeMap, BTreeSet};

use crate::format::{parse, stringify, stringify_blocks, BLOCK_SEPARATOR};

/// The default number of characters in a conflict marker run, matching git's own.
pub const DEFAULT_MARKER_SIZE: usize = 7;

/// The label written beside the `<<<<<<<` marker, for the side being merged into.
const OURS_LABEL: &str = "ours";
/// The label written beside the `>>>>>>>` marker, for the side being merged in.
const THEIRS_LABEL: &str = "theirs";

/// The outcome of merging one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The merged text. Canonical DOCSRC when [`Merged::conflicts`] is empty, and plain text
    /// carrying conflict markers when it is not.
    pub text: String,
    /// The id of every block the two sides changed differently, in document order.
    pub conflicts: Vec<String>,
}

impl Merged {
    /// Whether the merge needs no human.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Merge `ours` and `theirs` against their common `ancestor`.
///
/// `ancestor` is `None` when the two sides added the document independently — git's
/// "added by both" case — and the merge then has nothing to attribute a change to, so any
/// block the two sides spell differently is a conflict.
///
/// `marker_size` is the length of each conflict marker run; pass [`DEFAULT_MARKER_SIZE`]
/// unless git named a different one (`%L`).
///
/// Complexity: `O(n log n)` in the total number of blocks across the three revisions.
#[must_use]
pub fn merge_documents(
    ancestor: Option<&str>,
    ours: &str,
    theirs: &str,
    marker_size: usize,
) -> Merged {
    // The cheap answers first, and they are the common ones: a merge where only one side
    // moved must return that side's *exact* bytes, never a re-canonicalized approximation.
    if ours == theirs {
        return clean(ours);
    }
    if let Some(base) = ancestor {
        if base == ours {
            return clean(theirs);
        }
        if base == theirs {
            return clean(ours);
        }
    }

    let ours = Revision::of(ours);
    let theirs = Revision::of(theirs);
    let base = ancestor.map(Revision::of);
    let base_blocks = base.as_ref().map_or(&EMPTY, |revision| &revision.blocks);

    let mut pieces: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for id in weave(&ours.order, &theirs.order) {
        let mine = ours.blocks.get(&id);
        let yours = theirs.blocks.get(&id);
        let was = base_blocks.get(&id);
        match resolve(was, mine, yours) {
            Resolution::Take(text) => pieces.push(text.to_string()),
            Resolution::Drop => {}
            Resolution::Clash => {
                conflicts.push(id);
                pieces.push(clash(
                    mine.map(String::as_str).unwrap_or_default(),
                    yours.map(String::as_str).unwrap_or_default(),
                    marker_size,
                ));
            }
        }
    }

    Merged {
        text: assemble(&pieces),
        conflicts,
    }
}

/// Whether `text` still carries git conflict markers, and so is nobody's document yet.
///
/// A marker is only a marker at the start of a line: a body line that *mentions* `=======`
/// mid-sentence is prose, and a document that could not talk about conflict markers would be
/// a poor format to document a merge driver in.
#[must_use]
pub fn has_conflict_markers(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_end();
        starts_run(trimmed, '<') || starts_run(trimmed, '=') || starts_run(trimmed, '>')
    })
}

/// Whether `line` opens with at least [`DEFAULT_MARKER_SIZE`] copies of `marker`.
fn starts_run(line: &str, marker: char) -> bool {
    let run = line.chars().take_while(|found| *found == marker).count();
    run >= DEFAULT_MARKER_SIZE
}

/// An empty block map, so a merge with no ancestor needs no allocation to look one up in.
static EMPTY: BTreeMap<String, String> = BTreeMap::new();

/// One revision, reduced to what a merge compares: its blocks by id, and their order.
struct Revision {
    /// Canonical text of each block, keyed by the block's normalized id.
    blocks: BTreeMap<String, String>,
    /// The ids in document order.
    order: Vec<String>,
}

impl Revision {
    /// Read `source` into blocks keyed by id.
    ///
    /// The source is canonicalized once before it is split, so the ids compared here are the
    /// normalized ones — the same ids storage addresses the chunks by. Text that is not
    /// canonical (a document a person just hand-edited) therefore merges against a stored
    /// one without every block reading as changed.
    fn of(source: &str) -> Self {
        let canonical = stringify(&parse(source));
        let document = parse(&canonical);
        let texts = stringify_blocks(&document);
        let mut blocks = BTreeMap::new();
        let mut order = Vec::with_capacity(texts.len());
        for (block, text) in document.blocks.iter().zip(texts) {
            // Normalization made ids unique, so the first writer of an id is also the only
            // one; `entry` keeps that true even if a future normalization stops guaranteeing it.
            if blocks.contains_key(&block.id) {
                continue;
            }
            order.push(block.id.clone());
            blocks.insert(block.id.clone(), text);
        }
        Self { blocks, order }
    }
}

/// What to do with one block.
enum Resolution<'a> {
    /// Keep this text.
    Take(&'a str),
    /// The block is gone from the merged document.
    Drop,
    /// Both sides changed it, differently.
    Clash,
}

/// Decide one block from its three versions.
///
/// Read the arms as a table: whichever side still matches the ancestor did not change the
/// block, so the other side's version is simply the newer one. Everything else is either
/// agreement (both sides wrote the same thing, including both deleting it) or a clash.
fn resolve<'a>(
    was: Option<&'a String>,
    mine: Option<&'a String>,
    yours: Option<&'a String>,
) -> Resolution<'a> {
    match (was, mine, yours) {
        (_, Some(mine), Some(yours)) if mine == yours => Resolution::Take(mine),
        (Some(was), Some(mine), Some(yours)) => {
            if was == mine {
                Resolution::Take(yours)
            } else if was == yours {
                Resolution::Take(mine)
            } else {
                Resolution::Clash
            }
        }
        // Added on one side only: nothing to disagree with.
        (None, Some(mine), None) => Resolution::Take(mine),
        (None, None, Some(yours)) => Resolution::Take(yours),
        // Added on both sides with different text, and no ancestor to prefer either by.
        (None, Some(_), Some(_)) => Resolution::Clash,
        // Deleted on one side. The deletion carries only if the other side left it alone —
        // a delete against an edit is the one case a person has to settle.
        (Some(was), Some(mine), None) => {
            if was == mine {
                Resolution::Drop
            } else {
                Resolution::Clash
            }
        }
        (Some(was), None, Some(yours)) => {
            if was == yours {
                Resolution::Drop
            } else {
                Resolution::Clash
            }
        }
        (Some(_), None, None) | (None, None, None) => Resolution::Drop,
    }
}

/// The merged block order: ours as the spine, with theirs-only ids at their own positions.
///
/// Taking one side as the spine is what makes the result stable — a merge that reordered a
/// document nobody reordered would show up as a diff on every block. Ids only they have are
/// slotted in relative to the neighbours the spine already carries, which puts a block
/// appended on their branch at the end and one they inserted mid-document in the middle,
/// without any need to diff positions.
///
/// The cursor skips *our* own additions before inserting theirs, so two branches that each
/// appended a block land ours-then-theirs — the order git itself produces, and the one a
/// reader expects from the side they are merging into.
fn weave(ours: &[String], theirs: &[String]) -> Vec<String> {
    let mut order: Vec<String> = ours.to_vec();
    let shared: BTreeSet<&str> = theirs.iter().map(String::as_str).collect();
    // Where the next block of theirs goes: just past the last id both sides carry, and past
    // any run of ours-only blocks that follows it.
    let mut cursor = 0usize;
    for id in theirs {
        if let Some(at) = order.iter().position(|found| found == id) {
            cursor = at + 1;
            while cursor < order.len() && !shared.contains(order[cursor].as_str()) {
                cursor += 1;
            }
            continue;
        }
        order.insert(cursor, id.clone());
        cursor += 1;
    }
    order
}

/// A merge that took one side whole.
fn clean(text: &str) -> Merged {
    Merged {
        text: text.to_string(),
        conflicts: Vec::new(),
    }
}

/// Join merged pieces back into a document, matching [`stringify`] byte for byte.
fn assemble(pieces: &[String]) -> String {
    if pieces.is_empty() {
        return String::new();
    }
    let mut text = pieces.join(BLOCK_SEPARATOR);
    text.push('\n');
    text
}

/// One block, written as the disagreement it is.
///
/// An empty side is written as an empty region rather than skipped, because "they deleted
/// this and I changed it" is exactly the case where seeing the empty side is the point.
fn clash(mine: &str, yours: &str, marker_size: usize) -> String {
    let size = marker_size.max(DEFAULT_MARKER_SIZE);
    let open = "<".repeat(size);
    let middle = "=".repeat(size);
    let close = ">".repeat(size);
    let mut out = String::new();
    out.push_str(&open);
    out.push(' ');
    out.push_str(OURS_LABEL);
    out.push('\n');
    if !mine.is_empty() {
        out.push_str(mine);
        out.push('\n');
    }
    out.push_str(&middle);
    out.push('\n');
    if !yours.is_empty() {
        out.push_str(yours);
        out.push('\n');
    }
    out.push_str(&close);
    out.push(' ');
    out.push_str(THEIRS_LABEL);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document of `n` paragraphs, the one at `changed` carrying `text` instead.
    fn doc(ids: &[(&str, &str)]) -> String {
        let mut out = String::new();
        for (index, (id, text)) in ids.iter().enumerate() {
            if index > 0 {
                out.push_str(BLOCK_SEPARATOR);
            }
            out.push_str(&format!("::paragraph id={id}\n{text}\n::end"));
        }
        out.push('\n');
        out
    }

    #[test]
    fn a_side_that_did_not_move_yields_the_other_side_byte_for_byte() {
        // The property the pack merge leans on: an untouched document must come back as the
        // exact bytes that were stored, so its digest — and its stub — do not move either.
        let base = doc(&[("a", "one"), ("b", "two")]);
        let theirs = doc(&[("a", "one"), ("b", "changed")]);
        let merged = merge_documents(Some(&base), &base, &theirs, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean());
        assert_eq!(merged.text, theirs);

        let merged = merge_documents(Some(&base), &theirs, &base, DEFAULT_MARKER_SIZE);
        assert_eq!(merged.text, theirs);
    }

    #[test]
    fn two_sides_editing_two_different_blocks_both_land() {
        // The case that made every parallel branch unmergeable: master ticks one block, the
        // branch rewrites another. There is no conflict here and there never was.
        let base = doc(&[("a", "one"), ("b", "two"), ("c", "three")]);
        let ours = doc(&[("a", "ONE"), ("b", "two"), ("c", "three")]);
        let theirs = doc(&[("a", "one"), ("b", "two"), ("c", "THREE")]);

        let merged = merge_documents(Some(&base), &ours, &theirs, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean(), "{:?}", merged.conflicts);
        assert_eq!(
            merged.text,
            doc(&[("a", "ONE"), ("b", "two"), ("c", "THREE")])
        );
    }

    #[test]
    fn a_block_added_on_each_side_keeps_both() {
        let base = doc(&[("a", "one")]);
        let ours = doc(&[("a", "one"), ("mine", "m")]);
        let theirs = doc(&[("a", "one"), ("yours", "y")]);

        let merged = merge_documents(Some(&base), &ours, &theirs, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean());
        assert_eq!(
            merged.text,
            doc(&[("a", "one"), ("mine", "m"), ("yours", "y")])
        );
    }

    #[test]
    fn a_block_deleted_on_one_side_and_untouched_on_the_other_goes() {
        let base = doc(&[("a", "one"), ("b", "two")]);
        let ours = base.clone();
        let theirs = doc(&[("a", "one")]);

        let merged = merge_documents(Some(&base), &ours, &theirs, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean());
        assert_eq!(merged.text, theirs);
    }

    #[test]
    fn a_block_deleted_on_one_side_and_edited_on_the_other_is_a_conflict() {
        let base = doc(&[("a", "one"), ("b", "two")]);
        let ours = doc(&[("a", "one"), ("b", "EDITED")]);
        let theirs = doc(&[("a", "one")]);

        let merged = merge_documents(Some(&base), &ours, &theirs, DEFAULT_MARKER_SIZE);
        assert_eq!(merged.conflicts, vec!["b".to_string()]);
        assert!(merged.text.contains("EDITED"));
        assert!(has_conflict_markers(&merged.text));
    }

    #[test]
    fn both_sides_rewriting_one_block_conflicts_and_shows_both() {
        let base = doc(&[("a", "one")]);
        let ours = doc(&[("a", "mine")]);
        let theirs = doc(&[("a", "yours")]);

        let merged = merge_documents(Some(&base), &ours, &theirs, DEFAULT_MARKER_SIZE);
        assert_eq!(merged.conflicts, vec!["a".to_string()]);
        assert!(merged.text.contains("<<<<<<< ours"));
        assert!(merged.text.contains("mine"));
        assert!(merged.text.contains("======="));
        assert!(merged.text.contains("yours"));
        assert!(merged.text.contains(">>>>>>> theirs"));
        assert!(!merged.is_clean());
    }

    #[test]
    fn with_no_ancestor_identical_documents_still_merge_clean() {
        // "Added by both" is only a conflict when the two additions differ.
        let both = doc(&[("a", "one")]);
        let merged = merge_documents(None, &both, &both, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean());
        assert_eq!(merged.text, both);
    }

    #[test]
    fn with_no_ancestor_disjoint_blocks_are_taken_and_shared_ids_clash() {
        let ours = doc(&[("a", "mine"), ("m", "only mine")]);
        let theirs = doc(&[("a", "yours"), ("y", "only theirs")]);
        let merged = merge_documents(None, &ours, &theirs, DEFAULT_MARKER_SIZE);
        assert_eq!(merged.conflicts, vec!["a".to_string()]);
        assert!(merged.text.contains("only mine"));
        assert!(merged.text.contains("only theirs"));
    }

    #[test]
    fn a_clean_merge_of_changed_blocks_is_canonical_and_reparses() {
        let base = doc(&[("a", "one"), ("b", "two")]);
        let ours = doc(&[("a", "ONE"), ("b", "two")]);
        let theirs = doc(&[("a", "one"), ("b", "TWO")]);
        let merged = merge_documents(Some(&base), &ours, &theirs, DEFAULT_MARKER_SIZE);

        assert_eq!(
            merged.text,
            stringify(&parse(&merged.text)),
            "a clean merge must be canonical, or its digest is not the document's"
        );
    }

    #[test]
    fn a_marker_size_git_asked_for_is_honoured() {
        let base = doc(&[("a", "one")]);
        let merged = merge_documents(
            Some(&base),
            &doc(&[("a", "mine")]),
            &doc(&[("a", "yours")]),
            9,
        );
        assert!(merged.text.contains("<<<<<<<<< ours"), "{}", merged.text);
    }

    #[test]
    fn prose_that_mentions_a_marker_mid_line_is_not_a_conflict() {
        // The format has to be able to document its own merge driver.
        let text = "::paragraph id=p\nGit writes ======= between the two sides.\n::end\n";
        assert!(!has_conflict_markers(text));
    }

    #[test]
    fn a_marker_at_the_start_of_a_line_is_a_conflict() {
        assert!(has_conflict_markers(
            "::paragraph id=p\n<<<<<<< ours\n::end\n"
        ));
        assert!(has_conflict_markers(">>>>>>> theirs\n"));
        assert!(!has_conflict_markers("<<< short\n"));
    }

    #[test]
    fn weaving_keeps_our_order_and_slots_their_insertions_in_place() {
        let ours = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let theirs = vec![
            "a".to_string(),
            "new".to_string(),
            "b".to_string(),
            "c".to_string(),
        ];
        assert_eq!(weave(&ours, &theirs), vec!["a", "new", "b", "c"]);
    }

    #[test]
    fn weaving_puts_a_run_they_appended_at_the_end_in_their_order() {
        let ours = vec!["a".to_string()];
        let theirs = vec!["a".to_string(), "x".to_string(), "y".to_string()];
        assert_eq!(weave(&ours, &theirs), vec!["a", "x", "y"]);
    }

    #[test]
    fn weaving_puts_a_block_they_prepended_first() {
        let ours = vec!["a".to_string()];
        let theirs = vec!["z".to_string(), "a".to_string()];
        assert_eq!(weave(&ours, &theirs), vec!["z", "a"]);
    }

    #[test]
    fn deleting_every_block_on_both_sides_yields_no_text_rather_than_a_panic() {
        let base = doc(&[("a", "one")]);
        let empty = "";
        let merged = merge_documents(Some(&base), empty, empty, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean());
        assert_eq!(merged.text, empty);
    }

    #[test]
    fn a_hand_edited_side_merges_against_canonical_text_without_reading_as_rewritten() {
        // A person's plain-text edit is not canonical (spacing, attribute order). Comparing
        // raw text would call every block changed and conflict on all of them.
        let base = doc(&[("a", "one"), ("b", "two")]);
        let ours = "::paragraph id=a\none\n::end\n::paragraph id=b\nTWO\n::end\n";
        let theirs = doc(&[("a", "ONE"), ("b", "two")]);
        let merged = merge_documents(Some(&base), ours, &theirs, DEFAULT_MARKER_SIZE);
        assert!(merged.is_clean(), "{:?}", merged.conflicts);
        assert_eq!(merged.text, doc(&[("a", "ONE"), ("b", "TWO")]));
    }
}
