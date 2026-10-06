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
    /// Absolute paths of the raster images an `::image for=<this block>` in the same document
    /// claims this gate produced (the screens it proves), in document order. The board shows
    /// them only while the state is `pass`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

const TAIL_CAP: usize = 300;

/// Verdicts for every runnable block under `checkout`, in document then block order.
/// Nothing is executed or written. A document that will not resolve yields no verdicts.
pub fn verdicts(checkout: &Path) -> Vec<GateVerdict> {
    verdicts_in(checkout, RunOptions::default().cache_root)
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
            let images = images_for(&loaded.document, &options.document_dir, &block.id);
            out.push(GateVerdict {
                doc: loaded.relative.clone(),
                block: block.id,
                state,
                // An `::output` records no duration, so there is none to report.
                ms: None,
                tail,
                images,
            });
        }
    }
    out
}

/// The image files `::image ... for=<producer>` blocks of `document` name, made absolute
/// against the document's folder. Only a confined folder path to a raster file counts: a
/// remote URL or a `data:` URI is not a file this gate wrote.
fn images_for(document: &doc_core::model::Document, dir: &Path, producer: &str) -> Vec<String> {
    document
        .blocks
        .iter()
        .filter(|b| b.kind == "image" && b.for_block == producer)
        .filter_map(|b| doc_core::resolve::confined(&b.src))
        .filter(|src| super::board::raster_extension(src).is_some())
        .map(|src| dir.join(src).display().to_string())
        .collect()
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
        Some((hash, exit, _)) if Some(hash) == block.fingerprint.as_ref() => (
            if *exit == 0 {
                GateState::Pass
            } else {
                GateState::Fail
            },
            tail,
        ),
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
        let started = Instant::now();
        let v = verdicts(&docs);
        // Fingerprints hash every `reads=` file; an unoptimised build hashes ~15x slower
        // (about 2.3 s here), so the generous bound is 2 s optimised and 10 s unoptimised.
        let bound = if cfg!(debug_assertions) { 10 } else { 2 };
        assert!(
            started.elapsed() < Duration::from_secs(bound),
            "{:?}",
            started.elapsed()
        );
        eprintln!("{} verdicts in {:?}", v.len(), started.elapsed());
    }
}
