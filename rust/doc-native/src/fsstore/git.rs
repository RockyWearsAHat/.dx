//! Git-driven routing: classify a `.dx` path as repo-tracked or local-only.
//!
//! A document is **local-only** when it is ignored, not tracked, or removed from the
//! index — matching `localOnlyArchive = ignored || !tracked || removedFromIndex` in
//! `src/git-doc-state.ts`. Otherwise it is repo-tracked. When git is unavailable the
//! store defaults to the repo bundle, again matching the reference.

use std::process::Command;

use doc_core::bundle::GitFlags;

use super::{FsDocStore, Route, LOCAL_ARCHIVE_RELATIVE, REPO_ARCHIVE_RELATIVE};

impl Route {
    /// Workspace-relative path of the archive this route names.
    pub(super) fn archive_relative(self) -> &'static str {
        match self {
            Route::Repo => REPO_ARCHIVE_RELATIVE,
            Route::Local => LOCAL_ARCHIVE_RELATIVE,
        }
    }
}

/// Raw git classification of a single document path.
struct GitState {
    /// The file is tracked by git.
    tracked: bool,
    /// The file is ignored by a `.gitignore` rule.
    ignored: bool,
    /// The file has working-tree or index modifications.
    modified: bool,
    /// The file has staged changes in the index.
    staged: bool,
    /// The file was removed from the index (a `D` in the staged column).
    removed_from_index: bool,
}

impl FsDocStore {
    /// Decide whether `relative_path` routes to the repo or local archive via git.
    ///
    /// Runs `git` against the workspace root to classify the file (tracked / ignored /
    /// removed-from-index), reproducing `getGitDocState` in `src/git-doc-state.ts`. When
    /// git is unavailable or the file falls outside the git root, it defaults to
    /// [`Route::Repo`] — matching the reference's `includeInRepoArchive` default.
    pub(super) fn route(&self, relative_path: &str) -> (Route, GitFlags) {
        match self.git_state(relative_path) {
            Some(state) => {
                let local_only = state.ignored || !state.tracked || state.removed_from_index;
                let route = if local_only {
                    Route::Local
                } else {
                    Route::Repo
                };
                (
                    route,
                    GitFlags {
                        tracked: state.tracked,
                        untracked: !state.tracked && !state.ignored,
                        ignored: state.ignored,
                        modified: state.modified,
                        staged: state.staged,
                    },
                )
            }
            None => (Route::Repo, GitFlags::default()),
        }
    }

    /// Query git for `relative_path`'s state, or `None` when git is unavailable.
    fn git_state(&self, relative_path: &str) -> Option<GitState> {
        // Confirm a git repository covers the root; bail to the default route otherwise.
        if !self.git_ok(&["rev-parse", "--is-inside-work-tree"]) {
            return None;
        }

        let tracked = self.git_ok(&["ls-files", "--error-unmatch", "--", relative_path]);
        let ignored = self.git_ok(&["check-ignore", "-q", "--", relative_path]);

        let mut modified = false;
        let mut staged = false;
        let mut removed_from_index = false;
        if let Some(stdout) = self.git_output(&["status", "--porcelain=v1", "--", relative_path]) {
            for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
                let bytes = line.as_bytes();
                let x = bytes.first().copied().unwrap_or(b' ');
                let y = bytes.get(1).copied().unwrap_or(b' ');
                if x == b'?' && y == b'?' {
                    continue;
                }
                if x != b' ' && x != b'?' {
                    staged = true;
                }
                if x == b'D' {
                    removed_from_index = true;
                }
                if y != b' ' || matches!(x, b'M' | b'A' | b'D' | b'R' | b'C') {
                    modified = true;
                }
            }
        }

        Some(GitState {
            tracked,
            ignored,
            modified,
            staged,
            removed_from_index,
        })
    }

    /// Run a git subcommand against the root, returning `true` on a zero exit status.
    fn git_ok(&self, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// Run a git subcommand against the root, returning its stdout on a zero exit status.
    fn git_output(&self, args: &[&str]) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            None
        }
    }
}
