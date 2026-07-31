# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`dx` is a document format, a store, and a toolchain. A `.dx` document is block structured and
canonical, renders to a page, and can execute the code blocks inside it. `README.md` is the
user-facing description; read it first, and keep it true.

**A `.dx` file on disk is a one-line pointer, not the content.** The content lives in the
workspace store as content-addressed compressed chunks:

```
~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823
```

The rule everything rests on: **the resolver always hands back the true document.** Every read
— a person in the editor, `dx render`, an agent calling `dx_read`, git showing a diff — goes
through `doc-cli::workspace` or `doc_store::Store` and gets real content. Anything that lets a
caller see a pointer where a document belongs, or nothing where content exists, is a
regression. A missing document is an error that says what to run; it is never an empty one.

The digest is *in* the pointer on purpose. It makes the file change exactly when the content
changes, so git still tracks each document, and the diff driver can expand it. A constant
marker would make every edit invisible to git.

### What lives where

| Path | What it is | Committed? |
|------|------------|-----------|
| `*.dx` | One-line pointer carrying the content digest | yes |
| `.doc/repo.dxcp` | The repository's documents, chunk-deduplicated and compressed | **yes** — this is the content |
| `.doc/local.dxcp` | Git-ignored scratch documents | no |
| `.doc/index.db` | SQLite: the queryable local authority, rebuildable from the packs | no |

`dx sync` reconciles all of it: it adopts plain-text `.dx` files anything else wrote, restores
documents from the packs when the index is missing, and reports what it cannot resolve rather
than discarding it.

## Where the code lives

The engine is Rust, in `rust/`, and it is the only thing on a user's or agent's path.

| Crate | Responsibility |
|-------|----------------|
| `doc-core` | The format and its views. `format` (DOCSRC parse/stringify), `render` (HTML, Markdown, outline, sections, `nav` resolution, and `block` — one block exactly as the page carries it), `edit` (the four block operations every editing surface performs, plus `preview_block`, which changes nothing), `chunk` (content-addressed per-block chunks and the `DXCP1` pack), plus `digest`, `compress`, `search`. No OS dependencies — this crate also compiles to `wasm32`. |
| `doc-store` | The SQLite chunk store and the resolver: manifests, packs, git routing, and the stub format. The authority for content. |
| `doc-run` | Executing `::code … run` blocks: per-language plans, dependency install, timeouts, output capture, and `confine` — the kernel sandbox every block runs inside. |
| `doc-shot` | Rendering a document to PNG through an installed Chromium browser (two passes: measure, then capture). `capture_pages` divides a document into page images, breaking between blocks. |
| `doc-cli` | The `dx` binary: CLI commands, the MCP server (`dx mcp`), the local rendering service (`dx serve`, in `daemon/`), and the installer — `install` (MCP registration), `service` (the login agent that keeps `dx serve` running), `desktop` (`DX.app` and the LaunchServices registration that makes a double-clicked `.dx` open), `policies` (Firefox's policy file), `extension` (the browser extension and how each family receives it), `state` and `home` (shared by all five). |
| `doc-wasm` | `doc-core` for JavaScript hosts. Built into `editor/vscode/wasm/`. |

`editor/vscode` is the VS Code extension. It renders through `doc-wasm`, so the editor and
the CLI produce byte-identical HTML — that equality is the point, and it is pinned rather than
assumed. `doc-wasm` is compiled twice, because no one artifact loads in both hosts
(`no-modules` for a browser extension, `nodejs` for the editor), and `the … engine renders what
the dx binary renders` in `editor/github/test/engine.test.mjs` holds **each** build's
`render_html` to the bytes `dx render` wrote for the same document. Both come out of
`editor/build.sh`, in one command, for the same reason: a surface built at a different moment
is a surface running a different renderer.

**`editor/surface` is the editor, and there is one of it.** `edit.js` and `edit.css` are what
happens when a reader clicks a paragraph, and two rules are the whole of it:

- **The block keeps rendering while it is written.** The rendered block stays exactly where it
  was and redraws from the field as the characters change (`host.draw` → `edit::preview_block`,
  debounced). The field is the block's *sibling*, never its child — a `<div>` inside a `<p>` is
  not something a browser keeps, and a drawing has no inside to put a textarea in.
  The exception is a block that renders to the characters it is written in: plain prose, and a
  code listing, which is a fold over its own source. There the field **stands in** for the
  block, borrowing its type and its box from the live element (`getComputedStyle`, never a
  second copy of the type scale), so nothing on the page moves at all. The test for which
  shape applies is structural — does the renderer add any element the source did not name —
  so it needs no list of kinds to keep in step with the format.
- **Saving replaces content, never the page.** `commit`/`remove` answer with the re-rendered
  `.dx-doc` and `edit.js` swaps that one element. Nothing navigates: no flash, no lost scroll
  position, no reattaching to a page that just appeared, and nothing to "restore" afterwards.
  A host that reloads its whole view on every keystroke-sized edit is a window that blinks at
  a reader who is mid-sentence.

`editor/build.sh` copies the surface into `editor/vscode`, `packaging/build-app.sh` copies it
into `DX.app`, and neither host owns a line of it — a Mac and an editor disagreeing about
what Return does would be two products wearing one name.

What each host *does* own is the one thing only it can: how the calls get out.
`Editor.swift` answers them by running the bundled `dx`; the VS Code webview answers them
through `doc-wasm` and a `WorkspaceEdit`, so undo and source control keep working. Both land
on `doc_core::edit`, which is also what `dx source`/`dx set`/`dx insert`/`dx remove` and
`dx render --block` are — so a person typing on the page and an agent editing the file are
doing the identical thing. **Never re-implement a block operation in a host.** "What is the
editable text of a checklist?" has one answer, and a second one in an editor is a document
that changes shape depending on which surface last touched it. The same goes for "what does
this block look like": `render::block` is the page's own per-block renderer, so a block drawn
alone mid-edit and the same block drawn in its page are byte-identical.

`editor/github` is a browser extension that makes github.com show documents instead of
pointers. It gets no editing surface: github.com shows other people's repositories, which are
read there and edited nowhere. github.com's file viewer **cannot be extended server-side** — `github/markup` picks
from a fixed first-party list, and a GitHub App cannot touch a blob page — so resolution happens
in the reader's browser: fetch `.doc/repo.dxcp` from the page's own origin, ask the engine to
resolve the pointer, render. Everything that decides what the reader sees lives in
`editor/github/resolve.js` and is tested under node against a pack the real `dx` wrote
(`node --test "editor/github/test/*.test.mjs"`); `content.js` is the DOM adapter, which cannot be
tested from a terminal and is written to do nothing when it does not recognize a page.

**The engine is never in the page.** A content script inherits the host page's WebAssembly
policy, and github.com's `script-src` does not grant `'wasm-unsafe-eval'` — so compiling the
engine in a content script fails on every github.com page. `engine.js` runs in the extension's
own background context and answers from **`dx serve` when it is running, and bundled wasm when
it is not**: same engine, two doors, and `resolve.js` cannot tell which answered. The content
script keeps the *fetching*, because only a page-context request carries the reader's session,
which is what makes private repositories work with no token and no server in the middle. Do not
move the engine back into the page, and do not drop `content_security_policy.extension_pages`
from the manifest — either one silently renders nothing at all. See `docs/github.md`.

**A pack is named, never inlined.** An engine call carries `{ packRef: url }`; the engine
answers `needPack` if it does not hold it, and the bytes cross once per repository revision.
Sending the pack with each call meant a pull request touching six documents transferred and
decompressed the whole repository twelve times, which is what made diff pages crawl. Do not put
bytes back into a call.

`packaging/` builds what a person installs: `DX.app` for macOS, the Chrome Web Store archive,
and the XPI submitted to addons.mozilla.org for signing. **None of it is in the shipped
binary** — an archive writer and a rasterizer are release tools, and a document renderer has no
business carrying them onto a user's machine. `packaging/README.md` is the release runbook and
the record of what each browser actually permits, which was measured rather than assumed.

**`DX.app` is the Mac viewer, and it is one application.** `packaging/app` is AppKit and a
`WKWebView`; it renders nothing itself, it runs the `dx render` in **its own bundle** and shows
the page. A launch carrying a document is a read; a launch carrying nothing is the `dx setup`
the application has always run. Do not split those into two programs and do not let the viewer
reach for a `dx` on `PATH` — either one is a window that can disagree with the engine that drew
it. The bundle also still carries the Safari extension and the signed Firefox add-on, because
those have nowhere else to live. `packaging/README.md` has the file-type declarations and why
each is there.

**One install, per device — never one per program.** `dx setup` is the only install command.
It puts the binary on `PATH`, registers MCP, registers `dx serve` to start at login, installs
`DX.app` and registers the `.dx` type with LaunchServices (`desktop`), and configures each
browser by that browser's own route (`extension::Channel`). Adding a surface means adding a
channel, not a command: a second install step is a machine that is half set up.

**The extension is not in the binary.** Its files — page adapter, resolver, stylesheet, icons,
and the ~313 KB wasm engine — are read from a directory, never `include_bytes!`. A `dx` on a
build server, in a container, or in an agent's sandbox has no browser and no use for ~0.4 MB of
one; carrying it there cost every install 19% of the binary for a file nothing would open. So:

- `editor/github` is the **one source**, and `dx browser --from <dir>` is the **one builder**.
  `DX.app`'s copy, the Chrome Web Store archive, and the add-on Mozilla signs are all that
  command's output, so a store listing cannot drift from what a developer loads unpacked.
- `extension::installed_dir` finds what this machine has: the application's own copy first
  (built beside the binary that reads it, so it cannot be a version behind), then whatever
  `dx browser --from` wrote. Nothing else is guessed.
- A machine with neither gets `Channel::Absent` and a sentence saying where to get one.
  Naming a directory that was never installed is the failure this whole area is prone to — it
  sends a reader hunting through their browser's settings for something that is not there.

**The repository is Rust, and only Rust.** The original TypeScript implementation (`src/`,
`vscode-extension/`, `test/`, `scripts/`, the npm toolchain) was deleted on 2026-07-30: nothing
shipped from it, it had stopped being a compatibility target, and its round-trip quirks were
silently destroying content (see the format note below). The only JavaScript left is what a
browser has to run — `editor/github/*.js` and `editor/vscode/` — and both call `doc-core`
through `doc-wasm` rather than reimplementing it.

### The examples stay readable, and the fixtures are hermetic

`examples/` and `documents/` are kept as **plain text** so someone browsing the repository can
read them. `doc-core`'s round-trip assertions therefore read `tests/fixtures/*.input.dx`
copies, not those files — otherwise a `dx sync` at the repository root would convert them and
the suite would start testing pointers instead of documents. `tests/fixtures.rs` fails if a
copy drifts from the document it mirrors, so refreshing an example is a deliberate two-file
change.

## Non-negotiables

- **Storage cannot lose a byte.** A chunk holds the exact canonical text `stringify` writes for
  one block, so reassembly is concatenation and no field can be dropped by a codec that had not
  heard of it. Any encoding that understands only *some* block attributes has no place in the
  store — a structured binary one was removed for exactly that reason.
- **Canonical output must not drift.** `doc-core/src/format` round-trips real documents
  byte-for-byte against captured fixtures. `docs/dx-format-contract.md` is the authority on
  behavior. If a change makes those tests fail, the change is almost certainly wrong — but the
  fixtures are captured output, not scripture: four of them encoded genuine data loss (checklist
  items erased on save, nested list items dropped, unnamed paragraphs destroyed on re-read).
  Fixing a defect and regenerating the fixture is correct; changing a fixture to make a
  convenient change pass is not. Review the regenerated diff and say what changed and why.
- **Format changes are additive.** A new attribute must serialize to nothing when unset, or
  every existing document reformats on next save.
- **Reading never executes, and never writes.** Parsing, rendering, screenshotting, and
  resolving a pointer must stay free of side effects — resolving must not even create the index.
  This is what lets a page redraw itself on every pause in typing: `edit::preview_block` applies
  a body to a *parsed copy* and throws it away, so a reader who types and then presses Escape
  has changed no file.
  Only `dx run` and the `dx_run` tool execute code. This is why a `nav` block is resolved from
  the document it sits in and never by reading another file, and why `dx serve` reads no file,
  writes none, and runs nothing.
- **Running a document does not give it your authority.** A `.dx` is something you were
  handed, so `doc-run::confine` puts every block inside a kernel sandbox — Seatbelt on macOS,
  bubblewrap on Linux — and the shape of it is the whole security model:

  | Read | anything, minus a deny-list of credential stores |
  | Write | its own block directory, nowhere else |
  | Network | never, while the block's own code runs |

  The first is why the other two are absolute: read plus egress is exfiltration, and read
  without egress is a block that can *use* your files and do nothing but show you the result.
  Only dependency installation gets the network, which is why `plan.rs` arranges every
  language to fetch in `setup` and run offline — a `run` command that would download
  something is a reason to hand the network back, so there must never be one. A machine that
  cannot impose the boundary **does not run the block**; `DX_UNCONFINED=1` is the only way
  past, and a run made under it says so in its own output. `doc-run/tests/attacks.rs` is real
  payloads asserted to fail, and it is the evidence for every sentence above — if you change
  this area, run it, and check it still *can* fail by running the same payload with
  `DX_UNCONFINED=1`.
- **Untrusted input is everything a document carries.** A `.dx` rendered on github.com was
  written by whoever wrote that repository, and the page it lands in belongs to github.com —
  so author markup goes through `render::escape`, which is an **allow-list** of elements,
  attributes, and URL schemes. Never make it a deny-list again: the previous one was defeated
  by `<img src=x/onerror=…>`, by `<iframe srcdoc>`, and by entity-encoded schemes. A shadow
  root is not a boundary — it is the same document and the same origin.
- **The daemon holds private content, so it checks who is asking.** `dx serve` caches packs
  the reader's browser uploaded, which can come from private repositories, and pack keys are
  guessable. Every request must name a loopback `Host` (this is what stops DNS rebinding, the
  one thing a page cannot forge) and, if it carries an `Origin`, must be the extension. A
  cross-origin `POST` of `text/plain` is a *simple* request and arrives with no preflight, so
  this has to be refused in the handler and not in a CORS header.
- **Bound anything sized by untrusted input.** A frame's declared length, a `Content-Length`,
  a header line, the number of live connections. Each of these was, at some point, a way to
  make the process allocate until it died.
- **Compression is chosen, not fixed.** A `dxz` frame names its codec in its first four bytes.
  `compress` encodes each way and keeps the smallest, with "store the bytes as they are" always a
  candidate — so the stored form never exceeds the input plus an 8-byte header. `DXZ1` (the
  original LZSS) is no longer written and must be decoded **forever**: packs containing it are
  committed in real repositories. Adding a better codec means adding a magic, never converting
  anything.
- **An agent reads by looking.** `dx_read` returns the rendered pages as images, because a
  document renders to a page and the page is where the meaning is. Pages break between blocks,
  never through a line, and never leave a heading without the text it titles. `dx_source` is the
  exact text, for quoting and editing.
- **One engine.** If the editor and the CLI could render differently, they will. Render
  through `doc-core`; never re-implement a view. Rebuild the wasm after touching `doc-core` or
  the editor silently keeps the old renderer.
- **The page is a blank sheet.** One paper tone, one ink, no second surface: no panels, fills,
  tints, grids, rounded boxes, or shadows. Structure comes from type, whitespace, and hairline
  rules. Nothing is announced unless it earns the space — a successful run says nothing; only a
  failure is called out. The editing field obeys this too: it has no border, no background, and
  no box, and it takes either the block's own type or the marginal mono the sheet already uses
  for pencil notes — the page does not become a form when it is touched. A block that draws its
  own rule down the margin (a quotation, a listing) lends it to the field rather than being
  given a second one beside it.
- **A document opens as what it says, not how it was made.** Code blocks render folded behind
  their own label (`HtmlOptions::collapse_code`); what the code *produced* stays on the page.
  The fold is `details`/`summary` and must stay CSS-only — no page may need a script to open
  one, which is what lets a viewer refuse to run the document's scripts at all. A render
  nobody can click — a PNG, a printed sheet, the images `dx_read` hands an agent — passes
  `collapse_code: false`, because there is no "to start" in a picture and a reader who cannot
  expand a block must not be shown a label where the listing should be.

## Quality bar

```bash
cd rust
cargo test                                  # must be green (521 tests)
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

Tests are colocated with the code they test and named for the behavior they pin down. When
you fix a bug, add the test that would have caught it. Non-ASCII prose, empty input, and
missing toolchains are all real cases that have broken this code before.

## Building and trying it

```bash
cd rust && cargo build --release -p doc-cli   # → rust/target/release/dx
./target/release/dx doctor                    # what is installed and what is missing
./target/release/dx run ../examples/showcase.dx
```

Do **not** run `dx sync` at the repository root: it would convert `examples/` and `documents/`
into pointers. Try the store in a scratch directory instead:

```bash
mkdir /tmp/pad && cd /tmp/pad && git init -q .
cp ~/Desktop/DOC/examples/welcome.dx .
dx sync .          # adopt it: the file becomes a pointer, content goes to .doc/
dx git-setup .     # make git diff, git show, and git log -p render documents
cat welcome.dx     # a pointer
dx text welcome.dx # the document
dx stats .         # sharing and compaction
```

**One command builds every engine.** `doc-wasm` is compiled twice — `no-modules` for the
browser extension (a manifest content script cannot be an ES module) and `nodejs` for the
editor — and `editor/build.sh` does both, together, because a surface built at a different
moment is a surface running a different renderer:

```bash
./editor/build.sh                                    # → editor/{github,vscode}/wasm
cd editor/vscode && npm install && npm run package
code --install-extension dx-documents-1.0.0.vsix
```

`wasm-pack` needs the rustup toolchain on `PATH` ahead of any Homebrew `rustc`
(`PATH="$HOME/.cargo/bin:$PATH"`), and the `wasm32-unknown-unknown` target. **Rebuild after
touching `doc-core`** — both surfaces silently keep the old engine otherwise, which is what
`engine.test.mjs` now catches for each of them.

## Handoff discipline

After completing each task or wave, before ending the turn, update `HANDOFF.md` with what
was done **and validated** (with the proof) and the single concrete next step. Keep it
short — it is the resume point after `/compact` or `/clear`.
