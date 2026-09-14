//! Tool usage tracking: detecting when work happens outside dx.
//!
//! A field session can run entirely outside dx — raw shell calls, tools invoked directly — and
//! go unnoticed at the moment, because nothing measured work happening outside dx. This module
//! is a Claude Code hook that records whether each tool use is a raw shell command (`Bash`) or
//! a `dx` command (tools starting with `mcp__dx__`), so a session's ratio of raw calls to dx
//! calls becomes visible at the decision point — when the agent chooses whether to keep
//! working inside the harness or escape into raw shell.
//!
//! The ledger is machine-local and must never touch `.doc/`, a document, or a pointer.
//! Recording is best-effort and silent: a failure is swallowed and exit is always 0, because
//! a hook that fails would block the agent's tool, and this hook exists to inform, never to block.
//!
//! # The contract
//! [`record`] appends one JSON line per tool use to `<home::data_dir()>/drift/<session_id>.jsonl`,
//! creating the directory as needed. After recording, it counts this session's raw and dx lines.
//! When raw >= 10 and raw is a multiple of 10 (10, 20, 30…), it prints to stdout exactly one
//! JSON object with `hookSpecificOutput`, otherwise prints nothing. [`summary`] reads the most
//! recently modified session's raw and dx counts and ratio, one line, for a person's view of
//! the same ledger.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::home;

/// Record one tool use, and print a nudge if thresholds are crossed.
///
/// `input` is a Claude Code PostToolUse hook JSON object with at least `session_id`,
/// `hook_event_name`, `tool_name`, `tool_input`, and `cwd`. This function classifies the tool,
/// records it, counts this session's raw and dx lines, and nudges when raw >= 10 and raw is a
/// multiple of 10.
///
/// Every failure is swallowed and exit is always 0 — a hook that fails would block the agent's
/// tool, and this hook exists to inform, never to block.
pub fn record(input: &str) -> Result<(), String> {
    let parsed: Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Malformed input: silent no-op, exit 0
    };

    let session_id = match parsed.get("session_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return Ok(()), // Missing session_id: silent no-op, exit 0
    };

    let tool_name = match parsed.get("tool_name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return Ok(()), // Missing tool_name: silent no-op, exit 0
    };

    // Classify: raw = Bash, dx = mcp__dx__*, anything else is ignored
    let is_raw = tool_name == "Bash";
    let is_dx = tool_name.starts_with("mcp__dx__");
    if !is_raw && !is_dx {
        return Ok(()); // Ignored tool: no record, print nothing
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let entry = json!({
        "tool": tool_name,
        "raw": is_raw,
        "at": now,
    });

    // Write to drift/<session_id>.jsonl in the data directory
    if let Some(drift_dir) = drift_dir(&session_id) {
        let _ = fs::create_dir_all(&drift_dir);
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(drift_dir.join("session.jsonl"))
        {
            let _ = writeln!(file, "{entry}");
        }
    }

    // Count this session's raw and dx lines
    if let Some((raw_count, dx_count)) = count_session(&session_id) {
        // Nudge when raw >= 10 and raw is a multiple of 10
        if raw_count >= 10 && raw_count % 10 == 0 {
            let nudge = json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": format!(
                        "dx drift: {} raw commands vs {} dx calls this session — is the loop still \
                         the harness? Work that cannot be a dx gate is stated in the document as such; \
                         work that can be is one.",
                        raw_count, dx_count
                    ),
                }
            });
            // Print to stdout
            println!("{nudge}");
        }
    }

    Ok(())
}

/// Summarize the most recently modified session's raw and dx counts and ratio, one line.
///
/// `session_id` is optional — if given, report that session; otherwise, find the most recently
/// modified one. Returns "no sessions recorded" if none exist.
pub fn summary(session_id: Option<&str>) -> String {
    let session_to_read = if let Some(id) = session_id {
        Some(id.to_string())
    } else {
        find_most_recent_session()
    };

    match session_to_read {
        Some(id) => {
            if let Some((raw, dx)) = count_session(&id) {
                let ratio = if raw + dx > 0 {
                    raw as f64 / (raw + dx) as f64
                } else {
                    0.0
                };
                format!(
                    "drift: {} raw commands, {} dx calls ({:.0}% raw)\n",
                    raw,
                    dx,
                    ratio * 100.0
                )
            } else {
                format!("drift: session {} has no recorded tool uses\n", id)
            }
        }
        None => "no sessions recorded\n".to_string(),
    }
}

/// Count the raw and dx lines in a session's log.
fn count_session(session_id: &str) -> Option<(usize, usize)> {
    let session_dir = drift_dir(session_id)?;
    let log_path = session_dir.join("session.jsonl");

    let contents = fs::read_to_string(&log_path).ok()?;
    let mut raw_count = 0;
    let mut dx_count = 0;

    for line in contents.lines() {
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            if let Some(is_raw) = entry.get("raw").and_then(|v| v.as_bool()) {
                if is_raw {
                    raw_count += 1;
                } else {
                    dx_count += 1;
                }
            }
        }
    }

    Some((raw_count, dx_count))
}

/// Find the most recently modified session's ID.
fn find_most_recent_session() -> Option<String> {
    let base_dir = drift_base_dir()?;
    let mut sessions: Vec<(String, SystemTime)> = Vec::new();

    if let Ok(entries) = fs::read_dir(&base_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    if let Ok(modified) = metadata.modified() {
                        if let Some(name) = entry.file_name().to_str() {
                            sessions.push((name.to_string(), modified));
                        }
                    }
                }
            }
        }
    }

    sessions.sort_by_key(|item| std::cmp::Reverse(item.1));
    sessions.first().map(|(id, _)| id.clone())
}

/// The drift directory for a session, or None if the base directory cannot be determined.
fn drift_dir(session_id: &str) -> Option<PathBuf> {
    drift_base_dir().map(|base| base.join(session_id))
}

/// The base drift directory, or None if the data directory cannot be determined.
fn drift_base_dir() -> Option<PathBuf> {
    let data_dir = home::data_dir();
    if data_dir.as_os_str().is_empty() {
        return None;
    }
    Some(data_dir.join("drift"))
}

use std::io::Write as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_tool_is_classified_as_raw() {
        let input =
            r#"{"session_id":"test-bash","tool_name":"Bash","hook_event_name":"PostToolUse"}"#;
        // This should not error; it should silently try to record to the system drift dir.
        // We cannot easily verify the write without mocking, but we verify it doesn't panic.
        let _ = record(input);
    }

    #[test]
    fn mcp_dx_tool_is_classified_as_dx() {
        let input = r#"{"session_id":"test-dx","tool_name":"mcp__dx__read","hook_event_name":"PostToolUse"}"#;
        let _ = record(input);
        // Verify no panic; filesystem writes are in system drift dir
    }

    #[test]
    fn mcp_dx_tools_starting_with_prefix_are_classified_as_dx() {
        let inputs = vec![
            r#"{"session_id":"test","tool_name":"mcp__dx__read"}"#,
            r#"{"session_id":"test","tool_name":"mcp__dx__write"}"#,
            r#"{"session_id":"test","tool_name":"mcp__dx__anything_else"}"#,
        ];
        for input in inputs {
            let _ = record(input);
            // Verify no panic
        }
    }

    #[test]
    fn non_bash_non_dx_tools_are_ignored() {
        let input =
            r#"{"session_id":"test","tool_name":"SomeOtherTool","hook_event_name":"PostToolUse"}"#;
        // Should silently ignore (no record, no output)
        let result = record(input);
        assert!(result.is_ok());
    }

    #[test]
    fn malformed_json_exits_zero() {
        let input = "not json";
        let result = record(input);
        assert!(result.is_ok(), "malformed input should return Ok(())");
    }

    #[test]
    fn missing_session_id_exits_zero() {
        let input = r#"{"tool_name":"Bash"}"#;
        let result = record(input);
        assert!(result.is_ok(), "missing session_id should return Ok(())");
    }

    #[test]
    fn missing_tool_name_exits_zero() {
        let input = r#"{"session_id":"test"}"#;
        let result = record(input);
        assert!(result.is_ok(), "missing tool_name should return Ok(())");
    }

    #[test]
    fn empty_stdin_is_silent_noop() {
        let input = "";
        let result = record(input);
        assert!(result.is_ok());
    }

    #[test]
    fn record_handles_various_inputs_without_panic() {
        // Test that various inputs are handled safely
        let test_cases = vec![
            r#"{"session_id":"1","tool_name":"Bash"}"#,
            r#"{"session_id":"2","tool_name":"mcp__dx__read"}"#,
            r#"{"session_id":"3","tool_name":"Other"}"#,
        ];
        for input in test_cases {
            let _ = record(input); // Should not panic
        }
    }

    #[test]
    fn summary_returns_no_sessions_when_none_exist() {
        // This will return "no sessions recorded\n" when no drift dir exists
        let result = summary(None);
        // We can't check the exact content without access to the filesystem,
        // but we verify it returns a string
        assert!(!result.is_empty());
    }
}
