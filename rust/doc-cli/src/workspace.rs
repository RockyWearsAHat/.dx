//! Reading and writing `.dx` files, and finding them in a project.
//!
//! # `.dx` files are ordinary text files
//! This is the rule the whole platform rests on. A `.dx` file on disk is its own content —
//! plain DOCSRC text, readable with `cat`, searchable with `grep`, diffable in a pull
//! request, and editable by any tool or agent that can open a file. There is no database
//! to be in sync with and no pointer to dereference. Everything else here (rendering,
//! screenshots, execution, the MCP server) is a *view* over these bytes, never a
//! replacement for them.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use doc_core::format::{parse, stringify};
use doc_core::model::Document;
use doc_core::search::build_index;

/// Directories never walked when looking for documents.
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    ".venv",
    "__pycache__",
    ".doc",
];

/// A document loaded from disk, with the path it came from.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Path relative to the search root, for display.
    pub relative: String,
    /// The parsed document.
    pub document: Document,
}

impl Loaded {
    /// The document's display title: its metadata title, else its first heading, else its
    /// file name.
    #[must_use]
    pub fn title(&self) -> String {
        for candidate in [
            self.document.title.trim(),
            self.document.first_heading_text().trim(),
        ] {
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.relative.clone())
    }
}

/// Read and parse the document at `path`.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let source = read(path)?;
    Ok(Loaded {
        path: path.to_path_buf(),
        relative: path.to_string_lossy().into_owned(),
        document: parse(&source),
    })
}

/// Read a file's text, or `-` for standard input.
pub fn read(path: &Path) -> Result<String, String> {
    if path == Path::new("-") {
        let mut buffer = String::new();
        return std::io::stdin()
            .read_to_string(&mut buffer)
            .map(|_| buffer)
            .map_err(|error| format!("could not read standard input: {error}"));
    }
    fs::read_to_string(path).map_err(|error| format!("could not read {}: {error}", path.display()))
}

/// Write canonical DOCSRC for `document` to `path`, creating parent directories.
///
/// Writing always goes through [`stringify`], so a file this tool touches is left in
/// canonical form — the property that keeps two different editors from fighting over
/// formatting.
pub fn save(path: &Path, document: &Document) -> Result<(), String> {
    write_text(path, &stringify(document))
}

/// Write raw text to `path`, creating parent directories as needed.
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
    }
    fs::write(path, text).map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// The directory a document lives in, used as the working directory for its code blocks.
#[must_use]
pub fn document_dir(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Find every `.dx` file under `root`, sorted by path.
///
/// Build outputs, dependency directories, and version-control metadata are skipped, so
/// listing a project returns the documents a person wrote, not the ones a tool generated.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort();
    found
}

/// Recursively collect `.dx` files, skipping uninteresting directories.
fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if path.is_dir() {
            let skipped = name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name.as_str());
            if !skipped {
                walk(&path, found);
            }
            continue;
        }
        if path.extension().is_some_and(|extension| extension == "dx") {
            found.push(path);
        }
    }
}

/// Load every document under `root`, ignoring files that cannot be read.
#[must_use]
pub fn load_all(root: &Path) -> Vec<Loaded> {
    discover(root)
        .into_iter()
        .filter_map(|path| {
            let source = fs::read_to_string(&path).ok()?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            Some(Loaded {
                document: parse(&source),
                path,
                relative,
            })
        })
        .collect()
}

/// One search hit: a document and why it matched.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The matching document.
    pub document: Loaded,
    /// Relevance score from the index; higher is a better match.
    pub score: f64,
}

/// Search every document under `root` for `query`, best matches first.
#[must_use]
pub fn search(root: &Path, query: &str, limit: usize) -> Vec<Hit> {
    let documents = load_all(root);
    let indexed: Vec<(String, Document)> = documents
        .iter()
        .map(|loaded| (loaded.relative.clone(), loaded.document.clone()))
        .collect();
    let index = build_index(&indexed);

    index
        .search(query)
        .into_iter()
        .take(limit)
        .filter_map(|result| {
            documents
                .iter()
                .find(|loaded| loaded.relative == result.path)
                .map(|loaded| Hit {
                    document: loaded.clone(),
                    score: result.score,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-workspace-tests-{label}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");
        root
    }

    #[test]
    fn a_saved_document_is_plain_readable_text() {
        let root = scratch("plain-text");
        let path = root.join("notes.dx");
        save(&path, &parse("::heading level=1 id=h\nNotes\n::end\n")).expect("save");

        let raw = fs::read_to_string(&path).expect("read back");
        assert_eq!(raw, "::heading level=1 id=h\nNotes\n::end\n");
        assert!(
            raw.contains("Notes"),
            "content must be legible without tooling"
        );
    }

    #[test]
    fn saving_canonicalizes_sloppy_input() {
        let root = scratch("canonical");
        let path = root.join("messy.dx");
        save(&path, &parse("::heading level=2 id=h Hello ::end\n")).expect("save");
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "::heading level=2 id=h\nHello\n::end\n"
        );
    }

    #[test]
    fn discovery_finds_documents_and_skips_build_output() {
        let root = scratch("discover");
        save(&root.join("a.dx"), &parse("::paragraph id=p\na\n::end\n")).expect("a");
        save(
            &root.join("deep/b.dx"),
            &parse("::paragraph id=p\nb\n::end\n"),
        )
        .expect("b");
        save(
            &root.join("node_modules/c.dx"),
            &parse("::paragraph id=p\nc\n::end\n"),
        )
        .expect("c");
        fs::write(root.join("d.txt"), "not a document").expect("txt");

        let found = discover(&root);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|path| path.extension().unwrap() == "dx"));
        assert!(!found
            .iter()
            .any(|path| path.to_string_lossy().contains("node_modules")));
    }

    #[test]
    fn search_ranks_the_document_that_mentions_the_term() {
        let root = scratch("search");
        save(
            &root.join("kubernetes.dx"),
            &parse("::paragraph id=p\nkubernetes scheduling notes\n::end\n"),
        )
        .expect("k");
        save(
            &root.join("recipes.dx"),
            &parse("::paragraph id=p\nbread and soup\n::end\n"),
        )
        .expect("r");

        let hits = search(&root, "kubernetes", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.relative, "kubernetes.dx");
    }

    #[test]
    fn titles_fall_back_from_metadata_to_heading_to_file_name() {
        let root = scratch("titles");
        let with_heading = root.join("h.dx");
        save(
            &with_heading,
            &parse("::heading level=1 id=h\nReal Title\n::end\n"),
        )
        .expect("save");
        assert_eq!(load(&with_heading).expect("load").title(), "Real Title");

        let no_heading = root.join("plain-name.dx");
        save(&no_heading, &parse("::paragraph id=p\nbody\n::end\n")).expect("save");
        assert_eq!(load(&no_heading).expect("load").title(), "plain-name");
    }

    #[test]
    fn reading_a_missing_file_explains_which_one() {
        let error = load(Path::new("/dx/definitely/missing.dx")).expect_err("should fail");
        assert!(error.contains("missing.dx"));
    }

    #[test]
    fn document_dir_is_where_a_blocks_relative_paths_resolve() {
        assert_eq!(document_dir(Path::new("/a/b/c.dx")), PathBuf::from("/a/b"));
        assert_eq!(document_dir(Path::new("c.dx")), PathBuf::from("."));
    }
}
