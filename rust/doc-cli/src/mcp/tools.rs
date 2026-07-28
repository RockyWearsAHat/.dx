//! The MCP tool catalogue.
//!
//! Tool descriptions are the only documentation an agent ever reads, so they are written
//! for that reader: each one says what it returns, when to reach for it, and which tool to
//! use instead when this is the wrong one.
//!
//! The ordering is deliberate. `dx_view` — the *picture* of a document — comes before the
//! text tools, and its description says to prefer it. An agent that can see a rendered
//! table, chart, or diagram reasons about the actual result instead of imagining one from
//! source, which is the whole reason the viewing path exists.

use serde_json::{json, Value};

/// Every tool name the server advertises, in catalogue order. Used by the tests that
/// keep [`catalogue`] and this list in step.
#[cfg(test)]
pub const TOOL_NAMES: &[&str] = &[
    "dx_view",
    "dx_read",
    "dx_outline",
    "dx_list",
    "dx_search",
    "dx_render",
    "dx_write",
    "dx_edit",
    "dx_run",
];

/// Build the `tools/list` payload.
#[must_use]
pub fn catalogue() -> Value {
    json!([
        view_tool(),
        read_tool(),
        outline_tool(),
        list_tool(),
        search_tool(),
        render_tool(),
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

/// `dx_view` — see the document as a picture.
fn view_tool() -> Value {
    json!({
        "name": "dx_view",
        "description": "LOOK at a .dx document: returns the rendered page as an image. \
                        PREFER THIS over dx_read whenever the document contains tables, \
                        charts, diagrams, SVG, layout, or captured program output — you will \
                        see the actual rendered result instead of inferring it from source. \
                        Use `section` to view one part of a long document. Falls back to \
                        Markdown text automatically if this machine has no browser installed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "section": section_property(),
                "theme": {
                    "type": "string",
                    "enum": ["auto", "light", "dark"],
                    "description": "Palette to render with. Default: auto."
                },
                "width": {
                    "type": "number",
                    "description": "Image width in pixels. Default: 1200."
                }
            },
            "required": ["path"]
        }
    })
}

/// `dx_read` — the words, as Markdown.
fn read_tool() -> Value {
    json!({
        "name": "dx_read",
        "description": "Read a .dx document as Markdown, with headings, lists, code fences, \
                        and captured output preserved. Use this when you need the exact text \
                        (to quote it, edit it, or search within it). For anything visual, use \
                        dx_view instead. Set `ids` to true to get each block's id, which you \
                        need for dx_edit and for section selection.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "section": section_property(),
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
                        the sections you need with dx_view or dx_read.",
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
                        Use this to find out what documentation exists before guessing paths.",
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
                        matches first. Returns paths and titles; follow up with dx_view or \
                        dx_read on the ones that look right.",
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
                        dx_view, which returns an image instead of markup.",
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

/// `dx_edit` — change one block.
fn edit_tool() -> Value {
    json!({
        "name": "dx_edit",
        "description": "Replace the body of one block, leaving every other block byte-for-byte \
                        unchanged. This is the safe way to edit a long document. Get block ids \
                        from dx_outline or from dx_read with ids=true.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": path_property(),
                "block": { "type": "string", "description": "Id of the block to replace." },
                "text": { "type": "string", "description": "New body text for the block." }
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
                        Unchanged blocks are skipped, so re-running is cheap. After running, \
                        call dx_view to see the results rendered.",
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
                    "description": "Re-run blocks whose code has not changed. Default: false."
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

    #[test]
    fn the_viewing_tool_comes_first_and_says_to_prefer_it() {
        let tools = catalogue();
        let first = &tools.as_array().expect("array")[0];
        assert_eq!(first["name"], "dx_view");
        let description = first["description"].as_str().expect("description");
        assert!(description.contains("PREFER THIS"));
        assert!(description.contains("image"));
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
