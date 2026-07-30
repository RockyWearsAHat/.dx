//! Git-driven routing: does a document belong to the repository or only to this machine?
//!
//! A document is **local-only when git ignores it**, and repo content otherwise. The
//! distinction decides which pack a document is exported to, so a scratch note in an ignored
//! directory never lands in the artifact a teammate clones.
//!
//! # Untracked is not local-only
//! It is tempting to also treat an untracked file as local — it is, after all, not in the
//! repository yet. That is wrong, and dangerously so. The rule that matters is: **a document's
//! content must go wherever its pointer goes.** A file git does not ignore is a file the author
//! is about to `git add`; if its content sat in the ignored pack, committing would push a
//! pointer with nothing behind it and the document would be gone for everyone else. Ignored
//! status is the only signal that actually predicts whether the pointer will be committed.
//!
//! The failure mode of being wrong the other way is trivial: a scratch file that never gets
//! committed leaves a few bytes in the repo pack.
//!
//! When git is unavailable, or the workspace is not a repository, every document is treated as
//! repo content — the same safe default, because it keeps the document in the artifact that
//! travels.

use std::path::Path;
use std::process::Command;

/// Which pack a document belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Repository content: exported to the committed pack.
    Repo,
    /// Local-only content: exported to the ignored pack, never committed.
    Local,
}

impl Route {
    /// Whether this route keeps the document out of the committed artifact.
    #[must_use]
    pub fn is_local_only(self) -> bool {
        matches!(self, Route::Local)
    }
}

/// Decide the route for `relative` inside the workspace at `root`.
///
/// Complexity: two short-lived `git` invocations; callers that classify many documents at
/// once should expect one process pair per path.
#[must_use]
pub fn route(root: &Path, relative: &str) -> Route {
    if !succeeds(root, &["rev-parse", "--is-inside-work-tree"]) {
        return Route::Repo;
    }
    if succeeds(root, &["check-ignore", "-q", "--", relative]) {
        Route::Local
    } else {
        // Tracked, or untracked but not ignored — either way its pointer can be committed, so
        // its content belongs in the pack that travels with the repository.
        Route::Repo
    }
}

/// Whether `git args` run against `root` exits zero.
fn succeeds(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_that_is_not_a_repository_routes_to_the_repo_pack() {
        // The safe default: content still reaches the artifact that travels.
        let root = std::env::temp_dir();
        assert_eq!(route(&root, "nothing-here.dx"), Route::Repo);
    }

    #[test]
    fn this_repository_tracks_its_own_examples() {
        // Running inside the dx repo, a committed example must classify as repo content.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        if !succeeds(root, &["rev-parse", "--is-inside-work-tree"]) {
            return; // Not a git checkout (e.g. a vendored tarball); nothing to assert.
        }
        assert_eq!(route(root, "examples/welcome.dx"), Route::Repo);
    }

    #[test]
    fn an_ignored_path_is_local_only() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        if !succeeds(root, &["rev-parse", "--is-inside-work-tree"]) {
            return;
        }
        // `rust/target` is gitignored in this repository.
        assert_eq!(route(root, "rust/target/scratch.dx"), Route::Local);
    }

    #[test]
    fn a_new_untracked_document_routes_to_the_committed_pack() {
        // The bug this pins: a document copied into a project is untracked, and routing it to
        // the git-ignored pack meant committing its pointer pushed a pointer with no content
        // behind it. Content must go wherever the pointer goes.
        let root = std::env::temp_dir().join("dx-git-route-untracked");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        if !std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .is_ok_and(|status| status.success())
        {
            return; // No git on this machine; nothing to assert.
        }
        std::fs::write(root.join("fresh.dx"), "::paragraph id=p\nx\n::end\n").expect("write");

        assert_eq!(route(&root, "fresh.dx"), Route::Repo);
    }

    #[test]
    fn local_only_is_readable_on_the_route_itself() {
        assert!(Route::Local.is_local_only());
        assert!(!Route::Repo.is_local_only());
    }
}
