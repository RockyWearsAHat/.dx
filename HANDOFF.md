# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–58) lives in this file's own git history —
`git log -p HANDOFF.md` — and each part's summary is its commit message._

Last updated: 2026-08-09 (part 59 — docs-and-cleanup: the worklist stopped carrying its own history)

## Part 59

- **Full validation, both loops.** `dx run dev.dx`: five gates green (engine 381+52,
  lints clean, both JS suites 34/34 with parity, corpus 11/11 canonical, page contract
  holds). Host shell: full `cargo test` green to the end — attacks and Chromium
  captures included — plus doctor and `dx sync` (nothing to do).
- **The worklist stopped carrying its own history.** `index.dx#now-worklist` held ~28
  closed `[x]` lines (~6 KB) that every orientation read paid for again; closed lines
  are now deleted, and `now-note` states the rule (a part's record is its commit
  message). Ten open lines remain.
- **HANDOFF trimmed to a resume point.** Its "live worklist" paragraph had drifted —
  it restated items part 58 closed. The worklist has one home: `index.dx#now-worklist`;
  this file only points at it.
- **DX.app rebuilt from the current binary** (doctor had flagged the bundle carrying an
  older `dx`); installed via the bundle's own `dx setup`, doctor green.

## The live worklist

`index.dx#now-worklist` — open lines only, ranked. Top of the list: the edit.js
board-geometry parity pin (NODE_MIN_HEIGHT 60 vs FIT_MIN_HEIGHT 56 drifted), then the
edit.js kind/attr vocabulary export through doc-wasm; the two share the "engine
registries reach the surface" shape, so design them together.

## Open beyond the worklist

- **Store distribution** (money, not code) — steps are on the worklist line.
- **One claim still resting on reasoning:** a `.dx` on github.com in a signed-in browser
  on a **private** repository (the same-origin raw route should carry the reader's
  session).
- **Driving DX.app from automation:** post real `CGEvent`s — AppleScript's System Events
  click never reaches the DOM and makes a broken editor look like a working one.

Next step: work `index.dx#now-worklist` top-down — the board-geometry parity pin first,
the vocabulary export with it.
