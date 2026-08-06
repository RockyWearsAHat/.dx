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
| `doc-core` | The format and its views. `format` (DOCSRC parse/stringify, plus `mermaid`/`layout` — see below), `render` (HTML, Markdown, outline, sections, `nav` resolution, `board`, and `block` — one block exactly as the page carries it), `edit` (the block operations every editing surface performs — body, header, replace, insert, remove, tick a checklist box, arrange a board — plus `preview_block`, which changes nothing), `resolve` (what a document references past its own edge — `::code src=` files, `::view src=` coded pages shown as the page they render to, and `path.dx#block` board nodes — the path law, the reference grammar, `file_references` for what a fetched page names in turn, and `hydrate`, which fills a view/run copy through a host-supplied `Resolver` and is **never serialized**; `docs/dx-format-contract.md` § References is the authority), `chunk` (content-addressed per-block chunks and the `DXCP1` pack), plus `digest`, `compress`, `search`. No OS dependencies — this crate also compiles to `wasm32`. |
| `doc-store` | The SQLite chunk store and the resolver: manifests, packs, git routing, and the stub format. The authority for content. |
| `doc-run` | Executing `::code … run` blocks: per-language plans, dependency install, timeouts, output capture, and `confine` — the kernel sandbox every block runs inside. |
| `doc-shot` | Rendering a document to PNG through an installed Chromium browser (two passes: measure, then capture). `capture_pages` divides a document into page images, breaking between blocks, with every `::board` on an independent page at its natural size; `capture_block` photographs one block alone. `cdp` is a live DevTools session with the same browser (a hand-rolled WebSocket, no new dependency), and `play` drives the rendered page with scripted input — wait, key, click, scroll, hover — returning annotated PNG frames; it loads the same script-free render every screenshot uses, so nothing a document carries executes. `base64` is the platform's one copy of that codec (the MCP server encodes with it too). |
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

- **Clicking a block expands its controls — and the field is still the block.** A click
  replaces the block's rendered content with its editing form, in the block's own place: the
  writer's `::kind attrs` tag line (`edit::block_header`) in a quiet marginal mono above,
  and a field holding the exact body beneath. The field is the block's child and *inherits*
  its face — a heading is written at heading size, a listing in its own column, and the page
  does not move when reading becomes writing. A plain paragraph shows no tag line (prose
  carries no machinery); typing `::` at its start promotes the text into one. The tag line
  is a control: rewriting it retypes the block on save (`edit::replace_block` — the body is
  set by the block's own body rule and never re-scanned, so a listing containing `::end`
  keeps every byte). A prose field stays dressed as what it says while it is written: it is
  a `contenteditable` holding the exact source, decorated in place by the engine
  (`render::field_html`, reached as `host.decorate` — `doc-wasm`'s `field_html` in VS Code,
  `dx render --field` in DX.app), so `**bold**` is set in bold with its `**` still on the
  line in the margin ink. The decoration keeps every character of the source in the field's
  text — that is the invariant caret math stands on — and `edit.js` never tokenizes the
  format itself: the marks' *meaning* is only ever the engine's. ⌘/Ctrl+B, I, E wrap or
  unwrap the selection's mark, ⌘/Ctrl+K writes a link with the caret in the target, and
  ⌘/Ctrl+Z is a rebuilt undo (decoration redraws defeat the browser's own). Ghost text and
  a small menu at the caret complete kinds, per-kind attributes, and remembered values
  (`localStorage`, `dx.autocomplete-history.v1`); Tab or Return accepts, Escape dismisses
  the menu first and the field second. The menu is an opaque card in the page's own tone
  (only variables the theme actually publishes — an unpublished name is a dropped
  declaration, which is how it once shipped transparent), placed at the caret's rectangle
  and flipped above it when the viewport below runs out. Arrows cross the
  header/body seam at its edges; Enter in a non-tag header demotes the line back to prose.
  The one dress inheritance cannot supply: `html`/`svg` source has nothing to inherit — the
  field takes the listing's own mono (mirroring `.dx-code pre`), and a tag line naming
  `code`/`html`/`svg`/`mermaid` dresses the body the same way before any save
  (`.dx-writes-source`, which also strips the prose decoration: source means what it
  spells). No note, no preview, no second copy of the block anywhere on the
  sheet — a page with two renditions of one block is a form, not a page. Escape puts the
  rendered content back untouched. Blank paper is writable: a click on the sheet starts the
  next paragraph after the block above the click (a click that dismisses an open field only
  dismisses — it never also creates), and a paragraph left empty is removed on close rather
  than saved. The one structural wrinkle is the code fold: a `details` whose `summary` is
  removed invents a default one, so editing leaves a hidden stub summary in its place
  (`.dx-fold-stub`). A code block marked `run` (the renderer classes it `dx-runnable`)
  carries one more word in its label line: a `run` control, drawn only when the host
  offers `run(id)`, which executes that one block (`dx run --only … --approve`) and answers
  with the
  re-rendered document — a failed block arrives as content (its output block on the page),
  and only a run that changed nothing is reported as a sentence. A runnable block that was
  *edited* runs by itself the moment its field closes — the code on the page is new, so the
  output under it is stale, and showing what the changed code does is why it was changed.
  The click *is* the review the approval gate asks for: the reader is looking at that
  block's code and pressed the control beside it, and `--only` keeps the approval to that
  one block. Without it every freshly typed block would be refused (editing changes the
  fingerprint) and the surface's own run control would send people to a terminal.
  A code block is also the one kind that opens **without** its tag line: a person editing
  code wants the code, and the listing's own label already names it. ArrowUp at the very
  start reveals the `::code …` line when the block needs retyping.

  **A checklist's box is ticked by clicking it.** The renderer writes each mark's position
  on it (`data-check`, counting from zero — a statement of fact, like `dx-runnable`), and
  the surface turns those marks into checkboxes when the host offers `check(id, item)`,
  which lands on `edit::toggle_check` — the same `dx check` an agent runs. The click ticks
  and stops: it does not also open the list for writing, because ticking a thing off is
  what a checklist is *for*. Nothing is drawn to say so — the box a reader clicks is the
  same `[ ]` that was already on the page, taking the page's own ink under the pointer and
  answering to Space and Return when focused. A render with no host keeps three characters
  of ink and no affordance it cannot honour.

  **A `::board` is a node editor on the sheet.** Its body is one reference line per node
  (`- id x= y= w= h= to=`), each naming a block of the same document — usually one marked
  `hidden`, so it lives on the board instead of in the page flow — resolved like `nav`,
  never by reading another file.

  **A node's box is stated, never measured.** `x y w h` are the whole of it, the node is
  drawn at exactly that size (content longer than it scrolls inside), and every consumer
  works from those four numbers: the renderer, `edit`, the surface, and an agent running
  `dx board`. That is what lets a board be laid out identically by a browser, a PNG, and a
  terminal — the alternative, guessing how tall a block renders, was tried and produced a
  board that was one shape in the engine and another on the page. A dimension may state a
  *rule* instead of a number — `w=page`/`h=page` (the page's own measures) or `w=fit`/`h=fit`
  (the engine's deterministic estimate of the block's render, re-resolved as the block is
  rewritten) — and stays stated: `board::resolve_sizes` turns rules into numbers from
  nothing but the document, the line keeps the word forever, and an edit restating a rule's
  own resolved number keeps the rule (`edit::place_into`), so a drag that only moves a
  `fit` node does not strip the rule off it. Two consequences follow.
  The **fit** is exact: `render::board::fit` scales the canvas on *both* axes so the whole
  arrangement is in view from the first glance, and the surface re-fits by the same rule
  until the reader pans or zooms (⌘/Ctrl or a pinch — a plain wheel scrolls the page, because
  a board lives in a document). And **overlap is the engine's job**: `edit::settle` holds the
  nodes that were just placed and pushes anything they cover out past the nearest border
  (`edit::shove` — the shortest move that ends the overlap, never onto a negative coordinate,
  and downwards on a tie so a plan still grows down the column). `dx board --place` cannot
  leave one node buried any more than a drag can.

  **A whole edge is a connection point.** Each side of a node carries a strip a line is
  dragged out of, and dropping on another node lands on whichever of *its* sides is nearest —
  so a connection goes from any edge of any node to any edge of any other, and the two sides
  it was drawn between are kept (`to=steps:b-t`; a bare `to=steps` is unpinned and takes the
  facing pair). Several edges meeting one side spread evenly along it, ordered by where their
  other ends are, so a node with four connections fans them out instead of stacking them on a
  dot and crossing them on the way in. Each edge is tethered at the source and ends in an
  arrowhead, because an edge on a plan says *this, then that*, and may carry words
  (`to=steps:b-t:on%20failure`, drawn on the middle of the curve) — a decision with two
  unlabelled arrows out of it says nothing about which is which.

  **An edge is never hidden by a box.** A node is painted over the edge sheet, so a line
  passing under one is a line the reader simply loses. Two rules prevent it, both in
  `render::board`. An *unpinned* end picks its side by cost (`clearest_sides`): a box across
  the straight run outweighs any number of crossings, a shallow crossing costs more than a
  square one — two lines meeting at a right angle read as one passing over the other, a narrow
  one reads as a fork — and the facing pair breaks the tie, so a clear run keeps the
  arrangement a board has always had. Then the curve itself bends: `controls` pushes both
  handles square to the line, in growing steps and both directions, until the cubic clears
  every box it does not join. Bending the handles rather than inserting a waypoint is what
  keeps it mirrorable — the edge stays one cubic between the same two points.

  The geometry is stated once in `render::board` (sides, spread, curve, clearance) and
  mirrored in `edit.js` against the measured boxes — which are the stated boxes, so the two
  cannot disagree.

  The surface adds fit/pan/zoom, grip-drag, corner-reshape (double-click the corner to fit a
  node to its content), edge-drag linking, pick-and-Delete, and double-click-to-add — but
  **never writes a line itself**: every change is `host.board(id, action, spec)`, landing on
  `edit::board_place`/`board_arrange`/`board_detach`/`board_link`, the same operations
  `dx board` runs. Saving a board body creates a hidden paragraph for any line naming a block
  the document does not have; detaching a node removes its line, its edges, and its block only
  when the block was hidden and no other board shows it. After an edit, a node whose block
  outgrew it grows to hold it — never shrinks, and never on a plain read. Nodes are ordinary
  blocks: clicking inside one opens the same editor as anywhere else, and a checklist node's
  boxes tick like any other's.
- **Saving replaces content, never the page.** `commit`/`replace`/`remove` answer with the
  re-rendered `.dx-doc` and `edit.js` swaps that one element. Nothing navigates: no flash,
  no lost scroll position, no reattaching to a page that just appeared, and nothing to
  "restore" afterwards. A host that reloads its whole view on every keystroke-sized edit is
  a window that blinks at a reader who is mid-sentence.

`editor/build.sh` copies the surface into `editor/vscode`, `packaging/build-app.sh` copies it
into `DX.app`, and neither host owns a line of it — a Mac and an editor disagreeing about
what Return does would be two products wearing one name.

What each host *does* own is the one thing only it can: how the calls get out.
`Editor.swift` answers them by running the bundled `dx`; the VS Code webview answers them
through `doc-wasm` and a `WorkspaceEdit`, so undo and source control keep working. Both land
on `doc_core::edit`, which is also what `dx source` (`--header` for the tag line),
`dx set` (`--header` to retype), `dx insert`, `dx remove`, `dx check`, `dx render --block`,
and `dx render --field` (the decorated field view, no file read) are — so
a person typing on the page and an agent editing the file are doing the identical thing. **Never re-implement a block operation in a host.** "What is the
editable text of a checklist?" has one answer, and a second one in an editor is a document
that changes shape depending on which surface last touched it. The same goes for "what does
this block look like": `render::block` is the page's own per-block renderer, so a block drawn
alone (`dx render --block`) and the same block drawn in its page are byte-identical.

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

### The repository is a dx workspace, and the fixtures are hermetic

**This repository stores its own documents the way the product stores everything.**
`examples/*.dx` and `documents/*.dx` are one-line pointers; their content is in the committed
pack at `.doc/repo.dxcp`, and every read resolves them — `dx text`, `dx_read`, git after
`dx git-setup`. Edit them through `dx` (`dx set`, `dx_edit`, `dx sync` after a plain-text
write), never by writing pointer files by hand.

The round-trip test corpus must never become pointers, because `doc-core` compiles it in with
`include_str!` — a suite that compiled pointers would stop testing documents. So the store's
walker never adopts a `fixture`/`fixtures` directory (`doc-store::store::SKIPPED_DIRECTORIES`),
which keeps `rust/doc-core/tests/fixtures/` and `editor/github/test/fixture/` plain text.
The drift guard lives in `doc-cli` (`workspace::tests::
every_fixture_input_still_matches_the_document_it_mirrors`), the crate that can resolve a
pointer: it fails if a fixture drifts from the stored document it mirrors, so refreshing an
example is a deliberate two-file change.

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
- **A mermaid block is a board.** `::mermaid`/`::graph` is not a kind this format keeps:
  `format::mermaid` reads the flowchart at parse time and `format::layout` arranges it into a
  `::board` plus one hidden block per node, so the drawing arrives as something a reader can
  take hold of instead of a listing nothing draws. This is the one conversion `parse` performs
  — every other kind round-trips untouched — and it is deliberate: two diagram renderers that
  have to agree is the thing this repository exists to avoid. Node labels, edge direction, and
  edge labels all survive; mermaid's *shapes* do not, because a board node is a block and
  blocks are dressed by the document's own CSS. Source this converter cannot read (a sequence
  diagram, a Gantt chart, a dialect it does not know) is **left exactly as written** rather
  than replaced with an empty canvas. `layout` guarantees no two boxes overlap, ranks the
  nodes by longest path over the *forward* edges only (a loop has no longest path, and ranking
  through one stretched a six-node chart to fifteen hundred pixels), and orders each rank by
  barycentre so the links between ranks cross as little as they can.
- **A document's CSS always applies.** `::style` reaches every surface with no flag to
  forget, and it travels *inside* `.dx-doc` rather than the page `<head>` — that container is
  what an editing host swaps on save, so CSS left in a head the host never re-reads is a
  document that loses its dress the moment anyone touches it. It may dress and may not fetch:
  `render::escape::escape_style` blanks any `url(…)` naming a host and neutralizes `@import`
  and `expression(`. `docs/dx-format-contract.md` is the authority.
- **Reading never executes, and never writes.** Parsing, rendering, screenshotting, and
  resolving a pointer must stay free of side effects — resolving must not even create the index.
  `edit::preview_block` (`dx render --block --body`) applies a body to a *parsed copy* and
  throws it away, so asking what a block would look like changes no file — and a reader who
  types on the page and presses Escape has changed nothing at all.
  `dx run --review` (`dx_run` with `review`) is a read too: it reports what would run —
  including the blocks it would have to refuse — and folds no `::output` into the document
  and writes no file, because a review that left a record behind would be a read with a
  side effect.
  Only `dx run` and the `dx_run` tool execute code — with one stated exception: the agent
  read tools (`dx_read`, `dx_source` over MCP) refresh before answering, re-running code
  whose exact fingerprint *this machine already approved* when its recorded output has gone
  stale, so an agent never reads dead output and never spends a `dx_run` just to look.
  The gate holds: unreviewed code stays blocked on a read (the reply says what awaits
  review), `refresh: false` reads exactly what is stored, and every rendering surface —
  `dx render`, `dx text`, `dx serve`, the editor, the extension — stays a pure read. This
  is why a `nav` block is resolved from
  the document it sits in and never by reading another file, and why `dx serve` reads no file,
  writes none, and runs nothing. The one sanctioned outward read is `resolve::hydrate` — a
  document's own stated references (`::code src=`, `path.dx#block` board nodes), confined to
  the document's folder by `resolve::confined`, filled into a view/run copy that is never
  saved, still executing nothing. `dx serve` keeps its rule: it reads no file even for this —
  a caller passes resources in.
- **Running a document does not give it your authority.** A `.dx` is something you were
  handed, so `doc-run::confine` puts every block inside a kernel sandbox — Seatbelt on macOS,
  bubblewrap on Linux — and the shape of it is the whole security model:

  | Read | anything, minus a deny-list of credential stores |
  | Write | its own block directory, plus the folders a block's `writes=` grant names — inside the document's folder only, and the grant joins the fingerprint, so review shows it and widening it re-opens review |
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

  **And unreviewed code does not run at all.** `doc-run::approvals` is a ledger of
  fingerprints *this machine* approved (`<cache_root>/approvals`, never in a repository);
  a block whose fingerprint is not in it is `blocked` pending review, with the sentence
  naming `--review`, `--approve`, and `--force`. Only that ledger approves. A document's
  own `::output` record cannot: `hash=` is computed from the code above it, so the hand
  that writes the block writes the record too — treating it as approval let a handed-over
  document mark itself reviewed and, under `--force`, run with the bypass notice
  suppressed. For the same reason the gate is checked **before** the unchanged-fingerprint
  skip: a cached skip that trusted the document's record would hide unreviewed code behind
  it. `--force` is the one way past, and — like `DX_UNCONFINED=1` — it stamps
  `FORCED_NOTICE` into the output it produces, so a bypass is never silent.
- **Untrusted input is everything a document carries.** A `.dx` rendered on github.com was
  written by whoever wrote that repository, and the page it lands in belongs to github.com —
  so author markup goes through `render::escape`, which is an **allow-list** of elements,
  attributes, and URL schemes. Never make it a deny-list again: the previous one was defeated
  by `<img src=x/onerror=…>`, by `<iframe srcdoc>`, and by entity-encoded schemes. A shadow
  root is not a boundary — it is the same document and the same origin. The one real boundary
  the renderer uses is a `::view`'s frame: an engine-built `<iframe sandbox="">` (opaque
  origin, nothing allowed), which is what lets a whole coded page show with its own CSS where
  the allow-list could only mangle it. The `sandbox` attribute stays empty forever — granting
  it `allow-scripts` or `allow-same-origin` would turn "a page is shown" back into "a page
  runs" — and author markup still cannot write an iframe of its own.
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
- **An agent reads by the cheapest route that carries the meaning.** Prose and code are
  text: `dx_source` (with `section`) costs a fraction of a page image, so it is the default
  read, and `dx_outline` is the map that keeps every read to one section. `dx_read`'s
  images are spent on what text cannot carry — boards, diagrams, charts, rendered views —
  because a document renders to a page and, for those blocks, the page is where the meaning
  is. Both reads refresh approved stale output first (see the execution bullet above), so
  what is read is live. A project with no documents yet gets `dx index` (`dx_index`): a
  scaffolded `index.dx` from the file tree, read whole and improved by its first reader —
  TODOs replaced with what each area does, `::code src=` blocks for the load-bearing
  files — so orientation costs one read forever after. Pages break between blocks,
  never through a line, and never leave a heading without the text it titles. A `::board` is
  always a page of its own, captured **independently at its natural canvas size**
  (`render::block_page` → `doc_shot::capture_block`) — in flow the board is fitted into the
  column, which is right for context and unreadable as a picture — and only blocks in the
  page flow are measured for pagination, never the copies a board renders inside its nodes.
  One block renders out alone with `dx png --block <id>` / `dx_read` `block`: a board at
  natural size, anything else exactly as the page carries it (hidden node blocks included). The pages are
  sized for the reader they serve (`ShotOptions::for_reading`): each stays under the
  vision-ingestion limits (~1.15 MP, 1568 px longest edge), so the image the model sees is the
  image the browser captured, pixel for pixel — capturing larger only produces an image
  something else shrinks in transit. Human exports are the opposite trade: `dx png` captures
  at device scale 2 by default (`--scale`), so an exported page matches a high-density screen.
  `dx_source` is the exact text, for quoting and editing. `dx_play` (`dx play`) is the same read with hands: it
  loads the render live over `doc-shot::cdp`, performs a small input script — wait, key,
  click, scroll, hover — and returns frames stamped with their moment and action. It is still
  a read: the page carries no scripts, input is synthetic, and nothing the document says runs.
- **One engine.** If the editor and the CLI could render differently, they will. Render
  through `doc-core`; never re-implement a view. Rebuild the wasm after touching `doc-core` or
  the editor silently keeps the old renderer.
- **The page is a blank sheet.** One paper tone, one ink, no second surface: no panels, fills,
  tints, grids, rounded boxes, or shadows. Structure comes from type, whitespace, and hairline
  rules. Nothing is announced unless it earns the space — a successful run says nothing; only a
  failure is called out. The editing field obeys this too: it has no border, no background, and
  no box, and it always takes the block's own type — the page does not become a form when it
  is touched. A block that draws its own rule down the margin (a quotation, a listing) lends
  it to the field rather than being given a second one beside it.
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
cargo test                                  # must be green (807 tests)
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

The repository root is a live dx workspace — `dx sync .` here is normal and is how a
plain-text edit to an example is adopted back into `.doc/repo.dxcp` (the fixture directories
are never adopted; see above). To try the store from nothing:

```bash
mkdir /tmp/pad && cd /tmp/pad && git init -q .
dx textconv ~/Desktop/DOC/examples/welcome.dx > welcome.dx
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
