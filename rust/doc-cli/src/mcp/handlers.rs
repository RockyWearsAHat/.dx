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
use doc_shot::base64::encode as base64;
use doc_shot::play::{PlayFrame, PlayOptions};
use doc_shot::{capture_pages, ShotOptions};
use serde_json::{json, Value};

use crate::workspace;

/// Default number of search results returned.
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// Most frames one `dx_play` answer carries. A play that captured more is thinned to
/// this budget — action frames kept first — and the answer says so.
const PLAY_FRAME_BUDGET: usize = 12;

/// One tool call's outcome: the content items to return.
pub type ToolResult = Result<Vec<Value>, String>;

/// Dispatch a tool call by name.
pub fn call(name: &str, args: &Value, root: &Path) -> ToolResult {
    match name {
        "dx_read" => read(args, root),
        "dx_play" => play_frames(args, root),
        "dx_source" => source(args, root),
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

/// `dx_read` — the document as the pages of its rendered page, in order.
///
/// Reading a document means *looking* at it. A `.dx` document renders to a page, and what
/// is on that page — a table's alignment, a chart a code block drew, where the eye lands —
/// is in the rendering, not in the source. So a read returns pictures: one per page, each
/// labelled with the block ids on it so the reader can come back for one part
/// (`section`) instead of the next picture.
///
/// A machine with no browser still gets the document, as Markdown. That is a degraded
/// read, not a failure, and it says so.
fn read(args: &Value, root: &Path) -> ToolResult {
    let document = selected(args, root)?;
    let path = string(args, "path").unwrap_or("document");
    // Pages sized for the reader this tool serves: a vision model. `for_reading` keeps
    // every page under the limits past which ingestion would downscale it, so the image
    // the model sees is the image the browser captured, pixel for pixel.
    let options = ShotOptions {
        theme: Theme::parse(string(args, "theme").unwrap_or("auto")),
        ..ShotOptions::for_reading(number(args, "width"))
    };

    let pages = match capture_pages(&document, &options) {
        Ok(pages) if !pages.is_empty() => pages,
        // No browser, or nothing captured: the reader still gets the document, as text.
        Ok(_) => {
            return Ok(vec![text_content(&fallback_text(
                &document,
                "captured no pages",
            ))])
        }
        Err(reason) => return Ok(vec![text_content(&fallback_text(&document, &reason))]),
    };

    let total = pages[0].total;
    let mut content = vec![text_content(&opening_line(path, &pages, total))];
    for page in &pages {
        content.push(text_content(&format!(
            "Page {} of {total}{}",
            page.number,
            if page.blocks.is_empty() {
                String::new()
            } else {
                format!(" — blocks: {}", page.blocks.join(", "))
            }
        )));
        content.push(json!({
            "type": "image",
            "data": base64(&page.shot.png),
            "mimeType": "image/png"
        }));
    }
    Ok(content)
}

/// The line that opens a read: what was rendered, and what to do about what was not.
fn opening_line(path: &str, pages: &[doc_shot::Page], total: usize) -> String {
    let shown = pages.len();
    if shown < total {
        return format!(
            "{path} — pages 1–{shown} of {total}. The rest was not rendered: call dx_read \
             again with `section` set to a block id from dx_outline to read further, which \
             is cheaper and sharper than paging through the whole document."
        );
    }
    format!("{path} — {total} page(s), rendered whole.")
}

/// The Markdown a read falls back to, saying plainly that this is not the page.
fn fallback_text(document: &Document, reason: &str) -> String {
    format!(
        "Could not render page images: {reason}\n\nThis is the document as text, which \
         omits how it is laid out:\n\n{}",
        text(document, &TextOptions::default())
    )
}

/// `dx_play` — drive the rendered page with scripted input and return the frames.
///
/// The same page `dx_read` photographs, alive: inputs are dispatched as real browser
/// events, and every returned image is stamped with when it was taken and which action it
/// shows landing. Nothing the document carries executes — the render is script-free — so
/// this is a read that can also see behaviour: scrolling, hover, folds, a board panned.
fn play_frames(args: &Value, root: &Path) -> ToolResult {
    let document = selected(args, root)?;
    let path = string(args, "path").unwrap_or("document");
    let script = required(args, "script")?;
    let options = PlayOptions {
        width: number(args, "width").unwrap_or(doc_shot::DEFAULT_WIDTH),
        theme: Theme::parse(string(args, "theme").unwrap_or("auto")),
        fps: number(args, "fps").unwrap_or(doc_shot::play::DEFAULT_FPS),
        node: string(args, "node").map(str::to_string),
        ..PlayOptions::default()
    };

    let frames = doc_shot::play::play(&document, script, &options)?;
    let chosen = frame_sample(&frames, PLAY_FRAME_BUDGET);

    let elapsed = frames.last().map_or(0, |frame| frame.at_ms);
    let mut opening = format!("{path} — {} frames over {elapsed}ms.", frames.len());
    if chosen.len() < frames.len() {
        opening.push_str(&format!(
            " Showing {} — every action frame, waits thinned. Use dx play --out for all \
             of them.",
            chosen.len()
        ));
    }
    let mut content = vec![text_content(&opening)];
    for index in chosen {
        let frame = &frames[index];
        content.push(text_content(&format!(
            "frame {}/{} — t={}ms{}",
            index + 1,
            frames.len(),
            frame.at_ms,
            frame
                .note
                .as_deref()
                .map(|note| format!(" — {note}"))
                .unwrap_or_default()
        )));
        content.push(json!({
            "type": "image",
            "data": base64(&frame.png),
            "mimeType": "image/png"
        }));
    }
    Ok(content)
}

/// Which frames to return when a play captured more than the budget.
///
/// Frames that show an action landing are the ones a reviewer cannot do without, so they
/// are kept first (evenly thinned only if they alone exceed the budget); the remaining
/// slots go to wait frames spread evenly across the whole run.
fn frame_sample(frames: &[PlayFrame], budget: usize) -> Vec<usize> {
    if frames.len() <= budget {
        return (0..frames.len()).collect();
    }

    let noted: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.note.is_some())
        .map(|(index, _)| index)
        .collect();
    let mut chosen: Vec<usize> = if noted.len() >= budget {
        every_nth(&noted, budget)
    } else {
        let quiet: Vec<usize> = (0..frames.len())
            .filter(|i| frames[*i].note.is_none())
            .collect();
        let mut kept = noted.clone();
        kept.extend(every_nth(&quiet, budget - noted.len()));
        kept
    };
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}

/// At most `count` items of `indices`, spread evenly, first and last kept.
fn every_nth(indices: &[usize], count: usize) -> Vec<usize> {
    if indices.len() <= count || count == 0 {
        return indices.to_vec();
    }
    (0..count)
        .map(|slot| indices[slot * (indices.len() - 1) / (count - 1).max(1)])
        .collect()
}

/// `dx_source` — the document's exact text, for quoting and editing.
fn source(args: &Value, root: &Path) -> ToolResult {
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
        &workspace::resolver_for(&path),
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
        "next": "Call dx_read on this path to see the results rendered.",
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

/// Load the whole document named by `path`, with its references filled in.
///
/// Every reading tool comes through here, so an agent looking at a document sees what a
/// person sees: `::code src=` listings holding the file's current text, boards showing
/// the blocks their nodes name — including blocks of sibling documents. `dx_source`
/// does not: it hands over the exact stored characters, references included.
fn document_at(args: &Value, root: &Path) -> Result<Document, String> {
    let path = resolve(required(args, "path")?, root);
    let mut document = parse(&workspace::read(&path)?);
    doc_core::resolve::hydrate(&mut document, &workspace::resolver_for(&path));
    Ok(document)
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
    fn source_returns_the_document_as_markdown() {
        let root = project("source");
        let items = call("dx_source", &json!({ "path": "guide.dx" }), &root).expect("source");
        let body = text_of(&items);
        assert!(body.contains("# Guide"));
        assert!(body.contains("Then run it."));
    }

    #[test]
    fn a_section_narrows_the_result() {
        let root = project("section");
        let body = text_of(
            &call(
                "dx_source",
                &json!({ "path": "guide.dx", "section": "setup" }),
                &root,
            )
            .expect("source"),
        );
        assert!(body.contains("Install it first."));
        assert!(!body.contains("Then run it."));
    }

    #[test]
    fn reading_returns_pages_labelled_with_what_is_on_them() {
        let root = project("read");
        let items = call("dx_read", &json!({ "path": "guide.dx" }), &root).expect("read");
        let body = text_of(&items);

        if items.iter().any(|item| item["type"] == "image") {
            // Every image is preceded by the line that says which page it is and which
            // blocks are on it, so a reader can ask for one part next.
            assert!(body.contains("Page 1 of"));
            assert!(body.contains("blocks:"));
            for item in items.iter().filter(|item| item["type"] == "image") {
                assert_eq!(item["mimeType"], "image/png");
                let data = item["data"].as_str().expect("image data");
                assert!(data.starts_with("iVBOR"), "not a PNG payload");
            }
        } else {
            // No browser here: the read degrades to text and says so rather than failing.
            assert!(body.contains("Could not render page images"));
            assert!(body.contains("# Guide"), "fallback must carry the content");
        }
    }

    #[test]
    fn a_read_of_one_section_renders_only_that_section() {
        let root = project("read-section");
        let items = call(
            "dx_read",
            &json!({ "path": "guide.dx", "section": "setup" }),
            &root,
        )
        .expect("read");
        let body = text_of(&items);
        // Whether it rendered or fell back to text, it must never widen to the whole
        // document: a section read that quietly returns everything wastes the reader's
        // context on what they did not ask for.
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
    fn write_stores_a_document_and_reports_its_ids() {
        let root = project("write");
        let items = call(
            "dx_write",
            &json!({ "path": "new.dx", "content": "::paragraph id=hello\nHi there\n::end\n" }),
            &root,
        )
        .expect("write");
        assert!(text_of(&items).contains("hello"));
        // A pointer on disk, the document through the resolver.
        assert!(doc_store::stub::is_stub(
            &std::fs::read_to_string(root.join("new.dx")).expect("read")
        ));
        assert_eq!(
            workspace::read(&root.join("new.dx")).expect("resolve"),
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

        let saved = workspace::read(&root.join("guide.dx")).expect("resolve");
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

    fn wait_frame(at_ms: u64, note: Option<&str>) -> PlayFrame {
        PlayFrame {
            png: Vec::new(),
            at_ms,
            note: note.map(str::to_string),
            width: 1,
            height: 1,
        }
    }

    #[test]
    fn a_short_play_returns_every_frame_in_order() {
        let frames: Vec<PlayFrame> = (0..5).map(|n| wait_frame(n * 100, None)).collect();
        assert_eq!(frame_sample(&frames, 12), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn thinning_keeps_every_action_frame_and_the_run_endpoints() {
        // 30 frames, actions at 0, 7, and 29: the frames a reviewer cannot do without.
        let frames: Vec<PlayFrame> = (0..30)
            .map(|n| wait_frame(n * 100, matches!(n, 0 | 7 | 29).then_some("acted")))
            .collect();
        let chosen = frame_sample(&frames, 12);
        assert!(chosen.len() <= 12, "{chosen:?}");
        for action in [0, 7, 29] {
            assert!(chosen.contains(&action), "{chosen:?} misses {action}");
        }
        assert!(
            chosen.windows(2).all(|pair| pair[0] < pair[1]),
            "{chosen:?}"
        );
    }

    #[test]
    fn a_run_with_more_actions_than_budget_still_spans_first_to_last() {
        let frames: Vec<PlayFrame> = (0..40).map(|n| wait_frame(n, Some("acted"))).collect();
        let chosen = frame_sample(&frames, 12);
        assert_eq!(chosen.len(), 12);
        assert_eq!(chosen.first(), Some(&0));
        assert_eq!(chosen.last(), Some(&39));
    }

    #[test]
    fn play_requires_a_script_and_refuses_a_bad_one_by_name() {
        let root = project("play-args");
        assert!(call("dx_play", &json!({ "path": "guide.dx" }), &root)
            .unwrap_err()
            .contains("`script` is required"));
        let error = call(
            "dx_play",
            &json!({ "path": "guide.dx", "script": "levitate" }),
            &root,
        )
        .expect_err("should refuse");
        assert!(error.contains("levitate"), "{error}");
    }

    #[test]
    fn play_returns_annotated_frames_from_a_real_browser() {
        if doc_shot::browser::find().is_none() {
            return;
        }
        let root = project("play");
        let items = call(
            "dx_play",
            &json!({ "path": "guide.dx", "script": "wait 150ms; key PageDown" }),
            &root,
        )
        .expect("play");
        let body = text_of(&items);
        assert!(body.contains("frames over"), "{body}");
        assert!(body.contains("key PageDown"), "{body}");
        assert!(
            items.iter().any(|item| item["type"] == "image"),
            "a play must return pictures"
        );
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
