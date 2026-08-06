# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md)._

Last updated: 2026-08-05

## This wave, part 27: comprehensive audit, board safety validation, and test coverage

**Audit completed** (workflow wf_c0fe2592-861): Identified 8 specific issues across critical, high, medium, low.

**FIXED (committed b1b39a7):**
- ✅ **Block type validation**: Added `board_invalid_block_type()` to reject script/stylesheet/style/output/image blocks from boards with clear error messages
- ✅ **Test coverage**: Added 6 comprehensive tests for board code execution, node references, edge sequencing, content overflow, and cleanup
- ✅ Framework for security validation in place

**CRITICAL (not yet fixed):**
- ⏳ No approval gate before running hidden code blocks on boards — `dx_run` silently executes all marked blocks
- ⏳ Hidden code not inspectable before execution — board shows node ID, not code content

**HIGH (not yet fixed):**
- ⏳ No sequencing support — edge labels unused, no `--follow-edges` for board-ordered execution
- ⏳ TypeScript routed to deno instead of ts-node (limits node_modules access)

**Status**: Board node manipulation solid (36 tests pass). Code execution security incomplete. Remaining: approval workflow, code decomposition visibility, edge-based sequencing.

Next step: Implement approval gate and review mode for code execution.

## Previous wave, part 26: the run record fingerprints what it reads, and the map names what it points at

**"Read handoff and fix; rank dx-for-agents honestly first; fix unintended flaws; make the
site plan beautiful."** The part-25 fingerprint gap is closed at the engine, plus one map
flaw the grounded evaluation surfaced.

- **`reads=` closes the honesty gap** (the part-25 "next step"): a run block declares the
  sibling files its code reads (`::code … run reads=site/index.html,site/site.css` —
  comma-separated, path-law-confined, additive: unset serializes to nothing). The files'
  current text joins the fingerprint (`doc-run::declared_reads`/`fingerprint`), so a
  stylesheet-only edit re-runs the proof on the next **plain** `dx run` — no `--force`, no
  stale "no changes". A declared file that is missing or outside the folder **blocks** the
  run with a sentence; a fingerprint that silently omitted an input would be the same lie
  again. Wired through model, parse/stringify/normalize (normalize was the silent dropper —
  it rebuilds code blocks field-by-field), doc-wasm DTO, surface ATTRS autocomplete,
  format contract, README. 7 new tests.
- **Outline previews name references** (`render::outline::preview_content`): nine `::view`
  rows all previewed as the same hydrated `<!doctype html>` preamble — the map hid exactly
  what an agent needs to choose the next read. A `code src=`/`view src=` row now leads with
  its reference (`site/index.html#tonight — <!doctype…`), and previews the bare path when
  unhydrated instead of an empty string. One new test.
- **`examples/example_site_plan.dx` revised**: verify declares its reads; proof-note states
  the new truth (was worded around the gap); prose caught up with the board ("two screens" →
  the nine that exist, section screens introduced); structure gained two level-2 headings
  ("The plan, on one canvas", "One loop, two hands"); the board gained a full-width gallery
  caption node, the measure band is truly centered (x=335), gallery rows shifted down 60.
- **Validated**: cargo **713** green, clippy `-D warnings` clean, fmt clean, wasm rebuilt
  (`editor/build.sh`), github 34/34 + vscode 34/34, `~/.local/bin/dx` refreshed
  (rm-then-cp). Live: `dx fmt` idempotent; `dx run` re-ran verify off the header change
  (21/21); appended a comment to `site.css` → plain `dx run` **re-ran**; restored → re-ran;
  unchanged → `skipped cached / no changes`. Board + all four pages captured and inspected.
- **Found, not fixed (needs a restart, not code)**: the MCP server process outlives
  `/clear` and this session's `dx` MCP still runs a pre-part-22 engine — `dx_outline`
  reported every `::view` as an empty paragraph until the CLI was used directly. Restart
  the assistant to pick up the refreshed binary.

Next step: part 24's remaining store-listing steps, or `/code-review` the accumulated
parts 8–26 diff and commit.

## Previous wave, part 25: fragment views close the visual loop, and the site retested through them

**"Fix + retest"** (from the part-24 handoff). Two halves: an engine feature, then the QA it
unlocks — all inside dx, no browser driven.

- **Engine: `::view src=page.html#id` is now a fragment view** (`doc-core/src/resolve.rs`).
  The file before the `#` is read (stylesheets still inline; `references` strips the fragment
  for prefetching hosts), and hydration appends one selector to the page —
  `body :not(#id):not(#id *):not(:has(#id)) { display:none }` — so the frame shows just that
  element: one screen per section, script-free, identical on every surface. Only a plain-id
  fragment (`[A-Za-z0-9_-]+`) is one; any other `#` stays part of the filename, which also
  keeps the fragment inert in the selector (a `#x{}…` src is a filename, judged by the path
  law). Note nested `:has()` is invalid CSS — "hide what precedes" cannot be written, "show
  the named element" can, and is the better contract anyway. Three new tests in `resolve.rs`;
  `docs/dx-format-contract.md` § References updated. `cargo test` green, clippy/fmt clean,
  wasm rebuilt (`./editor/build.sh`), both JS suites 34/34, `~/.local/bin/dx` refreshed
  (rm-then-cp — cp over a signed Mac binary gets SIGKILL'd).
- **The board now carries seven section screens** (`examples/example_site_plan.dx`):
  tonight/fell/visit/journal at 1180, tonight/fell/visit at 390. Every part-24 visual claim
  was then *seen*: night-bar figcaption clear of the lower labels (the 68px fix holds); fell
  3-column desk grid and one-column phone; visit email field + aside; journal renumbered 04;
  Fraunces loads (URL fetches 200, 24 faces — the 5-axis tuple is well-formed).
- **Torch mode checked across the lower sections** via a scratchpad copy with the checkbox
  `checked` (site/ untouched): full red remap on visit/fell/journal, photographs re-toned, no
  palette leaks.
- **Print stylesheet rendered for the first time and fixed** (`site/site.css`): the print
  block now restates `--starlight/--ink-dim/--ink-faint/--ember/--hairline` as paper inks —
  they were translucent starlight, near-white on paper — and gives the night-bar a printable
  border (printers drop backgrounds). Checked via a scratch `@media print`→`all` copy.
- **Phone night-bar overflow fixed**: the 07:54 sunrise label ran past the right edge at
  ≤820px (page-wide horizontal scroll); the last mark's label now flips inward in the phone
  media block.
- **Fragile verify claim retired**: lazy-loading now pinned to `len(images) - 1` (hero eager,
  rest lazy) instead of a magic 4. Verify: **21/21 hold**, weight 25,779 of 30,000 bytes.
- **Honesty gap found and worded around**: `dx run` caches on the block's own text, so a
  `site.css`-only edit reports "no changes" — the proof-note claimed the record fingerprints
  "what was read", which it does not; it now says the next `dx run --force` rewrites the
  record. The deeper fix (a run block declaring the files it reads, so the fingerprint covers
  them) is open.

Next step: that fingerprint gap, or part 24's remaining store-listing steps below.

## This wave, part 24: the example site professionalized, tracked on its own board

**"10x professionalize the example site; use the plan document to track progress."** All in
`examples/` — no engine change, no wasm rebuild needed.

- `examples/site/` revised: CSS-only **red-torch mode** (a checkbox the stylesheet listens
  to via `:has()` — palette re-maps, photographs re-tone red; the page obeys its own
  dark-sky code), the **night drawn to scale** (an instrument bar under the almanac, the
  five times placed where they happen), a new **Fell** section (getting here after dark,
  the code, what to bring), plus skip link, `<main>`, favicon/theme-color/Open Graph,
  `required`/`autocomplete`, lazy images, a print stylesheet, and Fraunces `SOFT`/`WONK`
  axes on the hero. Still two files, still no script.
- `examples/example_site_plan.dx` tracks it: a `polish` checklist node on the board (all
  ticked), the sitemap grew the Fell entry, and **verify grew a claim per polish item** —
  recorded 21/21, 25,323 bytes (< 30 KB budget). Validated by `dx run` + board capture
  (`dx png --block plan`); the desk/phone views show the shipped revision live.
- **Gap found while working**: a `::view src=site/index.html#fell` does not resolve — the
  path law reads the fragment as part of the filename, so a board can only frame a page's
  *top*. Fragment-aware views (scroll the sandboxed frame to the anchor) would let a board
  show one screen per section, which is exactly the review loop this document exists for.

**Next step**: teach `resolve`/`render` view fragments (`src=page.html#anchor` → frame
scrolled to the anchor), then put per-section screens on the site plan board.

## Previous wave, part 23: sharp captures — boards paginate independently at natural size

**"The views and screenshots are super low quality. Fix and retest. Views should auto split
elements to the max whole number that fit. Boards always rendered independently, and boards
or individual nodes renderable out easily."** All fixed at the engine, and the part-21
pagination wart with them. Two root causes found:

- **Pagination measured the wrong boxes** — the measuring script collected every
  `[data-block-id]`, including the copies a board renders *inside* its nodes (at scaled,
  in-viewport positions), which is what cut the strip pages, the blank final page, and
  `site-css` attributed to three pages. The script now measures only the flow: an element
  inside a `.dx-board` other than the board itself is skipped (`with_measuring_script`).
  Flow pages therefore pack the max whole blocks per page again, as `packed_ranges` always
  intended.
- **Boards were photographed as their column-fitted miniature** — a ~1350px canvas fitted
  into the 680px static column, then captured at 860px width: ~0.5× twice. New
  `render::block_page` (doc-core) puts one block alone on a capture-ready page — a `::board`
  at its **natural viewport** (`board::natural_viewport`, nodes' bounding box + fit margin,
  so `fit` lands on scale 1 and every node is exactly its stated box), shrunk uniformly only
  past stated `PageBounds`, never enlarged; any other block (hidden node blocks included) in
  the ordinary column. `board_html` is unchanged in flow (`board_html_in` with the column
  viewport — byte-identical output, wasm parity holds without a rebuild).
- **`capture_pages` plans, then shoots** (`plan_pages`): flow segments paginate by
  `packed_ranges`; every non-hidden `::board` becomes its own independent page at natural
  size, slotted where the board sits in reading order; trailing sheet margin makes no page.
  `ShotOptions` gained `max_page_edge`/`max_page_pixels` — `for_reading` sets the vision
  caps so a board page arrives at the model unscaled (verified live: `dx_read block=plan` →
  1077×1068 = the 1.15MP budget exactly); exports default to 4000px/16MP and keep density
  via `--scale`.
- **One block renders out in one command**: `doc_shot::capture_block`, CLI
  `dx png <file> --block <id>` (writes `<stem>-<id>.png`), and `dx_read` gained a `block`
  argument (one labelled image back). A missing id is a sentence.
- **Flake fixed in passing**: live-browser tests raced the `DX_BROWSER` env tests — a
  capture measuring during another test's bogus override silently paginated blind.
  `browser::ENV_LOCK` is now crate-visible and every live test takes a turn on it.
- **Validated**: cargo **704** green (10 new: 3 block_page, 4 plan/budget/script, 1 live
  end-to-end board capture, plus refactors), clippy `-D warnings` clean, fmt clean, github
  34/34 + vscode 34/34 (parity holds, no wasm rebuild needed — flow bytes unchanged). Live:
  `example_site_plan.dx` paginates 4 clean pages (was strips/blanks/misattribution), board
  page 1398×1386 natural and fully legible (inspected), `--block plan` and `--block palette`
  inspected sharp, `block-reference.dx` (mermaid board) clean, MCP `dx_read block=plan`
  answers one vision-budget image. Docs updated in the same edit: README, CLAUDE.md
  (crate row + "an agent reads by looking"), HELP.

**Next step**: unchanged from part 22 — nothing from parts 8–23 committed; `/code-review`
the accumulated diff, then commit.

## Previous wave, part 22: `::view` — a document shows the page its code renders to

**"The example document isn't sourcing its views from the actual coded pages."** True: the
plan's three screens were hand-written `::html` mockups dressed by the doc's own `::style`,
parallel to `site/index.html` and free to drift. Fixed at the engine: a new reference kind
that shows a coded page as the page, then the example rewritten onto it.

- **`::view src=path width= height=` (new kind, additive)** — the third reference:
  `::code src=` is what the file *says*, `::view src=` is what it *renders to*. Hydration
  (`resolve`) fills the block with the page's current markup and **inlines its relative
  stylesheets** (`inline_stylesheets`; absolute hrefs stay the frame's own to fetch); new
  `resolve::file_references(path, text)` is the second round of the prefetch protocol for
  gathering hosts. The renderer frames the page in an **`<iframe sandbox="">`** — opaque
  origin, nothing allowed — which is the boundary that lets a whole page render with its own
  `<body>`/CSS where `escape` could only mangle it; reading still executes nothing, and the
  `sandbox` attribute must stay empty forever (CLAUDE.md's untrusted-input rule grew this).
  The frame is laid out at its stated viewport (defaults 1180×760) and scaled uniformly:
  in flow, into the column, never upscaled (`width=390` shows a real phone layout at phone
  size); on a board, a view node fills its stated box edge to edge (`dx-board-node-view`,
  padding 0, scale = (w-2)/width), and `h=fit` keeps the frame's aspect. Not template-
  interpolated (a coded page's braces are its own). Unresolved → sentence, never an empty
  frame; a view is never runnable.
- **Wired everywhere**: model (`Block.width`, mirrored in doc-wasm's DTO), format arms +
  round-trip test, `AUTHORABLE`, wasm `file_references`, daemon `file_references` (CALLS
  now 7), VS Code `resourcesFor` and github `content.js` gather as a **queue** (fetched
  file → `file_references` → fetch those too), surface vocab (`::view src=` in KINDS/ATTRS,
  kindOf maps `.dx-view`).
- **`examples/example_site_plan.dx` rewritten**: the three mockups and all `.wf` wireframe
  CSS are gone; the board now carries `desk` (view of `site/index.html` at 1180) and
  `phone` (same file at 390, `h=fit` both) — the real page, live, Unsplash photography and
  Fraunces and all; edges `site-html → desk` ("renders as") and `desk → phone` ("in a
  pocket"); board tightened (height 690, measure y=1260). `::style` keeps only the palette
  tile and wordmark — the artifacts with no file of their own. `dx fmt` idempotent.
- **Validated**: cargo **696** green (13 new: format round-trip, 5 resolve, 5 render, board
  fit), clippy `-D warnings` clean, fmt clean, `tsc --noEmit` clean, both wasm engines
  rebuilt (`editor/build.sh`), github 34/34 + vscode 34/34 (engine parity holds), live
  `dx render`/`dx png` of the example inspected: both screens are the shipped page at the
  right scales, stylesheet inlined (no `<link>` survives), phone layout is `site.css`'s own
  media query answering a 390px viewport.
- **Known gaps, deliberate**: github.com's CSP may block `about:srcdoc` frames on blob
  pages — unverified in a real browser (extension archives need their rebuild anyway, the
  part-20 gap); the part-21 `dx png --pages` hidden-block pagination wart stands.

**Next step**: unchanged from part 21 — nothing from parts 8–22 committed; `/code-review`
the accumulated diff, then commit. A DX.app (WebKit) look at the view frames before shipping
visuals stays the part-12 discipline.

## Previous wave, part 21: the site-plan example ships a real site

**"Rewrite the example website document to demo a beautiful site designed and created in
one shot, referenced and revised through the document."** Done and validated.

- **`examples/site/` is a real site** — `index.html` + `site.css`, a dark editorial
  one-pager for Hollow Fell (Fraunces display serif + mono almanac labels, Unsplash
  photography, zero JavaScript, palette tokens `--night/--zenith/--starlight/--ember`
  matching the plan's spec tile). Verified by headless-Chrome screenshots at 1440 and
  500 px: hero, almanac, booking form, journal, footer all render as designed.
  (Headless capture notes: window height inflates `vh` heroes — pin the hero for
  full-page shots; headless Chrome clamps window width to 500 CSS px minimum.)
- **`examples/example_site_plan.dx` rewritten** around design → ship → verify: the board
  now carries two `::code src=site/…` listing nodes (the shipped files' current text at
  every render), edges telling the story (`one sheet`, `browse`, `shipped as`), and a
  page-flow proof section — `::code id=verify lang=python run` reads the shipped files
  back and holds them to the spec (palette hexes, alt text, anchors→ids, no script,
  five almanac times, twelve-cap, page weight). `dx run` recorded
  **"13 of 13 spec claims hold"**; a second run is `skipped cached / no changes`.
  Board `height=780` fits the whole arrangement with no dead canvas.
- **Known wart (engine, next step candidate)**: `dx png --pages` on this document splits
  oddly around hidden blocks — a 400 px strip page for the board, the full board repeated
  on the next page, a blank 400 px final page, and per-page block lists that misattribute
  (`site-css` on three pages). The HTML render and single-shot PNG are correct; the
  pagination pass in `doc-shot::capture_pages` mis-measures hidden blocks. Worth a look
  before shipping `dx_read` demos of this example.

## Previous wave, part 20: references (one source of truth), vision-sized agent pages, 2× exports

**"Board nodes should be smart links so nothing is repeated and it's always up to date —
including local code as the source of truth, reviewed in .dx. Images must export at full
screen fidelity, and the agent must extract everything from its images accurately."** Done
and validated end to end.

- **`doc_core::resolve` (new module) — the resolver seam.** Two reference kinds:
  `::code src=path` (sibling file's *current* text is the listing; `run` executes it) and
  board node lines `- plan.dx#step x= y=` (one block of a sibling document, drawn current).
  `confined` is the path law (relative, downward only; `..`/absolute/`:`/`\` refused before
  any resolver is asked); `hydrate` fills a parsed view/run copy and is **never serialized**
  (the stored doc keeps the reference + empty body — verified: `dx run` saves
  `src=src/greet.sh` back with body empty); unresolved refs become sentences in the block's
  place and `dx run` records them `blocked` instead of executing. Fingerprints hash the
  file's text, so editing the file stales the recorded output (verified live: cached →
  edit file → re-runs). `docs/dx-format-contract.md` § References is the authority.
- **Wired everywhere, hosts transport only**: CLI (`workspace::resolver_for` — sibling `.dx`
  through the store), MCP (`document_at` hydrates; `dx_source` stays exact), `doc-run`
  (`run_document` takes `&dyn Resolver`; executes a hydrated copy, folds outputs into the
  original), daemon (`render_html` optional 4th resources arg + new `references` call, CALLS
  now 6), doc-wasm (`render_html(text, theme, fragment, resources?)` + `references(text)`),
  VS Code (`engine.resourcesFor` reads workspace files, resolves pointer siblings via
  `.doc/repo.dxcp` + `pack_document`), github extension (`content.js resourcesFor`: documents
  from the committed pack, files via the session-carrying raw route).
- **Vision-sized agent pages**: `ShotOptions::for_reading(width?)` — default 860 CSS px
  (content column 46rem@17px + margins), page height derived so every page ≤ ~1.15 MP and
  ≤ 1568 px edge → zero downscale between browser and model; `dx_read` uses it. Block-boundary
  page breaks unchanged.
- **2× human exports**: `ShotOptions.scale` (1–4; measure pass always 1×; `--window-size` is
  CSS px, PNG = window × scale — verified empirically), `dx png` defaults `--scale 2`
  (showcase: 2400×4478 actual pixels).
- **Validation**: 683 Rust tests green (includes 9 new resolve tests + 2 doc-shot),
  `clippy --all-targets -D warnings` clean, `cargo fmt --check` clean, github suite 34/34
  (engine parity holds for both wasm builds), vscode suite 34/34 (new cross-boundary
  reference test), `tsc --noEmit` clean, wasm rebuilt via `editor/build.sh`.
- **Known gaps, deliberate**: editing a `src=` listing in an editor edits the stored (empty)
  body, not the file — write-back is future work; `preview_block` does not hydrate; store
  archives (`dx browser --from`, DX.app copy) need a rebuild at next release to carry the new
  extension JS.

**Next step**: teach the editing surface to open a `src=` listing as the file (read-only or
write-back), and rebuild/re-package the browser-extension archives so the store copies carry
`references` support.

## Previous wave, part 19: lines enter the arrowhead dead-center, `page`/`fit` node sizes, and the plan redesigned from scratch

**"Lines always enter in the center of the arrowhead. Redesign the whole example site from
scratch. Allow a page width/height — or content autofit — for board nodes."** All three done
and validated.

- **Arrowhead entry**: an edge path is now a cubic to a `lead` point (≤16px out from the
  arrival anchor along its side's normal) plus a **straight `L` tail** into the anchor —
  `render::board::lead`/`curve`, mirrored verbatim as `leadOf`/`curveBetween` in `edit.js`
  (labels ride the cubic's midpoint in both). A bent cubic is still turning as it arrives, so
  the head pointed one way while the line came in another; the straight tail pins the head
  square to the side and the line dead down its axis. Pinned by
  `the_line_enters_the_arrowhead_straight_along_its_axis`.
- **`w=page|fit`, `h=page|fit` in the node line** (additive; numbers unchanged): `page` →
  the 680px column / 480px default viewport (`PAGE_NODE_WIDTH/HEIGHT`), `fit` → the engine's
  deterministic estimate of the block's render (`board::resolve_sizes` + `fit_width`/
  `fit_height`, metrics taken from `render::theme`'s own numbers; lists read `Block::items` —
  the empty-`text` bug is pinned by `a_list_fits_to_its_items`). The box stays *stated*: rules
  resolve from nothing but the document, every consumer draws the identical rectangle, and
  `node_line` writes the word back forever. `edit::SizeSpec` (`Keep|Px|Page|Fit`) is the one
  caller vocabulary — `dx board --place N --w page --h fit`, `--arrange "id,x,y,page,fit"` —
  and `place_into` keeps a rule when an edit restates its own resolved number, so a surface
  drag that only moves a `fit` node does not strip the rule (settle resolves before overlap
  math, wasm keeps its numeric door). `--enable-nontrapping-float-to-int` added to doc-wasm's
  wasm-opt flags (the new `f64 as u32` saturating casts emit `i32.trunc_sat_f64_u`).
- **`examples/example_site_plan.dx` redesigned from scratch**: Hollow Fell, a dark-sky
  observatory's booking site — night palette (night/zenith/starlight/ember), CSS-drawn skies
  (milky-way band, moonrise, meteor streak, plough star field, every horizon a hard stop),
  HOLLOW FELL wordmark SVG, home at desktop width, tonight's-sky and booking screens at phone
  width, the hero as shipped markup (CSS sky, no base64 PNG), launch checklist, and a
  `w=page h=fit` measure band the two bottom columns feed into. The brief, both checklists,
  the sitemap, the wordmark (viewBox aspect), and the measure all sit on `h=fit` — the new
  rules doing real work. Restored in passing: the repo root had been store-adopted again —
  `tutorial.dx` was a *dangling* pointer (content gone from the pack; restored from HEAD),
  `example_site_plan.dx` recovered through the store, root `.doc/repo.dxcp`/`index.db`
  removed (tracked `.doc/README.md` kept).
- **Docs in the same edit**: format contract (node-line rules), README (board section +
  `--w page --h fit` example), CLAUDE.md (stated-box paragraph grew the rules), `dx` HELP.
- **Validated**: cargo **672** green (242+275+2+5+44+13+42+49; 8 new), clippy `-D warnings`
  clean, fmt clean; `editor/build.sh` rebuilt both engines, engine parity + suites green
  (34 github + 33 vscode); example `dx fmt` idempotent, `--section plan` resolves all 11
  nodes, full-page `dx png` inspected in both themes plus crops of every arrow junction —
  every head entered dead-center on a straight run, no clipped list lines (the `fit`
  estimator gained per-item lead after the first render showed clipping).

**Next step**: nothing committed from parts 8–19; `/code-review` the accumulated diff, then
commit. A DX.app (WebKit) look at the new plan before shipping visuals is still the
part-12 discipline.

## Previous wave, part 18: edge curves fixed at the root, and the plan redrawn as a design system

**"Fix the arrows… rewrite the example site to be much more beautiful."** Both done and validated.

- **The wobble had one cause**: `render::board::controls` gave each cubic handle
  `clamp(span·0.42, 30, 160)` of reach regardless of room, so two nodes a short gap apart got
  handles that overshot past each other and the curve hooked into an S. Each handle is now
  capped at **half the run's advance along its own normal** (full reach kept when the run
  starts against the normal, so loop-back edges still loop). Same rule mirrored verbatim in
  `editor/surface/edit.js::controlsFor`. Arrowhead redrawn as a swept, slightly concave dart
  (9-unit marker), replacing the squat 11-unit triangle.
- **`examples/example_site_plan.dx` rewritten** as an aligned two-column spec: every
  same-column edge is now a dead-straight vertical (node centres aligned at x=190 and x=650),
  cross-edges (`browse`, `book`) curve through the gutters with their labels in clear space.
  New nodes show a board holds anything: an `::svg` wordmark (brass point, tracked serif
  caps), a palette tile with display/caption/interface voice rows, and the three screens at
  the widths the site will really be — home at desktop width (three gallery cards), galleries
  and booking at phone width (`MV` monogram bar). The `build` hero PNG was regenerated
  (176×88, 3 KB, Up-filtered) to match the new `.dusk` art direction — dusk glow, light lane
  on a dark sea.
- **Validated**: 664 Rust tests green, clippy `-D warnings` clean, `fmt` clean;
  `editor/build.sh` rebuilt both wasm engines and `engine.test.mjs` pins each to `dx render`'s
  bytes (34 github + 33 vscode tests green); full-page `dx png` inspected in dark and light
  with crops of every board region — no clipped node content, no edge under a box, every
  arrowhead arriving square.

Next step: nothing pending for boards; the plan document is the reference example.

## Previous wave, part 17: tools reviewed to 100%, installed fresh, and the plan made beautiful in the real app

**"Review the tools and completion… test with the example site dx document. Write it to be
absolutely beautiful."** Reviewed, completed, and validated end to end.

- **Tool completion review**: MCP catalogue holds all 10 tools including `dx_play` — but the
  *installed* `~/.local/bin/dx` (Aug 1) predated part 15, so the live MCP server was one tool
  short. `packaging/build-app.sh` rebuilt, running DX.app quit, installed via the fresh
  bundle's own `dx setup`: `/Applications/DX.app` binary `cmp`-identical to the build,
  `~/.local/bin/dx` now answers `dx play` (restart the assistant to see `dx_play` in MCP).
- **Tools exercised against `examples/example_site_plan.dx`** (store copy in a scratch pad):
  `dx sync` adopt → pointer, `dx check launch --item 2` ticked exactly that line,
  `dx board --place --w/--h` kept `x y` (part 10's fix holds), `--link`/`--unlink`
  round-tripped, `dx render --section plan` resolves all 10 nodes (part 16's fix holds),
  `dx render --block build` draws the one block, `dx play` returned 8 annotated frames with
  the board visibly mid-scroll. All green; 664 workspace tests pass, 0 failures.
- **The plan rewritten to beautiful**, each fix found by crop or by the real app:
  - *The `build` hero was a mud smear* — a 28px base64 PNG upscaled to 132px. Replaced with a
    crafted 132×66 photograph (generated, grain-dithered, 8.4 KB): sun behind cloud upper
    left, rain veils on the right, hard horizon, broken reflection lane in a lichen sea —
    the document's own palette. It now reads as the photograph the site is about.
  - *Labeled edges got real runs*: wireframes 240→230 wide (gutters 40→55) so `browse`/`book`
    clear their arrowheads; `build`/`launch` dropped 30 (60px run for `becomes`).
  - *Wireframe bars collided at 230* — `.wf .bar` mark and links touched. Bar gained
    `gap: 12px`, mark 11px/0.15em, links 7.5px/0.08em, both `nowrap`.
  - *`veil` had no form* (washed pale box in the grid) — added a dark radial anchor;
    `pines`' glow moved off its treeline into the sky.
  - **WebKit found one more real clip**: `sitemap`'s last item wraps "yet" onto a second line
    in the real DX.app (Chromium keeps it to one) and h=240 hid it. h→270, rows below moved
    down 30 to keep the 60px rhythm, board height recomputed by `fit`'s own formula
    (0.79 × 1510 + 48 → 1242). Re-verified in the app: "yet" on its own line with slack.
- **Validated**: `dx fmt` idempotent, `dx outline` clean, `--section plan` zero missing
  blocks, full-page `dx png` inspected in both themes plus 2–3× crops of every region, and
  the real DX.app (WebKit) driven page-by-page with frontmost-verified captures — no clipped
  text, no bar collisions, labels clear everywhere. No Rust touched this session, so the
  part 16 clippy/fmt/wasm gates stand; no fixture mirrors this example.

**Next step**: unchanged — nothing from parts 8–17 is committed; `/code-review` the
accumulated diff, then commit. The document open in DX.app is the scratch copy at
`/tmp/dx_view_pad2`, kept for a by-hand look.

## This wave, part 16: section-scoped boards resolve, and the site plan redesigned to be worth showing

**"Read the handoff, make it work. Then redesign the website plan to actually be a beautiful
'example' site plan."** Both halves done and validated.

- **The part-13 engine bug is fixed**: `dx render`/`dx png --section <board-id>` used to draw
  every node as `no block named '<id>'` — `render::outline::section` sliced the document
  before the board resolved its node lines. `carry_presentation` now also carries every block
  a `::board` inside the slice references (`board_references`, via `render::board::nodes` —
  the board's own grammar, not a re-parse), cloned with `hidden` set so it lives on the board
  instead of also appearing in the slice's flow. The whole document is untouched — the clone
  is the slice's, reading still writes nothing. Two tests pin it (`outline.rs`): a board
  section resolves all nodes exactly once, and a heading section holding a board resolves
  nodes outside the slice. Live: `dx render examples/example_site_plan.dx --section plan` →
  zero missing-block sentences, all 10 nodes + the board present.
- **`examples/example_site_plan.dx` redesigned** (working `--section plan` renders drove it):
  - *Photographs are now light, not smears*: each CSS photo is layered gradients with a
    **hard-stop horizon** (coast frames), a radial sun/window glow, and one shared inset
    vignette on `.wf .ph` — the difference between a tan fill and a photograph.
  - *Composition rebalanced*: bottom row is now two 380-wide columns (`build` the shipped
    hero, `launch` whose 6 items sit one per line instead of the old 460px wrapped tower),
    and the `measure` quote is a centered 520-wide band both columns feed into — a symmetric
    funnel closing the plan. Edge label `ships as` → `becomes` (the curve crossing the word
    gap read as a hyphen).
  - *Palette tile* gained a Georgia-italic type specimen line and taller swatches; captions
    across the wireframes are Georgia italic (the photographer's caption voice, named in the
    tile's own footnote).
  - *Sizes verified by crop*, part-13 style: palette 260 (was clipping its last line), home
    380 (the foot line was clipped under the two-line caption), measure 120; board height
    recomputed from `fit`'s own formula (0.79 × 1450 + 48 → 1194), not guessed.
- **Validated**: cargo 632/632 (2 new), clippy `-D warnings` clean, fmt clean;
  `./editor/build.sh` rerun (doc-core touched — both wasm engines current); node 34/34
  github + 33/33 vscode; `dx fmt` idempotent on the example, `dx outline` clean, full-page
  `dx png` inspected in **both themes** plus 2× crops of every previously-clipped region.
  No fixture mirrors this example, so no two-file change owed.

**Next step**: nothing committed this wave (nor from parts 8–15); `/code-review` the
accumulated diff, then commit. Per part 12's lesson, a DX.app (WebKit) look at the redesigned
plan before calling the visuals final would be prudent — Chromium crops all clear, and every
text node carries headroom, but WebKit metrics run taller.

## This wave, part 15: `dx play` — phase 1 of the agent dev workspace, built

**"Build it."** — phase 1 of the plan from part 14 is implemented end to end: a live browser
session over the same bundled Chromium, an input-script vocabulary, a frame pipeline, and the
`dx_play` MCP tool. Nothing in the security model changed: play loads the same script-free
render every screenshot uses, so nothing a document carries executes (phase 2's sanctioned
interactive kind remains unbuilt, by design of the phasing).

- **`doc-shot::cdp`** — a DevTools session with the browser `browser::find` already locates:
  launch with `--remote-debugging-port=0` + a scratch `--user-data-dir`, read the ws URL off
  stderr, then a hand-rolled RFC 6455 WebSocket client over `TcpStream` (handshake, masked
  frames, fragmentation, ping/pong, bounded message sizes — no new dependency beyond
  `serde_json`). `open` (create/attach/navigate/load-wait), `command`, `evaluate`,
  `screenshot(clip)` with `captureBeyondViewport`. Dropping the session kills the browser.
- **`doc-shot::play`** — the harness. Script statements split on `;`/newline: `wait 500ms|2s`,
  `key Space|Enter|Escape|arrows|PageDown|…|any char`, `click <block-id|x,y>`,
  `scroll [target] <±px>` (over a target = that node's own overflow), `hover <target>`.
  Targets resolve per action by walking `[data-block-id]` (no selector escaping), scrolled
  into view first. Frames: one at start, one per landed action (annotated with the action's
  own words), waits captured at `fps` (default 10, max 30), one at end; each `PlayFrame`
  carries png/at_ms/note/width/height. Bounds stated up front, not truncated silently:
  ≤100 actions, ≤30s total waiting, ≤300 frames. `--node <id>` clips every frame to that
  block's box. `base64` (encode+decode) moved to `doc-shot` as the platform's one copy;
  `mcp/encode.rs` deleted, MCP encodes through it.
- **CLI `dx play`** — `dx play <file> --script "…" [--node ID] [--fps N] [--width/--height]
  [--theme T] [--section ID] [--out DIR]`; writes `frame-NNN.png` into `<stem>-frames/` (or
  `--out`) and reports each frame's moment + action. In the command table (flags enforced),
  HELP updated.
- **MCP `dx_play`** — second tool in the catalogue (after `dx_read`): same script, returns
  interleaved text + image items, each frame stamped `t=…ms — action`. Over 12 frames the
  answer thins waits but keeps every action frame plus first/last (`frame_sample`), and says
  it did.
- **Validated**: full workspace green — doc-shot 42 (incl. two live-browser play tests: an
  annotated multi-action run, and a node clip smaller than the viewport + ghost-node
  refusal), doc-cli 241 (incl. live CLI + MCP play tests), doc-core 266+2+5, doc-run 44,
  doc-store 49, attacks 13; clippy `-D warnings` clean, fmt clean. Live smoke:
  `dx play examples/showcase.dx --script "wait 300ms; scroll 600; key PageDown; hover
  400,300; wait 200ms"` → 10 annotated frames; frame 5 visibly shows the page scrolled past
  frame 0's viewport (looked at, not assumed). Fixed in passing: the two `DX_BROWSER` env
  tests raced (process-global env, parallel tests) — now serialized on a mutex.
- **Docs in the same edit**: README (command list, agent table now ten tools, a "watch the
  page react" paragraph), CLAUDE.md (doc-shot crate row, "an agent reads by looking" grew
  the play sentence), HELP text.

**Next step**: phase 2 — a sanctioned interactive kind (a `run` block whose output mounts
live inside its node), executed only under play/run inside the same confinement story; then
phase 3 ergonomics (APNG assembly of frames, richer per-kind action vocabulary). Nothing
committed this wave; `/code-review` first.

## This wave, part 14: custom node size verified pixel-exact, and the agent-workspace question answered

**"When you or I custom size a node and you view it do you get the custom size rendered
pixel for pixel properly? Can you ensure?"** — verified yes, at every layer, and pinned:

- **Renderer**: `render::board::node_html` writes the stated box straight into the node's own
  style (`left/top/width/height`, board.rs:1052) — nothing is measured, content longer than
  the box scrolls inside it. The only sizing above the node is the canvas's *uniform*
  `scale()` from `fit` (clamped 0.1–1.5), so stated proportions are drawn proportions on
  every surface; physical pixels are stated × fit scale, 1:1 when the arrangement fits the
  column, and the surface's ⌘/pinch zoom reaches true 1:1.
- **Surface**: a plain view never resizes a node — `growOutgrownNodes` has exactly one call
  site, `apply()` (edit.js:946), the after-save path.
- **Pinned**: new test `a_custom_sized_node_is_drawn_at_exactly_its_stated_box` (html.rs)
  uses `x=-17 y=3 w=333 h=127` — numbers that are nobody's default, which the existing pin
  (`w=280`, coincidentally the default width) could not distinguish from "default applied".
- **Gates**: doc-core 266+2+5 green, clippy clean, fmt applied. Only a test added — no
  behavior changed, other crates untouched.

**The second ask — nodes agents can act on (scroll a view, press space to test a character's
jump) and watch frame-by-frame as video — was assessed, not built**: it needs a live CDP
session (doc-shot today is one-shot `--screenshot`, no input dispatch, no screencast) and a
sanctioned interactive kind (render::escape strips scripts; "reading never executes" means
playing must be an explicit run-shaped command). Phased design delivered in-conversation:
1) `dx play` — CDP input + frame capture over the rendered doc as-is; 2) a confined
interactive kind executed only under play/run; 3) `dx_play` MCP returning frames. Awaiting
the user's go-ahead and phase choice.

**Next step**: user picks a phase of `dx play` (or declines); nothing from this part is
committed beyond the one test.

## This wave, part 13: the site plan redesigned panel-by-panel, not as one document

**"The website in the example site document is pretty terrible… render out just the board or
individual nodes so that you can actually test how each view looks"**, then: **"generate an
actually beautiful site design and layout that could be used to showcase how people will
actually use DX to help guide agents."** `examples/example_site_plan.dx` rebuilt by rendering
each panel in isolation (`dx png --section <id>` on a hidden-stripped scratch copy, plus
magnified crops of the board) and fixing what each one showed:

- **The canvas was the killer**: 1220px of stated boxes fit into a ~680px board box → every
  node drew at ~0.56×, 10px wireframe type became 5–6px specks. Canvas is now 800 wide
  (2-col thinking rows of 380, a 3-screen strip of 240s), so nodes render at ~0.85×.
- **The `build` node was broken on dark theme** — its markup had no paper of its own, so it
  rendered page-ink-on-dark and read unfinished. New `.ship` class: same bone paper as the
  wireframes, because it is the same page.
- **Wireframes redesigned as panels**: layered radial+linear gradients that read as
  photographs instead of flat stretched fills, card counts (`seventeen frames`), Georgia
  lede on the booking screen, consistent bar/foot furniture. Palette tile now carries the
  exact hex values under each swatch — a spec an agent types from, which is the point.
- **Copy reframed around guiding agents**: lead and closing now say it straight — a person
  sketches the plan, `dx_read` hands an agent the same panels as pages, the checklist is the
  work queue it ticks with `dx check` as it lands.
- **Every node sized to its content** (scrollbar slivers and clipped footers found by crop
  at 170%), then text-heavy nodes given headroom per part 12's WebKit lesson — Chromium
  measurements alone are not enough for prose nodes.
- **Engine bug found, not yet fixed**: `dx render`/`dx png --section <board-id>` renders
  every node as `no block named '<id>'` — section narrowing drops the rest of the document
  before the board resolves its references, violating "resolved from the document it sits
  in" (nav's own rule). This is exactly the "render just the board" ask, and it fails today.
- **Validated**: full-page `dx png` in dark and light themes, all 10 nodes fit with no
  scrollbars, all 8 edges land with legible labels; `dx fmt` idempotent on the file
  (canonical as written); `dx outline` clean. No Rust touched, so the cargo/clippy/fmt
  gates stand as part 12 left them.

**Next step**: fix section-scoped board resolution in `doc-core` (board/nav should resolve
against the whole parsed document, not the section slice), with a test pinning it; then
re-check the plan in the real DX.app per part 12's discipline.

## This wave, part 12: validated against the real WebKit app, not just Chromium — two more real bugs found

Part 11's fix was checked with `dx png` (Chromium, via `doc-shot`) and called done. The user
pushed back with a real DX.app screenshot: **"I want actual views of the app... it can be a
lot cleaner and properly sized for views, correct colors for arrows and as close to zero
overlap as mathematically possible."** That screenshot showed clipped text in several nodes
that Chromium never reproduced — the gap between "I rendered it once, headlessly" and "I
looked at the thing the user actually opened" was exactly where the remaining defects were.

- **Node sizes were guessed, not measured** — `w`/`h` in part 11 were eyeballed. Rebuilt them
  properly: rendered the real document in a real browser (`claude-in-chrome`, not `dx png`),
  cloned each node's `.dx-board-node-body` into an off-screen sibling with `height:auto` at
  its real column width, and read `scrollHeight` — the same measurement "double-click the
  corner to fit" does interactively, done for all 10 nodes at once. Every node's stated `h` is
  now that measured natural height plus a real margin, and boxes are no longer forced into
  uniform row heights — each column stacks to only what its own content needs (`sitemap`
  alone is wider (320) and taller (390) than its row-mates, since nothing sits below it), which
  is what "close to zero overlap as mathematically possible" means for a stated-box format:
  not zero *margin*, but no box larger than it has to be to hold what's in it and clear its
  neighbours. `board`'s declared viewport `height` was also recomputed from `render::board::fit`'s
  own formula instead of guessed, so the canvas no longer sits in an oversized box with dead
  space beneath it.
- **Chromium measurement was not enough — WebKit needed more room.** Sized against the
  Chromium numbers alone, a fresh DX.app screenshot (rebuilt bundle, real `.dx` opened through
  LaunchServices, `screencapture`) still clipped `audience` and `brief` by about a line each:
  WebKit's serif/mono metrics run measurably taller than Chromium's at the same width. Bumped
  the safety margin from a thin buffer to +50% over the Chromium measurement across every
  text-heavy node; re-verified live and every node now clears its content with room to spare
  in the actual app, not just the headless renderer.
- **A real cross-engine CSS bug, not a sizing problem**: in WebKit, a checklist item whose text
  wrapped to two lines could tear the mark itself apart — `[` on one line, `]` on the next,
  each baseline-aligned to a different line of its sibling flex item. `.dx-checklist .dx-mark`
  had no `white-space: nowrap`/`flex-shrink: 0`, so under `align-items: baseline` WebKit was
  willing to let the 3-character mark shrink and break internally when its flex sibling wrapped.
  Fixed in `render::theme` (`.dx-checklist .dx-mark { white-space: nowrap; flex-shrink: 0; }`)
  — a real defect in every checklist this format renders, not specific to the example; found
  only because a real checklist item was long enough to wrap in the real engine that has the
  narrowest tolerance for it.
- **Arrowhead colour re-confirmed live, in WebKit**: cropped a zoom of the `home`→`menu`
  "browse" arrow from the real DX.app screenshot — head and stem are the same muted tone.
  Part 11's fallback CSS holds where it was actually meant to matter.
- **Gates**: cargo **630**/630, clippy clean, fmt clean, `./editor/build.sh` rerun, node
  **34/34** + **33/33**, all re-run after the `.dx-mark` fix.
- **Validated live, twice, against the actual app**: `packaging/build-app.sh` rebuilt,
  installed via the fresh bundle's own `dx setup` (`cmp`-verified against
  `/Applications/DX.app`), a stray running instance quit first each time. A scratch copy of
  `example_site_plan.dx` synced fresh (`dx sync` in `/tmp/dx_view_pad`) and opened via
  LaunchServices (`open <pointer>.dx`) so the app was reading real store content, not a stale
  cache. `screencapture` + page-down, twice — before and after the `.dx-mark` fix — confirms:
  no clipped text anywhere in either half of the document, no node overlapping another, and
  the previously-broken checklist item now wraps with its mark intact.
- **Lesson for next time this file is touched**: `dx png` (Chromium) is fast for layout
  iteration but is not the product's actual host CSS engine (WKWebView/WebKit is, for DX.app —
  which is what a person actually opens). Treat a Chromium render as a first pass, and check
  the real app before calling a visual fix done, the same way the format's own quality bar
  treats `cargo test` as necessary but not sufficient without live validation.

**Next step**: none owed — nothing in this wave has been committed; review the diff
(`/code-review` first) before committing.

## This wave, part 11: the arrowhead matches its stem everywhere, and the planner became a real site draft

Two asks: "Fix the arrowhead color to the same as the arrow stem color," and "Rewrite the
planner to be an `example_site_plan.dx` file and actually design a site, think of figma, the
main purpose of this document is to draft an incredible site really quick."

- **Arrowhead colour** (`render::theme`) — the marker's fill was `context-stroke` alone
  (`render::board::edges_svg`), which recolours the head with whatever the path's stroke is
  doing (default, hover, picked) **only in engines that understand the keyword**. WebKit (and
  any renderer without support) treats the declaration as invalid and drops it, so the head
  fell back to the marker's own default — black — while the stem carried the theme's
  `var(--dx-faint)`, and a picked/hovered edge's stem recoloured while its head stayed black.
  Fixed with the standard two-declaration CSS fallback: `.dx-board-edges marker path,
  .dx-board-edges marker circle { fill: var(--dx-faint); fill: context-stroke; }` — an engine
  that cannot parse the second declaration keeps the first, so the head always starts at the
  same colour as the line's default stroke; an engine that can, still gets the live recolour
  the board always had. No Rust logic changed, so `board.rs`'s own `context-stroke` assertion
  still holds (the CSS layers on top, doesn't replace the SVG attribute).
- **Found and fixed in passing**: the repo root had been `dx sync`-adopted again — a stray
  `examples/planner.dx` **pointer** plus a root `.doc/repo.dxcp`/`index.db`, the exact mistake
  CLAUDE.md's "do not run `dx sync` at the repository root" warns about (content recovered
  through the store first via `dx text`/`dx source`, then the store and the pointer both
  removed).
- **`examples/example_site_plan.dx`** replaces `planner.dx`: a small coffee roaster's whole
  site drafted on one `::board`, laid out as a clean three-row grid — foundations (a colour
  and type style tile, the brief, an audience checklist, the sitemap) feeding down into three
  real wireframes (home, menu, visit — actual `::svg` mockups, not placeholders) that flow
  left to right (`to=…:r-l:browse` / `:visit`), feeding down into the home page's real
  markup (`::html`), a success-metric quote, and the launch checklist. Every row is
  column-aligned so the vertical feeds (`b-t`/`t-b`) run straight and unobstructed — no
  obstacle-bending needed, which is what makes it read as an intentional grid rather than a
  stress test of the router. Found and fixed while writing it: the `build` node's markup used
  `<header>`/`<nav>`, neither on `escape::ALLOWED_ELEMENTS` — rewritten with `<div>`/`<span>`,
  both already allowed, rather than touching the allow-list. `README.md`'s board section
  updated to point at the new file and describe what it now shows.
- **Gates**: cargo **630**/630, clippy `-D warnings` clean, fmt clean; `./editor/build.sh`
  rerun (theme.rs is doc-core); node **34/34** github + **33/33** vscode.
- **Validated live**: `dx render`/`dx png` on `example_site_plan.dx` — all 10 nodes present,
  no `dx-board-missing` sentence, the 9 designed edges (plus the 2 labelled ones) all drawn;
  a cropped zoom on the `home`→`menu` "browse" arrow confirms the head reads the same muted
  grey as the stem, not black.

**Next step**: none owed — nothing in this wave has been committed; review the diff
(`/code-review` first) before committing.

## This wave, part 10: bigger arrowheads, a live HTML node, and a node's box typed instead of only dragged

Two screenshots of `planner.dx`'s board: arrowheads were "just a bit too small", and the
`build` node showed folded code instead of the rendered markup the wireframe SVG node
already showed. Then, mid-turn: "allow setting a node to a specific size via an agent or
right clicking and opening a typing menu."

- **Arrowheads** (`render::board::edges_svg`) — the marker's `markerWidth`/`markerHeight`
  went `7`→`11` against the same `viewBox="0 0 8 8"` and `refX/refY`, so the head is ~43%
  bigger with nothing else about the edge (position, curve, tether dot) touched.
- **The `build` node now renders live**, not folded — it was `::code lang=html`, and code
  always folds behind its label **by design** (non-negotiable per CLAUDE.md: a document
  opens as what it says). The right primitive already existed and is used one node over:
  `::html`, the same sanitized-inline-markup renderer the `wireframe` SVG node uses.
  Changed `examples/planner.dx`'s `build` node from `::code lang=html` to `::html`. One gap
  found along the way: `<main>` was not on `escape::ALLOWED_ELEMENTS` (a plain semantic
  landmark, exactly as safe as the already-allowed `<section>`) — added, so the markup
  keeps its wrapper instead of being unwrapped.
- **`dx board --place` used to default an omitted `--x`/`--y` to `0`** — so resizing a node
  with only `--w`/`--h` silently teleported it to the origin, the actual blocker behind "set
  a node to a specific size via an agent" (`w`/`h` already had the right behavior: `0` keeps
  the line's own). Fixed at the engine (`edit::board_place`/`place_into`, `x`/`y` now
  `Option<i32>` — `None` keeps what the line says, mirroring `w`/`h`'s own `0`-means-keep
  convention with a real optional instead of a magic number since `0,0` is a legitimate
  coordinate). `doc-wasm`'s `board_place` wrapper always passes `Some(x), Some(y)` — the
  surface's drag/resize always measures concrete numbers, so nothing there changes. CLI's
  `run_board` no longer defaults a missing `--x`/`--y` to `0`; docstring updated. 5 new
  tests (2 engine, 1 CLI regression pinning the exact bug, 2 mirror cases).
- **Right-click a node for a typing menu** (`edit.js`) — `onNodeContextMenu` +
  `openSizeMenu`/`closeSizeMenu`: a small `.dx-menu`-styled card (new `.dx-size-menu` CSS)
  with four number inputs (`x y w h`) pre-filled from the node's measured box, `set` or
  Enter applies through the *same* `arrange()` → `board_arrange` → settle path a drag
  already uses (no new engine call), Escape or an outside click cancels without writing
  anything. Suppressed while the right-click lands inside the node's own currently-open
  editing field, so copy/paste there still gets the browser's native menu.
- **Gates** — cargo **630**/630 (was 627; 3 new: 2 engine `board_place` cases + 1 CLI
  regression pinning the exact `--x`/`--y` bug), clippy `-D warnings` clean, fmt clean;
  node **34/34** github + **33/33** vscode
  (unchanged — no wasm-boundary or `.d.ts` signature changed); `editor/build.sh` rerun
  (both wasm engines current after the `escape.rs`/`board.rs` edits).
- **Validated live**: static-rendered `planner.dx` in Chrome — zoomed screenshot shows the
  visibly larger arrowhead; the `build` node shows "Bound by hand" / the image alt text /
  price line / link, live, not a fold. `dx board planner.dx plan --place brief --w 500 --h
  250` (no `--x`/`--y`) left `x=0 y=0` and only changed the box, confirmed by `dx text`.
  Right-click menu driven in a from-scratch Chrome harness (real no-modules wasm +
  real `edit.js`/`edit.css`, in-memory host wired straight to the wasm engine, no host
  stub for the interaction under test): right-clicking a node opened its own menu
  pre-filled with its real `x y w h`; typing `w=500` and clicking `set` wrote
  `x=0 y=0 w=500 h=170` (position kept) and visibly re-settled the board; right-clicking a
  second node showed *its* distinct values; Escape closed without writing anything.
- **Installed** — VSIX repackaged + installed (surface byte-identical in
  `~/.vscode/extensions/dx.dx-documents-1.0.0`); DX.app rebuilt via `build-app.sh`, the
  running instance quit, reinstalled via the fresh bundle's own `dx setup` — binary and
  surface both byte-identical to the built bundle, `dx doctor` prints no stale line.

**Next step**: none owed from this wave — reload the VS Code window (picks up the fresh
extension) and relaunch DX.app by hand if it's wanted open again. Nothing in this wave has
been committed; review the diff (`/code-review` first) before committing.

## This wave, part 9: the document's CSS applies everywhere, and mermaid became a board

**"Fix the HTML/CSS renderer so it works globally… it seems to be working in some places but
not in ::board//::graph blocks."** Then, asked about `::graph`: *"::mermaid is not a block
because we have a better more interactive version of mermaid, mermaid blocks should be
autoconverted… ensure that arrows are never overlapped by boxes… if they cross they must cross
minimally with the most amount of perpendicular possible. Never overlap boxes if possible."*
CSS was chosen **on everywhere, no exceptions**, which rewrites the old CSS Safety Contract.

- **A document's CSS always applies** — `document_css` is gone as an option, a CLI flag
  (`--doc-css`), a wasm parameter, and a VS Code setting. Two defects were behind the report:
  the flag was off everywhere (DX.app ran plain `dx render`), *and* `html()` returned early for
  a fragment before the CSS was ever attached — so every editing surface, which re-renders
  fragments, dropped it. The `<style data-dx-document-css>` now leads `.dx-doc` itself, so page
  and fragment carry identical bytes and a host that swaps `.dx-doc` swaps the dress with it.
  Turning it on globally made `escape_style` load-bearing, so it was brought up to the bar the
  inline `style="…"` path already held: remote `url(…)` is blanked (a beacon that fires on
  render), `@import`/`expression(`/`behavior:` neutralized, `data:` image artwork kept.
  `::stylesheet` still fetches — that is its stated purpose — but only for relative or
  `http(s)` hrefs. `::style media=` now works, as the contract always claimed.
- **A mermaid block is a board** — `format::mermaid` reads flowchart source at parse time and
  `format::layout` arranges it into a `::board` plus one hidden block per node. Labels, edge
  direction, and edge labels survive; source it cannot read (sequence, Gantt, unknown dialect)
  is left exactly as written. `examples/block-reference.dx` is migrated and its fixtures
  regenerated — that is the round-trip diff to review. Edge labels are new in the board format
  (`to=b:r-l:on%20failure`, percent-encoded, absent when unset, so no existing line changes).
- **Edges are never hidden, and boxes never overlap** — `clearest_sides` scores every side pair
  (a box across the run outweighs any number of crossings; a shallow crossing costs more than a
  square one; the facing pair breaks ties), then `controls` bends the cubic's handles square to
  the line until it clears every box it does not join. `edit::shove` replaces settle's
  always-downwards drop with the shortest escape past the nearest border, never onto a negative
  coordinate, downwards on a tie. All mirrored in `edit.js`.

**Validated:** `cargo test --workspace` 627 passing, `cargo clippy --all-targets` clean,
`cargo fmt --check` clean; `node --test editor/github/test/*.test.mjs` 34 passing and
`editor/vscode/test/*.test.mjs` 33 passing after `./editor/build.sh` rebuilt both wasm engines.
Rendered proof at `dx png`: flowchart reads top-down, `yes`/`no` sit on their curves, the loop
back-edge clears every box, and the document's own CSS dresses both the raw `::html` and the
board nodes.

**Fixed along the way:** ranking through a cycle stretched a six-node chart to ~1500px tall
(`back_edges` now excludes loop-closing edges); node boxes were sized from a too-narrow
character width and clipped their labels (metrics now read off `render::theme`'s own padding).

**Next:** the surface can move an edge label but not yet *write* one — `dx board --link` takes
no `--label`, and there is no click-to-edit on a label. That is the one gap in this part.

## This wave, part 8: the board's geometry is data, not a guess

Four asks about the board: connect from any edge to any edge, links that look like Blender's,
fit the whole graph from the start, and stop nodes overlapping (grow downwards) — then, on
review, **"don't estimate, fix html rendering, and allow agents to easily reshape and link
nodes properly."**

- **A node's box is stated** — the node line is now `- id x= y= w= h= to=` (`h` defaults to
  180, additive: an old line reads back with the default and reformats to the same box). The
  renderer draws the node at exactly that box (`left/top/width/height`) inside a
  `.dx-board-node-body` that scrolls what does not fit, so the rectangle the engine does its
  geometry against **is** the rectangle the browser lays out. The height *estimator* written
  earlier in this wave (wrap arithmetic, `viewBox` ratios, a safety margin) is gone — every
  consumer works from the same four numbers.
- **Overlap is the engine's** — `edit::settle` holds the nodes just placed and drops anything
  they cover to 28px below them, in reading order, downwards only (a board is a column, not a
  wall). It runs inside `board_place` **and** `board_arrange`, so `dx board --place` cannot
  leave a node buried any more than a drag can. The surface no longer computes displacements.
- **Any edge to any edge** — `to=steps:b-t` pins the two sides a line was drawn between
  (`l r t b`, `.` for unpinned; a bare `to=steps` still takes the facing pair). Every node
  carries four edge strips; dropping on a node lands on *its* nearest side. Edges sharing a
  side spread evenly along it, ordered by where their other ends are (`render::board::anchors`,
  mirrored in `layoutEdges`), each tethered at the source and pointed at the target. Only a
  *pinned* side reaches the page (`data-from-side`), so an unpinned edge re-routes as the board
  moves. Unpinned sides now pick the **longer axis, vertical winning ties**.
- **Fit on both axes** — `render::board::fit` (and `fitView`) scale to show everything, up to
  1.5×, centred where it fits and pinned to the top where it does not. A board re-fits on
  every change and on a column resize until the reader pans or zooms — which is now
  **⌘/Ctrl+wheel or a pinch**: a plain wheel scrolls the page, because a board lives in a
  document (found by driving it: the board ate page scroll).
- **For agents** — `dx board --place … --w --h`, `--arrange "id,x,y,w,h …"` (a whole board in
  one edit), `--link … --from-side --to-side`. Same wire through wasm
  (`board_place`/`board_arrange`/`board_link`), the VS Code host, and `Engine.swift`.
- **For humans** — corner reshapes both ways; double-clicking the corner fits a node to its
  content; after an edit, a node whose block outgrew it grows (never shrinks, never on a plain
  read — reading still writes nothing).
- **Demo** — `examples/planner.dx` is now a website being designed on a board: brief, audience,
  voice, sitemap, ink, an SVG wireframe, the markup, a measure, and a launch checklist, in a
  vertical spine with a side column, every box sized to its content.
- **Gates** — cargo **235+231+…** all green, clippy `-D warnings` clean, fmt clean; node
  **34/34** github + **33/33** vscode; `tsc` clean; both wasm engines rebuilt
  (`editor/build.sh`). `tests/fixtures/showcase.input.dx` had drifted from `examples/showcase.dx`
  before this wave (different `latency_ms` data and re-run chart output); the example was
  copied over the fixture to make the suite green again.
- **Validated live** (Chrome, real pointer input, real no-modules wasm + real surface +
  in-memory host over the actual `planner.dx`): board opened fitted at `scale(0.6)` with 9
  nodes and 36 edge strips; dragging measure's **left** edge onto build's **right** wrote
  `to=build:l-r`; dropping `voice` on `audience` left voice where dropped and cascaded
  audience → ink → measure down by 28px each; double-clicking `ink`'s corner grew it to fit;
  double-clicking empty paper added `node-1` with a full box and opened its field; clicking
  inside a node opened the decorated field in place; a plain wheel scrolled the page.
- Docs updated in the same edit: CLAUDE.md (the board section rewritten around the stated
  box), README.md (a "The board" section with the agent commands), the format contract (the
  node line, `h`, and the side suffix).

**Next step**: reload the VS Code window and reopen `examples/planner.dx` in DX.app (neither
was rebuilt/installed this wave — VSIX repackage + `packaging/build-app.sh` are the remaining
install steps), drive the board by hand, then `/code-review` and commit the accumulated diff.

## This wave, part 7: the boxes tick, and the charting reads

Two asks after driving `examples/planner.dx` by hand: clean up the charting and planning,
and make the checkboxes actually clickable in the boxes.

- **Ticking a box, engine-first** — `edit::toggle_check(source, id, item)` flips one `[x]`/
  `[ ]` marker by position and returns `(source, checked)`; every other item and block comes
  back byte-identical. Refuses a non-checklist and an out-of-range position with sentences.
  The renderer states which item each mark is — `data-check="N"` on `.dx-mark`, a fact like
  `dx-runnable`, not an affordance — so no surface counts boxes itself.
- **Reached everywhere** — wasm `toggle_check`; CLI `dx check <file> <id> --item N` (command
  row + flag table + documented-flags guard + HELP + file-level test); vscode `check` op
  through the wasm + `WorkspaceEdit`; `Editor.swift`/`Engine.swift` through the bundled
  `dx check`; `preview.ts` and the DX.app attach script both grew the bridge row.
- **Surface** — `dressChecks` turns marks into `role=checkbox` controls **only when the host
  offers `check`** (the run-control rule), `onClick` ticks and stops (it does not also open
  the list — ticking off is what a checklist is *for*), Space/Return tick a focused box. No
  new ink: the box a reader clicks is the same `[ ]` that was on the page, taking the page's
  own colour under the pointer. Works identically inside a board node.
- **Charting cleanup** — board edges were drawn right-edge→left-edge unconditionally, so any
  arrangement that was not strictly left-to-right hooked backwards and swooped across the
  whole board (visible in the PNG of the old `planner.dx`). Now one rule, stated in the two
  places that draw an edge (`render::board::sides` statically, `edgeCurve` in the surface
  against measured boxes): leave the side of the source facing the target (centres compared),
  arrive on whichever side of the target is nearer to where it left. Neither half needs a
  node's height. Every edge ends in an **arrowhead** (an SVG `marker`, id carrying the
  board's own id so two boards keep two, `fill="context-stroke"` so a picked edge's head is
  picked too); `.dx-board-edges > path` is now a child selector so the head keeps its fill.
  `examples/planner.dx` rearranged to read as a plan — edges point forward, and it says the
  boxes are clickable.
- **Gates** — cargo **592**/592 (7 new), clippy `-D warnings` clean, fmt clean; node **34/34**
  github + **31/31** vscode (3 new: tick/untick across the boundary, every box names its
  position, refusals as thrown sentences); `tsc` clean; `swiftc -typecheck` clean; helpers
  `lint` clean on every file touched; `editor/build.sh` rerun (both engines), surface copies
  byte-verified.
- **Validated live** (headless Chrome, CDP `Input.dispatchMouseEvent` — real trusted input;
  real no-modules wasm + real surface + in-memory host): 3 marks dressed as checkboxes,
  clicking the second box **inside the board node** made exactly one `check steps 1` call,
  wrote `[x] wire the save action`, ticked 1→2, and **did not open the editor**; Space on a
  focused box ticked it back and the source came out byte-identical to the file; opening the
  checklist for writing and pressing Escape left all 3 boxes still live (the restore path
  keeps them — checked, not assumed); edges measured `goal->steps` straight and
  `steps->sketch` out the left into the target's near side, both with `marker-end`, marker
  fill computing to `context-stroke`.
- **Installed** — VSIX repackaged + installed (surface byte-verified in
  `~/.vscode/extensions/dx.dx-documents-1.0.0`); DX.app rebuilt and installed via the fresh
  bundle's own `dx setup`, `/Applications/DX.app` binary + surface `cmp`-identical; a running
  DX.app was quit first. `~/.local/bin/dx` answers `dx check`.
- Docs updated in the same edit: CLAUDE.md (the box, the edge rule, the crate table, the
  operations list), README.md (`dx check` in the writing commands, and what a box does).
  The format contract needed no change — no new attribute; the `[x]`/`[ ]` spelling it
  already documents is exactly what flips.

**Next step**: reload the VS Code window (or reopen a document in DX.app) and drive
`examples/planner.dx` by hand — tick a box on the board, drag a node and watch the arrow
follow. If the feel is right, commit the accumulated diff as one unit (`/code-review` first).

## This wave, part 6: the board (a node editor on the sheet), auto-run on close, and the code tag line folded away

Three asks: an edited code block runs when its field closes; the `::code` editing header
goes away; and a new **interactive board** — a Blender-style node editor inside the
document for planning, whose nodes can be any dx block, in an iframe-like viewport that
fits/zooms/pans and can never push past the side of the page.

- **Format** — new `::board` kind (`height` attr, additive; body = one reference line per
  node, `- <block-id> x= y= w= to=a,b`, kept **verbatim** so unknown keys survive). Nodes
  are blocks of the *same document*, usually `hidden` (on the board, not in the flow).
  `model::Block.height`, parse/stringify/normalize arms, round-trip tests. Contract doc
  updated; `examples/planner.dx` added (canonical, renders 3 nodes).
- **Renderer** — `render/board.rs` owns the node-line grammar both directions (`nodes`/
  `parse_node_line`/`node_line`) plus `board_html`: clipping viewport (`height`, default
  480), canvas statically scaled to the column when nodes outgrow it, edge cubics under the
  nodes, each node rendered by the page's own `block_html` (missing ref → sentence; board-
  in-board refused). Stylesheet: hairline frame/nodes, `.dx-board-edges` is
  **pointer-events: none** — it spans the canvas and was swallowing clicks meant for nodes
  (found live; the editing CSS re-enables the strokes for click-to-pick).
- **Edit ops (engine, never JS)** — `edit::board_place` (move/resize/add; empty id → fresh
  `node-N` + hidden paragraph), `board_detach` (line + edges + the block, only when hidden
  and shown by no other board), `board_link` (idempotent, refuses self/missing ends);
  `set_block`/`replace_block` on a board create hidden paragraphs for lines naming missing
  blocks. `AUTHORABLE` += board. 10 new engine tests.
- **Reached everywhere** — wasm `board_place`/`board_detach`/`board_link`; CLI `dx board`
  (`--place/--add/--detach/--link --to/--unlink --x --y --w`, one action per call, flag
  table + HELP + guard test + 2 file-level tests); vscode `board` op through the wasm +
  `WorkspaceEdit`; Editor.swift/Engine.swift through the bundled `dx board`.
- **Surface** (`edit.js`/`edit.css`) — `dressBoards`: per-board remembered view
  (`boardViews`), fit-on-first-sight, wheel zoom at cursor (0.1–2.5×), drag-pan clamped so
  content never leaves the viewport, grip drag → `place`, right-edge resize (handle kept
  *inside* the border — the node clips overflow, an outside handle was unhittable), port
  drag → `link` (dashed probe curve), grip click picks a node / stroke click picks an edge
  → Delete detaches/unlinks (Escape lets go), double-click empty canvas → `add` + the new
  block's editor opens in place. `layoutEdges` re-anchors the engine's static edge guesses
  to measured boxes. Clicking a node's content edits it with the normal machinery; menu/
  ghost placement divides by `zoomOf(block)` so completions land right inside a scaled
  canvas. **Auto-run**: `save()` chains `autoRun(id)` after an *edited* commit/replace of a
  `dx-runnable` block (unchanged Escape runs nothing). **Code tag line hidden**: code opens
  body-only; ArrowUp at 0,0 reveals the header; `promote()` guarded so code text starting
  `::` is never lifted.
- **Gates** — cargo **585**/585 (30 new), clippy `-D warnings` clean, fmt clean; node
  **34/34** github + **28/28** vscode (4 new: board ops across the boundary, board render,
  refusal as thrown sentence); `tsc` clean; `swiftc -typecheck` clean; `editor/build.sh`
  rerun (both engines), surface copies byte-verified.
- **Validated live** (Chrome harness: real no-modules wasm + real surface + in-memory
  host; CDP input): drag → `- goals x=40 y=189 w=300`, zoom anchored, pan clamped,
  dblclick-add → `node-1` hidden paragraph opened for typing, `**bold**` decorated in a
  scaled node, port-drag wrote `to=mock`, Delete removed line+edges+orphan block, resize
  wrote `w=478`, code opened with **no** tag line, ArrowUp revealed
  `::code id=demo lang=python run`, ⌘Return after an edit logged `commit demo` → `run
  demo`, unchanged Escape ran nothing. Static render checked as PNG (`dx png`).
- **Installed** — VSIX repackaged + installed (surface byte-verified in
  `~/.vscode/extensions/dx.dx-documents-1.0.0`); DX.app rebuilt + installed via the fresh
  bundle's own `dx setup`, `/Applications/DX.app` binary + surface `cmp`-identical; stale
  DX.app process quit. `~/.local/bin/dx` answers `dx board`.
- **Incident, repaired in passing** — the repo root had been `dx sync`-adopted again
  (`.doc/repo.dxcp` + `examples/footprint-pair.dx` a pointer, `.doc/README.md` deleted):
  content recovered *through the store*, example restored as plain text, root store
  removed, README restored, fixture mirrors (`showcase`, `footprint-pair`) refreshed as
  the sanctioned two-file change.

(Superseded by part 7 above, which is the current resume point.)

## This wave, part 5: the field stays dressed, marks and all — plus per-block run and the menu fixed

Three complaints in one brief: code blocks could not be run from the page, the completion
menu's design/overlap was "mid", and editing should keep the block *looking* rendered while
showing all the characters that create it (`**bold**` bold, `**` visible), with ⌘/Ctrl
formatting chords. Mid-implementation correction from the user, honored throughout:
**no format logic in JS/TS** — the decoration is an engine render, not a JS tokenizer.

- **Engine — `doc-core/src/render/field.rs`, `render::field_html(text)`**: the editing-field
  view of source. Every character kept (the invariant caret math stands on — pinned by a
  `text_of(field_html(s)) == s` property test), marks wrapped `.dx-mark`, bold/italic/code
  styled as elements, links as `.dx-link-label`/`.dx-link-target`, list/checklist leads
  (`- `, `1. `, `[ ] `, `- [x] `) in the margin ink. Scanners **shared with `inline.rs`**
  (`split_code_spans`, `parse_link` made `pub(super)`) — one grammar, two emitters. One
  documented, accepted difference: emphasis straddling a link decorates on the page but not
  in the field (characters untouched). 7 new tests.
- **Reached everywhere**: wasm `field_html`; CLI `dx render --field=TEXT` (reads no file;
  flag table + documented-flags guard + test); `Engine.decorate` in DX.app.
- **Renderer marks runnables**: `dx-runnable` class on `run` code blocks (`html.rs`, test).
- **Surface rewrite (`edit.js`)**: prose kinds (paragraph/heading/quote/lists/checklist)
  now edit in a `contenteditable` `.dx-field-rich` holding the exact source, decorated per
  keystroke via `host.decorate(text)` — async with a ticket + text-equality stale-drop, caret
  restored by source offset (`offsetAt`/`positionAt`; all field access goes through
  `fieldText`/`spliceField`/`selection`/`setSelection`, which speak source offsets over both
  element kinds). Source kinds keep the textarea. Enter/paste handled manually (a CE's native
  Return invents elements); IME composition defers decoration; `.dx-writes-source` strips the
  dress. **⌘/Ctrl+B/I/E** toggle `**`/`*`/`` ` `` around the selection, **⌘/Ctrl+K** writes
  `[sel]()` caret-in-target, **⌘/Ctrl+Z/⇧Z** is a rebuilt undo/redo stack (decoration
  redraws defeat native CE undo; textareas keep native). **Run**: `dressRunnables` puts a
  `run` control in each `dx-runnable` summary (only when `host.run` exists), `running…`
  while working; answer applies like any save; `close()` strips the inert restored control.
- **Menu/ghost fixed**: `.dx-menu` used `var(--dx-panel)` — **never published by the theme,
  so the card rendered transparent over the document**; now `--dx-bg` + hairline + shadow.
  Placement by caret rectangle (`caretPoint`: selection rects for rich, canvas measure for
  textareas), clamped to the block, **flips above the caret when the viewport below runs
  out**. Ghost no longer fades the whole field to 0.2: textarea ghost mirrors with a
  transparent `.dx-ghost-mirror` + faint rest; rich ghost is one floating word at the caret.
- **Hosts**: vscode `decorate` (wasm, pure) + `run` ops (`dx run --only <id>` via CLI: save
  if dirty → run → read disk → if unchanged reply the error sentence, else sync buffer via
  `replaceAll`+save and reply `sheetHtml` — a failed *block* is content, its failure output
  is on the page). Editor.swift: `decorate` sync, `run` on a background queue (a run takes
  as long as the code; the main thread must repaint), same changed-file rule via `Data`
  compare. Bridge/attach rows added both sides.
- **Gates**: cargo **562**/562 (was 545; 17 new), clippy `-D warnings` clean, fmt clean;
  `editor/build.sh` rerun (both engines); node **34/34** github + **25/25** vscode (new:
  `field_html` across the boundary + every-char-kept); `tsc` clean; `swiftc -typecheck`
  clean; surface copies byte-verified in `editor/vscode/surface`; release `dx render
  --field` and `dx-runnable` class verified live against the built binary.
- **Installed**: VSIX repackaged + installed, surface byte-verified in
  `~/.vscode/extensions/dx.dx-documents-1.0.0/surface/`; DX.app rebuilt (`build-app.sh`) and
  installed via the fresh bundle's own `dx setup` — `/Applications/DX.app` surface + binary
  `cmp`-identical, `~/.local/bin/dx` answers `--field`. No DX.app process was running (a
  running one keeps the old attach script — relaunch if one is).
  Known accepted costs: DX.app spawns `dx render --field` per keystroke (small, and the
  stale-drop absorbs latency); decoration redraw briefly shows a typed char undecorated;
  VS Code may need a reload to shed a retained webview running the old surface.

**Next step**: drive it live in either host (reload VS Code window first): type `**bold**`
in a paragraph (dress + visible marks), ⌘B on a selection, complete `::code` (menu now
opaque, flips at viewport bottom), and click `run` on a runnable block. If the feel is
right, commit the accumulated diff as one unit (`/code-review` first).

## This wave, part 4: the block controls expand — tag line, retype, and completions

"When clicking a block it should expand the controls like it used to." Two Explore agents
mapped the deleted TS editor at `d2f0098^` (behavior + visuals, full specs in that session's
transcript): the real design was a **header/body expansion** — the block's `::kind attrs`
line editable above a per-kind styled body, retyping the block on commit, with ghost-text
autocomplete and a caret menu. Rebuilt on today's stack, engine-first:

- **Engine** (`doc-core/src/edit.rs`): `block_header` (the writer's own opening line, via
  the now-`pub(crate)` `stringify::block_header`) and `replace_block(source, id, header,
  body)` — `::` header parsed by the scanner's own grammar (`format::header_line_facts`),
  body applied via `set_body` (never re-scanned: a body containing `::end` keeps every
  byte); empty header = plain text through `parse` (markdown shorthand included, may yield
  several blocks). Id kept unless the header names one; unknown kinds and `output` refused;
  no replacement ever steals an existing id. 8 new tests.
- **Exposed everywhere**: wasm `block_header`/`replace_block`; CLI `dx source --header` and
  `dx set --header` (prints the focus id bare, like `dx insert`); flag table + HELP + the
  documented-flags guard test. Editor.swift `parts`/`replace` ops; vscode extension.ts
  `parts`/`replace` through the wasm engine + `WorkspaceEdit`; engine.ts interface rows.
- **Surface** (`editor/surface/edit.js|css`, whole-file rewrite around the part-1–3 core):
  tag line `.dx-header` (mono 0.78rem muted, hidden for paragraphs), promotion (`::` typed
  at the start of a plain paragraph lifts into the tag line), demotion (Enter on a non-tag
  header), seam navigation (ArrowUp/Left at 0,0 up; ArrowDown/Right at end down; Delete at
  header end joins), save routes: unchanged header → `commit`, changed → `replace`.
  Completions: `::` kinds, per-kind attribute keys (`ATTRS`), seed + learned values
  (`localStorage` `dx.autocomplete-history.v1`, cap 300), ghost overlay (`.dx-ghost`,
  line-end only) + menu card (`.dx-menu`, `--dx-panel`, max 20 items), Tab/Enter accept,
  Escape dismisses menu→field, Ctrl/Cmd+Space forces. `.dx-writes-source` dresses the body
  mono as soon as the tag names code/html/svg/mermaid, before any save.
- **Validated live in DX.app with CGEvents** (screenshots `s2–s12` in session scratchpad):
  heading expands with `::heading level=1 id=head` over a heading-size field; rewriting it
  `level=2` + ⌘Return re-renders at h2 and the file says so (id kept); paragraph opens with
  no tag line; blank-click → `::` promotes with ghost `::paragraph` + full kind menu; `co` →
  ghost `::code lang=`; Tab accepts; Return drops to body; ⌘Return lands
  `::code id=paragraph-5 lang=bash` in the store. Escape restores byte-exact.
  **Protocol notes**: activate `dx-app` and *verify frontmost* before every CGEvent (the
  terminal steals front — two stray events landed there); quit/relaunch DX.app after
  reinstalling (a running process keeps the old Swift attach script and the surface then
  falls back to body-only); synthetic DOM events are dead in this Chrome (the automation
  extension suppresses untrusted dispatch — page-world injection included), so the Chrome
  harness cannot click; CGEvents are the way.
- Gates: cargo **553**/553, clippy `-D warnings` clean, fmt clean; node **34**/34 github +
  **24**/24 vscode (4 new: header/replace across the wasm boundary); `tsc --noEmit` clean;
  helpers lint clean on all changed files. `editor/build.sh` rerun; VSIX repackaged +
  installed (byte-verified); DX.app rebuilt + installed via the bundle's own `dx setup`
  (byte-verified). CLAUDE.md editing section rewritten in the same wave.
- **Incident, resolved**: `examples/showcase.dx` was found adopted into a root store
  (`.doc/` at repo root — the thing CLAUDE.md forbids; came from the DX.app window opened
  on it earlier). Store content was byte-identical to HEAD; restored via `git checkout`,
  `.doc/` removed, github fixture regenerated, 34/34 again. Opening repo examples in DX.app
  and *saving* would adopt them — keep scratch copies for that.
- Known minor: ghost/menu measure with a canvas (`place()`); menu x can sit slightly off on
  proportional fonts. The fold-state wrinkle from part 3 stands.

**Next step**: click into blocks in VS Code or DX.app; if the feel is right, commit the
accumulated diff as one unit (`/code-review` first), then strip the unused `draw` bridges
from Editor.swift and vscode `preview.ts` (still unused by the surface).

## This wave, part 3: per-kind editing dress completed — markup source edits as a listing

The brief ("revert the missing specific styling for editing and seeing blocks as they are
edited") was read as the **styled field**, per `dx-editing-original-design.md` — no preview,
no second copy, no live redraw. A full per-kind survey in the Chrome harness (all 13 block
kinds, field's computed style vs the block's rendered style) found inheritance already
mirrors everything the block element carries — heading levels (31.2/22.4/18.4px w600),
quote (italic, muted, its own bar lent to the field), lists in their own column, code
(mono 13.28px, tab 2), mermaid (mono 12.8px muted) — because the field is the block's
*child*; the TS editor needed explicit `.block-src-type-*` mirrors only because its field
was a sibling. **The one real gap: `html`/`svg` fields edited markup as 17px prose serif,
tab 8.** Fix in `edit.css`: the existing code-field rule now also covers
`.dx-html.dx-editing .dx-field` and `.dx-svg.dx-editing .dx-field` — the page's one voice
for source, values mirroring `.dx-code pre` (mono 0.83rem/1.65, tab-size 2, pre-wrap).
TS-era pixel values (bordered code box, `--font-ui` headings) were deliberately **not**
ported: the constraint is mirror *today's* rendered look, and today's page has no boxes.

- **Validated live** in the Chrome harness (real `edit.js`+`edit.css` over the real wasm
  engine; MutationObserver/microtask waits — timers stall in a hidden tab): markup/drawing
  fields now mono 13.28px/21.9 tab 2 full ink in place, no box, editing hairline drawn;
  Escape restores byte-exact innerHTML on every probed kind; paragraph height unchanged
  open↔closed. Canonical-form probe covering all 13 kinds at
  `…/a3a7ef55-*/scratchpad/harness/probe.dx` (note: markdown shorthand mixed with `::`
  blocks parses to only the `::` blocks — a probe must be all one form).
- Gates: cargo test 545/545; node 34/34 github + 20/20 vscode; helpers lint clean on the
  changed files. Rust untouched (clippy/fmt not rerun — last green stands).
- Installed everywhere and byte-verified (`cmp`): `editor/vscode/surface/`, installed VSIX
  `~/.vscode/extensions/dx.dx-documents-1.0.0/surface/`, `/Applications/DX.app/Contents/
  Resources/surface/` (rebuilt via `build-app.sh`, installed via the bundle's own `dx setup`).
- CLAUDE.md editing bullet updated in the same edit (the one dress inheritance cannot supply).
- Known wrinkle seen on the way, pre-existing and left alone: editing a folded listing sets
  `open` on the `details`, and Escape restores content but not the fold state.

**Next step**: unchanged from part 2 — click into blocks in VS Code or DX.app; if the feel
is right, commit the accumulated surface diff as one unit (`/code-review` first), then strip
the unused `draw` bridges from Editor.swift and vscode `preview.ts`.

## This wave, part 2: the designed feel recovered from the TS editor

"Still not super clean, it's not like it was purposely designed to be." The purposeful
design is the deleted TypeScript editor (~5,000 lines, exhumed from `d2f0098^` and mapped by
two readers; digests in this session's transcript). Its core matches the restored surface
exactly — source-reveal in place, per-kind typography mirrored, caret at end on open, Enter
never splits, no hover chrome — but three Pages-like behaviors were missing, all
implementable with **zero host changes**:

1. **Blank paper is writable** (`blankClick` in `edit.js`): a click on the sheet itself
   (`event.target === .dx-doc`, margins excluded) inserts a paragraph after the last block
   whose vertical midpoint is above the pointer, via `commit(anchor, sameText, 'insert')`,
   and opens it. Two-step rule kept from the TS editor: a click that arrives with a field
   open (`openAtPointerDown`, set on mousedown capture) only dismisses. The sheet carries
   `cursor: text`.
2. **An empty paragraph is removed on close, never saved** (TS `commitBlockSrc` rule):
   `save()` routes a whitespace-only paragraph to `host.remove`; Return on an empty field
   is a no-op (no chains of invisible blocks).
3. **Spellcheck off everywhere** — source, not prose; no red underlines under `**strong**`.

- **Validated live in DX.app with CGEvents** (screenshots `look/4–9` in session scratchpad;
  window must be activated first — an activating click is consumed and drives nothing):
  blank-click below the last block opened a paragraph with hairline + caret, typed text
  saved on click-out (file adopted into store, 7→8 blocks), the click-out created nothing,
  a second blank-click opened an empty paragraph, Return kept it single, clicking away
  removed it (back to 8 blocks, `dx text` clean). 34/34 + 20/20 node suites; helpers lint
  clean; VSIX + DX.app rebuilt, reinstalled, byte-verified.
- CLAUDE.md editing bullet updated in the same change.
- Not carried over from the TS editor, deliberately: `::`-tag autocomplete with ghost text
  (current format has no `::` headers for prose kinds), Tab block-navigation + focus
  outlines (chrome the paper doctrine dropped), status messages, appearance controls,
  DX.app-side undo/redo (VS Code gets undo via `WorkspaceEdit`; a DX.app history is a real
  gap but a separate feature, not feel).

## This wave, part 1: the original editor restored — the field IS the block, nothing else appears

"Editing is still not what it was supposed to be when it was originally built and perfected."
The previous wave's hybrid (field in place + live preview beneath) was still not the original.
Reconstructed the actual c11a64bc surface from that session's transcript (one Write + Edits
replayed → 279-line edit.js, 92-line edit.css) and compared: the original replaced the
block's rendered content with a field holding its source — **the field is the block's child,
inheriting its type; no preview, no source note, no second copy of the block, for every
kind**. The "always rendering even when editing" brief (a307f64d) had been read as
"keep the block drawn and type beside it" — that arrangement is what the user rejected twice.

- `editor/surface/edit.js|css` rewritten to the original interaction, keeping the later
  plumbing that doesn't change the feel: no-reload commits (`{document, focus}`, one
  `.dx-doc` swap), `settling` so a click can't overtake a save, refusal `resume`, the
  extended `kindOf`/MULTILINE sets. Gone: `host.draw` calls, `stand`/`mirror`/`standsIn`,
  TYPE/BOX copying, redraw debounce. The one new wrinkle: editing a folded listing leaves a
  hidden stub `<summary>` (`.dx-fold-stub`, specificity beats `.dx-code-folded > summary`)
  because a `details` without a summary invents a default marker.
- Hosts untouched and byte-uncoupled (no `.dx-write`/`.dx-field` references in either).
  Their `draw` bridges (Editor.swift, vscode preview.ts) are now unused by the surface —
  contract says "a host may offer more"; remove them once the feel is blessed.
  `edit::preview_block` stays: it is `dx render --block --body`.
- CLAUDE.md editing sections rewritten to describe this (field-is-the-block, fold stub,
  preview_block no longer tied to a typing loop).
- **Validated live** in a Chrome harness driving the real `edit.js` over the real wasm engine
  (session scratchpad `harness/`, MutationObserver/microtask waits because the tab is
  hidden): all six kinds — field is the block's only child in the block's own computed type
  (17px serif prose, 22.4px/600 heading, italic quote, mono 13.28px pre-wrap code), Escape
  restores byte-exact innerHTML, typing adds nothing to the page, quote/code keep one rule
  (`::before` display none), fold stub hidden. Save on blur re-renders in place (probe node
  outside `.dx-doc` survived — no reload), Return inserts + focuses next paragraph, Backspace
  on empty erases and lands in the block above, a *refused* save shows the error note and
  resumes the field focused with the typed text, retry saves. helpers lint clean; 34/34
  github + 20/20 vscode node tests; VSIX repackaged + installed (surface byte-verified);
  DX.app rebuilt + installed via the bundle's own `dx setup` (surface byte-verified).
- **Not committed** — awaiting the user's look at the restored feel.

**Next step**: click into blocks in VS Code or DX.app; if the feel is right, commit, then
strip the now-unused `draw` bridges from Editor.swift and the vscode webview.

## Previous wave: every access point driven live, two stale installs refreshed

Every surface was exercised end-to-end with screenshots (session scratchpad `img/`), after the
full gates ran green first: **545 cargo tests, clippy `-D warnings` clean, fmt clean, 34/34
github + 20/20 vscode node tests** (`editor/build.sh` rerun first).

- **CLI**: every READ/WRITE/STORE/RUN verb driven in a scratch store — `new/append/insert/set/
  remove/fmt` round-trip, `sync` adopt + restore-from-pack (index deleted, reads survive), a
  dangling pointer errors with the `dx sync` sentence, `git-setup` + `git diff` shows document
  text, `png --pages/--section/--theme` writes real images. Checklist note: the editable body
  is `[x] item` lines — a Markdown `- [x]` prefix is treated as item text, by the format's rule.
- **Run + sandbox, live**: python/node/bash blocks all `ok`; an attack block got
  `network-blocked` and `write-confined` (`/tmp` write → `Operation not permitted`).
- **`dx serve`**: `/health` ok; bad `Host` → 403 (rebinding), foreign-`Origin` simple POST →
  403; `render_html`/`stylesheet` answer through the engine.
- **MCP**: all nine `dx_*` tools round-tripped (write → edit → run → read-as-images).
- **DX.app**: opened a store *pointer* via LaunchServices, typed on the page (CGEvents),
  ⌘Return committed — content changed on disk through the bundled `dx`. **Found stale** (binary
  predated today's HEAD): rebuilt `build-app.sh`, replaced `/Applications/DX.app`, re-verified.
- **VS Code**: **installed extension carried a stale wasm + surface** — repackaged, reinstalled,
  verified byte-current. Editing driven live: block redraws from the field while typing
  (preview), field is a sibling showing raw source, Escape discards, repo left clean.
- **github.com**: the extension is **not loaded in any Chrome profile** — the "Load unpacked"
  grant is the user's reserved step (`dx browser` prints it). The per-user extension payload
  wasm was also stale → `dx browser --from editor/github` refreshed chromium + firefox copies.
  Pipeline still proven end-to-end headlessly: fetched the real `.doc/repo.dxcp` GitHub serves
  for `RockyWearsAHat/dx-github-probe`, resolved `showcase.dx` by path through the shipped
  wasm, **digest verified**, rendered to HTML/PNG. (`pack_document` keys by *path*; resolve.js
  verifies the digest after.)

**Next step**: load the unpacked extension in Chrome (chrome://extensions → Developer mode →
Load unpacked → `~/Library/Application Support/dx/extension/chromium`), then re-check a blob
page and the private-repo claim in docs/github.md.

## Previous wave: full-history audit, five defects fixed, in-editor diff restored, everything committed

The whole git history (60 commits) was audited against the working tree: every design promise
(one engine, store resolution, sandbox, allow-list escaping, paper aesthetic, one install) was
re-verified with `file:line` evidence and the gates re-run — all survive. Three audit agents +
two implementer agents; findings and fixes:

1. **`dx search --limit` was refused** (HIGH — the new flag table missed it because `--limit`
   was documented nowhere). Row and HELP now carry it; guard test extended; two new tests
   (accepted + honored).
2. **Explicit-id refusal had a slugify gap**: `--id "My Id"` when `my-id` exists escaped the
   refusal and was silently renamed. `edit::colliding_id` now slugifies the candidate through
   the registry's own `slugify_heading` before matching; refusal names the taken id. Tests in
   both crates pin it, file byte-unchanged.
3. **The stale-engine check was a permanent false positive**: `build-app.sh` codesigns the
   bundled `dx`, so whole-file bytes never matched `current_exe()` even from the same build —
   the alarm always fired from a dev binary. `desktop.rs` now compares Mach-O **`LC_UUID`**
   (hand-parsed, thin + universal; any parse failure is "cannot tell", never "stale").
   Proven live: same link through a signature → clean; a genuinely older binary → stale line.
4. **`dx insert` gained `--id`/`--level`** (parity with `append`; both documented in HELP).
5. **One refusal path**: the duplicate `edit::authorable` pre-check in `run_append` removed;
   both verbs refuse through `doc_core::edit::insert_after` alone.
6. **Lost TS-era capability restored — in-editor `.dx` diff.** VS Code's git ignores
   `textconv`, so "Open Changes" showed two pointer lines. New "DX: Open Changes"
   (`dx.openChanges`): a `dx-text:` `TextDocumentContentProvider` resolves HEAD (via
   `git show`) and the working copy through **`dx textconv`** — the same driver git runs, no
   store logic in TS. Unresolvable side shows dx's own sentence, never empty. The working
   side follows unsaved edits. `editor/vscode/src/changes.ts` + `policy.ts` (CSP + nonce
   extracted and **pinned by test**: `img-src data:`, no `https:`, nonce 32-hex).
7. Cleanliness: `.gsh/` agent artifacts untracked + gitignored; CLAUDE.md test count trued
   (545); `erase()` failure note now lands block→note→field like the save path.
- **Validated**: `cargo test` **545 passing, 0 failed**; clippy `-D warnings` clean;
  `fmt --check` clean; `./editor/build.sh` rerun after the doc-core change; node suites
  **34/34 github, 20/20 vscode** (9 new); `tsc --noEmit` clean; `packaging/build-app.sh`
  rebuilt, installed via the fresh bundle's own `dx setup`,
  `/Applications/DX.app/Contents/MacOS/dx` `cmp`-identical, and `target/release/dx doctor`
  prints **no stale line** (the false positive is gone, and a real mismatch still says so).
- Known minor: `dx append --type widget` with no `--text`/`--from` reads stdin before the
  kind refusal (body gathered first). The vscode node tests import `.ts` directly and rely
  on node ≥23.6 type-stripping (v23.9 here).

## Previous wave: eight review findings fixed (append through one engine, refusals unified, no more silent staleness)

1. **`dx append` is `insert_after` now** — the hand-built Block in `run_append` is gone.
   `edit::Insertion` grew `id` and `level` (additive; `..default()` callers untouched), and
   the authorable-kind refusal lives once in `edit::authorable`. New behavior, on purpose:
   an explicit `--id` a block already answers to is **refused with a sentence** (matching by
   the document's own rule, so `#Intro` collides with `intro`) instead of the registry
   silently renaming; the file is left untouched. `insert`/`append` error text unified to
   "cannot add".
2. **One flag-refusal formatter** (`commands::refuse_unknown_flags`), used by dispatch,
   `dx serve`, and — new — `dx mcp`, which used to swallow every flag silently
   (`dx mcp --root /x` now errors naming "no flags").
3. **`require_code_for_attributes` uses `present()`**, so a trailing valueless `--lang`/
   `--deps` (parsed boolean) is refused on prose instead of slipping through.
4. **A stale installed DX.app says so.** `desktop::Outcome::StaleEngine`: when the running
   binary is not the one inside `/Applications/DX.app` (byte comparison, size first;
   "cannot read" is "cannot tell", never "stale"), both `dx setup`'s report line and
   `dx doctor` name the remedy — rebuild `packaging/build-app.sh`, run that bundle's
   `dx setup`. Verified live: `target/debug/dx doctor` prints the stale line; the installed
   app's own doctor does not. Copy semantics unchanged.
5. **A refused save no longer strands the editor** (`editor/surface/edit.js`): on
   `commit`/`remove` rejection the editing state is `resume`d — field kept with the typed
   text, re-focused, Escape/blur/retry alive again; `erase` re-attaches the field it had
   removed; opening another block closes an abandoned failed attempt instead of leaving two
   fields. No JS harness for edit.js — verified by code reading; all three host copies
   (vscode/surface, packaging bundle, /Applications) confirmed to carry the fix.
6. **Webview CSP matches DX.app**: `img-src data:` (dropped `https:` — a document must not
   reach the network on any host), and the CSP nonce is `crypto.randomBytes`, not
   `Math.random`.
- **Validated**: `cargo test` **537 passing, 0 failed** (7 new); clippy `-D warnings` clean;
  `fmt --check` clean; `./editor/build.sh` rerun; node suites **34/34 github, 11/11
  vscode**; `tsc --noEmit` clean; `packaging/build-app.sh` rebuilt, installed via the fresh
  bundle's `dx setup`, `/Applications/DX.app/Contents/MacOS/dx` `cmp`-identical to the
  build. Nothing committed.

## Previous wave: the stale DX.app rebuilt, reinstalled, and editing re-proven

"Blocks are not editable in DX.app" was reported. Root cause: **staleness, not a code
defect** — `/Applications/DX.app` and `~/.local/bin/dx` still dated Jul 30 19:19 (the
build before the edit round-trip and arg-parsing waves), while the current tree had never
been repackaged. The pending "rebuild `packaging/build-app.sh`" step from the last two
waves is now done.

- **Rebuilt**: `cargo build --release -p doc-cli`, `./editor/build.sh` (both wasm engines),
  `packaging/build-app.sh` (2.8 MB bundle, ad-hoc signed).
- **Installed**: `dx setup` run **from the fresh bundle's own binary** — running it from
  `target/release` leaves an already-installed `/Applications/DX.app` in place by design
  (`desktop.rs`: a bundle under Applications is never moved; only the *running* bundle is
  copied in over it). After the bundle-run setup: `/Applications/DX.app/Contents/MacOS/dx`
  is byte-identical to the fresh bundle (`cmp` clean), `~/.local/bin/dx` matches it, the
  surface files in `Resources/surface` are current, and `dx serve` restarted on the new
  binary. Behavioral proof of freshness: `dx insert --help` prints help without inserting.
- **Editing re-proven with real CGEvents** (Quartz `CGEventPost`, never System Events)
  against `/tmp/dxfix/welcome.dx` (a store pointer). Screenshots in the session scratchpad
  (`/private/tmp/claude-501/-Users-alexwaldmann-Desktop-DOC/68f79089-*/scratchpad/0*.png`):
  - click a paragraph → field appears in place, hairline rule, caret at end (`02`);
  - typing ` Typed by *CGEvents* now.` → block re-renders live, *CGEvents* italicised
    before any save, source written beneath (`03`);
  - Return → saved and re-rendered in place, next empty paragraph focused, **no reload**
    (scroll and layout intact, `04`); pointer digest changed on disk, file stayed one line;
  - typing then Escape → discarded: digest unchanged, text nowhere in content (`05`).
- Remaining gap: none new. The VS Code webview end-to-end drive and the store submissions
  (below) are still the open items. Nothing committed this wave.

## Previous wave: three CLI arg-parsing defects fixed

1. **`--help` never runs the command it was asked about.** `dx insert <file> --help` used to
   perform a real insert (a stray empty block, exit 0). `commands::dispatch` now answers
   `--help` with the help text before any command runs, and `dx serve`/`dx mcp` do the same
   in `main` since they bypass dispatch. (`dx --help` already exited 0; pinned by test.)
2. **A flag a command does not read is refused by name, never swallowed.** The dispatch
   table (`commands/mod.rs::COMMANDS`) now carries each verb's flags; anything else errors
   with the accepted list (`` `dx set` does not take --lang. It takes --text, --from``).
   `dx insert` gained `--lang`/`--run`/`--deps` (parity with `append`) via
   `edit::Insertion`, so a runnable code block can be authored mid-document; both commands
   refuse those attributes on a non-code `--type`, where the format would drop them.
3. **An insert never steals an existing block's id.** `doc_core::edit::insert_after` used to
   let the new block's natural name claim an existing id (old `code-3` silently became
   `code-3-2`, breaking nav targets and references). The new block now yields and takes the
   next free suffix; every existing id stays put. Same rule for heading-slug collisions.
- **Validated**: `cargo test` **530 passing, 0 failed** (10 new); clippy `-D warnings` clean;
  `fmt --check` clean; `./editor/build.sh` rerun (doc-wasm signature change) and node suites
  **34/34 github, 11/11 vscode**; all three defects re-driven against the release binary in a
  scratch store and confirmed fixed.
- **Next step**: done in the wave above (rebuilt, reinstalled, re-verified). The commit is
  still owed.

## Previous wave: review findings fixed (edit round-trip, kill_tree race, stale docs)

- **`edit::body`/`set_body` are now true inverses for lists — content and structure.** A
  list's editable text is the writer's own body lines (`stringify::list_lines`: indented
  `- text`), and `set_body` reads them back with the parser's own grammar
  (`parse_list_items` + `build_nested_list_structure`). One definition, two directions: a
  single `-`/`*` **followed by whitespace** comes off (`--verbose …` / `*emphasis*` stay
  verbatim, via the shared `strip_bullet_marker` inside `match_list_marker`), an indented
  line nests, and an unchanged save keeps the whole tree. Checklist bodies carry their
  `[x]`/`[ ]` markers and `set_body` reads them with `parse_checklist_line`, so ticks
  survive too (checklists cannot nest; their body is flat by the format's rule). Five new
  round-trip tests in `doc-core/src/edit.rs` pin all of it.
- **`kill_tree` really kills the tree now.** The attacks test's survivor marker was outside
  every writable grant, so it could never fail; moved into `$DX_SANDBOX` — and it immediately
  caught a real race (parent subshell runs its next command between its child's kill and its
  own). Fix in `doc-run/src/process.rs`: SIGSTOP the whole tree (two passes), then SIGKILL.
- Stale `dxz1`/LZSS comments reworded to the framed truth (`doc-wasm/src/lib.rs`,
  `doc-core/src/chunk.rs`, `doc-store/src/{pack,schema}.rs`); `AUTHORABLE` comment now names
  every excluded kind and why; last lib-code panic path in `compress::decompress` replaced
  with a slice pattern; dead `!SECRET_PATHS.is_empty()` removed; `escape.rs` comments state
  actual behavior, with a new test pinning that unterminated numeric entities
  (`&#106avascript:`) stay inert; doc-run env-mutating tests serialized via `crate::env_lock()`.
- **Docs brought back to true, in place.** `packaging/README.md` no longer claims the web
  view has JavaScript off (the truth: page under `script-src 'none'`, editor in its own
  `WKContentWorld`) or that `dx setup` writes the extension (it reports; only
  `dx browser --from` writes); Swift file count corrected; stale measured sizes re-measured
  (~313 KB wasm, ~0.4 MB extension). Hard test counts live only in `CLAUDE.md`'s quality
  bar now. `.gitignore` gained `/.doc/index.db*`. User-facing strings say `dx setup`, not
  `dx install`. The daemon no longer emits `access-control-allow-origin: null` for
  unrecognized origins (a sandboxed frame's Origin *is* `null`, so the literal was a real
  grant) — header omitted instead, two tests pin it. Smaller: `install.rs` folded into the
  shared `home`/`state` helpers, `browser.rs` excluded-family line no longer counts as a
  step, `policies.rs` field comments describe their own fields, unused binding out of
  `edit.js`.
- **Validated, full gate after the wasm rebuild**: `cargo test` **521 passing, 0 failed**;
  `clippy --all-targets -D warnings` clean; `fmt --check` clean; `./editor/build.sh` run
  (both engines current); node suites **34/34 github, 11/11 vscode** — one vscode
  expectation updated for the deliberate contract change (a list's editable text now
  carries its `- ` markers); `dx fmt examples/*.dx documents/*.dx --check` unchanged.
  (Daemon flake `no_other_origin_…` was rewritten by the ACAO fix; not seen since.)
- **Next step**: done — see the 2026-07-31 wave at the top (rebuilt and reinstalled).

## Previous wave: the page keeps rendering while you write on it

The brief was that dx should be consistently styled like a piece of paper and **always
rendering, even when editing**. Two things broke that, and both are fixed.

### 1. The block being edited stopped being rendered

Clicking a block replaced its rendered content with a field holding raw `.dx` source: a list
lost its bullets, a drawing became a wall of markup, `**strong**` became four characters.

- **`doc_core::render::block(document, id, options)`** — one block, exactly as the page carries
  it (the page's own `block_html`, the same options, the same `{{placeholder}}` values). A test
  asserts the one-block markup is a substring of the whole page's, so the two cannot diverge.
- **`doc_core::edit::preview_block(source, id, body, options)`** — that block drawn from
  characters that were never saved. It applies `set_body` to a parsed copy and throws it away,
  so it is a **read**: `Escape` after typing changes no file, and a test pins that.
- Reachable everywhere the engine is: `dx render <file> --block <id> [--body=<text>]` and
  `doc-wasm`'s `preview_block`. `Document::block_index` is now the one id-matching rule;
  `edit::find` is that plus the sentence naming the ids that exist.
- **`editor/surface/edit.js` decides between two shapes, structurally.** If the renderer adds
  any element the source did not name — a bullet, a strong word, a drawing — the block **stays
  drawn** and redraws from the field (debounced 90 ms), with the source written beneath it in
  the sheet's own marginal mono. Otherwise the field **stands in** for the block, borrowing its
  type and box from the live element via `getComputedStyle` — never a second copy of the type
  scale — so plain prose and a code listing edit exactly where and how they were set. The field
  is the block's *sibling*, because a `<div>` inside a `<p>` is not something a browser keeps.
- A block that already draws a rule down its margin (a quotation, a listing) lends it to the
  field instead of getting a second hairline beside it.

### 2. Every save reloaded the whole page

DX.app called `loadHTMLString` and the VS Code webview rebuilt its `html` on every commit — a
navigation per edit, which is why there was a scroll position to "restore" at all.

- `commit`/`remove` now answer with `{ document, focus }`, and `edit.js` swaps the one
  `.dx-doc` element. No navigation: no flash, no scroll jump, no reattaching, and `restore()`
  is gone from the contract along with `dxReady`/`dxRestore`.
- `Editor.swift` no longer holds a way back into its window; `DocumentViewer` loads once (and
  on ⌘R). The VS Code provider skips the refresh caused by its own `WorkspaceEdit` and still
  refreshes for anything else.

### Validated on this machine

- `cargo test` **514 passing** (was 511), `clippy --all-targets -D warnings` clean, `fmt
  --check` clean. `node --test` → 34 github + 11 vscode (2 new), 0 skipped. `tsc --noEmit`
  clean, `.vsix` packages, `dx fmt --check` unchanged across `examples/` and `documents/`.
- **DX.app driven with real CGEvents** (System Events' `click at` performs an accessibility
  press that never reaches the DOM — it is not a usable driver for this, and a claim of
  "clicked and typed" made through it means nothing). Screenshots at each step:
  - a paragraph with `**strong**` → stays rendered, source beneath it; typing `*this arrives
    live*` italicised it in place before the save.
  - a bulleted list → bullets stay; a fourth bullet appeared as the line was typed.
  - an `::svg` → the drawing stays drawn with its markup written under it.
  - plain prose, a quotation, and a code listing → field stands in, one rule, right column,
    indentation preserved.
  - Return → next paragraph created and focused, in place. Backspace on the empty block →
    removed, caret at the end of the block above. Escape → discarded, document unchanged.
  - **No reload, proven**: a probe appended to `document.body` (outside `.dx-doc`) survived a
    commit with its trace intact; and a save made far down `showcase.dx` left the page
    pixel-identical apart from the words that changed.
  - Light and dark appearance, and a document held in the store as a `~ dx1` pointer.

### Not done

**The VS Code webview still has not been driven end to end** — same reason as the previous
wave (Restricted Mode blocks custom editors, and a separate profile will not come to the
front). Its parts are verified: the wasm calls under `node --test`, `tsc` clean, the surface
packed into the `.vsix`. Open it by hand and check that a save no longer rebuilds the page.

`host.draw` in DX.app spawns `dx` per redraw. It is fast enough at 90 ms debounce on this
machine, but it is a process per pause in typing; if it ever isn't, `dx serve` is the door.
While an `::svg` body is mid-tag and renders to nothing, the last good drawing stays on the
page rather than blanking — deliberate, and worth knowing.

## Previous wave: folded code, a real sandbox, and a page you can write on

Three things were asked for, against a DX.app view that was otherwise right.

### 1. A document opens as what it says

`code_html` renders a code block as a CSS-only `details`/`summary` folded behind one faint
label (`python · run · numpy`); the `::output` it produced stays on the page. No script is
involved, which is what lets DX.app keep the document's own JavaScript off. `HtmlOptions::
collapse_code` is `false` for renders nobody can click — `doc-shot` (so `dx_read` still shows
an agent the listing) and `dx render --show-code`.

### 2. `doc-run` had no sandbox at all — now it has one

`sandbox.rs` was only a per-block *cache directory*. Blocks ran as the user, in the document's
own directory, with full write and network access, and Deno ran `--allow-all`. A `.dx` from
anywhere could read `~/.ssh` and post it.

- **`doc-run/src/confine.rs`** — Seatbelt (`sandbox-exec`, deny-by-default) on macOS,
  bubblewrap on Linux. Read anything but the credential stores; write only the block
  directory; **no network** for the block's own code. No boundary → the block does not run.
  `DX_UNCONFINED=1` overrides and stamps a line saying so into the output.
- **`plan.rs` split every language into fetch-in-`setup` / run-offline.** Rust and Go now
  build in setup and run the binary; Deno caches then runs `--cached-only` with narrow
  permissions; Python builds a venv in setup. Toolchain caches moved to a dx-owned directory
  because the reader's `$HOME` is read-only now.
- **`process.rs`** clears the environment down to an allow-list (a shell full of tokens is
  the most valuable thing on a dev machine) and kills the whole process tree on timeout.
- `sandbox.rs` → `workdir.rs`, which is what it always was.

### 3. Click a block and type

`editor/surface/{edit.js,edit.css}` is the one editor, copied into DX.app by
`build-app.sh` and into the extension by `editor/build.sh`. Click → the block becomes a field
holding its own source in its own type; Return saves and starts the next paragraph; ⌘Return
saves; Escape discards; Backspace on an empty block deletes it. Both hosts land on the new
**`doc_core::edit`** (four operations), which is also what `dx source`/`set`/`insert`/`remove`
are — so a person and an agent edit through one implementation. DX.app runs the editor in its
own `WKContentWorld` under a page CSP of `script-src 'none'`, so enabling JavaScript for the
window did not give the *document* any.

### Validated on this machine

- `cargo test` **504 passing** (was 467), `clippy --all-targets -D warnings` clean,
  `fmt --check` clean. `node --test` — 34 github-extension tests (including engine parity
  with `dx render`), 9 new VS Code engine tests.
- **The sandbox, attacked**: `doc-run/tests/attacks.rs`, 13 payloads, all blocked. Proved
  non-vacuous by running the identical document with `DX_UNCONFINED=1`: it then printed
  `WROTE FILE / REACHED NETWORK / READ SSH CONFIG` and planted the file; confined it exits 1
  with `Operation not permitted` and plants nothing.
- `dx run` on `examples/showcase.dx` (python + numpy, network install then offline run) →
  both blocks `ok`, output byte-identical to what was recorded before.
- **DX.app, driven for real**: clicked a paragraph, typed, pressed Return → text saved and
  `paragraph-3` created; ⌘Return on a second edit → saved (280→286 chars) and the block
  re-opened focused; Backspace on the empty block → removed. `welcome.dx` on disk stayed a
  one-line pointer throughout. Two UX defects found and fixed this way: an empty block had no
  clickable height, and an empty field collapsed to zero height.
- `dx doctor` now opens `code execution` with
  `sandbox   seatbelt — reads allowed, writes confined, no network`.

### Not done

**The VS Code webview editing was not driven end to end.** Its parts are verified — the four
wasm calls (9 node tests), TypeScript clean, `surface/` packed into the `.vsix`, extension
installs — but the bridge and focus restore were never exercised in a running editor:
automating VS Code's UI did not work here (Restricted Mode blocks custom editors, and a
separate `--user-data-dir` instance would not come to the front). **Open it by hand and check
Return, ⌘Return, Escape, and Backspace before trusting that surface.** The new `.vsix` was
installed into the real VS Code profile on this machine.

Also: the sandbox's credential list is a deny-list and so is best-effort by nature (the write
and network boundaries are not). A process orphaned by a block that exited normally is not
chased — it stays confined, but the timeout does not reach it.

## Previous wave: a `.dx` opens in Finder

`DX.app` was a delivery vehicle — a binary, the Safari extension, and a shell script that ran
`dx setup`. It is now also the Mac viewer: double-clicking a `.dx` opens a window showing the
rendered page.

- **`packaging/app/` — six Swift files, AppKit + `WKWebView`**, built into
  `Contents/MacOS/dx-app` by `build-app.sh` (`swiftc`, no Xcode project). It renders nothing
  itself: `Engine.page(for:)` runs `Contents/MacOS/dx render --theme auto` — the binary in its
  *own* bundle — and loads the page. A failure shows `dx`'s own sentence in the window rather
  than a blank one. (JavaScript was off in the web view at the time; the editing wave above
  turned it on for the *window* and took it away from the *document* — see `Editor.swift`.)
- **One application, two roles, chosen by what macOS hands the launch.** A document is a read;
  nothing is the `dx setup` this app has always run. The shell launcher is gone.
- **The bundle declares the type**: `CFBundleDocumentTypes` plus an exported
  `tools.dx.document` conforming to `public.plain-text`, and `build-icons.py --icns` now draws
  the icon the app and every `.dx` file wear (not committed — drawn on the way past).
- **`dx setup` installs and registers it** (`doc-cli/src/desktop.rs`): copies the bundle to
  `/Applications` (`~/Applications` if that is not writable) — for the same reason the binary
  is copied onto `PATH` — then `lsregister -f`. `dx doctor` grew a `documents` section.
  `--uninstall` leaves the app and says so.

### Validated on this machine

- `packaging/build-app.sh` → 2.7 MB bundle (`dx` 1,713,120 B, `dx-app` 177,344 B), ad-hoc
  signed. `DX.app/Contents/MacOS/dx setup` → `documents  installed  /Applications/DX.app`.
- `NSWorkspace.urlForApplication(toOpen:)` for `examples/welcome.dx` → `/Applications/DX.app`;
  `UTType(filenameExtension: "dx")` → `tools.dx.document`, declared.
- Opened `examples/welcome.dx`, `examples/showcase.dx`, and a **store pointer** (`/tmp/pad`,
  `~ dx1 7ed38ca…`) — all render as pages, screenshotted. A pointer with no content shows the
  `run \`dx sync\`` sentence, and no `.doc/` was created by reading it.
- Bare launch runs setup and shows the one dialog (screenshotted); View ▸ Reload re-renders.
- `cargo test` **467 passing**, `clippy --all-targets -D warnings` clean, `fmt --check` clean.

### The one defect found and fixed on the way

Activating the app *during* the load left the page scrolled to its end on 2 of 5 launches — a
reader opened a document and got its last line. Ordering the window front before rendering into
it fixed it: 10 of 10 at the top afterwards, measured through the scroll bar's accessibility
value. `window.isRestorable = false` for the same class of reason.

### Not done

Not signed with a Developer ID, so the application only runs on the machine that built it (see
`packaging/README.md`). No auto-reload when a document changes on disk — ⌘R is the whole story.
Windows are not restored across launches.

## Earlier wave: every layer driven, and what driving it found

The brief was to exercise dx across every layer on this machine and make each view read as a
sheet of scratch paper. Rendering every document and looking at it turned up four defects, two
of which were silently destroying content.

### Content loss, fixed

- **A block closed on `::end` anywhere in a line.** The recovery rule is *trailing* close
  token; the code matched the token mid-sentence, so a document explaining the format lost text
  at the moment it said so — "a block ends with `::end` on its own line" was cut at the
  backtick, and an SVG label containing `::end` truncated the drawing and every element under
  it. `parse::trailing_end_index` now requires whitespace before and nothing but whitespace
  after, which is the shape a single-line block already had. The documented recovery
  (`… } ::end`) still recovers.
- **An unknown `{{key}}` rendered as the empty string.** A typo in a key erased the word it
  stood for, with nothing to say it had. `nav` labels already keep an unknown token (the
  contract says why), and a malformed `{{ not a key }}` already survived — the well-formed
  unknown was the odd case out. It is now left exactly as written.

Both are pinned by tests named for the behavior, and both are now written into
`docs/dx-format-contract.md`, which was silent on each.

### The corpus was stale, and it was the whole shop window

Nineteen blocks across `examples/` and `documents/` used `::code language=html|svg`. Per the
contract that is *code in that language*, so every one rendered as a wall of raw markup — and
`block-reference.dx` documented a "rich rendering path" for it that does not exist. They were
written for the TypeScript renderer deleted on 2026-07-30.

All nine documents are rewritten: real `::html` and `::svg` blocks, and every drawing redrawn
in `currentColor` at hairline weight, so **one drawing is correct in both palettes** instead of
being a dark navy panel sitting on paper. The three `documents/` dashboards asserted PASS for
`docdb:/` paths and a `doc-service` that no longer exist; they now describe the real store, the
real browser path, and the real release gates. The corpus is canonical, so `dx fmt --check` is
clean and a save never rewrites it.

### The other two

- **The VS Code toolbar styled itself with `--dx-surface-2`, `--dx-border`, `--dx-sans`** —
  none of which the stylesheet has defined for some time. An undefined variable is not a
  fallback; the declaration is invalid and dropped, so the bar rendered unstyled. It is now
  marginal mono on the paper's own tone, and `engine.test.mjs` holds every surface to the
  palette `doc-core` publishes.
- **`dx png --pages --out pages.png` wrote `pages.png-1.png`**, because only the default path
  had its extension stripped. `page_stem`/`page_target` are pure and tested.

Also: wrapped code lines now hang under their own line rather than restarting at column 0
(per line — `text-indent` on the `pre` applies to its first line box only, and `min-height: 1lh`
is what keeps a blank line blank), and `dx run` names a drawing instead of printing an SVG's
opening tag into the terminal.

### Gates, all run this session

- `cargo test` → **463 passing, 0 failed**; `cargo clippy --all-targets -- -D warnings` → 0;
  `cargo fmt --check` → clean.
- `./editor/build.sh` then `node --test "editor/github/test/*.test.mjs"` → **34 passing, 0
  skipped**, so both wasm builds render what this binary renders.
- `dx fmt examples/*.dx documents/*.dx --check` → every document unchanged.
- Driven by hand: `dx doctor`, `text`, `outline`, `ls`, `search`, `render`, `png`,
  `png --pages`, `run`, `sync`, `stats`, `textconv`, `git-setup` + `git diff`, a missing-pointer
  error, `dx serve /health`, and `dx_list` / `dx_read` over MCP.

### Not done, and why

`dx serve` still answers only `/health`, `/pack`, `/engine` — there is no human-facing local
viewer at `127.0.0.1:7333`. That is the "local viewer" idea from the previous wave, and it is a
new surface rather than a fix, so it was left alone. `dx ls` lists the 13 test fixtures under
`rust/doc-core/tests/fixtures/` alongside real documents, which makes `dx_list` noisier than it
should be for an agent; deciding whether a fixture directory should be skipped is a product
call, not a cleanup.

## Earlier wave: one source of truth, and the size record corrected

### The size numbers, measured

The brief said "392 MB… or gigabytes". Both were wrong, and nothing that ships was ever large:

| Thing | Before | After | Ships? |
|---|---|---|---|
| `dx` release binary | 2,091,232 B | **1,694,960 B** (−18.9%) | yes |
| `DX.app` | 3.4 MB | **2.5 MB** | yes |
| `dx-chrome.zip` / `dx-firefox.xpi` | — | 162,881 / 162,943 B | store upload |
| `rust/target/` | 2.3 GB | 2.3 GB | **no** — build artifacts |
| `packaging/build/safari/` | 149 MB in-tree | **0** — now a temp dir | no |
| `node_modules/` | 179 MB | **gone** | no |

"392" was **392,564 bytes** — the embedded extension, 19% of the binary. "Gigabytes" was
`rust/target`. The release profile already carries every size lever (`opt-level="z"`,
`lto`, `codegen-units=1`, `panic="abort"`, `strip`). `wasm-opt -Oz` was measured on the
engine: **306,511 → 304,775 B, −0.57%** — not worth trading render speed for, and reverted.

### Done and validated

- **The TypeScript implementation is gone.** `src/`, `vscode-extension/`, `test/`, `scripts/`,
  `build/`, `data/`, `node_modules/`, `package*.json`, `tsconfig*.json`,
  `TEST_COVERAGE_REPORT.md`, `copilot-instructions.md`, `docs/archive/`, and two parity
  harnesses (`rust/doc-wasm/validate.mjs`, `validate-webview-parity.mjs`). Verified dead first:
  nothing under `rust/`, `editor/`, or `packaging/` referenced any of it. `copilot-instructions.md`
  was actively wrong (it described `.doc/.repo-docs.bin` and "SQLite is removed"). Rust doc
  comments that pointed at deleted files (`src/doc-format.ts`, `src/core/digest.ts`, "the
  TypeScript reference shape") now describe the behavior instead. **456 tests still green
  immediately after the deletion** — nothing shipped depended on it.
- **The extension left the binary.** `extension.rs` no longer `include_bytes!`s anything;
  `ASSET_PATHS` names the files and `assets(source)` reads them from a directory.
  - `editor/github` is the one source; **`dx browser --from <dir>` is the one builder**, and
    `build-app.sh` and `build-stores.sh` both call it. What a store gets is what a developer
    loads unpacked, from the same code path.
  - `extension::installed_dir` finds the app bundle's copy first, then whatever `--from` wrote.
    Verified live: `DX.app/Contents/MacOS/dx browser` reports the bundle's directories; the
    same binary run from `rust/target/release` reports this machine's per-user ones.
  - `Channel::Absent` is new, and is what a `dx` with no extension says. The invariant that a
    directory is **only named when it is really on disk** is now a test.
- **Both wasm builds come from one command.** `editor/build.sh` replaces
  `editor/github/build.sh` and emits `no-modules` → `editor/github/wasm` and `nodejs` →
  `editor/vscode/wasm`. **The vscode build is now held to `dx render`'s bytes too** — the test
  loops over every build, so the gap `CLAUDE.md` used to warn about is closed. 32 → 33 node
  tests, **0 skipped**.
- **`build-safari.sh` builds in `mktemp -d` with a cleanup trap.** It was leaving 149 MB of
  Xcode derived data inside the repository; every exit path in that script is a deliberate
  `exit 0`, which is why the cleanup is a trap and not a final line.
- **`.gitignore` rewritten** for a Rust-only tree (the old one ignored `data/*.sqlite`,
  `coverage/`, `vscode-extension/media/*.js`, and a `build/` that no longer exists).

### Gates

- `cd rust && cargo test` → **459 passing** (456 + 3 new); `cargo clippy --all-targets -D
  warnings` clean; `cargo fmt --check` clean.
- `node --test "editor/github/test/*.test.mjs"` → **33 passing, 0 skipped** (run
  `./editor/build.sh` and `./editor/github/test/fixture.sh` first).
- `packaging/build-app.sh` → DX.app 2.5 MB, ad-hoc signed, extension in `Resources/extension`.
  `packaging/build-stores.sh` → both archives. Both built and inspected this session.

### The one thing traded away

A per-user extension directory written by an older `dx` is no longer rewritten by `dx setup`,
so it can go stale, and `installed_dir` will still prefer it over nothing. It mostly does not
matter — `engine.js` prefers `dx serve`, which is always the current binary, and only the
offline fallback would be old — but there is no staleness check without a source to compare
against. If it bites, the fix is for `dx browser` to compare the installed `manifest.json`
with the one this binary generates and say so.

### Correction owed from the previous session

I told the user a local proxy for github.com "wouldn't work" **without having evaluated it**.
The conclusion holds but the reason is TLS: github.com is HTTPS with HSTS, so a proxy cannot
rewrite page HTML without terminating TLS — which means installing a trusted root CA and
man-in-the-middling their GitHub session. That is strictly more invasive than an extension
(every site, not just github.com; indistinguishable from malware to any EDR) and every browser
would still need its proxy settings changed, which is the per-browser configuration we are
avoiding. The genuinely promising middle path is that `dx serve` already exists: a **local
viewer** at `http://127.0.0.1:7333/…` that renders any pointer needs no browser extension at
all.

## Where we are

The store is the authority: a `.dx` file on disk is a one-line pointer (`~ dx1 <sha256>`), the
content lives in `.doc/repo.dxcp`, and every read resolves to the true document. The repository
is Rust and only Rust. The engine is built once for every surface, and every surface's build is
compared against the binary.

Authoritative docs, each the only place its topic is described: `README.md` (users), `CLAUDE.md`
(this codebase), `docs/dx-format-contract.md` (the format and how it is stored), `docs/github.md`
(rendering on github.com), `packaging/README.md` (the release runbook and what each browser
actually permits).

## Earlier waves, still true

- **`dx setup` is the one install command**: binary on `PATH`, MCP registered with every
  assistant, `dx serve` at login, each browser configured by its own route. `--uninstall`
  reverses everything written outside dx's own directory.
- **What each browser permits, measured not assumed.** Chromium: no silent install exists on an
  unmanaged machine (a root-owned managed-preferences plist *is* read and Chrome still never
  requests the update manifest). Firefox: zero clicks via `policies.json`, but release and beta
  refuse an unsigned XPI, so it needs a free AMO signature. Safari: one click, forever, and the
  extension must live in an app bundle.
- **Security pass.** `render::escape` is an allow-list (never make it a deny-list again — the
  old one fell to `<img src=x/onerror=…>`, `<iframe srcdoc>`, and entity-encoded schemes). The
  daemon requires a loopback `Host` (this is what stops DNS rebinding) and refuses a
  non-extension `Origin` in the handler, because a cross-origin `POST` of `text/plain` arrives
  with no preflight. Everything sized by untrusted input is bounded.
- **The github.com extension was driven in a real Chrome** against a real pushed repository and
  ten defects were fixed; blob, blame, commit, and pull-request pages all render, and no page
  shows a raw `~ dx1` line.
- **Do not run `dx sync` at the repository root** — `examples/` and `documents/` stay plain
  text, and `rust/doc-core/tests/fixtures/*.input.dx` are hermetic copies that `tests/fixtures.rs`
  checks for drift.
- **Environment note:** headless Chrome on this machine writes a fully transparent PNG for *any*
  page. Do not use it to render images; `packaging/build-icons.py` rasterizes directly.
- **Driving DX.app:** post real `CGEvent`s. AppleScript's `tell System Events to click at {x,y}`
  performs an accessibility press, which reports the element it hit and never reaches the DOM —
  it will make a broken editor look like a working one.

## NEXT STEP

Earlier waves are committed (`git status` clean as of 2026-07-30, before the CLI
arg-parsing wave above). In order:

0. **Open a `.dx` in VS Code by hand** and check the webview: click a block, type, save. What
   needs confirming is that the page is no longer rebuilt on each edit (the caret and the
   scroll should stay put). The `.vsix` in `editor/vscode/` is current.

1. **addons.mozilla.org — free, biggest win.** Submit `packaging/build/dx-firefox.xpi` as an
   *unlisted* add-on ("On your own site"), put the signed result at
   `packaging/signed/dx-firefox.xpi`, rebuild the app. Firefox then installs with **no clicks**.
2. **Chrome Web Store — $5 once, covers six browsers.** Upload `packaging/build/dx-chrome.zip`,
   then set `CHROME_WEB_STORE` in `rust/doc-cli/src/extension.rs`. One line; a test pins both
   halves. This is also what makes `Channel::Absent` rare instead of common.
3. **Apple Developer — $99/yr**, only for distribution: Safari will not load an ad-hoc-signed
   app extension for an ordinary reader.

Still open from earlier waves: open a `.dx` on github.com in a **signed-in** browser on a
**private** repository (the same-origin raw route is supposed to carry the reader's session —
that is the one claim in `docs/github.md` still resting on reasoning). Then delete the scratch
repository: `gh repo delete RockyWearsAHat/dx-github-probe`.
