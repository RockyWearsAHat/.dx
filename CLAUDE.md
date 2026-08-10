# CLAUDE.md

Rules for working in this repository. Each rule states what must hold; the reasoning for it
lives in the module or contract named beside it, already written out at length. Read that
when a change touches the area — not before. Nothing is described twice on purpose.

**Orient through the documents, not the tree.** `index.dx` is the map and the worklist
(`#now-worklist` — open lines only; a closed line is deleted, because a part's record is its
commit message). `dev.dx` is the harness. `README.md` is the user-facing description; keep it
true. `docs/dx-format-contract.md` is the authority on format behavior.

## What this is

`dx` is a document format, a store, and a toolchain. A `.dx` document is block structured and
canonical, renders to a page, and can execute the code blocks inside it.

**A `.dx` file on disk is a one-line pointer, not the content** — `~ dx1 <64-hex digest>`. The
content lives in the workspace store as content-addressed compressed chunks.

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
| `.doc/repo.dxcp` | The repository's documents, chunk-deduplicated and compressed | **yes** — this is the content |
| `.doc/local.dxcp` | Git-ignored scratch documents | no |
| `.doc/index.db` | SQLite: the queryable local authority, rebuildable from the packs | no |

`dx sync` reconciles all of it: it adopts plain-text `.dx` files anything else wrote, restores
documents from the packs when the index is missing, and reports what it cannot resolve rather
than discarding it.

**This repository is a dx workspace.** `index.dx`, `dev.dx`, and `examples/*.dx` are pointers.
Edit them through `dx` (`dx set`, `dx_edit`, or `dx sync` after a plain-text write), never by
writing pointer files by hand.

**The fixtures are hermetic.** `doc-core` compiles the round-trip corpus in with `include_str!`,
so it must stay plain text: the store's walker never adopts a `fixture`/`fixtures` directory
(`doc-store::store::SKIPPED_DIRECTORIES`). The drift guard lives in `doc-cli`
(`workspace::tests::every_fixture_input_still_matches_the_document_it_mirrors`) — refreshing an
example is a deliberate two-file change.

## Where the code lives

The engine is Rust, in `rust/`, and it is the only thing on a user's or agent's path. Each
crate's module docs are the authority for its area.

| Crate | Responsibility |
|-------|----------------|
| `doc-core` | The format and its views: `format` (parse/stringify, `mermaid`/`layout`), `render` (HTML, Markdown, outline, sections, `nav`, `board`, `block`, `field_html`), `edit` (every block operation an editing surface performs), `resolve` (references past the document's edge, and `hydrate`), `chunk`, `digest`, `compress`, `search`. No OS dependencies — also compiles to `wasm32`. |
| `doc-store` | The SQLite chunk store and the resolver: manifests, packs, git routing, the stub format. The authority for content. |
| `doc-run` | Executing `::code … run` blocks: per-language plans, dependency install, timeouts, output capture, `confine` (the kernel sandbox), `approvals` (the review ledger). |
| `doc-shot` | Rendering to PNG through an installed Chromium: `capture_pages`, `capture_block`, `cdp` (a live DevTools session), `play` (scripted input over the same script-free render). The capture browser resolves no hostnames — a capture is a read. |
| `doc-cli` | The `dx` binary: CLI, the MCP server (`dx mcp`), the rendering service (`dx serve`, in `daemon/`), and the installer (`install`, `service`, `desktop`, `policies`, `extension`, `state`, `home`). |
| `doc-wasm` | `doc-core` for JavaScript hosts. Built into `editor/{github,vscode}/wasm/`. |

**The repository is Rust, and only Rust.** The original TypeScript implementation was deleted
on 2026-07-30. The only JavaScript left is what a browser has to run — `editor/github/*.js` and
`editor/vscode/` — and both call `doc-core` through `doc-wasm` rather than reimplementing it.

### The surfaces

**`editor/surface` is the editor, and there is one of it.** `edit.js`/`edit.css` are the whole
of what happens when a reader clicks a block, and `edit.js`'s own module doc is the contract:
the field *is* the block (no second rendition anywhere on the sheet), saving replaces content
and never the page, and the file parses nothing — every operation is asked of the host, which
answers through `doc_core::edit`. `editor/build.sh` copies it into `editor/vscode` and
`packaging/build-app.sh` into `DX.app`; neither host owns a line of it.

What each host *does* own is how the calls get out: `Editor.swift` runs the bundled `dx`; the
VS Code webview goes through `doc-wasm` and a `WorkspaceEdit`, so undo and source control keep
working. Both land on `doc_core::edit` — the same thing `dx source`, `dx set` (`--header` to
retype, `--replace/--with` for a change-sized edit), `dx insert`, `dx remove`, `dx check`,
`dx render --block`, and `dx render --field` are.

- **Never re-implement a block operation in a host.** "What is the editable text of a
  checklist?" has one answer; a second one in an editor is a document that changes shape
  depending on which surface last touched it. Likewise `render::block` is the page's own
  per-block renderer, so a block drawn alone and the same block drawn in its page are
  byte-identical.
- **A board node's box is stated, never measured.** `x y w h` are the whole of it; a dimension
  may state a *rule* (`page`/`fit`) and keeps the word forever. The geometry — sides, spread,
  curve, clearance, overlap — is stated once in `render::board` and mirrored in `edit.js`
  against the measured boxes, which are the stated boxes. `render::board`'s module doc is the
  authority.

**`editor/github`** makes github.com show documents instead of pointers, and gets no editing
surface. `resolve.js` decides what the reader sees and is tested under node against a pack the
real `dx` wrote; `content.js` is the DOM adapter and does nothing on a page it does not
recognize. See `docs/github.md`. Two rules that silently render nothing if broken:

- **The engine is never in the page.** github.com's `script-src` grants no
  `'wasm-unsafe-eval'`, so `engine.js` runs in the extension's background context and answers
  from `dx serve` when it is running and bundled wasm when it is not. The content script keeps
  the *fetching*, because only a page-context request carries the reader's session — which is
  what makes private repositories work with no token. Do not move the engine back into the
  page, and do not drop `content_security_policy.extension_pages` from the manifest.
- **A pack is named, never inlined.** A call carries `{ packRef: url }`; the engine answers
  `needPack` if it does not hold it, and the bytes cross once per repository revision. Inlining
  them made a six-document pull request transfer the whole repository twelve times.

**`packaging/`** builds what a person installs, and **none of it is in the shipped binary** — an
archive writer and a rasterizer are release tools. `packaging/README.md` is the runbook and the
record of what each browser actually permits.

- **`DX.app` is the Mac viewer, and it is one application.** AppKit and a `WKWebView`; it
  renders nothing itself, it runs the `dx render` in **its own bundle**. A launch carrying a
  document is a read; a launch carrying nothing is `dx setup`. Never let it reach for a `dx` on
  `PATH`, and never split it in two.
- **One install, per device — never one per program.** `dx setup` is the only install command:
  `PATH`, MCP, the login service, `DX.app` and the LaunchServices type, and each browser by its
  own route (`extension::Channel`). Adding a surface means adding a channel, not a command.
- **The extension is not in the binary.** Its files are read from a directory, never
  `include_bytes!` — a `dx` in a container has no browser and no use for ~0.4 MB of one.
  `editor/github` is the one source, `dx browser --from <dir>` the one builder, and
  `extension::installed_dir` finds what this machine has (the application's own copy first).
  A machine with neither gets `Channel::Absent` and a sentence saying where to get one; never
  name a directory that was not installed.

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
- **A mermaid block is a board.** `::mermaid`/`::graph` is not a kind this format keeps:
  `format::mermaid` reads the flowchart at parse time and `format::layout` arranges it into a
  `::board` plus one hidden block per node. This is the one conversion `parse` performs — two
  diagram renderers that have to agree is the thing this repository exists to avoid. Source the
  converter cannot read is **left exactly as written**, never replaced with an empty canvas.
- **A document's CSS always applies.** `::style` reaches every surface with no flag to forget,
  and it travels *inside* `.dx-doc` rather than the page `<head>` — that container is what a
  host swaps on save. It may dress and may not fetch: `render::escape::escape_style` blanks any
  `url(…)` naming a host and neutralizes `@import` and `expression(`.
- **Reading never executes, and never writes.** Parsing, rendering, screenshotting, and
  resolving a pointer stay free of side effects — resolving must not even create the index.
  `edit::preview_block` applies a body to a parsed copy and throws it away. `dx run --review`
  is a read too: it reports what would run, folds no `::output`, writes no file. Only `dx run`
  and `dx_run` execute, with two stated exceptions: an edit to a runnable block through a local
  editing surface runs it at once (the edit is the review), and the agent read tools
  (`dx_read`, `dx_source`) refresh stale output of code *this machine already approved*, so an
  agent never reads dead output. Unreviewed code stays blocked on a read; `refresh: false`
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
- **An agent reads by the cheapest route that carries the meaning.** `dx_source` (with
  `section`) is the default read and `dx_outline` is the map that keeps it to one section.
  `dx_read`'s images are spent only on what text cannot carry — boards, diagrams, charts,
  rendered views. A project with no documents gets `dx index`, read whole and improved by its
  first reader, so orientation costs one read forever after. Pages break between blocks, never
  through a line, and never leave a heading without its text; a `::board` is always its own page
  captured at natural canvas size. Reading captures are sized for the model
  (`ShotOptions::for_reading`: ~1.15 MP, 1568 px longest edge) and supersampled
  (`ShotOptions::oversample`, averaged down in linear light) so a hairline survives the budget.
  Human exports go the other way — `dx png` captures at device scale 2. `dx_play` is the same
  read with hands: synthetic input over the script-free render, nothing the document says runs.
- **One engine.** If the editor and the CLI could render differently, they will. Render through
  `doc-core`; never re-implement a view. Rebuild the wasm after touching `doc-core` or the
  editor silently keeps the old renderer.
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
```

The two JavaScript surfaces have their own suites, and neither needs a browser:

```bash
./editor/github/test/fixture.sh             # a real pack, written by a real `dx sync`
node --test "editor/github/test/*.test.mjs" # the extension, and engine parity with dx
node --test "editor/vscode/test/*.test.mjs" # the editing calls, across the wasm boundary
```

No `unsafe`. Every public item documented. No panics in library code — fallible operations
return `Result`, and user-facing failures return a sentence saying what to do about it.

Tests are colocated with the code they test and named for the behavior they pin down. When you
fix a bug, add the test that would have caught it. Non-ASCII prose, empty input, and missing
toolchains are all real cases that have broken this code before.

## Building and trying it

```bash
cd rust && cargo build --release -p doc-cli   # → rust/target/release/dx
./target/release/dx doctor                    # what is installed and what is missing
./target/release/dx run ../examples/showcase.dx
```

The repository root is a live dx workspace — `dx sync .` here is normal and is how a plain-text
edit to an example is adopted back into `.doc/repo.dxcp`. To try the store from nothing:

```bash
mkdir /tmp/pad && cd /tmp/pad && git init -q .
dx textconv ~/Desktop/DOC/examples/welcome.dx > welcome.dx
dx sync .          # adopt it: the file becomes a pointer, content goes to .doc/
dx git-setup .     # make git diff, git show, and git log -p render documents
dx text welcome.dx # the document
dx stats .         # sharing and compaction
```

**One command builds every engine.** `doc-wasm` is compiled twice — `no-modules` for the browser
extension (a manifest content script cannot be an ES module) and `nodejs` for the editor — and
`editor/build.sh` does both together, because a surface built at a different moment is a surface
running a different renderer. It needs the rustup toolchain ahead of any Homebrew `rustc` and
the `wasm32-unknown-unknown` target.

```bash
./editor/build.sh                                    # → editor/{github,vscode}/wasm
cd editor/vscode && npm install && npm run package
code --install-extension dx-documents-1.0.0.vsix
```

## Working discipline

The worklist has one home: `index.dx#now-worklist`. Record what a task did in its commit
message and close the worklist line; do not open a second file that restates either.
