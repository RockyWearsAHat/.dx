# HANDOFF — DOC platform Rust migration

_Resume point after `/compact` or `/clear`. Update after every task/wave (see CLAUDE.md "Handoff & context discipline")._

Last updated: 2026-06-20

## Where we are

Migrating the engine off Node to a compiled Rust core (native + WASM). The TypeScript in
`src/` + `vscode-extension/` is now the **reference/fallback**, kept green until the wasm
editor path is live-validated. Full detail in memory `rust-wasm-migration.md` and `CLAUDE.md`.

### Done & validated (verified by running it myself, not agent claims)
- **`rust/doc-core`** (zero-dep, `unsafe`-free, wasm-safe): `digest`, `compress` (dxz1),
  `docbin` (DOCB1), `format` (DOCSRC), `bundle` (DXBUN5), `search` (dxlite). Byte-identical
  to the TS reference (proven non-circular). **67 Rust tests, clippy/fmt clean.**
- **`rust/doc-native`**: real `FsDocStore` (std::fs + git + doc-core) behind an MCP stdio
  server. **Proven Node-free**: `rust/target/release/doc-native` answers MCP create/get/search
  under `env -i`, no node linkage.
- **`rust/doc-wasm`**: doc-core → WASM (191 KB), **31/31 byte-identical to TS**.
- **webview**: wasm parser wired via `vscode-extension/media/doc-core-wasm.ts` with TS
  `doc-pipeline.ts` fallback; parser parity 9/9 docs; `build:ts` + `npm test` (142) green;
  CSP got `'wasm-unsafe-eval'`.
- Config: `.claude/settings.local.json` (autoCompact), this `HANDOFF.md`, CLAUDE.md updated.

## NEXT STEP (do this next)

**Manual F5 validation of the wasm webview** — the one thing that can't be automated here.
Follow `docs/wasm-webview-manual-test.md`: open `vscode-extension/` in VS Code → F5 → open a
`.dx` → confirm the wasm parser loads under the real CSP and edit/save/diff work live. Watch
the known behavior change: block source-editing now shows **canonical** serialization, not
verbatim bytes.

**After F5 passes:** decommission the dead Node MCP/CLI path (`src/mcp-server.ts`, `src/cli.ts`
are superseded by the Rust binary) and trim `src/` to only what the webview still needs.
**Do not delete `src/` before F5** — it's the byte-exact reference the wasm path is validated against.

## Known gaps / deferred
- 4 viewer/capture MCP tools + full themed HTML render (`doc-view.ts`) still stubbed in doc-native.
- `helpers grade` CLI is **Java-only** — it reports "0 source files" on Rust and scores F. Not a
  real grade of this code. The `helpers` MCP server is also down (`✘ Failed to connect`); the CLI
  works only via full path `/Users/alexwaldmann/bin/helpers`.
- TS `npm run typecheck` has 86 pre-existing baseline errors (untouched legacy files).
