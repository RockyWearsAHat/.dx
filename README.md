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

That is the document. Open it in VS Code and it renders as a page — clean, always, like a
sheet of paper. Run `dx run` and the code executes and stores what it produced, so whoever
opens it next sees the results, not just the source.

### Where the document actually lives

On disk, `notes.dx` is one line:

```text
~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823
```

The content is in the workspace store, in `.doc/`, held as **content-addressed chunks** — one
per block, compressed, and stored once no matter how many documents or versions share it. Edit
one paragraph in a fifty-block document and only that paragraph is new bytes.

Everything that reads a document resolves that pointer and gets the real thing: `dx text`,
`dx render`, the editor, and every MCP tool an agent calls. Git does too — after `dx git-setup`,
`git diff`, `git show`, and `git log -p` render document diffs, not digests:

```diff
 ::paragraph id=intro
-The numbers below are computed when this document runs.
+The numbers below are recomputed on every run.
 ::end
```

The digest is *in* the pointer so that the file changes exactly when the content does. Git
keeps tracking each document individually, and nothing has to be kept in sync by hand.

Measured on this repository's six example documents: 17,357 bytes of canonical source becomes
an 8,564-byte pack — 49%, losslessly.

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

### The store

```bash
dx sync      .          # adopt, restore, and repair — run this in a new project
dx git-setup .          # make git diff/show/log -p render documents
dx stats     .          # documents, block sharing, compaction
dx textconv  notes.dx   # print what a pointer stands for
```

`dx sync` is the one command that repairs a workspace. It adopts any plain-text `.dx` file
something else wrote, rebuilds the index from `.doc/repo.dxcp` when it is missing — the
fresh-clone case, where all you have is pointers and the committed pack — and rewrites pointers
that drifted. It never discards content: a pointer it cannot resolve is reported, not blanked.

Commit `.doc/repo.dxcp` — that is where your documents are. `dx git-setup` adds the
`.gitignore` lines that keep the rebuildable index out.

### Writing

Edit `.dx` files in your editor and they render as you go. When you want a targeted change from
the command line:

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
   notes.dx            .doc/index.db  +  .doc/repo.dxcp
  (one-line   ────────►  chunks, manifests, packs  ◄──── the content
   pointer)                        │
                                   │ resolve (always the true document)
                                   ▼
                            ┌─────────────┐
                            │  doc-core   │──► HTML page ──► browser, editor, screenshot
                            │  parse +    │──► Markdown  ──► agents, git diffs
                            │  render     │──► outline   ──► navigation
                            └──────┬──────┘
                                   │ compiled twice
                      ┌────────────┴────────────┐
                native binary              WebAssembly
                (dx CLI, dx mcp)           (VS Code extension)
```

One engine, compiled for both hosts. That is what guarantees a document cannot look like
one thing in your editor and another thing to an agent — the editor and the CLI produce
byte-identical HTML, and there is a test that asserts it.

A chunk holds the exact canonical text the writer would emit for one block, so reassembling a
document is concatenation rather than re-serialization. That is what makes the store lossless:
it cannot drop a field it had not heard of. Each saved version keeps a manifest — a list of the
chunks it is made of — so an old revision stays readable for almost nothing, which is what lets
`git log -p` render history.

| Crate | Responsibility |
|-------|----------------|
| `rust/doc-core` | The format and the views: parse, canonical write, HTML, Markdown, outline, sections, and content-addressed chunks. No OS dependencies; compiles to wasm |
| `rust/doc-store` | The SQLite chunk store and the resolver: manifests, packs, git routing, pointers |
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
