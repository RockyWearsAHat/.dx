//! The MCP tool catalogue.
//!
//! Tool descriptions are the only documentation an agent ever reads, so they are written
//! for that reader: each one says what it returns, when to reach for it, and which tool to
//! use instead when this is the wrong one.
//!
//! The ordering is deliberate, and it is the reading economy: *find* (`dx_list`,
//! `dx_search`), *map* (`dx_outline`), *read the text* (`dx_source` — prose and code cost
//! a fraction as text of what a page image costs), and only then *look* (`dx_read`), for
//! the pages that carry what text cannot: boards, diagrams, charts, rendered views. Both
//! reads are live: recorded output of already-approved code is refreshed before it is
//! handed over, so what the reader sees is what the code does now.

use serde_json::{json, Value};

/// Every tool name the server advertises, in catalogue order. Used by the tests that
/// keep [`catalogue`] and this list in step.
#[cfg(test)]
pub const TOOL_NAMES: &[&str] = &[
    "dx_list",
    "dx_search",
    "dx_coverage",
    "dx_outline",
    "dx_source",
    "dx_read",
    "dx_play",
    "dx_render",
    "dx_index",
    "dx_write",
    "dx_edit",
    "dx_board",
    "dx_append",
    "dx_check",
    "dx_report",
    "dx_run",
];

/// Build the `tools/list` payload.
#[must_use]
pub fn catalogue() -> Value {
    json!([
        list_tool(),
        search_tool(),
        coverage_tool(),
        outline_tool(),
        source_tool(),
        read_tool(),
        play_tool(),
        render_tool(),
        index_tool(),
        write_tool(),
        edit_tool(),
        board_tool(),
        append_tool(),
        check_tool(),
        report_tool(),
        run_tool(),
    ])
}

/// The `path` property shared by every document tool.
fn path_property() -> Value {
    json!({
        "type": "string",
        "description": "Path to the .dx file, absolute or relative to the workspace root."
    })
}

/// The `section` property shared by the reading tools.
fn section_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional block id. A heading id returns that whole section; any other \
                        block id returns just that block (and its output, for code). Get ids \
                        from dx_outline."
    })
}

/// `dx_read` — look at the document, page by page.
fn read_tool() -> Value {
    json!({
        "name": "dx_read",
        "description": "LOOK at a .dx document: returns the rendered pages as images. A \
                        page image costs many times what its text costs, so spend this on \
                        what text cannot carry — boards, diagrams, charts, images, rendered \
                        ::view frames, layout — and read prose and code with dx_source. The \
                        read is live: approved runnable code whose recorded output went \
                        stale is re-run first and the fresh output rendered (unreviewed \
                        code never runs on a read; the reply says what awaits review). \
                        Each page is labelled with the block ids on it. On a long document \
                        call dx_outline first and pass `section`. Pass `block` to photograph \
                        one block alone — a ::board arrives at its natural canvas size, \
                        which is the sharp way to inspect a board or a node. Falls back to \
                        Markdown text when this machine has no browser.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "section": section_property(),
                "block": {
                    "type": "string",
                    "description": "Optional block id: return one image of just this block. \
                                    A board renders at its natural size instead of fitted \
                                    into the page column; a hidden block (a board node) \
                                    renders too. Get ids from dx_outline."
                },
                "refresh": refresh_property(),
                "theme": {
                    "type": "string",
                    "enum": ["auto", "light", "dark"],
                    "description": "Palette to render with. Default: auto."
                },
                "width": {
                    "type": "number",
                    "description": "Page width in CSS pixels. Default: 860, the rendered \
                                    content column plus margins. Pages are sized to stay \
                                    under the vision-ingestion limits, so every captured \
                                    pixel reaches the model unscaled."
                }
            },
            "required": ["path"]
        }
    })
}

/// `dx_play` — watch the page react to input, frame by frame.
fn play_tool() -> Value {
    json!({
        "name": "dx_play",
        "description": "WATCH a .dx document react to input: loads the rendered page in a \
                        headless browser, performs a scripted sequence of real input events, \
                        and returns the frames as images — each stamped with its time and the \
                        action it shows landing, so behaviour can be reviewed frame by frame. \
                        Script statements are separated by ';': `wait 500ms`, `key Space`, \
                        `click <target>`, `scroll 200`, `scroll <target> 200` (a node's own \
                        overflow), `hover <target>`. A target is a block id from dx_outline \
                        or an x,y pixel pair — viewport pixels, unless `node` is set. Set \
                        `node` to clip every frame to one block's box and read x,y inside \
                        that box (0,0 its corner), so a native control inside an embedded \
                        ::view (a checkbox, a <details>) is targetable. Nothing in the \
                        document executes — this drives the same static render dx_read \
                        photographs, and a ::view's own script never runs even here (it is \
                        an inert iframe, on purpose). A live app's own JS-driven \
                        interactivity is not reachable this way at all — dx_run's \
                        lang=capture block is what drives a real, running page (real \
                        clicks, real state, real navigation) and captures it. Use dx_read \
                        for a plain look; use `dx play --out` from a shell to keep every \
                        frame as files.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "script": {
                    "type": "string",
                    "description": "The input sequence, e.g. \"wait 500ms; key Space; scroll 200\"."
                },
                "node": {
                    "type": "string",
                    "description": "Optional block id: clip every frame to this block's box \
                                    and read the script's x,y targets inside it — 0,0 is the \
                                    box's top-left corner."
                },
                "section": section_property(),
                "theme": {
                    "type": "string",
                    "enum": ["auto", "light", "dark"],
                    "description": "Palette to render with. Default: auto."
                },
                "fps": {
                    "type": "number",
                    "description": "Frames per second captured during waits, 1-30. Default: 10."
                },
                "width": {
                    "type": "number",
                    "description": "Viewport width in pixels. Default: 1200."
                }
            },
            "required": ["path", "script"]
        }
    })
}

/// The `refresh` property shared by the live reading tools.
fn refresh_property() -> Value {
    json!({
        "type": "boolean",
        "description": "Re-run approved code whose recorded output is stale before \
                        reading, so the document shows live output. Unreviewed code \
                        never runs on a read. Default: true; false reads exactly what \
                        is stored."
    })
}

/// `dx_source` — the exact words, as Markdown.
fn source_tool() -> Value {
    json!({
        "name": "dx_source",
        "description": "READ a .dx document as text — the default before dx_read, not \
                        Grep or Read. Exact Markdown with headings, lists, code fences, and \
                        captured run output preserved. This is the cheap way to read prose \
                        and code — a fraction of the tokens of dx_read's page images — and \
                        the exact characters, for quoting or preparing a dx_edit. Pass \
                        `section` (any block id) to read one part instead of the whole \
                        document; set `ids` to true for the block ids dx_edit and section \
                        selection need. The read is live: stale output of approved code is \
                        refreshed first. Reach for dx_read only when the page carries what \
                        text cannot: boards, diagrams, charts, layout.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "section": section_property(),
                "refresh": refresh_property(),
                "ids": {
                    "type": "boolean",
                    "description": "Prefix each block with its id. Default: false."
                }
            },
            "required": ["path"]
        }
    })
}

/// `dx_outline` — the map of a document.
fn outline_tool() -> Value {
    json!({
        "name": "dx_outline",
        "description": "Map a document first — the default way to get ids and structure, \
                        not Grep. One row per block with its id, kind, heading level, size, \
                        a preview, and whether it is runnable code. Call this first on a \
                        long document, then fetch only the sections you need with dx_source \
                        or dx_read.",
        "inputSchema": {
            "type": "object",
            "properties": { "path": path_property() },
            "required": ["path"]
        }
    })
}

/// `dx_list` — what documentation exists here.
fn list_tool() -> Value {
    json!({
        "name": "dx_list",
        "description": "List every .dx document in a project, with its title and block count. \
                        Use this to find out what documentation exists before guessing paths. \
                        A project with no documents yet is a project worth indexing: offer to \
                        run dx_index, which scaffolds index.dx for you to improve.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to search. Default: the workspace root."
                }
            },
            "required": []
        }
    })
}

/// `dx_search` — find documents by content.
fn search_tool() -> Value {
    json!({
        "name": "dx_search",
        "description": "Search first in a project with dx documents — this is the default \
                        before reaching for Grep or Bash. This tool searches the .dx \
                        documents *and* its source files — best matches first; a corpus with \
                        a real answer is never shut out entirely by the other. Each hit \
                        carries its answer: the id of the block that best matches and that \
                        block's text (a heading brings its section, capped; a source file's \
                        block id is the line range to open) — so a search that lands needs \
                        no follow-up read. When you do need more, dx_source with `section` \
                        on the hit's block is the cheap route. Ask in a whole question and \
                        in your own words: matching folds word endings (packed finds pack), \
                        splits compound names (packWeights is reachable as pack and \
                        weights), and falls back to a word's kin when the project never says \
                        it on its own (packed reaches bitpack.c, microphone reaches \
                        mic.py).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Words to search for." },
                "directory": {
                    "type": "string",
                    "description": "Directory to search. Default: the workspace root."
                },
                "limit": { "type": "number", "description": "Maximum hits. Default: 20." }
            },
            "required": ["query"]
        }
    })
}

/// `dx_coverage` — how often search lands on a document instead of falling back.
fn coverage_tool() -> Value {
    json!({
        "name": "dx_coverage",
        "description": "How well this project's documents already answer the questions asked \
                        of it. Reports the share of the last `window` dx_search/dx search \
                        calls whose top hit landed on a document rather than falling back to \
                        source or finding nothing, plus the fallback queries themselves, \
                        most-repeated first — that list is the point as much as the rate is: \
                        it names exactly what to write into a document next, per the \
                        write-it-in-at-once loop. Returns `has_data: false` for a workspace \
                        that has not been searched yet, rather than a misleading rate of zero.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to report on. Default: the workspace root."
                },
                "window": {
                    "type": "number",
                    "description": "How many of the most recent searches to summarize. \
                                    Default: 200."
                }
            },
            "required": []
        }
    })
}

/// `dx_render` — the HTML source of the page.
fn render_tool() -> Value {
    json!({
        "name": "dx_render",
        "description": "Get the document as a self-contained HTML page (no external assets). \
                        Use this to embed or publish a document. To simply look at it, use \
                        dx_read, which returns the rendered pages as images instead of markup.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "section": section_property(),
                "theme": { "type": "string", "enum": ["auto", "light", "dark"] }
            },
            "required": ["path"]
        }
    })
}

/// `dx_write` — create or replace a document.
fn write_tool() -> Value {
    json!({
        "name": "dx_write",
        "description": "Create a .dx document or replace its entire contents with DOCSRC \
                        source text. Search dx_search or dx_source first for any related \
                        existing content the request names or describes — reading saves \
                        inventing. The file is written as canonical plain text, so any \
                        tool can read it afterwards. To change one block in an existing \
                        document, use dx_edit instead — it is safer and much smaller.\n\n\
                        DOCSRC blocks look like:\n\
                        ::heading level=1 id=title\\nMy Title\\n::end\n\
                        ::paragraph id=intro\\nSome text.\\n::end\n\
                        ::bulleted-list id=steps\\n- one\\n- two\\n::end\n\
                        ::code id=demo lang=python run deps=\"requests\"\\nprint(1)\\n::end\n\
                        A code block marked `run` is executed by dx_run. A picture a run \
                        produces belongs on an ::image block naming its producer — \
                        ::image src=frames/one.png for=demo — so the page vouches for \
                        its freshness (a failed or unrun producer is called out on the \
                        figure). Embedded image files are capped at 8MB; the save warns \
                        about an oversized one (capture at scale 1, not 2).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "content": {
                    "type": "string",
                    "description": "Full DOCSRC source for the document."
                }
            },
            "required": ["path", "content"]
        }
    })
}

/// `dx_index` — scaffold the project map.
fn index_tool() -> Value {
    json!({
        "name": "dx_index",
        "description": "Scaffold index.dx — a precursor project index built from the file \
                        tree alone: one section per top-level area with its immediate \
                        contents and a TODO paragraph. Run this when dx_list finds no \
                        documentation in a project, then READ THE WHOLE SCAFFOLD and \
                        improve it before other work: replace each TODO with what the area \
                        actually does, and add ::code src= blocks for the load-bearing \
                        files — they render as the file's current text, never a stale \
                        copy — so every later session orients for the price of one read. \
                        Refuses to overwrite an existing index.dx unless force=true, \
                        because an existing index is presumed improved.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to map. Default: the workspace root."
                },
                "force": {
                    "type": "boolean",
                    "description": "Rewrite the scaffold over an existing index.dx. \
                                    Default: false."
                }
            },
            "required": []
        }
    })
}

/// `dx_edit` — change one block.
fn edit_tool() -> Value {
    json!({
        "name": "dx_edit",
        "description": "Change one block, leaving every other block byte-for-byte \
                        unchanged. Read the document first via dx_source if the request \
                        names or describes it — reading saves guessing. This is the safe way \
                        to edit a long document. Get block ids from dx_outline or from \
                        dx_source with ids=true. Pass `text` to replace the whole body — or, \
                        for a small change, `replace`+`with`: the exact string `replace` \
                        becomes `with` and every other character stays, so the call costs the \
                        change, not the block. Prefer replace for renames, one-line fixes, \
                        and cross-cutting edits; a whole-body `text` for one changed word is \
                        tokens spent retyping what already exists. Pass `header` to also \
                        retype the block's `::kind attrs` opening line — the way to change a \
                        block's kind or any attribute (src=, reads=, writes=, run, for=) in \
                        one call; never rewrite a whole document with dx_write to change one \
                        attribute. Editing a runnable \
                        code block RUNS IT immediately, approving the code you just wrote — \
                        the edit is the review, exactly as the editing surface runs a field \
                        the moment it closes — and the fresh output is folded into the \
                        document, so a read after an edit is never stale. Pass run=false to \
                        edit without executing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "block": { "type": "string", "description": "Id of the block to change." },
                "text": {
                    "type": "string",
                    "description": "New body text for the block — the whole body. For a \
                                    small change inside a body, use `replace`+`with` \
                                    instead."
                },
                "replace": {
                    "type": "string",
                    "description": "Exact text to find in the block's body — matched as \
                                    characters, never as a pattern. Must occur exactly \
                                    once unless `all` is true. Stands alone: do not also \
                                    pass `text` or `header`."
                },
                "with": {
                    "type": "string",
                    "description": "What `replace` becomes. An empty string deletes the \
                                    match. Required with `replace`."
                },
                "all": {
                    "type": "boolean",
                    "description": "Replace every occurrence of `replace` instead of \
                                    requiring exactly one. Default: false."
                },
                "header": {
                    "type": "string",
                    "description": "New `::kind attrs` opening line for the block, e.g. \
                                    `::code lang=py run reads=src writes=target`. The body \
                                    is still `text`. The block keeps its id unless the \
                                    header states one; an empty string retypes the block \
                                    as plain prose. Omit to leave the header untouched."
                },
                "run": {
                    "type": "boolean",
                    "description": "Run a runnable block after the edit, approving the new \
                                    code. Default: true; only code blocks marked `run` \
                                    execute either way."
                }
            },
            "required": ["path", "block"]
        }
    })
}

/// `dx_board` — place, arrange, detach, or link a node on a board.
fn board_tool() -> Value {
    json!({
        "name": "dx_board",
        "description": "Place, arrange, detach, or link/unlink a node on a `::board` — the \
                        same collision-safe operations `dx board` and every editing \
                        surface's drag/arrange/link actions perform, so a node placed here \
                        gets the identical board a person dragging one gets: anything it \
                        lands on is shoved out of its way, never left stacked. This is the \
                        tool for board layout — dx_edit writes a board's raw body text \
                        verbatim and never resolves overlap, which is right for stating a \
                        layout by hand but wrong for \"put this node here.\"",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "board": { "type": "string", "description": "Id of the `::board` block." },
                "action": {
                    "type": "string",
                    "enum": ["place", "arrange", "detach", "link", "unlink"],
                    "description": "place: move/add/resize one node, settling the board \
                                    afterwards. arrange: place several nodes in one call \
                                    (needs `placements`), settled once at the end. detach: \
                                    remove a node, its edges, and its block if the block \
                                    was the board's own and no other board shows it. \
                                    link/unlink: draw or erase the edge from `node` to `to`."
                },
                "node": {
                    "type": "string",
                    "description": "The node's id. For `place`, omit for a new node (a \
                                    fresh id is chosen and returned). Required for detach, \
                                    link, unlink — the edge's source node for link/unlink."
                },
                "x": {
                    "type": "number",
                    "description": "place only: left edge in canvas pixels. Omit to keep \
                                    the node's current x (0 for a new node)."
                },
                "y": {
                    "type": "number",
                    "description": "place only: top edge in canvas pixels. Omit to keep \
                                    the node's current y (0 for a new node)."
                },
                "w": {
                    "type": "string",
                    "description": "place only: width — a number of canvas pixels, \
                                    `page`, or `fit`. Omit to keep the node's current width."
                },
                "h": {
                    "type": "string",
                    "description": "place only: height, the same way."
                },
                "placements": {
                    "type": "string",
                    "description": "arrange only: one placement per node, `id,x,y[,w[,h]]`, \
                                    separated by spaces, semicolons, or newlines."
                },
                "to": {
                    "type": "string",
                    "description": "link/unlink only: the edge's target node id."
                },
                "from_side": {
                    "type": "string",
                    "description": "link only: which side of `node` the edge leaves — \
                                    left, right, top, bottom (or their initials). Omit for \
                                    unpinned."
                },
                "to_side": {
                    "type": "string",
                    "description": "link only: which side of `to` the edge meets, the same way."
                }
            },
            "required": ["path", "board", "action"]
        }
    })
}

/// `dx_append` — the cheap write.
fn append_tool() -> Value {
    json!({
        "name": "dx_append",
        "description": "The cheap write: a small thought lands for the cost of its own \
                        lines. Read the target document first via dx_source if the request \
                        names it — that avoids duplicating or contradicting existing content. \
                        Think by writing — a finding, a hypothesis, a worklist line \
                        goes into the document the moment it forms, not into conversation \
                        memory to be distilled later. Pass `block` to append `text` to that \
                        block's body (a finding onto the ledger, a line onto the `now` \
                        worklist) without resending what is already there; pass `after` to \
                        insert a new block after that id; with neither, the new block lands \
                        at the end of the document. New blocks are prose kinds (`type`; \
                        default paragraph) — a code block enters through dx_edit or \
                        dx_write, where its powers are stated and reviewed. Growing a \
                        runnable code block via `block` is an edit like any other: it runs, \
                        and the edit is the review (run=false declines).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "text": {
                    "type": "string",
                    "description": "The lines to add: appended to `block`'s body, or the \
                                    body of the new block."
                },
                "block": {
                    "type": "string",
                    "description": "Id of an existing block to grow. Its body becomes \
                                    body + newline + text. Not combinable with `after`."
                },
                "after": {
                    "type": "string",
                    "description": "Id of the block to insert a new block after. Omit \
                                    both this and `block` to append at the document's end."
                },
                "type": {
                    "type": "string",
                    "description": "Kind of the new block: paragraph, bulleted-list, \
                                    numbered-list, checklist, heading, quote… Default: \
                                    paragraph. Ignored when `block` is given."
                },
                "level": {
                    "type": "number",
                    "description": "A new heading's level (1–4). Default: 2."
                },
                "id": {
                    "type": "string",
                    "description": "Id for the new block; omit to let the document name it. \
                                    A taken id is refused, never silently renamed."
                },
                "run": {
                    "type": "boolean",
                    "description": "When growing a runnable code block: run it after the \
                                    append. Default: true."
                }
            },
            "required": ["path", "text"]
        }
    })
}

/// `dx_check` — tick one box.
fn check_tool() -> Value {
    json!({
        "name": "dx_check",
        "description": "Tick or untick one checklist box — the worklist move: claim a line \
                        before working it, check it off when its verdict holds. The same \
                        toggle a reader's click performs; every other item and block comes \
                        back byte-identical. Cheaper than editing the list's whole body, \
                        and the only honest way to spell the checked state.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "block": { "type": "string", "description": "Id of the checklist block." },
                "item": {
                    "type": "number",
                    "description": "The box's position in the checklist, counting from 0 — \
                                    the order the renderer draws them."
                }
            },
            "required": ["path", "block", "item"]
        }
    })
}

/// `dx_report` — file what dx itself got wrong.
fn report_tool() -> Value {
    json!({
        "name": "dx_report",
        "description": "REPORT DX ITSELF. File a bug, a suggestion, or an observation about \
                        dx — the tools, the CLI, the format, the editing surfaces — the \
                        moment it happens, from whatever project you are working in. The \
                        thing you just worked around, the message that did not say what to \
                        do next, the call you had to make twice, the answer that was not the \
                        block stating the fact: that is a report, and a workaround nobody \
                        filed is a defect that stays. This is the one tool that is not about \
                        the project you are in — a defect in *that* project's own code \
                        belongs in its documents, not here.\n\n\
                        Write `detail` for the person who will fix it: what you did, what \
                        you expected, what happened instead. `route` names the tool or \
                        command involved (dx_search, dx_run, dx sync, the VS Code surface) \
                        and `repro` the smallest sequence that shows it. Which dx, which \
                        platform, which workspace, and when are recorded for you.\n\n\
                        Filing is all you do. The report goes to the intake — the report \
                        database — and from there into the dx checkout's own reports.dx, \
                        which every agent working on dx reads; nobody relays it, and it \
                        waits on nobody's memory. It is written to this machine's inbox \
                        first, so an intake that cannot be reached delays a report and never \
                        loses one. Reporting the same thing twice is welcome: reports are \
                        keyed by kind, title, and route, so a repeat becomes another \
                        sighting on the one block rather than a duplicate, and that count is \
                        how a defect earns its priority.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["bug", "suggestion", "observation"],
                    "description": "bug: dx did something wrong. suggestion: dx could do \
                                    something better. observation: how dx behaved, with no \
                                    claim about whether it is wrong — the most useful kind \
                                    when you are unsure."
                },
                "title": {
                    "type": "string",
                    "description": "One line naming the defect. This, the kind, and the \
                                    route are the report's identity, so write the defect \
                                    rather than the episode: \"dx_search answers with the \
                                    heading instead of the block that states the fact\"."
                },
                "detail": {
                    "type": "string",
                    "description": "What you did, what you expected, and what happened \
                                    instead — enough for someone who was not here to act on \
                                    it without asking you."
                },
                "route": {
                    "type": "string",
                    "description": "The tool, command, or surface involved: dx_search, \
                                    dx_run, `dx sync`, the editor. Optional, and part of \
                                    the identity — the same symptom on two routes is two \
                                    reports."
                },
                "repro": {
                    "type": "string",
                    "description": "The smallest sequence that shows it, when you have one."
                }
            },
            "required": ["kind", "title", "detail"]
        }
    })
}

/// `dx_run` — execute the document's code.
fn run_tool() -> Value {
    json!({
        "name": "dx_run",
        "description": "RUNS CODE. Executes every code block marked `run` in a .dx document \
                        using the machine's own toolchains, installs any libraries the blocks \
                        declare with deps=\"…\", and stores each result in the document as an \
                        ::output block. Nothing else in this server executes anything. \
                        Unchanged blocks are skipped, so re-running is cheap. New or edited \
                        code is blocked until reviewed: pass review=true to see each block's \
                        exact code and fingerprint without executing (review writes nothing \
                        at all), then approve=true to approve the current code and run it. \
                        Only this machine's own approvals count — a result already recorded \
                        in the document approves nothing — and editing a block changes its \
                        fingerprint, so approval expires with the edit. A block that must \
                        write into the document's folder (a build directory, generated \
                        files, a test run over the repository) declares it with \
                        writes=target,gen — folders inside the document's folder only, \
                        created if missing; the grant joins the fingerprint, so review \
                        shows it and widening it re-opens review. It grants folders, never \
                        loose files, so a tool that rewrites one beside the document needs \
                        the flag that tells it not to (`cargo test --locked`). The sandbox \
                        otherwise keeps the folder read-only, and the network stays gone — \
                        with one deliberate exception: lang=capture. A block \
                        `::code lang=capture run target=<url> writes=out` opens a live, \
                        real URL (a running dev server, any page) in dx's own trusted \
                        browser, not the block's own code, and reaches only that one URL — \
                        every other host and port stays refused, the same way every other \
                        capture in this codebase refuses one. Its body is JavaScript, \
                        evaluated in that live page once it settles, so this is how a page \
                        is driven into a state before it is captured — fill a form, call \
                        the app's own API, wait for a condition — not only ever whatever \
                        renders on load; the last non-null value it returns is recorded. \
                        Give it a `setup=` shell command when the target needs starting \
                        first (most often the dev server itself) — that command gets the \
                        network too, same as any other runner's own deps= install, and is \
                        killed the moment the capture finishes. Add the bare `actions` \
                        attribute to write the body as a short action script instead of \
                        raw JavaScript — one statement per line or `;`-separated: \
                        `wait 500ms`, `click <selector>`, `hover <selector>`, \
                        `type <selector> \"text\"`, `key <name>`, `scroll <pixels>` or \
                        `scroll <selector> <pixels>`, and `eval <js>` to drop back into \
                        raw JavaScript for anything else — `<selector>` is a CSS selector \
                        (the target is not this document, so there is no block id to \
                        target). The screenshot lands in \
                        writes= as `<block id>.png`; reference it with \
                        `::image src=<block id>.png for=<block id>` the same as any other \
                        run's picture. This is the answer to \"::view only shows a static \
                        local file\" — ::view stays that way on purpose (it is a read, and \
                        reads never execute); a live app's current, driven-into-a-state \
                        rendering is what lang=capture is for. What a block reads it \
                        declares with reads=src,data.csv \
                        — files or folders; a folder covers every file under it (hidden \
                        entries, target/node_modules, and the block's own writes= folders \
                        left out). Declared content joins the run fingerprint, so an \
                        edited input re-runs approved code by itself; approval names the \
                        declared paths, never the content. Inside the sandbox $HOME is \
                        redirected, so a toolchain keyed off a real home is named \
                        explicitly (export CARGO_HOME from the rustup cargo's own path). \
                        After running, call dx_read to see the results rendered.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "block": {
                    "type": "string",
                    "description": "Run only this block id. Default: every runnable block."
                },
                "force": {
                    "type": "boolean",
                    "description": "Re-run blocks whose code has not changed, and run \
                                    unapproved code once — the bypass is announced in the \
                                    block's own output. Default: false."
                },
                "review": {
                    "type": "boolean",
                    "description": "Execute nothing: report each runnable block's exact \
                                    code, fingerprint, and approval status. Refused when \
                                    combined with approve — review records nothing. \
                                    Default: false."
                },
                // Deliberately agent-reachable: an agent asked to run a document must be
                // able to, and the block still runs confined — no network, no writes outside
                // its own directory. What `approve` buys past the sandbox is a durable
                // machine-wide record, so an agent that has not read the code should call
                // `review` first and say what it found. See README, "What executes".
                "approve": {
                    "type": "boolean",
                    "description": "Approve each runnable block's current fingerprint on \
                                    this machine, then execute. Read the code first — \
                                    review=true shows it — and say what it does; the \
                                    approval outlives this call and covers any document \
                                    carrying the same code. Default: false."
                },
                "follow_edges": {
                    "type": "boolean",
                    "description": "Run blocks in the order the document's board edges \
                                    state (an edge means this-then-that, conducted through \
                                    non-runnable nodes too) instead of document order. At \
                                    every step the earliest ready block by document \
                                    position runs next, so an edge that defers a block \
                                    lets later blocks — on a board or not — run before \
                                    it; ties break by document order. Only stated edges \
                                    order side effects — state an edge if you need an \
                                    order. A cycle among runnable blocks is an error \
                                    naming it. Default: false — document order."
                }
            },
            "required": ["path"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_tool_is_in_the_catalogue_exactly_once() {
        let tools = catalogue();
        let entries = tools.as_array().expect("array");
        assert_eq!(entries.len(), TOOL_NAMES.len());
        for name in TOOL_NAMES {
            let matches = entries.iter().filter(|tool| tool["name"] == *name).count();
            assert_eq!(matches, 1, "{name} appears {matches} times");
        }
    }

    /// The catalogue order is the reading economy: find, map, read the text, then look.
    /// Text costs a fraction of a page image, so `dx_source` stands ahead of `dx_read`,
    /// and `dx_read` spends its images on what text cannot carry.
    #[test]
    fn the_catalogue_orders_the_reading_economy() {
        let tools = catalogue();
        let names: Vec<&str> = tools
            .as_array()
            .expect("array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("name"))
            .collect();
        let position = |name: &str| {
            names
                .iter()
                .position(|n| *n == name)
                .unwrap_or_else(|| panic!("{name} missing"))
        };
        assert!(position("dx_outline") < position("dx_source"));
        assert!(position("dx_source") < position("dx_read"));

        let source = &tools.as_array().expect("array")[position("dx_source")];
        let description = source["description"].as_str().expect("description");
        assert!(description.starts_with("READ"));
        assert!(description.contains("exact"));
        assert!(description.contains("dx_edit"));

        let read = &tools.as_array().expect("array")[position("dx_read")];
        let description = read["description"].as_str().expect("description");
        assert!(description.contains("images"));
        assert!(description.contains("section"));
        assert!(description.contains("boards"));
    }

    /// Both reading tools carry the live-output switch, on by default: a read shows what
    /// approved code does now, and only unreviewed code waits.
    #[test]
    fn the_reads_are_live_and_the_refresh_is_declinable() {
        let tools = catalogue();
        for name in ["dx_read", "dx_source"] {
            let tool = tools
                .as_array()
                .expect("array")
                .iter()
                .find(|tool| tool["name"] == name)
                .expect(name);
            assert_eq!(
                tool["inputSchema"]["properties"]["refresh"]["type"], "boolean",
                "{name} is missing `refresh`"
            );
            assert!(tool["description"]
                .as_str()
                .expect("description")
                .contains("live"));
        }
    }

    /// The edit is the review: the edit tool says it runs what it writes, and offers the
    /// way to decline.
    #[test]
    fn the_edit_tool_declares_its_immediate_run() {
        let tools = catalogue();
        let edit = tools
            .as_array()
            .expect("array")
            .iter()
            .find(|tool| tool["name"] == "dx_edit")
            .expect("dx_edit");
        assert!(edit["description"]
            .as_str()
            .expect("description")
            .contains("RUNS IT"));
        assert_eq!(edit["inputSchema"]["properties"]["run"]["type"], "boolean");
    }

    /// The scaffold tool tells its reader the scaffold is a beginning, not a deliverable.
    #[test]
    fn the_index_tool_demands_improvement() {
        let tools = catalogue();
        let index = tools
            .as_array()
            .expect("array")
            .iter()
            .find(|tool| tool["name"] == "dx_index")
            .expect("dx_index");
        let description = index["description"].as_str().expect("description");
        assert!(description.contains("index.dx"));
        assert!(description.contains("improve"));
        assert!(description.contains("::code src="));
    }

    #[test]
    fn every_tool_declares_a_usable_schema() {
        for tool in catalogue().as_array().expect("array") {
            let name = tool["name"].as_str().expect("name");
            assert!(
                tool["description"]
                    .as_str()
                    .is_some_and(|text| text.len() > 60),
                "{name} needs a description an agent can act on"
            );
            assert_eq!(tool["inputSchema"]["type"], "object", "{name} schema");
            assert!(
                tool["inputSchema"]["properties"].is_object(),
                "{name} props"
            );
            assert!(
                tool["inputSchema"]["required"].is_array(),
                "{name} required"
            );
        }
    }

    #[test]
    fn the_executing_tool_is_unmistakable() {
        let tools = catalogue();
        let run = tools
            .as_array()
            .expect("array")
            .iter()
            .find(|tool| tool["name"] == "dx_run")
            .expect("dx_run");
        assert!(run["description"]
            .as_str()
            .expect("description")
            .starts_with("RUNS CODE."));
    }

    /// The approval gate is reachable over MCP: an agent can review, approve, and force
    /// exactly as the CLI can, or the gate would depend on which surface asked.
    #[test]
    fn the_run_tool_exposes_the_review_gate() {
        let tools = catalogue();
        let run = tools
            .as_array()
            .expect("array")
            .iter()
            .find(|tool| tool["name"] == "dx_run")
            .expect("dx_run")
            .clone();
        for param in ["review", "approve", "force", "follow_edges"] {
            assert_eq!(
                run["inputSchema"]["properties"][param]["type"], "boolean",
                "dx_run is missing the `{param}` parameter"
            );
        }
    }

    /// The reporting tool has to say what it is for — dx, not the project the agent is
    /// working in — and offer the three kinds, or every field report arrives as a bug.
    #[test]
    fn the_report_tool_names_its_subject_and_its_kinds() {
        let tools = catalogue();
        let report = tools
            .as_array()
            .expect("array")
            .iter()
            .find(|tool| tool["name"] == "dx_report")
            .expect("dx_report");
        let description = report["description"].as_str().expect("description");
        assert!(description.starts_with("REPORT DX ITSELF."));
        assert!(description.contains("workaround"));
        // What the tool promises an agent: filing is the whole job, because the report
        // travels to the checkout by itself.
        assert!(description.contains("reports.dx"));
        assert!(description.contains("Filing is all you do"));
        assert_eq!(
            report["inputSchema"]["properties"]["kind"]["enum"],
            json!(["bug", "suggestion", "observation"])
        );
        assert_eq!(
            report["inputSchema"]["required"],
            json!(["kind", "title", "detail"])
        );
    }

    #[test]
    fn the_write_tool_teaches_the_format_it_expects() {
        let tools = catalogue();
        let write = tools
            .as_array()
            .expect("array")
            .iter()
            .find(|tool| tool["name"] == "dx_write")
            .expect("dx_write");
        let description = write["description"].as_str().expect("description");
        assert!(description.contains("::heading"));
        assert!(description.contains("::end"));
    }
}
