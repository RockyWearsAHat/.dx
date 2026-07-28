# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

DOC is a canonical block-document system for human + AI collaboration. Rendering and
authoring are kept separate: humans edit visual blocks in a VS Code webview; AI agents
read/write one deterministic `DOCSRC` serialization (`.dx`) instead of ambiguous Markdown.
Documents are exposed to agents through an **MCP server** (`src/mcp-server.ts`, stdio).

## Handoff & context discipline (this migration is long — keep context lean)

After completing each task/wave, **before ending the turn**, update `HANDOFF.md` with: (1) what
was just done **and validated** (with the proof), (2) the single concrete **next step**. Keep it
short — it is the resume point after a `/compact` or `/clear`. Then it is safe to compact.

What's automated vs not: `autoCompactEnabled` (in `.claude/settings.local.json`) compacts when the
window fills — but no hook can compact *per task*, and no hook can author the next step (only the
model knows it). So: the model writes `HANDOFF.md`; the user runs `/compact` at task boundaries.

## ⚠️ Active migration: the engine is now Rust (`rust/`), not TypeScript

The deterministic core has been **rewritten in Rust** and is the canonical engine going
forward. The TypeScript in `src/` is now a **reference/fallback**, not the source of truth.

- `rust/doc-core` — zero-dependency, `unsafe`-free, `wasm32`-safe Rust library: `digest`
  (sha256/sha1), `compress` (the `dxz1` LZSS codec), `docbin` (DOCB1), `format` (DOCSRC
  parse/stringify), `bundle` (DXBUN5/dxz1 archive), `search` (dxlite index). Every output is
  **byte-identical to the old TS** (interop-vector tests prove it; do not let them regress).
- `rust/doc-native` — the **canonical MCP server**: a self-contained native binary
  (`cargo build --release -p doc-native` → `rust/target/release/doc-native`) speaking MCP
  over stdio with **no Node** (`FsDocStore` = `std::fs` + git + `doc-core`). This supersedes
  `src/mcp-server.ts` and `src/cli.ts` (still present as reference; not yet deleted).
- `rust/doc-wasm` — `doc-core` compiled to WebAssembly (wasm-bindgen). The webview loads it
  (`vscode-extension/media/wasm/`, gitignored) and parses through it via
  `media/doc-core-wasm.ts`, **falling back to `doc-pipeline.ts` if wasm init fails**. This is
  the path that finally collapses the dual parser — but the **live editor wasm path is
  unverified** (needs the manual F5 pass in `docs/wasm-webview-manual-test.md`).

Quality bar for `rust/`: `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --check` must all stay clean; document every public item; no `unsafe`; no
panics/`unwrap` in library code. Toolchain is rustup (`~/.cargo/bin`); the wasm build needs
the `wasm32-unknown-unknown` target + `wasm-pack`.

**Migration is NOT complete:** the webview UI, the build tooling (esbuild/Node), and the
4 viewer/capture MCP tools + full themed HTML render remain TS/Node. Do not delete `src/`
until the wasm webview path is live-validated and a human signs off the cutover.

## Storage model — bundle + dxlite, NOT SQLite

This project was migrated off SQLite. Despite leftover artifacts (`binding.gyp`, empty
`native/`, `build/Release/`), there is **no `src/database.ts`, no `data/doc-index.sqlite`,
and no native addon in the active runtime path** — `binding.gyp` is orphaned and the build
scripts skip the native step. If you see SQLite/native references anywhere, treat them as
stale. The real model:

- `.dx` files on disk are **stubs/pointers** (`@docstub 3` or tiny `~` form), not content.
- Canonical content = packed DOC-binary blocks inside **bundle archives**:
  - `.doc/.repo-docs.bin` — repo-tracked docs (commit this; lets a fresh clone rebuild).
  - `.doc/.local-docs.bin` — local-only docs (gitignored).
- **dxlite sidecars** (`*.dxlite.bin`) provide the token→doc search index (zero-dependency,
  custom). There is no SQL.
- View state (theme, appearance, edit buffer) persists in `.doc/view-state.json`.

Git tracking state drives which archive a doc belongs to — see `src/git-doc-state.ts` and
`src/file-discovery.ts` (the latter walks for `.dx` files, skipping `data/`, `node_modules`,
etc.). When editing storage code, reason about tracked/untracked/ignored/modified flags.

## Architecture

Data flows: `.dx` source ⇄ `doc-format` (parse/normalize/stringify) ⇄ `doc-binary` (pack) ⇄
`doc-archive` (bundle) + `dxlite` (index). The MCP server and CLI sit on top of `doc-service`.

- `src/doc-format.ts` — DOCSRC parsing, legacy migration, block normalization, canonical
  stringify. The grammar authority for the **backend**.
- `vscode-extension/media/doc-pipeline.ts` — the **webview** parser/renderer. `src/doc-view.ts`
  imports `parseSourceBlocks` from here, so this file is shared between webview and server-side
  HTML rendering.
- `src/doc-binary.ts` — packs/unpacks normalized docs into compact binary blocks.
- `src/doc-archive.ts` — bundle read/write (brotli-compressed), magic versions `DXBUN2/3/4`,
  repo vs local archive selection.
- `src/dxlite.ts` — search index build/query over bundle contents.
- `src/doc-service.ts` — top-level orchestration: stub I/O, create/get/save/list/search/ingest/
  reconstruct. The main entry point most features touch.
- `src/mcp-server.ts` — stdio MCP tools/resources (list/get/search/create/save/ingest +
  viewer-session tools). Resources: `doc:///<path>` (source), `docview:///<path>` (rendered HTML).
- `src/doc-view.ts` / `src/doc-view-capture.ts` — render to HTML / capture as PNG via Playwright.
- `src/cli.ts` — `setup`, `ingest`, `maintain`, `reconstruct` subcommands.
- `vscode-extension/extension.ts` — `docdb:/` virtual filesystem + `.dx` custom editor; many
  `media/webview-*.ts` controllers (FSM, autocomplete, block rendering, edit controllers).

## CRITICAL: dual-parser parity

There are **two parsers** — backend (`src/doc-format.ts`) and webview
(`vscode-extension/media/doc-pipeline.ts`). They must agree on every malformed/edge form, or
documents drift between editor and storage. Before changing either parser, read
`docs/dx-format-contract.md` (canonical behavior spec) and run through `docs/grill-me.md`
(the change-review checklist). Non-negotiables from those docs:

- A valid block must never round-trip into a paragraph containing literal `::heading … ::end`.
- No single-line inline blocks and no synthetic `paragraph-N` wrappers on save.
- Never strip `id`/`class`, lose list-item boundaries, or reorder blocks in a save round-trip.
- In-document CSS must not affect rendering without selector-targeted activation; `effectiveCss`
  must be empty when no scoped CSS surface is active (`::style` / `::stylesheet` blocks are
  presentation-only and excluded from search/section text).

## Build / test / run

Node **23+**, ESM (`"type": "module"`), TypeScript. Generated output goes under `build/`;
never commit it. Path aliases `#runtime-src/*` and `#runtime-media/*` resolve into `build/runtime`.

```bash
npm run build:ts        # tsc (noCheck) → build/runtime, then runtime-metadata + esbuild
                        # extension-host + esbuild webview bundle (3 scripts/build-*.mjs steps)
npm run typecheck       # strict tsc --noEmit (stabilized surface) — the real type gate
npm run typecheck:full  # type-check ALL files (migration diagnostics)
npm test                # build:ts → build:test → node --test build/test/test/**/*.test.js
npm run test:coverage   # c8 --100 gate on a fixed include-list of runtime modules
npm run mcp             # start MCP server from prebuilt runtime (does NOT rebuild)
npm run mcp:dev         # node --watch
```

- `npm run mcp` intentionally skips rebuilds for fast start — run `npm run build:ts` (or
  `npm run mcp:prepare`) first if you changed source.
- Run **one test file**: `npm run build:ts && npm run build:test && node --test build/test/test/unit/doc-format.test.js` (point at the compiled `build/test/...` path, not the `.ts` source).
- CLI: `npm run ingest` (reindex all `.dx` into bundles), `npm run setup`, `npm run reconstruct -- <path.dx>`, `npm run docs:seed` (rewrite baseline docs).
- `npm run verify:doc-fsm` exercises the document save FSM end-to-end.
- VS Code extension: open `vscode-extension/` and press `F5`; run `DOC DB: Mount Virtual Files` if it doesn't auto-mount.

Tests are layered: `test/unit/`, `test/integration/`, `test/coverage/` (the coverage tests
exist specifically to hold the `--100` c8 gate), plus `test/helpers/` and `test/types/`.

## Working notes

- Edits to runtime `.ts` require `npm run build:ts` before `npm run mcp` / extension reflect them.
- Webview modules in `media/` are bundled by esbuild (`scripts/build-webview-bundle.mjs`) and
  also imported server-side — keep them dependency-light and browser-safe.
- After touching a parser/renderer/save path, validate the round-trip (parse → stringify →
  parse) and re-check `docs/grill-me.md`; behavior here must stay deterministic.
