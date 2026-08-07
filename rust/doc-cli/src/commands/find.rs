//! Discovery commands: `ls` and `search`.
//!
//! Both walk a directory tree for `.dx` files and report what they find, so an agent
//! dropped into an unfamiliar project can answer "what documentation exists here?" in one
//! call instead of guessing at paths.

use std::path::PathBuf;

use crate::args::Args;
use crate::workspace;

/// Default number of search hits reported.
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// `dx ls [dir]` — every document under a directory, with its title and size.
pub fn run_ls(args: &Args) -> Result<String, String> {
    let root = root_of(args, 0);
    let documents = workspace::load_all(&root);
    if documents.is_empty() {
        return Ok(format!("no .dx documents under {}\n", root.display()));
    }

    let width = documents
        .iter()
        .map(|loaded| loaded.relative.len())
        .max()
        .unwrap_or(4);
    let mut out = String::new();
    for loaded in &documents {
        out.push_str(&format!(
            "{:<width$}  {}  ({} blocks)\n",
            loaded.relative,
            loaded.title(),
            loaded.document.blocks.len(),
            width = width
        ));
    }
    Ok(out)
}

/// `dx search <query> [dir]` — documents matching a query, best first.
pub fn run_search(args: &Args) -> Result<String, String> {
    let query = args
        .positional(0)
        .ok_or_else(|| "a search query is required".to_string())?;
    let root = root_of(args, 1);
    let limit = args
        .number("limit")
        .map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);

    let hits = workspace::search(&root, query, limit);
    if hits.is_empty() {
        return Ok(format!("no documents match `{query}`\n"));
    }

    let mut out = String::new();
    for hit in &hits {
        out.push_str(&format!(
            "{:.3}  {}  {}\n",
            hit.score,
            hit.document.relative,
            hit.document.title()
        ));
        // The hit carries its answer: the best-matching block's first line, and the id
        // to read the rest with `dx text <path> --section <id>`.
        if let Some(id) = &hit.block {
            if let Some(line) = first_line_of(&hit.document.document, id) {
                out.push_str(&format!("       #{id}  {line}\n"));
            }
        }
    }
    Ok(out)
}

/// The first non-empty line of one block's text, for the hit's answer line.
fn first_line_of(document: &doc_core::model::Document, id: &str) -> Option<String> {
    let scoped = doc_core::render::section(document, id)?;
    let text = doc_core::render::text(&scoped, &doc_core::render::TextOptions::default());
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
}

/// The directory to search: positional `index` if given, else the current directory.
fn root_of(args: &Args, index: usize) -> PathBuf {
    args.positional(index)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::format::parse;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    fn project(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-find-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        workspace::save(
            &root.join("guide.dx"),
            &parse("::heading level=1 id=h\nDeployment Guide\n::end\n\n::paragraph id=p\nkubernetes rollout steps\n::end\n"),
        )
        .expect("guide");
        workspace::save(
            &root.join("notes/misc.dx"),
            &parse("::heading level=1 id=h\nMisc Notes\n::end\n"),
        )
        .expect("misc");
        root
    }

    #[test]
    fn ls_reports_every_document_with_its_title() {
        let root = project("ls");
        let out = run_ls(&args(&[&root.to_string_lossy()])).expect("ls");
        assert!(out.contains("guide.dx"));
        assert!(out.contains("Deployment Guide"));
        assert!(out.contains("misc.dx"));
        assert_eq!(out.lines().count(), 2);
    }

    #[test]
    fn ls_of_an_empty_directory_says_so_instead_of_printing_nothing() {
        let root = std::env::temp_dir().join("dx-find-tests-empty");
        std::fs::create_dir_all(&root).expect("root");
        let out = run_ls(&args(&[&root.to_string_lossy()])).expect("ls");
        assert!(out.contains("no .dx documents"));
    }

    #[test]
    fn search_finds_the_matching_document_and_ranks_it() {
        let root = project("search");
        let out = run_search(&args(&["kubernetes", &root.to_string_lossy()])).expect("search");
        assert!(out.contains("guide.dx"));
        assert!(!out.contains("misc.dx"));
        // The hit carries its answer line: the matching block's id and first line.
        assert!(out.contains("#p"));
        assert!(out.contains("kubernetes rollout steps"));
    }

    /// `--limit` bounds the hits, which is how a caller asks for the best few instead of
    /// every match. It reached this function all along; the command table refused it by
    /// name, so nobody could type it.
    #[test]
    fn search_honors_the_limit_it_is_given() {
        let root = std::env::temp_dir().join("dx-find-tests-limit");
        let _ = std::fs::remove_dir_all(&root);
        for name in ["a", "b", "c"] {
            workspace::save(
                &root.join(format!("{name}.dx")),
                &parse(&format!(
                    "::heading level=1 id=h\nRollout {name}\n::end\n\n::paragraph id=p\nkubernetes rollout steps\n::end\n"
                )),
            )
            .expect("seed");
        }
        let directory = root.to_string_lossy().into_owned();

        // A hit line starts with its score; the indented answer line under it does not count.
        let hit_lines = |out: &str| out.lines().filter(|line| !line.starts_with(' ')).count();

        let all = run_search(&args(&["kubernetes", &directory])).expect("search");
        assert_eq!(hit_lines(&all), 3);

        let bounded =
            run_search(&args(&["kubernetes", &directory, "--limit", "2"])).expect("search");
        assert_eq!(hit_lines(&bounded), 2);
    }

    #[test]
    fn search_with_no_match_reports_the_query() {
        let root = project("search-empty");
        let out = run_search(&args(&["quasar", &root.to_string_lossy()])).expect("search");
        assert!(out.contains("no documents match `quasar`"));
    }

    #[test]
    fn search_requires_a_query() {
        assert!(run_search(&args(&[])).unwrap_err().contains("query"));
    }
}
