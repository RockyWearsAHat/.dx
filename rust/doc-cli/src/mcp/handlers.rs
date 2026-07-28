//! What each MCP tool actually does.
//!
//! Every handler takes the tool's arguments and returns MCP `content` items. They are pure
//! functions of the arguments plus the filesystem, with no session or cursor state, so an
//! agent can call them in any order and a dropped connection loses nothing.

use std::path::{Path, PathBuf};

use doc_core::format::parse;
use doc_core::model::Document;
use doc_core::render::{html, outline, section, text, HtmlOptions, TextOptions, Theme};
use doc_run::{run_document, RunOptions};
use doc_shot::{capture, ShotOptions};
use serde_json::{json, Value};

use super::encode::base64;
use crate::workspace;

/// Default number of search results returned.
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// One tool call's outcome: the content items to return.
pub type ToolResult = Result<Vec<Value>, String>;

/// Dispatch a tool call by name.
pub fn call(name: &str, args: &Value, root: &Path) -> ToolResult {
    match name {
        "dx_view" => view(args, root),
        "dx_read" => read(args, root),
        "dx_outline" => outline_of(args, root),
        "dx_list" => list(args, root),
        "dx_search" => search(args, root),
        "dx_render" => render(args, root),
        "dx_write" => write(args, root),
        "dx_edit" => edit(args, root),
        "dx_run" => run(args, root),
        other => Err(format!("unknown tool: {other}")),
    }
}

/// `dx_view` — the rendered document as an image, with a text fallback.
fn view(args: &Value, root: &Path) -> ToolResult {
    let document = selected(args, root)?;
    let options = ShotOptions {
        width: number(args, "width").unwrap_or(doc_shot::DEFAULT_WIDTH),
        theme: Theme::parse(string(args, "theme").unwrap_or("auto")),
        ..ShotOptions::default()
    };

    match capture(&document, &options) {
        Ok(shot) => Ok(vec![
            json!({
                "type": "text",
                "text": format!(
                    "Rendered view of {} ({}x{} px).",
                    string(args, "path").unwrap_or("document"),
                    shot.width,
                    shot.height
                )
            }),
            json!({
                "type": "image",
                "data": base64(&shot.png),
                "mimeType": "image/png"
            }),
        ]),
        // No browser on this machine: the reader still gets the document, as text.
        Err(reason) => Ok(vec![text_content(&format!(
            "Could not render an image: {reason}\n\nFalling back to text:\n\n{}",
            text(&document, &TextOptions::default())
        ))]),
    }
}

/// `dx_read` — the document as Markdown.
fn read(args: &Value, root: &Path) -> ToolResult {
    let document = selected(args, root)?;
    Ok(vec![text_content(&text(
        &document,
        &TextOptions {
            include_ids: boolean(args, "ids"),
            ..TextOptions::default()
        },
    ))])
}

/// `dx_outline` — one row per block.
fn outline_of(args: &Value, root: &Path) -> ToolResult {
    let document = document_at(args, root)?;
    let rows: Vec<Value> = outline(&document)
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "kind": row.kind,
                "level": row.level,
                "preview": row.preview,
                "chars": row.chars,
                "runnable": row.runnable,
            })
        })
        .collect();
    Ok(vec![json_content(&json!({ "blocks": rows }))])
}

/// `dx_list` — every document in a directory.
fn list(args: &Value, root: &Path) -> ToolResult {
    let directory = directory_arg(args, root);
    let documents: Vec<Value> = workspace::load_all(&directory)
        .into_iter()
        .map(|loaded| {
            json!({
                "path": loaded.relative,
                "title": loaded.title(),
                "blocks": loaded.document.blocks.len(),
            })
        })
        .collect();
    Ok(vec![json_content(&json!({ "documents": documents }))])
}

/// `dx_search` — documents matching a query.
fn search(args: &Value, root: &Path) -> ToolResult {
    let query = required(args, "query")?;
    let directory = directory_arg(args, root);
    let limit = number(args, "limit").map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);

    let hits: Vec<Value> = workspace::search(&directory, query, limit)
        .into_iter()
        .map(|hit| {
            json!({
                "path": hit.document.relative,
                "title": hit.document.title(),
                "score": hit.score,
            })
        })
        .collect();
    Ok(vec![json_content(&json!({ "matches": hits }))])
}

/// `dx_render` — the HTML page source.
fn render(args: &Value, root: &Path) -> ToolResult {
    let document = selected(args, root)?;
    Ok(vec![text_content(&html(
        &document,
        &HtmlOptions {
            theme: Theme::parse(string(args, "theme").unwrap_or("auto")),
            ..HtmlOptions::default()
        },
    ))])
}

/// `dx_write` — create or replace a document.
fn write(args: &Value, root: &Path) -> ToolResult {
    let path = resolve(required(args, "path")?, root);
    let content = required(args, "content")?;
    let document = parse(content);
    workspace::save(&path, &document)?;
    Ok(vec![text_content(&format!(
        "Wrote {} ({} blocks). Block ids: {}",
        path.display(),
        document.blocks.len(),
        block_ids(&document)
    ))])
}

/// `dx_edit` — replace one block's body.
fn edit(args: &Value, root: &Path) -> ToolResult {
    let path = resolve(required(args, "path")?, root);
    let wanted = required(args, "block")?.trim().trim_start_matches('#');
    let body = required(args, "text")?;

    let mut document = parse(&workspace::read(&path)?);
    let index = document
        .blocks
        .iter()
        .position(|block| block.id.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| {
            format!(
                "no block named `{wanted}` in {}. Available ids: {}",
                path.display(),
                block_ids(&document)
            )
        })?;

    document.blocks[index].text = body.to_string();
    workspace::save(&path, &document)?;
    Ok(vec![text_content(&format!(
        "Updated `{wanted}` in {}.",
        path.display()
    ))])
}

/// `dx_run` — execute the document's runnable blocks.
fn run(args: &Value, root: &Path) -> ToolResult {
    let path = resolve(required(args, "path")?, root);
    let source = workspace::read(&path)?;

    let report = run_document(
        &source,
        &RunOptions {
            document_dir: workspace::document_dir(&path),
            force: boolean(args, "force"),
            only: string(args, "block").map(str::to_string),
            ..RunOptions::default()
        },
    );

    if report.changed {
        workspace::write_text(&path, &report.source)?;
    }

    let results: Vec<Value> = report
        .runs
        .iter()
        .map(|entry| {
            json!({
                "block": entry.id,
                "language": entry.language,
                "status": entry.status,
                "exit": entry.exit,
                "ms": entry.duration_ms,
                "output": entry.output,
            })
        })
        .collect();

    Ok(vec![json_content(&json!({
        "executed": report.executed(),
        "allSucceeded": report.all_succeeded(),
        "saved": report.changed,
        "results": results,
        "next": "Call dx_view on this path to see the results rendered.",
    }))])
}

/// Load the document named by `path`, sliced by an optional `section`.
fn selected(args: &Value, root: &Path) -> Result<Document, String> {
    let document = document_at(args, root)?;
    match string(args, "section") {
        None => Ok(document),
        Some(id) => section(&document, id).ok_or_else(|| {
            format!(
                "no block named `{id}`. Available ids: {}",
                block_ids(&document)
            )
        }),
    }
}

/// Load the whole document named by `path`.
fn document_at(args: &Value, root: &Path) -> Result<Document, String> {
    let path = resolve(required(args, "path")?, root);
    Ok(parse(&workspace::read(&path)?))
}

/// A comma-separated list of a document's block ids, for error messages.
fn block_ids(document: &Document) -> String {
    document
        .blocks
        .iter()
        .map(|block| block.id.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve a caller-supplied path against the workspace root.
fn resolve(path: &str, root: &Path) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    }
}

/// The `directory` argument, defaulting to the workspace root.
fn directory_arg(args: &Value, root: &Path) -> PathBuf {
    string(args, "directory").map_or_else(|| root.to_path_buf(), |value| resolve(value, root))
}

/// A required string argument.
fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    string(args, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("`{key}` is required"))
}

/// An optional string argument.
fn string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// An optional numeric argument.
fn number(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|value| value as u32)
}

/// An optional boolean argument, defaulting to false.
fn boolean(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A text content item.
fn text_content(body: &str) -> Value {
    json!({ "type": "text", "text": body })
}

/// A JSON payload delivered as text content, which is how MCP carries structured results.
fn json_content(value: &Value) -> Value {
    text_content(&serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-mcp-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        workspace::write_text(
            &root.join("guide.dx"),
            "::heading level=1 id=title\nGuide\n::end\n\n\
::heading level=2 id=setup\nSetup\n::end\n\n\
::paragraph id=setup-body\nInstall it first.\n::end\n\n\
::heading level=2 id=usage\nUsage\n::end\n\n\
::paragraph id=usage-body\nThen run it.\n::end\n",
        )
        .expect("seed");
        root
    }

    fn text_of(items: &[Value]) -> String {
        items
            .iter()
            .filter(|item| item["type"] == "text")
            .map(|item| item["text"].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn read_returns_the_document_as_markdown() {
        let root = project("read");
        let items = call("dx_read", &json!({ "path": "guide.dx" }), &root).expect("read");
        let body = text_of(&items);
        assert!(body.contains("# Guide"));
        assert!(body.contains("Then run it."));
    }

    #[test]
    fn a_section_narrows_the_result() {
        let root = project("section");
        let body = text_of(
            &call(
                "dx_read",
                &json!({ "path": "guide.dx", "section": "setup" }),
                &root,
            )
            .expect("read"),
        );
        assert!(body.contains("Install it first."));
        assert!(!body.contains("Then run it."));
    }

    #[test]
    fn an_unknown_section_lists_the_real_ids() {
        let root = project("bad-section");
        let error = call(
            "dx_read",
            &json!({ "path": "guide.dx", "section": "nope" }),
            &root,
        )
        .expect_err("should fail");
        assert!(error.contains("no block named `nope`"));
        assert!(error.contains("setup"));
    }

    #[test]
    fn outline_reports_ids_and_runnability() {
        let root = project("outline");
        let body =
            text_of(&call("dx_outline", &json!({ "path": "guide.dx" }), &root).expect("out"));
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["blocks"][0]["id"], "title");
        assert_eq!(parsed["blocks"][0]["level"], 1);
        assert_eq!(parsed["blocks"][0]["runnable"], false);
    }

    #[test]
    fn write_creates_a_plain_text_file_and_reports_its_ids() {
        let root = project("write");
        let items = call(
            "dx_write",
            &json!({ "path": "new.dx", "content": "::paragraph id=hello\nHi there\n::end\n" }),
            &root,
        )
        .expect("write");
        assert!(text_of(&items).contains("hello"));
        assert_eq!(
            std::fs::read_to_string(root.join("new.dx")).expect("read"),
            "::paragraph id=hello\nHi there\n::end\n"
        );
    }

    #[test]
    fn edit_changes_one_block_and_leaves_the_rest_alone() {
        let root = project("edit");
        call(
            "dx_edit",
            &json!({ "path": "guide.dx", "block": "setup-body", "text": "Install it second." }),
            &root,
        )
        .expect("edit");

        let saved = std::fs::read_to_string(root.join("guide.dx")).expect("read");
        assert!(saved.contains("Install it second."));
        assert!(saved.contains("::paragraph id=usage-body\nThen run it.\n::end"));
    }

    #[test]
    fn edit_of_a_missing_block_names_the_ones_that_exist() {
        let root = project("edit-missing");
        let error = call(
            "dx_edit",
            &json!({ "path": "guide.dx", "block": "ghost", "text": "x" }),
            &root,
        )
        .expect_err("should fail");
        assert!(error.contains("ghost"));
        assert!(error.contains("usage-body"));
    }

    #[test]
    fn list_and_search_find_documents_in_the_project() {
        let root = project("find");
        let listed = text_of(&call("dx_list", &json!({}), &root).expect("list"));
        assert!(listed.contains("guide.dx"));

        let found = text_of(&call("dx_search", &json!({ "query": "install" }), &root).expect("s"));
        assert!(found.contains("guide.dx"));
    }

    #[test]
    fn run_executes_a_block_and_stores_the_result() {
        let root = project("run");
        workspace::write_text(
            &root.join("runnable.dx"),
            "::code id=hi lang=bash run\necho from-mcp\n::end\n",
        )
        .expect("seed");

        let body = text_of(&call("dx_run", &json!({ "path": "runnable.dx" }), &root).expect("run"));
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["allSucceeded"], true);
        assert_eq!(parsed["results"][0]["output"], "from-mcp");
        assert!(std::fs::read_to_string(root.join("runnable.dx"))
            .expect("read")
            .contains("from-mcp"));
    }

    #[test]
    fn view_always_returns_something_readable() {
        let root = project("view");
        let items = call("dx_view", &json!({ "path": "guide.dx" }), &root).expect("view");
        let has_image = items.iter().any(|item| item["type"] == "image");
        let has_text = items.iter().any(|item| item["type"] == "text");
        assert!(has_text, "view must always say what it returned");
        if has_image {
            let data = items
                .iter()
                .find(|item| item["type"] == "image")
                .and_then(|item| item["data"].as_str())
                .expect("image data");
            assert!(data.starts_with("iVBOR"), "not a PNG payload");
        } else {
            assert!(
                text_of(&items).contains("Guide"),
                "fallback must carry content"
            );
        }
    }

    #[test]
    fn a_missing_required_argument_is_a_clear_error() {
        let root = project("missing-args");
        assert!(call("dx_read", &json!({}), &root)
            .unwrap_err()
            .contains("`path` is required"));
        assert!(call("dx_search", &json!({}), &root)
            .unwrap_err()
            .contains("`query` is required"));
    }

    #[test]
    fn an_unknown_tool_is_rejected_by_name() {
        let root = project("unknown");
        assert!(call("dx_nope", &json!({}), &root)
            .unwrap_err()
            .contains("dx_nope"));
    }

    #[test]
    fn relative_paths_resolve_against_the_workspace_root() {
        let root = project("resolve");
        assert_eq!(resolve("a.dx", &root), root.join("a.dx"));
        assert_eq!(resolve("/abs/a.dx", &root), PathBuf::from("/abs/a.dx"));
    }
}
