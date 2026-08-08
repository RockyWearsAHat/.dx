//! `dx play` — drive the rendered page and write what happened, frame by frame.
//!
//! Where `dx png` photographs a document, `dx play` handles it: the page is loaded live,
//! a script of inputs is performed against it — wait, key, click, scroll, hover — and one
//! PNG per tick lands in a directory, each listed in the report with its moment and the
//! action it shows. `--node` clips every frame to one block's box and reads the script's
//! `x,y` targets inside that same box — clip and coordinates share one frame of
//! reference — while without it `x,y` is viewport-absolute. Nothing the document carries
//! executes; this is the same script-free render every screenshot uses, with synthetic
//! input dispatched at it.

use std::path::PathBuf;

use doc_core::render::Theme;
use doc_shot::play::{play, PlayOptions};

use crate::args::Args;
use crate::commands::view;

/// `dx play <file> --script "…" [--node ID] [--fps N] [--out DIR]`.
pub fn run(args: &Args) -> Result<String, String> {
    let script = args
        .value("script")
        .ok_or("a --script is required — e.g. --script \"wait 500ms; key Space; scroll 200\"")?;
    let path = view::document_path(args)?;
    let document = view::selected_document(args)?;

    let options = PlayOptions {
        width: args.number("width").unwrap_or(doc_shot::DEFAULT_WIDTH),
        height: args
            .number("height")
            .unwrap_or(doc_shot::play::DEFAULT_HEIGHT),
        theme: Theme::parse(args.value("theme").unwrap_or("auto")),
        fps: args.number("fps").unwrap_or(doc_shot::play::DEFAULT_FPS),
        node: args.value("node").map(str::to_string),
        ..PlayOptions::default()
    };
    let frames = play(&document, script, &options)?;

    let directory = args
        .value("out")
        .map_or_else(|| frames_directory(&path), PathBuf::from);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    let mut out = format!(
        "played {} — {} frames → {}\n",
        path.display(),
        frames.len(),
        directory.display()
    );
    for (index, frame) in frames.iter().enumerate() {
        let name = format!("frame-{index:03}.png");
        let target = directory.join(&name);
        std::fs::write(&target, &frame.png)
            .map_err(|error| format!("could not write {}: {error}", target.display()))?;
        out.push_str(&format!(
            "  {name}  {:>6}ms{}\n",
            frame.at_ms,
            frame
                .note
                .as_deref()
                .map(|note| format!("  {note}"))
                .unwrap_or_default()
        ));
    }
    Ok(out)
}

/// The default frame directory: `notes.dx` plays into `notes-frames/` beside it.
fn frames_directory(path: &std::path::Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    path.with_file_name(format!("{stem}-frames"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_missing_script_is_refused_with_an_example() {
        let error = run(&args(&["/dx/nowhere.dx"])).expect_err("should refuse");
        assert!(error.contains("--script"), "{error}");
        assert!(error.contains("key Space"), "{error}");
    }

    #[test]
    fn frames_land_beside_the_document_by_default() {
        assert_eq!(
            frames_directory(std::path::Path::new("dir/notes.dx")),
            PathBuf::from("dir/notes-frames")
        );
    }

    /// The whole command against a real browser: parse, play, write, report.
    #[test]
    fn a_played_document_writes_its_frames_and_reports_each_one() {
        if doc_shot::browser::find().is_none() {
            return;
        }
        let root = std::env::temp_dir().join("dx-play-cli-test");
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("doc.dx");
        crate::workspace::save(
            &path,
            &doc_core::format::parse("::paragraph id=only\nOne line.\n::end\n"),
        )
        .expect("fixture");

        let file = path.to_string_lossy().into_owned();
        let report = run(&args(&[
            &file,
            "--script",
            "wait 150ms; key Space",
            "--fps",
            "10",
        ]))
        .expect("play");

        assert!(report.contains("frame-000.png"), "{report}");
        assert!(report.contains("key Space"), "{report}");
        let first = root.join("doc-frames/frame-000.png");
        let bytes = std::fs::read(&first).expect("first frame written");
        assert_eq!(&bytes[1..4], b"PNG");
    }
}
