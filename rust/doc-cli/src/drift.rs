//! Work outside dx, made visible at the moment it happens.
//!
//! # Why
//! A field session ran an entire task outside dx — dozens of raw shell calls, a handful of dx
//! calls — and was confident the whole time that each step was right. Nothing stopped it,
//! because the only thing dx measured was search coverage, and that looked fine. The
//! instruction to work through documents lived upstream of every choice; the choice itself
//! had no checkpoint. This module is that checkpoint: a Claude Code `PostToolUse` hook that
//! counts each tool use as *raw* (`Bash`) or *dx* (`mcp__dx__*`) and, at every tenth raw call,
//! hands the agent one sentence naming the ratio — at the call, not in an orientation
//! document read an hour earlier.
//!
//! # The contract
//! - [`record`] reads one hook event (JSON on stdin: `session_id`, `tool_name`, and the rest)
//!   and appends one line to `<data_dir>/drift/<session_id>.jsonl`. Any other tool is neither
//!   recorded nor answered.
//! - The ledger is machine-local. It never touches `.doc/`, a document, or a pointer.
//! - When the call just recorded is raw and the session's raw count is a multiple of
//!   [`NUDGE_EVERY`], stdout carries exactly one JSON object whose `additionalContext` is the
//!   nudge; otherwise nothing. A dx call never nudges, or a session sitting at ten raw calls
//!   would be nudged on every dx call after them.
//! - Every failure is swallowed and the exit is always 0. A hook that fails blocks the tool
//!   it wraps, and this hook exists to inform, never to block.
//! - [`summary`] is a person's view of the same ledger: one line for the most recently
//!   written session, or a named one.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::home;

/// A nudge is spoken at every this-many raw calls: the tenth, the twentieth, and so on.
pub const NUDGE_EVERY: usize = 10;

/// The directory under the data directory that holds one ledger per session.
const LEDGER_DIR: &str = "drift";

/// Record one hook event in the machine-local ledger and print the nudge, if one is due.
///
/// This is the hook entry `dx drift` runs with JSON on stdin. It never fails: malformed or
/// unrelated input is a silent no-op, and an unwritable ledger loses the line rather than the
/// agent's tool call.
pub fn record(input: &str) {
    if let Some(nudge) = record_in(&ledger_base(), input) {
        println!("{nudge}");
    }
}

/// [`record`] against an explicit ledger directory; returns the nudge object when one is due.
///
/// Separated so the threshold can be pinned by a test without touching the real ledger.
pub fn record_in(base: &Path, input: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(input).ok()?;
    let session = parsed.get("session_id")?.as_str()?;
    if session.is_empty() || session.contains(['/', '\\']) || session.starts_with('.') {
        return None;
    }
    let tool = parsed.get("tool_name")?.as_str()?;
    let raw = classify(tool)?;

    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = json!({ "tool": tool, "raw": raw, "at": at });

    let path = ledger_path(base, session);
    let _ = fs::create_dir_all(base);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{entry}");
    }

    let (raw_count, dx_count) = count(&path)?;
    if raw && raw_count > 0 && raw_count % NUDGE_EVERY == 0 {
        return Some(
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": nudge_text(raw_count, dx_count),
                }
            })
            .to_string(),
        );
    }
    None
}

/// The one sentence the agent reads at the checkpoint.
fn nudge_text(raw: usize, dx: usize) -> String {
    format!(
        "dx drift: {raw} raw commands vs {dx} dx calls this session — is the loop still the \
         harness? Work that cannot be a dx gate is stated in the document as such; work that \
         can be is one."
    )
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

/// One line describing a session's ledger: the most recently written session, or `session`.
pub fn summary(session: Option<&str>) -> String {
    summary_in(&ledger_base(), session)
}

/// [`summary`] against an explicit ledger directory.
pub fn summary_in(base: &Path, session: Option<&str>) -> String {
    let Some(id) = session.map(str::to_string).or_else(|| most_recent(base)) else {
        return "drift: no sessions recorded\n".to_string();
    };
    match count(&ledger_path(base, &id)) {
        Some((raw, dx)) if raw + dx > 0 => {
            let percent = raw * 100 / (raw + dx);
            format!("drift: {raw} raw commands, {dx} dx calls ({percent}% raw) — session {id}\n")
        }
        _ => format!("drift: session {id} has no recorded tool uses\n"),
    }
}

/// Raw and dx counts in one ledger, or `None` when there is no ledger to read.
fn count(path: &Path) -> Option<(usize, usize)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut raw = 0;
    let mut dx = 0;
    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match entry.get("raw").and_then(Value::as_bool) {
            Some(true) => raw += 1,
            Some(false) => dx += 1,
            None => {}
        }
    }
    Some((raw, dx))
}

/// The session whose ledger was written last.
fn most_recent(base: &Path) -> Option<String> {
    let mut newest: Option<(SystemTime, String)> = None;
    for entry in fs::read_dir(base).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(when, _)| modified > *when) {
            newest = Some((modified, stem.to_string()));
        }
    }
    newest.map(|(_, id)| id)
}

/// Where one session's ledger lives.
fn ledger_path(base: &Path, session: &str) -> PathBuf {
    base.join(format!("{session}.jsonl"))
}

/// The real ledger directory: `<data_dir>/drift`.
fn ledger_base() -> PathBuf {
    home::data_dir().join(LEDGER_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-drift-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn event(session: &str, tool: &str) -> String {
        json!({
            "session_id": session,
            "hook_event_name": "PostToolUse",
            "tool_name": tool,
            "tool_input": {},
            "cwd": "/tmp",
        })
        .to_string()
    }

    #[test]
    fn bash_is_raw_dx_tools_are_dx_and_anything_else_is_not_recorded() {
        let base = scratch("classify");
        record_in(&base, &event("s", "Bash"));
        record_in(&base, &event("s", "mcp__dx__dx_search"));
        record_in(&base, &event("s", "Read"));
        record_in(&base, &event("s", "mcp__helpers__lookup"));
        assert_eq!(count(&ledger_path(&base, "s")), Some((1, 1)));
    }

    #[test]
    fn the_nudge_speaks_at_every_tenth_raw_call_and_never_on_a_dx_call() {
        let base = scratch("threshold");
        let mut spoken = Vec::new();
        for n in 1..=21 {
            if record_in(&base, &event("s", "Bash")).is_some() {
                spoken.push(n);
            }
            // A dx call after the tenth raw call must not repeat the nudge.
            assert!(record_in(&base, &event("s", "mcp__dx__dx_source")).is_none());
        }
        assert_eq!(spoken, vec![10, 20]);
    }

    #[test]
    fn the_nudge_is_a_post_tool_use_context_naming_both_counts() {
        let base = scratch("shape");
        let mut last = None;
        for _ in 0..10 {
            last = record_in(&base, &event("s", "Bash"));
        }
        let nudge: Value = serde_json::from_str(&last.expect("tenth call nudges")).expect("json");
        let output = &nudge["hookSpecificOutput"];
        assert_eq!(output["hookEventName"], "PostToolUse");
        let context = output["additionalContext"].as_str().expect("context");
        assert!(
            context.contains("10 raw commands vs 0 dx calls"),
            "{context}"
        );
    }

    #[test]
    fn malformed_or_incomplete_input_records_nothing_and_says_nothing() {
        let base = scratch("malformed");
        for input in [
            "",
            "not json",
            r#"{"tool_name":"Bash"}"#,
            r#"{"session_id":"s"}"#,
        ] {
            assert!(record_in(&base, input).is_none(), "{input:?}");
        }
        assert!(!base.exists(), "nothing was written for {}", base.display());
    }

    #[test]
    fn a_session_id_cannot_escape_the_ledger_directory() {
        let base = scratch("escape");
        assert!(record_in(&base, &event("../elsewhere", "Bash")).is_none());
        assert!(!base.exists());
    }

    #[test]
    fn summary_reads_the_named_or_the_latest_session() {
        let base = scratch("summary");
        assert_eq!(summary_in(&base, None), "drift: no sessions recorded\n");
        record_in(&base, &event("old", "Bash"));
        record_in(&base, &event("old", "Bash"));
        record_in(&base, &event("old", "mcp__dx__dx_search"));
        assert_eq!(
            summary_in(&base, Some("old")),
            "drift: 2 raw commands, 1 dx calls (66% raw) — session old\n"
        );
        assert_eq!(
            summary_in(&base, Some("never")),
            "drift: session never has no recorded tool uses\n"
        );
        assert!(summary_in(&base, None).ends_with("— session old\n"));
    }
}
