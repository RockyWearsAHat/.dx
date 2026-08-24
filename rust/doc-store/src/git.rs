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

/// Whether git will carry `relative` in this workspace's history.
///
/// True for a file git neither ignores nor sits outside of; false when the workspace is not a
/// repository at all. This is the question a *storage* decision asks, and it is deliberately
/// not [`route`]'s question. Routing defaults to `Repo` when git is absent because the risk
/// there is losing content — the safe answer is the artifact that travels. Storage has no such
/// risk: a directory with no git has no history to delta, so the safe answer is the smaller
/// file.
#[must_use]
pub fn tracks(root: &Path, relative: &str) -> bool {
    succeeds(root, &["rev-parse", "--is-inside-work-tree"])
        && !succeeds(root, &["check-ignore", "-q", "--", relative])
}

/// The repository's other working trees, if `root` is a repository that has any.
///
/// `root` itself is never in the list, so a non-empty answer means exactly one thing: this
/// path is not the only checkout of this repository, and a relative path names a document in
/// *this* one. A long-lived process rooted at a directory — the MCP server — cannot see which
/// checkout its caller is working in, so the honest thing is to say the choice exists rather
/// than resolve it silently against whichever tree the process happened to start in.
///
/// An empty vector for anything that is not a repository, and for a repository with no linked
/// worktrees. Complexity: one short-lived `git` invocation.
#[must_use]
pub fn linked_worktrees(root: &Path) -> Vec<std::path::PathBuf> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    // The name git prints is the real path; `root` may reach the same directory through a
    // symlink, so compare what both resolve to and fall back to the paths as given.
    let here = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(std::path::PathBuf::from)
        .filter(|path| {
            path.canonicalize().unwrap_or_else(|_| path.clone()) != here && path.as_path() != root
        })
        .collect()
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

/// Whether `relative` is a tracked `.dx` pointer whose path is git-ignored.
///
/// This detects the specific failure mode: a document's pointer was force-added and committed,
/// but the pointer's path is itself ignored by git. Content for such a pointer stays in the
/// git-ignored `.doc/local.dxcp` pack, never in the committed artifact, so the pointer is
/// unresolvable on any other machine.
#[must_use]
pub fn is_tracked_but_ignored(root: &Path, relative: &str) -> bool {
    // Both conditions must be true: the file is tracked by git, AND its path is git-ignored.
    // We can't use check-ignore directly on a tracked file — git reports it as not ignored
    // because tracking overrides the ignore pattern. Instead, we check if a sibling path
    // in the same directory would be ignored. If the directory is in an ignore pattern,
    // the sibling probe will report as ignored.
    let is_tracked = succeeds(root, &["ls-files", "--error-unmatch", "--", relative]);
    if !is_tracked {
        return false;
    }

    // Create a hypothetical probe path in the same directory to test if it would be ignored.
    let path = std::path::PathBuf::from(relative);
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let probe = parent.join(".gitignore-probe");
    succeeds(
        root,
        &["check-ignore", "-q", "--", &probe.to_string_lossy()],
    )
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
    fn a_directory_that_is_not_a_repository_tracks_nothing() {
        // Storage's default is the opposite of routing's, and on purpose: no history to
        // delta means the compressed form is simply the better file.
        assert!(!tracks(&std::env::temp_dir(), "nothing-here.dxcp"));
    }

    #[test]
    fn the_committed_pack_is_tracked_and_the_local_one_is_not() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        if !succeeds(root, &["rev-parse", "--is-inside-work-tree"]) {
            return;
        }
        assert!(tracks(root, ".doc/repo.dxcp"));
        assert!(!tracks(root, ".doc/local.dxcp"));
    }

    #[test]
    fn a_directory_that_is_not_a_repository_has_no_linked_worktrees() {
        assert!(linked_worktrees(&std::env::temp_dir()).is_empty());
    }

    #[test]
    fn a_checkout_lists_its_other_working_trees_and_never_itself() {
        // What this pins: a process rooted at one checkout must be able to find out that the
        // repository has others. Answering with the root itself would make "are there other
        // trees?" always true and the answer useless.
        let root = std::env::temp_dir().join("dx-git-linked-worktrees");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .is_ok_and(|out| out.status.success())
        };
        if !git(&["init", "-q", "-b", "main"]) {
            return; // No git on this machine; nothing to assert.
        }
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.dx"), "::paragraph id=p\nx\n::end\n").expect("write");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "one"]);

        assert!(linked_worktrees(&root).is_empty());

        let side = root.join("trees").join("side");
        assert!(git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            &side.to_string_lossy(),
        ]));

        let listed = linked_worktrees(&root);
        assert_eq!(listed.len(), 1, "only the linked tree, never the root");
        assert_eq!(
            listed[0].canonicalize().expect("linked tree"),
            side.canonicalize().expect("linked tree")
        );
        // And from inside the linked tree, the main checkout is the other one.
        assert_eq!(linked_worktrees(&side).len(), 1);
    }

    #[test]
    fn local_only_is_readable_on_the_route_itself() {
        assert!(Route::Local.is_local_only());
        assert!(!Route::Repo.is_local_only());
    }

    #[test]
    fn a_pointer_forced_into_an_ignored_path_is_tracked_but_ignored() {
        // Create a temporary git repository.
        let root = std::env::temp_dir().join("dx-git-tracked-but-ignored");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        if !std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .is_ok_and(|status| status.success())
        {
            return; // No git on this machine.
        }

        // Create an ignored directory.
        let ignored_dir = root.join("ignored");
        std::fs::create_dir_all(&ignored_dir).expect("ignored dir");
        std::fs::write(root.join(".gitignore"), "ignored/\n").expect("gitignore");

        // Commit the .gitignore.
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", ".gitignore"])
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-q", "-m", "add gitignore"])
            .output()
            .expect("git commit");

        // Create a document under the ignored directory.
        let doc_file = ignored_dir.join("doc.dx");
        std::fs::write(&doc_file, "::paragraph id=p\nContent\n::end\n").expect("write doc");

        // Verify it's not currently tracked (it's in an ignored directory).
        assert!(!tracks(&root, "ignored/doc.dx"));

        // Now force-add it to git (simulating the failure mode).
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "-f", "ignored/doc.dx"])
            .output()
            .expect("git add -f");

        // After force-add, it should be tracked but still in an ignored path.
        assert!(tracks(&root, "ignored/doc.dx"));
        assert!(is_tracked_but_ignored(&root, "ignored/doc.dx"));

        // A regular committed file in an ignored path should NOT be both conditions at once.
        // (This shouldn't happen in practice, but let's be thorough.)
    }

    #[test]
    fn a_normal_committed_document_is_not_tracked_but_ignored() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        if !succeeds(root, &["rev-parse", "--is-inside-work-tree"]) {
            return;
        }

        // `examples/welcome.dx` is committed, so it should not match the condition.
        assert!(!is_tracked_but_ignored(root, "examples/welcome.dx"));
    }

    #[test]
    fn an_ordinary_ignored_document_is_not_tracked_but_ignored() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        if !succeeds(root, &["rev-parse", "--is-inside-work-tree"]) {
            return;
        }

        // `rust/target/scratch.dx` is ignored but not tracked.
        assert!(!is_tracked_but_ignored(root, "rust/target/scratch.dx"));
    }
}
