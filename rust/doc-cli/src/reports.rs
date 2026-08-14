//! The report ledger: what using dx turned up, on its way to being fixed.
//!
//! An agent working in any project can hit a defect in *dx itself* — a tool that misled it,
//! a message that did not say what to do next, a call it had to make twice. That report has
//! to reach the dx checkout, which is almost never the project the agent is working in, so
//! it cannot be written into the workspace at hand: this machine keeps one **inbox** outside
//! every repository (exactly as [`doc_run`]'s approval ledger does, and for the same reason),
//! and `dx report drain` folds what is waiting into a `reports.dx` document in the checkout,
//! where it is committed with the rest of the repository's documents.
//!
//! # Why a document, and not a table
//! `.doc/index.db` is derived data, dropped and rebuilt whenever the schema moves, so a
//! report stored there would be a report that vanishes on the next upgrade. The document
//! *is* the database: it is committed content, `dx search` already indexes and answers from
//! it, `dx render` draws it, and fixing a report deletes its block — the fix's record being
//! its commit message, exactly as the worklist works.
//!
//! # One block per defect, however many times it is reported
//! A report's block id is `report-<fingerprint>`, the fingerprint covering its kind, its
//! title, and the route it names — not its prose, which two sessions will never word the
//! same way. The same defect filed by three sessions is therefore one block with three
//! `seen` lines rather than three blocks, which is what makes *how often does this actually
//! happen* answerable at all.
//!
//! # One record per file
//! Agents file concurrently from unrelated processes, so the inbox is a directory of
//! one-record files rather than a shared log: two writers cannot interleave a line, and a
//! drain that fails partway leaves every record it has not yet folded in. A record is
//! deleted only after the document that now carries it has been saved.

use std::fs;
use std::path::{Path, PathBuf};

use doc_core::digest::sha256_hex;
use doc_core::edit;
use doc_core::format::parse;
use serde_json::{json, Value};

use crate::workspace;

/// Environment override for the inbox directory.
///
/// The suite points this at a temporary directory: a test run must never file into — or
/// drain — the developer's real inbox.
const INBOX_ENV: &str = "DX_REPORTS_DIR";

/// Name of the document a drain folds reports into, inside the workspace it is given.
pub const DOCUMENT: &str = "reports.dx";

/// Prefix every report block id carries, and how a report is recognised in the document.
const ID_PREFIX: &str = "report-";

/// Hex digits of the fingerprint kept in a block id. Eight is short enough to read aloud
/// and long enough that two live reports colliding is not a thing that happens.
const FINGERPRINT_LENGTH: usize = 8;

/// The document written on the first drain, when the workspace has no `reports.dx` yet.
pub const SCAFFOLD: &str = "\
::heading level=1 id=reports
Reports — what using dx turned up
::end

::paragraph id=reports-note
Filed with `dx_report` from wherever an agent was working, folded in by `dx report drain`, \
and deleted when fixed — `dx remove reports.dx <id>` closes one, and the fix's own record is \
its commit message. One block per defect, keyed `report-<fingerprint>` over its kind, title, \
and route, so the same thing reported twice becomes a second `seen` line rather than a second \
block, and the sighting count is what earns a fix. `dx report list` shows these beside \
whatever is still waiting in the machine's inbox.
::end
";

/// What kind of thing is being reported.
///
/// Three, deliberately: a defect to fix, an idea to weigh, and a plain observation of how
/// dx behaved. The third exists because the most useful field reports are often not yet a
/// claim about what is wrong — "I expected the search hit to be the block that states the
/// fact" is worth more than a guess at the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something dx did wrong.
    Bug,
    /// Something dx could do better.
    Suggestion,
    /// How dx behaved, without a claim about whether that is wrong.
    Observation,
}

impl Kind {
    /// The kind named by `word`, case-insensitively.
    ///
    /// # Errors
    /// Returns a sentence naming the three words that work, because the caller is usually
    /// an agent that guessed a synonym and can correct itself from the message.
    pub fn parse(word: &str) -> Result<Self, String> {
        match word.trim().to_ascii_lowercase().as_str() {
            "bug" => Ok(Self::Bug),
            "suggestion" => Ok(Self::Suggestion),
            "observation" => Ok(Self::Observation),
            other => Err(format!(
                "`{other}` is not a report kind — it is bug, suggestion, or observation"
            )),
        }
    }

    /// The word this kind is written as, everywhere.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Suggestion => "suggestion",
            Self::Observation => "observation",
        }
    }
}

/// One report, as filed.
///
/// The three context fields — when, which dx, which machine, which workspace — are recorded
/// rather than asked for: a reporter who has to remember them files fewer reports, and a
/// version-less report cannot be told from one about a build fixed months ago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// What kind of thing this is.
    pub kind: Kind,
    /// One line naming the defect. This, the kind, and the route are the identity.
    pub title: String,
    /// What happened, what was expected, and anything else the fixer needs.
    pub detail: String,
    /// The tool, command, or surface involved — `dx_search`, `dx run`, the editor. Empty
    /// when the reporter did not name one.
    pub route: String,
    /// The smallest sequence that shows it, when the reporter has one.
    pub repro: String,
    /// When it was filed, ISO-8601 UTC.
    pub at: String,
    /// The dx version that produced the behaviour.
    pub dx: String,
    /// The operating system it happened on.
    pub platform: String,
    /// The workspace the reporter was working in when it happened.
    pub workspace: String,
}

impl Report {
    /// A report of `kind` about `title`, stamped with this moment, this build, and
    /// `workspace`.
    ///
    /// # Errors
    /// Returns a sentence when the title or the detail is empty: a report nobody can act on
    /// is worse than no report, because it still costs a fixer the read.
    pub fn now(
        kind: Kind,
        title: &str,
        detail: &str,
        route: &str,
        repro: &str,
        workspace: &Path,
    ) -> Result<Self, String> {
        let title = collapse(title);
        if title.is_empty() {
            return Err("a report needs a title: one line naming what went wrong".to_string());
        }
        if detail.trim().is_empty() {
            return Err(
                "a report needs detail: what you did, what you expected, what happened instead"
                    .to_string(),
            );
        }
        Ok(Self {
            kind,
            title,
            detail: detail.trim().to_string(),
            route: collapse(route),
            repro: repro.trim().to_string(),
            at: doc_store::timestamp(),
            dx: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            workspace: workspace.display().to_string(),
        })
    }

    /// The block id this report lands under: `report-` and the fingerprint of its identity.
    ///
    /// The identity is the kind, the title, and the route — never the detail, which two
    /// sessions describing one defect will always word differently.
    #[must_use]
    pub fn id(&self) -> String {
        let identity = format!(
            "{}\n{}\n{}",
            self.kind.as_str(),
            self.title.to_lowercase(),
            self.route.to_lowercase()
        );
        let digest = sha256_hex(identity.as_bytes());
        format!("{ID_PREFIX}{}", &digest[..FINGERPRINT_LENGTH])
    }

    /// The block body a fresh report becomes: the headline, the detail, what was named, and
    /// the first `seen` line.
    #[must_use]
    fn body(&self) -> String {
        let mut lines = vec![
            format!("**{} — {}**", self.kind.as_str(), self.title),
            self.detail.clone(),
        ];
        if !self.route.is_empty() {
            lines.push(format!("Route: {}", self.route));
        }
        if !self.repro.is_empty() {
            lines.push(format!("Repro: {}", self.repro));
        }
        lines.push(self.seen());
        lines.join("\n")
    }

    /// The one line a repeat sighting adds to a report already in the document.
    #[must_use]
    fn seen(&self) -> String {
        format!(
            "seen {} · dx {} · {} · {}",
            self.at, self.dx, self.platform, self.workspace
        )
    }

    /// This report as the JSON one inbox file holds.
    #[must_use]
    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.as_str(),
            "title": self.title,
            "detail": self.detail,
            "route": self.route,
            "repro": self.repro,
            "at": self.at,
            "dx": self.dx,
            "platform": self.platform,
            "workspace": self.workspace,
        })
    }

    /// The report a stored record stands for.
    ///
    /// # Errors
    /// Returns a sentence when the record is not a report — a file dropped into the inbox by
    /// something else, or one written by a dx that spelled a field differently. The drain
    /// leaves such a record in place and names it rather than deleting what it cannot read.
    fn from_json(value: &Value) -> Result<Self, String> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let kind = Kind::parse(&text("kind"))?;
        let title = text("title");
        if title.is_empty() {
            return Err("the record has no title".to_string());
        }
        Ok(Self {
            kind,
            title,
            detail: text("detail"),
            route: text("route"),
            repro: text("repro"),
            at: text("at"),
            dx: text("dx"),
            platform: text("platform"),
            workspace: text("workspace"),
        })
    }
}

/// The inbox directory this machine files into.
///
/// Durable per-user data, not a cache: a cleared cache must never be a lost report. Never
/// inside a repository — the workspace an agent is in is almost never the dx checkout the
/// report is about.
#[must_use]
pub fn inbox() -> PathBuf {
    if let Some(explicit) = std::env::var_os(INBOX_ENV) {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    crate::home::data_dir().join("dx").join("reports")
}

/// Write `report` into the inbox at `dir`, returning its block id **and the record file**.
///
/// The path is what [`crate::intake`] needs: a report the intake has accepted is removed from
/// this machine's inbox, and removing it means naming the file that holds it.
///
/// # Errors
/// Returns a sentence naming what could not be written. A report that cannot be filed is never
/// swallowed: the reporter is told, so it can say so in its own answer.
pub fn file_record(report: &Report, dir: &Path) -> Result<(String, PathBuf), String> {
    fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let id = report.id();
    let stem = format!("{}-{id}", report.at.replace(':', ""));

    // Two reports of the same defect in the same second are still two sightings.
    let mut record = dir.join(format!("{stem}.json"));
    let mut nth = 1;
    while record.exists() {
        record = dir.join(format!("{stem}-{nth}.json"));
        nth += 1;
    }

    let body = serde_json::to_string_pretty(&report.to_json())
        .map_err(|error| format!("could not encode the report: {error}"))?;
    fs::write(&record, format!("{body}\n"))
        .map_err(|error| format!("could not file {}: {error}", record.display()))?;
    Ok((id, record))
}

/// One record waiting in the inbox.
#[derive(Debug, Clone)]
pub struct Pending {
    /// The file holding it, deleted once the document carries it.
    pub record: PathBuf,
    /// The report itself.
    pub report: Report,
}

/// What the inbox holds right now.
#[derive(Debug, Clone, Default)]
pub struct Inbox {
    /// Records that can be folded in, oldest first.
    pub pending: Vec<Pending>,
    /// Records that could not be read, each with its reason. They stay on disk.
    pub unreadable: Vec<String>,
}

/// Read every record in the inbox at `dir`, oldest first — records filed within the same
/// second are ordered by fingerprint, since a stamp to the second cannot separate them.
///
/// A missing inbox is an empty one, not an error: nothing has been filed on this machine yet.
///
/// # Errors
/// Returns a sentence when the directory exists but cannot be listed — a permissions
/// problem the caller must show rather than report as "nothing waiting".
pub fn read_inbox(dir: &Path) -> Result<Inbox, String> {
    if !dir.exists() {
        return Ok(Inbox::default());
    }
    let listing =
        fs::read_dir(dir).map_err(|error| format!("could not read {}: {error}", dir.display()))?;

    let mut records: Vec<PathBuf> = listing
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    records.sort();

    let mut inbox = Inbox::default();
    for record in records {
        match read_record(&record) {
            Ok(report) => inbox.pending.push(Pending { record, report }),
            Err(reason) => inbox
                .unreadable
                .push(format!("{}: {reason}", record.display())),
        }
    }
    Ok(inbox)
}

/// The report stored in one record file.
fn read_record(path: &Path) -> Result<Report, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{error}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| format!("{error}"))?;
    Report::from_json(&value)
}

/// What one drain did.
#[derive(Debug, Clone, Default)]
pub struct Drained {
    /// Ids of reports that became new blocks.
    pub filed: Vec<String>,
    /// Ids of reports already in the document, which gained another `seen` line.
    pub repeated: Vec<String>,
    /// Records left in the inbox, each with the reason it could not be folded in.
    pub unreadable: Vec<String>,
}

impl Drained {
    /// How many records this drain folded in, new and repeated together.
    #[must_use]
    pub fn total(&self) -> usize {
        self.filed.len() + self.repeated.len()
    }

    /// The sentence a reader gets: what landed, what was already known, what was left.
    #[must_use]
    pub fn summary(&self, document: &Path) -> String {
        let mut lines = Vec::new();
        if self.total() == 0 {
            lines.push("nothing waiting in the report inbox".to_string());
        } else {
            lines.push(format!(
                "folded {} report(s) into {}",
                self.total(),
                document.display()
            ));
            if !self.filed.is_empty() {
                lines.push(format!("  new: {}", self.filed.join(", ")));
            }
            if !self.repeated.is_empty() {
                lines.push(format!("  seen again: {}", self.repeated.join(", ")));
            }
        }
        for reason in &self.unreadable {
            lines.push(format!("  left in the inbox — {reason}"));
        }
        lines.join("\n")
    }
}

/// Fold every readable record in `dir` into the document at `document`, deleting each
/// record once the document that now carries it is saved.
///
/// A report already in the document gains a `seen` line instead of a second block, so the
/// document answers *how often* as well as *what*. The document is created from the
/// scaffold when it does not exist yet, and only then: an empty inbox writes nothing at all.
///
/// # Errors
/// Returns a sentence when the document cannot be read or saved. The inbox is left as it
/// was in that case — a record is removed only after the save that carries it succeeded.
pub fn drain(dir: &Path, document: &Path) -> Result<Drained, String> {
    let inbox = read_inbox(dir)?;
    let mut drained = Drained {
        unreadable: inbox.unreadable,
        ..Drained::default()
    };
    if inbox.pending.is_empty() {
        return Ok(drained);
    }
    if !document.exists() {
        // Through the store, like any other new document: a reports file written as plain
        // text would be the one document in the workspace git tracks as its own content.
        workspace::save(document, &parse(SCAFFOLD))?;
    }

    for waiting in inbox.pending {
        let id = waiting.report.id();
        let source = workspace::read(document)?;
        let parsed = parse(&source);

        let updated = match edit::find(&parsed, &id) {
            Ok(index) => {
                let grown = format!(
                    "{}\n{}",
                    edit::body(&parsed.blocks[index]),
                    waiting.report.seen()
                );
                drained.repeated.push(id.clone());
                edit::set_block(&source, &id, &grown)?
            }
            Err(_) => {
                let insertion = edit::Insertion {
                    kind: "paragraph",
                    body: &waiting.report.body(),
                    id: &id,
                    level: 0,
                    language: "",
                    run: false,
                    deps: "",
                };
                let last = parsed.blocks.last().map(|block| block.id.clone());
                drained.filed.push(id.clone());
                edit::insert_after(&source, last.as_deref(), &insertion)?.0
            }
        };
        workspace::save_source(document, &updated)?;
        fs::remove_file(&waiting.record).map_err(|error| {
            format!(
                "{id} was folded into {} but its record {} could not be removed: {error} — \
                 remove it by hand, or the next drain files it twice",
                document.display(),
                waiting.record.display()
            )
        })?;
    }
    Ok(drained)
}

/// One report the document is still carrying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    /// The block id, which is also the fingerprint.
    pub id: String,
    /// The report's headline, without its markup.
    pub headline: String,
    /// How many times it has been reported.
    pub sightings: usize,
}

/// Every open report in the document at `path`, in document order.
///
/// A document that does not exist yet has no open reports; that is an answer, not a failure.
///
/// # Errors
/// Returns a sentence when the document exists but cannot be resolved — a pointer with no
/// content behind it is exactly the case that must never read as "no reports".
pub fn open_reports(path: &Path) -> Result<Vec<Open>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let document = parse(&workspace::read(path)?);
    Ok(document
        .blocks
        .iter()
        .filter(|block| block.id.starts_with(ID_PREFIX))
        .map(|block| {
            let body = edit::body(block);
            Open {
                id: block.id.clone(),
                headline: body
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim_matches('*')
                    .to_string(),
                sightings: body
                    .lines()
                    .filter(|line| line.starts_with("seen "))
                    .count(),
            }
        })
        .collect())
}

/// Collapse every run of whitespace in `text` into one space, and trim it.
///
/// A title is one line by construction, so a pasted newline must not change the identity of
/// the defect it names.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-report-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch");
        root
    }

    fn report(title: &str) -> Report {
        Report::now(
            Kind::Bug,
            title,
            "The search answered with the heading rather than the block stating the fact.",
            "dx_search",
            "dx search \"what sampling rate\"",
            Path::new("/tmp/some-project"),
        )
        .expect("report")
    }

    #[test]
    fn a_report_needs_a_title_and_a_detail_worth_reading() {
        let workspace = Path::new("/tmp/p");
        assert!(Report::now(Kind::Bug, "   ", "detail", "", "", workspace)
            .expect_err("empty title")
            .contains("title"));
        assert!(Report::now(Kind::Bug, "title", "\n\n", "", "", workspace)
            .expect_err("empty detail")
            .contains("detail"));
    }

    #[test]
    fn the_kinds_are_the_three_words_and_nothing_else() {
        assert_eq!(Kind::parse("Bug").expect("bug"), Kind::Bug);
        assert_eq!(Kind::parse(" suggestion ").expect("s"), Kind::Suggestion);
        assert_eq!(Kind::parse("observation").expect("o"), Kind::Observation);
        let refused = Kind::parse("feature-request").expect_err("not a kind");
        assert!(
            refused.contains("bug, suggestion, or observation"),
            "{refused}"
        );
    }

    /// The identity is the kind, the title, and the route — so the same defect described in
    /// different words is one block, and a different defect is never folded into it.
    #[test]
    fn the_id_follows_the_defect_and_not_the_prose() {
        let first = report("dx_search returns the heading, not the stating block");
        let mut second = first.clone();
        second.detail = "Completely different words for the same thing.".to_string();
        second.at = "2030-01-01T00:00:00Z".to_string();
        assert_eq!(first.id(), second.id());

        let elsewhere = report("dx run leaves output stale after a reads= edit");
        assert_ne!(first.id(), elsewhere.id());

        // Whitespace and case are not the defect.
        let spaced = report("dx_search   returns the heading, not the stating block");
        assert_eq!(first.id(), spaced.id());
        assert!(first.id().starts_with("report-"));
        assert_eq!(first.id().len(), "report-".len() + FINGERPRINT_LENGTH);
    }

    #[test]
    fn filing_writes_one_record_per_sighting_and_reading_gets_them_back() {
        let dir = scratch("file").join("inbox");
        let one = report("a first defect");
        let two = report("a second defect");
        assert_eq!(file_record(&one, &dir).expect("file").0, one.id());
        file_record(&two, &dir).expect("file");
        // The same defect, the same second: still two sightings, so still two records.
        file_record(&one, &dir).expect("file again");

        let inbox = read_inbox(&dir).expect("read");
        assert_eq!(inbox.pending.len(), 3);
        assert!(inbox.unreadable.is_empty());
        let titles: Vec<&str> = inbox
            .pending
            .iter()
            .map(|pending| pending.report.title.as_str())
            .collect();
        assert_eq!(
            titles.iter().filter(|title| **title == one.title).count(),
            2
        );
        assert!(titles.contains(&two.title.as_str()));
    }

    #[test]
    fn an_absent_inbox_is_empty_rather_than_an_error() {
        let dir = scratch("absent").join("never-filed");
        let inbox = read_inbox(&dir).expect("read");
        assert!(inbox.pending.is_empty());
    }

    /// A record this build cannot read is named and left on disk. Deleting it would lose
    /// exactly the report someone went to the trouble of filing.
    #[test]
    fn an_unreadable_record_is_reported_and_kept() {
        let dir = scratch("unreadable").join("inbox");
        fs::create_dir_all(&dir).expect("dir");
        let stray = dir.join("2030-not-a-report.json");
        fs::write(&stray, "{ this is not json").expect("write");

        let inbox = read_inbox(&dir).expect("read");
        assert!(inbox.pending.is_empty());
        assert_eq!(inbox.unreadable.len(), 1);

        let document = scratch("unreadable-doc").join("reports.dx");
        let drained = drain(&dir, &document).expect("drain");
        assert_eq!(drained.total(), 0);
        assert_eq!(drained.unreadable.len(), 1);
        assert!(
            stray.exists(),
            "an unreadable record must survive the drain"
        );
        assert!(
            !document.exists(),
            "a drain with nothing to fold in writes no document"
        );
    }

    #[test]
    fn draining_folds_each_report_into_the_document_and_empties_the_inbox() {
        let root = scratch("drain");
        let dir = root.join("inbox");
        let document = root.join("reports.dx");
        file_record(&report("the first thing"), &dir).expect("file");
        file_record(&report("the second thing"), &dir).expect("file");

        let drained = drain(&dir, &document).expect("drain");
        assert_eq!(drained.filed.len(), 2);
        assert!(drained.repeated.is_empty());

        let source = workspace::read(&document).expect("read");
        assert!(source.contains("the first thing"), "{source}");
        assert!(source.contains("the second thing"), "{source}");
        assert!(source.contains("Route: dx_search"), "{source}");
        assert!(
            source.contains("Reports — what using dx turned up"),
            "{source}"
        );
        assert!(read_inbox(&dir).expect("read").pending.is_empty());
        // The database is a document like any other: stored content behind a pointer, so
        // git tracks it exactly as it tracks the rest of the repository's documents.
        let on_disk = fs::read_to_string(&document).expect("pointer");
        assert!(on_disk.starts_with("~ dx1 "), "{on_disk}");
    }

    /// The point of the fingerprint: the same defect filed twice is one block with two
    /// sightings, so the document answers "how often" as well as "what".
    #[test]
    fn the_same_defect_reported_twice_is_one_block_with_two_sightings() {
        let root = scratch("repeat");
        let dir = root.join("inbox");
        let document = root.join("reports.dx");

        file_record(&report("one recurring defect"), &dir).expect("file");
        drain(&dir, &document).expect("first drain");

        let mut later = report("one recurring defect");
        later.at = "2030-06-01T09:00:00Z".to_string();
        later.workspace = "/tmp/another-project".to_string();
        file_record(&later, &dir).expect("file again");
        let second = drain(&dir, &document).expect("second drain");

        assert_eq!(second.filed.len(), 0);
        assert_eq!(second.repeated.len(), 1);

        let open = open_reports(&document).expect("open");
        assert_eq!(open.len(), 1, "a repeat must not become a second block");
        assert_eq!(open[0].sightings, 2);
        assert!(open[0].headline.contains("one recurring defect"));
        let source = workspace::read(&document).expect("read");
        assert!(source.contains("/tmp/another-project"), "{source}");
    }

    /// A body line shaped like a close token is content, and the engine's writer is what
    /// makes it survive — which is why a report is never assembled as source text by hand.
    #[test]
    fn a_report_may_say_anything_including_dx_syntax() {
        let root = scratch("syntax");
        let dir = root.join("inbox");
        let document = root.join("reports.dx");
        let mut tricky = report("a block ended early");
        tricky.detail = "Pasting\n::end\ninto a paragraph truncated the document.".to_string();
        file_record(&tricky, &dir).expect("file");
        drain(&dir, &document).expect("drain");

        let open = open_reports(&document).expect("open");
        assert_eq!(open.len(), 1);
        let carried = parse(&workspace::read(&document).expect("read"));
        let index = edit::find(&carried, &tricky.id()).expect("block");
        assert!(
            edit::body(&carried.blocks[index]).contains("\n::end\n"),
            "the close token was not carried as content"
        );
    }

    #[test]
    fn an_absent_document_has_no_open_reports() {
        let document = scratch("no-doc").join("reports.dx");
        assert!(open_reports(&document).expect("open").is_empty());
    }

    #[test]
    fn the_inbox_is_outside_every_repository_and_overridable_for_a_test_run() {
        let _env = crate::env_lock();
        std::env::set_var(INBOX_ENV, "/tmp/dx-report-inbox-override");
        assert_eq!(inbox(), PathBuf::from("/tmp/dx-report-inbox-override"));

        std::env::remove_var(INBOX_ENV);
        let default = inbox();
        assert!(default.ends_with("dx/reports"), "{}", default.display());
        assert!(
            !default.starts_with(std::env::temp_dir()),
            "a report must not live somewhere a cleanup would take it"
        );
    }

    #[test]
    fn a_drain_says_what_it_did() {
        let drained = Drained {
            filed: vec!["report-aaaaaaaa".to_string()],
            repeated: vec!["report-bbbbbbbb".to_string()],
            unreadable: vec!["broken.json: not json".to_string()],
        };
        let summary = drained.summary(Path::new("reports.dx"));
        assert!(summary.contains("folded 2 report(s)"), "{summary}");
        assert!(summary.contains("new: report-aaaaaaaa"), "{summary}");
        assert!(summary.contains("seen again: report-bbbbbbbb"), "{summary}");
        assert!(summary.contains("left in the inbox"), "{summary}");
        assert!(Drained::default()
            .summary(Path::new("reports.dx"))
            .contains("nothing waiting"));
    }
}
