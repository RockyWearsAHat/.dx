//! Watches registered repos for ref changes and refreshes their live snapshot.
//!
//! Idle cost is a handful of `stat` calls per repo per second; git runs only on a change.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// (path, mtime, size) of every watched file; equal fingerprints mean nothing changed.
pub type Fingerprint = Vec<(PathBuf, Option<SystemTime>, u64)>;

fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DX_LIVE_REPOS") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".config/dx/live-repos")
}

/// Repos listed in the config file (one absolute path per line); missing paths are skipped.
pub fn repos() -> Vec<PathBuf> {
    let text = fs::read_to_string(config_path()).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

thread_local! {
    /// git processes this thread started — the idle test's proof that nothing spawned.
    static GIT_SPAWNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    GIT_SPAWNS.with(|n| n.set(n.get() + 1));
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Register a repo: canonicalize, require a git work tree, append if absent.
pub fn add_repo(path: &Path) -> Result<PathBuf, String> {
    let abs = fs::canonicalize(path)
        .map_err(|e| format!("{} cannot be resolved: {e}", path.display()))?;
    if git(&abs, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return Err(format!("{} is not a git work tree", abs.display()));
    }
    let cfg = config_path();
    let text = fs::read_to_string(&cfg).unwrap_or_default();
    let line = abs.to_string_lossy().to_string();
    if text.lines().any(|l| l.trim() == line) {
        return Ok(abs);
    }
    if let Some(dir) = cfg.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("{} cannot be created: {e}", dir.display()))?;
    }
    let mut next = text;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&line);
    next.push('\n');
    fs::write(&cfg, next).map_err(|e| format!("{} cannot be written: {e}", cfg.display()))?;
    Ok(abs)
}

fn common_dir(repo: &Path) -> Option<PathBuf> {
    let d = git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Some(PathBuf::from(d))
}

fn base_of(common: &Path) -> &'static str {
    if common.join("refs/heads/main").exists() || packed_has(common, "refs/heads/main") {
        "main"
    } else {
        "master"
    }
}

fn packed_has(common: &Path, name: &str) -> bool {
    fs::read_to_string(common.join("packed-refs"))
        .map(|t| t.lines().any(|l| l.ends_with(&format!(" {name}"))))
        .unwrap_or(false)
}

fn stat(out: &mut Fingerprint, p: PathBuf) {
    match fs::metadata(&p) {
        Ok(m) => out.push((p, m.modified().ok(), m.len())),
        Err(_) => out.push((p, None, 0)),
    }
}

fn walk(dir: &Path, out: &mut Fingerprint) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            stat(out, p);
        }
    }
}

/// Stats HEAD, packed-refs, refs/heads/** and worktrees/*/HEAD (not dx's own worktrees).
pub fn fingerprint(common: &Path) -> Fingerprint {
    let mut out = Fingerprint::new();
    stat(&mut out, common.join("HEAD"));
    stat(&mut out, common.join("packed-refs"));
    walk(&common.join("refs/heads"), &mut out);
    if let Ok(rd) = fs::read_dir(common.join("worktrees")) {
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            let ours = fs::read_to_string(d.join("gitdir"))
                .map(|g| g.contains("/dx-live/"))
                .unwrap_or(false);
            if !ours {
                stat(&mut out, d.join("HEAD"));
            }
        }
    }
    out
}

/// True when the watched files differ from `last`; `last` is updated to the current state.
pub fn changed(common: &Path, last: &mut Option<Fingerprint>) -> bool {
    let now = fingerprint(common);
    let differs = last.as_ref() != Some(&now);
    *last = Some(now);
    differs
}

fn refresh_one(repo: &Path, common: &Path) {
    let start = Instant::now();
    let base = base_of(common);
    match super::refresh(repo, base, true) {
        Ok(snap) => {
            let ran = snap
                .base_gates
                .iter()
                .chain(snap.branches.iter().flat_map(|b| b.gates.iter()))
                .filter(|g| g.ms.is_some())
                .count();
            if let Err(e) = super::write(repo, &snap) {
                eprintln!("dx live: {} snapshot not written: {e}", repo.display());
            }
            eprintln!(
                "dx live: {} refreshed in {} ms ({ran} gates ran)",
                repo.display(),
                start.elapsed().as_millis()
            );
        }
        Err(e) => eprintln!("dx live: {} refresh failed: {e}", repo.display()),
    }
}

/// Poll every second; refresh a repo (one at a time) when its refs changed. The fingerprint is
/// taken before the refresh, so changes arriving during it trigger one follow-up.
pub fn spawn(repos: Vec<PathBuf>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut state: Vec<Watched> = repos.into_iter().map(|r| (r, None, None)).collect();
        loop {
            tick(&mut state, &mut |repo, common| refresh_one(repo, common));
            std::thread::sleep(Duration::from_secs(1));
        }
    })
}

/// One watched repo: its path, its git common dir once known, and its last fingerprint.
type Watched = (PathBuf, Option<PathBuf>, Option<Fingerprint>);

/// One poll of every watched repo: `refresh` runs for each whose refs changed. git runs
/// once per repo, to find its common dir; after that an idle poll is stat calls only.
fn tick(state: &mut [Watched], refresh: &mut dyn FnMut(&Path, &Path)) {
    for (repo, common, last) in state.iter_mut() {
        if common.is_none() {
            *common = common_dir(repo);
        }
        let Some(c) = common.clone() else { continue };
        if changed(&c, last) {
            refresh(repo, &c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dxwatch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn add_repo_and_repos_round_trip() {
        let d = temp("rt");
        sh(&d, &["init", "-b", "main"]);
        std::env::set_var("DX_LIVE_REPOS", d.join("cfg/live-repos"));
        let added = add_repo(&d).unwrap();
        assert_eq!(add_repo(&d).unwrap(), added);
        assert_eq!(repos(), vec![added.clone()]);
        let plain = temp("plain");
        assert!(add_repo(&plain).is_err());
        fs::write(
            d.join("cfg/live-repos"),
            format!("{}\n/nonexistent/zzz\n", added.display()),
        )
        .unwrap();
        assert_eq!(repos(), vec![added]);
        std::env::remove_var("DX_LIVE_REPOS");
    }

    #[test]
    fn an_idle_poll_is_stat_calls_only_no_git_and_no_gate_work() {
        let d = temp("idle");
        sh(&d, &["init", "-b", "main"]);
        sh(&d, &["commit", "--allow-empty", "-m", "one"]);
        let mut state: Vec<Watched> = vec![(d.clone(), None, None)];
        let mut refreshed = 0;
        tick(&mut state, &mut |_, _| refreshed += 1);
        assert_eq!(refreshed, 1, "the first poll sees the repo");
        let spawned = GIT_SPAWNS.with(std::cell::Cell::get);
        for _ in 0..5 {
            tick(&mut state, &mut |_, _| refreshed += 1);
        }
        assert_eq!(refreshed, 1, "nothing changed, so no refresh (no gate work)");
        assert_eq!(GIT_SPAWNS.with(std::cell::Cell::get), spawned, "no git process while idle");
        sh(&d, &["commit", "--allow-empty", "-m", "two"]);
        tick(&mut state, &mut |_, _| refreshed += 1);
        assert_eq!(refreshed, 2, "a new commit is seen on the next poll");
    }

    #[test]
    fn a_new_commit_on_a_branch_flips_changed() {
        let d = temp("chg");
        sh(&d, &["init", "-b", "main"]);
        sh(&d, &["commit", "--allow-empty", "-m", "one"]);
        let common = common_dir(&d).unwrap();
        assert_eq!(base_of(&common), "main");
        let mut last = None;
        assert!(changed(&common, &mut last));
        assert!(!changed(&common, &mut last));
        sh(&d, &["checkout", "-b", "feat"]);
        assert!(changed(&common, &mut last));
        assert!(!changed(&common, &mut last));
        sh(&d, &["commit", "--allow-empty", "-m", "two"]);
        assert!(changed(&common, &mut last));
    }
}
