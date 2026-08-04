//! `dx run` — execute a document's code blocks and save the results back into it.
//!
//! This is the only command that runs code, and it never happens by accident: reading,
//! rendering, and screenshotting a document never execute anything.

use std::time::Duration;

use doc_run::{run_document, RunOptions, RunReport, DEFAULT_TIMEOUT_SECONDS};

use crate::args::Args;
use crate::commands::view::document_path;
use crate::workspace;

/// `dx run <file>` — run the document's `run` blocks and write the outputs back.
///
/// `--dry` runs everything but prints the result instead of saving, which is how you
/// check what a document would do before letting it change the file.
pub fn run(args: &Args) -> Result<String, String> {
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
            default_timeout: Duration::from_secs(timeout_seconds),
            force: args.present("force"),
            only: args.value("only").map(str::to_string),
            ..RunOptions::default()
        },
        &workspace::resolver_for(&path),
    );

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
    use std::path::PathBuf;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn seed(label: &str, source: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-exec-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("doc.dx");
        workspace::write_text(&path, source).expect("seed");
        path
    }

    #[test]
    fn running_writes_the_output_into_the_document() {
        let path = seed("basic", "::code id=hello lang=bash run\necho hi\n::end\n");
        let report = run(&args(&[&path.to_string_lossy()])).expect("run");
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
        let report = run(&args(&[&path.to_string_lossy(), "--dry"])).expect("run");
        assert!(report.contains("not saved"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), source);
    }

    #[test]
    fn a_failing_block_is_reported_as_an_error_but_still_recorded() {
        let path = seed("failure", "::code id=bad lang=bash run\nexit 7\n::end\n");
        let report = run(&args(&[&path.to_string_lossy()])).expect_err("should report failure");
        assert!(report.contains("error"));
        assert!(std::fs::read_to_string(&path)
            .expect("read")
            .contains("exit=7"));
    }

    #[test]
    fn a_document_with_nothing_to_run_says_so() {
        let path = seed("nothing", "::paragraph id=p\njust prose\n::end\n");
        let report = run(&args(&[&path.to_string_lossy()])).expect("run");
        assert!(report.contains("no runnable blocks"));
    }

    #[test]
    fn the_second_run_of_an_unchanged_document_is_cached() {
        let path = seed("cache", "::code id=c lang=bash run\necho stable\n::end\n");
        run(&args(&[&path.to_string_lossy()])).expect("first");
        let second = run(&args(&[&path.to_string_lossy()])).expect("second");
        assert!(second.contains("skipped"));
        assert!(second.contains("cached"));
    }

    #[test]
    fn only_limits_execution_to_one_block() {
        let path = seed(
            "only",
            "::code id=a lang=bash run\necho a\n::end\n\n::code id=b lang=bash run\necho b\n::end\n",
        );
        let report = run(&args(&[&path.to_string_lossy(), "--only", "b"])).expect("run");
        assert!(report.contains("b  "));
        assert!(!report.lines().any(|line| line.starts_with("a ")));
    }
}
