//! `dx report` — file what dx got wrong, and let it reach the checkout that fixes it.
//!
//! ```text
//! dx report bug|suggestion|observation --title T --detail D [--route R] [--repro S]
//! dx report list [dir]        what is waiting here, and what the checkout carries
//! dx report sync [dir]        push what is waiting, pull the project's open reports
//! dx report subscribe [dir] [--project dx] [--endpoint URL] [--token T]
//! dx report unsubscribe [dir]
//! dx report close <id> [dir]  a fix: the block goes, and the database is told
//! dx report drain [dir]       fold this machine's inbox in without a network
//! ```
//!
//! Filing goes two ways at once: into this machine's inbox ([`crate::reports`]) and, unless
//! the endpoint is turned off, straight to the intake ([`crate::intake`]) — so a defect an
//! agent hits while working on some unrelated project still reaches the dx checkout, where
//! the next agent reads it in `reports.dx`. `drain` remains the offline route: it folds the
//! local inbox into the document with no network at all.

use std::path::{Path, PathBuf};

use crate::args::Args;
use crate::commands::Output;
use crate::intake::{self, Subscription};
use crate::reports::{self, Kind, Report};
use crate::workspace;

/// Run `dx report`.
///
/// # Errors
/// Returns a sentence when the kind word is not one of the three, when a filed report has no
/// title or detail, or when the inbox, the subscription, or the document cannot be read or
/// written.
pub fn run(args: &Args) -> Result<Output, String> {
    match args.positional(0).unwrap_or("list") {
        "list" => list(args).map(Output::Document),
        "drain" => drain(args).map(Output::Report),
        "sync" => sync(args).map(Output::Report),
        "subscribe" => subscribe(args).map(Output::Report),
        "unsubscribe" => unsubscribe(args).map(Output::Report),
        "close" => close(args).map(Output::Report),
        kind => file(kind, args).map(Output::Report),
    }
}

/// `dx report <kind> --title T --detail D` — file one report, here and at the intake.
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

    let filed = intake::file(&report)?;
    Ok(filed.summary(kind.as_str(), &reports::inbox()))
}

/// `dx report drain [dir]` — fold this machine's inbox into `<dir>/reports.dx`, no network.
fn drain(args: &Args) -> Result<String, String> {
    let document = document_for(args.positional(1));
    let drained = reports::drain(&reports::inbox(), &document)?;
    Ok(format!("{}\n", drained.summary(&document)))
}

/// `dx report sync [dir]` — push what is waiting, then fold the project's open reports in.
fn sync(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));
    let subscription = subscription_or_hint(&root)?;
    let synced = intake::sync(&subscription)?;
    Ok(format!("{}\n", synced.summary(&subscription.document())))
}

/// `dx report subscribe [dir]` — this checkout receives a project's reports from now on.
fn subscribe(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));
    let stated = args.value("endpoint");
    let subscription = Subscription {
        workspace: root.clone(),
        // The service is the endpoint's query, so `--endpoint …/report?billing` registers
        // `billing` with nothing else said. `--project` still wins when both are given.
        project: args
            .value("project")
            .map(str::to_string)
            .or_else(|| intake::service_from(stated))
            .or_else(intake::service)
            .unwrap_or_else(|| intake::DEFAULT_PROJECT.to_string()),
        endpoint: stated
            .map(|value| intake::split_service(value).0)
            .or_else(intake::endpoint)
            .unwrap_or_else(|| intake::DEFAULT_ENDPOINT.to_string()),
        token: args.value("token").unwrap_or_default().to_string(),
    };
    intake::subscribe(&subscription)?;

    let mut out = format!(
        "{} now receives `{}` reports from {}\n",
        subscription.document().display(),
        subscription.project,
        intake::address(&subscription.endpoint, "", &subscription.project)
    );
    if subscription.token.is_empty() && std::env::var("DX_REPORT_TOKEN").is_err() {
        out.push_str(
            "no token stored, so this checkout can file but not read — run \
             `selfhost reports token` on the box and re-run with --token\n",
        );
        return Ok(out);
    }
    let synced = intake::sync(&subscription)?;
    out.push_str(&format!("{}\n", synced.summary(&subscription.document())));
    Ok(out)
}

/// `dx report unsubscribe [dir]` — stop receiving. The document is left exactly as it is.
fn unsubscribe(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));
    if intake::unsubscribe(&root)? {
        return Ok(format!("{} no longer receives reports\n", root.display()));
    }
    Ok(format!("{} was not subscribed\n", root.display()))
}

/// `dx report close <id> [dir]` — tell the database, then remove the block.
///
/// Both halves, because either alone is wrong: a block removed but not closed comes back on
/// the next sync, and a report closed but not removed leaves the document claiming an open
/// defect nobody will ever see again. The database is told **first**: it is the one call that
/// can fail on grounds the caller could not see locally (a stale block, a race with another
/// close), and a failure there must leave the document exactly as it was rather than losing the
/// record of an open report the server still disagrees about.
fn close(args: &Args) -> Result<String, String> {
    let id = args
        .positional(1)
        .ok_or("`dx report close` needs a report id, e.g. `dx report close report-1a2b3c4d`")?;
    let root = root_for(args.positional(2));
    let subscription = subscription_or_hint(&root)?;
    let document = subscription.document();

    intake::close(
        &subscription.endpoint,
        &subscription.project,
        id,
        &intake::token_for(&subscription),
    )
    .map_err(|error| {
        format!(
            "{error} — run `dx report sync`, which removes a report the intake has already \
             closed elsewhere"
        )
    })?;
    let mut out = format!(
        "closed {id} at {}\n",
        intake::address(&subscription.endpoint, "close", &subscription.project)
    );

    if document.exists() {
        let source = workspace::read(&document)?;
        let parsed = doc_core::format::parse(&source);
        if doc_core::edit::find(&parsed, id).is_ok() {
            let without = doc_core::edit::remove_block(&source, id)?;
            workspace::save_source(&document, &without)?;
            out.push_str(&format!("removed {id} from {}\n", document.display()));
        }
    }
    Ok(out)
}

/// `dx report list [dir]` — what is waiting here, and what the checkout is carrying.
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
            "inbox {} — {} waiting for `dx report sync`\n",
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

    match intake::subscription_for(&workspace::workspace_root(&document_root(args))) {
        Ok(Some(subscription)) => out.push_str(&format!(
            "subscribed to `{}` at {}\n",
            subscription.project,
            intake::address(&subscription.endpoint, "", &subscription.project)
        )),
        Ok(None) => out.push_str(
            "not subscribed — `dx report subscribe --token <t>` keeps this document current\n",
        ),
        Err(reason) => out.push_str(&format!("subscription unreadable — {reason}\n")),
    }
    Ok(out)
}

/// The workspace root a command is about: the directory named, or the current one.
fn root_for(directory: Option<&str>) -> PathBuf {
    let start = directory.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    );
    workspace::workspace_root(&start)
}

/// The directory a listing is about, before it is resolved to a workspace root.
fn document_root(args: &Args) -> PathBuf {
    args.positional(1).map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    )
}

/// The `reports.dx` of the workspace containing `directory`, or of the current directory.
fn document_for(directory: Option<&str>) -> PathBuf {
    root_for(directory).join(reports::DOCUMENT)
}

/// The subscription for `root`, or a sentence naming the command that creates one.
fn subscription_or_hint(root: &Path) -> Result<Subscription, String> {
    intake::subscription_for(root)?.ok_or_else(|| {
        format!(
            "{} is not subscribed to a report project — `dx report subscribe --token <t>` here \
             makes this checkout receive them",
            root.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    /// The suite files into a temporary inbox with the push turned off: a test run must never
    /// touch the developer's real inbox, and must never reach the real intake. Both are
    /// process-wide variables, so the cases that need them share one test.
    #[test]
    fn filing_listing_and_draining_are_one_loop() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-tests");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORTS_DIR", root.join("inbox"));
        std::env::set_var("DX_REPORT_ENDPOINT", "off");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));

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
        let filed_id = filed
            .text()
            .split_whitespace()
            .nth(1)
            .expect("the summary names the id")
            .to_string();

        let waiting = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(waiting.text().contains("1 waiting"), "{}", waiting.text());
        assert!(
            waiting.text().contains("no open reports"),
            "{}",
            waiting.text()
        );
        assert!(
            waiting.text().contains("not subscribed"),
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
        assert!(after.text().contains("1 open"), "{}", after.text());
        assert!(after.text().contains("once"), "{}", after.text());

        // The listing is content a reader may redirect; a drain reports work already done.
        assert!(matches!(after, Output::Document(_)));
        assert!(matches!(drained, Output::Report(_)));

        // Subscribing without a token says so rather than pretending to sync.
        let subscribed = run(&args(&[
            "subscribe",
            root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report",
        ]))
        .expect("subscribe");
        assert!(
            subscribed.text().contains("no token stored"),
            "{}",
            subscribed.text()
        );
        let listed = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(
            listed.text().contains("subscribed to `dx`"),
            "{}",
            listed.text()
        );
        assert!(
            listed.text().contains("https://example.invalid/report?dx"),
            "the address a reader is shown is the one calls go to: {}",
            listed.text()
        );

        // Registering another internal service is the address and nothing else.
        let registered = run(&args(&[
            "subscribe",
            root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report?billing",
        ]))
        .expect("subscribe");
        assert!(
            registered
                .text()
                .contains("`billing` reports from https://example.invalid/report?billing"),
            "{}",
            registered.text()
        );

        // A close the intake refuses must not have already thrown the local record away —
        // that was exactly the bug: the block was removed before the network call answered, so
        // a refusal (a stale id, a race with another close) silently lost an open report the
        // intake still disagreed about. The block must survive a refused close untouched.
        {
            use std::io::{BufRead, BufReader, Read as _, Write as _};
            use std::net::TcpListener;

            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let address = listener.local_addr().expect("address");
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("head");
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).expect("body");
                let answer = "{\"error\":\"`dx` holds no such report\"}";
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                            answer.len()
                        )
                        .as_bytes(),
                    )
                    .expect("answer");
            });

            let refusing = Subscription {
                workspace: workspace::workspace_root(&root),
                project: "dx".to_string(),
                endpoint: format!("http://{address}"),
                token: "t".to_string(),
            };
            intake::subscribe(&refusing).expect("subscribe");
            let before = workspace::read(&refusing.document()).expect("read");

            let error = run(&args(&["close", &filed_id, root.to_str().expect("path")]))
                .expect_err("the intake refused");
            assert!(error.contains("holds no such report"), "{error}");

            server.join().expect("listener");

            let after = workspace::read(&refusing.document()).expect("read");
            assert_eq!(before, after, "a refused close must not touch the document");
            let parsed = doc_core::format::parse(&after);
            assert!(
                doc_core::edit::find(&parsed, &filed_id).is_ok(),
                "the block must still be there after a refused close"
            );
        }

        std::env::remove_var("DX_REPORTS_DIR");
        std::env::remove_var("DX_REPORT_ENDPOINT");
        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
    }

    #[test]
    fn a_kind_nobody_recognises_is_refused_by_name() {
        let error =
            run(&args(&["feature", "--title", "t", "--detail", "d"])).expect_err("not a kind");
        assert!(error.contains("bug, suggestion, or observation"), "{error}");
    }

    #[test]
    fn syncing_a_checkout_nobody_subscribed_names_the_command_that_subscribes_it() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-unsubscribed");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));

        let error =
            run(&args(&["sync", root.to_str().expect("path")])).expect_err("no subscription");
        assert!(error.contains("dx report subscribe"), "{error}");

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
    }
}
