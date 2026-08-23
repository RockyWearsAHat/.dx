# CLAUDE.md

Rules for working in this repository. Each rule states what must hold; the reasoning lives in
the module or document named beside it, already written out at length. Read that when a change
touches the area — not before. Nothing is described twice on purpose.

**Orient through the documents, not the tree, and not this file.** `index.dx` is the map, the
worklist (`#now-worklist` — open lines only; a closed line is deleted, because a part's record
is its commit message), and the engine's contracts in the engine's own words. `docs/method.dx`
is how to work here. `dev.dx` is the harness. `docs/dx-format-contract.dx` is the authority on
format behavior. `dx search` reaches every document *and* every source file, so a question is
one search away — use it before reading anything whole.

## What this is

`dx` is a document format, a store, and a toolchain. A `.dx` document is block structured and
canonical, renders to a page, and can execute the code blocks inside it.

**A `.dx` file on disk is a one-line pointer, not the content** — `~ dx1 <64-hex digest>`. The
content lives in the workspace store as content-addressed chunks.

The rule everything rests on: **the resolver always hands back the true document.** Every read
— a person in the editor, `dx render`, an agent calling `dx_read`, git showing a diff — goes
through `doc-cli::workspace` or `doc_store::Store` and gets real content. Anything that lets a
caller see a pointer where a document belongs, or nothing where content exists, is a
regression. A missing document is an error that says what to run; it is never an empty one.

The digest is *in* the pointer so the file changes exactly when the content changes — git still
tracks each document, and the diff driver can expand it. A constant marker would make every
edit invisible to git.

| Path | What it is | Committed? |
|------|------------|-----------|
| `*.dx` | One-line pointer carrying the content digest | yes |
| `.doc/repo.dxcp` | The repository's documents, chunk-deduplicated, stored uncompressed so git can delta it | **yes** — this is the content |
| `.doc/local.dxcp` | Git-ignored scratch documents | no |
| `.doc/index.db` | SQLite: the queryable local authority, rebuildable from the packs | no |

`dx sync` reconciles all of it: it adopts plain-text `.dx` files anything else wrote, restores
documents from the packs when the index is missing, and reports what it cannot resolve rather
than discarding it.

**This repository is a dx workspace, and its own documentation is documents.** Edit them through
`dx` (`dx set`, `dx_edit`, or `dx sync` after a plain-text write), never by writing pointer
files by hand.

**The fixtures are hermetic.** `doc-core` compiles the round-trip corpus in with `include_str!`,
so it must stay plain text: the store's walker never adopts a `fixture`/`fixtures` directory
(`doc-store::store::SKIPPED_DIRECTORIES`). The drift guard lives in `doc-cli`
(`workspace::tests::every_fixture_input_still_matches_the_document_it_mirrors`) — refreshing an
example is a deliberate two-file change.

**The repository is Rust, and only Rust.** The only JavaScript left is what a browser has to
run — `editor/surface/edit.js`, `editor/github/*.js`, and `editor/vscode/` — and all of it
calls `doc-core` through `doc-wasm` rather than reimplementing anything. `index.dx` maps the
crates and the surfaces; `rust/engine.dx` is the crate-by-crate responsibility list.

## Non-negotiables

- **Storage cannot lose a byte.** A chunk holds the exact canonical text `stringify` writes for
  one block, so reassembly is concatenation. Any encoding that understands only *some* block
  attributes has no place in the store.
- **Canonical output must not drift.** `doc-core/src/format` round-trips real documents
  byte-for-byte against captured fixtures. Those fixtures are captured output, not scripture —
  four of them once encoded genuine data loss. Fixing a defect and regenerating a fixture is
  correct; changing one to make a convenient change pass is not. Review the regenerated diff
  and say what changed and why.
- **Format changes are additive.** A new attribute must serialize to nothing when unset, or
  every existing document reformats on next save.
- **A document must be able to carry dx syntax.** A body line shaped like a close token is
  content when a backslash precedes it, and the canonical writer puts that backslash there
  (`format::parse::close_token`, `format::stringify::escape_close_tokens`). Without it the
  format could not document itself — every example truncated the document that showed it. The
  ladder is one backslash per level and must stay reversible in both directions.
- **A mermaid block is a board.** `::mermaid`/`::graph` is not a kind this format keeps:
  `format::mermaid` reads the flowchart at parse time and `format::layout` arranges it into a
  `::board` plus one hidden block per node. This is the one conversion `parse` performs — two
  diagram renderers that have to agree is the thing this repository exists to avoid. Source the
  converter cannot read is **left exactly as written**, never replaced with an empty canvas.
- **A document's CSS always applies.** `::style` reaches every surface with no flag to forget,
  and it travels *inside* `.dx-doc` rather than the page `<head>` — that container is what a
  host swaps on save. It may dress and may not fetch: `render::escape::escape_style` blanks any
  `url(…)` naming a host and neutralizes `@import` and `expression(`.
- **One engine, and never a second answer in a host.** Render through `doc-core`; never
  re-implement a view or a block operation in an editor. "What is the editable text of a
  checklist?" has one answer; a second one in JavaScript is a document that changes shape
  depending on which surface last touched it. `render::block` is the page's own per-block
  renderer, so a block drawn alone and the same block drawn in its page are byte-identical.
  Rebuild the wasm after touching `doc-core` (`./editor/build.sh`, both targets together) or a
  surface silently keeps the old renderer.
- **A board node's box is stated, never measured.** `x y w h` are the whole of it; a dimension
  may state a *rule* (`page`/`fit`) and keeps the word forever. The geometry — sides, spread,
  curve, clearance, overlap — is stated once in `render::board` and mirrored in `edit.js`
  against the measured boxes, which are the stated boxes.
- **The engine is never in the github.com page.** That page's `script-src` grants no
  `'wasm-unsafe-eval'`, so `engine.js` runs in the extension's background context. The content
  script keeps the *fetching*, because only a page-context request carries the reader's
  session — which is what makes private repositories work with no token. A pack is named, never
  inlined: a call carries `{ packRef: url }` and the bytes cross once per repository revision.
  `docs/github.dx` is the authority.
- **Reading never executes, and never writes.** Parsing, rendering, screenshotting, and
  resolving a pointer stay free of side effects — resolving must not even create the index.
  `edit::preview_block` applies a body to a parsed copy and throws it away. `dx run --review`
  is a read too: it reports what would run, folds no `::output`, writes no file. Only `dx run`
  and `dx_run` execute, with three stated exceptions: an edit to a runnable block through a
  local editing surface runs it at once (the edit is the review), the agent read tools
  (`dx_read`, `dx_source`) refresh stale output of code *this machine already approved*, so an
  agent never reads dead output, and `dx_search`/`dx search` appends one best-effort record to
  `.doc/coverage.jsonl` — document, source, or no hit, the ranking outcome the search already
  computed and used to discard. The third is not execution and touches none of what the other
  two guard: never a document, a pointer, or `.doc/index.db`. It never creates `.doc/` — a
  workspace search has not touched stays untouched — any I/O failure is silently swallowed
  rather than surfaced, and the log is self-pruning so it stays bounded. `doc-cli::coverage` is
  the authority. Unreviewed code stays blocked on a read; `refresh: false`
  reads exactly what is stored. This is why a `nav` block and a `::board` resolve from their own
  document and never by reading another file. The one sanctioned outward read is
  `resolve::hydrate` — a document's own stated references, confined by `resolve::confined`,
  filled into a copy that is never saved. `dx serve` reads no file even for this: a caller
  passes resources in.
- **Running a document does not give it your authority.** `doc-run::confine` puts every block
  inside a kernel sandbox — Seatbelt on macOS, bubblewrap on Linux. Its module doc is the
  authority; the shape is: **read** the repository the document lives in, the run caches, and
  the system toolchains, never the rest of the user's files and never the credential stores;
  **write** its own block directory plus the `writes=` folders inside the document's folder;
  **network** never, while the block's own code runs. Only dependency installation gets the
  network, which is why `plan.rs` arranges every language to fetch in `setup` and run offline —
  there must never be a `run` command that would download something. A machine that cannot
  impose the boundary **does not run the block**; `DX_UNCONFINED=1` is the only way past and
  says so in the output it produces. `doc-run/tests/attacks.rs` is the evidence — if you change
  this area, run it, and check it still *can* fail by running the same payload unconfined.
  `lang=capture` is the one deliberate exception, and does not weaken this: there is no
  interpreter and no subprocess of the block's own code, so `confine`'s boundary is not what
  is protecting anything there — `live.rs`'s module doc is the authority on what scopes it
  instead (a browser that resolves and proxies only the one `target=` host, hostname or IP
  literal alike), and `capture_network.rs` is its own attack evidence.
- **Unreviewed code does not run at all — but a local edit is the review.**
  `doc-run::approvals` (module doc is the authority) is a ledger of fingerprints *this machine*
  recorded, never inside a repository. The identity is the code and its powers — runner, deps,
  exact code, `reads=` paths *as declared*, the `writes=` grant — and deliberately not the
  `reads=` files' current text. Every local editing surface records the approval as it saves
  (`doc_run::approve_edited_block`); `dx sync` and adoption never approve. A document's own
  `::output` record cannot approve anything — `hash=` is computed from the code above it, so
  the hand that writes the block writes the record too. For the same reason the gate is checked
  **before** the unchanged-fingerprint skip. `--force` is the one way past and stamps
  `FORCED_NOTICE` into its output, so a bypass is never silent.
- **Untrusted input is everything a document carries.** Author markup goes through
  `render::escape`, which is an **allow-list** of elements, attributes, and URL schemes; its
  module doc lists the payloads that defeated the previous deny-list. Never make it a deny-list
  again. A shadow root is not a boundary. The one real boundary the renderer uses is a
  `::view`'s engine-built `<iframe sandbox="">` — the attribute stays empty forever, and author
  markup cannot write an iframe of its own.
- **The daemon holds private content, so it checks who is asking.** Every `dx serve` request
  must name a loopback `Host` (this is what stops DNS rebinding) and, if it carries an
  `Origin`, must be the extension. A cross-origin `POST` of `text/plain` is a *simple* request
  and arrives with no preflight, so this is refused in the handler, not in a CORS header.
- **Bound anything sized by untrusted input.** A frame's declared length, a `Content-Length`, a
  header line, the number of live connections. Each was once a way to allocate until death.
- **Compression is chosen, not fixed.** A `dxz` frame names its codec in its first four bytes;
  `compress` encodes each way and keeps the smallest, with "store as-is" always a candidate.
  `DXZ1` is no longer written and must be decoded **forever**. Adding a better codec means
  adding a magic, never converting anything.
- **One install, per device — never one per program.** `dx setup` is the only install command:
  `PATH`, MCP, the login service, `DX.app` and the LaunchServices type, and each browser by its
  own route (`extension::Channel`). Adding a surface means adding a channel, not a command.
  `DX.app` renders nothing itself — it runs the `dx` in **its own bundle**, never one on
  `PATH`. The extension is read from a directory, never `include_bytes!`. `packaging/packaging.dx`
  is the runbook.
- **An agent reads by the cheapest route that carries the meaning.** `dx_search` covers the
  documents *and* the source files, and a hit carries the answering excerpt with the block id
  or line range to read — so a search that lands is the read. It answers a question asked in
  the reader's words rather than only the project's, and the block it hands back is the one
  that *states* the fact, never a heading or a test over it (`index.dx#contract-search`).
  `dx_source` (with `section`) is the default read and `dx_outline` is the map that keeps it
  to one section. `dx_read`'s images
  are spent only on what text cannot carry — boards, diagrams, charts, rendered views. Pages
  break between blocks, never through a line, and never leave a heading without its text; a
  `::board` is always its own page captured at natural canvas size. Reading captures are sized
  for the model (`ShotOptions::for_reading`: ~1.15 MP, 1568 px longest edge) and supersampled
  (`ShotOptions::oversample`, averaged down in linear light) so a hairline survives the budget.
  Human exports go the other way — `dx png` captures at device scale 2. `dx_play` is the same
  read with hands.
- **The page is a blank sheet.** One paper tone, one ink: no panels, fills, tints, grids,
  rounded boxes, or shadows. Structure comes from type, whitespace, and hairline rules. Nothing
  is announced unless it earns the space — a successful run says nothing. The editing field
  obeys this too: no border, no background, no box, always the block's own type.
- **A document opens as what it says, not how it was made.** Code blocks render folded behind
  their own label (`HtmlOptions::collapse_code`); what the code *produced* stays on the page.
  The fold is `details`/`summary` and stays CSS-only — no page may need a script to open one. A
  render nobody can click (a PNG, a printed sheet, the images `dx_read` hands an agent) passes
  `collapse_code: false`.

## Quality bar

The fast loop is the repository's own harness: `dx run dev.dx` runs the engine suites, the
lints, both JS surface suites, corpus resolution, and the rendered-page contract inside the
sandbox, recording each verdict in the document. Its gates declare their input trees with
`reads=`, so an edit stales exactly the gates that read it; `--force` re-runs everything.

The full proof still needs the host shell — the attack payloads watch the kernel boundary from
outside it, and doc-shot drives a real Chromium. **Use the rustup toolchain**: this machine also
has a Homebrew `rustc` earlier on `PATH`, and two rustc versions writing one `target/` make
doctests fail with `E0514`.

```bash
cd rust
export PATH="$HOME/.cargo/bin:$PATH"        # the toolchain dev.dx's gates use
cargo test                                  # must be green, every crate
cargo clippy --all-targets -- -D warnings   # must be clean
cargo fmt --check                           # must be clean
./editor/github/test/fixture.sh             # a real pack, written by a real `dx sync`
node --test "editor/github/test/*.test.mjs" # the extension, and engine parity with dx
node --test "editor/vscode/test/*.test.mjs" # the editing calls, across the wasm boundary
```

No `unsafe`. Every public item documented. No panics in library code — fallible operations
return `Result`, and user-facing failures return a sentence saying what to do about it. This
forbids a panic reachable from user or document input; it does not forbid `expect()`/`unwrap()`
on a condition an adjacent check already made impossible (a length just verified, a variant just
matched) — that is documentation of an invariant, not a missing `Result`. Write the `expect()`
message as the invariant, so it reads as a proof, not an excuse, and keep the check and the
`expect()` beside each other so a later edit cannot separate them.

Tests are colocated with the code they test and named for the behavior they pin down. When you
fix a bug, add the test that would have caught it. Non-ASCII prose, empty input, and missing
toolchains are all real cases that have broken this code before.

## Working discipline

The worklist has one home: `index.dx#now-worklist`. Record what a task did in its commit
message and close the worklist line; do not open a second file that restates either.

Every request leaves the project's documents in the truest state, no matter how it found them —
that is the same work, or less work. `docs/method.dx` is the authority on how.
