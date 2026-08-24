//! Merging two branches of a workspace: the committed pack, and the pointers that name it.
//!
//! # The problem this exists for
//! A dx workspace commits two kinds of file, and git could merge neither. `.doc/repo.dxcp`
//! is a container git reads as binary and refuses outright ("warning: Cannot merge binary
//! files"), and a `.dx` pointer is one digest line, so two branches that changed the same
//! document produce a line conflict on a hex string no person can resolve. The consequence
//! was that *any* two branches that both touched documents — the ordinary case for two
//! agents in two worktrees — could not be merged back at all, and the workaround was to
//! abort and re-apply one side's edits by hand.
//!
//! # Why this is well-defined
//! Neither file is really an atom. The pack is a *set* of documents keyed by path, and a
//! document is a *sequence of blocks* keyed by id ([`doc_core::merge`]). So the merge
//! decomposes twice, and at the bottom sits a three-way comparison of one block's canonical
//! text against the same block on the other branch. A branch that added a document and a
//! master that ticked a checkbox in a different one do not overlap anywhere in that
//! decomposition, and the merge takes both without asking anybody.
//!
//! # The two drivers agree because they compute the same thing
//! Git invokes a merge driver once per path and in no order a caller may rely on, so the
//! driver that merges the pack and the driver that merges a pointer cannot hand results to
//! each other. They do not have to: both call [`doc_core::merge::merge_documents`] on the
//! same three revisions, and it is a pure function. The pointer driver writes the digest of
//! what the pack driver stored, without either one knowing whether the other has run.
//!
//! # Nothing here writes to the store
//! A merge driver runs in the middle of a git operation, with the working tree half
//! rewritten and `.doc/repo.dxcp` about to be replaced by git itself. Ingesting into the
//! index from there would race git's own write and would make a `git merge` mutate a
//! database that is meant to be rebuildable. So the drivers are pure: bytes in, bytes out.
//! `dx sync` afterwards is what puts the index back in agreement — and it is the same
//! `dx sync` a fresh clone runs, not a special path.

use std::collections::{BTreeMap, BTreeSet};

use doc_core::chunk::{encode_pack_for, Pack, PackStorage};
use doc_core::format::parse;
use doc_core::merge::{merge_documents, Merged};

use crate::pack;
use crate::StoreError;

/// The result of merging two packs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPack {
    /// The merged pack, encoded and ready to write where git asked for it.
    pub bytes: Vec<u8>,
    /// Workspace-relative path of every document the two sides changed irreconcilably.
    pub conflicts: Vec<String>,
    /// The merged canonical source of every document in the result, keyed by path. The
    /// pointer driver needs exactly this to compute a stub, and a caller that already has
    /// the merged pack should not have to decode it again to get it.
    pub documents: BTreeMap<String, String>,
}

impl MergedPack {
    /// Whether the merge needs no human.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Merge two revisions of a pack against their common ancestor.
///
/// `ancestor` is `None` when both branches created the pack independently — a workspace
/// that became a dx workspace twice — and every document only one side has is simply taken.
///
/// A document that conflicts is kept in the merged pack **carrying its conflict markers**,
/// rather than resolved to one side. The markers are ordinary body text as far as the format
/// is concerned, so the pack stays readable and, crucially, still holds both branches' words:
/// a person who resolves the pointer by hand can see what the other side wrote, and nothing
/// was quietly discarded to make a binary file decode.
///
/// # Errors
/// [`StoreError::Corrupt`] when any of the three packs will not decode. A pack that will not
/// decode is never treated as empty — that would look exactly like the branch having deleted
/// every document it holds.
pub fn merge_packs(
    ancestor: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    storage: PackStorage,
    marker_size: usize,
) -> Result<MergedPack, StoreError> {
    let base = match ancestor {
        Some(bytes) => Some(documents_of(bytes, "the merge base's .doc/repo.dxcp")?),
        None => None,
    };
    let ours = documents_of(ours, "this branch's .doc/repo.dxcp")?;
    let theirs = documents_of(theirs, "the incoming branch's .doc/repo.dxcp")?;

    let mut documents: BTreeMap<String, String> = BTreeMap::new();
    let mut conflicts = Vec::new();
    let paths: BTreeSet<&String> = ours.keys().chain(theirs.keys()).collect();
    for path in paths {
        let was = base.as_ref().and_then(|base| base.get(path));
        match (ours.get(path), theirs.get(path)) {
            (Some(mine), Some(yours)) => {
                let merged = merge_documents(was.map(String::as_str), mine, yours, marker_size);
                record(&mut documents, &mut conflicts, path, merged);
            }
            // Present on one side only. With no ancestor it is an addition; with one it is
            // the other side's deletion, which carries unless this side also edited it.
            (Some(mine), None) => match was {
                Some(was) if was == mine => {}
                Some(was) => {
                    let merged = merge_documents(Some(was), mine, "", marker_size);
                    record(&mut documents, &mut conflicts, path, merged);
                }
                None => {
                    documents.insert(path.clone(), mine.clone());
                }
            },
            (None, Some(yours)) => match was {
                Some(was) if was == yours => {}
                Some(was) => {
                    let merged = merge_documents(Some(was), "", yours, marker_size);
                    record(&mut documents, &mut conflicts, path, merged);
                }
                None => {
                    documents.insert(path.clone(), yours.clone());
                }
            },
            (None, None) => {}
        }
    }

    let parsed: Vec<(String, doc_core::model::Document)> = documents
        .iter()
        .map(|(path, source)| (path.clone(), parse(source)))
        .collect();
    let pack = Pack::build(
        parsed
            .iter()
            .map(|(path, document)| (path.as_str(), document)),
    );
    Ok(MergedPack {
        bytes: encode_pack_for(&pack, storage),
        conflicts,
        documents,
    })
}

/// File the merge of one document, dropping it when the merge left nothing.
///
/// A document that merged to no blocks was deleted on one side and emptied on the other; it
/// leaves the pack rather than being stored as an empty document that would resolve to a
/// default paragraph nobody wrote.
fn record(
    documents: &mut BTreeMap<String, String>,
    conflicts: &mut Vec<String>,
    path: &str,
    merged: Merged,
) {
    if !merged.is_clean() {
        conflicts.push(path.to_owned());
    }
    if merged.text.is_empty() {
        return;
    }
    documents.insert(path.to_owned(), merged.text);
}

/// Decode one side of the merge; an empty blob is an absent pack, not a corrupt one.
///
/// Git hands the driver an empty file for a side that does not have the path at all, which
/// is how "this branch added `.doc/` and that one did not" arrives.
fn documents_of(bytes: &[u8], label: &str) -> Result<BTreeMap<String, String>, StoreError> {
    if bytes.is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(pack::decode(bytes, label)?.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::merge::DEFAULT_MARKER_SIZE;

    fn doc(text: &str) -> String {
        format!("::paragraph id=p\n{text}\n::end\n")
    }

    fn packed(documents: &[(&str, &str)]) -> Vec<u8> {
        let parsed: Vec<(String, doc_core::model::Document)> = documents
            .iter()
            .map(|(path, source)| ((*path).to_string(), parse(source)))
            .collect();
        let pack = Pack::build(
            parsed
                .iter()
                .map(|(path, document)| (path.as_str(), document)),
        );
        encode_pack_for(&pack, PackStorage::ForVersionControl)
    }

    fn merge(base: Option<&[u8]>, ours: &[u8], theirs: &[u8]) -> MergedPack {
        merge_packs(
            base,
            ours,
            theirs,
            PackStorage::ForVersionControl,
            DEFAULT_MARKER_SIZE,
        )
        .expect("merge")
    }

    #[test]
    fn one_branch_adding_a_document_and_the_other_editing_a_different_one_merges_clean() {
        // The exact scenario report-6922595c filed: a worktree wrote two new documents while
        // master ticked a box, and git produced an unresolvable binary conflict.
        let base = packed(&[("index.dx", &doc("one"))]);
        let ours = packed(&[("index.dx", &doc("one")), ("console.dx", &doc("new"))]);
        let theirs = packed(&[("index.dx", &doc("ticked"))]);

        let merged = merge(Some(&base), &ours, &theirs);
        assert!(merged.is_clean(), "{:?}", merged.conflicts);
        assert_eq!(merged.documents.len(), 2);
        assert_eq!(merged.documents["index.dx"], doc("ticked"));
        assert_eq!(merged.documents["console.dx"], doc("new"));
    }

    #[test]
    fn the_merged_bytes_decode_back_to_the_merged_documents() {
        let base = packed(&[("a.dx", &doc("one"))]);
        let ours = packed(&[("a.dx", &doc("one")), ("b.dx", &doc("b"))]);
        let theirs = packed(&[("a.dx", &doc("one")), ("c.dx", &doc("c"))]);

        let merged = merge(Some(&base), &ours, &theirs);
        let decoded: BTreeMap<String, String> = pack::decode(&merged.bytes, "merged")
            .expect("decode")
            .into_iter()
            .collect();
        assert_eq!(decoded, merged.documents);
        assert_eq!(decoded.len(), 3);
    }

    #[test]
    fn a_document_only_one_side_deleted_leaves_the_pack() {
        let base = packed(&[("a.dx", &doc("one")), ("gone.dx", &doc("bye"))]);
        let ours = base.clone();
        let theirs = packed(&[("a.dx", &doc("one"))]);

        let merged = merge(Some(&base), &ours, &theirs);
        assert!(merged.is_clean());
        assert!(!merged.documents.contains_key("gone.dx"));
    }

    #[test]
    fn a_document_deleted_on_one_side_and_edited_on_the_other_conflicts_and_keeps_the_words() {
        let base = packed(&[("a.dx", &doc("one"))]);
        let ours = packed(&[("a.dx", &doc("kept and edited"))]);
        let theirs = packed(&[]);

        let merged = merge(Some(&base), &ours, &theirs);
        assert_eq!(merged.conflicts, vec!["a.dx".to_string()]);
        assert!(merged.documents["a.dx"].contains("kept and edited"));
    }

    #[test]
    fn both_sides_rewriting_one_block_conflicts_and_the_pack_still_decodes() {
        // The pack must stay readable through a conflict: a container that would not decode
        // is a merge that lost the branch it could not represent.
        let base = packed(&[("a.dx", &doc("one"))]);
        let ours = packed(&[("a.dx", &doc("mine"))]);
        let theirs = packed(&[("a.dx", &doc("yours"))]);

        let merged = merge(Some(&base), &ours, &theirs);
        assert_eq!(merged.conflicts, vec!["a.dx".to_string()]);
        let decoded = pack::decode(&merged.bytes, "merged").expect("still decodes");
        let source = &decoded
            .iter()
            .find(|(path, _)| path == "a.dx")
            .expect("a.dx")
            .1;
        assert!(source.contains("mine"), "{source}");
        assert!(source.contains("yours"), "{source}");
    }

    #[test]
    fn with_no_ancestor_both_sides_documents_are_taken() {
        let ours = packed(&[("a.dx", &doc("mine"))]);
        let theirs = packed(&[("b.dx", &doc("yours"))]);
        let merged = merge(None, &ours, &theirs);
        assert!(merged.is_clean());
        assert_eq!(merged.documents.len(), 2);
    }

    #[test]
    fn an_absent_side_is_empty_rather_than_corrupt() {
        let ours = packed(&[("a.dx", &doc("mine"))]);
        let merged = merge(None, &ours, &[]);
        assert!(merged.is_clean());
        assert_eq!(merged.documents.len(), 1);
    }

    #[test]
    fn a_pack_that_will_not_decode_is_refused_rather_than_read_as_empty() {
        // Reading a damaged side as "no documents" would merge to a pack that deleted every
        // document that branch had.
        let ours = packed(&[("a.dx", &doc("mine"))]);
        let error = merge_packs(
            None,
            &ours,
            b"this is not a pack",
            PackStorage::ForVersionControl,
            DEFAULT_MARKER_SIZE,
        )
        .expect_err("must refuse");
        assert!(matches!(error, StoreError::Corrupt(_)), "{error}");
    }

    #[test]
    fn an_untouched_document_keeps_its_exact_bytes_so_its_pointer_does_not_move() {
        // The property the pointer driver depends on: a document neither branch touched must
        // come back byte-identical, or every stub in the tree would be rewritten by a merge
        // that changed nothing.
        let untouched = doc("steady");
        let base = packed(&[("a.dx", &untouched), ("b.dx", &doc("one"))]);
        let ours = packed(&[("a.dx", &untouched), ("b.dx", &doc("mine"))]);
        let theirs = packed(&[("a.dx", &untouched), ("b.dx", &doc("one"))]);

        let merged = merge(Some(&base), &ours, &theirs);
        assert_eq!(merged.documents["a.dx"], untouched);
    }
}
