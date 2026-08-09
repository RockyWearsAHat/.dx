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
        "dx_index" => index(args, root),
        "dx_render" => render(args, root),
        "dx_write" => write(args, root),
        "dx_edit" => edit(args, root),
        "dx_board" => board(args, root),
        "dx_append" => append(args, root),
        "dx_check" => check(args, root),
        "dx_run" => run(args, root),
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Refresh a document's recorded output before a read, so the page shows live results.
///
/// This is the agent read's HMR: a plain run — no approval, no force — executes exactly
/// the blocks whose code this machine already approved but whose recorded output went
/// stale (editing code, or a file it declares with `reads=`, changes the fingerprint),
/// folds the fresh output in, and skips everything current. Unreviewed code still never
/// runs on a read — it stays blocked, and the returned note says so instead of the page
/// silently showing old output. A refresh that cannot run degrades to a note; it never
/// fails the read.
///
/// Returns the note to show the reader, or `None` when there was nothing to say.
fn refresh_outputs(args: &Value, root: &Path, cache_root: &Path) -> Option<String> {
    let path = resolve(string(args, "path")?, root);
    let source = workspace::read(&path).ok()?;

    let report = match run_document(
        &source,
        &RunOptions {
            document_dir: workspace::document_dir(&path),
            cache_root: cache_root.to_path_buf(),
            ..RunOptions::default()
        },
        &workspace::resolver_for(&path),
    ) {
        Ok(report) => report,
        Err(reason) => return Some(format!("Output not refreshed: {reason}")),
    };

    if report.changed {
        if let Err(reason) = workspace::save_source(&path, &report.source) {
            return Some(format!("Refreshed output could not be saved: {reason}"));
        }
    }

    let refreshed: Vec<&str> = report
        .runs
        .iter()
        .filter(|run| run.status == "ok" || run.status == "error")
        .map(|run| run.id.as_str())
        .collect();
    let blocked = report
        .runs
        .iter()
        .filter(|run| run.status == "blocked")
        .count();

    let mut note = String::new();
    if !refreshed.is_empty() {
        note.push_str(&format!(
            "Refreshed stale output of approved block(s): {}.",
            refreshed.join(", ")
        ));
    }
    if blocked > 0 {
        if !note.is_empty() {
            note.push(' ');
        }
        note.push_str(&format!(
            "{blocked} runnable block(s) changed since approval and await review — their \
             recorded output is stale. Call dx_run with review=true, then approve=true."
        ));
    }
    (!note.is_empty()).then_some(note)
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
    read_in(args, root, &RunOptions::default().cache_root)
}

/// The body of [`read`], with the run cache — the approval ledger included — stated
/// rather than defaulted, so the suite's refreshes never touch the developer's real one.
fn read_in(args: &Value, root: &Path, cache_root: &Path) -> ToolResult {
    let note = if boolean_or(args, "refresh", true) {
        refresh_outputs(args, root, cache_root)
    } else {
        None
    };
    let document = selected(args, root)?;
    let path = string(args, "path").unwrap_or("document");
    // Pages sized for the reader this tool serves: a vision model. `for_reading` keeps
    // every page under the limits past which ingestion would downscale it, so the image
    // the model sees is the image this process delivered, pixel for pixel — and each of
    // those pixels is averaged down from a denser rasterization, so fine ink survives
    // the budget as gray instead of vanishing.
    let options = ShotOptions {
        theme: Theme::parse(string(args, "theme").unwrap_or("auto")),
        ..ShotOptions::for_reading(number(args, "width"))
    };

    let mut content: Vec<Value> = note.as_deref().map(text_content).into_iter().collect();

    if let Some(id) = string(args, "block") {
        content.extend(read_block(&document, id, path, &options)?);
        return Ok(content);
    }

    let pages = match capture_pages(&document, &options) {
        Ok(pages) if !pages.is_empty() => pages,
        // No browser, or nothing captured: the reader still gets the document, as text.
        Ok(_) => {
            content.push(text_content(&fallback_text(&document, "captured no pages")));
            return Ok(content);
        }
        Err(reason) => {
            content.push(text_content(&fallback_text(&document, &reason)));
            return Ok(content);
        }
    };

    let total = pages[0].total;
    content.push(text_content(&opening_line(path, &pages, total)));
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

/// `dx_read` with `block`: one image of one block — a board at its natural size, a node's
/// block exactly as the page (or its board box) carries it.
fn read_block(
    document: &doc_core::model::Document,
    id: &str,
    path: &str,
    options: &ShotOptions,
) -> ToolResult {
    match doc_shot::capture_block(document, id, options) {
        Ok(shot) => Ok(vec![
            text_content(&format!(
                "{path} — block `{id}` ({}x{} px)",
                shot.width, shot.height
            )),
            json!({
                "type": "image",
                "data": base64(&shot.png),
                "mimeType": "image/png"
            }),
        ]),
        Err(reason) => Ok(vec![text_content(&fallback_text(document, &reason))]),
    }
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
///
/// A text read is live too: the same refresh `dx_read` performs runs first, so the
/// captured output this hands over is what the approved code produces *now*.
fn source(args: &Value, root: &Path) -> ToolResult {
    source_in(args, root, &RunOptions::default().cache_root)
}

/// The body of [`source`], with the run cache stated rather than defaulted.
fn source_in(args: &Value, root: &Path, cache_root: &Path) -> ToolResult {
    let note = if boolean_or(args, "refresh", true) {
        refresh_outputs(args, root, cache_root)
    } else {
        None
    };
    let document = selected(args, root)?;
    let mut content: Vec<Value> = note.as_deref().map(text_content).into_iter().collect();
    content.push(text_content(&text(
        &document,
        &TextOptions {
            include_ids: boolean(args, "ids"),
            ..TextOptions::default()
        },
    )));
    Ok(content)
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

/// `dx_search` — documents matching a query, each hit carrying its answer.
///
/// A hit names the block that best matches the query and hands over that block's text (a
/// heading brings its whole section), capped at [`SEARCH_EXCERPT_LIMIT`] — so the search
/// that lands usually needs no follow-up read at all.
fn search(args: &Value, root: &Path) -> ToolResult {
    let query = required(args, "query")?;
    let directory = directory_arg(args, root);
    let limit = number(args, "limit").map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);

    let hits: Vec<Value> = workspace::search(&directory, query, limit)
        .into_iter()
        .map(|hit| {
            let mut item = json!({
                "path": hit.document.relative,
                "title": hit.document.title(),
                "score": hit.score,
            });
            if let Some(id) = &hit.block {
                if let Some(scoped) = section(&hit.document.document, id) {
                    item["block"] = json!(id);
                    item["excerpt"] = json!(excerpt(&text(&scoped, &TextOptions::default())));
                }
            }
            item
        })
        .collect();
    Ok(vec![json_content(&json!({ "matches": hits }))])
}

/// Longest excerpt one search hit carries, in bytes — enough to answer most questions on
/// the spot, while the hit still names its block for a full section read.
const SEARCH_EXCERPT_LIMIT: usize = 700;

/// Cap `answer` at [`SEARCH_EXCERPT_LIMIT`] bytes on a character boundary, marking the cut.
fn excerpt(answer: &str) -> String {
    let trimmed = answer.trim_end();
    if trimmed.len() <= SEARCH_EXCERPT_LIMIT {
        return trimmed.to_string();
    }
    let mut end = SEARCH_EXCERPT_LIMIT;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &trimmed[..end])
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
    let mut content = vec![text_content(&format!(
        "Wrote {} ({} blocks). Block ids: {}",
        path.display(),
        document.blocks.len(),
        block_ids(&document)
    ))];
    for warning in workspace::oversized_image_warnings(&document, &path) {
        content.push(text_content(&warning));
    }
    Ok(content)
}

/// `dx_edit` — replace one block's body.
fn edit(args: &Value, root: &Path) -> ToolResult {
    edit_in(args, root, RunOptions::default().cache_root)
}

/// The body of [`edit`], with the run cache — the approval ledger included — stated
/// rather than defaulted, so the suite never approves anything in the developer's real
/// one.
///
/// Editing a *runnable* block also runs it, exactly as the editing surface does the
/// moment a field closes: the hand that typed the code is the review the approval gate
/// asks for, and the output under new code is stale by definition. `run: false` declines.
///
/// `header` retypes the whole block through [`doc_core::edit::replace_block`] — the same
/// operation a reader performs rewriting the tag line above a block — so changing a
/// block's kind or one attribute (`src=`, `reads=`, `writes=`, `run`) is one call, never
/// a rewrite of the document. `replace`+`with` edit an exact string inside the body
/// through [`doc_core::edit::replace_in_block`], so a one-word change is priced at the
/// change, not the block.
fn edit_in(args: &Value, root: &Path, cache_root: PathBuf) -> ToolResult {
    let path = resolve(required(args, "path")?, root);
    let wanted = required(args, "block")?.trim().trim_start_matches('#');

    // The change-sized edit: `replace`+`with` touch an exact string inside the body and
    // nothing else, so a one-word change is priced at the change, not the block. It
    // stands alone — combined with `text` there would be two instructions for one body.
    let (document, focus, replaced) = if let Some(old) = string(args, "replace") {
        if string(args, "text").is_some() || string(args, "header").is_some() {
            return Err(
                "replace edits the body in place — pass it alone, not with text or header"
                    .to_string(),
            );
        }
        let new = string(args, "with").ok_or_else(|| {
            "replace needs `with` (an empty string deletes the match)".to_string()
        })?;
        let all = boolean_or(args, "all", false);
        let (updated, count) =
            doc_core::edit::replace_in_block(&workspace::read(&path)?, wanted, old, new, all)?;
        (parse(&updated), wanted.to_string(), Some(count))
    } else if let Some(header) = string(args, "header") {
        let body = required(args, "text")?;
        let (updated, focus) =
            doc_core::edit::replace_block(&workspace::read(&path)?, wanted, header, body)?;
        (parse(&updated), focus, None)
    } else {
        // The body goes through the engine's own `set_block`, never a direct field
        // write: a list's or checklist's content lives in its items, which only
        // `set_body` knows how to rebuild — assigning `.text` here was a silent no-op
        // for exactly those kinds, and skipped the hidden-node creation a board edit
        // performs.
        let body = required(args, "text")?;
        let document = parse(&doc_core::edit::set_block(
            &workspace::read(&path)?,
            wanted,
            body,
        )?);
        let index = doc_core::edit::find(&document, wanted)?;
        let focus = document.blocks[index].id.clone();
        (document, focus, None)
    };

    let index = document.blocks.iter().position(|block| block.id == focus);
    let runnable =
        index.is_some_and(|at| document.blocks[at].kind == "code" && document.blocks[at].run);
    workspace::save(&path, &document)?;

    let mut content = vec![text_content(&if let Some(count) = replaced {
        let occurrences = if count == 1 {
            "1 occurrence".to_string()
        } else {
            format!("{count} occurrences")
        };
        format!("Replaced {occurrences} in `{focus}` in {}.", path.display())
    } else if focus.eq_ignore_ascii_case(wanted) {
        format!("Updated `{focus}` in {}.", path.display())
    } else {
        format!(
            "Updated `{wanted}` in {} — the block is now `{focus}`.",
            path.display()
        )
    })];
    for warning in workspace::oversized_image_warnings(&document, &path) {
        content.push(text_content(&warning));
    }

    if runnable && boolean_or(args, "run", true) {
        let run_args = json!({
            "path": required(args, "path")?,
            "block": focus,
            "approve": true,
        });
        match run_in(&run_args, root, cache_root) {
            Ok(mut ran) => content.append(&mut ran),
            // The edit itself succeeded; a run that could not is reported, not fatal.
            Err(reason) => content.push(text_content(&format!(
                "Edited, but running the block failed: {reason}"
            ))),
        }
    } else if let Some(at) = index.filter(|_| runnable) {
        // `run: false` declines the immediate run, not the review the edit already was:
        // the block is approved as typed, so the next run or live read executes it.
        if let Err(sentence) = doc_run::approve_edited_block(&document.blocks[at], &cache_root) {
            content.push(text_content(&format!(
                "Edited, but the approval could not be recorded: {sentence}"
            )));
        }
    }
    Ok(content)
}

/// `dx_board` — place, arrange, detach, or link/unlink a node on a `::board`.
///
/// The one MCP path to [`doc_core::edit`]'s board operations — the same ones `dx board`
/// and every editing surface's drag/arrange/link actions call — so a node an agent places
/// gets the identical collision-safe board a person dragging one gets: anything it lands
/// on is [settled](doc_core::edit) out of its way, never left stacked. `dx_edit` writes a
/// board's raw body text verbatim and stops at `create_missing_nodes` — right for a caller
/// stating exact coordinates by hand, wrong for "put this node here": this is that tool.
fn board(args: &Value, root: &Path) -> ToolResult {
    let path = resolve(required(args, "path")?, root);
    let board_id = required(args, "board")?;
    let action = required(args, "action")?;
    let source = workspace::read(&path)?;

    let (updated, message) = match action {
        "place" => {
            let node = string(args, "node").unwrap_or_default();
            let w = doc_core::edit::SizeSpec::parse(string(args, "w").unwrap_or_default())
                .map_err(|why| format!("`w`: {why}"))?;
            let h = doc_core::edit::SizeSpec::parse(string(args, "h").unwrap_or_default())
                .map_err(|why| format!("`h`: {why}"))?;
            let (updated, id) = doc_core::edit::board_place(
                &source,
                board_id,
                node,
                integer(args, "x"),
                integer(args, "y"),
                w,
                h,
            )?;
            (
                updated,
                format!("Placed `{id}` on `{board_id}` in {}.", path.display()),
            )
        }
        "arrange" => {
            let spec = required(args, "placements")?;
            let placements = doc_core::edit::placements(spec)?;
            let count = placements.len();
            let updated = doc_core::edit::board_arrange(&source, board_id, &placements)?;
            (
                updated,
                format!(
                    "Placed {count} node{} on `{board_id}` in {}.",
                    if count == 1 { "" } else { "s" },
                    path.display()
                ),
            )
        }
        "detach" => {
            let node = required(args, "node")?;
            let updated = doc_core::edit::board_detach(&source, board_id, node)?;
            (
                updated,
                format!("Detached `{node}` from `{board_id}` in {}.", path.display()),
            )
        }
        "link" | "unlink" => {
            let from = required(args, "node")?;
            let to = required(args, "to")?;
            let updated = doc_core::edit::board_link(
                &source,
                board_id,
                from,
                to,
                action == "link",
                string(args, "from_side").unwrap_or_default(),
                string(args, "to_side").unwrap_or_default(),
            )?;
            (
                updated,
                format!(
                    "{} `{from}` -> `{to}` on `{board_id}` in {}.",
                    if action == "link" {
                        "Linked"
                    } else {
                        "Unlinked"
                    },
                    path.display()
                ),
            )
        }
        other => {
            return Err(format!(
                "`action` must be one of place, arrange, detach, link, unlink — not `{other}`"
            ))
        }
    };

    workspace::save(&path, &parse(&updated))?;
    Ok(vec![text_content(&message)])
}

/// `dx_append` — the cheap write: a small thought lands for the cost of its own lines.
///
/// This is what makes think-by-writing the path of least resistance instead of a
/// discipline: a finding, a hypothesis, a worklist line goes into the document the
/// moment it forms, without resending anything that is already there. `block` grows
/// that block's body by `text` — it lands on the same replace [`edit`] performs, so a
/// runnable code block grown this way runs, exactly as any edit of it would (`run:
/// false` declines). `after` inserts a new block after that id; with neither, the new
/// block lands at the end of the document. New blocks are prose kinds only — code
/// enters through `dx_edit` or `dx_write`, where its powers are stated and reviewed,
/// so the quick path can never quietly grow the executable surface.
fn append(args: &Value, root: &Path) -> ToolResult {
    if string(args, "block").is_some() && string(args, "after").is_some() {
        return Err(
            "pass `block` (grow that block's body) or `after` (insert a new block after it), \
             not both"
                .into(),
        );
    }
    let text = required(args, "text")?;
    let path = resolve(required(args, "path")?, root);

    if let Some(wanted) = string(args, "block") {
        let wanted = wanted.trim().trim_start_matches('#');
        let document = parse(&workspace::read(&path)?);
        let index = doc_core::edit::find(&document, wanted)?;
        let existing = doc_core::edit::body(&document.blocks[index]);
        let grown = if existing.is_empty() {
            text.to_string()
        } else {
            format!("{existing}\n{text}")
        };
        return edit(
            &json!({
                "path": required(args, "path")?,
                "block": document.blocks[index].id.clone(),
                "text": grown,
                "run": boolean_or(args, "run", true),
            }),
            root,
        );
    }

    let kind = string(args, "type").unwrap_or("paragraph");
    if kind == "code" {
        return Err(
            "dx_append writes prose; a code block enters through dx_edit or dx_write, where \
             its powers (run, reads=, writes=) are stated and reviewed"
                .into(),
        );
    }
    let source = workspace::read(&path)?;
    let after = match string(args, "after") {
        Some(id) => Some(id.trim().trim_start_matches('#').to_string()),
        None => parse(&source).blocks.last().map(|block| block.id.clone()),
    };
    let insertion = doc_core::edit::Insertion {
        kind,
        body: text,
        id: string(args, "id").unwrap_or_default(),
        level: number(args, "level").map_or(0, |level| level as u8),
        language: "",
        run: false,
        deps: "",
    };
    let (updated, id) = doc_core::edit::insert_after(&source, after.as_deref(), &insertion)?;
    workspace::save(&path, &parse(&updated))?;
    Ok(vec![text_content(&format!(
        "Added `{id}` to {}.",
        path.display()
    ))])
}

/// `dx_check` — tick or untick one checklist box: the worklist move.
///
/// The same [`doc_core::edit::toggle_check`] a reader's click and `dx check` perform,
/// so every surface spells the checked state the same way. Every other item and every
/// other block comes back byte-identical.
fn check(args: &Value, root: &Path) -> ToolResult {
    let path = resolve(required(args, "path")?, root);
    let wanted = required(args, "block")?.trim().trim_start_matches('#');
    let item = args
        .get("item")
        .and_then(Value::as_u64)
        .ok_or("`item` is required: the box's position in the checklist, counting from 0")?
        as usize;
    let (updated, now_checked) =
        doc_core::edit::toggle_check(&workspace::read(&path)?, wanted, item)?;
    workspace::save(&path, &parse(&updated))?;
    Ok(vec![text_content(&format!(
        "Item {item} of `{wanted}` is now {}.",
        if now_checked { "checked" } else { "unchecked" }
    ))])
}

/// `dx_index` — scaffold `index.dx`, the precursor project map, from the file tree.
fn index(args: &Value, root: &Path) -> ToolResult {
    let directory = directory_arg(args, root);
    let scaffold = crate::commands::index::write_scaffold(&directory, boolean(args, "force"))?;
    let harness = match &scaffold.harness {
        Some((path, gates)) => format!(
            " Wrote {} — {gates} verify gate(s) for the detected build system: review \
             them, then run dev.dx with approve=true to record the first green.",
            path.display()
        ),
        None => String::new(),
    };
    Ok(vec![text_content(&format!(
        "Wrote {} — {} area(s), {} file(s), ranked from the tree alone.{harness} Read \
         the whole index now and improve it before other work: replace each TODO with \
         what the area does, and add ::code src= blocks for the load-bearing files so \
         the index shows their current text forever.",
        scaffold.path.display(),
        scaffold.areas,
        scaffold.files
    ))])
}

/// `dx_run` — execute the document's runnable blocks.
///
/// `review` is a read: it executes nothing and writes nothing, here or in the engine.
fn run(args: &Value, root: &Path) -> ToolResult {
    run_in(args, root, RunOptions::default().cache_root)
}

/// The body of [`run`], with the run cache — the approval ledger included — stated rather
/// than defaulted, so the suite never records an approval in the developer's real cache.
fn run_in(args: &Value, root: &Path, cache_root: PathBuf) -> ToolResult {
    let review_only = boolean(args, "review");
    let path = resolve(required(args, "path")?, root);
    let source = workspace::read(&path)?;

    let report = run_document(
        &source,
        &RunOptions {
            document_dir: workspace::document_dir(&path),
            cache_root,
            force: boolean(args, "force"),
            only: string(args, "block").map(str::to_string),
            review_only,
            approve: boolean(args, "approve"),
            follow_board_edges: boolean(args, "follow_edges"),
            ..RunOptions::default()
        },
        &workspace::resolver_for(&path),
    )?;

    // Review reads a document; it never writes one. `run_document` folds nothing in review
    // mode, and this is the second lock on the same door: the tool that promises to execute
    // nothing must not be able to touch the file either.
    let saved = report.changed && !review_only;
    if saved {
        workspace::save_source(&path, &report.source)?;
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
        "saved": saved,
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
/// Every reading tool comes through here — `dx_source` included — so an agent looking
/// at a document sees what a person sees: `::code src=` listings holding the file's
/// current text, boards showing the blocks their nodes name — including blocks of
/// sibling documents. The text view is what keeps that honest for an `::image src=`:
/// hydration fills it with the file's bytes, and `render::text` states what they are
/// instead of printing megabytes of base64 at a reader who asked for words.
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

/// An optional signed integer argument — a board coordinate may be any whole number.
fn integer(args: &Value, key: &str) -> Option<i32> {
    args.get(key)
        .and_then(Value::as_i64)
        .map(|value| value as i32)
}

/// An optional boolean argument, defaulting to false.
fn boolean(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// An optional boolean argument with a stated default, for flags that are on by default.
fn boolean_or(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
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

    /// The gap this tool closes: an MCP-driven placement must get the same collision
    /// guarantee a drag or `dx board --place` gives, not the raw verbatim write `dx_edit`
    /// performs on a board's body text.
    #[test]
    fn placing_a_node_through_dx_board_settles_it_off_anything_it_covers() {
        let root = project("board-place-settles");
        workspace::write_text(
            &root.join("plan.dx"),
            "::board id=plan height=520\n- ideas x=40 y=40 w=280 h=160\n::end\n\n\
::paragraph id=ideas hidden\nRough ideas.\n::end\n",
        )
        .expect("seed");

        // Land squarely on top of the existing node.
        call(
            "dx_board",
            &json!({
                "path": "plan.dx", "board": "plan", "action": "place", "node": "intruder",
                "x": 40, "y": 40, "w": "280", "h": "160",
            }),
            &root,
        )
        .expect("place");

        let source = workspace::read(&root.join("plan.dx")).expect("read");
        let node_box = |id: &str| -> (i32, i32, i32, i32) {
            let line = source
                .lines()
                .find(|line| line.trim_start().starts_with(&format!("- {id} ")))
                .unwrap_or_else(|| panic!("no line for `{id}` in:\n{source}"));
            let field = |name: &str| -> i32 {
                line.split_whitespace()
                    .find_map(|token| token.strip_prefix(&format!("{name}=")))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| panic!("no {name}= on `{line}`"))
            };
            (field("x"), field("y"), field("w"), field("h"))
        };
        let (ix, iy, iw, ih) = node_box("intruder");
        let (ox, oy, ow, oh) = node_box("ideas");
        let overlaps = ix < ox + ow && ox < ix + iw && iy < oy + oh && oy < iy + ih;
        assert!(
            !overlaps,
            "intruder ({ix},{iy},{iw}x{ih}) still overlaps ideas ({ox},{oy},{ow}x{oh}):\n{source}"
        );

        // Contrast: `dx_edit` on the same board writes the body verbatim, uncollided —
        // that is the hand-authored-layout path `dx_board` is deliberately not.
        call(
            "dx_edit",
            &json!({
                "path": "plan.dx",
                "block": "plan",
                "text": "- ideas x=40 y=40 w=280 h=160\n- stacked x=40 y=40 w=280 h=160",
            }),
            &root,
        )
        .expect("raw board edit");
        let source = workspace::read(&root.join("plan.dx")).expect("read");
        assert!(
            source.contains("- ideas x=40 y=40 w=280 h=160")
                && source.contains("- stacked x=40 y=40 w=280 h=160"),
            "dx_edit must still write a board body verbatim: {source}"
        );
    }

    #[test]
    fn arranging_several_nodes_through_dx_board_settles_them_together() {
        let root = project("board-arrange");
        workspace::write_text(&root.join("plan.dx"), "::board id=plan height=520\n::end\n")
            .expect("seed");
        let result = call(
            "dx_board",
            &json!({
                "path": "plan.dx", "board": "plan", "action": "arrange",
                "placements": "first,40,40,200,120 second,40,40,200,120",
            }),
            &root,
        )
        .expect("arrange");
        assert!(text_of(&result).contains("Placed 2 nodes"), "{result:?}");
        let source = workspace::read(&root.join("plan.dx")).expect("read");
        assert!(source.contains("- first"), "{source}");
        assert!(source.contains("- second"), "{source}");
        // Both new hidden blocks were created for the nodes named on a board with none yet.
        assert!(
            source.contains("id=first") && source.contains("id=second"),
            "{source}"
        );
    }

    #[test]
    fn detaching_and_linking_through_dx_board_reach_the_same_edit_core_operations() {
        let root = project("board-detach-link");
        workspace::write_text(
            &root.join("plan.dx"),
            "::board id=plan height=520\n- a x=40 y=40 w=200 h=120\n\
- b x=300 y=40 w=200 h=120\n::end\n\n\
::paragraph id=a hidden\nA.\n::end\n\n::paragraph id=b hidden\nB.\n::end\n",
        )
        .expect("seed");

        call(
            "dx_board",
            &json!({ "path": "plan.dx", "board": "plan", "action": "link", "node": "a", "to": "b" }),
            &root,
        )
        .expect("link");
        let linked = workspace::read(&root.join("plan.dx")).expect("read");
        assert!(linked.contains("to=b"), "{linked}");

        call(
            "dx_board",
            &json!({ "path": "plan.dx", "board": "plan", "action": "unlink", "node": "a", "to": "b" }),
            &root,
        )
        .expect("unlink");
        let unlinked = workspace::read(&root.join("plan.dx")).expect("read");
        assert!(!unlinked.contains("to=b"), "{unlinked}");

        call(
            "dx_board",
            &json!({ "path": "plan.dx", "board": "plan", "action": "detach", "node": "b" }),
            &root,
        )
        .expect("detach");
        let detached = workspace::read(&root.join("plan.dx")).expect("read");
        assert!(!detached.contains("- b "), "{detached}");
        assert!(
            !detached.contains("id=b"),
            "the orphaned hidden block leaves too: {detached}"
        );
    }

    #[test]
    fn an_unknown_board_action_is_refused() {
        let root = project("board-bad-action");
        workspace::write_text(&root.join("plan.dx"), "::board id=plan\n::end\n").expect("seed");
        let err = call(
            "dx_board",
            &json!({ "path": "plan.dx", "board": "plan", "action": "teleport" }),
            &root,
        )
        .expect_err("unknown action");
        assert!(err.contains("teleport"), "{err}");
    }

    /// The change-sized edit reaches MCP: `replace`+`with` change the match and keep
    /// every other character, and the answer counts what changed.
    #[test]
    fn editing_with_replace_changes_the_match_and_keeps_the_rest() {
        let root = project("edit-replace");
        let answer = call(
            "dx_edit",
            &json!({ "path": "guide.dx", "block": "setup-body", "replace": "Install", "with": "Download" }),
            &root,
        )
        .expect("edit");
        assert!(
            text_of(&answer).contains("Replaced 1 occurrence"),
            "{answer:?}"
        );
        let text =
            text_of(&call("dx_source", &json!({ "path": "guide.dx" }), &root).expect("read"));
        assert!(text.contains("Download it first."), "{text}");
        assert!(text.contains("Then run it."), "{text}");
    }

    /// Without `all`, a multi-match replace is refused with the count — landing on
    /// whichever match came first is how a rename hits the wrong line.
    #[test]
    fn an_ambiguous_mcp_replace_is_refused_with_the_count() {
        let root = project("edit-replace-ambiguous");
        workspace::write_text(
            &root.join("ports.dx"),
            "::paragraph id=note\nport 80 now, port 80 later\n::end\n",
        )
        .expect("seed");
        let refusal = call(
            "dx_edit",
            &json!({ "path": "ports.dx", "block": "note", "replace": "port 80", "with": "port 81" }),
            &root,
        )
        .expect_err("refuse");
        assert!(refusal.contains("2 times"), "{refusal}");
    }

    #[test]
    fn appending_to_a_block_costs_only_the_new_lines() {
        let root = project("append-grow");
        workspace::write_text(
            &root.join("lab.dx"),
            "::checklist id=worklist\n[x] shipped\n[ ] open\n::end\n",
        )
        .expect("seed");
        call(
            "dx_append",
            &json!({ "path": "lab.dx", "block": "worklist", "text": "[ ] a fresh thought" }),
            &root,
        )
        .expect("append");
        let text = text_of(&call("dx_source", &json!({ "path": "lab.dx" }), &root).expect("read"));
        // Everything already there survives, marks included, and the new line is last.
        assert!(text.contains("shipped"), "{text}");
        assert!(text.contains("a fresh thought"), "{text}");
    }

    #[test]
    fn appending_inserts_a_new_block_after_an_id_or_at_the_end() {
        let root = project("append-insert");
        let after = call(
            "dx_append",
            &json!({ "path": "guide.dx", "after": "setup-body", "text": "A note in place." }),
            &root,
        )
        .expect("insert");
        assert!(text_of(&after).contains("Added"), "{after:?}");
        call(
            "dx_append",
            &json!({ "path": "guide.dx", "text": "A trailing thought." }),
            &root,
        )
        .expect("append at end");
        let source = workspace::read(&root.join("guide.dx")).expect("read");
        let note = source.find("A note in place.").expect("inserted");
        assert!(
            note < source.find("Usage").expect("usage"),
            "in place, not at the end"
        );
        assert!(
            source.trim_end().ends_with("::end"),
            "canonical to the last byte"
        );
        assert!(source.contains("A trailing thought."));

        let both = call(
            "dx_append",
            &json!({ "path": "guide.dx", "block": "setup-body", "after": "usage", "text": "x" }),
            &root,
        );
        assert!(both.is_err(), "block and after together is a refusal");
        let code = call(
            "dx_append",
            &json!({ "path": "guide.dx", "type": "code", "text": "x" }),
            &root,
        );
        assert!(code.expect_err("code is refused").contains("dx_edit"));
    }

    #[test]
    fn a_checkbox_ticks_by_position_and_everything_else_is_untouched() {
        let root = project("check");
        workspace::write_text(
            &root.join("lab.dx"),
            "::checklist id=worklist\n[ ] first\n[ ] second\n::end\n",
        )
        .expect("seed");
        let ticked = call(
            "dx_check",
            &json!({ "path": "lab.dx", "block": "worklist", "item": 1 }),
            &root,
        )
        .expect("tick");
        assert!(text_of(&ticked).contains("now checked"), "{ticked:?}");
        let source = workspace::read(&root.join("lab.dx")).expect("read");
        assert!(source.contains("[ ] first"), "{source}");
        assert!(source.contains("[x] second"), "{source}");
    }

    /// The regression this wave fixed: `dx_edit` wrote `.text` directly, which for a
    /// checklist or list is a silent no-op — their content lives in items only
    /// `set_body` rebuilds. The body now goes through the engine's `set_block`.
    #[test]
    fn editing_a_checklist_body_through_dx_edit_actually_changes_it() {
        let root = project("edit-checklist");
        workspace::write_text(
            &root.join("lab.dx"),
            "::checklist id=worklist\n[x] old line\n::end\n",
        )
        .expect("seed");
        call(
            "dx_edit",
            &json!({ "path": "lab.dx", "block": "worklist", "text": "[ ] rewritten" }),
            &root,
        )
        .expect("edit");
        let source = workspace::read(&root.join("lab.dx")).expect("read");
        assert!(source.contains("rewritten"), "{source}");
        assert!(!source.contains("old line"), "{source}");
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
    fn saving_an_oversized_image_warns_at_write_time() {
        let root = project("big-image");
        let too_big = vec![0u8; doc_core::resolve::MAX_IMAGE_BYTES + 1];
        std::fs::write(root.join("big.png"), too_big).expect("seed image");
        let items = call(
            "dx_write",
            &json!({ "path": "doc.dx", "content": "::image id=shot src=big.png\n::end\n" }),
            &root,
        )
        .expect("write");
        let reply = text_of(&items);
        assert!(reply.contains("embed"), "{reply}");
        assert!(reply.contains("big.png"), "{reply}");
    }

    #[test]
    fn edit_with_header_retypes_the_block_in_one_call() {
        let root = project("edit-header");
        let items = call(
            "dx_edit",
            &json!({
                "path": "guide.dx",
                "block": "setup-body",
                "header": "::quote",
                "text": "Install it first.",
            }),
            &root,
        )
        .expect("edit");
        assert!(text_of(&items).contains("setup-body"));

        let saved = workspace::read(&root.join("guide.dx")).expect("resolve");
        assert!(saved.contains("::quote id=setup-body\nInstall it first.\n::end"));
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
        // The hit carries its answer: the matching block's id and text, no second read.
        let parsed: Value = serde_json::from_str(&found).expect("json");
        assert_eq!(parsed["matches"][0]["block"], "setup-body");
        assert!(parsed["matches"][0]["excerpt"]
            .as_str()
            .expect("excerpt")
            .contains("Install it first."));
    }

    #[test]
    fn a_long_search_answer_is_capped_on_a_character_boundary() {
        let long = "é".repeat(SEARCH_EXCERPT_LIMIT); // 2 bytes per char, well past the cap
        let capped = excerpt(&long);
        assert!(capped.len() <= SEARCH_EXCERPT_LIMIT + '…'.len_utf8());
        assert!(capped.ends_with('…'));
        assert!(excerpt("short answer") == "short answer");
    }

    #[test]
    fn run_executes_a_block_and_stores_the_result() {
        let root = project("run");
        workspace::write_text(
            &root.join("runnable.dx"),
            "::code id=hi lang=bash run\necho from-mcp\n::end\n",
        )
        .expect("seed");

        let body = text_of(
            &run_in(
                &json!({ "path": "runnable.dx", "approve": true }),
                &root,
                root.join("cache"),
            )
            .expect("run"),
        );
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["allSucceeded"], true);
        assert_eq!(parsed["results"][0]["output"], "from-mcp");
        assert!(std::fs::read_to_string(root.join("runnable.dx"))
            .expect("read")
            .contains("from-mcp"));
    }

    #[test]
    fn run_refuses_unapproved_code_pending_review() {
        let root = project("run-gate");
        // The ledger is this project's own scratch cache, wiped by `project`, so nothing is
        // approved by construction rather than by hoping no sibling test approved this line.
        workspace::write_text(
            &root.join("gated.dx"),
            "::code id=hi lang=bash run\necho mcp-review-gate-refusal\n::end\n",
        )
        .expect("seed");

        let body = text_of(
            &run_in(&json!({ "path": "gated.dx" }), &root, root.join("cache")).expect("run"),
        );
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["allSucceeded"], false);
        assert_eq!(parsed["results"][0]["status"], "blocked");
        let sentence = parsed["results"][0]["output"].as_str().expect("output");
        assert!(sentence.contains("blocked pending review"), "{sentence}");
        // Nothing executed and nothing was written into the document.
        assert!(!std::fs::read_to_string(root.join("gated.dx"))
            .expect("read")
            .contains("::output"));
    }

    #[test]
    fn run_review_reports_the_code_without_executing() {
        let root = project("run-review");
        let source = "::code id=hi lang=bash run\necho mcp-review-inspection\n::end\n";
        workspace::write_text(&root.join("inspect.dx"), source).expect("seed");

        let body = text_of(
            &run_in(
                &json!({ "path": "inspect.dx", "review": true }),
                &root,
                root.join("cache"),
            )
            .expect("review"),
        );
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["executed"], 0);
        assert_eq!(parsed["results"][0]["status"], "review");
        let text = parsed["results"][0]["output"].as_str().expect("output");
        assert!(text.contains("fingerprint "), "{text}");
        assert!(text.contains("echo mcp-review-inspection"), "{text}");
        // Reading never executes, and never writes.
        assert_eq!(
            std::fs::read_to_string(root.join("inspect.dx")).expect("read"),
            source
        );
    }

    #[test]
    fn run_refuses_review_with_approve_rather_than_swallowing_one() {
        let root = project("run-review-approve");
        workspace::write_text(
            &root.join("both.dx"),
            "::code id=hi lang=bash run\necho mcp-review-approve-conflict\n::end\n",
        )
        .expect("seed");

        let sentence = run_in(
            &json!({ "path": "both.dx", "review": true, "approve": true }),
            &root,
            root.join("cache"),
        )
        .expect_err("conflicting parameters");
        assert!(sentence.contains("approve separately"), "{sentence}");

        // The refusal approved nothing: a plain run still hits the gate.
        let body = text_of(
            &run_in(&json!({ "path": "both.dx" }), &root, root.join("cache")).expect("run"),
        );
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["results"][0]["status"], "blocked");
    }

    #[test]
    fn run_review_writes_nothing_even_when_it_must_refuse_a_block() {
        let root = project("run-review-refusal");
        // A block whose declared `reads=` file is not there: refused before the gate is
        // ever consulted, and once folded an ::output into the file on a review.
        let source = "::code id=hi lang=bash run reads=missing.txt\necho no\n::end\n";
        workspace::write_text(&root.join("needy.dx"), source).expect("seed");

        let body = text_of(
            &run_in(
                &json!({ "path": "needy.dx", "review": true }),
                &root,
                root.join("cache"),
            )
            .expect("review"),
        );
        let parsed: Value = serde_json::from_str(&body).expect("json");
        assert_eq!(parsed["executed"], 0);
        assert_eq!(parsed["saved"], false);
        assert_eq!(
            std::fs::read_to_string(root.join("needy.dx")).expect("read"),
            source
        );
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

    /// The read-side HMR: approved code whose recorded output is stale re-runs before
    /// the page is rendered, so a read always shows live output without a dx_run first.
    #[test]
    fn a_read_refreshes_approved_stale_output_before_rendering() {
        let root = project("read-refresh");
        let cache = root.join("cache");
        // Approve this exact code once, through a run of a sibling document.
        workspace::write_text(
            &root.join("approved.dx"),
            "::code id=hi lang=bash run\necho hmr-live\n::end\n",
        )
        .expect("seed");
        run_in(
            &json!({ "path": "approved.dx", "approve": true }),
            &root,
            cache.clone(),
        )
        .expect("approve");

        // The same approved code, no output recorded yet: stale by definition.
        workspace::write_text(
            &root.join("stale.dx"),
            "::code id=hi lang=bash run\necho hmr-live\n::end\n",
        )
        .expect("seed");

        let items = read_in(&json!({ "path": "stale.dx" }), &root, &cache).expect("read");
        assert!(text_of(&items).contains("Refreshed stale output"));
        let saved = workspace::read(&root.join("stale.dx")).expect("resolve");
        assert!(saved.contains("::output"), "{saved}");
        assert!(saved.contains("hmr-live"), "{saved}");
    }

    /// The refresh saves through the store when the document lives there. This is the
    /// Self-Host field failure verbatim: a read of a stored document re-ran its approved
    /// stale block and saved the result as plain text over the pointer — the store went
    /// stale behind a working tree git saw as a giant diff.
    #[test]
    fn a_refresh_of_a_stored_document_keeps_its_pointer_and_freshens_the_store() {
        let root = project("read-refresh-pointer");
        let cache = root.join("cache");
        let source = "::code id=hi lang=bash run\necho pointer-live\n::end\n";
        workspace::write_text(&root.join("approved.dx"), source).expect("seed");
        run_in(
            &json!({ "path": "approved.dx", "approve": true }),
            &root,
            cache.clone(),
        )
        .expect("approve");

        let path = root.join("stored.dx");
        workspace::save(&path, &doc_core::format::parse(source)).expect("adopt");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .starts_with("~ dx1 "),
            "the fixture must start as a pointer"
        );

        let items = read_in(&json!({ "path": "stored.dx" }), &root, &cache).expect("read");
        assert!(text_of(&items).contains("Refreshed stale output"));
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert!(
            on_disk.starts_with("~ dx1 "),
            "the refresh demoted the pointer to plain text: {on_disk}"
        );
        let resolved = workspace::read(&path).expect("resolve");
        assert!(resolved.contains("pointer-live"), "{resolved}");
        assert!(resolved.contains("::output"), "{resolved}");
    }

    /// The refresh is a plain run, so the approval gate holds: unreviewed code renders
    /// its stale page and the read says so, instead of executing on a look.
    #[test]
    fn a_read_never_runs_unreviewed_code_and_says_so() {
        let root = project("read-gate");
        let source = "::code id=hi lang=bash run\necho mcp-read-sneaky\n::end\n";
        workspace::write_text(&root.join("handed.dx"), source).expect("seed");

        let items =
            read_in(&json!({ "path": "handed.dx" }), &root, &root.join("cache")).expect("read");
        assert!(text_of(&items).contains("await review"));
        assert_eq!(
            workspace::read(&root.join("handed.dx")).expect("resolve"),
            source,
            "a read must not write anything for unreviewed code"
        );
    }

    #[test]
    fn a_read_with_refresh_declined_renders_exactly_what_is_stored() {
        let root = project("read-no-refresh");
        let cache = root.join("cache");
        workspace::write_text(
            &root.join("quiet.dx"),
            "::code id=hi lang=bash run\necho untouched\n::end\n",
        )
        .expect("seed");
        run_in(
            &json!({ "path": "quiet.dx", "approve": true }),
            &root,
            cache.clone(),
        )
        .expect("approve");
        workspace::write_text(
            &root.join("copy.dx"),
            "::code id=hi lang=bash run\necho untouched\n::end\n",
        )
        .expect("seed");

        read_in(
            &json!({ "path": "copy.dx", "refresh": false }),
            &root,
            &cache,
        )
        .expect("read");
        assert!(
            !workspace::read(&root.join("copy.dx"))
                .expect("resolve")
                .contains("::output"),
            "refresh=false must leave the stored document alone"
        );
    }

    /// A text read is live by the same rule as a page read.
    #[test]
    fn source_refreshes_before_handing_over_the_text() {
        let root = project("source-refresh");
        let cache = root.join("cache");
        workspace::write_text(
            &root.join("first.dx"),
            "::code id=hi lang=bash run\necho text-live\n::end\n",
        )
        .expect("seed");
        run_in(
            &json!({ "path": "first.dx", "approve": true }),
            &root,
            cache.clone(),
        )
        .expect("approve");
        workspace::write_text(
            &root.join("second.dx"),
            "::code id=hi lang=bash run\necho text-live\n::end\n",
        )
        .expect("seed");

        let items = source_in(&json!({ "path": "second.dx" }), &root, &cache).expect("source");
        let body = text_of(&items);
        assert!(body.contains("Refreshed stale output"), "{body}");
        assert!(body.contains("text-live"), "{body}");
    }

    /// Editing runnable code runs it at once — the edit is the review, exactly as the
    /// editing surface runs a field the moment it closes — so the output is never stale.
    #[test]
    fn editing_runnable_code_runs_it_immediately() {
        let root = project("edit-run");
        workspace::write_text(
            &root.join("live.dx"),
            "::code id=hi lang=bash run\necho before\n::end\n",
        )
        .expect("seed");

        let items = edit_in(
            &json!({ "path": "live.dx", "block": "hi", "text": "echo edited-live" }),
            &root,
            root.join("cache"),
        )
        .expect("edit");
        let body = text_of(&items);
        assert!(body.contains("Updated `hi`"), "{body}");
        assert!(body.contains("\"allSucceeded\": true"), "{body}");

        let saved = workspace::read(&root.join("live.dx")).expect("resolve");
        assert!(saved.contains("edited-live"), "{saved}");
        assert!(saved.contains("::output"), "{saved}");
    }

    #[test]
    fn editing_runnable_code_with_run_declined_only_edits() {
        let root = project("edit-no-run");
        workspace::write_text(
            &root.join("later.dx"),
            "::code id=hi lang=bash run\necho before\n::end\n",
        )
        .expect("seed");

        edit_in(
            &json!({ "path": "later.dx", "block": "hi", "text": "echo not-yet", "run": false }),
            &root,
            root.join("cache"),
        )
        .expect("edit");
        let saved = workspace::read(&root.join("later.dx")).expect("resolve");
        assert!(saved.contains("echo not-yet"), "{saved}");
        assert!(!saved.contains("::output"), "{saved}");
    }

    #[test]
    fn index_scaffolds_a_map_and_keeps_an_existing_one() {
        let root = project("index");
        std::fs::create_dir_all(root.join("src")).expect("dirs");
        std::fs::write(root.join("src/main.rs"), "fn main() {}").expect("file");

        let items = call("dx_index", &json!({}), &root).expect("index");
        assert!(text_of(&items).contains("Read the whole index now"));
        let text = workspace::read(&root.join("index.dx")).expect("resolve");
        assert!(text.contains("TODO"), "{text}");
        assert!(text.contains("src/"), "{text}");

        let refused = call("dx_index", &json!({}), &root).expect_err("should refuse");
        assert!(refused.contains("force"), "{refused}");
        call("dx_index", &json!({ "force": true }), &root).expect("forced");
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
