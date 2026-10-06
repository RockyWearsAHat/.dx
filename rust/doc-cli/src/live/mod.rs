//! `dx live` — every branch's merge state against base and every gate verdict, precomputed
//! into one snapshot so a single read answers it all.
//!
//! All work happens on branches: dx never writes a checkout an agent owns, only its own
//! worktrees under [`dir`]. Git failures come back as data, never panics or prompts.

pub mod checkout;
pub mod gates;
pub mod merge;
pub mod watch;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Most branches one snapshot covers (the most recent by committer date).
const MAX_BRANCHES: usize = 100;

/// One branch: its merge against base, its checkout, and its gate verdicts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchState {
    /// Branch name.
    pub branch: String,
    /// Tip commit.
    pub sha: String,
    /// The worktree that has this branch checked out, if any.
    pub worktree: Option<String>,
    /// The virtual merge of this branch into base.
    pub merge: merge::MergeStatus,
    /// dx's own checkout of the merged tree, when one was materialized.
    pub checkout: Option<String>,
    /// Gate verdicts in that checkout.
    pub gates: Vec<gates::GateVerdict>,
}

/// Everything known about one repo at one moment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Format version.
    pub v: u32,
    /// The repo's main worktree path.
    pub repo: String,
    /// Base branch name.
    pub base: String,
    /// Base tip commit.
    pub base_sha: String,
    /// When the snapshot was taken, ms since the epoch.
    pub updated_ms: u64,
    /// Gate verdicts of the main worktree (read only).
    pub base_gates: Vec<gates::GateVerdict>,
    /// One entry per live branch.
    pub branches: Vec<BranchState>,
}

/// Run git in `repo` without prompts; stdout on success, a sentence on failure.
fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The directory dx keeps snapshots and checkouts in: `<absolute git-common-dir>/dx-live`.
pub fn dir(repo: &Path) -> PathBuf {
    match git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
        Ok(out) => PathBuf::from(out.trim()).join("dx-live"),
        Err(_) => repo.join(".git").join("dx-live"),
    }
}

/// The base branch to use when none was given: `main`, else `master`, else the current one.
pub fn default_base(repo: &Path) -> String {
    for name in ["main", "master"] {
        if git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_ok() {
            return name.to_string();
        }
    }
    git(repo, &["symbolic-ref", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "main".to_string())
}

/// The top-level directory of the repo containing `path` (itself when git cannot say).
pub fn repo_of(path: &Path) -> PathBuf {
    match git(path, &["rev-parse", "--show-toplevel"]) {
        Ok(out) if !out.trim().is_empty() => PathBuf::from(out.trim()),
        _ => path.to_path_buf(),
    }
}

/// `(path, branch)` for every worktree, main worktree first.
fn worktrees(repo: &Path) -> Vec<(String, Option<String>)> {
    let Ok(out) = git(repo, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut list: Vec<(String, Option<String>)> = Vec::new();
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            list.push((path.to_string(), None));
        } else if let Some(branch) = line.strip_prefix("branch ") {
            if let Some(last) = list.last_mut() {
                last.1 = Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).to_string());
            }
        }
    }
    list
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Take a fresh snapshot. With `run`, clean branches are materialized and their stale
/// gates run; without it, only what is already known is read.
pub fn refresh(repo: &Path, base: &str, run: bool) -> Result<Snapshot, String> {
    let wts = worktrees(repo);
    let main_wt = wts
        .first()
        .map_or_else(|| repo.to_path_buf(), |(p, _)| PathBuf::from(p));
    let base_sha = git(repo, &["rev-parse", "--verify", &format!("refs/heads/{base}")])
        .map_err(|_| format!("base branch `{base}` does not exist in {}", repo.display()))?
        .trim()
        .to_string();
    let own = dir(repo);

    let refs = git(
        repo,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)\t%(objectname)",
            "refs/heads",
        ],
    )?;
    let mut branches = Vec::new();
    for line in refs.lines() {
        let Some((name, sha)) = line.split_once('\t') else {
            continue;
        };
        if name == base {
            continue;
        }
        let worktree = wts
            .iter()
            .find(|(_, b)| b.as_deref() == Some(name))
            .map(|(p, _)| p.clone());
        // dx-live's own checkouts are not branches an agent works on.
        if worktree.as_ref().is_some_and(|p| Path::new(p).starts_with(&own)) {
            continue;
        }
        if branches.len() >= MAX_BRANCHES {
            break;
        }
        let merge = merge::status(repo, base, name);
        let mut checkout_path = None;
        let mut verdicts = Vec::new();
        if merge.state == merge::MergeState::Clean {
            if let Some(tree) = merge.tree.as_deref().filter(|_| run) {
                match checkout::materialize(repo, base, name, tree) {
                    Ok(path) => {
                        let _ = checkout::run_stale(&path);
                        verdicts = gates::verdicts(&path);
                        checkout_path = Some(path.display().to_string());
                    }
                    Err(_) => {}
                }
            }
        }
        branches.push(BranchState {
            branch: name.to_string(),
            sha: sha.to_string(),
            worktree,
            merge,
            checkout: checkout_path,
            gates: verdicts,
        });
    }

    let names: Vec<String> = branches.iter().map(|b| b.branch.clone()).collect();
    let _ = checkout::prune(repo, &names);
    Ok(Snapshot {
        v: 1,
        repo: main_wt.display().to_string(),
        base: base.to_string(),
        base_sha,
        updated_ms: now_ms(),
        base_gates: gates::verdicts(&main_wt),
        branches,
    })
}

fn atomic_write(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot move {}: {e}", path.display()))
}

/// Write `snapshot.json` and `snapshot.txt` under [`dir`], each via tmp file + rename.
pub fn write(repo: &Path, s: &Snapshot) -> Result<(), String> {
    let d = dir(repo);
    std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    atomic_write(&d.join("snapshot.json"), &json)?;
    atomic_write(&d.join("snapshot.txt"), &render_text(s))
}

/// The cached text snapshot.
pub fn read_text(repo: &Path) -> Result<String, String> {
    let path = dir(repo).join("snapshot.txt");
    std::fs::read_to_string(&path).map_err(|e| format!("no snapshot at {} ({e})", path.display()))
}

/// The cached JSON snapshot.
pub fn read_json(repo: &Path) -> Result<String, String> {
    let path = dir(repo).join("snapshot.json");
    std::fs::read_to_string(&path).map_err(|e| format!("no snapshot at {} ({e})", path.display()))
}

/// `YYYY-MM-DDTHH:MM:SSZ` for ms since the epoch.
fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn sha7(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// `P/T pass` plus the failing names and stale count, as the summary lines show them.
fn gate_summary(gs: &[gates::GateVerdict]) -> (String, Vec<String>, usize) {
    let pass = gs.iter().filter(|g| g.state == gates::GateState::Pass).count();
    let fails = gs
        .iter()
        .filter(|g| g.state == gates::GateState::Fail)
        .map(|g| format!("{}#{}", g.doc, g.block))
        .collect();
    let stale = gs.iter().filter(|g| g.state == gates::GateState::Stale).count();
    (format!("{pass}/{}", gs.len()), fails, stale)
}

/// The snapshot as the one text an agent reads.
pub fn render_text(s: &Snapshot) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "dx live {} base {}@{} updated {}",
        s.repo,
        s.base,
        sha7(&s.base_sha),
        iso(s.updated_ms)
    );
    let (ratio, fails, _) = gate_summary(&s.base_gates);
    let _ = write!(o, "base: gates {ratio} pass");
    if !fails.is_empty() {
        let _ = write!(o, " fail: {}", fails.join(" "));
    }
    o.push('\n');
    for b in &s.branches {
        let state = format!("{:?}", b.merge.state).to_lowercase();
        let _ = write!(o, "{} {state}", b.branch);
        match b.merge.state {
            merge::MergeState::Clean => {
                let (ratio, fails, stale) = gate_summary(&b.gates);
                let _ = write!(o, " gates {ratio}");
                if !fails.is_empty() {
                    let _ = write!(o, " fail: {}", fails.join(" "));
                }
                if stale > 0 {
                    let _ = write!(o, " stale: {stale}");
                }
                if b.checkout.is_none() {
                    o.push_str(" (not run)");
                }
            }
            merge::MergeState::Conflict => {
                let _ = write!(o, " ({} files)", b.merge.conflicts.len());
            }
            merge::MergeState::Merged => o.push_str(" (already in base)"),
            merge::MergeState::Error => {
                let _ = write!(o, " ({})", b.merge.error.as_deref().unwrap_or("unknown error"));
            }
        }
        o.push('\n');
    }
    for b in &s.branches {
        if b.merge.state == merge::MergeState::Conflict {
            let _ = writeln!(o, "--- {} conflicts", b.branch);
            for c in &b.merge.conflicts {
                let _ = writeln!(o, "{}", c.path);
                let _ = writeln!(o, "{}", c.hunks.trim_end());
            }
        }
    }
    for b in &s.branches {
        for g in b.gates.iter().filter(|g| g.state == gates::GateState::Fail) {
            let _ = writeln!(o, "--- {} {}#{}", b.branch, g.doc, g.block);
            let _ = writeln!(o, "{}", g.tail.trim_end());
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use gates::{GateState, GateVerdict};
    use merge::{Conflict, MergeState, MergeStatus};

    fn ms(state: MergeState, conflicts: Vec<Conflict>) -> MergeStatus {
        MergeStatus {
            state,
            base_sha: "aaaaaaaaaa".into(),
            branch_sha: "bbbbbbbbbb".into(),
            tree: None,
            conflicts,
            error: None,
        }
    }

    fn gate(block: &str, state: GateState, tail: &str) -> GateVerdict {
        GateVerdict { doc: "a.dx".into(), block: block.into(), state, ms: Some(5), tail: tail.into() }
    }

    fn branch(name: &str, merge: MergeStatus, gates: Vec<GateVerdict>) -> BranchState {
        BranchState {
            branch: name.into(),
            sha: "bbbbbbbbbb".into(),
            worktree: None,
            merge,
            checkout: Some("/x".into()),
            gates,
        }
    }

    #[test]
    fn render_has_header_summary_branches_and_details() {
        let s = Snapshot {
            v: 1,
            repo: "/r".into(),
            base: "main".into(),
            base_sha: "aaaaaaaaaa".into(),
            updated_ms: 0,
            base_gates: vec![gate("t", GateState::Pass, ""), gate("u", GateState::Fail, "x")],
            branches: vec![
                branch(
                    "ok",
                    ms(MergeState::Clean, vec![]),
                    vec![gate("t", GateState::Pass, ""), gate("s", GateState::Stale, "")],
                ),
                branch(
                    "clash",
                    ms(MergeState::Conflict, vec![Conflict { path: "f.rs".into(), hunks: "<<<<<<< a\n=======\n>>>>>>> b".into() }]),
                    vec![],
                ),
                branch(
                    "bad",
                    ms(MergeState::Clean, vec![]),
                    vec![gate("chk", GateState::Fail, "boom line")],
                ),
            ],
        };
        let t = render_text(&s);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "dx live /r base main@aaaaaaa updated 1970-01-01T00:00:00Z");
        assert_eq!(lines[1], "base: gates 1/2 pass fail: a.dx#u");
        assert_eq!(lines[2], "ok clean gates 1/2 stale: 1");
        assert!(lines[3].starts_with("clash conflict"));
        assert_eq!(lines[4], "bad clean gates 0/1 fail: a.dx#chk");
        assert!(t.contains("--- clash conflicts\nf.rs\n<<<<<<< a"));
        assert!(t.contains("--- bad a.dx#chk\nboom line"));
    }

    #[test]
    fn iso_formats_a_known_instant() {
        assert_eq!(iso(1_700_000_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn dir_is_inside_the_git_common_dir_and_snapshot_round_trips() {
        let root = std::env::temp_dir().join(format!("dxlive-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for a in [&["init", "-q", "-b", "main"][..]] {
            git(&root, a).unwrap();
        }
        let d = dir(&root);
        assert!(d.ends_with(".git/dx-live"), "{}", d.display());
        let s = Snapshot { v: 1, repo: "/r".into(), base: "main".into(), base_sha: "abc".into(), updated_ms: 0, base_gates: vec![], branches: vec![] };
        write(&root, &s).unwrap();
        assert!(read_text(&root).unwrap().starts_with("dx live /r base main@abc"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
