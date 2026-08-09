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
/// The `.gitattributes` line that points `.dx` files at the driver.
const ATTRIBUTES_LINE: &str = "*.dx diff=dx";

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
    if report.is_clean() {
        out.push_str("\n  nothing to do; every document already resolves\n");
        return Ok(out);
    }

    let sections: [(&str, &Vec<String>); 5] = [
        ("adopted from plain text", &report.ingested),
        ("restored from a pack", &report.restored),
        ("pointer rewritten", &report.stubs_written),
        ("pruned (file deleted from the tree)", &report.pruned),
        ("UNRESOLVED", &report.unresolved),
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
    if !report.unresolved.is_empty() {
        out.push_str(
            "\nThe unresolved pointers above have no content in .doc/. Restore \
             .doc/repo.dxcp from version control, or delete the pointers if the documents \
             are gone.\n",
        );
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
/// resolution goes by the digest inside the pointer rather than by path. `--root DIR` names the
/// workspace to resolve against; without it the current directory is used, which is where git
/// runs the driver from. [`run_git_setup`] bakes an absolute `--root` into the config so the
/// driver does not depend on that.
pub fn run_textconv(args: &Args) -> Result<String, String> {
    let file = args
        .positional(0)
        .ok_or_else(|| "dx textconv needs a file".to_string())?;
    let path = Path::new(file);
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;

    let root = args.value("root").map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    );
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

    let attributes = root.join(".gitattributes");
    if ensure_line(&attributes, ATTRIBUTES_LINE)? {
        out.push_str(&format!(
            "  wrote     {ATTRIBUTES_LINE}  (.gitattributes)\n"
        ));
    } else {
        out.push_str("  present   .gitattributes already routes *.dx to the dx driver\n");
    }

    let ignore = root.join(".gitignore");
    let mut ignored = 0;
    for line in pack::gitignore_lines().lines() {
        if ensure_line(&ignore, line)? {
            ignored += 1;
        }
    }
    out.push_str(&format!(
        "  {:<9} {ignored} .gitignore line(s) for the local index\n",
        if ignored > 0 { "wrote" } else { "present" }
    ));

    let command = textconv_command(&root);
    match set_git_config(&root, TEXTCONV_KEY, &command) {
        Ok(()) => out.push_str(&format!(
            "  wrote     git config {TEXTCONV_KEY}={command}\n"
        )),
        Err(error) => out.push_str(&format!("  skipped   git config: {error}\n")),
    }

    out.push_str("\nNow `git diff`, `git show`, and `git log -p` render .dx documents.\n");
    Ok(out)
}

/// The command git should run to expand a pointer.
///
/// Two things are pinned here, and both were needed to make the driver actually work:
///
/// - The **absolute path of the running binary**, not the bare word `dx`. Git runs the driver
///   with whatever environment it happens to have, and a different or older `dx` earlier on
///   `PATH` answers with a usage error instead of the document — which git reports only as
///   `unable to read files to diff`.
/// - The **absolute workspace root**, because git hands the driver a temporary blob copy from
///   somewhere under the system temp directory. Searching for a workspace from there finds
///   nothing.
fn textconv_command(root: &Path) -> String {
    let root = shell_quote(&root.to_string_lossy());
    std::env::current_exe().map_or_else(
        |_| format!("dx textconv --root {root}"),
        |exe| {
            format!(
                "{} textconv --root {root}",
                shell_quote(&exe.to_string_lossy())
            )
        },
    )
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
    fn sync_flags_a_pointer_it_cannot_resolve() {
        let root = scratch("sync-orphan");
        std::fs::write(root.join("orphan.dx"), doc_store::stub::render(NOTES)).expect("write");
        let report = run_sync(&args(&[&root.to_string_lossy()])).expect("sync");
        assert!(report.contains("UNRESOLVED"), "{report}");
        assert!(report.contains("orphan.dx"), "{report}");
        // And it says what to do about it rather than just naming the problem.
        assert!(report.contains("Restore .doc/repo.dxcp"), "{report}");
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

    #[test]
    fn git_setup_writes_the_attributes_and_ignore_lines_once() {
        let root = scratch("git-setup");
        let first = run_git_setup(&args(&[&root.to_string_lossy()])).expect("setup");
        assert!(first.contains(".gitattributes"), "{first}");

        let attributes = std::fs::read_to_string(root.join(".gitattributes")).expect("read");
        assert!(attributes.contains(ATTRIBUTES_LINE));
        let ignore = std::fs::read_to_string(root.join(".gitignore")).expect("read");
        assert!(ignore.contains("index.db"));
        assert!(ignore.contains(pack::LOCAL_PACK));
        assert!(
            !ignore.contains(&format!("\n{}\n", pack::REPO_PACK)),
            "the committed pack must not be ignored: {ignore}"
        );

        // Running again must not duplicate anything.
        run_git_setup(&args(&[&root.to_string_lossy()])).expect("again");
        let attributes_again = std::fs::read_to_string(root.join(".gitattributes")).expect("read");
        assert_eq!(attributes, attributes_again);
    }

    #[test]
    fn the_diff_driver_points_at_this_binary_not_whatever_is_on_path() {
        // A stale `dx` earlier on PATH answers `textconv` with a usage error, and git reports
        // "unable to read files to diff". The driver must name the running executable.
        let command = textconv_command(Path::new("/tmp/some-workspace"));
        assert!(
            command.contains(" textconv --root /tmp/some-workspace"),
            "{command}"
        );
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
