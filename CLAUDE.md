# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`dx` is a document format, a store, and a toolchain. A `.dx` document is block structured and
canonical, renders to a page, and can execute the code blocks inside it. `README.md` is the
user-facing description; read it first, and keep it true.

**A `.dx` file on disk is a one-line pointer, not the content.** The content lives in the
workspace store as content-addressed compressed chunks:

```
~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823
```

The rule everything rests on: **the resolver always hands back the true document.** Every read
— a person in the editor, `dx render`, an agent calling `dx_read`, git showing a diff — goes
through `doc-cli::workspace` or `doc_store::Store` and gets real content. Anything that lets a
caller see a pointer where a document belongs, or nothing where content exists, is a
regression. A missing document is an error that says what to run; it is never an empty one.

The digest is *in* the pointer on purpose. It makes the file change exactly when the content
changes, so git still tracks each document, and the diff driver can expand it. A constant
marker would make every edit invisible to git.

### What lives where

| Path | What it is | Committed? |
|------|------------|-----------|
| `*.dx` | One-line pointer carrying the content digest | yes |
| `.doc/repo.dxcp` | The repository's documents, chunk-deduplicated and compressed | **yes** — this is the content |
| `.doc/local.dxcp` | Git-ignored scratch documents | no |
| `.doc/index.db` | SQLite: the queryable local authority, rebuildable from the packs | no |

`dx sync` reconciles all of it: it adopts plain-text `.dx` files anything else wrote, restores
documents from the packs when the index is missing, and reports what it cannot resolve rather
than discarding it.

## Where the code lives

The engine is Rust, in `rust/`, and it is the only thing on a user's or agent's path.

| Crate | Responsibility |
|-------|----------------|
| `doc-core` | The format and its views. `format` (DOCSRC parse/stringify), `render` (HTML, Markdown, outline, sections), `chunk` (content-addressed per-block chunks and the `DXCP1` pack), plus `digest`, `compress`, `search`. No OS dependencies — this crate also compiles to `wasm32`. |
| `doc-store` | The SQLite chunk store and the resolver: manifests, packs, git routing, and the stub format. The authority for content. |
| `doc-run` | Executing `::code … run` blocks: per-language plans, dependency install, sandboxes, timeouts, output capture. |
| `doc-shot` | Rendering a document to PNG through an installed Chromium browser (two passes: measure, then capture). |
| `doc-cli` | The `dx` binary: CLI commands, the MCP server (`dx mcp`), and the installer. |
| `doc-wasm` | `doc-core` for JavaScript hosts. Built into `editor/vscode/wasm/`. |

`editor/vscode` is the VS Code extension. It renders through `doc-wasm`, so the editor and
the CLI produce byte-identical HTML — that equality is the point, and there is a test for it.

`src/` and `vscode-extension/` are the **original TypeScript implementation**. Nothing in the
shipped product runs it, and it is no longer a compatibility target — its round-trip quirks
were silently destroying content (see the format note below). Do not extend it, and do not
delete it without asking.

### The examples stay readable, and the fixtures are hermetic

`examples/` and `documents/` are kept as **plain text** so someone browsing the repository can
read them. `doc-core`'s round-trip assertions therefore read `tests/fixtures/*.input.dx`
copies, not those files — otherwise a `dx sync` at the repository root would convert them and
the suite would start testing pointers instead of documents. `tests/fixtures.rs` fails if a
copy drifts from the document it mirrors, so refreshing an example is a deliberate two-file
change.

## Non-negotiables

- **Storage cannot lose a byte.** A chunk holds the exact canonical text `stringify` writes for
  one block, so reassembly is concatenation and no field can be dropped by a codec that had not
  heard of it. Any encoding that understands only *some* block attributes has no place in the
  store — a structured binary one was removed for exactly that reason.
- **Canonical output must not drift.** `doc-core/src/format` round-trips real documents
  byte-for-byte against captured fixtures. `docs/dx-format-contract.md` is the authority on
  behavior. If a change makes those tests fail, the change is almost certainly wrong — but the
  fixtures are captured output, not scripture: four of them encoded genuine data loss (checklist
  items erased on save, nested list items dropped, unnamed paragraphs destroyed on re-read).
  Fixing a defect and regenerating the fixture is correct; changing a fixture to make a
  convenient change pass is not. Review the regenerated diff and say what changed and why.
- **Format changes are additive.** A new attribute must serialize to nothing when unset, or
  every existing document reformats on next save.
- **Reading never executes, and never writes.** Parsing, rendering, screenshotting, and
  resolving a pointer must stay free of side effects — resolving must not even create the index.
  Only `dx run` and the `dx_run` tool execute code.
- **One engine.** If the editor and the CLI could render differently, they will. Render
  through `doc-core`; never re-implement a view. Rebuild the wasm after touching `doc-core` or
  the editor silently keeps the old renderer.
- **The page is a blank sheet.** One paper tone, one ink, no second surface: no panels, fills,
  tints, grids, rounded boxes, or shadows. Structure comes from type, whitespace, and hairline
  rules. Nothing is announced unless it earns the space — a successful run says nothing; only a
  failure is called out.

## Quality bar

```bash
cd rust
cargo test                                  # must be green
cargo clippy --all-targets -- -D warnings   # must be clean
cargo fmt --check                           # must be clean
```

No `unsafe`. Every public item documented. No panics in library code — fallible operations
return `Result`, and user-facing failures return a sentence saying what to do about it.

Tests are colocated with the code they test and named for the behavior they pin down. When
you fix a bug, add the test that would have caught it. Non-ASCII prose, empty input, and
missing toolchains are all real cases that have broken this code before.

## Building and trying it

```bash
cd rust && cargo build --release -p doc-cli   # → rust/target/release/dx
./target/release/dx doctor                    # what is installed and what is missing
./target/release/dx run ../examples/showcase.dx
```

Do **not** run `dx sync` at the repository root: it would convert `examples/` and `documents/`
into pointers. Try the store in a scratch directory instead:

```bash
mkdir /tmp/pad && cd /tmp/pad && git init -q .
cp ~/Desktop/DOC/examples/welcome.dx .
dx sync .          # adopt it: the file becomes a pointer, content goes to .doc/
dx git-setup .     # make git diff, git show, and git log -p render documents
cat welcome.dx     # a pointer
dx text welcome.dx # the document
dx stats .         # sharing and compaction
```

The VS Code extension needs the wasm build first:

```bash
cd rust/doc-wasm && wasm-pack build --release --target nodejs --out-dir ../../editor/vscode/wasm
cd editor/vscode && npm install && npm run package
code --install-extension dx-documents-1.0.0.vsix
```

`wasm-pack` needs the rustup toolchain on `PATH` ahead of any Homebrew `rustc`
(`PATH="$HOME/.cargo/bin:$PATH"`), and the `wasm32-unknown-unknown` target.

## Handoff discipline

After completing each task or wave, before ending the turn, update `HANDOFF.md` with what
was done **and validated** (with the proof) and the single concrete next step. Keep it
short — it is the resume point after `/compact` or `/clear`.
