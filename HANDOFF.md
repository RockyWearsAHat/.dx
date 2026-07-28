# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md)._

Last updated: 2026-07-28

## Where we are

The `dx` platform is built and working end to end. `README.md` describes it for users;
`CLAUDE.md` describes the codebase. This file only records what was verified and what is next.

### Done and verified by running it

- **Format** — `::code` gained `run` / `deps` / `timeout` / `format`; new `::output` block
  carries `for` / `status` / `exit` / `hash` / `format`. Additive: the byte-identical
  round-trip fixtures for every real document still pass untouched.
- **Renderer** (`doc-core::render`) — themed self-contained HTML, Markdown, outlines, and
  section slicing. Light/dark, no external assets, no scripts.
- **Execution** (`doc-run`) — python / node / deno / bash / rust / go / ruby, with real
  dependency installation, per-block sandboxes, timeouts, and fingerprint caching.
  Verified: `numpy` installed and ran from a clean cache; the second run skipped in 2 ms.
- **Screenshots** (`doc-shot`) — two-pass Chrome capture (measure height, then shoot).
  Verified: exact-height PNGs, whole-document and `--section`.
- **CLI + MCP** (`doc-cli`) — `dx` with 17 verbs; `dx mcp` serves nine tools. Verified over
  real stdio: handshake, `tools/list`, `dx_outline`, `dx_view` (returned a 23 KB PNG),
  `resources/list`.
- **Editor** (`editor/vscode`) — packaged `.vsix` (127 KB), installed into VS Code, opened
  `examples/showcase.dx`, rendered correctly. Its wasm render is **byte-identical** to
  `dx render` (asserted directly).
- **Cross-platform** — `cargo build --target x86_64-pc-windows-gnu -p doc-cli` produces a
  PE32+ `dx.exe`. The extension is wasm + TypeScript with no native dependency.
- **Install** — `dx install` registered the MCP server with claude, codex, cursor, vscode,
  and gemini, merging into each config without disturbing existing servers.
- **Gates** — 238 Rust tests green across 5 consecutive runs; clippy `-D warnings` clean;
  `cargo fmt --check` clean.

### Bugs found and fixed along the way (all have regression tests)

- The inline renderer panicked on any non-ASCII character (byte-index slicing).
- `--out` overwrote the PNG that `dx png` had just written.
- Capture height was viewport-clamped, padding short documents with blank space.
- Emphasis marks were being applied inside code spans.

## Changes a human should know about

- **`local.docdb-virtual-files` was uninstalled from VS Code.** It also claimed `*.dx` as a
  default editor, so VS Code prompted on every open. Its source in `vscode-extension/` is
  untouched; reinstall it from there if you want it back.
- **`rust/doc-native` was removed** (in the commit after `12d829b`). It was a second MCP
  server that wrote `.dx` files as opaque stub pointers — the exact thing that stops other
  agents from reading them. Its protocol layer lives on in `doc-cli/src/mcp/`.

## NEXT STEP

Decide the fate of the TypeScript reference (`src/`, `vscode-extension/`, the npm build).
Nothing ships from it now; `doc-core`'s fixtures are the only thing still pointing at it.
Removing it would drop several thousand lines and the whole Node toolchain from the repo —
but that is a call for a human, not a cleanup to do unasked.

## Known gaps

- Windows and Linux are verified by construction (a cross-compiled binary, a wasm
  extension), not by running on those machines.
- `doc-shot` needs a Chromium-family browser installed. Without one, `dx png` explains what
  to install and `dx_view` falls back to Markdown.
- `.doc/*.bin` bundles and `dxlite` sidecars still exist from the old storage model. Nothing
  in the `dx` path reads or writes them.
