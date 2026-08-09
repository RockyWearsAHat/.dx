//! Reading commands: `text`, `outline`, `render`, `png`, and `open`.
//!
//! Every one of these takes the same two optional ideas — *which part* of the document
//! (`--section`) and *how it should look* (`--theme`) — so moving between the text view,
//! the page view, and the picture is a one-word change.

use std::path::{Path, PathBuf};

use doc_core::edit;
use doc_core::format::parse;
use doc_core::model::Document;
use doc_core::render::{self, html, outline, section, text, HtmlOptions, TextOptions, Theme};
use doc_shot::{capture, ShotOptions};

use crate::args::Args;
use crate::workspace;

/// `dx text <file>` — the document as Markdown.
pub fn run_text(args: &Args) -> Result<String, String> {
    let document = selected_document(args)?;
    Ok(text(
        &document,
        &TextOptions {
            include_ids: args.present("ids"),
            include_hidden: args.present("hidden"),
            include_presentation: args.present("all"),
        },
    ))
}

/// `dx outline <file>` — one line per block: id, kind, and a preview.
pub fn run_outline(args: &Args) -> Result<String, String> {
    let document = selected_document(args)?;
    let rows = outline(&document);
    if rows.is_empty() {
        return Ok("(empty document)\n".to_string());
    }

    let width = rows
        .iter()
        .map(|row| row.id.len())
        .max()
        .unwrap_or(2)
        .max(2);
    let mut out = String::new();
    for row in rows {
        let indent = "  ".repeat(usize::from(row.level.saturating_sub(1)));
        let marks = marks_for(&row);
        out.push_str(&format!(
            "{:<width$}  {:<14} {indent}{}{marks}\n",
            row.id,
            row.kind,
            row.preview,
            width = width
        ));
    }
    Ok(out)
}

/// Trailing annotations for an outline row: run marker and size.
fn marks_for(row: &doc_core::render::OutlineEntry) -> String {
    let mut marks = String::new();
    if row.runnable {
        marks.push_str("  [runnable]");
    }
    if row.chars > 0 {
        marks.push_str(&format!("  ({} chars)", row.chars));
    }
    marks
}

/// `dx render <file>` — the document as a self-contained HTML page.
///
/// `--fragment` emits just the `.dx-doc` container, which is what a surface swaps in when a
/// block changed and the page around it did not. `--block <id>` narrows that to one block,
/// and `--body <text>` renders it with a body that has not been saved — the two together are
/// how a page keeps rendering while a reader is still typing into it. `--field <text>`
/// renders characters as the editing field decorates them — marks styled in place, every
/// byte kept — and reads no file at all.
pub fn run_render(args: &Args) -> Result<String, String> {
    // A field render is a pure function of the characters: no file, no store, no options.
    if let Some(text) = args.value("field") {
        return Ok(doc_core::render::field_html(text));
    }

    if args.present("all") {
        return run_render_all(args);
    }

    let Some(id) = args.value("block") else {
        let document = selected_document(args)?;
        return Ok(html(&document, &html_options(args)));
    };

    let source = workspace::read(&document_path(args)?)?;
    let options = html_options(args);
    match args.value("body") {
        Some(body) => edit::preview_block(&source, id, body, &options),
        None => {
            let mut document = parse(&source);
            doc_core::resolve::hydrate(
                &mut document,
                &workspace::resolver_for(&document_path(args)?),
            );
            // Asked for first, so a mistyped id is answered with the ids that do exist
            // rather than with nothing.
            edit::find(&document, id)?;
            Ok(render::block(&document, id, &options).unwrap_or_default())
        }
    }
}

/// `dx render --all [dir] --out <dir>` — every document in a workspace as its own page.
///
/// One command exports a whole site: each `.dx` under `dir` (the current directory when
/// unnamed) becomes `<out>/<same relative path>.html`, rendered exactly as `dx render`
/// renders it alone — hydrated, self-contained, one engine. This exists because the
/// per-document route made a site deliverable pay one command per page after every edit,
/// and a regeneration step anyone can forget is an export that drifts from its source.
///
/// The answer is a report of the files written, never page content — so `--out` names the
/// export directory here, not a file to redirect one page into.
fn run_render_all(args: &Args) -> Result<String, String> {
    let root = args
        .positional(0)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let out = PathBuf::from(args.value("out").ok_or_else(|| {
        "dx render --all writes one page per document — say where with --out <dir>".to_string()
    })?);

    let documents = workspace::load_all(&root);
    if documents.is_empty() {
        return Err(format!("no .dx documents under {}", root.display()));
    }

    let options = html_options(args);
    let mut report = String::new();
    for loaded in documents {
        let mut document = loaded.document;
        doc_core::resolve::hydrate(&mut document, &workspace::resolver_for(&loaded.path));
        let target = out.join(Path::new(&loaded.relative).with_extension("html"));
        workspace::write_text(&target, &html(&document, &options))?;
        report.push_str(&format!("wrote {}\n", target.display()));
    }
    Ok(report)
}

/// `dx png <file>` — the document as an image, written to a file.
///
/// With `--pages` it writes one image per page — `notes-1.png`, `notes-2.png`, … — which is
/// the same division an agent gets from `dx_read`, for anyone driving `dx` from a shell.
/// With `--block <id>` it photographs that one block alone: a `::board` at its natural
/// canvas size (every node exactly the box its line states), anything else in the ordinary
/// column — which is how one board, or one node's block, is rendered out and checked
/// without paging through the document around it. `--block` also takes a comma-separated
/// list (`--block desk,phone`), captured from **one** browser session with one
/// `<stem>-<id>.png` per block. `--block <id> --against <golden.png>` writes nothing and
/// prints a one-line verdict instead ([`run_png_against`]).
pub fn run_png(args: &Args) -> Result<String, String> {
    if let Some(golden) = args.value("against") {
        return run_png_against(args, golden);
    }
    if args.present("pages") {
        return run_png_pages(args);
    }
    if let Some(list) = args.value("block") {
        return run_png_blocks(args, list);
    }
    let path = document_path(args)?;
    let document = selected_document(args)?;
    let shot = capture(
        &document,
        &ShotOptions {
            width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
            theme: theme_of(args),
            scale: export_scale(args),
            ..ShotOptions::default()
        },
    )?;

    let target = export_target(args.value("out"), path.with_extension("png"));
    std::fs::write(&target, &shot.png)
        .map_err(|error| format!("could not write {}: {error}", target.display()))?;

    Ok(format!(
        "wrote {} ({}x{} px)\n",
        target.display(),
        shot.width,
        shot.height
    ))
}

/// `dx png <file> --block <id>[,<id>…]` — the named blocks, each as its own image.
///
/// One id keeps the single-block export exactly as it has always been. A list captures
/// every named block from one browser session ([`doc_shot::capture_blocks`]) — a visual
/// loop over N blocks pays one browser startup instead of N — writing `<stem>-<id>.png`
/// per block, the same name the single export chooses.
fn run_png_blocks(args: &Args, list: &str) -> Result<String, String> {
    let ids = block_ids(list);
    match ids.as_slice() {
        [] => Err(format!(
            "no block ids in `{list}` — name at least one, e.g. --block desk,phone"
        )),
        [id] => run_png_block(args, id),
        many => {
            let path = document_path(args)?;
            let document = selected_document(args)?;
            let shots = doc_shot::capture_blocks(
                &document,
                many,
                &ShotOptions {
                    width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
                    theme: theme_of(args),
                    scale: export_scale(args),
                    ..ShotOptions::default()
                },
            )?;

            let stem = page_stem(args.value("out"), &path);
            let mut out = String::new();
            for (id, shot) in many.iter().zip(&shots) {
                let target = block_target(&stem, id);
                std::fs::write(&target, &shot.png)
                    .map_err(|error| format!("could not write {}: {error}", target.display()))?;
                out.push_str(&format!(
                    "wrote {} ({}x{} px)\n",
                    target.display(),
                    shot.width,
                    shot.height
                ));
            }
            Ok(out)
        }
    }
}

/// `dx png <file> --block <id> --against <golden.png>` — a text verdict, no file written.
///
/// The block is captured exactly as `--block` alone would capture it, then compared to the
/// golden PNG pixel by pixel through the platform's own codec ([`doc_shot::diff`]). The
/// answer is one line: `identical` (every pixel within the antialiasing tolerance
/// [`doc_shot::diff::CHANNEL_TOLERANCE`] — 3 of 255 per channel), or
/// `differs: N px in x,y wxh` naming the bounding box of every changed pixel. Different
/// dimensions are a stated verdict too, never an error — a board that grew *is* the
/// difference being asked about. This is how a visual check is read without spending an
/// image.
fn run_png_against(args: &Args, golden: &str) -> Result<String, String> {
    let Some(list) = args.value("block") else {
        return Err("--against compares one block's render — add --block <id>".to_string());
    };
    let ids = block_ids(list);
    let [id] = ids.as_slice() else {
        return Err(format!(
            "--against compares one block at a time — name exactly one id, not `{list}`"
        ));
    };
    if args.present("pages") {
        return Err("--against compares one block — it does not take --pages".to_string());
    }
    if args.value("out").is_some() {
        return Err("--against prints a verdict and writes no file — drop --out".to_string());
    }

    let document = selected_document(args)?;
    let shot = doc_shot::capture_block(
        &document,
        id,
        &ShotOptions {
            width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
            theme: theme_of(args),
            scale: export_scale(args),
            ..ShotOptions::default()
        },
    )?;
    let golden_bytes = std::fs::read(golden)
        .map_err(|error| format!("could not read the golden image {golden}: {error}"))?;
    let verdict = doc_shot::diff::compare_png(&shot.png, &golden_bytes)?;
    Ok(format!("{verdict}\n"))
}

/// The ids a `--block` list names: split on commas, stray whitespace and empty entries
/// dropped, order kept — the shots come back in this order.
fn block_ids(list: &str) -> Vec<&str> {
    list.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect()
}

/// Where one block of a batch lands, given the stem [`page_stem`] chose.
fn block_target(stem: &Path, id: &str) -> PathBuf {
    PathBuf::from(format!("{}-{id}.png", stem.display()))
}

/// `dx png <file> --block <id>` — one block as an image: a board at its natural size,
/// anything else exactly as the page carries it.
fn run_png_block(args: &Args, id: &str) -> Result<String, String> {
    let path = document_path(args)?;
    let document = selected_document(args)?;
    let shot = doc_shot::capture_block(
        &document,
        id,
        &ShotOptions {
            width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
            theme: theme_of(args),
            scale: export_scale(args),
            ..ShotOptions::default()
        },
    )?;

    let target = export_target(
        args.value("out"),
        path.with_file_name(png_block_name(&path, id)),
    );
    std::fs::write(&target, &shot.png)
        .map_err(|error| format!("could not write {}: {error}", target.display()))?;

    Ok(format!(
        "wrote {} ({}x{} px)\n",
        target.display(),
        shot.width,
        shot.height
    ))
}

/// The default file name for a one-block export: `<stem>-<block>.png`, beside the document.
fn png_block_name(path: &Path, id: &str) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    format!("{stem}-{id}.png")
}

/// The device scale a `dx png` export captures at: 2 unless `--scale` says otherwise.
///
/// An export exists to look like the document did on the author's screen, and the
/// author's screen is almost certainly high-density. `--scale 1` remains available for
/// anything that wants CSS-pixel images.
fn export_scale(args: &Args) -> u32 {
    args.number("scale").unwrap_or(2)
}

/// `dx png <file> --pages` — one image per page, numbered in reading order.
///
/// The report names every file written and says which blocks are on each page, so the
/// caller can go straight to `--section` for the part they actually wanted.
fn run_png_pages(args: &Args) -> Result<String, String> {
    let path = document_path(args)?;
    let document = selected_document(args)?;
    let pages = doc_shot::capture_pages(
        &document,
        &ShotOptions {
            width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
            theme: theme_of(args),
            page_height: args
                .number("page-height")
                .unwrap_or(doc_shot::DEFAULT_PAGE_HEIGHT),
            scale: export_scale(args),
            ..ShotOptions::default()
        },
    )?;

    let stem = page_stem(args.value("out"), &path);
    let mut out = String::new();
    for page in &pages {
        let target = page_target(&stem, page.number);
        std::fs::write(&target, &page.shot.png)
            .map_err(|error| format!("could not write {}: {error}", target.display()))?;
        out.push_str(&format!(
            "wrote {} ({}x{} px) — blocks: {}\n",
            target.display(),
            page.shot.width,
            page.shot.height,
            page.blocks.join(", ")
        ));
    }

    let total = pages.first().map_or(0, |page| page.total);
    if pages.len() < total {
        out.push_str(&format!(
            "stopped after {} of {total} pages — use --section to read further\n",
            pages.len()
        ));
    }
    Ok(out)
}

/// Where an export lands: `--out` when given, else `fallback` beside the document.
///
/// An `--out` naming an existing directory keeps the fallback's file name and writes
/// inside it — a folder says *where*, not *what to call it*. Taking the directory as
/// the name itself produced `.-1.png` from `--out .` and "Is a directory" from a
/// trailing slash.
fn export_target(out: Option<&str>, fallback: PathBuf) -> PathBuf {
    match out {
        None => fallback,
        Some(out) => {
            let given = PathBuf::from(out);
            match (given.is_dir(), fallback.file_name()) {
                (true, Some(name)) => given.join(name),
                _ => given,
            }
        }
    }
}

/// The base name page images are numbered from: `--out` when given, else the document's path.
///
/// The extension comes off either way, because the page number goes *between* the name and the
/// extension. Leaving it on turned `--out pages.png` into `pages.png-1.png`.
fn page_stem(out: Option<&str>, path: &Path) -> PathBuf {
    export_target(out, path.to_path_buf()).with_extension("")
}

/// Where page `number` is written, given the stem [`page_stem`] chose.
fn page_target(stem: &Path, number: usize) -> PathBuf {
    PathBuf::from(format!("{}-{number}.png", stem.display()))
}

/// `dx open <file>` — render to a temporary page and open it in the default browser.
pub fn run_open(args: &Args) -> Result<String, String> {
    let path = document_path(args)?;
    let document = selected_document(args)?;
    let page = html(&document, &html_options(args));

    let target = std::env::temp_dir().join(format!(
        "dx-preview-{}.html",
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_string())
    ));
    workspace::write_text(&target, &page)?;
    open_in_browser(&target)?;
    Ok(format!("opened {}\n", target.display()))
}

/// Hand a file to the operating system's default handler.
fn open_in_browser(path: &Path) -> Result<(), String> {
    let (program, leading): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(windows) {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };

    std::process::Command::new(program)
        .args(leading)
        .arg(path)
        .status()
        .map(|_| ())
        .map_err(|error| format!("could not open a browser with `{program}`: {error}"))
}

/// Load the document named by the first positional argument, sliced by `--section`.
pub fn selected_document(args: &Args) -> Result<Document, String> {
    let path = document_path(args)?;
    let mut document = workspace::load(&path)?.document;
    // Fill in what the document references — sibling files behind `::code src=`, sibling
    // documents behind board node lines — so every view shows current content instead of
    // a copy. A reference that resolves to nothing is already a sentence in its block's
    // place, which is the honest render. Hydration is view-only: nothing loaded here is
    // ever saved.
    doc_core::resolve::hydrate(&mut document, &workspace::resolver_for(&path));
    slice(document, args.value("section"))
}

/// Apply a `--section` selector to a document, or return it whole.
pub fn slice(document: Document, selector: Option<&str>) -> Result<Document, String> {
    match selector {
        None => Ok(document),
        Some(id) => section(&document, id).ok_or_else(|| {
            let available = outline(&document)
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>()
                .join(", ");
            format!("no block named `{id}`. Available: {available}")
        }),
    }
}

/// The document path from the first positional argument.
pub fn document_path(args: &Args) -> Result<PathBuf, String> {
    args.positional(0)
        .map(PathBuf::from)
        .ok_or_else(|| "a .dx file is required".to_string())
}

/// Build HTML options from `--theme`, `--fragment`, `--hidden`, and `--show-code`.
///
/// The document's own `::style` blocks are not among them: a document dresses itself on
/// every surface, so there is no flag to forget.
///
/// Code starts folded, because a page is read for what it says. `--show-code` is for the
/// renders that are read without a pointer — a printed sheet, a page archived as a file —
/// where a fold is a listing the reader has no way to open.
fn html_options(args: &Args) -> HtmlOptions {
    HtmlOptions {
        theme: theme_of(args),
        fragment: args.present("fragment"),
        include_hidden: args.present("hidden"),
        collapse_code: !args.present("show-code"),
        title: args.value("title").unwrap_or_default().to_string(),
    }
}

/// The `--theme` value, defaulting to following the reader's system setting.
fn theme_of(args: &Args) -> Theme {
    Theme::parse(args.value("theme").unwrap_or("auto"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    const SAMPLE: &str = "::heading level=1 id=top\nTop\n::end\n\n\
::heading level=2 id=alpha\nAlpha\n::end\n\n\
::code id=demo lang=python run\nprint(1)\n::end\n\n\
::heading level=2 id=beta\nBeta\n::end\n";

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-view-tests-{label}"));
        let path = root.join("sample.dx");
        workspace::save(&path, &parse(SAMPLE)).expect("fixture");
        path
    }

    #[test]
    fn text_renders_the_whole_document_by_default() {
        let path = fixture("text");
        let out = run_text(&args(&[&path.to_string_lossy()])).expect("text");
        assert!(out.contains("# Top"));
        assert!(out.contains("## Beta"));
    }

    /// One command exports the workspace: every document becomes its own page under
    /// `--out`, keeping its relative path, and the answer reports the files written.
    #[test]
    fn render_all_writes_one_page_per_document() {
        let root = std::env::temp_dir().join("dx-view-tests-render-all");
        let _ = std::fs::remove_dir_all(&root);
        workspace::save(
            &root.join("index.dx"),
            &parse("::heading level=1 id=h\nHome\n::end\n"),
        )
        .expect("seed");
        workspace::save(
            &root.join("guide/start.dx"),
            &parse("::paragraph id=p\nStart here.\n::end\n"),
        )
        .expect("seed");
        let out = root.join("dist");

        let report = run_render(&args(&[
            &root.to_string_lossy(),
            "--all",
            "--out",
            &out.to_string_lossy(),
        ]))
        .expect("render --all");

        assert!(report.contains("index.html"), "{report}");
        let home = std::fs::read_to_string(out.join("index.html")).expect("page");
        assert!(home.contains("Home"));
        let start = std::fs::read_to_string(out.join("guide/start.html")).expect("nested page");
        assert!(start.contains("Start here."));
    }

    /// The export refuses to guess where the pages go: no `--out`, no files.
    #[test]
    fn render_all_without_out_is_refused_with_the_flag_to_pass() {
        let refusal = run_render(&args(&["--all"])).expect_err("refuse");
        assert!(refusal.contains("--out"), "{refusal}");
    }

    /// `--block` is what a surface calls between keystrokes, so it has to answer with the
    /// block alone and with the same markup the page carries.
    #[test]
    fn one_block_renders_as_the_page_carries_it() {
        let path = fixture("block");
        let file = path.to_string_lossy().into_owned();

        let one = run_render(&args(&[&file, "--block", "alpha"])).expect("block");
        assert!(one.contains("Alpha"), "{one}");
        assert!(!one.contains("Beta"), "{one}");
        assert!(!one.contains("<!doctype"), "{one}");

        let page = run_render(&args(&[&file, "--fragment"])).expect("page");
        assert!(page.contains(one.trim()), "{one}\n\nnot found in\n{page}");
    }

    /// `--body` renders characters that were never saved, and saves none of them: this is
    /// how a page keeps rendering while a reader is still typing into it.
    #[test]
    fn a_body_is_drawn_without_being_written() {
        let path = fixture("body");
        let file = path.to_string_lossy().into_owned();

        let drawn =
            run_render(&args(&[&file, "--block", "alpha", "--body=Still typing"])).expect("draw");
        assert!(drawn.contains("Still typing"), "{drawn}");
        assert_eq!(workspace::read(&path).expect("source"), SAMPLE);
    }

    /// `--field` is the editing surface's per-keystroke call on hosts that reach the engine
    /// through this binary: pure characters in, decorated characters out, no file touched.
    #[test]
    fn a_field_is_decorated_without_reading_any_file() {
        let out = run_render(&args(&["--field=**loud** words"])).expect("field");
        assert!(out.contains("<strong>loud</strong>"), "{out}");
        assert!(out.contains("dx-mark"), "{out}");
    }

    #[test]
    fn an_unknown_block_names_the_ones_that_exist() {
        let path = fixture("unknown-block");
        let failure = run_render(&args(&[&path.to_string_lossy(), "--block", "nope"]))
            .expect_err("no such block");
        assert!(failure.contains("alpha"), "{failure}");
    }

    #[test]
    fn a_section_narrows_every_view_the_same_way() {
        let path = fixture("section");
        let file = path.to_string_lossy().into_owned();

        let markdown = run_text(&args(&[&file, "--section", "alpha"])).expect("text");
        assert!(markdown.contains("## Alpha"));
        assert!(!markdown.contains("Beta"));

        let page = run_render(&args(&[&file, "--section", "alpha"])).expect("render");
        assert!(page.contains("Alpha"));
        assert!(!page.contains(">Beta<"));
    }

    #[test]
    fn an_unknown_section_lists_what_is_available() {
        let path = fixture("bad-section");
        let error = run_text(&args(&[&path.to_string_lossy(), "--section", "nope"]))
            .expect_err("should fail");
        assert!(error.contains("no block named `nope`"));
        assert!(error.contains("alpha"));
    }

    #[test]
    fn outline_marks_runnable_blocks() {
        let path = fixture("outline");
        let out = run_outline(&args(&[&path.to_string_lossy()])).expect("outline");
        assert!(out.contains("demo"));
        assert!(out.contains("[runnable]"));
        assert!(out.lines().count() >= 4);
    }

    #[test]
    fn render_honors_the_requested_theme() {
        let path = fixture("theme");
        let page =
            run_render(&args(&[&path.to_string_lossy(), "--theme", "dark"])).expect("render");
        assert!(page.contains("data-theme=\"dark\""));
    }

    #[test]
    fn a_missing_file_argument_is_a_clear_error() {
        assert!(run_text(&args(&[])).unwrap_err().contains("required"));
    }

    #[test]
    fn a_page_number_goes_before_the_extension_not_after_the_whole_name() {
        // `--out pages.png` used to produce `pages.png-1.png`, because only the default
        // path had its extension stripped.
        let named = page_stem(Some("pages.png"), Path::new("notes.dx"));
        assert_eq!(page_target(&named, 1), PathBuf::from("pages-1.png"));

        let defaulted = page_stem(None, Path::new("dir/notes.dx"));
        assert_eq!(page_target(&defaulted, 2), PathBuf::from("dir/notes-2.png"));
    }

    #[test]
    fn a_block_list_splits_on_commas_dropping_stray_blanks() {
        assert_eq!(
            block_ids("desk,phone, journal-view"),
            vec!["desk", "phone", "journal-view"]
        );
        assert_eq!(block_ids("solo"), vec!["solo"]);
        assert!(block_ids(" , ,").is_empty());
    }

    /// `--against` names one block or nothing: without `--block` there is no render to
    /// compare, and a list would be a verdict about nobody-knows-which image.
    #[test]
    fn against_without_exactly_one_block_is_refused_before_any_file_is_read() {
        let missing = run_png(&args(&["/dx/nowhere.dx", "--against", "gold.png"]))
            .expect_err("no block named");
        assert!(missing.contains("--block"), "{missing}");

        let many = run_png(&args(&[
            "/dx/nowhere.dx",
            "--block",
            "desk,phone",
            "--against",
            "gold.png",
        ]))
        .expect_err("two blocks");
        assert!(many.contains("exactly one"), "{many}");
    }

    /// A comparison writes nothing, so flags about where to write are a contradiction to
    /// refuse by name, never swallow.
    #[test]
    fn against_refuses_the_flags_that_would_write_files() {
        let with_out = run_png(&args(&[
            "/dx/nowhere.dx",
            "--block",
            "desk",
            "--against",
            "gold.png",
            "--out",
            "x.png",
        ]))
        .expect_err("nothing is written");
        assert!(with_out.contains("--out"), "{with_out}");

        let with_pages = run_png(&args(&[
            "/dx/nowhere.dx",
            "--block",
            "desk",
            "--against",
            "gold.png",
            "--pages",
        ]))
        .expect_err("one block, not pages");
        assert!(with_pages.contains("--pages"), "{with_pages}");
    }

    #[test]
    fn an_empty_block_list_is_refused_before_any_file_is_read() {
        let error =
            run_png(&args(&["/dx/nowhere.dx", "--block", " , "])).expect_err("should refuse");
        assert!(error.contains("--block desk,phone"), "{error}");
    }

    #[test]
    fn a_batch_export_names_each_file_by_its_block() {
        let named = page_stem(Some("shots.png"), Path::new("notes.dx"));
        assert_eq!(
            block_target(&named, "desk"),
            PathBuf::from("shots-desk.png")
        );
        let defaulted = page_stem(None, Path::new("dir/notes.dx"));
        assert_eq!(
            block_target(&defaulted, "phone"),
            PathBuf::from("dir/notes-phone.png")
        );
    }

    #[test]
    fn an_out_naming_a_directory_keeps_the_default_file_name_inside_it() {
        // `--out .` used to take the directory as the stem itself, producing `.-1.png`
        // page names and "Is a directory" from the whole-document export.
        let dir = std::env::temp_dir();
        let dir_arg = dir.to_string_lossy().into_owned();

        let whole = export_target(Some(&dir_arg), PathBuf::from("notes.png"));
        assert_eq!(whole, dir.join("notes.png"));

        let paged = page_stem(Some(&dir_arg), Path::new("sub/notes.dx"));
        assert_eq!(page_target(&paged, 1), dir.join("notes-1.png"));

        // A file-shaped --out is still taken exactly as written.
        let file = export_target(Some("shots/page.png"), PathBuf::from("notes.png"));
        assert_eq!(file, PathBuf::from("shots/page.png"));
    }
}
