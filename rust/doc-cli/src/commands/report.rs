//! `dx report` — file what dx got wrong, see what is waiting, fold it into the repository.
//!
//! Three moves, one verb. `dx report bug|suggestion|observation` files into this machine's
//! inbox from whatever project you are standing in; `dx report list` shows what is waiting
//! there and what the checkout's `reports.dx` is already carrying; `dx report drain` folds
//! the inbox into that document, where the fixer works and git keeps it.
//!
//! [`crate::reports`] is the authority on why the inbox sits outside every repository and
//! why the document — not a table — is the database.

use std::path::PathBuf;

use crate::args::Args;
use crate::commands::Output;
use crate::reports::{self, Kind, Report};
use crate::workspace;

/// Run `dx report`.
///
/// With no word at all, or `list`, it answers with the triage view — content, so `--out`
/// may redirect it. `drain` and a filed report are reports about work already done.
///
/// # Errors
/// Returns a sentence when the kind word is not one of the three, when a filed report has
/// no title or detail, or when the inbox or the document cannot be read or written.
pub fn run(args: &Args) -> Result<Output, String> {
    match args.positional(0).unwrap_or("list") {
        "list" => list(args).map(Output::Document),
        "drain" => drain(args).map(Output::Report),
        kind => file(kind, args).map(Output::Report),
    }
}

/// `dx report <kind> --title T --detail D` — file one report into this machine's inbox.
fn file(kind: &str, args: &Args) -> Result<String, String> {
    let kind = Kind::parse(kind)?;
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let report = Report::now(
        kind,
        args.value("title").unwrap_or_default(),
        args.value("detail").unwrap_or_default(),
        args.value("route").unwrap_or_default(),
        args.value("repro").unwrap_or_default(),
        &workspace::workspace_root(&here),
    )?;

    let inbox = reports::inbox();
    let id = reports::file(&report, &inbox)?;
    let waiting = reports::read_inbox(&inbox)?.pending.len();
    Ok(format!(
        "filed {id} ({}) — {waiting} waiting in {}\nrun `dx report drain` in the dx checkout \
         to fold them into {}\n",
        kind.as_str(),
        inbox.display(),
        reports::DOCUMENT
    ))
}

/// `dx report drain [dir]` — fold the inbox into `<dir>/reports.dx`.
fn drain(args: &Args) -> Result<String, String> {
    let document = document_for(args.positional(1));
    let drained = reports::drain(&reports::inbox(), &document)?;
    Ok(format!("{}\n", drained.summary(&document)))
}

/// `dx report list` — what is waiting, and what the document is still carrying.
fn list(args: &Args) -> Result<String, String> {
    let inbox = reports::inbox();
    let waiting = reports::read_inbox(&inbox)?;
    let document = document_for(args.positional(1));
    let open = reports::open_reports(&document)?;

    let mut out = String::new();
    if waiting.pending.is_empty() {
        out.push_str(&format!("inbox {} — empty\n", inbox.display()));
    } else {
        out.push_str(&format!(
            "inbox {} — {} waiting for `dx report drain`\n",
            inbox.display(),
            waiting.pending.len()
        ));
        for pending in &waiting.pending {
            out.push_str(&format!(
                "  {} {} — {}\n",
                pending.report.at,
                pending.report.kind.as_str(),
                pending.report.title
            ));
        }
    }
    for reason in &waiting.unreadable {
        out.push_str(&format!("  unreadable — {reason}\n"));
    }

    if open.is_empty() {
        out.push_str(&format!("{} — no open reports\n", document.display()));
    } else {
        out.push_str(&format!("{} — {} open\n", document.display(), open.len()));
        for report in &open {
            let times = if report.sightings == 1 {
                "once".to_string()
            } else {
                format!("{} times", report.sightings)
            };
            out.push_str(&format!("  {} {} — {times}\n", report.id, report.headline));
        }
    }
    Ok(out)
}

/// The `reports.dx` of the workspace containing `directory`, or of the current directory.
fn document_for(directory: Option<&str>) -> PathBuf {
    let start = directory.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    );
    workspace::workspace_root(&start).join(reports::DOCUMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    /// The suite files into a temporary inbox: a test run must never touch the developer's
    /// real one, and both cases share one process-wide variable, so they share one test.
    #[test]
    fn filing_listing_and_draining_are_one_loop() {
        let root = std::env::temp_dir().join("dx-report-cli-tests");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORTS_DIR", root.join("inbox"));

        let filed = run(&args(&[
            "bug",
            "--title",
            "dx report has no test",
            "--detail",
            "It did not, until this one.",
            "--route",
            "dx report",
        ]))
        .expect("file");
        assert!(
            filed.text().starts_with("filed report-"),
            "{}",
            filed.text()
        );

        let waiting = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(waiting.text().contains("1 waiting"), "{}", waiting.text());
        assert!(
            waiting.text().contains("no open reports"),
            "{}",
            waiting.text()
        );

        let drained = run(&args(&["drain", root.to_str().expect("path")])).expect("drain");
        assert!(
            drained.text().contains("folded 1 report(s)"),
            "{}",
            drained.text()
        );

        let after = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(after.text().contains("inbox"), "{}", after.text());
        assert!(after.text().contains("1 open"), "{}", after.text());
        assert!(after.text().contains("once"), "{}", after.text());

        // The listing is content a reader may redirect; a drain reports work already done.
        assert!(matches!(after, Output::Document(_)));
        assert!(matches!(drained, Output::Report(_)));

        std::env::remove_var("DX_REPORTS_DIR");
    }

    #[test]
    fn a_kind_nobody_recognises_is_refused_by_name() {
        let error =
            run(&args(&["feature", "--title", "t", "--detail", "d"])).expect_err("not a kind");
        assert!(error.contains("bug, suggestion, or observation"), "{error}");
    }
}
