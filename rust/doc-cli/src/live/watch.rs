//! Watches registered repos for ref changes and refreshes their live snapshot.
//!
//! Idle cost is a handful of `stat` calls per repo per second; git runs only on a change.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// (path, mtime, size) of every watched file; equal fingerprints mean nothing changed.
pub type Fingerprint = Vec<(PathBuf, Option<SystemTime>, u64)>;

/// The watch list: `$DX_LIVE_REPOS`, else `~/.config/dx/live-repos`.
pub(crate) fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DX_LIVE_REPOS") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
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
        fs::create_dir_all(dir).map_err(|e| format!("{} cannot be created: {e}", dir.display()))?;
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
    let d = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
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

fn refresh_cached_only(repo: &Path, common: &Path, all_repos: &Arc<Vec<PathBuf>>) {
    let start = Instant::now();
    let base = base_of(common);

    // Read cached verdicts and write the snapshot (fast, no gates run).
    match super::refresh(repo, base, false) {
        Ok(snap) => {
            if let Err(e) = super::write(repo, &snap) {
                eprintln!("dx live: {} snapshot not written: {e}", repo.display());
            }
            if let Err(e) = super::board::write_combined(all_repos.as_ref()) {
                eprintln!("dx live: board not written: {e}");
            }
            eprintln!(
                "dx live: {} snapshot in {} ms (cached)",
                repo.display(),
                start.elapsed().as_millis()
            );
        }
        Err(e) => eprintln!("dx live: {} refresh (cached) failed: {e}", repo.display()),
    }
}

fn refresh_with_gates(repo: &Path, common: &Path, all_repos: &Arc<Vec<PathBuf>>) {
    let start = Instant::now();
    let base = base_of(common);

    // Run all stale gates.
    match super::refresh(repo, base, true) {
        Ok(_) => {
            // Gates ran; now get a fresh snapshot (with potentially stale verdicts updated).
            match super::refresh(repo, base, false) {
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
                    if let Err(e) = super::board::write_combined(all_repos.as_ref()) {
                        eprintln!("dx live: board not written: {e}");
                    }
                    eprintln!(
                        "dx live: {} refreshed in {} ms ({ran} gates ran)",
                        repo.display(),
                        start.elapsed().as_millis()
                    );
                }
                Err(e) => eprintln!(
                    "dx live: {} refresh (after gates) failed: {e}",
                    repo.display()
                ),
            }
        }
        Err(e) => eprintln!("dx live: {} refresh (gates) failed: {e}", repo.display()),
    }
}

struct GateQueue {
    queue: std::collections::VecDeque<(PathBuf, PathBuf)>,
    waiting: std::collections::HashSet<PathBuf>,
}

impl GateQueue {
    fn new() -> Self {
        GateQueue {
            queue: std::collections::VecDeque::new(),
            waiting: std::collections::HashSet::new(),
        }
    }
}

/// Poll every `poll_interval`; refresh a repo (one at a time) when its refs changed.
/// `cached_fn` is called for cached passes; `gates_fn` for gate passes.
pub fn spawn_with<C, G>(
    repos: Vec<PathBuf>,
    cached_fn: C,
    gates_fn: G,
    poll_interval: Duration,
) -> JoinHandle<()>
where
    C: Fn(&Path, &Path) + Send + Sync + 'static,
    G: Fn(&Path, &Path) + Send + Sync + 'static,
{
    let cached_fn = Arc::new(cached_fn);
    let gates_fn = Arc::new(gates_fn);

    let gate_queue = Arc::new(Mutex::new(GateQueue::new()));
    let gate_cond = Arc::new(Condvar::new());
    let gate_queue_worker = gate_queue.clone();
    let gate_cond_worker = gate_cond.clone();
    let gates_fn_worker = gates_fn.clone();

    // Start the gate worker thread.
    std::thread::spawn(move || {
        loop {
            let next = {
                let mut g = gate_queue_worker.lock().unwrap();
                // Standard wait pattern: check condition and wait atomically.
                while g.queue.is_empty() {
                    g = gate_cond_worker.wait(g).unwrap();
                }
                // Pop from queue and remove from waiting set.
                let item = g.queue.pop_front();
                if let Some(ref repo) = item {
                    g.waiting.remove(&repo.0);
                }
                item
            };

            if let Some((repo, common)) = next {
                gates_fn_worker(&repo, &common);
            }
        }
    });

    // Watcher thread.
    std::thread::spawn(move || {
        let mut state: Vec<Watched> = repos.into_iter().map(|r| (r, None, None)).collect();

        loop {
            let _refreshed = tick(&mut state, &mut |repo, common| {
                cached_fn(repo, common);

                // Queue this repo for gate work (only if not already waiting).
                let mut g = gate_queue.lock().unwrap();
                if !g.waiting.contains(repo) {
                    g.queue
                        .push_back((repo.to_path_buf(), common.to_path_buf()));
                    g.waiting.insert(repo.to_path_buf());
                    drop(g); // Release lock before notify.
                    gate_cond.notify_one();
                }
            });

            std::thread::sleep(poll_interval);
        }
    })
}

/// Poll every second; refresh a repo (one at a time) when its refs changed. The fingerprint is
/// taken before the refresh, so changes arriving during it trigger one follow-up.
pub fn spawn(repos: Vec<PathBuf>) -> JoinHandle<()> {
    let all_repos = Arc::new(repos.clone());

    let all_repos_cached = all_repos.clone();
    let cached_fn = move |repo: &Path, common: &Path| {
        refresh_cached_only(repo, common, &all_repos_cached);
    };

    let all_repos_gates = all_repos.clone();
    let gates_fn = move |repo: &Path, common: &Path| {
        refresh_with_gates(repo, common, &all_repos_gates);
    };

    spawn_with(repos, cached_fn, gates_fn, Duration::from_secs(1))
}

/// One watched repo: its path, its git common dir once known, and its last fingerprint.
type Watched = (PathBuf, Option<PathBuf>, Option<Fingerprint>);

/// One poll of every watched repo: `refresh` runs for each whose refs changed. git runs
/// once per repo, to find its common dir; after that an idle poll is stat calls only.
/// Returns whether any repo was refreshed.
fn tick(state: &mut [Watched], refresh: &mut dyn FnMut(&Path, &Path)) -> bool {
    let mut refreshed = false;
    for (repo, common, last) in state.iter_mut() {
        if common.is_none() {
            *common = common_dir(repo);
        }
        let Some(c) = common.clone() else { continue };
        if changed(&c, last) {
            refresh(repo, &c);
            refreshed = true;
        }
    }
    refreshed
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
        assert!(tick(&mut state, &mut |_, _| refreshed += 1));
        assert_eq!(refreshed, 1, "the first poll sees the repo");
        let spawned = GIT_SPAWNS.with(std::cell::Cell::get);
        for _ in 0..5 {
            assert!(!tick(&mut state, &mut |_, _| refreshed += 1));
        }
        assert_eq!(
            refreshed, 1,
            "nothing changed, so no refresh (no gate work)"
        );
        assert_eq!(
            GIT_SPAWNS.with(std::cell::Cell::get),
            spawned,
            "no git process while idle"
        );
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

    #[test]
    fn refresh_cached_writes_snapshot_before_running_gates() {
        let repo = temp("refresh-cached");
        sh(&repo, &["init", "-b", "main"]);
        sh(&repo, &["commit", "--allow-empty", "-m", "base"]);
        sh(&repo, &["checkout", "-b", "feature"]);
        sh(&repo, &["commit", "--allow-empty", "-m", "feature work"]);

        // Cached refresh should list the new branch without running any gates.
        let snap_cached =
            super::super::refresh(&repo, "main", false).expect("cached refresh should succeed");
        assert!(
            snap_cached.branches.iter().any(|b| b.branch == "feature"),
            "snapshot should name the new branch: {snap_cached:#?}"
        );

        // No gates should have run yet (all verdicts empty or stale).
        let all_gates = snap_cached
            .base_gates
            .iter()
            .chain(snap_cached.branches.iter().flat_map(|b| b.gates.iter()));
        for gate in all_gates {
            assert!(
                gate.ms.is_none(),
                "cached refresh should not run gates: {gate:?}"
            );
        }

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_deleted_branch_loses_its_dx_checkout() {
        let repo = temp("deleted-branch");
        sh(&repo, &["init", "-b", "main"]);
        sh(&repo, &["commit", "--allow-empty", "-m", "base"]);
        sh(&repo, &["checkout", "-b", "feature"]);
        sh(&repo, &["commit", "--allow-empty", "-m", "feature work"]);
        sh(&repo, &["checkout", "main"]);

        // First refresh with gates to materialize checkouts.
        let snap1 =
            super::super::refresh(&repo, "main", true).expect("first refresh should succeed");
        assert!(
            snap1.branches.iter().any(|b| b.branch == "feature"),
            "snapshot should list feature branch"
        );
        let feature_checkout = snap1
            .branches
            .iter()
            .find(|b| b.branch == "feature")
            .and_then(|b| b.checkout.as_ref())
            .cloned();
        assert!(feature_checkout.is_some(), "feature checkout should exist");

        // Verify the checkout directory exists.
        if let Some(checkout_path) = &feature_checkout {
            assert!(
                std::path::Path::new(checkout_path).exists(),
                "checkout directory should exist"
            );
        }

        // Delete the feature branch.
        sh(&repo, &["branch", "-D", "feature"]);

        // Second refresh with gates should prune the deleted branch's checkout.
        let snap2 = super::super::refresh(&repo, "main", true)
            .expect("second refresh after deleting branch should succeed");
        assert!(
            !snap2.branches.iter().any(|b| b.branch == "feature"),
            "snapshot should not list deleted feature branch"
        );

        // Verify the checkout directory no longer exists.
        if let Some(checkout_path) = &feature_checkout {
            assert!(
                !std::path::Path::new(checkout_path).exists(),
                "deleted branch's checkout directory should be removed"
            );
        }

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_slow_gate_in_one_repo_does_not_delay_another_repos_snapshot() {
        use std::sync::Arc;
        let a = temp("iso-a");
        sh(&a, &["init", "-q", "-b", "main"]);
        sh(&a, &["commit", "-q", "--allow-empty", "-m", "a0"]);
        let b = temp("iso-b");
        sh(&b, &["init", "-q", "-b", "main"]);
        sh(&b, &["commit", "-q", "--allow-empty", "-m", "b0"]);
        let log: Arc<Mutex<Vec<(String, PathBuf, Instant)>>> = Arc::new(Mutex::new(Vec::new()));
        let (lc, lg) = (log.clone(), log.clone());
        let slow = a.clone();
        let cached = move |r: &Path, _c: &Path| {
            lc.lock()
                .unwrap()
                .push(("cached".into(), r.to_path_buf(), Instant::now()));
        };
        let gates = move |r: &Path, _c: &Path| {
            lg.lock()
                .unwrap()
                .push(("gates".into(), r.to_path_buf(), Instant::now()));
            if r == slow.as_path() {
                std::thread::sleep(Duration::from_secs(4));
            }
        };
        let _h = spawn_with(
            vec![a.clone(), b.clone()],
            cached,
            gates,
            Duration::from_millis(100),
        );
        let count = |k: &str, r: &Path| {
            log.lock()
                .unwrap()
                .iter()
                .filter(|(x, p, _)| x == k && p == r)
                .count()
        };
        let wait = |f: &dyn Fn() -> bool, secs: u64| {
            let t = Instant::now();
            while !f() {
                assert!(
                    t.elapsed() < Duration::from_secs(secs),
                    "timeout waiting for condition after {secs}s"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        // Startup: both repos are seen once; wait until A's (slow) gate pass has begun.
        wait(&|| count("gates", &a) >= 1, 3);
        let b_before = count("cached", &b);
        sh(&b, &["commit", "-q", "--allow-empty", "-m", "b1"]);
        let t = Instant::now();
        wait(&|| count("cached", &b) > b_before, 3); // B's snapshot while A's gate still runs
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "B waited {:?} behind A's gate",
            t.elapsed()
        );
        // A commit in A while A's gate pass runs must queue A again (dedupe clears on pop).
        sh(&a, &["commit", "-q", "--allow-empty", "-m", "a1"]);
        wait(&|| count("gates", &a) >= 2, 12);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
