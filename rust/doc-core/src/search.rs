//! The dxlite-equivalent search index: tokenised, in-memory, deterministic.
//!
//! Mirrors the reference `*.dxlite.bin` sidecar's purpose — a token → document index for
//! fast lookup over bundle contents — but as a pure in-memory structure built from
//! [`Document`] values. Building and querying stay `wasm32`-friendly (no I/O); the host
//! decides where the index lives.
//!
//! # Searchable text (a format contract)
//! Indexed text comes from a document's `title`, `summary`, `tags`, and the text of these
//! block kinds: `heading`, `paragraph`, `quote`, `bulleted-list`, `numbered-list`,
//! `checklist`. The `style`, `stylesheet`, and `script` block kinds are presentation /
//! behaviour only and are **excluded** — CSS or script text never makes a document
//! findable. This matches the rendering contract where in-document CSS is presentation,
//! not content.
//!
//! # Tokenisation
//! Text is lowercased and split on every non-alphanumeric character; empty tokens are
//! dropped. Tokens are compared with simple equality (no stemming), keeping the index
//! deterministic and dependency-free.

use crate::model::{Block, Document};
use std::collections::HashMap;

/// A search result: the document path and its relevance score (higher is better).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHit {
    /// The document path supplied to [`build_index`].
    pub path: String,
    /// Relevance score; larger means a better match. See [`SearchIndex::search`].
    pub score: f64,
}

/// Per-document token statistics held by the index.
struct DocStats {
    /// Document path, echoed back in [`ScoredHit`].
    path: String,
    /// Token → count of that token within this document.
    counts: HashMap<String, u32>,
    /// Total token count, used to normalise term frequency.
    total: u32,
}

/// An in-memory token index over a set of documents.
///
/// Build it with [`build_index`], then call [`SearchIndex::search`]. The index keeps the
/// documents in their supplied order so ties break deterministically.
pub struct SearchIndex {
    docs: Vec<DocStats>,
}

impl SearchIndex {
    /// Rank documents against `query`, returning hits sorted by descending score.
    ///
    /// Scoring is the sum, over each distinct query token, of that token's normalised term
    /// frequency in the document (`occurrences / total_tokens`) plus a small fixed
    /// coverage bonus of `1.0` per distinct query token that the document contains. The
    /// coverage bonus makes a document that matches *more* of the query rank above one
    /// that merely repeats a single query term, while term frequency still separates docs
    /// that cover the same terms. Documents scoring `0` are omitted.
    ///
    /// Ties (equal score) are broken stably by ascending `path`, so results are fully
    /// deterministic. An **empty query** (no tokens) returns an empty vector — there is
    /// nothing to rank against.
    ///
    /// Complexity: `O(q · d)` time, where `q` is the number of distinct query tokens and
    /// `d` is the number of indexed documents (each token's per-document count is an `O(1)`
    /// hash-map lookup), plus `O(h log h)` for the final sort over the `h` scored hits.
    pub fn search(&self, query: &str) -> Vec<ScoredHit> {
        let query_tokens = distinct_tokens(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let mut hits: Vec<ScoredHit> = Vec::new();
        for doc in &self.docs {
            let mut score = 0.0f64;
            for token in &query_tokens {
                if let Some(&count) = doc.counts.get(token) {
                    let frequency = f64::from(count) / f64::from(doc.total.max(1));
                    score += frequency + 1.0; // coverage bonus per matched distinct token
                }
            }
            if score > 0.0 {
                hits.push(ScoredHit {
                    path: doc.path.clone(),
                    score,
                });
            }
        }

        // Higher score first; equal scores fall back to ascending path for stability.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
        });
        hits
    }
}

/// Build a [`SearchIndex`] over `(path, document)` pairs.
///
/// Paths are stored as-is and echoed in results; the same path may appear more than once
/// (the caller owns uniqueness). Indexing order is preserved for stable tie-breaking.
///
/// Complexity: `O(total tokens)` time and space across every document, since each token is
/// extracted and tallied into a per-document hash map exactly once.
pub fn build_index(docs: &[(String, Document)]) -> SearchIndex {
    let stats = docs
        .iter()
        .map(|(path, document)| {
            let tokens = document_tokens(document);
            let total = tokens.len() as u32;
            let mut counts: HashMap<String, u32> = HashMap::new();
            for token in tokens {
                *counts.entry(token).or_insert(0) += 1;
            }
            DocStats {
                path: path.clone(),
                counts,
                total,
            }
        })
        .collect();
    SearchIndex { docs: stats }
}

/// Whether a block's text contributes to the search index (see the module contract).
fn is_searchable(kind: &str) -> bool {
    matches!(
        kind,
        "heading" | "paragraph" | "quote" | "bulleted-list" | "numbered-list" | "checklist"
    )
}

/// Collect every searchable token from a document, in reading order.
fn document_tokens(document: &Document) -> Vec<String> {
    let mut tokens = Vec::new();
    push_tokens(&mut tokens, &document.title);
    push_tokens(&mut tokens, &document.summary);
    for tag in &document.tags {
        push_tokens(&mut tokens, tag);
    }
    for block in &document.blocks {
        if is_searchable(&block.kind) {
            push_block_tokens(&mut tokens, block);
        }
    }
    tokens
}

/// Tokenise a searchable block's text and item text into `out`.
fn push_block_tokens(out: &mut Vec<String>, block: &Block) {
    push_tokens(out, &block.text);
    for item in &block.items {
        push_tokens(out, &item.text);
    }
}

/// Lowercase, split on non-alphanumeric runs, and append non-empty tokens to `out`.
fn push_tokens(out: &mut Vec<String>, text: &str) {
    for piece in text.split(|c: char| !c.is_alphanumeric()) {
        if !piece.is_empty() {
            out.push(piece.to_lowercase());
        }
    }
}

/// The distinct tokens of `query`, preserving first-seen order.
fn distinct_tokens(query: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut raw = Vec::new();
    push_tokens(&mut raw, query);
    for token in raw {
        if !seen.contains(&token) {
            seen.push(token);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Item};

    fn heading(text: &str) -> Block {
        Block {
            kind: "heading".to_string(),
            level: 1,
            text: text.to_string(),
            ..Block::default()
        }
    }

    fn paragraph(text: &str) -> Block {
        Block {
            kind: "paragraph".to_string(),
            text: text.to_string(),
            ..Block::default()
        }
    }

    fn doc(title: &str, blocks: Vec<Block>) -> Document {
        Document {
            title: title.to_string(),
            blocks,
            ..Document::default()
        }
    }

    #[test]
    fn finds_by_title_heading_and_body() {
        let documents = vec![(
            "a.dx".to_string(),
            doc(
                "Storage Layer",
                vec![
                    heading("Bundle Container"),
                    paragraph("The archive stores packed documents."),
                ],
            ),
        )];
        let index = build_index(&documents);
        assert_eq!(index.search("storage")[0].path, "a.dx");
        assert_eq!(index.search("bundle")[0].path, "a.dx");
        assert_eq!(index.search("archive")[0].path, "a.dx");
    }

    #[test]
    fn finds_by_tags_and_list_and_checklist_items() {
        let mut document = doc("Notes", vec![]);
        document.tags = vec!["rust".to_string()];
        document.blocks.push(Block {
            kind: "bulleted-list".to_string(),
            items: vec![Item {
                text: "wasm target".to_string(),
                ..Item::default()
            }],
            ..Block::default()
        });
        document.blocks.push(Block {
            kind: "checklist".to_string(),
            items: vec![Item {
                checked: true,
                text: "ported codec".to_string(),
                ..Item::default()
            }],
            ..Block::default()
        });
        let index = build_index(&[("n.dx".to_string(), document)]);
        assert_eq!(index.search("rust")[0].path, "n.dx");
        assert_eq!(index.search("wasm")[0].path, "n.dx");
        assert_eq!(index.search("codec")[0].path, "n.dx");
    }

    #[test]
    fn style_stylesheet_and_script_text_is_not_indexed() {
        // Contract: presentation/behaviour blocks never make a doc findable.
        let document = doc(
            "Plain Title",
            vec![
                Block {
                    kind: "style".to_string(),
                    text: ".x { color: magentaonly; }".to_string(),
                    ..Block::default()
                },
                Block {
                    kind: "stylesheet".to_string(),
                    href: "sheetonly.css".to_string(),
                    ..Block::default()
                },
                Block {
                    kind: "script".to_string(),
                    text: "const scriptonly = 1;".to_string(),
                    ..Block::default()
                },
            ],
        );
        let index = build_index(&[("s.dx".to_string(), document)]);
        assert!(index.search("magentaonly").is_empty());
        assert!(index.search("sheetonly").is_empty());
        assert!(index.search("scriptonly").is_empty());
        // The (non-style) title is still findable.
        assert_eq!(index.search("plain")[0].path, "s.dx");
    }

    #[test]
    fn ranks_more_relevant_document_first() {
        let strong = doc(
            "Compression",
            vec![paragraph(
                "Compression compresses. Compression is everywhere here.",
            )],
        );
        let weak = doc("Other", vec![paragraph("A passing compression mention.")]);
        let index = build_index(&[
            ("weak.dx".to_string(), weak),
            ("strong.dx".to_string(), strong),
        ]);
        let hits = index.search("compression");
        assert_eq!(hits[0].path, "strong.dx");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn covering_more_query_terms_ranks_higher() {
        let both = doc("Rust Wasm", vec![paragraph("rust and wasm together")]);
        let one = doc(
            "Rust Only",
            vec![paragraph("rust rust rust rust rust rust")],
        );
        let index = build_index(&[("one.dx".to_string(), one), ("both.dx".to_string(), both)]);
        let hits = index.search("rust wasm");
        assert_eq!(hits[0].path, "both.dx");
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let index = build_index(&[("a.dx".to_string(), doc("Title", vec![]))]);
        assert!(index.search("").is_empty());
        assert!(index.search("   ::  ").is_empty()); // tokenises to nothing
    }

    #[test]
    fn equal_scores_break_ties_by_path() {
        let a = doc("Same", vec![paragraph("same")]);
        let b = doc("Same", vec![paragraph("same")]);
        let index = build_index(&[("z.dx".to_string(), a), ("a.dx".to_string(), b)]);
        let hits = index.search("same");
        assert_eq!(hits[0].path, "a.dx");
        assert_eq!(hits[1].path, "z.dx");
        assert_eq!(hits[0].score, hits[1].score);
    }
}
