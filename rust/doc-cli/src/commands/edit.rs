//! Writing commands: `new`, `fmt`, `set`, `append`, `insert`, and `remove`.
//!
//! Editing a `.dx` file with an ordinary text editor is always allowed and always safe —
//! these commands exist because *targeted* edits are safer still. `dx set` replaces one
//! block by id without touching a byte of the rest, which is what lets an agent change a
//! paragraph in a long document without rewriting (and risking) the whole file.
//!
//! They are also the whole vocabulary the editing surfaces speak. Clicking a paragraph in
//! DX.app and typing into it is `dx source` then `dx set`; pressing Return at the end of it
//! is `dx insert`; emptying a block and pressing Backspace is `dx remove`. A person editing
//! a page and an agent editing a document are doing the identical thing through the identical
//! commands, which is the only way the two can be trusted to agree.

use std::path::{Path, PathBuf};

use doc_core::edit;
use doc_core::format::{parse, stringify};
use doc_core::model::Block;

use crate::args::Args;
use crate::workspace;

/// Block kinds `dx append` accepts — the format's own authorable kinds, named once in
/// `doc-core` so the command line and every editing surface agree on what may be written.
const APPENDABLE: &[&str] = edit::AUTHORABLE;

/// `dx new <file>` — create a document with a title heading and an opening paragraph.
pub fn run_new(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a path for the new document is required")?;
    if path.exists() && !args.present("force") {
        return Err(format!(
            "{} already exists — pass --force to overwrite it",
            path.display()
        ));
    }

    let title = args
        .value("title")
        .map(str::to_string)
        .unwrap_or_else(|| default_title(&path));
    let document = parse(&format!(
        "::heading level=1 id=title\n{title}\n::end\n\n::paragraph id=intro\nStart writing here.\n::end\n"
    ));
    workspace::save(&path, &document)?;
    Ok(format!("created {}\n", path.display()))
}

/// Turn a file name into a readable default title: `release-notes.dx` → `Release notes`.
fn default_title(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string());
    let words = stem.replace(['-', '_'], " ");
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => "Untitled".to_string(),
    }
}

/// `dx fmt <file...>` — rewrite each document in canonical form.
pub fn run_fmt(args: &Args) -> Result<String, String> {
    let paths = args.positionals();
    if paths.is_empty() {
        return Err("at least one .dx file is required".to_string());
    }

    let mut report = String::new();
    for raw in paths {
        let path = PathBuf::from(raw);
        let before = workspace::read(&path)?;
        let after = stringify(&parse(&before));
        if before == after {
            report.push_str(&format!("unchanged  {}\n", path.display()));
            continue;
        }
        if args.present("check") {
            report.push_str(&format!("would fix  {}\n", path.display()));
            continue;
        }
        workspace::write_text(&path, &after)?;
        report.push_str(&format!("formatted  {}\n", path.display()));
    }
    Ok(report)
}

/// `dx set <file> <block-id>` — replace one block's body, leaving every other block alone.
pub fn run_set(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let id = args
        .positional(1)
        .ok_or_else(|| "a block id is required — see `dx outline <file>`".to_string())?;
    let body = body_argument(args)?;

    let updated = edit::set_block(&workspace::read(&path)?, id, &body)?;
    workspace::save(&path, &parse(&updated))?;
    Ok(format!("updated `{id}` in {}\n", path.display()))
}

/// `dx append <file>` — add a block to the end of a document.
pub fn run_append(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let kind = args.value("type").unwrap_or("paragraph").to_string();
    if !APPENDABLE.contains(&kind.as_str()) {
        return Err(format!(
            "cannot append a `{kind}` block. Supported: {}",
            APPENDABLE.join(", ")
        ));
    }
    let body = body_argument(args)?;

    let mut document = parse(&workspace::read(&path)?);
    let mut block = Block {
        kind,
        id: args.value("id").unwrap_or_default().to_string(),
        language: args.value("lang").unwrap_or_default().to_string(),
        level: args.number("level").unwrap_or(2) as u8,
        run: args.present("run"),
        deps: args.value("deps").unwrap_or_default().to_string(),
        ..Block::default()
    };
    edit::set_body(&mut block, &body);
    document.blocks.push(block);

    workspace::save(&path, &document)?;
    Ok(format!("appended to {}\n", path.display()))
}

/// `dx source <file> --block ID` — the exact characters of one block, and nothing else.
///
/// This is what an editing surface puts in the field when a reader clicks a paragraph: not
/// the rendered HTML, which has already lost the difference between the text and the way it
/// is drawn, but the text the author actually wrote.
///
/// Reading only. With no `--block` it prints the whole document's canonical source.
pub fn run_source(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let source = workspace::read(&path)?;

    match args.value("block").or_else(|| args.positional(1)) {
        Some(id) => edit::block_source(&source, id),
        None => Ok(stringify(&parse(&source))),
    }
}

/// `dx insert <file> --after ID` — add a block directly after another one.
///
/// `dx append` can only reach the end of a document, which makes it useless for the one
/// thing a person writing a page does constantly: press Return in the middle and keep going.
/// The new block's id is printed so the caller can put the cursor in it.
pub fn run_insert(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let (updated, id) = edit::insert_after(
        &workspace::read(&path)?,
        args.value("after"),
        args.value("type").unwrap_or("paragraph"),
        args.value("text").unwrap_or_default(),
    )?;
    workspace::save(&path, &parse(&updated))?;
    Ok(format!("{id}\n"))
}

/// `dx remove <file> <block-id>` — take one block out of a document.
pub fn run_remove(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let id = args
        .value("block")
        .or_else(|| args.positional(1))
        .ok_or_else(|| "a block id is required — see `dx outline <file>`".to_string())?;

    let updated = edit::remove_block(&workspace::read(&path)?, id)?;
    workspace::save(&path, &parse(&updated))?;
    Ok(format!("removed `{id}` from {}\n", path.display()))
}

/// Read the new block body from `--text`, `--from <file>`, or standard input.
fn body_argument(args: &Args) -> Result<String, String> {
    if let Some(text) = args.value("text") {
        if text != "-" {
            return Ok(text.to_string());
        }
    }
    if let Some(file) = args.value("from") {
        return workspace::read(Path::new(file));
    }
    workspace::read(Path::new("-"))
}

/// Read a required path from positional `index`.
fn path_argument(args: &Args, index: usize, message: &str) -> Result<PathBuf, String> {
    args.positional(index)
        .map(PathBuf::from)
        .ok_or_else(|| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-edit-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn new_creates_a_resolvable_document_and_refuses_to_clobber() {
        let path = scratch("new").join("release-notes.dx");
        let file = path.to_string_lossy().into_owned();
        run_new(&args(&[&file])).expect("create");

        // On disk: a pointer. Through the resolver: the document.
        assert!(doc_store::stub::is_stub(
            &std::fs::read_to_string(&path).expect("read")
        ));
        let resolved = workspace::read(&path).expect("resolve");
        assert!(resolved.contains("Release notes"));
        assert!(resolved.starts_with("::heading level=1"));

        assert!(run_new(&args(&[&file])).unwrap_err().contains("--force"));
        run_new(&args(&[&file, "--force", "--title", "Renamed"])).expect("force");
        assert!(workspace::read(&path).expect("resolve").contains("Renamed"));
    }

    #[test]
    fn set_replaces_one_block_and_leaves_the_others_byte_identical() {
        let path = scratch("set").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(
            &path,
            "::heading level=1 id=h\nTitle\n::end\n\n::paragraph id=body\nOld text\n::end\n\n::paragraph id=tail\nKeep me\n::end\n",
        )
        .expect("seed");

        run_set(&args(&[&file, "body", "--text", "New text"])).expect("set");

        let raw = workspace::read(&path).expect("resolve");
        assert!(raw.contains("New text"));
        assert!(!raw.contains("Old text"));
        assert!(raw.contains("::paragraph id=tail\nKeep me\n::end"));
        assert!(raw.contains("::heading level=1 id=h\nTitle\n::end"));
    }

    #[test]
    fn set_on_a_missing_block_lists_the_real_ids() {
        let path = scratch("set-missing").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=only\nx\n::end\n").expect("seed");
        let error = run_set(&args(&[&file, "nope", "--text", "y"])).expect_err("should fail");
        assert!(error.contains("no block named `nope`"));
        assert!(error.contains("only"));
    }

    #[test]
    fn append_adds_typed_blocks_including_runnable_code() {
        let path = scratch("append").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nstart\n::end\n").expect("seed");

        run_append(&args(&[
            &file, "--type", "code", "--lang", "python", "--run", "--text", "print(1)",
        ]))
        .expect("append code");
        run_append(&args(&[
            &file,
            "--type",
            "bulleted-list",
            "--text",
            "- one\n- two",
        ]))
        .expect("append list");

        let raw = workspace::read(&path).expect("resolve");
        assert!(raw.contains("::code id=code-2 lang=python run"));
        assert!(raw.contains("- one\n- two"));
    }

    #[test]
    fn append_rejects_a_block_type_the_format_does_not_have() {
        let path = scratch("append-bad").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nx\n::end\n").expect("seed");
        let error = run_append(&args(&[&file, "--type", "widget", "--text", "x"]))
            .expect_err("should fail");
        assert!(error.contains("cannot append"));
    }

    #[test]
    fn fmt_canonicalizes_and_check_reports_without_writing() {
        let path = scratch("fmt").join("messy.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::heading level=2 id=h Hello ::end\n").expect("seed");

        let checked = run_fmt(&args(&[&file, "--check"])).expect("check");
        assert!(checked.contains("would fix"));
        assert!(std::fs::read_to_string(&path)
            .expect("read")
            .contains("Hello ::end"));

        let fixed = run_fmt(&args(&[&file])).expect("fmt");
        assert!(fixed.contains("formatted"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "::heading level=2 id=h\nHello\n::end\n"
        );
        assert!(run_fmt(&args(&[&file]))
            .expect("fmt again")
            .contains("unchanged"));
    }
}
