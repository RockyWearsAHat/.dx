//! `dx coverage` — the document-hit rate over a project's recent searches, and what fell back.
//!
//! Reads what [`crate::workspace::search`] has been recording all along (`crate::coverage`):
//! the fraction of the last `--window` searches whose top hit landed on a document rather than
//! source or nothing, and the queries that did not, most-repeated first — the actionable half
//! of the report, since it names exactly what to write into a document next. `--min-rate`
//! turns the rate into a normal dx gate: the command exits non-zero when the rate falls short,
//! the same shape every other gate in `dev.dx` uses.

use std::path::PathBuf;

use crate::args::Args;
use crate::coverage;

/// Default number of recent searches a report is drawn from.
const DEFAULT_WINDOW: usize = 200;

/// `dx coverage [dir] [--window N] [--min-rate R]` — the coverage report, or a gate failure.
///
/// # Errors
/// Returns a sentence when `--min-rate` is given and the observed rate falls below it. A
/// workspace with no coverage data yet never fails a floor it has no evidence for — it reports
/// "no data" and succeeds regardless of `--min-rate`.
pub fn run(args: &Args) -> Result<String, String> {
    let root = root_of(args, 0);
    let window = args
        .number("window")
        .map_or(DEFAULT_WINDOW, |window| window as usize);

    let Some(report) = coverage::report(&root, window) else {
        return Ok(format!(
            "no coverage data yet for {} — nothing has been searched\n",
            root.display()
        ));
    };

    let mut out = format!(
        "{}/{} searches landed on a document ({:.0}%), over the last {} recorded\n",
        report.document_hits,
        report.total,
        report.document_rate * 100.0,
        report.window,
    );
    out.push_str(&format!(
        "  document {}  source {}  none {}\n",
        report.document_hits, report.source_hits, report.none_hits
    ));
    if report.fallbacks.is_empty() {
        out.push_str("  no fallbacks — every recorded search landed on a document\n");
    } else {
        out.push_str("  fell back to source or found nothing, most-repeated first:\n");
        for (query, count) in &report.fallbacks {
            out.push_str(&format!("    {count:>3}×  {query}\n"));
        }
    }

    if let Some(floor) = args
        .value("min-rate")
        .and_then(|value| value.trim().parse::<f64>().ok())
    {
        if report.document_rate < floor {
            return Err(format!(
                "{out}coverage rate {:.0}% is below the floor of {:.0}%",
                report.document_rate * 100.0,
                floor * 100.0
            ));
        }
    }

    Ok(out)
}

/// The directory to report on: positional `index` if given, else the current directory.
fn root_of(args: &Args, index: usize) -> PathBuf {
    args.positional(index)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{self, Hit, Loaded};
    use doc_core::format::parse;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn workspace_dir(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-coverage-cmd-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        workspace::save(
            &root.join("seed.dx"),
            &parse("::heading level=1 id=h\nSeed\n::end\n"),
        )
        .expect("seed");
        root
    }

    fn hit(relative: &str) -> Hit {
        Hit {
            document: Loaded {
                path: PathBuf::from(relative),
                relative: relative.to_string(),
                document: parse("::paragraph id=p\nx\n::end\n"),
            },
            score: 1.0,
            block: None,
        }
    }

    #[test]
    fn a_fresh_workspace_reports_no_data_and_succeeds_even_with_a_floor() {
        let root = workspace_dir("fresh");
        let out = run(&args(&[&root.to_string_lossy(), "--min-rate", "0.9"])).expect("no data");
        assert!(out.contains("no coverage data yet"));
    }

    #[test]
    fn a_healthy_rate_reports_and_passes_its_floor() {
        let root = workspace_dir("healthy");
        coverage::record(&root, "q1", &[hit("guide.dx")]);
        coverage::record(&root, "q2", &[hit("guide.dx")]);
        let out = run(&args(&[&root.to_string_lossy(), "--min-rate", "0.5"])).expect("passes");
        assert!(out.contains("2/2 searches landed on a document (100%)"));
    }

    #[test]
    fn a_rate_below_the_floor_fails_the_gate() {
        let root = workspace_dir("low");
        coverage::record(&root, "asked and missed", &[hit("main.rs")]);
        let error = run(&args(&[&root.to_string_lossy(), "--min-rate", "0.9"]))
            .expect_err("should fail the floor");
        assert!(error.contains("below the floor"));
        assert!(error.contains("asked and missed"));
    }

    #[test]
    fn fallback_queries_are_listed_best_first() {
        let root = workspace_dir("fallbacks");
        coverage::record(&root, "common miss", &[hit("main.rs")]);
        coverage::record(&root, "common miss", &[]);
        coverage::record(&root, "rare miss", &[hit("main.rs")]);
        let out = run(&args(&[&root.to_string_lossy()])).expect("report");
        let common_at = out.find("common miss").expect("common miss listed");
        let rare_at = out.find("rare miss").expect("rare miss listed");
        assert!(common_at < rare_at, "{out}");
    }

    #[test]
    fn the_window_flag_is_honored() {
        let root = workspace_dir("window");
        coverage::record(&root, "old", &[hit("main.rs")]);
        coverage::record(&root, "new", &[hit("guide.dx")]);
        let out = run(&args(&[&root.to_string_lossy(), "--window", "1"])).expect("report");
        assert!(out.contains("1/1 searches"));
    }
}
