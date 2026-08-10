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
//! `checklist`, `code`, and `output`. The `style`, `stylesheet`, and `script` block kinds
//! are presentation or behaviour only and are **excluded** — CSS or script text never makes
//! a document findable, matching the rendering contract where in-document CSS is
//! presentation, not content.
//!
//! # Tokenisation
//! Text is lowercased and split on every non-alphanumeric character; empty tokens are
//! dropped. Tokens are compared with simple equality (no stemming), keeping the index
//! deterministic and dependency-free.
//!
//! # Ranking
//! One scorer ranks documents and blocks alike: BM25 over field-weighted term frequencies,
//! with inverse document frequency computed over the corpus (a narrowed candidate set
//! states the full size it came from — [`SearchIndex::with_corpus_size`]), plus a phrase
//! bonus for word pairs typed side by side. What each piece buys:
//!
//! - **IDF** makes rarity worth more than repetition: in "what does confine do", a term
//!   held by one document outvotes a term held by all of them. Ubiquitous words damp
//!   themselves, so there is no stopword list to maintain and no language baked in.
//! - **Field weights** (title ×3, summary/tags/headings ×2, body ×1) land a query on the
//!   document *about* a thing rather than one that merely mentions it.
//! - **BM25 saturation** caps what repeating one term can earn, and **length
//!   normalisation** stops a long document from winning on bulk alone.
//! - **The phrase bonus** rewards word pairs that appear side by side exactly as the
//!   query typed them, and a phrase never bridges a field, block, or list-item boundary.
//!
//! Scores stay fully deterministic: no floating-point value depends on hash-map iteration
//! order, and ties break by ascending path (documents) or earliest block.

use crate::model::{Block, Document};
use std::collections::HashMap;

/// BM25 term-frequency saturation: how quickly repeats of one term stop earning score.
const BM25_K1: f64 = 1.2;
/// BM25 length normalisation: how strongly a unit's length discounts its term frequencies.
const BM25_B: f64 = 0.75;
/// Field weight of a document's title.
const WEIGHT_TITLE: f64 = 3.0;
/// Field weight of a document's summary.
const WEIGHT_SUMMARY: f64 = 2.0;
/// Field weight of each document tag.
const WEIGHT_TAG: f64 = 2.0;
/// Field weight of a heading block's text.
const WEIGHT_HEADING: f64 = 2.0;
/// Field weight of every other searchable block's text.
const WEIGHT_BODY: f64 = 1.0;
/// An adjacent query-token pair earns this share of its two terms' summed IDF.
const PHRASE_BONUS: f64 = 0.5;
/// Positions skipped between fields, blocks, and list items so a phrase never bridges them.
const SEGMENT_GAP: u64 = 2;

/// A search result: the document path and its relevance score (higher is better).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHit {
    /// The document path supplied to [`build_index`].
    pub path: String,
    /// Relevance score; larger means a better match. See [`SearchIndex::search`].
    pub score: f64,
}

/// Field-weighted token statistics for one ranked unit — a whole document, or one block.
///
/// `weighted` and `length` carry the field-weighted term frequencies BM25 consumes;
/// `positions` carries each token's unweighted stream positions for the phrase bonus.
struct UnitStats {
    /// Token → field-weighted occurrence count.
    weighted: HashMap<String, f64>,
    /// Token → ascending positions in the unit's token stream.
    positions: HashMap<String, Vec<u64>>,
    /// Field-weighted total token count, used for length normalisation.
    length: f64,
}

/// Accumulates a unit's statistics one field segment at a time.
///
/// Each `segment` call closes with a positional gap, so tokens from different fields,
/// blocks, or list items are never adjacent — a phrase cannot bridge a boundary the
/// reader would experience as one.
struct Recorder {
    stats: UnitStats,
    cursor: u64,
}

impl Recorder {
    fn new() -> Self {
        Recorder {
            stats: UnitStats {
                weighted: HashMap::new(),
                positions: HashMap::new(),
                length: 0.0,
            },
            cursor: 0,
        }
    }

    /// Tokenise `text` into the unit at `weight`, then close the segment.
    fn segment(&mut self, weight: f64, text: &str) {
        let mut tokens = Vec::new();
        push_tokens(&mut tokens, text);
        if tokens.is_empty() {
            return;
        }
        for token in tokens {
            *self.stats.weighted.entry(token.clone()).or_insert(0.0) += weight;
            self.stats
                .positions
                .entry(token)
                .or_default()
                .push(self.cursor);
            self.stats.length += weight;
            self.cursor += 1;
        }
        self.cursor += SEGMENT_GAP;
    }

    /// Record one searchable block: its text and each list item as separate segments.
    fn block(&mut self, block: &Block) {
        let weight = if block.kind == "heading" {
            WEIGHT_HEADING
        } else {
            WEIGHT_BODY
        };
        self.segment(weight, &block.text);
        for item in &block.items {
            self.segment(weight, &item.text);
        }
    }
}

/// The statistics of a whole document: title, summary, tags, then every searchable block.
fn document_stats(document: &Document) -> UnitStats {
    let mut recorder = Recorder::new();
    recorder.segment(WEIGHT_TITLE, &document.title);
    recorder.segment(WEIGHT_SUMMARY, &document.summary);
    for tag in &document.tags {
        recorder.segment(WEIGHT_TAG, tag);
    }
    for block in &document.blocks {
        if is_searchable(&block.kind) {
            recorder.block(block);
        }
    }
    recorder.stats
}

/// The statistics of one block ranked on its own, for [`best_block_id`].
fn block_stats(block: &Block) -> UnitStats {
    let mut recorder = Recorder::new();
    recorder.block(block);
    recorder.stats
}

/// The one scoring rule, at every granularity: BM25 over field-weighted term frequencies
/// with IDF taken from the ranked set, plus the phrase bonus for `pairs` (word pairs
/// adjacent in the query as typed — see [`phrase_pairs`]).
/// [`SearchIndex::search`] applies it to documents and [`best_block_id`] to single blocks,
/// so the two rankings cannot drift apart. Returns one score per unit, in unit order; a
/// unit containing none of the query tokens scores exactly `0.0`.
///
/// `corpus_size` states how many documents the full corpus holds when `units` are the
/// survivors of a narrowing pass; `None` means `units` *are* the corpus. Term document
/// frequencies always count over `units` — exact under token narrowing, which keeps every
/// holder of every query token — so only `n` needs stating to keep IDF honest.
fn rank(
    units: &[&UnitStats],
    query_tokens: &[String],
    pairs: &[(String, String)],
    corpus_size: Option<u32>,
) -> Vec<f64> {
    let unit_count = units.len() as u32;
    // A stated corpus can only be larger than what survived narrowing.
    let n = f64::from(corpus_size.unwrap_or(unit_count).max(unit_count));
    let average_length =
        (units.iter().map(|unit| unit.length).sum::<f64>() / f64::from(unit_count.max(1))).max(1.0);

    // IDF per query token (Robertson–Spärck Jones, in the always-positive Lucene form):
    // a token held by few units is worth far more than one held by all of them.
    let idf: Vec<f64> = query_tokens
        .iter()
        .map(|token| {
            let holders = units
                .iter()
                .filter(|unit| unit.weighted.contains_key(token))
                .count() as u32;
            let df = f64::from(holders);
            ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
        })
        .collect();
    let idf_of: HashMap<&str, f64> = query_tokens
        .iter()
        .map(String::as_str)
        .zip(idf.iter().copied())
        .collect();

    units
        .iter()
        .map(|unit| {
            let mut score = 0.0f64;
            for (token, token_idf) in query_tokens.iter().zip(&idf) {
                let Some(&frequency) = unit.weighted.get(token) else {
                    continue;
                };
                let saturated = frequency * (BM25_K1 + 1.0)
                    / (frequency
                        + BM25_K1 * (1.0 - BM25_B + BM25_B * unit.length / average_length));
                score += token_idf * saturated;
            }
            for (first, second) in pairs {
                if has_adjacent_pair(unit, first, second) {
                    let bonus = idf_of.get(first.as_str()).copied().unwrap_or(0.0)
                        + idf_of.get(second.as_str()).copied().unwrap_or(0.0);
                    score += PHRASE_BONUS * bonus;
                }
            }
            score
        })
        .collect()
}

/// The word pairs of `query` that were actually typed side by side, in order,
/// deduplicated, self-pairs dropped.
///
/// Derived from the raw token stream rather than [`distinct_tokens`], because
/// deduplication invents adjacencies: in `wasm rust wasm compile` the typed pairs are
/// (wasm, rust), (rust, wasm), and (wasm, compile) — never (rust, compile), which the
/// deduplicated list would suggest.
fn phrase_pairs(query: &str) -> Vec<(String, String)> {
    let mut raw = Vec::new();
    push_tokens(&mut raw, query);
    let mut pairs: Vec<(String, String)> = Vec::new();
    for window in raw.windows(2) {
        if window[0] == window[1] {
            continue;
        }
        let pair = (window[0].clone(), window[1].clone());
        if !pairs.contains(&pair) {
            pairs.push(pair);
        }
    }
    pairs
}

/// Whether `second` ever appears at the position directly after `first` in this unit.
fn has_adjacent_pair(unit: &UnitStats, first: &str, second: &str) -> bool {
    let (Some(first_positions), Some(second_positions)) =
        (unit.positions.get(first), unit.positions.get(second))
    else {
        return false;
    };
    let mut i = 0;
    let mut j = 0;
    while i < first_positions.len() && j < second_positions.len() {
        let wanted = first_positions[i] + 1;
        if second_positions[j] == wanted {
            return true;
        }
        if second_positions[j] < wanted {
            j += 1;
        } else {
            i += 1;
        }
    }
    false
}

/// Per-document entry held by the index.
struct DocEntry {
    /// Document path, echoed back in [`ScoredHit`].
    path: String,
    /// The document's field-weighted statistics.
    stats: UnitStats,
}

/// An in-memory token index over a set of documents.
///
/// Build it with [`build_index`], then call [`SearchIndex::search`]. The index keeps the
/// documents in their supplied order so ties break deterministically.
pub struct SearchIndex {
    docs: Vec<DocEntry>,
    /// The full corpus size behind a narrowed candidate set; `None` when the indexed
    /// documents are the whole corpus. See [`SearchIndex::with_corpus_size`].
    corpus_size: Option<u32>,
}

impl SearchIndex {
    /// State the size of the full corpus these documents were narrowed from, so IDF's
    /// `n` is the real document count rather than the survivor count.
    ///
    /// Correct only when the narrowing kept every document containing any query token —
    /// exactly what token narrowing does — because term document frequencies still count
    /// over the supplied documents. Without this, a multi-term query ranked over a
    /// narrowed set sees its common terms as rarer than they are, and the outcome shifts
    /// with how many unrelated documents happened to be dropped. A stated size smaller
    /// than the supplied set is clamped up to it.
    #[must_use]
    pub fn with_corpus_size(mut self, total_documents: usize) -> Self {
        self.corpus_size = Some(u32::try_from(total_documents).unwrap_or(u32::MAX));
        self
    }

    /// Rank documents against `query`, returning hits sorted by descending score.
    ///
    /// Scoring is BM25 over field-weighted term frequencies with IDF computed across the
    /// indexed set, plus a phrase bonus for query tokens appearing adjacent in query order
    /// (see the module's Ranking contract). Documents containing none of the query tokens
    /// are omitted.
    ///
    /// Ties (equal score) are broken stably by ascending `path`, so results are fully
    /// deterministic. An **empty query** (no tokens) returns an empty vector — there is
    /// nothing to rank against.
    ///
    /// Complexity: `O(q · d)` time for the term scores, where `q` is the number of distinct
    /// query tokens and `d` is the number of indexed documents, plus the phrase pass — a
    /// linear merge over each matched pair's position lists — and `O(h log h)` for the
    /// final sort over the `h` scored hits.
    pub fn search(&self, query: &str) -> Vec<ScoredHit> {
        let query_tokens = distinct_tokens(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let units: Vec<&UnitStats> = self.docs.iter().map(|doc| &doc.stats).collect();
        let scores = rank(
            &units,
            &query_tokens,
            &phrase_pairs(query),
            self.corpus_size,
        );
        let mut hits: Vec<ScoredHit> = self
            .docs
            .iter()
            .zip(scores)
            .filter(|(_, score)| *score > 0.0)
            .map(|(doc, score)| ScoredHit {
                path: doc.path.clone(),
                score,
            })
            .collect();

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

/// The id of the block within `document` that best matches `query`, under the same scoring
/// [`SearchIndex::search`] ranks documents with — BM25 whose IDF is taken across the
/// document's own searchable blocks, so a block holding the query's rare word beats one
/// repeating its common ones. Only searchable blocks compete (see the module contract),
/// the earliest block wins a tie so the answer is deterministic, and `None` means nothing
/// matched — or the query tokenised to nothing.
///
/// A heading is a label, not an answer: when a heading scores best, the winner is the
/// best-matching non-heading block inside that heading's section (up to the next heading
/// of the same or higher level), and the heading itself only when its section holds no
/// match. This is what lets a search hit carry its answer: the caller hands back this one
/// block's text with the hit, instead of leaving the reader a second read to find it.
#[must_use]
pub fn best_block_id(document: &Document, query: &str) -> Option<String> {
    let query_tokens = distinct_tokens(query);
    if query_tokens.is_empty() {
        return None;
    }

    let candidates: Vec<(usize, &Block)> = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| is_searchable(&block.kind) && !block.id.is_empty())
        .collect();
    let stats: Vec<UnitStats> = candidates
        .iter()
        .map(|(_, block)| block_stats(block))
        .collect();
    let units: Vec<&UnitStats> = stats.iter().collect();
    let scores = rank(&units, &query_tokens, &phrase_pairs(query), None);

    let mut best: Option<(f64, usize)> = None;
    for (position, score) in scores.iter().enumerate() {
        if *score > 0.0 && best.is_none_or(|(top, _)| *score > top) {
            best = Some((*score, position));
        }
    }
    let (_, winner) = best?;
    let (heading_index, heading) = candidates[winner];

    if heading.kind == "heading" {
        let section_end = document.blocks[heading_index + 1..]
            .iter()
            .position(|block| block.kind == "heading" && block.level <= heading.level)
            .map_or(document.blocks.len(), |offset| heading_index + 1 + offset);
        let mut answer: Option<(f64, usize)> = None;
        for (position, score) in scores.iter().enumerate() {
            let (index, block) = candidates[position];
            let inside = index > heading_index && index < section_end;
            if !inside || block.kind == "heading" {
                continue;
            }
            if *score > 0.0 && answer.is_none_or(|(top, _)| *score > top) {
                answer = Some((*score, position));
            }
        }
        if let Some((_, position)) = answer {
            return Some(candidates[position].1.id.clone());
        }
    }
    Some(heading.id.clone())
}

/// Build a [`SearchIndex`] over `(path, document)` pairs.
///
/// Paths are stored as-is and echoed in results; the same path may appear more than once
/// (the caller owns uniqueness). Indexing order is preserved for stable tie-breaking.
///
/// IDF needs two numbers: how many documents hold a term, counted over the documents
/// supplied here, and how many documents exist. A caller narrowing candidates by token
/// first (as the store does) still supplies every holder of every query token, so the
/// first number stays exact — state the second with [`SearchIndex::with_corpus_size`] so
/// a multi-term query's common terms are not mistaken for rare ones.
///
/// Complexity: `O(total tokens)` time and space across every document, since each token is
/// tallied and its position recorded exactly once.
pub fn build_index(docs: &[(String, Document)]) -> SearchIndex {
    let entries = docs
        .iter()
        .map(|(path, document)| DocEntry {
            path: path.clone(),
            stats: document_stats(document),
        })
        .collect();
    SearchIndex {
        docs: entries,
        corpus_size: None,
    }
}

/// Whether a block's text contributes to the search index (see the module contract).
///
/// Prose, lists, code, and captured output are all content someone might come looking for —
/// "where did we call `retry_budget`?" and "which document printed that error?" are the same
/// kind of question. `style`, `stylesheet`, and `script` are presentation and stay out, as do
/// the raw-markup kinds, whose tag names would swamp the index with noise.
fn is_searchable(kind: &str) -> bool {
    matches!(
        kind,
        "heading"
            | "paragraph"
            | "quote"
            | "bulleted-list"
            | "numbered-list"
            | "checklist"
            | "code"
            | "output"
    )
}

/// Collect every searchable token from a document, in reading order.
///
/// Public so a persistent store can record the same tokens the in-memory index would
/// derive. Ranking stays in [`SearchIndex::search`] alone: a store should use these tokens
/// to *narrow* candidates, then rank the survivors through [`build_index`], so there is only
/// ever one scoring implementation to keep honest.
#[must_use]
pub fn document_tokens(document: &Document) -> Vec<String> {
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
///
/// Public for the same reason as [`document_tokens`]: a store needs the query's tokens to
/// select candidate documents before ranking them.
#[must_use]
pub fn distinct_tokens(query: &str) -> Vec<String> {
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

/// Lines a source chunk grows to before the next blank line ends it.
///
/// Small enough that a hit is an answer rather than a file dump, large enough that a function
/// and its doc comment usually land in one chunk.
const SOURCE_CHUNK_LINES: usize = 40;

/// Read a source file as a searchable document, so the cheap route covers a project's code and
/// not only its documents.
///
/// A question whose answer lives in a `.rs` file used to miss entirely, and a miss is what sends
/// a session back to grep-and-read — the expensive route this index exists to replace. The file
/// becomes a document whose title is its path (so path words rank, which is how "format
/// contract" finds `format/mod.rs`) and whose blocks are `code` chunks: runs of lines split at
/// blank lines and grown to about [`SOURCE_CHUNK_LINES`], each identified by its line range
/// (`L120-L158`) so the hit says where to read.
///
/// Pure text in, document out — no I/O, so it compiles to `wasm32` and is testable without a
/// filesystem. The caller decides which files are worth offering.
///
/// Complexity: `O(n)` in the file's byte size.
#[must_use]
pub fn source_document(path: &str, text: &str) -> Document {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut blocks = Vec::new();
    let mut start = 0usize;

    while start < lines.len() {
        // A chunk's line range has to be the range a reader would open, so blank lines at
        // either edge belong to neither chunk.
        if lines[start].trim().is_empty() {
            start += 1;
            continue;
        }
        let mut end = (start + SOURCE_CHUNK_LINES).min(lines.len());
        // Prefer to break where the file already breaks: back up to the last blank line inside
        // the window, so a chunk holds whole paragraphs of code rather than half a function.
        if end < lines.len() {
            if let Some(blank) = (start + 1..end).rev().find(|i| lines[*i].trim().is_empty()) {
                end = blank;
            }
        }
        let mut last = end;
        while last > start && lines[last - 1].trim().is_empty() {
            last -= 1;
        }
        blocks.push(Block {
            id: format!("L{}-L{}", start + 1, last),
            kind: "code".to_string(),
            text: lines[start..last].join("\n"),
            ..Block::default()
        });
        start = end.max(start + 1);
    }

    Document {
        title: path.to_string(),
        blocks,
        ..Document::default()
    }
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

    /// The whole point of reading a source file as a document: the hit says where to read, and
    /// says it in the file's own line numbers.
    #[test]
    fn a_source_file_becomes_chunks_identified_by_their_line_range() {
        let text = (1..=95)
            .map(|n| {
                if n % 10 == 0 {
                    String::new()
                } else {
                    format!("line {n}")
                }
            })
            .collect::<Vec<String>>()
            .join("\n");
        let document = source_document("rust/doc-core/src/lib.rs", &text);

        assert_eq!(document.title, "rust/doc-core/src/lib.rs");
        assert!(
            document.blocks.len() > 1,
            "a 95-line file is more than one chunk"
        );
        assert_eq!(document.blocks[0].id, "L1-L39");
        assert!(document.blocks.iter().all(|block| block.kind == "code"));

        // Every line of the file survives, in order, across the chunks — a chunker that drops
        // lines makes the index quietly answer the wrong thing.
        let rejoined: Vec<&str> = document
            .blocks
            .iter()
            .flat_map(|block| block.text.split('\n'))
            .filter(|line| !line.trim().is_empty())
            .collect();
        let expected: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(rejoined, expected);
    }

    /// A chunk breaks where the file already breaks, so a hit is a whole thought.
    #[test]
    fn a_chunk_ends_at_a_blank_line_rather_than_mid_function() {
        let mut text = String::new();
        for group in 0..4 {
            for line in 0..12 {
                text.push_str(&format!("fn f{group}_{line}() {{}}\n"));
            }
            text.push('\n');
        }
        let document = source_document("src/many.rs", &text);
        for block in &document.blocks {
            assert!(
                !block.text.ends_with("{}") || block.text.lines().count() <= SOURCE_CHUNK_LINES,
                "chunk {} is not bounded",
                block.id
            );
            assert!(block.text.lines().count() <= SOURCE_CHUNK_LINES);
        }
    }

    /// An empty or blank file is a real case: it contributes nothing rather than a phantom hit.
    #[test]
    fn a_blank_source_file_contributes_no_blocks() {
        assert!(source_document("empty.rs", "").blocks.is_empty());
        assert!(source_document("blank.rs", "\n\n   \n").blocks.is_empty());
    }

    /// Source files and documents rank in one index, so a question lands on whichever holds the
    /// answer — and the document still wins when both mention the words.
    #[test]
    fn source_files_and_documents_rank_against_each_other_in_one_index() {
        let guide = doc(
            "guide.dx",
            vec![
                heading("Confinement"),
                paragraph("Every block runs in a sandbox."),
            ],
        );
        let code = source_document(
            "src/confine.rs",
            "pub fn confine(command: &mut Command) {\n    // seatbelt profile\n}\n",
        );
        let index = build_index(&[
            ("guide.dx".to_string(), guide),
            ("src/confine.rs".to_string(), code),
        ]);

        let hits = index.search("confinement sandbox");
        assert_eq!(hits[0].path, "guide.dx");

        // And a question only the code answers finds the code, which used to be a miss.
        let hits = index.search("seatbelt profile");
        assert_eq!(hits[0].path, "src/confine.rs");

        // And the hit names the chunk to read, in the file's own line numbers.
        let chunked = source_document(
            "src/confine.rs",
            "pub fn confine(command: &mut Command) {\n    // seatbelt profile\n}\n",
        );
        assert_eq!(
            best_block_id(&chunked, "seatbelt profile").as_deref(),
            Some("L1-L3")
        );
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
    fn code_and_its_captured_output_are_findable() {
        let source = "::paragraph id=p\nprose\n::end\n\n\
::code id=c lang=python run\nretry_budget = compute()\n::end\n\n\
::output id=o for=c status=ok\nConnectionResetError\n::end\n";
        let docs = vec![("notes.dx".to_string(), crate::format::parse(source))];
        let index = build_index(&docs);
        assert_eq!(index.search("retry_budget").len(), 1);
        assert_eq!(index.search("ConnectionResetError").len(), 1);
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
    fn rare_terms_outvote_ubiquitous_question_words() {
        // Question words live in every document, the answer's word in one. IDF must make
        // the one-document term decide the ranking, with no stopword list involved.
        let noise = |title: &str, text: &str| doc(title, vec![paragraph(text)]);
        let documents = vec![
            (
                "runner.dx".to_string(),
                noise(
                    "Runner",
                    "what does the runner do, what does it do when asked, what to do",
                ),
            ),
            (
                "loop.dx".to_string(),
                noise("Loop", "what does the loop do, what does it repeat and do"),
            ),
            (
                "phase.dx".to_string(),
                noise("Phase", "what does the phase do, and what does it not do"),
            ),
            (
                "confine.dx".to_string(),
                noise(
                    "Confinement",
                    "what confine does: it must do its work inside the sandbox",
                ),
            ),
        ];
        let index = build_index(&documents);
        assert_eq!(index.search("what does confine do")[0].path, "confine.dx");
    }

    #[test]
    fn a_title_hit_outranks_a_passing_body_mention() {
        let about = doc(
            "Confinement",
            vec![paragraph("something else entirely here")],
        );
        let mentions = doc(
            "Other Notes",
            vec![paragraph("confinement mentioned once here")],
        );
        let index = build_index(&[
            ("mentions.dx".to_string(), mentions),
            ("about.dx".to_string(), about),
        ]);
        let hits = index.search("confinement");
        assert_eq!(hits[0].path, "about.dx");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn an_adjacent_phrase_outranks_the_same_words_scattered() {
        // Same tokens, same lengths, same document frequencies — only adjacency differs.
        let phrase = doc("A", vec![paragraph("state the board geometry once")]);
        let scattered = doc("B", vec![paragraph("geometry rules the board layout")]);
        let index = build_index(&[
            ("scattered.dx".to_string(), scattered),
            ("phrase.dx".to_string(), phrase),
        ]);
        let hits = index.search("board geometry");
        assert_eq!(hits[0].path, "phrase.dx");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn a_phrase_never_bridges_two_blocks() {
        // Identical token multisets; only one document holds the phrase inside one block.
        let joined = doc("J", vec![paragraph("the board geometry rule")]);
        let split = doc(
            "S",
            vec![paragraph("the board"), paragraph("geometry rule")],
        );
        let index = build_index(&[
            ("split.dx".to_string(), split),
            ("joined.dx".to_string(), joined),
        ]);
        let hits = index.search("board geometry");
        assert_eq!(hits[0].path, "joined.dx");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn phrase_pairs_come_from_the_query_as_typed_not_its_deduplication() {
        // Query "wasm rust wasm compile" types the pairs (wasm,rust), (rust,wasm),
        // (wasm,compile) — never (rust,compile), which deduplication would invent.
        // Same tokens, same lengths in both documents; only which pair they hold differs.
        let typed = doc("A", vec![paragraph("rust and wasm compile here")]);
        let invented = doc("B", vec![paragraph("wasm and rust compile here")]);
        let index = build_index(&[
            ("invented.dx".to_string(), invented),
            ("typed.dx".to_string(), typed),
        ]);
        let hits = index.search("wasm rust wasm compile");
        assert_eq!(hits[0].path, "typed.dx");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn a_stated_corpus_size_restores_full_corpus_idf_after_narrowing() {
        // Every document weighs the same, so scores under a stated corpus size must be
        // byte-identical to ranking the full corpus — narrowing loses no information.
        let matching = vec![
            (
                "alpha.dx".to_string(),
                doc("T", vec![paragraph("alpha only here")]),
            ),
            (
                "beta1.dx".to_string(),
                doc("T", vec![paragraph("beta words here")]),
            ),
            (
                "beta2.dx".to_string(),
                doc("T", vec![paragraph("beta words here")]),
            ),
            (
                "beta3.dx".to_string(),
                doc("T", vec![paragraph("beta words here")]),
            ),
        ];
        let mut full = matching.clone();
        for filler in 0..6 {
            full.push((
                format!("filler{filler}.dx"),
                doc("T", vec![paragraph("nothing to see")]),
            ));
        }
        let full_hits = build_index(&full).search("alpha beta");
        let narrowed_hits = build_index(&matching)
            .with_corpus_size(full.len())
            .search("alpha beta");
        assert_eq!(full_hits, narrowed_hits);
        // Without the stated size, the survivors masquerade as the corpus and IDF shifts.
        let unstated_hits = build_index(&matching).search("alpha beta");
        assert_ne!(full_hits[0].score, unstated_hits[0].score);
    }

    #[test]
    fn best_block_is_the_block_that_answers_the_query() {
        let source = "::heading level=1 id=top\nGuide\n::end\n\n\
::paragraph id=intro\nInstalling is elsewhere.\n::end\n\n\
::paragraph id=rollout\nkubernetes rollout steps, rollout twice\n::end\n";
        let document = crate::format::parse(source);
        assert_eq!(
            best_block_id(&document, "rollout").as_deref(),
            Some("rollout")
        );
    }

    #[test]
    fn best_block_covering_more_query_terms_beats_repeating_one() {
        let source = "::paragraph id=repeat\ninstalling installing installing\n::end\n\n\
::paragraph id=covers\ninstalling elsewhere\n::end\n";
        let document = crate::format::parse(source);
        assert_eq!(
            best_block_id(&document, "installing elsewhere").as_deref(),
            Some("covers")
        );
    }

    #[test]
    fn best_block_prefers_the_rare_answer_word_over_question_words() {
        // Block-level IDF: the block holding the query's one rare token beats the block
        // repeating its ubiquitous ones.
        let source = "::paragraph id=chatter\nwhat the engine does, and what the runner \
does, and what the store does\n::end\n\n\
::paragraph id=answer\nconfine seals what the block does\n::end\n";
        let document = crate::format::parse(source);
        assert_eq!(
            best_block_id(&document, "what does confine do").as_deref(),
            Some("answer")
        );
    }

    #[test]
    fn best_block_descends_from_a_winning_heading_to_the_block_that_answers() {
        // The heading names the topic, so it scores best — but a label is not an answer.
        let source = "::heading level=3 id=escape-label\nEscaping: what markup survives\n::end\n\n\
::paragraph id=escape-answer\nMarkup survives only the allow-list of elements.\n::end\n\n\
::heading level=3 id=next-label\nAnother section\n::end\n\n\
::paragraph id=stray\nmarkup mentioned far away from the label\n::end\n";
        let document = crate::format::parse(source);
        assert_eq!(
            best_block_id(&document, "what markup survives").as_deref(),
            Some("escape-answer")
        );
    }

    #[test]
    fn best_block_stays_the_heading_when_its_section_holds_no_match() {
        let source = "::heading level=2 id=boards\nBoards\n::end\n\n\
::paragraph id=body\nnothing related here\n::end\n";
        let document = crate::format::parse(source);
        assert_eq!(
            best_block_id(&document, "boards").as_deref(),
            Some("boards")
        );
    }

    #[test]
    fn best_block_is_none_when_nothing_matches_or_the_query_is_empty() {
        let document = crate::format::parse("::paragraph id=p\nplain prose\n::end\n");
        assert!(best_block_id(&document, "absent").is_none());
        assert!(best_block_id(&document, "  ::  ").is_none());
    }

    #[test]
    fn best_block_ties_go_to_the_earliest_block() {
        let source = "::paragraph id=first\nsame words\n::end\n\n\
::paragraph id=second\nsame words\n::end\n";
        let document = crate::format::parse(source);
        assert_eq!(best_block_id(&document, "same").as_deref(), Some("first"));
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
