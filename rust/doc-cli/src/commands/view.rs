//! Reading commands: `text`, `outline`, `render`, `png`, and `open`.
//!
//! Every one of these takes the same two optional ideas — *which part* of the document
//! (`--section`) and *how it should look* (`--theme`) — so moving between the text view,
//! the page view, and the picture is a one-word change.

use std::path::{Path, PathBuf};

use doc_core::model::Document;
use doc_core::render::{html, outline, section, text, HtmlOptions, TextOptions, Theme};
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
pub fn run_render(args: &Args) -> Result<String, String> {
    let document = selected_document(args)?;
    Ok(html(&document, &html_options(args)))
}

/// `dx png <file>` — the document as an image, written to a file.
pub fn run_png(args: &Args) -> Result<String, String> {
    let path = document_path(args)?;
    let document = selected_document(args)?;
    let shot = capture(
        &document,
        &ShotOptions {
            width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
            theme: theme_of(args),
            document_css: args.present("doc-css"),
            ..ShotOptions::default()
        },
    )?;

    let target = args
        .value("out")
        .map_or_else(|| path.with_extension("png"), PathBuf::from);
    std::fs::write(&target, &shot.png)
        .map_err(|error| format!("could not write {}: {error}", target.display()))?;

    Ok(format!(
        "wrote {} ({}x{} px)\n",
        target.display(),
        shot.width,
        shot.height
    ))
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
    let loaded = workspace::load(&path)?;
    slice(loaded.document, args.value("section"))
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

/// Build HTML options from `--theme`, `--fragment`, `--doc-css`, and `--hidden`.
fn html_options(args: &Args) -> HtmlOptions {
    HtmlOptions {
        theme: theme_of(args),
        fragment: args.present("fragment"),
        include_hidden: args.present("hidden"),
        document_css: args.present("doc-css"),
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
}
