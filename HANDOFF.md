# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md)._

Last updated: 2026-07-29

## Where we are

The store is back, and it is the authority. A `.dx` file on disk is a one-line pointer
(`~ dx1 <sha256>`); content lives in `.doc/` as content-addressed chunks. Every read resolves to
the true document. `README.md` describes it for users, `CLAUDE.md` for this codebase,
`docs/dx-format-contract.md` for the format and how it is stored.

### Done and verified by running it

- **Chunk storage** (`doc-core::chunk`) — one chunk per block, payload is the exact canonical
  text `stringify` writes for that block, addressed by SHA-256. Reassembly is concatenation, so
  it cannot drop a field. `DXCP1` packs a document set into one deduplicated, compressed
  container. Measured: the six example documents, 17,357 B of source → an 8,564 B pack (49.3%),
  losslessly.
- **The store** (`doc-store`) — SQLite. `chunks` + `manifests` + `manifest_chunks`, with
  `documents` naming a path and pointing at its current manifest. `sections` and `tokens` are
  derived read models. `Store::source` verifies what it reassembles against the recorded digest
  before returning it. Garbage is found by querying the join table, not by a refcount.
- **Versions are keyed by digest, not path.** Required twice over: git hands a `textconv` driver
  a *temporary copy of a blob*, so a path tells it nothing; and diffing an old revision needs
  that revision's content. A manifest per version costs a row per block.
- **The resolver** (`doc-cli::workspace`) — plain text on disk wins (something else wrote it, so
  it is newest), then the store, then the packs. An unresolvable pointer is an error naming what
  to run. Resolving creates nothing: no database is built by reading.
- **Git behaves normally.** Verified in a throwaway repo: `git status` sees an edited document,
  `git diff` and `git log -p` render readable document diffs across two commits, and a fresh
  clone with only pointers + `repo.dxcp` reads, lists, and searches with no `index.db`.
- **Rendering is a blank sheet.** One paper tone, one ink, no second surface — no panels, fills,
  tints, grids, rounded boxes, or shadows. Header bars and pill badges are gone; a code block
  names itself in a faint margin note. A successful run says nothing; only a failure is called
  out. Verified by rendering light and dark to PNG and looking at them.
- **One engine still holds.** wasm rebuilt; `dx render` and the editor's engine produce
  byte-identical HTML (12,942 B on `showcase.dx`), asserted directly.
- **Gates** — 319 Rust tests green, clippy `-D warnings` clean, `cargo fmt --check` clean.

### Bugs found and fixed along the way (all have regression tests)

All four were faithful ports of the unreleased TypeScript reference, and all four were silent
data loss in the code storage has to trust:

- The canonical writer had no `checklist` case, so it wrote an empty body: saving
  `examples/welcome.dx` **erased all four of its checklist items**.
- `build_nested_list_structure` reproduced a reference object-aliasing quirk that **dropped
  every indented list item**.
- `sha256_hex`/`sha1_hex` **panicked** for any input of length 56..=62 mod 64 — padding
  underflowed `usize`, then overflowed on the next add. This is the hash addressing every chunk.
- `parse` discarded `::paragraph id=paragraph-N` blocks as legacy "synthetic wrappers", but
  `paragraph-N` is exactly the id the *writer* generates for an unnamed paragraph, so
  **write-then-read destroyed those paragraphs**.

The lossy `DOCB1` codec and its `DXBUN5` archive were deleted rather than kept: `DOCB1` stored a
`code` block's language and text and nothing else, dropping `id`, `run`, `deps`, `timeout`,
`format`, and every `output` attribute.

## Changes a human should know about

- **Do not run `dx sync` at the repository root.** It would convert `examples/` and `documents/`
  into pointers. Those stay plain text so they are readable on GitHub, and `doc-core`'s
  round-trip tests now read hermetic copies in `rust/doc-core/tests/fixtures/*.input.dx`.
  `tests/fixtures.rs` fails if a copy drifts from the document it mirrors.
- **`.doc/.repo-docs.bin`, `.doc/.local-docs.bin`, `.doc/.preview/`, and `.doc/view-state.json`
  were removed.** They were artifacts of the retired TypeScript brotli archive; nothing can read
  that format any more. They remain in git history.
- **`doc-wasm` exports changed**: `split_chunks` / `join_chunks` replace `pack` / `unpack`. The
  editor never used the removed pair.
- **`rusqlite` (bundled SQLite) is a new dependency** of `doc-store`, and therefore of `dx`. It
  compiles SQLite from source, so the first build is slower.
- An `index.db` written by an older schema is dropped and rebuilt rather than migrated — it is
  derived data, and `dx sync` restores it from the packs.

## NEXT STEP

Convert a real workspace and live in it for a while: `dx sync` + `dx git-setup` in a scratch
project, then edit through the VS Code extension rather than the CLI. The editor's save path is
the one part of the loop that has only been exercised by its own tests — confirm that editing a
block writes through the store, leaves the pointer updated, and that the rendered page updates
without a reload.

Still open, and a human's call: the fate of the TypeScript reference (`src/`,
`vscode-extension/`, the npm build). It is no longer a compatibility target — its round-trip
quirks are the four bugs above — so nothing points at it now. Removing it would drop several
thousand lines and the whole Node toolchain from the repository.
