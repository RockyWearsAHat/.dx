# dx — a notepad for code

A `.dx` file is a document you can read, a page you can look at, and a program you can run.

```text
::heading level=1 id=title
Latency report
::end

::paragraph id=intro
The numbers below are computed when this document runs.
::end

::code id=stats lang=python run deps="numpy"
import numpy as np
print(np.percentile([12, 18, 9, 44, 15], 95))
::end

::output id=stats-output for=stats status=ok hash=b132…
41.6
::end
```

That is the whole idea. The file is plain text, so `cat`, `grep`, `git diff`, and any editor
work on it. Open it in VS Code and it renders as a page. Run `dx run` and the code executes
and stores what it produced, right there in the file, for whoever opens it next.

---

## Install

```bash
cd rust && cargo build --release -p doc-cli
./target/release/dx install
```

`dx install` copies the binary somewhere on your `PATH` and registers its MCP server with
every AI assistant it finds — Claude, Codex, Cursor, VS Code, Windsurf, Gemini. Run
`dx doctor` afterwards to see what is wired up and what is missing.

For the editor:

```bash
cd editor/vscode && npm install && npm run package
code --install-extension dx-documents-1.0.0.vsix
```

The extension is platform-independent — the same `.vsix` runs on macOS, Windows, and Linux.

---

## Using it

```bash
dx text     notes.dx                 # the document as Markdown
dx outline  notes.dx                 # block ids, kinds, and previews
dx render   notes.dx                 # a self-contained HTML page
dx png      notes.dx                 # an image of the rendered page
dx open     notes.dx                 # open it in your browser
dx run      notes.dx                 # execute its code blocks
dx ls       .                        # every .dx document in a project
dx search   "deploy" .               # find documents by content
```

Add `--section <block-id>` to any reading command to get one part of a long document.
`dx help` lists everything.

### Writing

Edit `.dx` files in any editor — they are text. When you want a targeted change:

```bash
dx new    guide.dx --title "Deployment guide"
dx set    guide.dx intro --text "New opening paragraph."
dx append guide.dx --type code --lang python --run --text "print(1)"
dx fmt    guide.dx                   # rewrite in canonical form
```

`dx set` replaces one block by id and leaves every other byte alone.

### Running code

Mark a code block `run` and name what it needs:

```text
::code id=chart lang=python run deps="numpy matplotlib" timeout=120 format=svg
```

| Attribute | Meaning |
|-----------|---------|
| `run`     | This block is executable |
| `deps`    | Libraries to install before running |
| `timeout` | Seconds before the block is killed (default 60) |
| `format`  | `svg` or `html` — render the output as a drawing, not as quoted text |

Languages: **python** (via `uv` or a cached virtualenv), **node**, **typescript** (Deno),
**bash**, **rust** (Cargo), **go**, **ruby**. Each uses the toolchain already on your
machine, so a block behaves exactly like the same code pasted into a terminal.

Blocks run in the document's own directory, so `open("data.csv")` finds the file next to
the `.dx`. Results are fingerprinted: re-running a document only executes what changed.

**Running a document runs its code.** It happens only through `dx run` or the `dx_run` tool
— never while reading, rendering, or screenshotting. Set `DX_NO_EXEC=1` to disable it
entirely.

See [`examples/showcase.dx`](examples/showcase.dx) for a document that computes its own
numbers and draws its own chart.

---

## For AI agents

Once `dx install` has run, any MCP-capable assistant gets nine tools:

| Tool | What it does |
|------|--------------|
| `dx_view` | **Returns the rendered page as an image.** Use it whenever a document has tables, charts, diagrams, or program output |
| `dx_read` | The document as Markdown |
| `dx_outline` | One row per block: id, kind, size, whether it is runnable |
| `dx_list` / `dx_search` | Find documents in a project |
| `dx_render` | The HTML page source |
| `dx_write` / `dx_edit` | Create a document, or replace one block by id |
| `dx_run` | Execute the code blocks and report what each produced |

`dx_view` exists so an agent can *look* at a result instead of reasoning about the code
that would have produced it. A generated chart arrives as a chart.

An assistant without MCP support needs no configuration at all — `dx` is a command, and
`dx help` explains itself.

---

## How it fits together

```text
                    ┌─────────────┐
     notes.dx ─────►│  doc-core   │──► HTML page  ──► browser, editor, screenshot
    (plain text)    │  parse +    │──► Markdown   ──► agents, diffs
                    │  render     │──► outline    ──► navigation
                    └──────┬──────┘
                           │ compiled twice
              ┌────────────┴────────────┐
        native binary              WebAssembly
        (dx CLI, dx mcp)           (VS Code extension)
```

One engine, compiled for both hosts. That is what guarantees a document cannot look like
one thing in your editor and another thing to an agent — the editor and the CLI produce
byte-identical HTML.

| Crate | Responsibility |
|-------|----------------|
| `rust/doc-core` | The format and the views: parse, canonical write, HTML, Markdown, outline, sections. No OS dependencies; compiles to wasm |
| `rust/doc-run` | Executing code blocks: language plans, dependency installation, sandboxes, timeouts |
| `rust/doc-shot` | Rendering a document to PNG with an installed Chromium browser |
| `rust/doc-cli` | The `dx` command and the MCP server |
| `rust/doc-wasm` | `doc-core` for JavaScript hosts |
| `editor/vscode` | The VS Code extension |

The format itself is specified in [`docs/dx-format-contract.md`](docs/dx-format-contract.md).

---

## Development

```bash
cd rust
cargo test                                  # 238 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Every public item is documented, there is no `unsafe`, and library code does not panic.

The `src/` and `vscode-extension/` directories hold the original TypeScript implementation.
It is the historical reference the Rust core was validated against — `doc-core`'s fixtures
still assert byte-identical output against it — and is no longer on the path any user or
agent takes.
