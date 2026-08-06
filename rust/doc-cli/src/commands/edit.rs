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

use crate::args::Args;
use crate::workspace;

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
        workspace::save_source(&path, &after)?;
        report.push_str(&format!("formatted  {}\n", path.display()));
    }
    Ok(report)
}

/// `dx set <file> <block-id>` — replace one block's body, leaving every other block alone.
///
/// With `--header` the whole block is replaced, header and all — `--header "::quote"`
/// retypes a paragraph into a quotation, and `--header ""` says the body is plain text,
/// read the way the file itself would read it. This is [`edit::replace_block`], the same
/// operation the editing surfaces perform when a reader rewrites the tag line above a
/// block, so a person and an agent retype a block through one implementation.
pub fn run_set(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let id = args
        .positional(1)
        .ok_or_else(|| "a block id is required — see `dx outline <file>`".to_string())?;
    refuse_extra_positionals(args, 2)?;
    let body = body_argument(args)?;

    if args.present("header") {
        let header = args.value("header").unwrap_or_default();
        let (updated, focus) = edit::replace_block(&workspace::read(&path)?, id, header, &body)?;
        workspace::save(&path, &parse(&updated))?;
        // The replacement's id, bare, the way `dx insert` answers: it is the block the
        // caller's cursor belongs in next, and a surface reads it straight off stdout.
        return Ok(format!("{focus}\n"));
    }

    let updated = edit::set_block(&workspace::read(&path)?, id, &body)?;
    workspace::save(&path, &parse(&updated))?;
    Ok(format!("updated `{id}` in {}\n", path.display()))
}

/// `dx append <file>` — add a block to the end of a document.
///
/// The same operation as `dx insert`, anchored on the last block — one engine, one rule
/// set: the new block's id is stable against the document's existing ids, and an explicit
/// `--id` some block already answers to is refused with a sentence, never silently renamed.
pub fn run_append(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    refuse_extra_positionals(args, 1)?;
    let kind = args.value("type").unwrap_or("paragraph");
    require_code_for_attributes(kind, args)?;
    let body = body_argument(args)?;

    let source = workspace::read(&path)?;
    let insertion = edit::Insertion {
        kind,
        body: &body,
        id: args.value("id").unwrap_or_default(),
        level: args.number("level").map_or(0, |level| level as u8),
        language: args.value("lang").unwrap_or_default(),
        run: args.present("run"),
        deps: args.value("deps").unwrap_or_default(),
    };
    let last = parse(&source).blocks.last().map(|block| block.id.clone());
    let (updated, _) = edit::insert_after(&source, last.as_deref(), &insertion)?;
    workspace::save(&path, &parse(&updated))?;
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
        Some(id) if args.present("header") => {
            edit::block_header(&source, id).map(|header| format!("{header}\n"))
        }
        Some(id) => edit::block_source(&source, id),
        None => Ok(stringify(&parse(&source))),
    }
}

/// `dx insert <file> --after ID` — add a block directly after another one.
///
/// `dx append` can only reach the end of a document, which makes it useless for the one
/// thing a person writing a page does constantly: press Return in the middle and keep going.
/// The new block's id is printed so the caller can put the cursor in it.
///
/// It takes every flag `dx append` takes — `--id` and `--level` as well as `--lang`,
/// `--run`, and `--deps` — because they are one operation with one anchor: a block a caller
/// could name at the end of a document but not in the middle of it is a difference between
/// two spellings of the same edit, and nothing an author could explain.
pub fn run_insert(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    refuse_extra_positionals(args, 1)?;
    let kind = args.value("type").unwrap_or("paragraph");
    require_code_for_attributes(kind, args)?;
    let insertion = edit::Insertion {
        kind,
        body: args.value("text").unwrap_or_default(),
        id: args.value("id").unwrap_or_default(),
        level: args.number("level").map_or(0, |level| level as u8),
        language: args.value("lang").unwrap_or_default(),
        run: args.present("run"),
        deps: args.value("deps").unwrap_or_default(),
    };
    let (updated, id) =
        edit::insert_after(&workspace::read(&path)?, args.value("after"), &insertion)?;
    workspace::save(&path, &parse(&updated))?;
    Ok(format!("{id}\n"))
}

/// Refuse `--lang`/`--run`/`--deps` on a block that is not code, where the format would
/// silently drop them — the caller would believe they authored a runnable block.
///
/// Presence is what matters, not a value: a trailing valueless `--lang` parses as a
/// boolean flag, and it still says "I meant a code block".
fn require_code_for_attributes(kind: &str, args: &Args) -> Result<(), String> {
    let carries_code_attributes =
        args.present("lang") || args.present("run") || args.present("deps");
    if carries_code_attributes && kind != "code" {
        return Err(format!(
            "--lang, --run, and --deps describe a code block — pass --type code, not --type {kind}"
        ));
    }
    Ok(())
}

/// `dx board <file> <board-id>` — the node operations of a planning board.
///
/// This is what dragging on the canvas *is*, with a command line in front — the same
/// `doc_core::edit` operations the editing surfaces perform, so an agent arranging a plan
/// and a person dragging a node are doing the identical thing:
///
/// - `--place <node> [--x N] [--y N] [--w N] [--h N]` moves and/or reshapes (or adds) a
///   node — each of the four keeps what the node's line already says when left out, so
///   resizing a node in place needs only `--w`/`--h`, and moving one needs only `--x`/`--y`.
///   `--place ""` is spelled `--add`, which picks a fresh id, writes its line, and creates a
///   hidden paragraph for it. Both print the node's id, bare, the way `dx insert` answers.
///   The board settles after the placement: the node stays where it was put, and anything it
///   landed on moves down, so no call can leave one node covering another.
/// - `--arrange "<node,x,y[,w[,h]]> …"` places every node named, in one edit — a whole board
///   laid out in one call, which is how an agent arranges what it has just written.
/// - `--detach <node>` removes the node's line, every edge pointing at it, and — when it
///   was the board's own hidden block, shown nowhere else — the block itself.
/// - `--link <from> --to <id>` / `--unlink <from> --to <id>` draw and erase an edge.
///   `--from-side` and `--to-side` (`left`, `right`, `top`, `bottom`, or their initials)
///   pin which edge of each node the line meets; unnamed leaves that end to the renderer.
///
/// Exactly one action per call: a command that did several would have to invent an order,
/// and the caller already has one — their own next call. `--arrange` is the one exception,
/// and it is not several actions but one: a board settles as a whole or not at all.
pub fn run_board(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let board = args
        .positional(1)
        .ok_or_else(|| "a board id is required — see `dx outline <file>`".to_string())?;
    let source = workspace::read(&path)?;

    let actions = ["place", "add", "arrange", "detach", "link", "unlink"];
    let chosen: Vec<&str> = actions
        .iter()
        .copied()
        .filter(|action| args.present(action))
        .collect();
    let [action] = chosen[..] else {
        return Err(
            "pick exactly one of --place <node>, --add, --arrange <placements>, \
             --detach <node>, --link <from> --to <id>, or --unlink <from> --to <id>"
                .to_string(),
        );
    };

    // `None` when the flag is absent — `board_place` then keeps the node's own x/y (or, for
    // a brand new node, takes 0) rather than a caller resizing a node having to repeat its
    // position, or one moving it having to repeat its size.
    let coordinate = |name: &str| -> Result<Option<i32>, String> {
        match args.value(name) {
            None => Ok(None),
            Some(raw) => raw.trim().parse().map(Some).map_err(|_| {
                format!("--{name} takes a whole number of canvas pixels, not `{raw}`")
            }),
        }
    };

    match action {
        "place" | "add" => {
            let node = if action == "add" {
                ""
            } else {
                args.value("place")
                    .ok_or_else(|| "--place takes the node's id".to_string())?
            };
            // A size may be a number of pixels, `page`, or `fit` — the same three
            // spellings a node line itself takes.
            let size = |name: &str| -> Result<edit::SizeSpec, String> {
                edit::SizeSpec::parse(args.value(name).unwrap_or_default())
                    .map_err(|why| format!("--{name}: {why}"))
            };
            let (updated, id) = edit::board_place(
                &source,
                board,
                node,
                coordinate("x")?,
                coordinate("y")?,
                size("w")?,
                size("h")?,
            )?;
            workspace::save(&path, &parse(&updated))?;
            Ok(format!("{id}\n"))
        }
        "arrange" => {
            let spec = args.value("arrange").ok_or_else(|| {
                "--arrange takes `node,x,y[,width[,height]]` for each node it places".to_string()
            })?;
            let placements = edit::placements(spec)?;
            let updated = edit::board_arrange(&source, board, &placements)?;
            workspace::save(&path, &parse(&updated))?;
            Ok(format!(
                "placed {} node{} on `{board}`\n",
                placements.len(),
                if placements.len() == 1 { "" } else { "s" }
            ))
        }
        "detach" => {
            let node = args
                .value("detach")
                .ok_or_else(|| "--detach takes the node's id".to_string())?;
            let updated = edit::board_detach(&source, board, node)?;
            workspace::save(&path, &parse(&updated))?;
            Ok(format!("detached `{node}` from `{board}`\n"))
        }
        linking => {
            let from = args
                .value(linking)
                .ok_or_else(|| format!("--{linking} takes the source node's id"))?;
            let to = args
                .value("to")
                .ok_or_else(|| format!("--{linking} needs --to <id>"))?;
            let updated = edit::board_link(
                &source,
                board,
                from,
                to,
                linking == "link",
                args.value("from-side").unwrap_or_default(),
                args.value("to-side").unwrap_or_default(),
            )?;
            workspace::save(&path, &parse(&updated))?;
            Ok(format!(
                "{} `{from}` → `{to}` on `{board}`\n",
                if linking == "link" {
                    "linked"
                } else {
                    "unlinked"
                }
            ))
        }
    }
}

/// `dx check <file> <checklist-id> --item N` — tick or untick one box of a checklist.
///
/// This is what clicking a box on the page *is*, with a command line in front. The item is
/// named by position, counting from zero, which is the position the renderer writes on every
/// mark (`data-check`) and the one argument `edit::toggle_check` takes — so a person clicking
/// the third box and an agent passing `--item 2` flip the identical marker.
///
/// One call is one box: there is no `--all`, because a checklist ticked wholesale is a
/// checklist nobody read.
pub fn run_check(args: &Args) -> Result<String, String> {
    let path = path_argument(args, 0, "a .dx file is required")?;
    let id = args
        .value("block")
        .or_else(|| args.positional(1))
        .ok_or_else(|| "a checklist id is required — see `dx outline <file>`".to_string())?;
    let item = args
        .number("item")
        .ok_or_else(|| "--item takes the box's position, counting from zero".to_string())?;

    let (updated, checked) = edit::toggle_check(
        &workspace::read(&path)?,
        id,
        usize::try_from(item).unwrap_or(usize::MAX),
    )?;
    workspace::save(&path, &parse(&updated))?;
    Ok(format!(
        "{} item {item} of `{id}`\n",
        if checked { "ticked" } else { "cleared" }
    ))
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

/// Refuse positional words past the ones a command takes.
///
/// A body passed bare — `dx set doc.dx block print(1)` — used to be silently ignored:
/// with no `--text` the command fell back to stdin, read nothing, and replaced the
/// block's body with the empty string it got. A stray word is a caller who believes
/// they said something, so it is an error naming the way the something is actually said.
fn refuse_extra_positionals(args: &Args, expected: usize) -> Result<(), String> {
    match args.positionals().get(expected) {
        Some(stray) => Err(format!(
            "unexpected argument `{stray}` — a block's body is passed with --text \
             (multi-line is fine), --from <file>, or piped on stdin"
        )),
        None => Ok(()),
    }
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
    fn a_bare_body_word_is_refused_not_silently_dropped() {
        // `dx set doc.dx block print(1)` once ignored the body word, fell back to an
        // empty stdin, and wiped the block to nothing — content destroyed by a typo.
        let path = scratch("set-bare").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nkeep me\n::end\n").expect("seed");
        let error = run_set(&args(&[&file, "p", "new body"])).expect_err("should refuse");
        assert!(error.contains("--text"), "{error}");
        let raw = workspace::read(&path).expect("resolve");
        assert!(raw.contains("keep me"), "body was touched: {raw}");
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

    /// The defect this pins down: `dx insert --lang … --run --deps …` used to exit 0 while
    /// dropping all three, so a runnable block could only be authored by hand-writing source.
    #[test]
    fn insert_authors_a_runnable_code_block() {
        let path = scratch("insert-code").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nstart\n::end\n").expect("seed");

        let id = run_insert(&args(&[
            &file, "--after", "p", "--type", "code", "--lang", "python", "--run", "--deps",
            "requests", "--text", "print(1)",
        ]))
        .expect("insert code");

        let raw = workspace::read(&path).expect("resolve");
        assert!(
            raw.contains(&format!(
                "::code id={} lang=python run deps=requests",
                id.trim()
            )),
            "{raw}"
        );
    }

    #[test]
    fn insert_refuses_code_attributes_on_a_block_that_is_not_code() {
        let path = scratch("insert-attrs").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nx\n::end\n").expect("seed");

        let error = run_insert(&args(&[&file, "--after", "p", "--lang", "python"]))
            .expect_err("should fail");
        assert!(error.contains("--type code"), "{error}");
        assert_eq!(
            workspace::read(&path).expect("resolve"),
            "::paragraph id=p\nx\n::end\n"
        );
    }

    #[test]
    fn append_rejects_a_block_type_the_format_does_not_have() {
        let path = scratch("append-bad").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nx\n::end\n").expect("seed");
        let error = run_append(&args(&[&file, "--type", "widget", "--text", "x"]))
            .expect_err("should fail");
        assert!(error.contains("cannot add"));
    }

    /// The defect this pins down: an explicit `--id` a block already carried used to be
    /// resolved by the registry silently renaming — now it is refused, and the file is
    /// left untouched.
    #[test]
    fn append_refuses_an_id_the_document_already_uses() {
        let path = scratch("append-dup-id").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nx\n::end\n").expect("seed");

        let error =
            run_append(&args(&[&file, "--id", "p", "--text", "again"])).expect_err("should fail");
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(
            workspace::read(&path).expect("resolve"),
            "::paragraph id=p\nx\n::end\n"
        );

        run_append(&args(&[&file, "--id", "note", "--text", "fresh"])).expect("append");
        assert!(workspace::read(&path)
            .expect("resolve")
            .contains("::paragraph id=note\nfresh\n::end"));
    }

    /// `dx insert` takes every flag `dx append` takes: the same block, at a different
    /// anchor. `--id` and `--level` used to be refused by name here and accepted there.
    #[test]
    fn insert_names_and_levels_a_block_exactly_as_append_does() {
        let path = scratch("insert-id-level").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nstart\n::end\n").expect("seed");

        let id = run_insert(&args(&[
            &file, "--after", "p", "--type", "heading", "--id", "callout", "--level", "3",
            "--text", "Deep",
        ]))
        .expect("insert");
        assert_eq!(id.trim(), "callout");
        assert!(workspace::read(&path)
            .expect("resolve")
            .contains("::heading level=3 id=callout\nDeep\n::end"));

        run_append(&args(&[
            &file, "--type", "heading", "--id", "tail", "--level", "3", "--text", "End",
        ]))
        .expect("append");
        assert!(workspace::read(&path)
            .expect("resolve")
            .contains("::heading level=3 id=tail\nEnd\n::end"));
    }

    /// An id is slugified on its way into the document, so a spelling that lands on a taken
    /// one is the same collision — refused with the sentence, and the file left alone.
    #[test]
    fn a_differently_spelled_id_that_lands_on_a_taken_one_is_refused() {
        let path = scratch("insert-slug-id").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        let before = "::paragraph id=my-id\nx\n::end\n";
        workspace::write_text(&path, before).expect("seed");

        let inserted = run_insert(&args(&[
            &file, "--after", "my-id", "--id", "My Id", "--text", "again",
        ]))
        .expect_err("should be refused");
        assert!(inserted.contains("`my-id` already exists"), "{inserted}");

        let appended = run_append(&args(&[&file, "--id", "My Id", "--text", "again"]))
            .expect_err("should be refused");
        assert!(appended.contains("`my-id` already exists"), "{appended}");

        assert_eq!(workspace::read(&path).expect("resolve"), before);
    }

    /// A trailing `--lang` with no value parses as a boolean flag, and it still means
    /// "code" — it must be refused on prose, not silently swallowed.
    #[test]
    fn a_valueless_code_flag_is_still_refused_on_prose() {
        let path = scratch("append-bare-lang").join("doc.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::paragraph id=p\nx\n::end\n").expect("seed");

        for flag in ["--lang", "--deps"] {
            let error = run_append(&args(&[&file, "--text", "y", flag])).expect_err("should fail");
            assert!(error.contains("--type code"), "{error}");
        }
        let error = run_insert(&args(&[&file, "--after", "p", "--text", "y", "--lang"]))
            .expect_err("should fail");
        assert!(error.contains("--type code"), "{error}");
    }

    /// Ticking a box, end to end through the file: the marker flips, the answer says which
    /// way it went, and a second call puts it back.
    #[test]
    fn check_ticks_one_box_through_the_file() {
        let path = scratch("check").join("todo.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::checklist id=todo\n[ ] a\n[x] b\n::end\n").expect("seed");

        let ticked = run_check(&args(&[&file, "todo", "--item", "0"])).expect("tick");
        assert!(ticked.contains("ticked item 0"), "{ticked}");
        assert!(workspace::read(&path).expect("resolve").contains("[x] a"));

        let cleared = run_check(&args(&[&file, "todo", "--item", "0"])).expect("untick");
        assert!(cleared.contains("cleared item 0"), "{cleared}");
        assert_eq!(
            workspace::read(&path).expect("resolve"),
            "::checklist id=todo\n[ ] a\n[x] b\n::end\n"
        );

        let missing = run_check(&args(&[&file, "todo"])).expect_err("no --item");
        assert!(missing.contains("--item"), "{missing}");
        let past = run_check(&args(&[&file, "todo", "--item", "9"])).expect_err("no such box");
        assert!(past.contains("has 2 items"), "{past}");
    }

    /// The board verbs, end to end through the file: place moves, add creates a node and
    /// its hidden block, link draws the edge, detach takes all of it back out.
    #[test]
    fn board_arranges_nodes_through_the_file() {
        let path = scratch("board").join("plan.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(
            &path,
            "::board id=plan\n- ideas x=40 y=40 w=280 h=160\n::end\n\n\
             ::paragraph id=ideas hidden\nRough.\n::end\n",
        )
        .expect("seed");

        let placed = run_board(&args(&[
            &file, "plan", "--place", "ideas", "--x", "200", "--y", "80",
        ]))
        .expect("place");
        assert_eq!(placed.trim(), "ideas");
        assert!(workspace::read(&path)
            .expect("resolve")
            .contains("- ideas x=200 y=80 w=280 h=160"));

        let added =
            run_board(&args(&[&file, "plan", "--add", "--x", "500", "--y", "40"])).expect("add");
        let node = added.trim().to_string();
        let raw = workspace::read(&path).expect("resolve");
        assert!(
            raw.contains(&format!("- {node} x=500 y=40 w=280 h=180")),
            "{raw}"
        );
        assert!(
            raw.contains(&format!("::paragraph id={node} hidden")),
            "{raw}"
        );

        run_board(&args(&[&file, "plan", "--link", &node, "--to", "ideas"])).expect("link");
        assert!(workspace::read(&path)
            .expect("resolve")
            .contains(&format!("- {node} x=500 y=40 w=280 h=180 to=ideas")));

        run_board(&args(&[&file, "plan", "--detach", &node])).expect("detach");
        let raw = workspace::read(&path).expect("resolve");
        assert!(!raw.contains(&format!("- {node}")), "{raw}");
        assert!(!raw.contains(&format!("id={node}")), "{raw}");
    }

    /// Regression: `--place` used to default an omitted `--x`/`--y` to `0`, so resizing a
    /// node with only `--w`/`--h` silently teleported it to the origin. Each of the four now
    /// keeps what the line already said when its flag is left out.
    #[test]
    fn board_place_can_resize_without_repeating_position_and_move_without_repeating_size() {
        let path = scratch("board-partial").join("plan.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(
            &path,
            "::board id=plan\n- ideas x=200 y=80 w=280 h=160\n::end\n\n\
             ::paragraph id=ideas hidden\nRough.\n::end\n",
        )
        .expect("seed");

        run_board(&args(&[
            &file, "plan", "--place", "ideas", "--w", "400", "--h", "300",
        ]))
        .expect("resize");
        assert!(
            workspace::read(&path)
                .expect("resolve")
                .contains("- ideas x=200 y=80 w=400 h=300"),
            "{}",
            workspace::read(&path).expect("resolve")
        );

        run_board(&args(&[
            &file, "plan", "--place", "ideas", "--x", "50", "--y", "60",
        ]))
        .expect("move");
        assert!(
            workspace::read(&path)
                .expect("resolve")
                .contains("- ideas x=50 y=60 w=400 h=300"),
            "{}",
            workspace::read(&path).expect("resolve")
        );
    }

    /// A size flag speaks the node line's own vocabulary: `--w page` and `--h fit` write
    /// the words onto the line, and a spelling that is neither a number nor a rule is
    /// refused with a sentence.
    #[test]
    fn board_place_takes_page_and_fit_as_sizes() {
        let path = scratch("board-rules").join("plan.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(
            &path,
            "::board id=plan\n- ideas x=200 y=80 w=280 h=160\n::end\n\n\
             ::paragraph id=ideas hidden\nRough.\n::end\n",
        )
        .expect("seed");

        run_board(&args(&[
            &file, "plan", "--place", "ideas", "--w", "page", "--h", "fit",
        ]))
        .expect("rule");
        assert!(
            workspace::read(&path)
                .expect("resolve")
                .contains("- ideas x=200 y=80 w=page h=fit"),
            "{}",
            workspace::read(&path).expect("resolve")
        );

        let refused = run_board(&args(&[&file, "plan", "--place", "ideas", "--w", "huge"]))
            .expect_err("not a size");
        assert!(refused.contains("`huge` is not a size"), "{refused}");
    }

    /// The two things a reader does with the pointer, reachable from a terminal: pinning an
    /// edge to the sides it was dragged between, and settling the whole board in one call.
    #[test]
    fn board_links_between_sides_and_arranges_every_node_at_once() {
        let path = scratch("board-sides").join("plan.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(
            &path,
            "::board id=plan\n- a x=0 y=0 w=280 h=160\n- b x=0 y=300 w=280 h=160\n::end\n\n\
             ::paragraph id=a hidden\nOne.\n::end\n\n::paragraph id=b hidden\nTwo.\n::end\n",
        )
        .expect("seed");

        run_board(&args(&[
            &file,
            "plan",
            "--link",
            "a",
            "--to",
            "b",
            "--from-side",
            "bottom",
            "--to-side",
            "t",
        ]))
        .expect("link");
        assert!(workspace::read(&path)
            .expect("resolve")
            .contains("- a x=0 y=0 w=280 h=160 to=b:b-t"));

        let laid = run_board(&args(&[
            &file,
            "plan",
            "--arrange",
            "a,20,0,280,200 b,20,340,320,240",
        ]))
        .expect("arrange");
        assert!(laid.contains("placed 2 nodes"), "{laid}");
        let raw = workspace::read(&path).expect("resolve");
        assert!(raw.contains("- a x=20 y=0 w=280 h=200 to=b:b-t"), "{raw}");
        assert!(raw.contains("- b x=20 y=340 w=320 h=240"), "{raw}");

        // Reshaping one node so it covers the other pushes the other down, for an agent
        // exactly as for a drag: a `dx board` call cannot leave a node buried.
        run_board(&args(&[
            &file, "plan", "--place", "a", "--x=20", "--y=0", "--w=280", "--h=400",
        ]))
        .expect("reshape");
        let raw = workspace::read(&path).expect("resolve");
        assert!(raw.contains("- a x=20 y=0 w=280 h=400"), "{raw}");
        assert!(raw.contains("- b x=20 y=428 w=320 h=240"), "{raw}");

        let bad = run_board(&args(&[&file, "plan", "--arrange", "a,20"])).expect_err("shapeless");
        assert!(bad.contains("node,x,y"), "{bad}");
    }

    #[test]
    fn board_refuses_ambiguity_and_bad_numbers() {
        let path = scratch("board-refuse").join("plan.dx");
        let file = path.to_string_lossy().into_owned();
        workspace::write_text(&path, "::board id=plan\n- a x=0 y=0\n::end\n").expect("seed");

        let none = run_board(&args(&[&file, "plan"])).expect_err("no action");
        assert!(none.contains("exactly one"), "{none}");
        let both =
            run_board(&args(&[&file, "plan", "--add", "--detach", "a"])).expect_err("two actions");
        assert!(both.contains("exactly one"), "{both}");
        let bad = run_board(&args(&[&file, "plan", "--place", "a", "--x", "wide"]))
            .expect_err("bad number");
        assert!(bad.contains("--x"), "{bad}");
        assert_eq!(
            workspace::read(&path).expect("resolve"),
            "::board id=plan\n- a x=0 y=0\n::end\n",
            "a refused call must leave the file untouched"
        );
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
