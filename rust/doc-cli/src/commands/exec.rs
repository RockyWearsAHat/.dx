//! `dx run` — execute a document's code blocks and save the results back into it.
//!
//! This is the only command that runs code, and it never happens by accident: reading,
//! rendering, and screenshotting a document never execute anything.

use std::path::PathBuf;
use std::time::Duration;

use doc_run::{run_document, RunOptions, RunReport, DEFAULT_TIMEOUT_SECONDS};

use crate::args::Args;
use crate::commands::view::document_path;
use crate::workspace;

/// `dx run <file>` — run the document's `run` blocks and write the outputs back.
///
/// `--dry` runs everything but prints the result instead of saving, which is how you
/// check what a document would do before letting it change the file.
///
/// New or edited code is gated on review: `--review` prints each runnable block's exact
/// code, fingerprint, and approval standing without executing anything; `--approve`
/// records the current fingerprints as approved and runs; `--force` runs unapproved code
/// once, announced in its output. Editing a block changes its fingerprint, so approval
/// expires with the edit.
///
/// `--follow-edges` runs the blocks in the order the document's board edges state
/// instead of document order: at every step the earliest ready block by document
/// position runs next, so an edge that defers a block lets later blocks — on a board or
/// not — run before it, and ties always break by document order. Document-order side
/// effects between blocks no edge relates are not preserved — state an edge if you need
/// an order. A cycle among runnable blocks is refused with a sentence naming it. The
/// flag composes with the others: `--review` lists the blocks in the order they would
/// run, except `--approve` and `--force` — review shows code without executing or
/// recording, so the engine refuses either pair rather than half-honouring it.
pub fn run(args: &Args) -> Result<String, String> {
    run_in(args, RunOptions::default().cache_root)
}

/// The body of [`run`], with the run cache — the approval ledger included — stated rather
/// than defaulted.
///
/// An approval is machine-wide and permanent, so the suite must never record one in the
/// developer's real cache: a test that approved `echo hi` would hand every later document
/// containing that line a standing approval, and the gate's own tests would be green only
/// because nothing had happened to approve them yet.
fn run_in(args: &Args, cache_root: PathBuf) -> Result<String, String> {
    let path = document_path(args)?;
    let source = workspace::read(&path)?;

    let timeout_seconds = args
        .number("timeout")
        .filter(|seconds| *seconds > 0)
        .map_or(DEFAULT_TIMEOUT_SECONDS, u64::from);

    let report = run_document(
        &source,
        &RunOptions {
            document_dir: workspace::document_dir(&path),
            cache_root,
            default_timeout: Duration::from_secs(timeout_seconds),
            force: args.present("force"),
            only: args.value("only").map(str::to_string),
            review_only: args.present("review"),
            approve: args.present("approve"),
            follow_board_edges: args.present("follow-edges"),
        },
        &workspace::resolver_for(&path),
    )?;

    if args.present("review") {
        return Ok(review(&report));
    }

    let mut output = summary(&report);
    if args.present("dry") {
        output.push_str("\n--- not saved (--dry) ---\n");
    } else if report.changed {
        workspace::write_text(&path, &report.source)?;
        output.push_str(&format!("saved {}\n", path.display()));
    } else {
        output.push_str("no changes\n");
    }

    if report.all_succeeded() {
        Ok(output)
    } else {
        Err(output)
    }
}

/// The `--review` report: each runnable block's code, fingerprint, and approval standing.
///
/// Full text, never previews — review exists so the reader can see exactly what would
/// run, and a truncated line defeats the inspection it is for. Nothing executed and
/// nothing was saved, and the report ends by saying so.
fn review(report: &RunReport) -> String {
    if report.runs.is_empty() {
        return "no runnable blocks — mark a code block with `run` to execute it\n".to_string();
    }
    let mut out = String::new();
    for entry in &report.runs {
        out.push_str(&format!(
            "block {} ({})\n{}\n\n",
            entry.id, entry.language, entry.output
        ));
    }
    out.push_str("review only — nothing ran, nothing saved. `dx run --approve` approves this code and runs it.\n");
    out
}

/// A per-block report: what ran, how it ended, and how long it took.
fn summary(report: &RunReport) -> String {
    if report.runs.is_empty() {
        return "no runnable blocks — mark a code block with `run` to execute it\n".to_string();
    }

    let width = report
        .runs
        .iter()
        .map(|entry| entry.id.len())
        .max()
        .unwrap_or(2);
    let mut out = String::new();
    for entry in &report.runs {
        // Only a skip means a cached result; a blocked block never ran at all.
        let timing = if entry.duration_ms > 0 {
            format!("{} ms", entry.duration_ms)
        } else if entry.status == "skipped" {
            "cached".to_string()
        } else {
            "—".to_string()
        };
        out.push_str(&format!(
            "{:<width$}  {:<8} {:<10} {}\n",
            entry.id,
            entry.status,
            timing,
            preview(&entry.output),
            width = width
        ));
        // A refusal's whole sentence says what to run next, and the terminal is the only
        // place it lives — a gate-blocked block leaves no ::output — so it prints in full.
        if entry.status == "blocked" {
            for line in entry.output.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
    }
    out
}

/// How wide a one-line preview may be before it is cut short.
const PREVIEW_WIDTH: usize = 56;

/// A one-line preview of what a block produced, for the per-block summary.
///
/// A block that drew something produced markup, and quoting the opening tag of an SVG tells a
/// reader nothing while filling their terminal with a line they cannot read. Such an output is
/// named instead of quoted; anything else is its first line, cut to [`PREVIEW_WIDTH`].
fn preview(output: &str) -> String {
    let first = output.lines().next().unwrap_or("").trim();
    if first.starts_with("<svg") {
        return "a drawing".to_string();
    }
    if first.starts_with('<') {
        return "markup".to_string();
    }
    if first.chars().count() <= PREVIEW_WIDTH {
        return first.to_string();
    }
    let cut: String = first.chars().take(PREVIEW_WIDTH - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    /// A scratch document, in a directory wiped first — cache and ledger with it.
    fn seed(label: &str, source: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-exec-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("doc.dx");
        workspace::write_text(&path, source).expect("seed");
        path
    }

    /// This test's own run cache, so nothing it approves outlives it.
    fn cache(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("dx-exec-tests-{label}"))
            .join("cache")
    }

    #[test]
    fn running_writes_the_output_into_the_document() {
        let path = seed("basic", "::code id=hello lang=bash run\necho hi\n::end\n");
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--approve"]),
            cache("basic"),
        )
        .expect("run");
        assert!(report.contains("hello"));
        assert!(report.contains("ok"));

        let saved = std::fs::read_to_string(&path).expect("read");
        assert!(saved.contains("::output id=hello-output for=hello status=ok"));
        assert!(saved.contains("hi"));
    }

    #[test]
    fn dry_runs_report_without_touching_the_file() {
        let source = "::code id=hello lang=bash run\necho hi\n::end\n";
        let path = seed("dry", source);
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--dry", "--approve"]),
            cache("dry"),
        )
        .expect("run");
        assert!(report.contains("not saved"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), source);
    }

    #[test]
    fn a_failing_block_is_reported_as_an_error_but_still_recorded() {
        let path = seed("failure", "::code id=bad lang=bash run\nexit 7\n::end\n");
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--approve"]),
            cache("failure"),
        )
        .expect_err("should report failure");
        assert!(report.contains("error"));
        assert!(std::fs::read_to_string(&path)
            .expect("read")
            .contains("exit=7"));
    }

    #[test]
    fn a_document_with_nothing_to_run_says_so() {
        let path = seed("nothing", "::paragraph id=p\njust prose\n::end\n");
        let report = run_in(&args(&[&path.to_string_lossy()]), cache("nothing")).expect("run");
        assert!(report.contains("no runnable blocks"));
    }

    #[test]
    fn the_second_run_of_an_unchanged_document_is_cached() {
        let path = seed("cache", "::code id=c lang=bash run\necho stable\n::end\n");
        run_in(
            &args(&[&path.to_string_lossy(), "--approve"]),
            cache("cache"),
        )
        .expect("first");
        let second = run_in(&args(&[&path.to_string_lossy()]), cache("cache")).expect("second");
        assert!(second.contains("skipped"));
        assert!(second.contains("cached"));
    }

    #[test]
    fn only_limits_execution_to_one_block() {
        let path = seed(
            "only",
            "::code id=a lang=bash run\necho a\n::end\n\n::code id=b lang=bash run\necho b\n::end\n",
        );
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--only", "b", "--approve"]),
            cache("only"),
        )
        .expect("run");
        assert!(report.contains("b  "));
        assert!(!report.lines().any(|line| line.starts_with("a ")));
    }

    #[test]
    fn follow_edges_runs_the_boards_order_not_the_documents() {
        let path = seed(
            "follow",
            "::code id=second lang=bash run hidden\necho ran-second\n::end\n\n\
             ::code id=first lang=bash run hidden\necho ran-first\n::end\n\n\
             ::board id=plan\n- first x=0 y=0 to=second\n- second x=0 y=200\n::end\n",
        );
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--follow-edges", "--approve"]),
            cache("follow"),
        )
        .expect("run");
        let first_line = report
            .lines()
            .position(|line| line.starts_with("first "))
            .expect("first ran");
        let second_line = report
            .lines()
            .position(|line| line.starts_with("second "))
            .expect("second ran");
        assert!(first_line < second_line, "{report}");
    }

    #[test]
    fn follow_edges_refuses_a_cycle_by_name() {
        let path = seed(
            "follow-cycle",
            "::code id=a lang=bash run hidden\necho a\n::end\n\n\
             ::code id=b lang=bash run hidden\necho b\n::end\n\n\
             ::board id=plan\n- a x=0 y=0 to=b\n- b x=0 y=200 to=a\n::end\n",
        );
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--follow-edges"]),
            cache("follow-cycle"),
        )
        .expect_err("a cycle");
        assert_eq!(
            report,
            "blocks a -> b -> a form a cycle; --follow-edges needs an order"
        );
    }

    #[test]
    fn an_unapproved_document_is_refused_with_the_review_sentence() {
        // The ledger is this test's own and was wiped by `seed`, so nothing is approved by
        // construction rather than by hoping no sibling test approved the same line.
        let path = seed(
            "gate",
            "::code id=fresh lang=bash run\necho review-gate-refusal\n::end\n",
        );
        let report =
            run_in(&args(&[&path.to_string_lossy()]), cache("gate")).expect_err("should refuse");
        assert!(report.contains("blocked"), "{report}");
        assert!(report.contains("`review`"), "{report}");
        // Nothing executed and nothing was written into the document.
        assert!(!std::fs::read_to_string(&path)
            .expect("read")
            .contains("::output"));
    }

    #[test]
    fn review_prints_the_code_and_fingerprint_without_executing() {
        let source = "::code id=peek lang=bash run\necho review-gate-inspection\n::end\n";
        let path = seed("review", source);
        let report = run_in(
            &args(&[&path.to_string_lossy(), "--review"]),
            cache("review"),
        )
        .expect("review");
        assert!(report.contains("block peek (bash)"), "{report}");
        assert!(report.contains("fingerprint "), "{report}");
        assert!(report.contains("echo review-gate-inspection"), "{report}");
        assert!(report.contains("nothing ran, nothing saved"), "{report}");
        // Reading never executes, and never writes: the file is untouched.
        assert_eq!(std::fs::read_to_string(&path).expect("read"), source);
    }

    #[test]
    fn review_with_approve_is_refused_not_swallowed() {
        let path = seed(
            "review-approve",
            "::code id=x lang=bash run\necho review-approve-conflict\n::end\n",
        );
        let name = path.to_string_lossy().into_owned();
        let sentence = run_in(
            &args(&[&name, "--review", "--approve"]),
            cache("review-approve"),
        )
        .expect_err("conflicting flags");
        assert!(sentence.contains("approve separately"), "{sentence}");
        // The refusal approved nothing: a plain run still hits the gate.
        let report =
            run_in(&args(&[&name]), cache("review-approve")).expect_err("still unapproved");
        assert!(report.contains("blocked pending review"), "{report}");
    }

    #[test]
    fn a_document_that_ships_its_own_run_record_is_still_refused() {
        // What a cloned repository looks like — and what a hostile document is trivially
        // made to look like, the hash being a function of the code it sits under. The gate
        // stands ahead of the cache, so this is reviewed rather than reported as cached.
        let code = "::code id=hi lang=bash run\necho shipped\n::end\n";
        let path = seed("shipped-record", code);
        let name = path.to_string_lossy().into_owned();

        // The fingerprint is no secret — `--review` prints it, and anyone can compute it
        // from the code — so the record below is exactly what a forger would ship.
        let review = run_in(&args(&[&name, "--review"]), cache("shipped-record")).expect("review");
        let hash = review
            .split("fingerprint ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("review names the fingerprint")
            .to_string();
        workspace::write_text(
            &path,
            &format!(
                "{code}\n::output id=hi-output for=hi status=ok exit=0 hash={hash}\nshipped\n::end\n"
            ),
        )
        .expect("forge");

        let report = run_in(&args(&[&name]), cache("shipped-record")).expect_err("should refuse");
        assert!(report.contains("blocked pending review"), "{report}");
        assert!(!report.contains("cached"), "{report}");
    }
}
