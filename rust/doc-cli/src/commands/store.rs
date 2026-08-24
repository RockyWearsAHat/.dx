//! Store commands: `sync`, `stats`, `rm`, `textconv`, and `git-setup`.
//!
//! These are the commands that exist because content lives in the store rather than in the
//! `.dx` files. [`run_sync`] repairs a workspace, [`run_stats`] reports what storage bought,
//! [`run_rm`] deletes a document deliberately, and [`run_textconv`] plus [`run_git_setup`]
//! are what make git behave normally against a workspace of pointers — `git diff` shows the
//! document, not the digest.

use std::path::{Path, PathBuf};

use doc_store::pack;

use crate::args::Args;
use crate::workspace;

/// Git config key for the dx diff driver's text conversion.
const TEXTCONV_KEY: &str = "diff.dx.textconv";
/// Git config key naming the merge driver, and the key holding its command line.
const MERGE_NAME_KEY: &str = "merge.dx.name";
/// The command git runs to merge a path it could not merge itself.
const MERGE_DRIVER_KEY: &str = "merge.dx.driver";
/// What `git status` calls the driver when it reports a conflict.
const MERGE_NAME: &str = "dx documents, merged block by block";
/// The `.gitattributes` line that points `.dx` files at the driver.
const ATTRIBUTES_LINE: &str = "*.dx diff=dx merge=dx";
/// The `.gitattributes` line that routes the committed pack through the same driver.
///
/// Both lines are needed and neither is enough. Without the pack line git calls the pack
/// binary and keeps one side whole; without the `*.dx` line every pointer conflicts on a hex
/// digest. They are written together, and [`crate::commands::merge`] is why they agree.
const PACK_ATTRIBUTES_LINE: &str = ".doc/repo.dxcp merge=dx";

/// The directory a command should operate on: the first positional, else the current one.
fn target_directory(args: &Args) -> PathBuf {
    args.positional(0)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// `dx sync` — reconcile the workspace so every `.dx` pointer resolves.
///
/// Adopts plain-text documents anything else wrote, restores documents from the committed
/// pack when the local index is missing, rewrites drifted pointers, and collects unreferenced
/// chunks. Reports what it could *not* resolve rather than papering over it.
pub fn run_sync(args: &Args) -> Result<String, String> {
    let root = workspace::workspace_root(&target_directory(args));
    let report = workspace::sync(&root)?;

    let mut out = format!("dx sync — {}\n", root.display());
    // A workspace subscribed to a report project is reconciled with the intake too: `dx sync`
    // is the command a person runs to make a checkout true, and a `reports.dx` missing what
    // other agents filed is exactly that kind of untrue. Unsubscribed workspaces read one
    // file and move on.
    let reports = match crate::intake::subscription_for(&root) {
        Ok(Some(subscription)) => match crate::intake::sync(&subscription) {
            Ok(synced) if synced.changed() || !synced.problems.is_empty() => {
                Some(synced.summary(&subscription.document()))
            }
            Ok(_) => None,
            Err(reason) => Some(format!("reports could not be synced — {reason}")),
        },
        Ok(None) => None,
        Err(reason) => Some(format!("subscriptions unreadable — {reason}")),
    };
    if let Some(said) = &reports {
        out.push_str(&format!("\n  {}\n", said.replace('\n', "\n  ")));
    }
    if report.is_clean() {
        if reports.is_none() {
            out.push_str("\n  nothing to do; every document already resolves\n");
        }
        return Ok(out);
    }

    let sections: [(&str, &Vec<String>); 8] = [
        ("adopted from plain text", &report.ingested),
        (
            "CONFLICTED — a merge left markers in the file",
            &report.conflicted,
        ),
        ("restored from a pack", &report.restored),
        ("pointer rewritten", &report.stubs_written),
        ("pruned (file deleted from the tree)", &report.pruned),
        ("pack rewritten", &report.packs_rewritten),
        ("UNRESOLVED", &report.unresolved),
        (
            "UNRESOLVED — pointer forced into git-ignored path",
            &report.tracked_but_ignored,
        ),
    ];
    for (label, paths) in sections {
        if paths.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{label} ({})\n", paths.len()));
        for path in paths {
            out.push_str(&format!("  {path}\n"));
        }
    }
    if report.chunks_collected > 0 {
        out.push_str(&format!(
            "\ncollected {} unreferenced chunk(s)\n",
            report.chunks_collected
        ));
    }
    if !report.conflicted.is_empty() {
        out.push_str(
            "\nConflicted documents were left exactly as they are. Open each one, keep the \
             words\nyou want, delete the <<<<<<< ======= >>>>>>> lines, and run `dx sync` \
             again —\nit adopts the file the moment the markers are gone.\n",
        );
    }
    if !report.unresolved.is_empty() || !report.tracked_but_ignored.is_empty() {
        out.push_str("\nUnresolved pointers have no content in .doc/:\n");
        if !report.unresolved.is_empty() {
            out.push_str(
                "  Generic unresolved: restore .doc/repo.dxcp from version control, or delete \
                 the pointers if the documents are gone.\n",
            );
        }
        if !report.tracked_but_ignored.is_empty() {
            out.push_str(
                "  Tracked-but-ignored: these pointers were force-added to git, but their path \
                 is git-ignored, so content stays in .doc/local.dxcp (never committed). Restore \
                 .doc/local.dxcp from this machine's backup, remove the pointers, or unforce-add \
                 them from git: `git rm --cached <path>` + `git commit`.\n",
            );
        }
    }
    Ok(out)
}

/// `dx rm <file>` — delete one document, deliberately.
///
/// Deleting the pointer file by hand and running `dx sync` prunes too, but that route reads
/// as repair. This command names the intent: the store forgets the document, the packs are
/// rewritten so the deletion sticks, and the file goes with it. Version history survives —
/// `git checkout` + `dx sync` restores an old revision whole.
pub fn run_rm(args: &Args) -> Result<String, String> {
    let path = args.positional(0).map(PathBuf::from).ok_or_else(|| {
        "dx rm deletes one document — name its .dx file: dx rm notes.dx".to_string()
    })?;
    let removed = workspace::remove(&path)?;
    Ok(if removed.stored {
        format!(
            "removed {} — the store forgot it and the packs were rewritten; version \
             history stays (`git checkout` + `dx sync` restores)\n",
            path.display()
        )
    } else {
        format!(
            "removed {} — a plain file nothing had adopted; the store never held it\n",
            path.display()
        )
    })
}

/// `dx stats` — what the store holds, and what sharing and compression saved.
pub fn run_stats(args: &Args) -> Result<String, String> {
    let root = workspace::workspace_root(&target_directory(args));
    let stats = workspace::stats(&root)?;

    let mut out = format!("dx stats — {}\n\n", root.display());
    if stats.documents == 0 {
        out.push_str("  no documents stored yet\n");
        return Ok(out);
    }

    out.push_str(&format!("  documents        {}\n", stats.documents));
    out.push_str(&format!(
        "  blocks           {} ({} distinct)\n",
        stats.chunk_references, stats.chunks
    ));
    // More distinct chunks than current references is version history — chunks kept so
    // old revisions still diff. Name it as what it is, never round it to "shared 0".
    if stats.chunks > stats.chunk_references {
        out.push_str(&format!(
            "  blocks in history {} (older versions, kept for git diff)\n",
            stats.chunks - stats.chunk_references
        ));
    } else {
        out.push_str(&format!(
            "  blocks shared    {}\n",
            stats.chunk_references - stats.chunks
        ));
    }
    out.push_str(&format!("  source bytes     {}\n", stats.source_bytes));
    out.push_str(&format!("  stored bytes     {}\n", stats.stored_bytes));
    if let Some(percent) = stats.compaction_percent() {
        out.push_str(&format!("  stored / source  {percent:.1}%\n"));
    }
    Ok(out)
}

/// `dx textconv <file>` — print the document a pointer stands for.
///
/// This is git's `textconv` hook, so it must behave like `cat` on a document: content to
/// standard output, and never a diagnostic in place of content. A file that is already plain
/// text passes straight through, which is what makes the driver safe to install before a
/// workspace has been converted.
///
/// The file git passes is a **temporary copy of a blob**, not the document in the workspace, so
/// resolution goes by the digest inside the pointer rather than by path.
///
/// The workspace is the one git is running in, asked for at each invocation
/// ([`merge::worktree_root`](crate::commands::merge::worktree_root)) — never a path baked into
/// the config. A baked root is wrong the moment a second worktree exists: every `git diff` in
/// the new checkout would resolve against the original one, showing its documents and opening
/// its index. `--root DIR` still overrides, for a caller that knows better.
pub fn run_textconv(args: &Args) -> Result<String, String> {
    let file = args
        .positional(0)
        .ok_or_else(|| "dx textconv needs a file".to_string())?;
    let path = Path::new(file);
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;

    let root = args
        .value("root")
        .map_or_else(crate::commands::merge::worktree_root, PathBuf::from);
    workspace::resolve_contents(&text, &root)
}

/// `dx git-setup` — teach git to diff `.dx` pointers as documents.
///
/// Writes the `.gitattributes` entry and the local `diff.dx.textconv` config. Without this,
/// `git diff` on a pointer shows one changed digest line; with it, the real document diff.
/// Also appends the `.gitignore` lines that keep the local index out of the repository and the
/// committed pack in.
pub fn run_git_setup(args: &Args) -> Result<String, String> {
    let root = workspace::workspace_root(&target_directory(args));
    let mut out = format!("dx git-setup — {}\n\n", root.display());
    if !root.join(".git").exists() {
        out.push_str("  nothing to do; this is not a git repository\n");
        return Ok(out);
    }

    // The command a person ran on purpose reports the true state, so it does not consult the
    // once-per-process mark the automatic path keeps.
    let mut said = prepare_git(&root);
    said.extend(untrack_machine_local_files(&root));
    if said.is_empty() {
        out.push_str("  present   this repository already diffs and merges dx documents\n");
    }
    for line in said {
        out.push_str(&format!("  {line}\n"));
    }

    out.push_str(
        "\nNow `git diff`, `git show`, and `git log -p` render .dx documents, and two \
         branches\nthat both changed documents merge block by block instead of colliding.\n",
    );
    Ok(out)
}

/// Teach `root`'s repository to diff and merge dx documents, and say what it wrote.
///
/// # Why this is not only a command
/// It used to be only `dx git-setup`, run by hand, once, by whoever thought of it — and every
/// fresh clone, every `git worktree add`, and every agent that started in a checkout nobody
/// had prepared got the untaught behavior: pointer conflicts on a hex line and a pack git
/// calls binary. The repository is the thing that needs teaching, not the person, so every
/// write path calls this and it is cheap enough to mean it.
///
/// # Why it never touches git's index
/// This runs inside `dx set`, `dx sync`, and the editors' saves. Staging or unstaging a file
/// from under a person mid-edit is not a thing a save may do, so this writes only
/// configuration and two text files, and is a no-op the second time. Untracking what should
/// never have been committed is [`untrack_machine_local_files`], and stays in the command a
/// person runs on purpose.
///
/// Returns one line per thing written; empty when the repository was already ready. A
/// directory that is not a git repository gets nothing at all.
pub fn ensure_git_ready(root: &Path) -> Vec<String> {
    if !root.join(".git").exists() || !mark_ensured(root) {
        return Vec::new();
    }
    prepare_git(root)
}

/// Write everything [`ensure_git_ready`] promises, without consulting its mark.
fn prepare_git(root: &Path) -> Vec<String> {
    let mut wrote = Vec::new();
    let attributes = root.join(".gitattributes");
    // `*.dx diff=dx` is what earlier versions wrote, so the line is *upgraded* in place
    // rather than appended beside — two lines matching the same pattern would leave which
    // driver wins up to git's ordering rules.
    if ensure_attribute(&attributes, "*.dx", ATTRIBUTES_LINE).unwrap_or(false) {
        wrote.push(format!("wrote     {ATTRIBUTES_LINE}  (.gitattributes)"));
    }
    if ensure_attribute(&attributes, pack::REPO_PACK, PACK_ATTRIBUTES_LINE).unwrap_or(false) {
        wrote.push(format!(
            "wrote     {PACK_ATTRIBUTES_LINE}  (.gitattributes)"
        ));
    }

    let ignore = root.join(".gitignore");
    let mut ignored = 0;
    for line in pack::gitignore_lines().lines() {
        if ensure_line(&ignore, line).unwrap_or(false) {
            ignored += 1;
        }
    }
    if ignored > 0 {
        wrote.push(format!(
            "wrote     {ignored} .gitignore line(s) for the machine-local files"
        ));
    }

    for (key, value) in [
        (TEXTCONV_KEY, textconv_command()),
        (MERGE_NAME_KEY, MERGE_NAME.to_string()),
        (MERGE_DRIVER_KEY, merge_driver_command()),
    ] {
        if git_config(root, key).as_deref() == Some(value.as_str()) {
            continue;
        }
        match set_git_config(root, key, &value) {
            Ok(()) => wrote.push(format!("wrote     git config {key}={value}")),
            Err(error) => wrote.push(format!("skipped   git config {key}: {error}")),
        }
    }
    wrote
}

/// Whether this process has yet to prepare `root`, marking it prepared.
///
/// A save is a hot path and this is the same answer every time within one run, so the git
/// calls happen once per workspace per process rather than once per document written.
fn mark_ensured(root: &Path) -> bool {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    static ENSURED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
    ENSURED
        .lock()
        .map_or(true, |mut seen| seen.insert(root.to_path_buf()))
}

/// Stop tracking the files that are this machine's, not the repository's.
///
/// `.doc/index.db` is the one that hurts: it is a SQLite file, rebuildable from the pack, and
/// once committed it conflicts on **every** merge between branches that both wrote documents —
/// a binary conflict no resolution can be correct for, since each side's database describes
/// its own machine. Committing it also publishes `.doc/local.dxcp`'s scratch work in some
/// checkouts. The content survives untouched on disk; only git forgets it.
fn untrack_machine_local_files(root: &Path) -> Vec<String> {
    let mut said = Vec::new();
    for relative in [
        ".doc/index.db",
        ".doc/index.db-wal",
        ".doc/index.db-shm",
        pack::LOCAL_PACK,
        ".doc/coverage.jsonl",
    ] {
        if !in_the_index(root, relative) {
            continue;
        }
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rm", "--cached", "-q", "--", relative])
            .output();
        match output {
            Ok(done) if done.status.success() => said.push(format!(
                "untracked {relative}  (still on disk; commit the removal)"
            )),
            Ok(done) => said.push(format!(
                "skipped   could not untrack {relative}: {}",
                String::from_utf8_lossy(&done.stderr).trim()
            )),
            Err(error) => said.push(format!("skipped   could not untrack {relative}: {error}")),
        }
    }
    said
}

/// Whether git's index carries `relative` — committed, not merely present on disk.
///
/// This asks the index rather than `.gitignore`: a file that was committed *before* the ignore
/// line existed stays tracked forever, and that is exactly the case being repaired here.
pub(crate) fn in_the_index(root: &Path, relative: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", "-z", "--", relative])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The command git should run to expand a pointer.
///
/// The **absolute path of the running binary** is pinned, not the bare word `dx`: git runs the
/// driver with whatever environment it happens to have, and a different or older `dx` earlier
/// on `PATH` answers with a usage error instead of the document — which git reports only as
/// `unable to read files to diff`.
///
/// The workspace root is deliberately *not* pinned. It used to be, because git hands the driver
/// a temporary blob copy from under the system temp directory and searching from there finds no
/// workspace; but a root in the config is a root for every checkout that shares the config, so
/// `git diff` inside a linked worktree resolved against — and opened the index of — the
/// original checkout. The driver asks git where it is instead, per invocation.
fn textconv_command() -> String {
    format!("{} textconv", quoted_exe())
}

/// The command git should run to merge a path it cannot merge itself.
///
/// `%O %A %B %P %L` are git's placeholders: the ancestor, our side (which is also where the
/// result goes), their side, the path in the working tree, and the conflict-marker size.
fn merge_driver_command() -> String {
    format!(
        "{} merge-driver --ancestor %O --ours %A --theirs %B --path %P --marker-size %L",
        quoted_exe()
    )
}

/// This binary's absolute path, quoted for the shell git runs a driver through.
fn quoted_exe() -> String {
    std::env::current_exe().map_or_else(
        |_| "dx".to_string(),
        |exe| shell_quote(&exe.to_string_lossy()),
    )
}

/// The value of a repository-local git config key, when it is set.
fn git_config(root: &Path, key: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
    )
}

/// Write `line` into `.gitattributes` for `pattern`, replacing whatever it said before.
///
/// Reports whether the file changed. An existing line for the same pattern is rewritten in
/// place rather than appended beside, so upgrading a repository that only knew about `diff=dx`
/// leaves one line, not two that disagree.
fn ensure_attribute(path: &Path, pattern: &str, line: &str) -> Result<bool, String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut replaced = false;
    let mut lines: Vec<String> = Vec::new();
    for found in existing.lines() {
        if found.split_whitespace().next() == Some(pattern) {
            replaced = true;
            lines.push(line.to_string());
        } else {
            lines.push(found.to_string());
        }
    }
    if replaced {
        if existing.lines().eq(lines.iter().map(String::as_str)) {
            return Ok(false);
        }
    } else {
        lines.push(line.to_string());
    }

    let mut updated = lines.join("\n");
    updated.push('\n');
    std::fs::write(path, updated)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(true)
}

/// Quote a path for the shell git invokes the driver through, if it needs it.
fn shell_quote(path: &str) -> String {
    let safe = path
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+'));
    if safe {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

/// Append `line` to `path` unless it is already there, reporting whether it was written.
fn ensure_line(path: &Path, line: &str) -> Result<bool, String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.lines().any(|found| found.trim() == line.trim()) {
        return Ok(false);
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(line);
    updated.push('\n');
    std::fs::write(path, updated)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(true)
}

/// Set a repository-local git config value.
fn set_git_config(root: &Path, key: &str, value: &str) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", key, value])
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-store-cmd-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".doc")).expect("scratch");
        std::fs::canonicalize(&root).expect("canonical")
    }

    const NOTES: &str =
        "::heading level=1 id=n\nNotes\n::end\n\n::paragraph id=p\nA line.\n::end\n";

    #[test]
    fn textconv_prints_the_document_a_pointer_stands_for() {
        let root = scratch("textconv");
        let path = root.join("notes.dx");
        workspace::save(&path, &parse(NOTES)).expect("save");

        // The file itself is a pointer …
        assert!(doc_store::stub::is_stub(
            &std::fs::read_to_string(&path).expect("read")
        ));
        // … and textconv is what git will show instead.
        let shown = run_textconv(&args(&[
            &path.to_string_lossy(),
            "--root",
            &root.to_string_lossy(),
        ]))
        .expect("textconv");
        assert_eq!(shown, NOTES);
    }

    #[test]
    fn textconv_passes_plain_text_straight_through() {
        let root = scratch("textconv-plain");
        let path = root.join("plain.dx");
        std::fs::write(&path, NOTES).expect("write");
        assert_eq!(
            run_textconv(&args(&[&path.to_string_lossy()])).expect("textconv"),
            NOTES
        );
    }

    #[test]
    fn textconv_resolves_a_blob_copied_outside_the_workspace() {
        // This is exactly what git does: extract the blob to a temporary file, then run the
        // driver on that. Resolution must go by the digest inside, not by where the file sits.
        let root = scratch("textconv-blob");
        workspace::save(&root.join("notes.dx"), &parse(NOTES)).expect("save");

        let elsewhere = scratch("textconv-blob-tmp").join("git-blob-copy.dx");
        std::fs::copy(root.join("notes.dx"), &elsewhere).expect("copy the pointer away");

        let shown = run_textconv(&args(&[
            &elsewhere.to_string_lossy(),
            "--root",
            &root.to_string_lossy(),
        ]))
        .expect("textconv");
        assert_eq!(shown, NOTES);
    }

    #[test]
    fn textconv_resolves_an_earlier_version_of_a_document() {
        // `git log -p` asks for old blobs; each names a version the store still holds.
        let root = scratch("textconv-history");
        let path = root.join("notes.dx");
        workspace::save(&path, &parse(NOTES)).expect("first");
        let old_pointer = std::fs::read_to_string(&path).expect("read pointer");

        workspace::save(&path, &parse("::paragraph id=p\nrewritten\n::end\n")).expect("second");

        let old_blob = root.join(".doc").join("old-blob.dx");
        std::fs::write(&old_blob, &old_pointer).expect("stage old blob");
        let shown = run_textconv(&args(&[
            &old_blob.to_string_lossy(),
            "--root",
            &root.to_string_lossy(),
        ]))
        .expect("textconv");
        assert_eq!(shown, NOTES, "the old revision should still render");
    }

    #[test]
    fn textconv_without_a_file_says_so() {
        assert!(run_textconv(&args(&[]))
            .unwrap_err()
            .contains("needs a file"));
    }

    #[test]
    fn sync_reports_what_it_adopted() {
        let root = scratch("sync");
        std::fs::write(root.join("found.dx"), NOTES).expect("write");
        let report = run_sync(&args(&[&root.to_string_lossy()])).expect("sync");
        assert!(report.contains("adopted from plain text"), "{report}");
        assert!(report.contains("found.dx"), "{report}");
    }

    #[test]
    fn sync_on_a_settled_workspace_says_there_is_nothing_to_do() {
        let root = scratch("sync-clean");
        workspace::save(&root.join("notes.dx"), &parse(NOTES)).expect("save");
        run_sync(&args(&[&root.to_string_lossy()])).expect("first");
        let second = run_sync(&args(&[&root.to_string_lossy()])).expect("second");
        assert!(second.contains("nothing to do"), "{second}");
    }

    #[test]
    fn sync_refuses_to_adopt_a_document_a_merge_left_conflicted() {
        // The markers are ordinary body lines as far as the format is concerned, so adopting
        // the file would store both branches' words as one document and quietly lose the fact
        // that a person still has to choose.
        let root = scratch("sync-conflicted");
        let conflicted = "::paragraph id=p\n<<<<<<< ours\nmine\n=======\ntheirs\n>>>>>>> \
                          theirs\n::end\n";
        std::fs::write(root.join("notes.dx"), conflicted).expect("write");

        let report = run_sync(&args(&[&root.to_string_lossy()])).expect("sync");
        assert!(report.contains("CONFLICTED"), "{report}");
        assert!(report.contains("notes.dx"), "{report}");
        assert!(report.contains("dx sync` again"), "{report}");
        assert_eq!(
            std::fs::read_to_string(root.join("notes.dx")).expect("read"),
            conflicted,
            "the file must be left exactly as the merge wrote it"
        );

        // And the moment the markers are gone it adopts normally.
        std::fs::write(root.join("notes.dx"), NOTES).expect("resolve by hand");
        let after = run_sync(&args(&[&root.to_string_lossy()])).expect("sync");
        assert!(after.contains("adopted from plain text"), "{after}");
    }

    #[test]
    fn sync_flags_a_pointer_it_cannot_resolve() {
        let root = scratch("sync-orphan");
        std::fs::write(root.join("orphan.dx"), doc_store::stub::render(NOTES)).expect("write");
        let report = run_sync(&args(&[&root.to_string_lossy()])).expect("sync");
        assert!(report.contains("UNRESOLVED"), "{report}");
        assert!(report.contains("orphan.dx"), "{report}");
        // And it says what to do about it rather than just naming the problem.
        assert!(report.contains("restore .doc/repo.dxcp"), "{report}");
    }

    #[test]
    fn stats_reports_sharing_and_compaction() {
        let root = scratch("stats");
        let shared = "::paragraph id=p\nthe very same paragraph in both\n::end\n";
        workspace::save(&root.join("a.dx"), &parse(shared)).expect("a");
        workspace::save(&root.join("b.dx"), &parse(shared)).expect("b");

        let report = run_stats(&args(&[&root.to_string_lossy()])).expect("stats");
        assert!(report.contains("documents        2"), "{report}");
        assert!(report.contains("blocks shared    1"), "{report}");
        assert!(report.contains("stored / source"), "{report}");
    }

    #[test]
    fn stats_on_an_empty_workspace_is_not_an_error() {
        let root = scratch("stats-empty");
        let report = run_stats(&args(&[&root.to_string_lossy()])).expect("stats");
        assert!(report.contains("no documents stored yet"), "{report}");
    }

    /// A scratch workspace that is also a real git repository, which is what the setup path
    /// is about — a directory with no `.git` is deliberately left alone.
    fn scratch_repo(label: &str) -> PathBuf {
        let root = scratch(label);
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .output()
            .expect("git init");
        assert!(
            done.status.success(),
            "git init failed in {}",
            root.display()
        );
        root
    }

    #[test]
    fn git_setup_writes_the_attributes_and_ignore_lines_once() {
        let root = scratch_repo("git-setup");
        let first = run_git_setup(&args(&[&root.to_string_lossy()])).expect("setup");
        assert!(first.contains(".gitattributes"), "{first}");

        let attributes = std::fs::read_to_string(root.join(".gitattributes")).expect("read");
        assert!(attributes.contains(ATTRIBUTES_LINE), "{attributes}");
        assert!(attributes.contains(PACK_ATTRIBUTES_LINE), "{attributes}");
        let ignore = std::fs::read_to_string(root.join(".gitignore")).expect("read");
        assert!(ignore.contains("index.db"));
        assert!(ignore.contains(pack::LOCAL_PACK));
        assert!(
            !ignore.contains(&format!("\n{}\n", pack::REPO_PACK)),
            "the committed pack must not be ignored: {ignore}"
        );

        // Running again must not duplicate anything, and must say so rather than repeating.
        let again = run_git_setup(&args(&[&root.to_string_lossy()])).expect("again");
        assert!(again.contains("already diffs and merges"), "{again}");
        let attributes_again = std::fs::read_to_string(root.join(".gitattributes")).expect("read");
        assert_eq!(attributes, attributes_again);
    }

    #[test]
    fn git_setup_configures_the_merge_driver_git_will_actually_call() {
        // Without this config the `merge=dx` attribute names a driver that does not exist and
        // git silently falls back to the built-in one — which is where the binary conflicts
        // came from.
        let root = scratch_repo("git-setup-merge");
        run_git_setup(&args(&[&root.to_string_lossy()])).expect("setup");
        let driver = git_config(&root, MERGE_DRIVER_KEY).expect("the driver is configured");
        assert!(driver.contains("merge-driver"), "{driver}");
        for placeholder in ["%O", "%A", "%B", "%P", "%L"] {
            assert!(driver.contains(placeholder), "{driver} lacks {placeholder}");
        }
        assert_eq!(
            git_config(&root, MERGE_NAME_KEY).as_deref(),
            Some(MERGE_NAME)
        );
    }

    #[test]
    fn a_repository_that_only_knew_about_diff_is_upgraded_in_place() {
        // Every workspace set up before merging existed carries `*.dx diff=dx`. A second line
        // beside it would leave which driver wins to git's ordering rules.
        let root = scratch_repo("git-setup-upgrade");
        std::fs::write(root.join(".gitattributes"), "*.dx diff=dx\n").expect("old line");
        run_git_setup(&args(&[&root.to_string_lossy()])).expect("setup");

        let attributes = std::fs::read_to_string(root.join(".gitattributes")).expect("read");
        assert_eq!(
            attributes
                .lines()
                .filter(|line| line.starts_with("*.dx"))
                .count(),
            1,
            "{attributes}"
        );
        assert!(attributes.contains("merge=dx"), "{attributes}");
    }

    #[test]
    fn git_setup_untracks_the_index_a_repository_committed_by_mistake() {
        // A committed .doc/index.db conflicts on every merge between branches that both wrote
        // documents, and no resolution of it can be right — each side's database is its own
        // machine's. The file stays on disk; only git forgets it.
        let root = scratch_repo("git-setup-untrack");
        std::fs::write(root.join(".doc").join("index.db"), b"pretend sqlite").expect("write");
        let added = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "-f", "--", ".doc/index.db"])
            .output()
            .expect("git add");
        assert!(added.status.success());
        assert!(in_the_index(&root, ".doc/index.db"));

        let report = run_git_setup(&args(&[&root.to_string_lossy()])).expect("setup");
        assert!(report.contains("untracked .doc/index.db"), "{report}");
        assert!(!in_the_index(&root, ".doc/index.db"));
        assert!(root.join(".doc").join("index.db").exists(), "still on disk");
    }

    #[test]
    fn setting_up_a_directory_that_is_not_a_repository_writes_nothing() {
        let root = scratch("git-setup-not-a-repo");
        let report = run_git_setup(&args(&[&root.to_string_lossy()])).expect("setup");
        assert!(report.contains("not a git repository"), "{report}");
        assert!(!root.join(".gitattributes").exists());
    }

    #[test]
    fn the_diff_driver_points_at_this_binary_and_bakes_in_no_workspace() {
        // Two failures in one line. A stale `dx` earlier on PATH answers `textconv` with a
        // usage error, which git reports only as "unable to read files to diff" — so the
        // running executable is named. And a baked `--root` makes every `git diff` in a
        // second worktree resolve against the first checkout, opening its index; so the root
        // is asked for per invocation instead.
        let command = textconv_command();
        assert!(command.ends_with(" textconv"), "{command}");
        assert!(!command.contains("--root"), "{command}");
        if let Ok(exe) = std::env::current_exe() {
            assert!(
                command.contains(exe.to_string_lossy().trim()),
                "expected the running binary in {command}"
            );
            assert!(
                command.starts_with('/') || command.starts_with('\''),
                "{command}"
            );
        }
    }

    #[test]
    fn a_path_needing_quotes_gets_them() {
        assert_eq!(shell_quote("/usr/local/bin/dx"), "/usr/local/bin/dx");
        assert_eq!(shell_quote("/has space/dx"), "'/has space/dx'");
        assert_eq!(shell_quote("/it's/dx"), r"'/it'\''s/dx'");
    }

    #[test]
    fn ensure_line_appends_to_a_file_with_no_trailing_newline() {
        let root = scratch("ensure-line");
        let path = root.join("file");
        std::fs::write(&path, "first").expect("write");
        assert!(ensure_line(&path, "second").expect("append"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "first\nsecond\n"
        );
        assert!(!ensure_line(&path, "second").expect("idempotent"));
    }
}
