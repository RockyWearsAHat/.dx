# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–55) lives in this file's own git history —
`git log -p HANDOFF.md` — and each part's summary is its commit message._

Last updated: 2026-08-09 (full-stack rating pass)

## Rating pass (2026-08-09, later same day)

Independent full verification, all mechanical: `dx run dev.dx` five gates current
(engine 380+51, lints clean, surfaces 34/34 + parity, corpus 11/11 canonical, page
contract holds); host-shell `cargo test` exit 0 across the workspace (attacks +
doc-shot included); both economics documents' 7 claims re-proven live (ten-question
session 43% of whole-reads; lean design round 18% of naive); index map board captured
clean at natural size. `dx doctor` flagged one drift — /Applications/DX.app bundled an
older `dx` — fixed by `packaging/build-app.sh` + the bundle's own `dx setup`; doctor
now fully green. No code changed. Next step unchanged (below).

## Current state

Everything green, proven mechanically this pass:

- **The ghost documents are gone, and the mechanism with them.** `Store::sync` grew the
  reverse pass: an index row whose `.dx` file was deleted is pruned and the packs are
  rewritten so the deletion sticks (the pack-materialization loop skips what was just
  pruned instead of resurrecting it). Run on this repository it pruned exactly the six
  part-40 ghosts; `dx stats` reads 208/208 blocks and the committed pack fell
  33.7 → 29.0 KB. Version history is deliberately untouched: manifests stay so
  `source_of_version` keeps answering after a prune, exactly as after `Store::delete` —
  a first GC draft deleted them, failed the old-commit-diff test, and was reverted (that
  one run collected this machine's pre-prune history chunks; old revisions restore via
  `git checkout` + `dx sync`). `dx stats` also stopped rounding the surplus away — more
  distinct chunks than current references now prints `blocks in history N`, never a
  saturated `shared 0`.
- **The handshake teaches lean delegation.** `SUBAGENTS RUN LEAN` joined the MCP
  always-on instructions: every subagent and workflow agent takes the least-powered model
  that carries its task and never inherits the operator's tier; a stronger model is the
  justified exception. Pinned by the same instruction tests as the rest of the doctrine.
- **The execution-paths doctrine says one thing everywhere**: three paths (`dx run`/
  `dx_run`; a local edit to a runnable block, the edit being the review; the agent read
  tools refreshing approved-stale output) — now stated identically by `dx help`, the
  format contract, and CLAUDE.md. `board_place` refuses kinds that render empty on a
  canvas for *new* nodes (the documented validator is finally wired; moves stay free).
- **Six dead wasm exports removed** (`split_chunks`, `join_chunks`, `render_section`,
  `compress`, `sha1_hex`, `build_index_and_search`): no consumer in any hand-written JS;
  the engine fell 471 → 428 KB and both JS suites hold 34/34 against the rebuilt wasm.
  The unused `wasm/*.d.ts` are untracked and ignored.
- **Docs match reality again**: `rust/ANALYSIS.md` re-measured against the living modules
  (chunk/pack rows instead of the deleted docbin/bundle, DXZ2 instead of LZSS); contract
  pack ratio re-measured (12 docs, 35.7%); CLAUDE.md wasm/binary numbers refreshed
  (~420 KB, ~21%); README's doc table now lists `rust/README.md` and `rust/ANALYSIS.md`
  so they cannot rot unnoticed; packaging/README describes DX.app as the editor it is;
  the VS Code extension no longer claims `.dx` has HTML comments, and `.vscodeignore`
  stops shipping tests and type declarations.

Proof: cargo test green across the workspace, clippy `-D warnings` clean, fmt clean,
github + vscode suites 34/34 each, `dx sync .` settles to "nothing to do", `dx run dev.dx`
gates green. `dx setup` installed the build.

## The live worklist

`index.dx#now-worklist` — the three survey reports (docs truth, engine health,
editor/packaging) were folded into it as ranked open lines: error-swallowing in
`workspace.rs`, the `dx_list` subdirectory inconsistency, the 4× title-derivation copy,
the `SKIPPED_DIRECTORIES` write trap, a deliberate `dx rm`, the edit.js board-geometry
and vocabulary drift (no parity pin), vscode `exportHtml` without resources, DX.app's
missing Edit menu, the duplicated prefetch walk, and the corpus gaps
(`block-reference.dx` missing `::view`; `::image`/`::rule`/`::stylesheet` unused).

## Open beyond this wave

- **Store distribution** (money, not code): submit `packaging/build/dx-firefox.xpi` to
  addons.mozilla.org as unlisted → `packaging/signed/`, upload `dx-chrome.zip` to the
  Chrome Web Store and set `CHROME_WEB_STORE` in `extension.rs`, Apple Developer for
  Safari distribution.
- **One claim still resting on reasoning:** a `.dx` on github.com in a signed-in browser
  on a **private** repository (the same-origin raw route should carry the reader's
  session).
- **Driving DX.app from automation:** post real `CGEvent`s — AppleScript's System Events
  click never reaches the DOM and makes a broken editor look like a working one.

Next step: work `index.dx#now-worklist` top-down — the `workspace.rs` error-swallowing
line first (it violates the resolver's own contract).
