# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–55) lives in this file's own git history —
`git log -p HANDOFF.md` — and each part's summary is its commit message._

Last updated: 2026-08-09 (part 58 — a read-only medium still answers, and the catalog wears its own kinds)

## Part 58 (same day, after part 57)

- **The part-57 error channel met the sandbox, and both got fixed.** The dev.dx sandbox
  mounts the repository read-only; `Store::open` wants write access; part 56's `.ok()`
  had silently fallen to the packs, and part 57's error channel turned every sandboxed
  read into a hard failure (`dx ls` answered nothing, showcase refused to render).
  Neither was right: `Store::open_read_only` (plain read-only open probed with a real
  read, `immutable=1` URI fallback for WAL on media that refuse the `-shm` file) and
  `open_existing` falls back to it, so a read-only checkout reads and a genuinely broken
  store still errors. `documents_still_resolve_when_the_medium_is_read_only` pins it.
  The corpus gate had passed on `ok - 0 documents` — it now refuses fewer than 10.
- **block-reference.dx wears the kinds it catalogs**: table rows for `view` and
  `mermaid / graph`, and a real `::rule`, `::image` (figures/block-anatomy.png, exported
  by `dx png --block` from the page's own anatomy SVG), `::view` of site/index.html, and
  `::style` (small-caps on the table header) — all rendered and eyeballed. welcome.dx's
  fabricated readiness percentages are gone. Fixtures refreshed as the deliberate
  two-file change; round-trip + drift guard green.
- **DX.app has a real Edit menu**: Undo/Redo/Cut/Copy/Paste/Select All on the responder
  chain — a keystroke only reaches the WKWebView through a menu key equivalent, so the
  read-only-era menu (Copy, Select All) was a dead ⌘V. App rebuilt, ad-hoc signed,
  installed; doctor green.

## Current state

Part 57 worked `index.dx#now-worklist` top-down; five open lines closed, each with the
test that would have caught it:

- **The resolver reports every failure it meets.** `workspace.rs` had four sites that
  swallowed store errors (`Store::open(...).ok()`, the `load_all` if-lets, `search`'s
  `document().ok()?`) and `search` had no error channel. Now: `open_existing` returns
  `Result<Option<Store>>` (a store that exists but will not open is an error, never "no
  store" — falling to packs would answer stale), `load_all` returns a `Listing` whose
  `unresolved` names every document that exists but would not resolve (with the error
  saying what to run), and `search` returns `Result<Vec<Hit>>`. `dx ls` prints
  unresolved lines, `dx_list` carries an `unresolved` array, `dx render --all` refuses
  to export around a hole, MCP `resources/list` errors instead of listing half.
- **A subdirectory query answers from the workspace store.** `load_all`/`search` used to
  open the store at the *argument* directory, so `dx_list {directory: "docs"}` was
  tree-only. Both now find the store at `workspace_root(directory)` and scope-filter its
  rows (`scope_of`/`in_scope` — not `relative_of`, which appends `.dx` and would turn
  the scope `docs` into `docs.dx`). A scoped search over-fetches then filters, so
  `limit` still means "best N here".
- **One title derivation.** `Document::display_title(relative)` in doc-core is the rule
  (metadata title → first heading → file stem); the store's summaries, `Loaded::title`,
  and the page `<title>` (`pick_title`, which keeps its override and "Document" last
  resort) all delegate. The three divergent copies are gone.
- **The store refuses to create ghosts.** `Store::save` into a directory `walk` never
  enters (`build/`, `fixtures/`, dotted names — `unwalked_directory`) is refused with
  `StoreError::Unlistable`, whose sentence names the rule and the way out. Files are
  exempt: discovery matches `*.dx` by extension.
- **`dx rm` is the deliberate delete.** `dx remove <file> [block-id]` — no block named
  means the document (unix `rm` semantics, one command, the argument names what goes).
  `workspace::remove` forgets it in the store, rewrites the packs so the deletion
  sticks, and handles the fresh-clone case (stubs + packs, no index) by building the
  store first so the next sync cannot resurrect it. Version history survives, exactly
  as a sync prune leaves it; the report says so.

Proof this pass: doc-cli 316/316 (7 new tests), doc-store 52/52 (+1), doc-core lib
+display_title test; live smoke on a scratch pad (scoped ls, rm sticks through sync,
unlistable refusal through `dx new`); full host-shell suite + wasm rebuild + both JS
suites re-run — see the commit for the final numbers.

## The live worklist

`index.dx#now-worklist` — still open, ranked: the edit.js board-geometry parity pin
(NODE_MIN_HEIGHT 60 vs FIT_MIN_HEIGHT 56 drifted), the edit.js kind/attr vocabulary
drift (export the doc-core registries through doc-wasm), vscode `exportHtml` without
resources, DX.app's missing Edit menu, the duplicated prefetch walk, and the corpus
gaps (`block-reference.dx` missing `::view`; `::image`/`::rule`/`::stylesheet` unused;
welcome.dx unsourced percentages).

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

Next step: work `index.dx#now-worklist` top-down — the edit.js board-geometry parity pin
first (NODE_MIN_HEIGHT 60 vs FIT_MIN_HEIGHT 56 has already drifted), then the kind/attr
vocabulary export through doc-wasm; the two share the "engine registries reach the
surface" shape, so design them together.
