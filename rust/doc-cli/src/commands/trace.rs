//! `dx trace [dir] [--brief]` — a real symbol + reference index over a tree, printed as a
//! report. `doc_core::trace` does the extraction (see its module doc for the technique
//! and its honest limits — regex/line heuristics, not a parser); this file only walks
//! the tree, reads the files, and formats the result.
//!
//! Two shapes, one trace:
//! - **The full report** (default) is a lookup table — one entry per defined symbol,
//!   its definition site, and the files that reference it — shaped like the prior art
//!   this mirrors, `helpers index lookup <name>`.
//! - **`--brief`** ranks symbols by fan-in and caps the list, shaped like `helpers index
//!   map`'s "Top modules" summary. This is the form meant to be embedded in a document:
//!   `dx index`'s scaffold wires it into a run block (`commands::index`) so a project's
//!   traced index re-derives itself whenever the tree changes, the same way the cargo
//!   gates re-derive a green test run.

use std::path::{Path, PathBuf};

use crate::args::Args;
use crate::commands::index::{collect_files, CODE_EXTENSIONS};

use doc_core::trace::{trace, Trace};

/// Largest file the tracer reads; anything bigger is skipped rather than risking a slow
/// read of a generated or vendored blob that slipped past the directory skip list —
/// mirrors `commands::index::SURVEY_READ_CAP`'s reasoning at a slightly larger size,
/// since a trace is read on demand rather than on every scaffold.
const READ_CAP: u64 = 2 * 1024 * 1024;

/// Referencing files shown per symbol in the full report before the rest are counted
/// instead — keeps one widely-referenced symbol's entry from swamping the page.
const REFERENCE_CAP: usize = 20;

/// Symbols ranked in `--brief` output, sized to embed in a scaffolded document — the
/// same role `commands::index::AREA_ENTRY_CAP` plays for one area's file listing.
const BRIEF_CAP: usize = 25;

/// `dx trace [dir] [--brief]`.
pub fn run(args: &Args) -> Result<String, String> {
    let root = PathBuf::from(args.positional(0).unwrap_or("."));
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }

    let traced = trace(&read_files(&root));
    Ok(if args.present("brief") {
        brief_report(&traced)
    } else {
        full_report(&traced)
    })
}

/// Every code file under `root` (by [`CODE_EXTENSIONS`], the same set `dx index`'s
/// survey reads), as `(path relative to root, text)` pairs — bounded to [`READ_CAP`]
/// bytes and silently skipping anything not valid UTF-8, the same tolerance the survey
/// already extends to a tree it does not control the contents of.
fn read_files(root: &Path) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    collect_files(root, &mut paths);
    paths
        .into_iter()
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                CODE_EXTENSIONS.contains(&extension.to_string_lossy().as_ref())
            })
        })
        .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.len() <= READ_CAP))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            Some((relative, text))
        })
        .collect()
}

/// The full report: every symbol found, sorted by definition site, each with its
/// definition line and the files that reference it.
fn full_report(traced: &Trace) -> String {
    if traced.symbols.is_empty() {
        return "no symbols found\n".to_string();
    }

    let mut symbols: Vec<&doc_core::trace::Symbol> = traced.symbols.iter().collect();
    symbols.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    let mut out = String::new();
    for symbol in symbols {
        out.push_str(&format!(
            "{} ({}) — {}:{}\n",
            symbol.name,
            symbol.kind.label(),
            symbol.file,
            symbol.line
        ));
        let mut references = traced.references_to(&symbol.name);
        references.sort_by(|a, b| a.file.cmp(&b.file));
        if references.is_empty() {
            out.push_str("  not referenced by any other traced file\n");
            continue;
        }
        let files: Vec<&str> = references.iter().map(|r| r.file.as_str()).collect();
        let shown = files.len().min(REFERENCE_CAP);
        out.push_str(&format!("  referenced by {}", files[..shown].join(", ")));
        if files.len() > shown {
            out.push_str(&format!(", and {} more", files.len() - shown));
        }
        out.push('\n');
    }
    out
}

/// `--brief`: symbols ranked by fan-in, capped at [`BRIEF_CAP`] — the compact form a
/// scaffolded document embeds, shaped like `helpers index map`'s "Top modules" list.
fn brief_report(traced: &Trace) -> String {
    let ranked = traced.ranked_by_fan_in();
    if ranked.is_empty() {
        return "no referenced symbols found\n".to_string();
    }

    let mut out = String::from("Top symbols by fan-in:\n");
    for (name, fan_in) in ranked.iter().take(BRIEF_CAP) {
        let site = traced
            .definitions_of(name)
            .into_iter()
            .min_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        match site {
            Some(symbol) => out.push_str(&format!(
                "- {name} ({}) — {}:{} — referenced by {}\n",
                symbol.kind.label(),
                symbol.file,
                symbol.line,
                counted_files(*fan_in)
            )),
            None => out.push_str(&format!(
                "- {name} — referenced by {}\n",
                counted_files(*fan_in)
            )),
        }
    }
    if ranked.len() > BRIEF_CAP {
        out.push_str(&format!("… and {} more\n", ranked.len() - BRIEF_CAP));
    }
    out
}

/// `1 file`, `3 files` — the count with its noun, singular exactly at one.
fn counted_files(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn project(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-trace-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn scaffold() -> bool {\n    true\n}\n",
        )
        .expect("lib");
        std::fs::write(
            root.join("src/main.rs"),
            "fn main() {\n    scaffold();\n}\n",
        )
        .expect("main");
        root
    }

    #[test]
    fn the_full_report_shows_the_definition_and_who_references_it() {
        let root = project("full");
        let out = run(&args(&[&root.to_string_lossy()])).expect("trace");
        assert!(out.contains("scaffold (function) — src/lib.rs:1"), "{out}");
        assert!(out.contains("referenced by src/main.rs"), "{out}");
    }

    #[test]
    fn brief_ranks_by_fan_in() {
        let root = project("brief");
        let out = run(&args(&[&root.to_string_lossy(), "--brief"])).expect("trace");
        assert!(out.starts_with("Top symbols by fan-in:\n"), "{out}");
        assert!(out.contains("scaffold"), "{out}");
        assert!(out.contains("referenced by 1 file"), "{out}");
    }

    #[test]
    fn an_unreferenced_symbol_says_so() {
        let root = std::env::temp_dir().join("dx-trace-tests-lonely");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("solo.rs"), "pub fn alone() {}\n").expect("solo");
        let out = run(&args(&[&root.to_string_lossy()])).expect("trace");
        assert!(
            out.contains("not referenced by any other traced file"),
            "{out}"
        );
    }

    #[test]
    fn a_tree_with_no_code_reports_no_symbols() {
        let root = std::env::temp_dir().join("dx-trace-tests-empty");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("README.md"), "hello\n").expect("readme");
        let out = run(&args(&[&root.to_string_lossy()])).expect("trace");
        assert_eq!(out, "no symbols found\n");
        let brief = run(&args(&[&root.to_string_lossy(), "--brief"])).expect("trace");
        assert_eq!(brief, "no referenced symbols found\n");
    }

    /// worklist item 11: Objective-C++ (`.mm`), Metal (`.metal`), and assembly (`.S`)
    /// files were once dropped entirely by [`CODE_EXTENSIONS`], so a project mixing them
    /// with Rust/C got no references from its GPU/native half at all — not even the
    /// reference-only coverage a non-covered-but-listed language already gets (mirrors
    /// `doc_core::trace`'s `a_non_covered_language_still_contributes_references_but_no_definitions`).
    #[test]
    fn objective_c_metal_and_assembly_files_are_read_and_contribute_references() {
        let root = std::env::temp_dir().join("dx-trace-tests-native-mix");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("lib.rs"), "pub fn distinctive_gpu_symbol() {}\n").expect("lib");
        std::fs::write(
            root.join("glue.mm"),
            "void bridge() { distinctive_gpu_symbol(); }\n",
        )
        .expect("mm");
        std::fs::write(
            root.join("shader.metal"),
            "void kernel_entry() { distinctive_gpu_symbol(); }\n",
        )
        .expect("metal");
        std::fs::write(root.join("routine.S"), "call distinctive_gpu_symbol\n").expect("asm");

        let out = run(&args(&[&root.to_string_lossy()])).expect("trace");
        assert!(out.contains("referenced by"), "{out}");
        for file in ["glue.mm", "shader.metal", "routine.S"] {
            assert!(
                out.contains(file),
                "{file} should be traced for references: {out}"
            );
        }
    }

    #[test]
    fn a_missing_directory_is_a_clear_error() {
        let error = run(&args(&["/dx/nowhere"])).expect_err("should fail");
        assert!(error.contains("not a directory"), "{error}");
    }
}
