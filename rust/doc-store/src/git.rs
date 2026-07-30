//! Git-driven routing: does a document belong to the repository or only to this machine?
//!
//! A document is **local-only** when git ignores it, has never tracked it, or has it staged
//! for deletion. Everything else is repo content. The distinction decides which pack a
//! document is exported to, so a scratch note in an ignored directory never lands in the
//! artifact a teammate clones.
//!
//! When git is unavailable, or the workspace is not a repository, every document is treated
//! as repo content — the safe default, because it keeps the document in the artifact that
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
        return Route::Local;
    }
    if succeeds(root, &["ls-files", "--error-unmatch", "--", relative]) {
        return Route::Repo;
    }
    // Present but never tracked: keep it local until someone commits it.
    Route::Local
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
    fn local_only_is_readable_on_the_route_itself() {
        assert!(Route::Local.is_local_only());
        assert!(!Route::Repo.is_local_only());
    }
}
