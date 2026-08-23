//! Search coverage: a discarded fact, kept.
//!
//! [`crate::workspace::search`] already knows, for every query, whether the hit it is about to
//! hand back landed on a document or fell back to source — and used to throw that fact away the
//! moment it returned. This module gives it somewhere to land instead: one best-effort JSONL
//! line per search, appended to `.doc/coverage.jsonl`, that a caller can total up later with
//! [`report`]. `docs/search-coverage.dx` is the design; this module is the mechanism it
//! describes.
//!
//! # What this is not
//! Not a store, not a schema, not a second `.doc/index.db`. `doc-store::schema` owns
//! rebuildable, derived data with a versioned `DROP_ALL`; coverage history is neither derived
//! from anything else on disk nor safe to drop on a version bump, so it lives in its own
//! append-only file with no schema to migrate. `doc-run::approvals::Ledger` is the other
//! precedent this deliberately does not follow: that ledger is one fingerprint store *per
//! machine*, kept outside every repository. Coverage is the reverse — per-workspace,
//! git-ignored, meaningless once copied to another machine.
//!
//! # The contract
//! Every write here is best-effort — a missing `.doc` directory, a read-only mount, a
//! permission error all fall through to a silent no-op, because a search must never fail, slow
//! down measurably, or change its answer on account of a sidecar file it does not need in
//! order to search. [`record`] never creates `.doc` itself, so a workspace `dx` has never
//! touched stays untouched. The log prunes itself past [`MAX_ENTRIES`] lines, so it stays
//! bounded no matter how long a workspace lives.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use serde_json::{json, Value};

use crate::workspace::Hit;

/// Name of the coverage log, inside the workspace's `.doc` directory.
const LOG_NAME: &str = "coverage.jsonl";

/// Once the log passes this many lines, [`record`] rewrites it down to [`PRUNE_KEEP`] — bounded
/// so a long-lived workspace never grows an unbounded file.
const MAX_ENTRIES: usize = 2000;

/// How many of the most recent lines survive a prune.
const PRUNE_KEEP: usize = 1000;

/// Where a search's top hit landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landed {
    /// The top hit was a `.dx` document.
    Document,
    /// The top hit was a source file.
    Source,
    /// The search returned nothing.
    None,
}

impl Landed {
    fn as_str(self) -> &'static str {
        match self {
            Landed::Document => "document",
            Landed::Source => "source",
            Landed::None => "none",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "document" => Some(Landed::Document),
            "source" => Some(Landed::Source),
            "none" => Some(Landed::None),
            _ => None,
        }
    }
}

/// One recorded search: the query, and where its top hit landed.
#[derive(Debug, Clone)]
struct Entry {
    query: String,
    landed: Landed,
}

/// Record one search's outcome, best-effort.
///
/// `hits` is the final, already-ranked, already-capped list a caller is about to receive — the
/// first entry, if any, is what coverage judges by, because that is the hit a caller actually
/// sees and acts on. Never creates `.doc`; never panics; never surfaces an error, because
/// recording is not part of what a search promises to do.
pub fn record(directory: &Path, query: &str, hits: &[Hit]) {
    let doc_dir = directory.join(".doc");
    if !doc_dir.is_dir() {
        return;
    }
    let (landed, path) = match hits.first() {
        Some(hit) if hit.document.relative.ends_with(".dx") => {
            (Landed::Document, Some(hit.document.relative.as_str()))
        }
        Some(hit) => (Landed::Source, Some(hit.document.relative.as_str())),
        None => (Landed::None, None),
    };
    let line = json!({
        "query": query,
        "landed": landed.as_str(),
        "path": path,
    })
    .to_string();

    let log_path = doc_dir.join(LOG_NAME);
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    else {
        return;
    };
    if writeln!(file, "{line}").is_err() {
        return;
    }
    drop(file);
    prune_if_needed(&log_path);
}

/// Rewrite the log down to [`PRUNE_KEEP`] lines once it passes [`MAX_ENTRIES`].
///
/// Best-effort like [`record`]: a failure here just means the log grows a little past its
/// bound until the next successful search prunes it again.
fn prune_if_needed(log_path: &Path) {
    let Ok(contents) = fs::read_to_string(log_path) else {
        return;
    };
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= MAX_ENTRIES {
        return;
    }
    let kept = lines[lines.len() - PRUNE_KEEP..].join("\n");
    let _ = fs::write(log_path, kept + "\n");
}

/// Read every entry the log holds, oldest first, silently skipping a line that fails to parse
/// (hand-edited or truncated by a crash mid-write) rather than losing the rest of the log.
fn read_entries(directory: &Path) -> Vec<Entry> {
    let log_path = directory.join(".doc").join(LOG_NAME);
    let Ok(contents) = fs::read_to_string(log_path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            let query = value.get("query")?.as_str()?.to_string();
            let landed = Landed::parse(value.get("landed")?.as_str()?)?;
            Some(Entry { query, landed })
        })
        .collect()
}

/// A window's worth of coverage: the rate a search landed on a document, and what fell back.
#[derive(Debug, Clone)]
pub struct Report {
    /// How many of the most recent entries this report asked for.
    pub window: usize,
    /// Entries actually available to answer from (may be less than `window`).
    pub total: usize,
    /// Count that landed on a document.
    pub document_hits: usize,
    /// Count that landed on a source file.
    pub source_hits: usize,
    /// Count that landed on nothing.
    pub none_hits: usize,
    /// `document_hits / total` — the coverage rate.
    pub document_rate: f64,
    /// Queries that did not land on a document, most-repeated first, then alphabetically.
    pub fallbacks: Vec<(String, usize)>,
}

/// Summarize the last `window` searches — `None` when the workspace has no coverage data yet,
/// because a project with nothing recorded has no rate to report, not a rate of zero.
#[must_use]
pub fn report(directory: &Path, window: usize) -> Option<Report> {
    let mut entries = read_entries(directory);
    if entries.is_empty() {
        return None;
    }
    if entries.len() > window {
        entries = entries.split_off(entries.len() - window);
    }

    let total = entries.len();
    let mut document_hits = 0;
    let mut source_hits = 0;
    let mut none_hits = 0;
    let mut fallbacks: Vec<(String, usize)> = Vec::new();
    for entry in &entries {
        match entry.landed {
            Landed::Document => document_hits += 1,
            Landed::Source => {
                source_hits += 1;
                bump(&mut fallbacks, &entry.query);
            }
            Landed::None => {
                none_hits += 1;
                bump(&mut fallbacks, &entry.query);
            }
        }
    }
    fallbacks.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    Some(Report {
        window,
        total,
        document_hits,
        source_hits,
        none_hits,
        document_rate: document_hits as f64 / total as f64,
        fallbacks,
    })
}

/// Add one more occurrence of `query` to `counts`, in whatever order it is already in.
fn bump(counts: &mut Vec<(String, usize)>, query: &str) {
    if let Some(existing) = counts.iter_mut().find(|(seen, _)| seen == query) {
        existing.1 += 1;
    } else {
        counts.push((query.to_string(), 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{self, Loaded};
    use doc_core::format::parse;

    fn workspace_dir(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("dx-coverage-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        // `workspace::save` creates `.doc` as a side effect of writing a document — the same
        // way a real workspace gets one, so `record` finds it exactly as it would in the field.
        workspace::save(
            &root.join("seed.dx"),
            &parse("::heading level=1 id=h\nSeed\n::end\n"),
        )
        .expect("seed");
        root
    }

    fn loaded(relative: &str) -> Loaded {
        Loaded {
            path: std::path::PathBuf::from(relative),
            relative: relative.to_string(),
            document: parse("::paragraph id=p\nx\n::end\n"),
        }
    }

    fn hit(relative: &str) -> Hit {
        Hit {
            document: loaded(relative),
            score: 1.0,
            block: None,
        }
    }

    #[test]
    fn a_fresh_workspace_reports_no_data() {
        let root = workspace_dir("fresh");
        assert!(report(&root, 200).is_none());
    }

    #[test]
    fn record_is_a_silent_no_op_without_a_doc_directory() {
        let root = std::env::temp_dir().join("dx-coverage-tests-untouched");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        record(&root, "anything", &[hit("guide.dx")]);
        assert!(!root.join(".doc").exists());
    }

    #[test]
    fn a_document_hit_and_a_source_hit_and_no_hit_all_count_distinctly() {
        let root = workspace_dir("mixed");
        record(&root, "landed on a document", &[hit("guide.dx")]);
        record(&root, "landed on source", &[hit("main.rs")]);
        record(&root, "landed nowhere", &[]);

        let report = report(&root, 200).expect("report");
        assert_eq!(report.total, 3);
        assert_eq!(report.document_hits, 1);
        assert_eq!(report.source_hits, 1);
        assert_eq!(report.none_hits, 1);
        assert!((report.document_rate - (1.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn fallbacks_are_grouped_and_ranked_by_how_often_they_repeat() {
        let root = workspace_dir("fallbacks");
        for _ in 0..3 {
            record(&root, "asked often", &[hit("main.rs")]);
        }
        record(&root, "asked once", &[]);
        record(&root, "answered fine", &[hit("guide.dx")]);

        let report = report(&root, 200).expect("report");
        assert_eq!(
            report.fallbacks,
            vec![
                ("asked often".to_string(), 3),
                ("asked once".to_string(), 1),
            ]
        );
    }

    #[test]
    fn the_window_keeps_only_the_most_recent_entries() {
        let root = workspace_dir("window");
        record(&root, "old", &[hit("main.rs")]);
        record(&root, "new", &[hit("guide.dx")]);

        let report = report(&root, 1).expect("report");
        assert_eq!(report.total, 1);
        assert_eq!(report.document_hits, 1);
        assert!(report.fallbacks.is_empty());
    }

    #[test]
    fn the_log_prunes_itself_once_it_grows_past_the_bound() {
        let root = workspace_dir("prune");
        let writes = MAX_ENTRIES + 5;
        for i in 0..writes {
            record(&root, &format!("q{i}"), &[hit("guide.dx")]);
        }
        let lines = fs::read_to_string(root.join(".doc").join(LOG_NAME))
            .expect("log")
            .lines()
            .count();
        // The prune trips exactly once, the moment the log passes MAX_ENTRIES lines, cutting
        // it to PRUNE_KEEP — so the final count is PRUNE_KEEP plus whatever landed afterward
        // (the writes still left once MAX_ENTRIES was crossed), never anywhere near `writes`.
        assert!(
            lines <= PRUNE_KEEP + (writes - MAX_ENTRIES),
            "log did not prune: {lines} lines"
        );
        assert!(lines < writes, "log grew without bound: {lines} lines");
    }
}
