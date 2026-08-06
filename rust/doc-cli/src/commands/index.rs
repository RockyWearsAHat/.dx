//! `dx index` — scaffold a project index document.
//!
//! Writes `index.dx` at the mapped root: a *precursor* index built from nothing but the
//! file tree — one section per top-level area, each holding the area's immediate contents
//! and a TODO paragraph. The scaffold is deliberately shallow: its job is to be read whole
//! and improved by whoever ran it — replace each TODO with what the area actually does,
//! add `::code src=` blocks for the load-bearing files (they render as the file's current
//! text, never a stale copy) — so every later reader orients for the price of one read.

use std::path::{Path, PathBuf};

use crate::args::Args;
use crate::workspace;

/// File name the scaffold is written to, at the mapped root.
pub const INDEX_FILE: &str = "index.dx";

/// Directories the map never descends into: version control, stores, caches, build
/// output, and test fixtures — none of them orient a reader.
const SKIPPED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    "venv",
    "__pycache__",
    "fixture",
    "fixtures",
];

/// Most immediate entries one area lists; the rest are counted rather than named.
const AREA_ENTRY_CAP: usize = 20;

/// What [`write_scaffold`] produced.
#[derive(Debug)]
pub struct Scaffold {
    /// Where the index document was written.
    pub path: PathBuf,
    /// Top-level areas mapped (root files count as one).
    pub areas: usize,
    /// Files counted across the whole tree.
    pub files: usize,
}

/// `dx index [dir] [--force]`.
pub fn run(args: &Args) -> Result<String, String> {
    let root = PathBuf::from(args.positional(0).unwrap_or("."));
    let scaffold = write_scaffold(&root, args.present("force"))?;
    Ok(format!(
        "Wrote {} — {} area(s), {} file(s), from the tree alone. Read it whole and \
         improve it before other work: replace each TODO with what the area does, and \
         add ::code src= blocks for the load-bearing files.\n",
        scaffold.path.display(),
        scaffold.areas,
        scaffold.files
    ))
}

/// Write the scaffold index for `root`, refusing to overwrite one unless `force`.
///
/// The refusal matters: an existing `index.dx` is presumed *improved*, and a scaffold
/// silently replacing it would trade a real map for a file listing.
pub fn write_scaffold(root: &Path, force: bool) -> Result<Scaffold, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let path = root.join(INDEX_FILE);
    if path.exists() && !force {
        return Err(format!(
            "{} already exists — improve it instead, or pass --force to rewrite the \
             scaffold over it.",
            path.display()
        ));
    }

    let (source, areas, files) = scaffold_source(root);
    let document = doc_core::format::parse(&source);
    workspace::save(&path, &document)?;
    Ok(Scaffold { path, areas, files })
}

/// The scaffold's DOCSRC, plus how many areas and files it mapped.
fn scaffold_source(root: &Path) -> (String, usize, usize) {
    let name = root
        .canonicalize()
        .ok()
        .and_then(|real| real.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "project".to_string());

    let mut directories = Vec::new();
    let mut loose_files = Vec::new();
    for entry in listed(root) {
        if entry.is_dir() {
            directories.push(entry);
        } else if entry.file_name().is_some_and(|n| n != INDEX_FILE) {
            loose_files.push(entry);
        }
    }

    let mut body = format!(
        "::heading level=1 id=index\n{name} — project index\n::end\n\n\
         ::paragraph id=index-purpose\n\
         Scaffold from the file tree, written by `dx index`. Improve it: replace each \
         TODO with what the area does and how it connects to the rest, and add \
         `::code src=` blocks for the load-bearing files — they render as the file's \
         current text, never a stale copy. Keep this document true as the code changes.\n\
         ::end\n"
    );

    let mut total = loose_files.len();
    let mut areas = 0;
    let mut used_slugs = Vec::new();

    if !loose_files.is_empty() {
        areas += 1;
        body.push_str(&format!(
            "\n::heading level=2 id=area-root\n./ — {} file(s)\n::end\n\n\
             ::bulleted-list id=area-root-files\n{}::end\n",
            loose_files.len(),
            listing(&loose_files, root)
        ));
    }

    for directory in &directories {
        let files = file_count(directory);
        total += files;
        areas += 1;
        let label = directory
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let slug = unique_slug(&label, &mut used_slugs);
        let children = listed(directory);
        body.push_str(&format!(
            "\n::heading level=2 id=area-{slug}\n{label}/ — {files} file(s)\n::end\n\n\
             ::paragraph id=area-{slug}-notes\n\
             TODO: what {label}/ is for, its entry points, and how it connects to the \
             rest.\n::end\n\n\
             ::bulleted-list id=area-{slug}-files\n{}::end\n",
            listing(&children, root)
        ));
    }

    (body, areas, total)
}

/// One `- ` line per entry, relative to `root`, capped at [`AREA_ENTRY_CAP`] with the
/// remainder counted. A directory line carries its recursive file count.
fn listing(entries: &[PathBuf], root: &Path) -> String {
    let mut lines = String::new();
    for entry in entries.iter().take(AREA_ENTRY_CAP) {
        let relative = entry.strip_prefix(root).unwrap_or(entry).display();
        if entry.is_dir() {
            lines.push_str(&format!("- {relative}/ ({} files)\n", file_count(entry)));
        } else {
            lines.push_str(&format!("- {relative}\n"));
        }
    }
    if entries.len() > AREA_ENTRY_CAP {
        lines.push_str(&format!(
            "- … and {} more entries\n",
            entries.len() - AREA_ENTRY_CAP
        ));
    }
    lines
}

/// The entries of `directory` the map shows: sorted by name, hidden and skipped
/// directories left out.
fn listed(directory: &Path) -> Vec<PathBuf> {
    let Ok(reader) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = reader
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                let skipped = path.is_dir() && SKIPPED_DIRECTORIES.contains(&name.as_ref());
                !name.starts_with('.') && !skipped
            })
        })
        .collect();
    entries.sort();
    entries
}

/// Recursive file count under `directory`, honouring the same skip rules as [`listed`].
fn file_count(directory: &Path) -> usize {
    listed(directory)
        .iter()
        .map(|entry| if entry.is_dir() { file_count(entry) } else { 1 })
        .sum()
}

/// A block-id-safe slug of `label`, made unique against `used` by numbering repeats.
fn unique_slug(label: &str, used: &mut Vec<String>) -> String {
    let base: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "area".to_string()
    } else {
        base
    };

    let mut slug = base.clone();
    let mut counter = 2;
    while used.contains(&slug) {
        slug = format!("{base}-{counter}");
        counter += 1;
    }
    used.push(slug.clone());
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-index-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/deep")).expect("dirs");
        std::fs::write(root.join("README.md"), "hello").expect("file");
        std::fs::write(root.join("src/main.rs"), "fn main() {}").expect("file");
        std::fs::write(root.join("src/deep/util.rs"), "// util").expect("file");
        std::fs::create_dir_all(root.join("node_modules/junk")).expect("dirs");
        std::fs::write(root.join("node_modules/junk/x.js"), "junk").expect("file");
        root
    }

    #[test]
    fn the_scaffold_maps_areas_and_skips_build_junk() {
        let root = project("map");
        let scaffold = write_scaffold(&root, false).expect("scaffold");
        assert_eq!(scaffold.areas, 2); // ./ and src/
        assert_eq!(scaffold.files, 3);

        let text = workspace::read(&scaffold.path).expect("read");
        assert!(text.contains("src/ — 2 file(s)"), "{text}");
        assert!(text.contains("- src/deep/ (1 files)"), "{text}");
        assert!(text.contains("- README.md"), "{text}");
        assert!(text.contains("TODO: what src/ is for"), "{text}");
        assert!(!text.contains("node_modules"), "{text}");
        // The index never lists itself as a loose file.
        assert!(!text.contains("- index.dx"), "{text}");
    }

    #[test]
    fn an_existing_index_is_kept_unless_forced() {
        let root = project("keep");
        write_scaffold(&root, false).expect("first");
        let refused = write_scaffold(&root, false).expect_err("should refuse");
        assert!(refused.contains("--force"), "{refused}");
        write_scaffold(&root, true).expect("forced rewrite");
    }

    #[test]
    fn slugs_are_id_safe_and_unique() {
        let mut used = Vec::new();
        assert_eq!(unique_slug("My Dir!", &mut used), "my-dir");
        assert_eq!(unique_slug("My Dir!", &mut used), "my-dir-2");
        assert_eq!(unique_slug("---", &mut used), "area");
    }

    #[test]
    fn a_missing_directory_is_a_clear_error() {
        let error = write_scaffold(Path::new("/dx/nowhere"), false).expect_err("should fail");
        assert!(error.contains("not a directory"));
    }
}
