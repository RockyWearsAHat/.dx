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
//! dropped. Each word is then indexed under the tokens a reader might ask for it by, which
//! is what keeps a question answerable in the asker's vocabulary rather than only the
//! project's:
//!
//! - **Its stem.** A conservative English suffix strip ([`stem`]) folds `weights` onto
//!   `weight` and `packed` onto `pack`, applied identically to text and query so the two
//!   meet in the middle. It refuses whenever the remainder would be too short to be a word
//!   or holds no vowel (`ring` keeps its `ing`, and so does `string`), because a wrong stem
//!   is a wrong match, and it never touches a word carrying digits (`sha256`, `utf8`).
//! - **The parts of a compound identifier.** `packWeights`, `PackWeights`, and
//!   `pack_weights` all yield `pack` and `weight` beside the whole word, so the name a
//!   codebase writes is reachable by the words it is made of. Parts earn
//!   [`PART_WEIGHT`] of the whole word's weight — a part is weaker evidence than the word.
//!
//! Every token derived from one word shares that word's position, so deriving tokens never
//! invents or breaks a phrase, and a word still counts once toward length.
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
//! - **The selectivity floor** finishes that job where damping alone could not. A term more
//!   than [`SELECTIVE_SHARE`] of the corpus holds selects nothing — it describes the corpus,
//!   not the question — so it may add to a score but may not *carry* one: a unit matching
//!   only such terms scores zero rather than ranking. This is what stops a question the
//!   corpus cannot answer from returning the shortest files that happen to say "the" and
//!   "come from", which reads as an answer, costs a read, and is not one. It is measured
//!   against the corpus rather than a word list, so no language is assumed here either, and
//!   it stands down entirely when *no* query term is selective — asked only for words
//!   everything holds, ranking them is the best answer available.
//! - **Field weights** (title ×3, summary/tags/headings ×2, body ×1, captured output ×0.5)
//!   land a query on the document *about* a thing rather than one that merely mentions it,
//!   or that merely printed it.
//! - **BM25 saturation** caps what repeating one term can earn, and **length
//!   normalisation** stops a long document from winning on bulk alone.
//! - **The phrase bonus** rewards word pairs that appear side by side exactly as the
//!   query typed them, and a phrase never bridges a field, block, or list-item boundary.
//! - **The loose tier** catches the word a corpus never says on its own: a query token no
//!   unit holds — and only then — also scores against its kin at [`LOOSE_WEIGHT`], the words
//!   that carry it inside them (`bitpack` for `packed`) and the abbreviation it starts with
//!   (`mic` for `microphone`). It costs nothing on a query the corpus can answer as asked,
//!   and it can only add: a token with no holder scored zero before.
//! - **The vocabulary bridge** ([`VOCABULARY`]) is the same idea for the words that share no
//!   letters at all. No rule over spelling reaches `gpu` from *graphics card* or `cpu` from
//!   *processor*, and a reader who does not yet know a project's words is exactly who search
//!   is for — so a token's kin join the query as terms of their own at [`SYNONYM_WEIGHT`],
//!   each keeping its own IDF. Terms, not variants, and unlike the loose tier not gated on
//!   the word being absent: in any corpus of size some file says "graphics" in passing, and
//!   gating on that would answer the question from whichever file mentioned the word. This
//!   is the one place a word list is baked in; it is a glossary of general computing
//!   English, deliberately not project vocabulary.
//!
//! The same scorer also chooses *which part* of a long block a hit shows
//! ([`answering_line`]), so an excerpt states the fact instead of starting at the block's
//! first line — a hit on the right block showing the wrong sentence reads as a miss.
//!
//! Scores stay fully deterministic: no floating-point value depends on hash-map iteration
//! order, and ties break by ascending path (documents) or earliest block.

use crate::model::{Block, Document};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// FNV-1a over a token's bytes, and the reason there is a hasher in this file at all.
///
/// Tokenising a repository is the most expensive thing search does — three quarters of a
/// query's wall clock here — and nearly all of it is hashing short ASCII words into one map
/// per unit. The standard hasher is SipHash, chosen to be hard to flood with collisions from
/// untrusted keys; these keys are the words of files the caller already chose to read, and
/// paying for that protection on every word of every file is the whole cost of the safety
/// nobody needed. FNV-1a is a few instructions per byte and keeps the map's behaviour
/// otherwise identical — including determinism, which never depended on iteration order.
#[derive(Default)]
struct TokenHasher(u64);

impl Hasher for TokenHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // The empty-state constant is FNV's offset basis; every byte folds in then mixes.
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        self.0 = hash;
    }
}

/// A map keyed by tokens, hashed with [`TokenHasher`].
type TokenMap<V> = HashMap<String, V, BuildHasherDefault<TokenHasher>>;

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
/// Field weight of a captured `::output` block.
///
/// Output stays searchable — "which document printed that error?" is a real question — but it
/// is a record of a run, not something written to answer anything, and it quotes whatever was
/// asked of it. A measurement page that prints its twelve questions must not outrank the
/// twelve documents that answer them.
const WEIGHT_OUTPUT: f64 = 0.5;
/// An adjacent query-token pair earns this share of its two terms' summed IDF.
const PHRASE_BONUS: f64 = 0.5;
/// Positions skipped between fields, blocks, and list items so a phrase never bridges them.
const SEGMENT_GAP: u64 = 2;
/// Share of a word's weight each part of a compound identifier earns.
///
/// A part is real evidence — `packWeights` is about packing — but weaker than the word as
/// written, or every long identifier would outrank the file named for the thing.
const PART_WEIGHT: f64 = 0.5;
/// Share of an exact match's weight a loose (inside-a-word) match earns.
const LOOSE_WEIGHT: f64 = 0.35;
/// Shortest query token matched inside longer words. Below this a token is a fragment that
/// appears in everything (`get`, `set`, `id`) and matching it inside words says nothing.
///
/// Public because a store that narrows a corpus before ranking it must narrow by the same
/// rule the ranker scores by, or narrowing decides the question the scorer was asked.
pub const MIN_LOOSE_LEN: usize = 4;
/// Most containing words one absent query token expands to, shortest first — the closest
/// containers — so an unlucky query cannot turn one term into a scan of the vocabulary.
///
/// Public for the same reason as [`MIN_LOOSE_LEN`].
pub const MAX_LOOSE_VARIANTS: usize = 64;
/// Shortest stored word treated as an abbreviation of a longer query word (`mic` for
/// `microphone`). Two letters is a preposition, not a shortening.
const MIN_ABBREVIATION: usize = 3;
/// Share of an exact match's weight a [`VOCABULARY`] match earns.
///
/// Above [`LOOSE_WEIGHT`], and for a reason: one word containing another is a coincidence of
/// spelling that often means nothing, while a vocabulary group is a stated equivalence — a
/// file that says `gpu` really is what *graphics card* was asking for. Still below an exact
/// match, because the word the project chose is the better evidence when it is there.
const SYNONYM_WEIGHT: f64 = 0.6;
/// Share of the corpus above which a term is too common to carry a hit on its own.
///
/// A word more than half the documents hold describes the corpus rather than the question:
/// it can refine a ranking among units that already matched something, but a unit whose
/// whole case rests on it has not answered anything. See the module's Ranking contract.
const SELECTIVE_SHARE: f64 = 0.5;

/// Words that name the same thing in computing, grouped.
///
/// The gap this closes is not spelling, it is vocabulary: a reader asks about the *graphics
/// card* and the code says `gpu`, asks about a *folder* and the code says `directory`. No
/// stemmer, substring, or abbreviation rule can cross that — the words share no letters — so
/// the bridge has to be stated. What belongs here is **general computing English**: the
/// everyday word, the field's word, and the abbreviation code actually types. What does not
/// belong is any particular project's vocabulary, which is what the project's own documents
/// are for.
///
/// Groups are consulted only for a query token no unit holds ([`query_terms`]), and only the
/// members the corpus does hold become variants — so a group naming five things costs a
/// corpus that mentions none of them exactly nothing. Order within a group carries no
/// meaning. Keep groups tight: a group is a claim that these words are interchangeable, and
/// a loose claim shows up as a wrong first hit.
const VOCABULARY: &[&[&str]] = &[
    // Hardware a reader names by sight, code names by acronym.
    &["gpu", "graphics", "videocard"],
    &["cpu", "processor"],
    &["ram", "memory"],
    &["disk", "drive", "storage"],
    &["screen", "display", "monitor"],
    &["microphone", "mic"],
    &["speaker", "playback"],
    &["mouse", "pointer", "cursor"],
    &["keyboard", "keypress"],
    // The filesystem.
    &["folder", "directory"],
    &["filename", "path", "basename"],
    &["delete", "remove", "erase", "unlink"],
    &["rename", "move"],
    &["copy", "duplicate", "clone"],
    // Media.
    &["picture", "image", "photo", "bitmap"],
    &["movie", "video"],
    &["sound", "audio"],
    &["font", "typeface", "glyph"],
    &["colour", "color"],
    // Moving bytes.
    &["download", "fetch", "retrieve"],
    &["upload", "publish", "send"],
    &["network", "internet", "online"],
    &["server", "host", "backend"],
    &["client", "frontend"],
    &["link", "url", "href", "uri"],
    &["api", "endpoint", "interface"],
    // Squeezing and scrambling bytes.
    &["compress", "zip", "deflate", "squeeze"],
    &["decompress", "unzip", "inflate", "expand"],
    &["encrypt", "cipher", "encipher"],
    &["hash", "digest", "checksum", "fingerprint"],
    // Who is asking.
    &["login", "signin", "authenticate", "auth"],
    &["password", "passphrase", "credential", "secret"],
    &[
        "permission",
        "privilege",
        "access",
        "authorise",
        "authorize",
    ],
    &["user", "account", "profile"],
    // Where data sits.
    &["database", "table", "record"],
    &["cache", "buffer"],
    &["queue", "backlog", "pending"],
    &["list", "array", "vector", "slice"],
    &["dictionary", "hashmap", "lookup"],
    &["setting", "config", "preference", "option"],
    // What the machine is doing.
    &["thread", "worker", "concurrent", "parallel"],
    &["lock", "mutex", "exclusive"],
    &["schedule", "timer", "cron", "interval"],
    &["random", "rng", "entropy"],
    &["crash", "panic", "abort", "fatal"],
    &["bug", "defect", "fault", "failure"],
    &["log", "trace", "diagnostic"],
    &["speed", "performance", "latency", "throughput"],
    // Working on the code.
    &["build", "compile", "toolchain"],
    &["test", "spec", "assertion", "check", "verify"],
    &["library", "package", "module", "crate", "dependency"],
    &["function", "method", "procedure", "routine"],
    &["variable", "field", "attribute", "property"],
    &["install", "setup", "bootstrap"],
    &["launch", "start", "boot", "spawn"],
    &["quit", "stop", "shutdown", "terminate"],
    &["update", "upgrade", "refresh"],
    &["version", "revision", "release"],
    // Text and numbers.
    &["text", "string", "prose"],
    &["number", "integer", "numeric"],
    &["date", "timestamp", "datetime"],
    &["language", "locale", "translation"],
    &["word", "token", "term"],
];
/// What a test chunk's score is multiplied by when choosing a hit's *answering* block.
///
/// A test quotes a fact to check it; the code states it. Asked "what magic bytes does a
/// frame carry", a reader wants the constant, not the loop that asserts over it — and the
/// test wins on repetition every time, because repeating the query is what a test does.
const TEST_DISCOUNT: f64 = 0.4;

/// A search result: the document path and its relevance score (higher is better).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHit {
    /// The document path supplied to [`build_index`].
    pub path: String,
    /// Relevance score; larger means a better match. See [`SearchIndex::search`].
    pub score: f64,
}

/// What one token contributes to one unit: how much weight it earned, and where it sat.
#[derive(Default)]
struct TokenStats {
    /// Field-weighted occurrence count — what BM25 saturates.
    weighted: f64,
    /// Ascending positions in the unit's token stream, for the phrase bonus.
    positions: Vec<u64>,
}

/// Field-weighted token statistics for one ranked unit — a whole document, or one block.
///
/// One map, not two: weight and positions are recorded together for every token, so keeping
/// them apart meant hashing and allocating each key twice on the hottest path there is.
struct UnitStats {
    /// Token → what it contributed to this unit.
    entries: TokenMap<TokenStats>,
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
                entries: TokenMap::default(),
                length: 0.0,
            },
            cursor: 0,
        }
    }

    /// Tokenise `text` into the unit at `weight`, then close the segment.
    ///
    /// One word occupies one position and adds `weight` to the unit's length however many
    /// tokens it is indexed under: the word's own stem at full weight, each part of a
    /// compound identifier at [`PART_WEIGHT`] of it.
    fn segment(&mut self, weight: f64, text: &str) {
        let start = self.cursor;
        for_each_word(text, |variants| {
            for (index, token) in variants.iter().enumerate() {
                let earned = if index == 0 {
                    weight
                } else {
                    weight * PART_WEIGHT
                };
                // One lookup for both halves of a token's record, and the key is copied only
                // when the token is new to this unit.
                let entry = match self.stats.entries.get_mut(token.as_str()) {
                    Some(entry) => entry,
                    None => self.stats.entries.entry(token.clone()).or_default(),
                };
                entry.weighted += earned;
                // Only the word as written takes a position. A phrase is made of the words
                // that were typed ([`phrase_pairs`]), and every part of a word shares the
                // word's own position anyway, so recording them would cost memory to prove
                // an adjacency that can never differ.
                if index == 0 {
                    entry.positions.push(self.cursor);
                }
            }
            self.stats.length += weight;
            self.cursor += 1;
        });
        if self.cursor != start {
            self.cursor += SEGMENT_GAP;
        }
    }

    /// Record one searchable block: its text and each list item as separate segments.
    fn block(&mut self, block: &Block) {
        let weight = match block.kind.as_str() {
            "heading" => WEIGHT_HEADING,
            "output" => WEIGHT_OUTPUT,
            _ => WEIGHT_BODY,
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

/// One query word and the index tokens it scores against.
///
/// Normally that is the word itself. When the ranked set holds the word in no unit at all,
/// it is also every word that *contains* it, at [`LOOSE_WEIGHT`] — the tier that lets a
/// question asked in the reader's vocabulary reach a project that only ever writes its own.
struct QueryTerm {
    /// Token and the share of its frequency this term counts, best match first.
    variants: Vec<(String, f64)>,
}

impl QueryTerm {
    /// This term's field-weighted frequency in `unit`: its variants' frequencies, each at
    /// the share the variant earns. Zero means the unit does not hold the term at all.
    fn frequency_in(&self, unit: &UnitStats) -> f64 {
        self.variants
            .iter()
            .filter_map(|(token, share)| {
                unit.entries.get(token).map(|found| found.weighted * share)
            })
            .sum()
    }
}

/// Expand each query token into the term the ranker scores, consulting the units' own
/// vocabulary — and building that vocabulary only when a token needs it, which is never for
/// a query the corpus can answer as asked.
///
/// The returned list can be **longer than `query_tokens`**: a word with kin in [`VOCABULARY`]
/// contributes those as terms of their own, discounted to [`SYNONYM_WEIGHT`]. They are extra
/// terms rather than extra variants of the asked word, and the difference is the whole
/// behaviour. A variant joins its term's document frequency, so bridging *sound* to *audio*
/// in a corpus full of audio would make the asked word look common and stop it selecting —
/// the rare word would be diluted by the bridge meant to help it. As separate terms each
/// keeps its own IDF, nothing already scoring loses a point, and a document that only ever
/// says `gpu` can still answer *graphics card*.
///
/// Complexity: `O(q · u)` for the exact pass over `u` units, plus — only for a token no unit
/// holds — one `O(total tokens)` vocabulary pass shared by every such token.
fn query_terms(units: &[&UnitStats], query_tokens: &[String]) -> Vec<QueryTerm> {
    let mut vocabulary: Option<Vec<&str>> = None;
    let mut terms: Vec<QueryTerm> = query_tokens
        .iter()
        .map(|token| {
            let mut variants = vec![(token.clone(), 1.0)];
            let held = units.iter().any(|unit| unit.entries.contains_key(token));
            if !held && token.chars().count() >= MIN_LOOSE_LEN {
                let vocabulary = vocabulary.get_or_insert_with(|| {
                    let mut words: Vec<&str> = units
                        .iter()
                        .flat_map(|unit| unit.entries.keys().map(String::as_str))
                        .collect();
                    words.sort_unstable();
                    words.dedup();
                    words
                });
                // Both directions of "the same word, written differently": the words that
                // carry this one inside them (`bitpack` for `pack`), and the abbreviation
                // this one starts with (`mic` for `microphone`), which is how code names
                // things the prose spells out.
                let mut kin: Vec<&str> = vocabulary
                    .iter()
                    .copied()
                    .filter(|word| {
                        word.contains(token.as_str())
                            || (word.chars().count() >= MIN_ABBREVIATION
                                && token.starts_with(*word))
                    })
                    .collect();
                // Closest first — the shortest container, the longest abbreviation — then
                // alphabetical, so the cut at [`MAX_LOOSE_VARIANTS`] is deterministic.
                kin.sort_by_key(|word| (word.len().abs_diff(token.len()), *word));
                variants.extend(
                    kin.into_iter()
                        .take(MAX_LOOSE_VARIANTS)
                        .map(|word| (word.to_string(), LOOSE_WEIGHT)),
                );
            }
            QueryTerm { variants }
        })
        .collect();

    // The other names for the thing asked about, kept only where the corpus actually uses
    // one — so a group naming five things costs a corpus that mentions none of them nothing,
    // and a name the query already carries is never counted twice.
    let mut bridged: Vec<String> = Vec::new();
    for token in query_tokens {
        for kin in vocabulary_kin(token) {
            if query_tokens.contains(&kin)
                || bridged.contains(&kin)
                || !units.iter().any(|unit| unit.entries.contains_key(&kin))
            {
                continue;
            }
            bridged.push(kin.clone());
            terms.push(QueryTerm {
                variants: vec![(kin, SYNONYM_WEIGHT)],
            });
        }
    }
    terms
}

/// The stems of every other word grouped with `token` in [`VOCABULARY`], deduplicated.
///
/// `token` is already a stem, and the table is written in plain words, so both sides are put
/// through [`stem`] — the table can never drift out of step with the tokeniser. Empty for the
/// overwhelming majority of words, and only ever consulted for a token the corpus does not
/// hold, so the linear scan over a table this size is not on any hot path.
///
/// Public for the same reason as [`MIN_LOOSE_LEN`]: a store that narrows a corpus before
/// ranking it must narrow by every rule the ranker scores by. A document reachable only
/// through this bridge has to survive narrowing, or the bridge is never crossed.
#[must_use]
pub fn vocabulary_kin(token: &str) -> Vec<String> {
    let mut kin: Vec<String> = Vec::new();
    for group in VOCABULARY {
        if !group.iter().any(|word| stem(word) == token) {
            continue;
        }
        for word in *group {
            let stemmed = stem(word).into_owned();
            if stemmed != token && !kin.contains(&stemmed) {
                kin.push(stemmed);
            }
        }
    }
    kin
}

/// Whether a ranking applies the selectivity floor — see [`rank`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Floor {
    /// Ranking a corpus: a unit matching only ubiquitous terms has not answered anything.
    Apply,
    /// Ranking the parts of something already found: every part is inside an answer.
    Off,
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
///
/// `floor` says whether the selectivity floor applies. It belongs to the corpus-level
/// question — *does anything here answer this?* — and nowhere else: choosing which block or
/// which line of an already-relevant document to show is a choice among parts of an answer,
/// where a floor can only throw information away. The units there also overlap or are few
/// enough that a document frequency over them measures nothing.
fn rank(
    units: &[&UnitStats],
    query_tokens: &[String],
    pairs: &[(String, String)],
    corpus_size: Option<u32>,
    floor: Floor,
) -> Vec<f64> {
    let unit_count = units.len() as u32;
    // A stated corpus can only be larger than what survived narrowing.
    let n = f64::from(corpus_size.unwrap_or(unit_count).max(unit_count));
    let average_length =
        (units.iter().map(|unit| unit.length).sum::<f64>() / f64::from(unit_count.max(1))).max(1.0);

    // IDF per query token (Robertson–Spärck Jones, in the always-positive Lucene form):
    // a token held by few units is worth far more than one held by all of them.
    let terms = query_terms(units, query_tokens);
    let document_frequency: Vec<f64> = terms
        .iter()
        .map(|term| {
            units
                .iter()
                .filter(|unit| term.frequency_in(unit) > 0.0)
                .count() as f64
        })
        .collect();
    let idf: Vec<f64> = document_frequency
        .iter()
        .map(|df| ((n - df + 0.5) / (df + 0.5) + 1.0).ln())
        .collect();

    // A term more than SELECTIVE_SHARE of the corpus holds cannot carry a hit by itself —
    // see the module's Ranking contract. "Answering" means a term that could actually select
    // a unit: held by something, and held by few enough things to mean anything.
    let answering: Vec<bool> = document_frequency
        .iter()
        .map(|df| *df > 0.0 && *df <= SELECTIVE_SHARE * n)
        .collect();
    // With no answering term the query is one of two things, and they end differently. If
    // every term is at least held, it asked only for words this corpus spreads everywhere —
    // ranking them is the best answer there is. If some term is held nowhere, the words that
    // would have selected are simply not here, and saying so beats ranking whatever said
    // "the".
    let applies = floor == Floor::Apply;
    let has_answering = answering.iter().any(|answering| *answering);
    let every_term_held = document_frequency.iter().all(|df| *df > 0.0);
    if applies && !has_answering && !every_term_held {
        return vec![0.0; units.len()];
    }
    let floor_applies = applies && has_answering;
    let idf_of: HashMap<&str, f64> = query_tokens
        .iter()
        .map(String::as_str)
        .zip(idf.iter().copied())
        .collect();

    units
        .iter()
        .map(|unit| {
            let mut score = 0.0f64;
            let mut answered = false;
            for ((term, token_idf), answering) in terms.iter().zip(&idf).zip(&answering) {
                let frequency = term.frequency_in(unit);
                if frequency <= 0.0 {
                    continue;
                }
                answered |= *answering;
                let saturated = frequency * (BM25_K1 + 1.0)
                    / (frequency
                        + BM25_K1 * (1.0 - BM25_B + BM25_B * unit.length / average_length));
                score += token_idf * saturated;
            }
            if floor_applies && !answered {
                return 0.0;
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

/// The line of `text` that answers `query`, or `None` when no line matches at all.
///
/// A hit is only cheaper than a read if the excerpt shown *states the fact*, and a block can
/// be far longer than an excerpt — so something has to choose which part of it to show. That
/// choice is a ranking, and this repository has exactly one ranker: the line is picked by
/// [`rank`], with the same stemming and vocabulary bridge that chose the block, so an
/// excerpt can land on the line saying `GPU` for a question about a graphics card. A second,
/// weaker matcher in a host is how a hit lands on the right file and shows the wrong
/// sentence — which reads as a miss, and costs the read it was saving.
///
/// Each **line** is one ranked unit, deliberately: ranking overlapping windows instead makes
/// document frequency meaningless, since one line then belongs to every window around it and
/// nothing looks rare. The caller decides how many lines to show from here, and what a line
/// is (one that drops blanks and fence markers passes what it kept). Ties go to the earliest
/// line, so the excerpt is deterministic.
///
/// Complexity: `O(l · q)` for `l` lines and `q` query tokens.
#[must_use]
pub fn answering_line(text: &str, query: &str) -> Option<usize> {
    let query_tokens = distinct_tokens(query);
    let lines: Vec<&str> = text.lines().collect();
    if query_tokens.is_empty() || lines.is_empty() {
        return None;
    }

    let stats: Vec<UnitStats> = lines
        .iter()
        .map(|line| {
            let mut recorder = Recorder::new();
            recorder.segment(WEIGHT_BODY, line);
            recorder.stats
        })
        .collect();
    let units: Vec<&UnitStats> = stats.iter().collect();
    let scores = rank(
        &units,
        &query_tokens,
        &phrase_pairs(query),
        None,
        Floor::Off,
    );

    scores
        .iter()
        .enumerate()
        .filter(|(_, score)| **score > 0.0)
        .max_by(|(left_index, left), (right_index, right)| {
            left.partial_cmp(right)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
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
    push_words(&mut raw, query);
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
    let (Some(first), Some(second)) = (unit.entries.get(first), unit.entries.get(second)) else {
        return false;
    };
    let (first_positions, second_positions) = (&first.positions, &second.positions);
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

    /// One index over the documents of several, built over disjoint sets.
    ///
    /// Tokenising is the expensive half of a search — three quarters of a query's wall clock
    /// over a repository of source files — and it is per-document work with nothing shared,
    /// so a host with cores to spare can index in parallel and join the parts here. The join
    /// itself is a concatenation: an index is its documents' statistics and nothing else.
    ///
    /// The result's corpus size is unset, because the parts *are* the corpus; a caller that
    /// narrowed before indexing restates it with [`SearchIndex::with_corpus_size`]. Order is
    /// the order the parts arrive in, which is what keeps a parallel build deterministic —
    /// scores never depend on it, and ties break by path regardless.
    ///
    /// This crate never spawns a thread itself: it compiles to `wasm32`, where the host owns
    /// concurrency. Splitting the work is the caller's decision; only the joining is here.
    #[must_use]
    pub fn merge(parts: impl IntoIterator<Item = SearchIndex>) -> SearchIndex {
        SearchIndex {
            docs: parts.into_iter().flat_map(|part| part.docs).collect(),
            corpus_size: None,
        }
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
            Floor::Apply,
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
///
/// A test is a label of the same kind. A chunk that asserts over a constant repeats the
/// query more often than the line declaring it, so test code scores at [`TEST_DISCOUNT`]
/// here — unless the query is itself about tests, when the test *is* the answer.
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
    let mut scores = rank(
        &units,
        &query_tokens,
        &phrase_pairs(query),
        None,
        Floor::Off,
    );
    if !asks_about_tests(&query_tokens) {
        for (score, (_, block)) in scores.iter_mut().zip(&candidates) {
            if is_test_code(&block.text) {
                *score *= TEST_DISCOUNT;
            }
        }
    }

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

/// Whether a query is asking about tests, in which case a test block is the answer and must
/// not be discounted. Tokens arrive stemmed, so `tests` and `testing` are both `test`.
fn asks_about_tests(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| matches!(token.as_str(), "test" | "spec" | "assert" | "fixture"))
}

/// Markers that a chunk of source is test code rather than the code under test.
///
/// One per ecosystem this repository or its readers write in. A marker is deliberately a
/// declaration — `#[test]`, `def test_`, `describe(` — never a bare word like "assert",
/// which appears in the code that raises the error a test checks for.
const TEST_MARKERS: &[&str] = &[
    "#[test]",
    "#[cfg(test)]",
    "mod tests",
    "def test_",
    "func Test",
    "TEST(",
    "TEST_F(",
    "describe(",
    "it(",
    "@Test",
    "assert_eq!",
    "assertEqual",
];

/// Whether a block's text reads as test code (see [`TEST_MARKERS`]).
fn is_test_code(text: &str) -> bool {
    TEST_MARKERS.iter().any(|marker| text.contains(marker))
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
/// Documents are **borrowed**, never copied: a caller with a loaded corpus hands out
/// `(path, &document)` pairs and pays for the statistics alone. Copying the corpus to index
/// it was the largest single cost of a project-wide search, and it bought nothing.
///
/// Complexity: `O(total tokens)` time and space across every document, since each token is
/// tallied and its position recorded exactly once.
pub fn build_index<'a>(docs: impl IntoIterator<Item = (&'a str, &'a Document)>) -> SearchIndex {
    let entries = docs
        .into_iter()
        .map(|(path, document)| DocEntry {
            path: path.to_string(),
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

/// Visit every word of `text` with the tokens it is indexed and matched under: the word's
/// own stem first, then the stems of a compound identifier's parts.
///
/// A callback rather than a returned list, and one buffer reused across the words, because
/// this runs over every byte of every document indexed — returning a vector per word made
/// tokenising the corpus cost more than searching it. One call is one word, which is what
/// lets a recorder give every token of a word the same position and charge its length once.
fn for_each_word(text: &str, mut visit: impl FnMut(&[String])) {
    let mut scratch: Vec<String> = Vec::new();
    for piece in text.split(|c: char| !c.is_alphanumeric()) {
        if piece.is_empty() {
            continue;
        }
        let used = expand_word(piece, &mut scratch);
        visit(&scratch[..used]);
    }
}

/// Write the tokens one word is indexed under into `tokens` — its stem, then the stems of
/// its identifier parts — and return how many were written.
///
/// `tokens` is a scratch buffer reused across words and **overwritten in place**, never
/// cleared and re-pushed: after the first few words it has the capacity every later word
/// needs, so tokenising a corpus stops allocating a string per word. Parts follow the whole
/// word, never replace it, and duplicates are dropped — a caller weights the first entry as
/// the word itself and the rest as parts.
fn expand_word(word: &str, tokens: &mut Vec<String>) -> usize {
    let lowered = if word.bytes().any(|byte| byte.is_ascii_uppercase()) || !word.is_ascii() {
        Cow::Owned(word.to_lowercase())
    } else {
        Cow::Borrowed(word)
    };
    let mut used = 0;
    let keep = |candidate: &str, tokens: &mut Vec<String>, used: &mut usize| {
        if tokens[..*used].iter().any(|held| held == candidate) {
            return;
        }
        match tokens.get_mut(*used) {
            Some(slot) => {
                slot.clear();
                slot.push_str(candidate);
            }
            None => tokens.push(candidate.to_string()),
        }
        *used += 1;
    };

    keep(&stem(&lowered), tokens, &mut used);
    // A word of plain lowercase letters has no boundary to find, and most words are that.
    let may_be_compound = !word.is_ascii()
        || word
            .bytes()
            .any(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
    if may_be_compound {
        for part in identifier_parts(word) {
            if part.chars().count() >= 2 {
                keep(&stem(&part), tokens, &mut used);
            }
        }
    }
    used
}

/// The parts of a compound identifier, lowercased — empty when the word is not compound.
///
/// Boundaries are the ones a programmer types: `bitPack` and `BitPack` break between case
/// runs, `HTTPServer` breaks before the last capital of a run, and `utf8Decode` breaks where
/// letters meet digits. A `snake_case` name is already split by the tokeniser, which treats
/// every non-alphanumeric character as a break.
fn identifier_parts(word: &str) -> Vec<String> {
    // Nothing is allocated until a boundary is found, which is the common case: an ordinary
    // lowercase word is not a compound identifier.
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0;
    let mut characters = word.char_indices().peekable();
    let mut previous: Option<char> = None;
    while let Some((index, current)) = characters.next() {
        if let Some(previous) = previous {
            let following = characters.peek().map(|(_, character)| *character);
            let case_break = previous.is_lowercase() && current.is_uppercase();
            let acronym_break = previous.is_uppercase()
                && current.is_uppercase()
                && following.is_some_and(char::is_lowercase);
            let digit_break = previous.is_numeric() != current.is_numeric();
            if case_break || acronym_break || digit_break {
                parts.push(word[start..index].to_lowercase());
                start = index;
            }
        }
        previous = Some(current);
    }
    if !parts.is_empty() {
        parts.push(word[start..].to_lowercase());
    }
    parts
}

/// Shortest word a suffix strip will touch, and the shortest remainder it may leave:
/// below this the strip has eaten the word rather than its ending.
const MIN_STEM: usize = 3;

/// A conservative English suffix strip, applied to indexed text and to queries alike.
///
/// Its whole job is to let `weights` find `weight` and `packed` find `pack` without letting
/// `string` find `str`. So it refuses a strip that would leave fewer than [`MIN_STEM`]
/// characters or no vowel, it never touches a word carrying a digit (`sha256`, `utf8`,
/// `x86`), and it applies exactly one rule — the first that matches. It is deliberately
/// short of Porter: it folds the endings a question and an answer differ by, and leaves
/// every ending whose rule would need a dictionary to be safe.
///
/// Applied to both sides of a search, an imperfect stem is still symmetric: it can fail to
/// join two spellings, and it cannot make a word match something it does not share a stem
/// with. Complexity: `O(n)` in the word's length.
fn stem(word: &str) -> Cow<'_, str> {
    // English suffixes are ASCII, and a word that is not is not one this strip understands.
    let letters = word.as_bytes();
    if letters.len() < MIN_STEM || !word.is_ascii() || letters.iter().any(u8::is_ascii_digit) {
        return Cow::Borrowed(word);
    }
    let ends = |suffix: &str| word.ends_with(suffix);
    let keep = |candidate: &str| candidate.len() >= MIN_STEM && candidate.bytes().any(is_vowel);
    let strip = |take: usize| -> Cow<'_, str> {
        let candidate = undouble(&word[..letters.len() - take]);
        if keep(candidate) {
            Cow::Borrowed(candidate)
        } else {
            Cow::Borrowed(word)
        }
    };

    if ends("sses") {
        return Cow::Borrowed(&word[..letters.len() - 2]);
    }
    if ends("ies") {
        let candidate = format!("{}y", &word[..letters.len() - 3]);
        return if keep(&candidate) {
            Cow::Owned(candidate)
        } else {
            Cow::Borrowed(word)
        };
    }
    if ends("s") && !ends("ss") && !ends("us") && !ends("is") {
        return strip(1);
    }
    if ends("ing") {
        return strip(3);
    }
    if ends("ed") {
        return strip(2);
    }
    Cow::Borrowed(word)
}

/// Collapse a stem's doubled final consonant, so `running` and `runs` meet at `run`.
///
/// `ll`, `ss`, and `zz` keep both letters: they are how English spells the word itself
/// (`call`, `pass`, `buzz`), not an artefact of the suffix.
fn undouble(stem: &str) -> &str {
    let letters = stem.as_bytes();
    let doubled = letters.len() > MIN_STEM
        && letters[letters.len() - 1] == letters[letters.len() - 2]
        && !is_vowel(letters[letters.len() - 1])
        && !matches!(letters[letters.len() - 1], b'l' | b's' | b'z');
    if doubled {
        &stem[..letters.len() - 1]
    } else {
        stem
    }
}

/// Whether `letter` is an English vowel — `y` counts, as it does in `carry`.
fn is_vowel(letter: u8) -> bool {
    matches!(letter, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

/// Lowercase, split on non-alphanumeric runs, and append every derived token to `out`.
fn push_tokens(out: &mut Vec<String>, text: &str) {
    for_each_word(text, |variants| out.extend_from_slice(variants));
}

/// Append only each word's own token — the first of its variants — to `out`.
///
/// Phrase adjacency is about the words that were typed, so the tokens a word *derives* must
/// not stand between it and the next word.
fn push_words(out: &mut Vec<String>, text: &str) {
    for_each_word(text, |variants| out.extend(variants.first().cloned()));
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
    /// Index a corpus the tests own by value; the engine indexes borrowed documents.
    fn indexed(docs: &[(String, Document)]) -> SearchIndex {
        build_index(
            docs.iter()
                .map(|(path, document)| (path.as_str(), document)),
        )
    }

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

    /// A corpus shaped like the first-contact battery's hard cases: one file that answers,
    /// and several that only share the question's ordinary words.
    fn a_corpus_that_says_it_its_own_way() -> Vec<(String, Document)> {
        let mut docs = vec![
            (
                "backend.c".to_string(),
                doc(
                    "backend.c",
                    vec![paragraph(
                        "BACKEND_AUTO tries the GPU first and falls back to the CPU \
                         when no device is present",
                    )],
                ),
            ),
            (
                "mic.py".to_string(),
                doc(
                    "mic.py",
                    vec![paragraph("open the mic at 16 kHz and read from the input")],
                ),
            ),
        ];
        for name in ["a", "b", "c", "d", "e"] {
            docs.push((
                format!("note_{name}.md"),
                doc(
                    &format!("note_{name}.md"),
                    vec![paragraph(
                        "where the values come from, and where they come from next; \
                         they come from the table above",
                    )],
                ),
            ));
        }
        docs
    }

    #[test]
    fn a_readers_word_reaches_the_acronym_the_code_writes() {
        // No rule over spelling gets from "graphics card" to GPU: the words share no
        // letters. The vocabulary bridge is what carries it.
        let docs = a_corpus_that_says_it_its_own_way();
        let hits =
            indexed(&docs).search("how does it pick between the graphics card and the processor");
        assert_eq!(
            hits.first().map(|hit| hit.path.as_str()),
            Some("backend.c"),
            "{hits:?}"
        );
    }

    #[test]
    fn a_word_the_corpus_never_says_beats_the_words_it_always_says() {
        // The long-question failure: five files saying "come from" used to outrank the one
        // file that answers, because ordinary words repeated often enough to win.
        let docs = a_corpus_that_says_it_its_own_way();
        let hits = indexed(&docs).search("where does microphone input come from");
        assert_eq!(hits.first().map(|hit| hit.path.as_str()), Some("mic.py"));
        assert!(
            hits.iter().all(|hit| !hit.path.starts_with("note_")),
            "a file matching only the question's ordinary words is not an answer: {hits:?}"
        );
    }

    #[test]
    fn a_question_the_corpus_cannot_answer_returns_nothing_rather_than_noise() {
        // Three plausible-looking hits that answer nothing cost a read and mislead. No
        // result is the honest answer, and the cheaper one.
        let docs = a_corpus_that_says_it_its_own_way();
        let hits = indexed(&docs).search("how do I configure the quantum flux capacitor");
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn a_query_of_only_common_words_still_ranks_them() {
        // The floor stands down when nothing in the query selects: asked for words
        // everything holds, ranking on them is the best answer available.
        let docs = a_corpus_that_says_it_its_own_way();
        let hits = indexed(&docs).search("come from");
        assert!(
            hits.iter().any(|hit| hit.path.starts_with("note_")),
            "{hits:?}"
        );
    }

    #[test]
    fn the_vocabulary_bridge_stays_out_of_the_way_when_the_corpus_says_the_word() {
        // Consulted only for a token no unit holds: a corpus that says "folder" must not
        // have its own word outranked by kin.
        let docs = vec![
            (
                "folders.md".to_string(),
                doc("folders.md", vec![paragraph("how a folder is created")]),
            ),
            (
                "dirs.md".to_string(),
                doc(
                    "dirs.md",
                    vec![paragraph(
                        "how a directory is created, and another directory",
                    )],
                ),
            ),
        ];
        let hits = indexed(&docs).search("folder");
        assert_eq!(
            hits.first().map(|hit| hit.path.as_str()),
            Some("folders.md")
        );
    }

    #[test]
    fn every_vocabulary_group_is_a_real_group_of_distinct_words() {
        // The table is a claim that these words are interchangeable. A one-word group
        // claims nothing, and a word in two groups quietly joins them.
        let mut seen: Vec<String> = Vec::new();
        for group in VOCABULARY {
            assert!(group.len() >= 2, "a group of one states nothing: {group:?}");
            for word in *group {
                let stemmed = stem(word).into_owned();
                assert!(
                    !seen.contains(&stemmed),
                    "`{word}` is in two groups, which merges them: {group:?}"
                );
                seen.push(stemmed);
            }
        }
    }

    #[test]
    fn indexing_in_parts_ranks_exactly_as_indexing_at_once() {
        // The property the parallel build rests on: how the work was split must not reach
        // the ranking. Same documents, same query, same scores — to the last bit.
        let docs = a_corpus_that_says_it_its_own_way();
        let whole = indexed(&docs).search("how does it pick the graphics card");
        let split = SearchIndex::merge(docs.chunks(3).map(|slice| {
            build_index(
                slice
                    .iter()
                    .map(|(path, document)| (path.as_str(), document)),
            )
        }))
        .search("how does it pick the graphics card");
        assert_eq!(whole, split);
        assert!(!whole.is_empty(), "the corpus does answer this");
    }

    #[test]
    fn the_answering_line_is_the_one_that_states_the_fact() {
        // A block far longer than an excerpt: the window must land on the answer, not on
        // the block's opening, and must find it through the vocabulary bridge too.
        let text = "an opening paragraph about the project and its goals\n\
                    more preamble that mentions nothing in particular\n\
                    TB_BACKEND_AUTO prefers the GPU and falls back to CPU\n\
                    a closing line";
        let start = answering_line(text, "how does it pick the graphics card").expect("a line");
        assert_eq!(
            text.lines().nth(start),
            Some("TB_BACKEND_AUTO prefers the GPU and falls back to CPU")
        );
    }

    #[test]
    fn a_block_that_answers_nothing_names_no_line() {
        let text = "one line\nanother line";
        assert!(answering_line(text, "quantum flux capacitor").is_none());
        assert!(answering_line("", "anything").is_none());
        assert!(answering_line("a line", "").is_none());
    }

    #[test]
    fn vocabulary_kin_are_returned_as_stems_and_never_include_the_word_asked_for() {
        let kin = vocabulary_kin(&stem("processor"));
        assert!(kin.contains(&"cpu".to_string()), "{kin:?}");
        assert!(!kin.contains(&stem("processor").into_owned()));
        assert!(vocabulary_kin("nothinglikethis").is_empty());
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
        let index = indexed(&[
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
        let index = indexed(&documents);
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
        let index = indexed(&[("n.dx".to_string(), document)]);
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
        let index = indexed(&docs);
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
        let index = indexed(&[("s.dx".to_string(), document)]);
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
        let index = indexed(&[
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
        let index = indexed(&[("one.dx".to_string(), one), ("both.dx".to_string(), both)]);
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
        let index = indexed(&documents);
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
        let index = indexed(&[
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
        let index = indexed(&[
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
        let index = indexed(&[
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
        let index = indexed(&[
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
        let full_hits = indexed(&full).search("alpha beta");
        let narrowed_hits = indexed(&matching)
            .with_corpus_size(full.len())
            .search("alpha beta");
        assert_eq!(full_hits, narrowed_hits);
        // Without the stated size, the survivors masquerade as the corpus and IDF shifts.
        let unstated_hits = indexed(&matching).search("alpha beta");
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
        let index = indexed(&[("a.dx".to_string(), doc("Title", vec![]))]);
        assert!(index.search("").is_empty());
        assert!(index.search("   ::  ").is_empty()); // tokenises to nothing
    }

    #[test]
    fn equal_scores_break_ties_by_path() {
        let a = doc("Same", vec![paragraph("same")]);
        let b = doc("Same", vec![paragraph("same")]);
        let index = indexed(&[("z.dx".to_string(), a), ("a.dx".to_string(), b)]);
        let hits = index.search("same");
        assert_eq!(hits[0].path, "a.dx");
        assert_eq!(hits[1].path, "z.dx");
        assert_eq!(hits[0].score, hits[1].score);
    }

    // ---- reaching a fact stated in the project's words, not the reader's ----------------

    /// The strip exists to join two spellings of one word, and to refuse when it cannot: a
    /// stem short enough to be another word entirely is a wrong match, not a loose one.
    #[test]
    fn stemming_folds_word_forms_and_refuses_when_the_remainder_is_not_a_word() {
        for (word, expected) in [
            ("weights", "weight"),
            ("packed", "pack"),
            ("packing", "pack"),
            ("running", "run"),
            ("runs", "run"),
            ("chunks", "chunk"),
            ("classes", "class"),
            ("queries", "query"),
        ] {
            assert_eq!(stem(word), expected, "{word}");
        }
        // Refusals: the remainder would be too short, has no vowel, or the word is spelled
        // that way — and a token carrying digits is a name, never an English word.
        for word in [
            "string", "ring", "pass", "sha256", "utf8", "bus", "this", "was",
        ] {
            assert_eq!(stem(word), word, "{word}");
        }
    }

    /// A question asked in the words a codebase composes its identifiers from reaches the
    /// identifier: `pack weights` finds `packWeights`, which is what a reader would grep for.
    #[test]
    fn a_compound_identifier_is_reachable_by_the_words_it_is_made_of() {
        let code = source_document(
            "src/weights.c",
            "void packWeights(Tensor *t) {\n    step();\n}\n",
        );
        let plain = source_document("src/other.c", "void step(Tensor *t) {\n    walk();\n}\n");
        let index = indexed(&[
            ("src/weights.c".to_string(), code),
            ("src/other.c".to_string(), plain),
        ]);
        let hits = index.search("pack weights");
        assert_eq!(
            hits.first().map(|hit| hit.path.as_str()),
            Some("src/weights.c")
        );
    }

    /// The measured first-contact miss: the project writes `bitpack` and never `pack`, so a
    /// question about packing matched nothing at all. A word no unit holds is looked for
    /// inside the words that do.
    #[test]
    fn a_word_the_corpus_never_says_alone_is_found_inside_the_words_that_carry_it() {
        let bitpack = source_document(
            "bitpack.c",
            "uint64_t bitpack_getu(const uint64_t *w) {\n    return *w;\n}\n",
        );
        let other = source_document(
            "audio.c",
            "void render(float *samples) {\n    mix(samples);\n}\n",
        );
        let index = indexed(&[
            ("bitpack.c".to_string(), bitpack),
            ("audio.c".to_string(), other),
        ]);
        let hits = index.search("how are weights packed");
        assert_eq!(hits.first().map(|hit| hit.path.as_str()), Some("bitpack.c"));
    }

    /// The loose tier is a fallback, not a second opinion: a word the corpus does hold is
    /// scored on that word alone, so a document merely containing it inside a longer word
    /// cannot outrank the one that says it.
    #[test]
    fn a_word_the_corpus_holds_is_not_also_matched_inside_longer_words() {
        let says = doc("Says it", vec![paragraph("the pack is written on save")]);
        let contains = doc(
            "Contains it",
            vec![paragraph("bitpack unpack repack bitpacking unpacked")],
        );
        let index = indexed(&[
            ("says.dx".to_string(), says),
            ("contains.dx".to_string(), contains),
        ]);
        let hits = index.search("pack");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "says.dx");
    }

    /// Deriving tokens from a word must not invent an adjacency: every token of one word
    /// shares that word's position, and only the word itself takes part in a phrase.
    #[test]
    fn derived_tokens_share_their_word_position_and_never_form_a_phrase() {
        let compound = doc("C", vec![paragraph("packWeights")]);
        let separated = doc("S", vec![paragraph("pack and then weights")]);
        let index = indexed(&[
            ("compound.dx".to_string(), compound),
            ("separated.dx".to_string(), separated),
        ]);
        // Both hold the words; neither typed them side by side, so neither earns the bonus.
        let hits = index.search("pack weights");
        assert_eq!(hits.len(), 2);
    }

    /// A test repeats the query — that is what a test does — so it wins on frequency and
    /// hands back an assertion where the reader asked for the fact.
    #[test]
    fn the_answering_block_is_the_code_that_states_a_fact_not_the_test_over_it() {
        let chunk = |id: &str, text: &str| Block {
            id: id.to_string(),
            kind: "code".to_string(),
            text: text.to_string(),
            ..Block::default()
        };
        let document = doc(
            "compress.rs",
            vec![
                chunk(
                    "L1-L2",
                    "//! A frame opens with four magic bytes naming its codec.\n\
const MAGIC_LZSS: &[u8; 4] = b\"DXZ1\";",
                ),
                chunk(
                    "L4-L13",
                    "#[cfg(test)]\n\
mod tests {\n\
    #[test]\n\
    fn every_magic_byte_decodes() {\n\
        for magic in [MAGIC_LZSS] {\n\
            let frame = magic.to_vec();\n\
            assert_eq!(magic_of(&frame), magic);\n\
        }\n\
    }\n\
}",
                ),
            ],
        );
        assert_eq!(
            best_block_id(&document, "what magic bytes does a frame carry").as_deref(),
            Some("L1-L2")
        );
        // Asked about the test, the test is the answer.
        assert_eq!(
            best_block_id(&document, "what test covers every magic byte").as_deref(),
            Some("L4-L13")
        );
    }
}
