# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`dx` is a document format and toolchain. A `.dx` file is **plain text on disk** — block
structured, canonical, diffable — that renders to a page and can execute the code blocks
inside it. `README.md` is the user-facing description; read it first, and keep it true.

The rule everything rests on: **a `.dx` file is its own content.** No database, no stub
pointer, no sidecar to stay in sync with. Anything that would make a `.dx` file unreadable
with `cat` is a regression, not a feature.

## Where the code lives

The engine is Rust, in `rust/`, and it is the only thing on a user's or agent's path.

| Crate | Responsibility |
|-------|----------------|
| `doc-core` | The format and its views. `format` (DOCSRC parse/stringify), `render` (HTML, Markdown, outline, sections), plus `digest`, `compress`, `docbin`, `bundle`, `search`. No OS dependencies — this crate also compiles to `wasm32`. |
| `doc-run` | Executing `::code … run` blocks: per-language plans, dependency install, sandboxes, timeouts, output capture. |
| `doc-shot` | Rendering a document to PNG through an installed Chromium browser (two passes: measure, then capture). |
| `doc-cli` | The `dx` binary: CLI commands, the MCP server (`dx mcp`), and the installer. |
| `doc-wasm` | `doc-core` for JavaScript hosts. Built into `editor/vscode/wasm/`. |

`editor/vscode` is the VS Code extension. It renders through `doc-wasm`, so the editor and
the CLI produce byte-identical HTML — that equality is the point, and there is a test for it.

`src/` and `vscode-extension/` are the **original TypeScript implementation**, kept as the
historical reference the Rust core was validated against. `doc-core`'s fixtures still assert
byte-identical output against it. Nothing in the shipped product runs it. Do not extend it,
and do not delete it without asking.

## Non-negotiables

- **Canonical output must not drift.** `doc-core/src/format` round-trips real documents
  byte-for-byte against captured fixtures. If a change makes those tests fail, the change is
  wrong — not the fixtures. `docs/dx-format-contract.md` is the authority on behavior.
- **Format changes are additive.** A new attribute must serialize to nothing when unset, or
  every existing document reformats on next save.
- **Reading never executes.** Parsing, rendering, and screenshotting must stay free of side
  effects. Only `dx run` and the `dx_run` tool execute code.
- **One engine.** If the editor and the CLI could render differently, they will. Render
  through `doc-core`; never re-implement a view.

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
