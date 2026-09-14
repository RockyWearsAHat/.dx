//! Work outside dx, counted per project, at the moment it happens.
//!
//! # Why
//! A field session ran an entire task outside dx — dozens of raw shell calls, a handful of dx
//! calls — and was confident the whole time that each step was right. Nothing stopped it,
//! because the only thing dx measured was search coverage, and that looked fine. The
//! instruction to work through documents lived upstream of every choice; the choice itself
//! had no checkpoint. And nothing could say afterwards *which* sessions in *which* project
//! went off method, so there was no way to tune the method against real behavior rather
//! than against a guess.
//!
//! This module is that checkpoint and that record. A Claude Code `PostToolUse` hook hands
//! every tool use to [`record`]; each is counted as *raw* (`Bash`) or *dx* (`mcp__dx__*`)
//! against the project the call was made in, and at every tenth raw call the agent reads one
//! sentence naming its own ratio — at the call, not in an orientation document read an hour
//! earlier. [`report`] then answers, per project, which sessions worked through the
//! documents and which did not.
//!
//! # Where the ledger lives
//! With the project, like [`crate::coverage`]: `<workspace>/.doc/drift.jsonl`, git-ignored,
//! one line per tool use carrying the session. The hook never creates `.doc` — a project dx
//! has not touched stays untouched — so a call made in a project without dx documents is
//! written to one machine-local file, `<data_dir>/drift/elsewhere.jsonl`, with the
//! project's path on the line. That file is how a project that *should* have documents shows
//! up at all. Both logs prune themselves past [`MAX_ENTRIES`] lines.
//!
//! # The contract
//! - Every failure is swallowed and the hook's exit is always 0. A hook that fails blocks the
//!   tool it wraps, and this one exists to inform, never to block.
//! - Only the raw call that reaches a multiple of [`NUDGE_EVERY`] speaks. A dx call never
//!   does, or a session sitting at ten raw calls would be nudged on every dx call after.
//! - The nudge names the project, and — when the project has no documents — the one command
//!   that gives it some, because "use dx" in a project with nothing to use is not advice.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::{home, workspace};

/// A nudge is spoken at every this-many raw calls: the tenth, the twentieth, and so on.
pub const NUDGE_EVERY: usize = 10;

/// The share of raw calls, in percent, past which a session with at least [`NUDGE_EVERY`]
/// raw calls is reported as off method.
pub const OFF_METHOD_PERCENT: usize = 70;

/// Name of the per-project ledger, inside the workspace's `.doc` directory.
const LOG_NAME: &str = "drift.jsonl";

/// The machine-local ledger for calls made where no `.doc` exists.
const ELSEWHERE: &str = "elsewhere.jsonl";

/// Once a ledger passes this many lines it is rewritten down to [`PRUNE_KEEP`].
const MAX_ENTRIES: usize = 4000;

/// How many of the most recent lines survive a prune.
const PRUNE_KEEP: usize = 2000;

/// Record one hook event and print the nudge, if one is due. The hook entry `dx drift`
/// runs with JSON on stdin; it never fails.
pub fn record(input: &str) {
    if let Some(nudge) = record_with(&elsewhere_ledger(), input) {
        println!("{nudge}");
    }
}

/// [`record`] with an explicit machine-local fallback ledger; returns the nudge when due.
///
/// The project ledger is found from the event's `cwd`. Separated so a test can run against
/// scratch directories without touching the real fallback.
pub fn record_with(elsewhere: &Path, input: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(input).ok()?;
    let session = parsed.get("session_id")?.as_str()?;
    if session.is_empty() || session.len() > 200 || session.contains(['\n', '"']) {
        return None;
    }
    let tool = parsed.get("tool_name")?.as_str()?;
    let raw = classify(tool)?;
    let cwd = parsed.get("cwd").and_then(Value::as_str).unwrap_or(".");
    let root = workspace::workspace_root(Path::new(cwd));
    let indexed = root.join(doc_store::STORE_DIR).is_dir();

    let ledger = if indexed {
        root.join(doc_store::STORE_DIR).join(LOG_NAME)
    } else {
        elsewhere.to_path_buf()
    };
    let entry = json!({
        "session": session,
        "root": root.to_string_lossy(),
        "tool": tool,
        "raw": raw,
        "at": now(),
    });
    append(&ledger, &entry);

    let counts = read(&ledger)
        .into_iter()
        .filter(|line| line.session == session && line.root == root)
        .fold(
            (0, 0),
            |(r, d), line| if line.raw { (r + 1, d) } else { (r, d + 1) },
        );
    if raw && counts.0 > 0 && counts.0 % NUDGE_EVERY == 0 {
        return Some(
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": nudge_text(&root, indexed, counts.0, counts.1),
                }
            })
            .to_string(),
        );
    }
    None
}

/// The one sentence the agent reads at the checkpoint.
fn nudge_text(root: &Path, indexed: bool, raw: usize, dx: usize) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let mut text = format!(
        "dx drift in {name}: {raw} raw commands vs {dx} dx calls this session — is the loop \
         still the harness? Work that cannot be a dx gate is stated in the document as such; \
         work that can be is one."
    );
    if !indexed {
        text.push_str(
            " This project has no dx documents yet: dx_index scaffolds index.dx and dev.dx, \
             and the work continues from there.",
        );
    }
    text
}

/// `Some(true)` for a raw shell call, `Some(false)` for a dx call, `None` for any other tool.
fn classify(tool: &str) -> Option<bool> {
    if tool == "Bash" {
        Some(true)
    } else if tool.starts_with("mcp__dx__") {
        Some(false)
    } else {
        None
    }
}

/// One session's standing in one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The session id as the hook received it.
    pub id: String,
    /// Raw shell calls recorded.
    pub raw: usize,
    /// dx tool calls recorded.
    pub dx: usize,
    /// Unix seconds of the last recorded call.
    pub last_at: u64,
}

impl Session {
    /// Raw calls as a share of all calls, in percent.
    pub fn raw_percent(&self) -> usize {
        match self.raw + self.dx {
            0 => 0,
            total => self.raw * 100 / total,
        }
    }

    /// Whether this session has left the method: enough raw calls to mean something, and
    /// most of its work done outside dx.
    pub fn off_method(&self) -> bool {
        self.raw >= NUDGE_EVERY && self.raw_percent() > OFF_METHOD_PERCENT
    }
}

/// Every session recorded for the project at `directory`, most recently active first.
///
/// A project with a `.doc` reads its own ledger; one without reads its lines out of the
/// machine-local fallback, so an unindexed project still answers.
pub fn report(directory: &Path) -> Vec<Session> {
    report_with(&elsewhere_ledger(), directory)
}

/// [`report`] with an explicit machine-local fallback ledger.
pub fn report_with(elsewhere: &Path, directory: &Path) -> Vec<Session> {
    let root = workspace::workspace_root(directory);
    let ledger = root.join(doc_store::STORE_DIR).join(LOG_NAME);
    let lines = if ledger.is_file() {
        read(&ledger)
    } else {
        read(elsewhere)
    };
    let mut sessions: Vec<Session> = Vec::new();
    for line in lines.into_iter().filter(|line| line.root == root) {
        let session = match sessions.iter_mut().find(|s| s.id == line.session) {
            Some(session) => session,
            None => {
                sessions.push(Session {
                    id: line.session.clone(),
                    raw: 0,
                    dx: 0,
                    last_at: 0,
                });
                sessions.last_mut().expect("just pushed")
            }
        };
        if line.raw {
            session.raw += 1;
        } else {
            session.dx += 1;
        }
        session.last_at = session.last_at.max(line.at);
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_at));
    sessions
}

/// The text `dx drift [dir]` prints: one line per session, off-method ones marked.
pub fn summary(directory: &Path) -> String {
    summary_with(&elsewhere_ledger(), directory)
}

/// [`summary`] with an explicit machine-local fallback ledger.
pub fn summary_with(elsewhere: &Path, directory: &Path) -> String {
    let root = workspace::workspace_root(directory);
    let sessions = report_with(elsewhere, &root);
    if sessions.is_empty() {
        return format!("drift: no sessions recorded in {}\n", root.display());
    }
    let off = sessions.iter().filter(|s| s.off_method()).count();
    let mut out = format!(
        "drift in {}: {} session(s), {off} off method\n",
        root.display(),
        sessions.len()
    );
    for session in &sessions {
        let mark = if session.off_method() {
            "  OFF METHOD"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {:<12} {:>4} raw {:>4} dx  ({:>3}% raw){mark}\n",
            short(&session.id),
            session.raw,
            session.dx,
            session.raw_percent()
        ));
    }
    out
}

/// A session id cut to what a person can tell apart.
fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

/// One recorded call.
struct Line {
    session: String,
    root: PathBuf,
    raw: bool,
    at: u64,
}

/// Append one entry and prune the ledger if it has grown past its bound. Best-effort.
fn append(ledger: &Path, entry: &Value) {
    if let Some(parent) = ledger.parent() {
        if !parent.is_dir() && fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger)
    else {
        return;
    };
    let _ = writeln!(file, "{entry}");
    drop(file);
    prune_if_needed(ledger);
}

/// Rewrite the ledger down to [`PRUNE_KEEP`] lines once it passes [`MAX_ENTRIES`].
fn prune_if_needed(ledger: &Path) {
    let Ok(contents) = fs::read_to_string(ledger) else {
        return;
    };
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= MAX_ENTRIES {
        return;
    }
    let kept = lines[lines.len() - PRUNE_KEEP..].join("\n");
    let _ = fs::write(ledger, kept + "\n");
}

/// Every parseable line in a ledger, oldest first.
fn read(ledger: &Path) -> Vec<Line> {
    let Ok(contents) = fs::read_to_string(ledger) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|text| {
            let value: Value = serde_json::from_str(text).ok()?;
            Some(Line {
                session: value.get("session")?.as_str()?.to_string(),
                root: PathBuf::from(value.get("root")?.as_str()?),
                raw: value.get("raw")?.as_bool()?,
                at: value.get("at").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

/// Unix seconds now, or 0 when the clock is before the epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The machine-local ledger for calls made outside any dx workspace.
fn elsewhere_ledger() -> PathBuf {
    home::data_dir().join("drift").join(ELSEWHERE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-drift-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch");
        fs::canonicalize(&root).expect("canonical")
    }

    fn indexed(label: &str) -> PathBuf {
        let root = scratch(label);
        fs::create_dir_all(root.join(doc_store::STORE_DIR)).expect("store dir");
        root
    }

    fn event(session: &str, tool: &str, cwd: &Path) -> String {
        json!({
            "session_id": session,
            "hook_event_name": "PostToolUse",
            "tool_name": tool,
            "tool_input": {},
            "cwd": cwd.to_string_lossy(),
        })
        .to_string()
    }

    #[test]
    fn a_call_in_an_indexed_project_lands_in_that_project_s_ledger() {
        let project = indexed("indexed");
        let elsewhere = scratch("indexed-elsewhere").join("elsewhere.jsonl");
        record_with(&elsewhere, &event("s", "Bash", &project));
        record_with(&elsewhere, &event("s", "mcp__dx__dx_search", &project));
        record_with(&elsewhere, &event("s", "Read", &project));
        assert!(project.join(".doc").join(LOG_NAME).is_file());
        assert!(
            !elsewhere.exists(),
            "an indexed project never uses the fallback"
        );
        let sessions = report_with(&elsewhere, &project);
        assert_eq!((sessions[0].raw, sessions[0].dx), (1, 1));
    }

    #[test]
    fn a_call_outside_any_workspace_never_creates_doc_and_is_still_reported() {
        let project = scratch("bare");
        let elsewhere = scratch("bare-elsewhere").join("elsewhere.jsonl");
        record_with(&elsewhere, &event("s", "Bash", &project));
        assert!(
            !project.join(".doc").exists(),
            ".doc is never created by a hook"
        );
        assert!(elsewhere.is_file());
        let sessions = report_with(&elsewhere, &project);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].raw, 1);
    }

    #[test]
    fn the_nudge_speaks_at_every_tenth_raw_call_and_never_on_a_dx_call() {
        let project = indexed("threshold");
        let elsewhere = scratch("threshold-elsewhere").join("elsewhere.jsonl");
        let mut spoken = Vec::new();
        for n in 1..=21 {
            if record_with(&elsewhere, &event("s", "Bash", &project)).is_some() {
                spoken.push(n);
            }
            assert!(record_with(&elsewhere, &event("s", "mcp__dx__dx_source", &project)).is_none());
        }
        assert_eq!(spoken, vec![10, 20]);
    }

    #[test]
    fn the_nudge_names_the_project_and_offers_dx_index_only_where_there_are_no_documents() {
        let bare = scratch("nudge-bare");
        let elsewhere = scratch("nudge-elsewhere").join("elsewhere.jsonl");
        let mut last = None;
        for _ in 0..NUDGE_EVERY {
            last = record_with(&elsewhere, &event("s", "Bash", &bare));
        }
        let nudge: Value = serde_json::from_str(&last.expect("tenth call nudges")).expect("json");
        let context = nudge["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("context");
        assert_eq!(nudge["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert!(
            context.contains("dx drift in dx-drift-tests-nudge-bare"),
            "{context}"
        );
        assert!(
            context.contains("10 raw commands vs 0 dx calls"),
            "{context}"
        );
        assert!(context.contains("dx_index"), "{context}");

        let project = indexed("nudge-indexed");
        let mut last = None;
        for _ in 0..NUDGE_EVERY {
            last = record_with(&elsewhere, &event("s", "Bash", &project));
        }
        let nudge: Value = serde_json::from_str(&last.expect("nudge")).expect("json");
        let context = nudge["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("context");
        assert!(!context.contains("dx_index"), "{context}");
    }

    #[test]
    fn sessions_are_kept_apart_per_project_and_marked_off_method() {
        let a = indexed("per-project-a");
        let b = indexed("per-project-b");
        let elsewhere = scratch("per-project-elsewhere").join("elsewhere.jsonl");
        for _ in 0..12 {
            record_with(&elsewhere, &event("wild", "Bash", &a));
        }
        record_with(&elsewhere, &event("wild", "mcp__dx__dx_list", &a));
        for _ in 0..3 {
            record_with(&elsewhere, &event("tame", "Bash", &a));
            record_with(&elsewhere, &event("tame", "mcp__dx__dx_edit", &a));
        }
        record_with(&elsewhere, &event("other", "Bash", &b));

        let sessions = report_with(&elsewhere, &a);
        assert_eq!(sessions.len(), 2, "project b's session is not project a's");
        let wild = sessions.iter().find(|s| s.id == "wild").expect("wild");
        let tame = sessions.iter().find(|s| s.id == "tame").expect("tame");
        assert!(wild.off_method(), "{wild:?}");
        assert!(!tame.off_method(), "{tame:?}");
        let text = summary_with(&elsewhere, &a);
        assert!(text.contains("2 session(s), 1 off method"), "{text}");
        assert!(text.contains("wild"), "{text}");
        assert!(
            text.lines()
                .any(|l| l.contains("wild") && l.ends_with("OFF METHOD")),
            "{text}"
        );
        assert_eq!(report_with(&elsewhere, &b).len(), 1);
    }

    #[test]
    fn malformed_or_incomplete_input_records_nothing_and_says_nothing() {
        let project = indexed("malformed");
        let elsewhere = scratch("malformed-elsewhere").join("elsewhere.jsonl");
        for input in [
            "",
            "not json",
            r#"{"tool_name":"Bash"}"#,
            r#"{"session_id":"s"}"#,
        ] {
            assert!(record_with(&elsewhere, input).is_none(), "{input:?}");
        }
        assert!(!project.join(".doc").join(LOG_NAME).exists());
        assert!(!elsewhere.exists());
    }

    #[test]
    fn the_ledger_prunes_itself_past_its_bound() {
        let project = indexed("prune");
        let elsewhere = scratch("prune-elsewhere").join("elsewhere.jsonl");
        let ledger = project.join(".doc").join(LOG_NAME);
        let one =
            json!({"session":"s","root":project.to_string_lossy(),"tool":"Bash","raw":true,"at":1})
                .to_string();
        let mut big = String::new();
        for _ in 0..MAX_ENTRIES {
            big.push_str(&one);
            big.push('\n');
        }
        fs::write(&ledger, big).expect("seed");
        record_with(&elsewhere, &event("s", "Bash", &project));
        let lines = fs::read_to_string(&ledger).expect("read").lines().count();
        assert_eq!(lines, PRUNE_KEEP);
    }

    #[test]
    fn an_empty_project_summary_says_so() {
        let project = indexed("empty");
        let elsewhere = scratch("empty-elsewhere").join("elsewhere.jsonl");
        assert!(summary_with(&elsewhere, &project).starts_with("drift: no sessions recorded in "));
    }
}
