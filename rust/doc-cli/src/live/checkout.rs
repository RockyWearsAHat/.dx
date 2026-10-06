//! The dx-owned virtual-merge checkouts: one detached worktree per live branch, under
//! `dir(repo)/wt/`, always sitting on a commit that merges the base and the branch.
//!
//! dx never writes a checkout an agent owns. Everything here happens inside `dir(repo)/wt`,
//! and git failures come back as sentences, never panics or prompts.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use doc_run::{run_document, RunOptions, DEFAULT_TIMEOUT_SECONDS};

use crate::workspace;

/// Ignored build folders worth cloning from the main working tree so cargo starts warm.
const WARM_TARGETS: [&str; 2] = ["target", "rust/target"];

/// The folder that holds every dx-live checkout of `repo`.
fn wt_root(repo: &Path) -> PathBuf {
    super::dir(repo).join("wt")
}

/// The checkout folder name for a branch: slashes become `__`.
fn folder_name(branch: &str) -> String {
    branch.replace('/', "__")
}

/// Run `git` with `args` in `cwd`: no prompts, no stdin. Returns trimmed stdout or a sentence.
fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "dx-live")
        .env("GIT_AUTHOR_EMAIL", "dx-live@localhost")
        .env("GIT_COMMITTER_NAME", "dx-live")
        .env("GIT_COMMITTER_EMAIL", "dx-live@localhost")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The commit a revision names.
fn commit_of(repo: &Path, rev: &str) -> Result<String, String> {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
        .map_err(|_| format!("`{rev}` is not a commit in {}", repo.display()))
}

/// Clone the ignored build folders of the main working tree into a checkout that lacks them.
fn warm_targets(repo: &Path, checkout: &Path) {
    for rel in WARM_TARGETS {
        let from = repo.join(rel);
        let to = checkout.join(rel);
        if !from.is_dir() || to.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                continue;
            }
        }
        let mut cmd = Command::new("cp");
        if cfg!(target_os = "macos") {
            cmd.arg("-cR");
        } else {
            cmd.args(["-r", "--reflink=auto"]);
        }
        // A failed warm-up only costs a cold build; never fail the checkout over it.
        let ok = cmd
            .arg(&from)
            .arg(&to)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_dir_all(&to);
        }
    }
}

/// Make the dx-owned checkout of the virtual merge `tree` of `base` and `branch`, and
/// return its path.
///
/// The worktree is created once, detached; each call then moves it to a fresh merge commit
/// with `checkout -f`, so git rewrites only the files that changed and unchanged mtimes
/// keep cargo's incremental state.
///
/// # Errors
/// A sentence when git cannot resolve the commits, create the worktree or move it.
pub fn materialize(repo: &Path, base: &str, branch: &str, tree: &str) -> Result<PathBuf, String> {
    let base_sha = commit_of(repo, base)?;
    let branch_sha = commit_of(repo, branch)?;
    let path = wt_root(repo).join(folder_name(branch));

    if !path.join(".git").exists() {
        if path.exists() {
            // A leftover folder git does not know: ours to clear, it lives under wt/.
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("could not clear {}: {e}", path.display()))?;
        }
        std::fs::create_dir_all(wt_root(repo))
            .map_err(|e| format!("could not create {}: {e}", wt_root(repo).display()))?;
        let _ = git(repo, &["worktree", "prune"]);
        let target = path.to_string_lossy().to_string();
        git(repo, &["worktree", "add", "--detach", &target, &base_sha])?;
    }

    let commit = git(
        repo,
        &["commit-tree", tree, "-p", &base_sha, "-p", &branch_sha, "-m", "dx-live virtual merge"],
    )?;
    git(&path, &["checkout", "--detach", "-f", &commit])?;
    warm_targets(repo, &path);
    Ok(path)
}

/// Run every stale runnable block of every `.dx` document in `checkout`, exactly as
/// `dx run <doc>` would, approving nothing: only code already approved on this machine
/// runs (approval is keyed by fingerprint, so the base's approvals carry over).
/// Returns how many blocks executed.
///
/// # Errors
/// A sentence when no document could be run at all.
///
/// One machine-wide exclusive lock ([`run_lock_path`]) is held for the whole run, so the
/// watcher never runs two gate runs at once across every watched repo, and gates run one
/// at a time (`jobs = 1`). An approved `confine=host` gate does not inherit
/// `CARGO_TARGET_DIR` unless `DX_KEEP_TARGET_DIR=1`: an agent shell's target directory
/// would silently replace the checkout's own warm one.
pub fn run_stale(checkout: &Path) -> Result<usize, String> {
    let _held = run_lock()?;
    #[cfg(test)]
    tests::while_held();
    let memo = super::gates::memo_file(checkout);
    if let Some(memo) = &memo {
        doc_run::load_memo(memo);
    }
    let result = run_stale_held(checkout);
    if let Some(memo) = &memo {
        let _ = doc_run::save_memo(memo);
    }
    result
}

/// The machine-wide live-run lock: `DX_LIVE_RUN_LOCK`, else `~/.dx/live/run.lock`.
fn run_lock_path() -> PathBuf {
    if let Some(path) = std::env::var_os("DX_LIVE_RUN_LOCK") {
        return PathBuf::from(path);
    }
    if cfg!(test) {
        // Tests never queue behind a real watcher's run.
        return std::env::temp_dir().join(format!("dx-live-run-{}.lock", std::process::id()));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".dx/live/run.lock")
}

/// Take the live-run lock, waiting for whoever holds it; released when the file drops.
fn run_lock() -> Result<std::fs::File, String> {
    let path = run_lock_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    file.lock()
        .map_err(|e| format!("could not lock {}: {e}", path.display()))?;
    Ok(file)
}

/// The variables a live gate run keeps out of a host gate's environment.
fn unset_for(keep_target_dir: Option<&str>) -> Vec<String> {
    if keep_target_dir == Some("1") {
        Vec::new()
    } else {
        vec!["CARGO_TARGET_DIR".to_string()]
    }
}

/// How the live runner runs one document: serially, without `CARGO_TARGET_DIR`.
fn live_options(path: &Path) -> RunOptions {
    RunOptions {
        document_dir: workspace::document_dir(path),
        default_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        jobs: Some(1),
        unset_env: unset_for(std::env::var("DX_KEEP_TARGET_DIR").ok().as_deref()),
        ..RunOptions::default()
    }
}

fn run_stale_held(checkout: &Path) -> Result<usize, String> {
    let mut executed = 0;
    let mut problems = Vec::new();
    for path in workspace::discover(checkout) {
        let source = match workspace::read(&path) {
            Ok(s) => s,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        let report = match run_document(
            &source,
            &live_options(&path),
            &workspace::resolver_for(&path),
        ) {
            Ok(r) => r,
            Err(e) => {
                problems.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        executed += report
            .runs
            .iter()
            .filter(|run| run.status == "ok" || run.status == "error")
            .count();
        if report.changed {
            if let Err(e) = workspace::save_run_result(&path, &source, &report.source) {
                problems.push(e);
            }
        }
    }
    if executed == 0 && !problems.is_empty() {
        return Err(problems.join("; "));
    }
    Ok(executed)
}

/// Remove every dx-live checkout whose branch is not in `live_branches`, then prune git's
/// worktree list. Nothing outside `dir(repo)/wt` is ever touched.
///
/// # Errors
/// A sentence naming the checkouts that could not be removed.
pub fn prune(repo: &Path, live_branches: &[String]) -> Result<(), String> {
    let root = wt_root(repo);
    let keep: Vec<String> = live_branches.iter().map(|b| folder_name(b)).collect();
    let mut failed = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !path.is_dir() || keep.contains(&name) {
                continue;
            }
            let target = path.to_string_lossy().to_string();
            if git(repo, &["worktree", "remove", "--force", &target]).is_err() {
                // Git did not know it (or lost it): the folder is still ours to delete.
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    failed.push(format!("{}: {e}", path.display()));
                }
            }
        }
    }
    let _ = git(repo, &["worktree", "prune"]);
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("could not remove checkouts: {}", failed.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    /// Milliseconds a test makes [`run_stale`] hold its lock, and the spans it held it.
    static HOLD_MS: AtomicU64 = AtomicU64::new(0);
    static HELD: Mutex<Vec<(Instant, Instant)>> = Mutex::new(Vec::new());

    pub(super) fn while_held() {
        let ms = HOLD_MS.load(Ordering::SeqCst);
        if ms > 0 {
            let start = Instant::now();
            std::thread::sleep(Duration::from_millis(ms));
            HELD.lock().unwrap().push((start, Instant::now()));
        }
    }

    #[test]
    fn two_concurrent_live_runs_serialize_on_the_machine_lock() {
        let empty = std::env::temp_dir().join(format!("dx-live-serial-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&empty);
        HOLD_MS.store(300, Ordering::SeqCst);
        let started = Instant::now();
        let runs: Vec<_> = (0..2)
            .map(|_| {
                let dir = empty.clone();
                std::thread::spawn(move || run_stale(&dir))
            })
            .collect();
        for run in runs {
            let _ = run.join().unwrap();
        }
        HOLD_MS.store(0, Ordering::SeqCst);
        let mut spans = HELD.lock().unwrap().clone();
        spans.sort();
        assert!(spans.len() >= 2, "{spans:?}");
        // The second waited for the first: the spans do not overlap, and two 300 ms
        // holds took at least 600 ms end to end.
        for pair in spans.windows(2) {
            assert!(pair[1].0 >= pair[0].1, "overlapping runs: {spans:?}");
        }
        assert!(started.elapsed() >= Duration::from_millis(600), "{:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn live_runs_are_serial_and_drop_cargo_target_dir_unless_kept() {
        assert_eq!(unset_for(None), vec!["CARGO_TARGET_DIR".to_string()]);
        assert_eq!(unset_for(Some("0")), vec!["CARGO_TARGET_DIR".to_string()]);
        assert!(unset_for(Some("1")).is_empty());
        let options = live_options(Path::new("/x/doc.dx"));
        assert_eq!(options.jobs, Some(1));
    }

    fn sh(cwd: &Path, args: &[&str]) -> String {
        git(cwd, args).unwrap_or_else(|e| panic!("{e}"))
    }

    /// A temp repo on `main` with a, b, and a branch `feat/x` adding c.
    fn seed(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-live-checkout-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        sh(&root, &["init", "-q", "-b", "main"]);
        sh(&root, &["config", "user.email", "t@t"]);
        sh(&root, &["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-q", "-m", "base"]);
        sh(&root, &["checkout", "-q", "-b", "feat/x"]);
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-q", "-m", "feat"]);
        sh(&root, &["checkout", "-q", "main"]);
        root
    }

    /// The merge tree of base and branch, computed by git itself.
    fn merge_tree(repo: &Path, base: &str, branch: &str) -> String {
        sh(repo, &["merge-tree", "--write-tree", base, branch])
            .lines()
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn materialize_checks_out_the_merged_files_and_rewrites_only_changes() {
        let repo = seed("mat");
        let tree = merge_tree(&repo, "main", "feat/x");
        let wt = materialize(&repo, "main", "feat/x", &tree).expect("materialize");
        assert!(wt.starts_with(wt_root(&repo)));
        assert!(wt.ends_with("feat__x"));
        for f in ["a.txt", "b.txt", "c.txt"] {
            assert!(wt.join(f).is_file(), "{f} missing");
        }
        let before = std::fs::metadata(wt.join("a.txt")).unwrap().modified().unwrap();

        // The branch moves on: c changes, a and b do not.
        sh(&repo, &["checkout", "-q", "feat/x"]);
        std::fs::write(repo.join("c.txt"), "c2\n").unwrap();
        sh(&repo, &["commit", "-q", "-am", "more"]);
        sh(&repo, &["checkout", "-q", "main"]);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let tree2 = merge_tree(&repo, "main", "feat/x");
        let wt2 = materialize(&repo, "main", "feat/x", &tree2).expect("again");
        assert_eq!(wt, wt2);
        assert_eq!(std::fs::read_to_string(wt.join("c.txt")).unwrap(), "c2\n");
        let after = std::fs::metadata(wt.join("a.txt")).unwrap().modified().unwrap();
        assert_eq!(before, after, "an unchanged file must keep its mtime");
        // The agent's own checkout was never touched.
        assert!(!repo.join("c.txt").exists());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn materialize_clones_a_build_folder_and_reports_bad_input_as_data() {
        let repo = seed("warm");
        std::fs::create_dir_all(repo.join("target")).unwrap();
        std::fs::write(repo.join("target/artifact"), "x").unwrap();
        let tree = merge_tree(&repo, "main", "feat/x");
        let wt = materialize(&repo, "main", "feat/x", &tree).expect("materialize");
        assert!(wt.join("target/artifact").is_file());
        assert!(materialize(&repo, "main", "no-such-branch", &tree).is_err());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn prune_removes_only_dead_checkouts() {
        let repo = seed("prune");
        let tree = merge_tree(&repo, "main", "feat/x");
        let wt = materialize(&repo, "main", "feat/x", &tree).expect("materialize");
        prune(&repo, &["feat/x".to_string()]).expect("keep");
        assert!(wt.is_dir(), "a live branch's checkout stays");
        prune(&repo, &[]).expect("prune");
        assert!(!wt.exists());
        assert!(!sh(&repo, &["worktree", "list"]).contains("feat__x"));
        assert!(repo.join("a.txt").is_file(), "the main checkout is untouched");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
