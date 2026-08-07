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
    "dx_outline",
    "dx_source",
    "dx_read",
    "dx_play",
    "dx_render",
    "dx_index",
    "dx_write",
    "dx_edit",
    "dx_run",
];

/// Build the `tools/list` payload.
#[must_use]
pub fn catalogue() -> Value {
    json!([
        list_tool(),
        search_tool(),
        outline_tool(),
        source_tool(),
        read_tool(),
        play_tool(),
        render_tool(),
        index_tool(),
        write_tool(),
        edit_tool(),
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
                        or an x,y pixel pair. Set `node` to clip every frame to one block's \
                        box. Nothing in the document executes — this drives the same static \
                        render dx_read photographs. Use dx_read for a plain look; use `dx play \
                        --out` from a shell to keep every frame as files.",
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
                    "description": "Optional block id: clip every frame to this block's box."
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
        "description": "READ a .dx document as text: exact Markdown with headings, lists, \
                        code fences, and captured run output preserved. This is the cheap \
                        way to read prose and code — a fraction of the tokens of dx_read's \
                        page images — and the exact characters, for quoting or preparing a \
                        dx_edit. Pass `section` (any block id) to read one part instead of \
                        the whole document; set `ids` to true for the block ids dx_edit and \
                        section selection need. The read is live: stale output of approved \
                        code is refreshed first. Reach for dx_read only when the page \
                        carries what text cannot: boards, diagrams, charts, layout.",
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
        "description": "Map a .dx document without reading all of it: one row per block with \
                        its id, kind, heading level, size, a preview, and whether it is \
                        runnable code. Call this first on a long document, then fetch only \
                        the sections you need with dx_read.",
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
        "description": "Search every .dx document in a project by content and title, best \
                        matches first. Returns paths and titles; follow up with dx_read on the \
                        ones that look right.",
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
                        source text. The file is written as canonical plain text, so any \
                        tool can read it afterwards. To change one block in an existing \
                        document, use dx_edit instead — it is safer and much smaller.\n\n\
                        DOCSRC blocks look like:\n\
                        ::heading level=1 id=title\\nMy Title\\n::end\n\
                        ::paragraph id=intro\\nSome text.\\n::end\n\
                        ::bulleted-list id=steps\\n- one\\n- two\\n::end\n\
                        ::code id=demo lang=python run deps=\"requests\"\\nprint(1)\\n::end\n\
                        A code block marked `run` is executed by dx_run.",
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
        "description": "Replace the body of one block, leaving every other block byte-for-byte \
                        unchanged. This is the safe way to edit a long document. Get block ids \
                        from dx_outline or from dx_source with ids=true. Editing a runnable \
                        code block RUNS IT immediately, approving the code you just wrote — \
                        the edit is the review, exactly as the editing surface runs a field \
                        the moment it closes — and the fresh output is folded into the \
                        document, so a read after an edit is never stale. Pass run=false to \
                        edit without executing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "block": { "type": "string", "description": "Id of the block to replace." },
                "text": { "type": "string", "description": "New body text for the block." },
                "run": {
                    "type": "boolean",
                    "description": "Run a runnable block after the edit, approving the new \
                                    code. Default: true; only code blocks marked `run` \
                                    execute either way."
                }
            },
            "required": ["path", "block", "text"]
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
                        otherwise keeps the folder read-only, and the network stays gone \
                        either way. What a block reads it declares with reads=src,data.csv \
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
