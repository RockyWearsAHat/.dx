//! Gate verdicts for a checkout, read without executing anything.
//!
//! Every runnable block of every `.dx` document (the listing `dx ls` makes) is placed into
//! one [`GateState`] from its recorded `::output`, this machine's approval ledger and the
//! block's present fingerprint, all computed by `doc_run::standing` (the same code the
//! runner uses before it would execute).

use std::path::{Path, PathBuf};

use doc_run::RunOptions;

use crate::workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateState {
    Pass,
    Fail,
    Stale,
    Unrun,
    Unapproved,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GateVerdict {
    pub doc: String,
    pub block: String,
    pub state: GateState,
    pub ms: Option<u64>,
    pub tail: String,
}

const TAIL_CAP: usize = 300;

/// Verdicts for every runnable block under `checkout`, in document then block order.
/// Nothing is executed or written. A document that will not resolve yields no verdicts.
///
/// Fingerprints come from doc-run's memo, which this loads from and saves to
/// [`memo_file`] so a fresh process (`dx live --once`, the MCP read) starts warm: over
/// unchanged inputs a call is stat calls only and hashes no file bytes.
pub fn verdicts(checkout: &Path) -> Vec<GateVerdict> {
    let memo = memo_file(checkout);
    if let Some(memo) = &memo {
        doc_run::load_memo(memo);
    }
    let out = verdicts_in(checkout, RunOptions::default().cache_root);
    if let Some(memo) = &memo {
        let _ = doc_run::save_memo(memo);
    }
    out
}

/// Where the fingerprint memo of `checkout`'s repository lives: beside the live snapshot,
/// `<git-common-dir>/dx-live/fingerprints.v1`. Found by reading `.git` (a folder, or a
/// worktree's `gitdir:` file and its `commondir`) — no git process — so an idle read stays
/// stat calls only. `None` outside a git checkout.
pub fn memo_file(checkout: &Path) -> Option<PathBuf> {
    common_dir(checkout).map(|common| common.join("dx-live").join("fingerprints.v1"))
}

fn common_dir(checkout: &Path) -> Option<PathBuf> {
    let dot_git = checkout.join(".git");
    let meta = std::fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?.trim();
    let gitdir = checkout.join(gitdir);
    match std::fs::read_to_string(gitdir.join("commondir")) {
        Ok(common) => Some(gitdir.join(common.trim())),
        Err(_) => Some(gitdir),
    }
}

fn verdicts_in(checkout: &Path, cache_root: PathBuf) -> Vec<GateVerdict> {
    let Ok(listing) = workspace::load_all(checkout) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for loaded in &listing.documents {
        let options = RunOptions {
            document_dir: workspace::document_dir(&loaded.path),
            cache_root: cache_root.clone(),
            ..RunOptions::default()
        };
        let resolver = workspace::resolver_for(&loaded.path);
        for block in doc_run::standing(&loaded.document, &options, &resolver) {
            let (state, tail) = classify(&block);
            out.push(GateVerdict {
                doc: loaded.relative.clone(),
                block: block.id,
                state,
                // An `::output` records no duration, so there is none to report.
                ms: None,
                tail,
            });
        }
    }
    out
}

fn classify(block: &doc_run::BlockStanding) -> (GateState, String) {
    if let Some(problem) = &block.problem {
        // Inputs that cannot be resolved can never pass: a failure, with its sentence.
        return (GateState::Fail, tail_of(problem));
    }
    let tail = block
        .recorded
        .as_ref()
        .map(|(_, _, text)| tail_of(text))
        .unwrap_or_default();
    // The approval gate stands ahead of the cache, exactly as in the runner.
    if !block.approved {
        return (GateState::Unapproved, tail);
    }
    match &block.recorded {
        None => (GateState::Unrun, tail),
        Some((hash, exit, _)) if Some(hash) == block.fingerprint.as_ref() => {
            (if *exit == 0 { GateState::Pass } else { GateState::Fail }, tail)
        }
        Some(_) => (GateState::Stale, tail),
    }
}

/// Last 3 non-empty lines joined by ' | ', capped at 300 characters.
fn tail_of(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "(no output)")
        .collect();
    let joined = lines[lines.len().saturating_sub(3)..].join(" | ");
    joined.chars().take(TAIL_CAP).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn temp(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-live-gates-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        root
    }

    fn run(root: &Path, cache: &Path, approve: bool) {
        let path = root.join("g.dx");
        let source = std::fs::read_to_string(&path).expect("read");
        let report = doc_run::run_document(
            &source,
            &RunOptions {
                document_dir: root.to_path_buf(),
                cache_root: cache.to_path_buf(),
                approve,
                ..RunOptions::default()
            },
            &workspace::resolver_for(&path),
        )
        .expect("run");
        std::fs::write(&path, report.source).expect("write");
    }

    const DOC: &str = "::code id=good lang=bash run reads=data.txt\ncat data.txt\n::end\n\n\
                       ::code id=bad lang=bash run\necho one\necho two\nexit 3\n::end\n";

    fn state(v: &[GateVerdict], id: &str) -> GateState {
        v.iter().find(|g| g.block == id).expect("block").state
    }

    #[test]
    fn pass_fail_stale_unrun_unapproved() {
        let root = temp("states");
        let cache = root.join("cache");
        std::fs::write(root.join("data.txt"), "a\n").unwrap();
        std::fs::write(root.join("g.dx"), DOC).unwrap();

        let v = verdicts_in(&root, cache.clone());
        assert_eq!(state(&v, "good"), GateState::Unapproved);

        run(&root, &cache, true);
        let v = verdicts_in(&root, cache.clone());
        assert_eq!(state(&v, "good"), GateState::Pass, "{v:?}");
        assert_eq!(state(&v, "bad"), GateState::Fail, "{v:?}");
        let bad = v.iter().find(|g| g.block == "bad").unwrap();
        assert_eq!(bad.doc, "g.dx");
        assert!(bad.tail.ends_with("one | two"), "{}", bad.tail);

        std::fs::write(root.join("data.txt"), "changed\n").unwrap();
        let v = verdicts_in(&root, cache.clone());
        assert_eq!(state(&v, "good"), GateState::Stale, "{v:?}");
        assert_eq!(state(&v, "bad"), GateState::Fail);

        // Approved but never run -> Unrun.
        let fresh = temp("unrun");
        std::fs::write(fresh.join("data.txt"), "a\n").unwrap();
        std::fs::write(fresh.join("g.dx"), DOC).unwrap();
        // Same code as the approved document above: its approval stands on this machine.
        let v = verdicts_in(&fresh, cache);
        assert_eq!(state(&v, "good"), GateState::Unrun, "{v:?}");
    }

    #[test]
    fn tail_is_last_three_lines_capped() {
        assert_eq!(tail_of("a\n\nb\nc\nd\n"), "b | c | d");
        assert_eq!(tail_of(&"x".repeat(500)).len(), 300);
        assert_eq!(tail_of("(no output)"), "");
    }

    #[test]
    fn reads_the_dx_documents_quickly() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let timed = || {
            let hashed = doc_run::hashed_bytes();
            let started = Instant::now();
            let v = verdicts(&docs);
            (format!("{v:?}"), v.len(), started.elapsed(), doc_run::hashed_bytes() - hashed)
        };
        let (cold, n, cold_took, cold_bytes) = timed();
        // Warm: every unchanged input answers from the fingerprint memo — stat calls only.
        let (warm, _, warm_took, warm_bytes) = timed();
        // A fresh process: nothing in memory, the memo file beside the snapshot loaded.
        doc_run::forget_all();
        let (fresh, _, fresh_took, fresh_bytes) = timed();
        eprintln!(
            "{n} verdicts: cold {cold_took:?} ({cold_bytes} B hashed), warm {warm_took:?} \
             ({warm_bytes} B), fresh process {fresh_took:?} ({fresh_bytes} B)"
        );
        assert_eq!(cold, warm, "a memoized fingerprint must not change a verdict");
        assert_eq!(cold, fresh, "a persisted fingerprint must not change a verdict");
        assert_eq!(warm_bytes, 0, "a warm read hashes no bytes");
        assert_eq!(fresh_bytes, 0, "a fresh process after a warm run hashes no bytes");
        // Only an optimised build is held to the live bound; an unoptimised one need only
        // complete.
        if !cfg!(debug_assertions) {
            assert!(warm_took < Duration::from_millis(100), "warm {warm_took:?}");
            assert!(fresh_took < Duration::from_millis(100), "fresh {fresh_took:?}");
        }
    }
}
