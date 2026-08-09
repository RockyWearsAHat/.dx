# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–39) lives in this file's own git history —
`git log -p HANDOFF.md`._

Last updated: 2026-08-09

## Current state

Everything green, everything current, proven mechanically:

- `dx run dev.dx` after this wave's engine edits: **all five gates re-ran and passed** —
  engine (378+50), lints, both JS surfaces (34/34, parity to the rebuilt wasm), corpus,
  page-contract (which now declares `reads=rust,examples`, so engine edits stale it).
  Host shell confirmed the same: cargo test 918 passed 0 failed, clippy `-D warnings`,
  fmt. `dx setup` installed the new build. **A session's own `dx mcp` process keeps the
  engine it started with** — reads through MCP reflect an engine fix only after the agent
  session (and its MCP server) restarts; the CLI reflects it immediately.
- `examples/example_site_plan.dx` verify: **33 of 33 claims hold**, weight
  **29,978 of 30,000 bytes — 22 in hand**; `site-economics.dx` 4/4 (lean round 18% of
  naive) and `route-economics.dx` 3/3 (ten questions at 43%).

## Part 55: three judged render defects became engine law, the harness learned to distrust itself, and the orientation A/B finally landed at ≤1.0×

Worked entirely as the standing loop: use dx, fix every resistance, measure honestly.

- **A closed fold states its size** (`render::html::code_html`): the summary reads
  `sql · 14 lines`, so a folded listing reads as content put away, never content missing —
  the defect 2/3 judges hit in part 53. And a new **additive `open` attribute** on `::code`
  starts the listing expanded (fold kept, page still script-free) for the page where the
  sample is the point. En route this caught a latent parser hazard: bare booleans were
  type-blind, so inline `::heading id=h Open questions ::end` would have swallowed "Open"
  as an attribute (and `run`/`hidden` prose already could be). `format::attrs::bare_booleans`
  now scopes them per kind — `run`/`open` to code, `module` to script, `hidden` to all —
  pinned by `a_bare_keyword_opening_an_inline_body_stays_prose`.
- **Edges hold the label's own legibility law** (`render::board::EDGE_STROKE_MIN`): below
  the floor every path states the compensating canvas-pixel stroke, exactly as
  `EDGE_LABEL_MIN` does for words, and the markers scale with it (`markerUnits`
  strokeWidth). The site-plan board's edges went from 0.61px displayed wires to
  `stroke-width:3.06` at its 0.409 fit. An adversarial pixel reviewer confirmed the wiry
  complaint dead — and caught the *next* defect: **labels sliced by node boxes**. Fix:
  labels ride their own sheet (`.dx-board-edge-labels`) painted after the nodes — an
  edge's words are content, and content is never hidden by a box; the paper halo keeps
  them readable over a node's ink. `edit.js` finds labels on the canvas, not the edge
  sheet; one-line change, geometry untouched.
- **Emphasis composes across a code span** (`render::inline`): the line is assembled
  first (code spans opaque), then emphasis applies — `**bold `code` bold**` is bold end
  to end instead of literal `**` ink, with `x**y` inside backticks still verbatim.
- **The harness stopped trusting a stale gate:** `dev.dx#page-contract` read only
  `examples`, so the gate about the *renderer* never re-ran on an engine edit. It now
  declares `reads=rust,examples` (edited through `dx set` with the body piped — the
  erase-refusal did its job again) and proved itself by re-running unprompted on the next
  engine change.
- **Experiment #5 — the orientation-dominated A/B (the part-54 named next measurement).**
  Corpus: ripgrep @ 3fce3b5, 110 rust files / 56k LOC, two identical arms. Setup agent
  built arm-dx's index via `dx index` + verification-first improvement (78.5k tokens,
  25 tools, ~4.5 min — the amortized session-1 cost). Two fresh maintainers, identical
  8-question comprehension task with file:line evidence, judged mechanically per claim.
  **Cost: dx 79,172 output tokens / 25 tools / 48.6k chars read / 40 turns / 228s vs
  standard 79,087 / 30 / 51.6k / 47 / 250s** — token tie, −17% tool calls, −6% bytes
  read, −15% turns, −9% wall. **Quality: dx 8.0/8, standard 7.5/8** — every dx citation
  verified line-exact; the standard arm confidently inverted `--ignore-file` precedence
  on the precedence question (a claim a maintainer would act on and get wrong).
  **Doctrine at n=5: the first measurement at-or-under 1.0× on tokens, and the win is
  accuracy at parity — the index turns exploration into address lookup; the remaining
  cost floor is opening the cited files themselves, which both arms pay identically.**

The review loop then ran a second round and earned its keep twice more:

- **The label sheet traded slicing for occlusion, and the re-judge caught it:** at page
  fit the floor-compensated font makes a label many canvas pixels wide, and painted above
  the nodes its halo erased node text ("carries the moon's o…", a blanked checkbox). Fix:
  **`render::board::label_spot`** — the words slide along their curve, middle outward, to
  the first sample whose estimated box (at the *effective* font) covers no node, else the
  least-covering sample; mirrored as `labelSpot` in `edit.js` with the same numbers.
  Pinned by `a_label_slides_along_its_curve_to_clear_a_box`.
- **The reviewer's operational note was a real regression:** whole-document `dx png` of
  the site plan died with `no answer to Page.captureScreenshot: timed out` — the flat
  15s `COMMAND_TIMEOUT` read a 41-megapixel rasterize+encode as a dead browser.
  `cdp::SCREENSHOT_TIMEOUT` (120s) now covers exactly that command; the plan document
  captures whole again (2400×24000 px, ~13s).

Then the third review round pulled the thread that unravelled two deep defects:

- **Every capture path was shipping every `::view` frame as empty paper** — lone block,
  paged read, tall single capture — and bisection found the trigger: the site's
  `<link>` to fonts.googleapis.com. A render-blocking external stylesheet fetch from
  the sandboxed frame never completes under the CDP session, so the frame never
  paints; the old one-shot's virtual time had been skipping past it, and virtual time
  itself now freezes frames at every stage it is granted (tried after-load,
  pre-navigation, and re-armed: blank, hung, black). Two fixes, one law: **the
  capture browser resolves no hostnames, ever** (`--host-resolver-rules=MAP *
  ~NOTFOUND` — a capture is a read, and a document must not reach the network by
  being looked at; the fetch fails instantly and the page paints with what it
  carries), and **settling is real time now** (`open_settled`, real-ms grace +
  `stable_screenshot` agreement), virtual time deleted from the capture path
  entirely. Whole-document captures of tall pages also scroll-and-stitch
  (`STRIP_HEIGHT`, `png::stack`) so deep frames pass through the viewport they need
  to paint. Lone view: 5.5KB blank → 135KB painted, in a third of the time. All 12
  pages of the plan document now carry their views.
- **The label fix that "didn't land" exposed a fingerprint hole:** the in-flow
  capture came back byte-identical because the label collided only with the edge's
  *own endpoints* — excluded from `obstacles` (right for curve routing, wrong for
  words). Labels now treat every node as an obstacle and yield the size floor when
  nothing clears (`label_layout`). And the `surfaces` gate had skipped as cached
  across a wasm rebuild: `collect_tree` fingerprinted binary files as *lossy UTF-8*,
  and two different wasm builds collapse to identical replacement-character soup — a
  binary now contributes its bytes' sha256
  (`two_different_binaries_never_share_a_fingerprint_contribution`).

Proof: cargo 922 tests 0 fail (doc-core 379, doc-cli 309, doc-shot 69), clippy
`-D warnings`, fmt, both wasm engines rebuilt (`editor/build.sh`), both JS suites
34/34 against the fresh fixture, release built and installed by its own `dx setup`,
all five `dev.dx` gates green — surfaces re-running on the wasm change for the first
time, page-contract on every engine change. Contract doc carries `open`; `edit.js`
autocomplete offers it.

## Part 54: the cost levers shipped — the change-sized edit, the one-command export, and the self-truing conversion height

The part-53b diagnosis became engine law the same day, one complete change set:

- **`edit::replace_in_block`** — the change-sized edit. `dx set <doc> <block> --replace OLD
  --with NEW [--all]` and `dx_edit`'s `replace`/`with`/`all` params land on one
  implementation: exact-match inside the body, refused with the count when ambiguous
  (`--all` to mean every one), refused when absent, board courtesy kept, edit-is-the-review
  kept. A rename now pays for the characters that changed, never a retyped block — the
  +60% lever from the session-2 measurement.
- **`dx render --all [dir] --out DIR`** — one command exports a whole workspace, each
  document to `<out>/<same relative path>.html`, hydrated, same engine as a lone render.
  The double-bookkeeping regeneration ceremony is one call now. `--all` answers with a
  report (never redirected into `--out`, which names the directory in that form).
- **Converted mermaid boards state no height.** `format::layout::arrange` returns the body
  alone; the frame is re-sized from the nodes on every render (`flow_height`), so the
  part-53 stale `height=268` cramping cannot recur. `viewport_height` and its clamps
  deleted.

Docs true in the same pass: `dx help` (set + render), the `dx_edit` tool schema (replace
taught as the preferred small edit), the MCP handshake, CLAUDE.md's edit-ops sentence.
Proof: cargo test 910 passed 0 failed across all crates (doc-cli 302→308, doc-core +5),
clippy `-D warnings` clean, fmt clean, `editor/build.sh` rebuilt both wasm engines, both
JS suites 34/34, release binary installed by its own `dx setup`, and all three features
smoke-tested live (replace on a synced doc, `render --all` to dist, a fresh `::mermaid`
converting to a height-less board). Worklist items ticked on `index.dx#now-worklist`;
fold-strip and board-edge items remain open.

**The proof run (`wf_33b3d294-650`, fresh copies, identical workload) shows the gap
closed.** Maintainer cost, measured from transcripts: standard 10,242 output tokens /
15 tools / 136s; dx **10,823 / 14 tools / 151s** — the pre-fix +60% token gap fell to
**+5.7%**, wall +37% → +11%, and dx now makes *fewer* tool calls. The mechanism is visible
in the payload decomposition: dx's Bash payload fell 15,990 → 8,444 chars (the port change
went through `dx set --replace`, three occurrences; the exports through one
`dx render --all . --out dist`), putting dx's total tool payload (15,107 chars) *below*
standard's (16,515). Correctness verified mechanically in both arms: zero stale ports,
Release Notes wired 2× on every page, both directions, reports present. The blind judge pass (resumed from the run's cache after the account
session limit reset; maintainers replayed, judges ran live) confirmed it: **both judges
scored both arms 10/10** — every comprehension answer verified against the site's own
text, zero stale ports reproduced by their own greps, the new page's identity confirmed
from rendered pixels (one judge md5-matched the `::style` block across all five exported
pages), zero contradicted claims. Final ledger for the day: pre-fix dx +60% tokens for
the same 10/10 → post-fix **+5.7% tokens, fewer tool calls, same 10/10, blind-confirmed**.
The single next measurement worth running: the same A/B on an orientation-dominated
corpus (≥tens of files), where index+search vs grep+Read is the live variable and dx has
a mechanical reason to land under 1.0×.

## Part 53: experiment #3 — the greenfield A/B re-run with iterate-until-satisfied; parity confirmed, dx measured slightly heavier

The part-51 method lesson was applied and the experiment re-run (workflow `wf_c58aa3d2-b68`,
transcripts in this session's subagents dir): identical Meridian 4-page site spec, identical
pre-registered rubric, both arms told to **iterate until the rendered pixels satisfy them**
(no fixed pass count), then three blind judges. The engine carried all eight part-51 fixes
(binary built 19:46 same day; dx builder directed to the CLI for renders since the session's
`dx mcp` predated the build).

**Measured cost (from agent transcripts, not self-report).** Standard: 19,266 output tokens,
17 tool calls, 33 API turns, 245s wall, 1.81M cache-read. dx: 21,667 output tokens, 29 tool
calls, 51 API turns, 372s wall, 3.56M cache-read. **dx +12% output tokens, +71% tool calls,
+52% wall, ~2× cache reads.** Standard converged in 2 render-look-fix iterations, dx in 3.

**Scores (3-judge avg).** Standard: **spec 10.0, design 9.5**. dx: **spec 9.83, design 8.33**.
Zero contradicted REPORT claims in either arm. The part-51 design gap narrowed (7.17 → 8.33 —
the eight fixes were real) but did not close, and dx's extra iteration went to fighting its
own tooling, not polish: the mermaid→board conversion left a stale `height=268` (diagram at
0.44 fit until hand-restated), `**` spanning inline code didn't compose, and 2/3 judges read
the dist HTML's collapsed code fold (`‣ sql` strip) as missing content on the page where the
first-query sample is the point. Judges also still called board edges "wiry hairline, curves
crossing mid-canvas."

**Efficiency doctrine, confirmed at n=2: greenfield generation is ≈ 0.9–1.0× for dx — it is
not where the efficiency lives.** Iterate-until-satisfied did not flip it, because per-look
cost is near-identical (headless-Chrome screenshot ≈ `dx png`) and dx pays extra passes on
tool defects. The 2–5× range remains a maintenance/orientation claim: index + recorded
verdicts replacing re-reading and re-verification on an existing corpus, compounding with
corpus size × session count. Three part-53 defects queued on `index.dx#now-worklist` (fold
strip in site exports, stale mermaid height, board edge quality).

### 53b: experiment #4 — session 2 on the inherited corpus; quality tie at ceiling, dx still costs more, and the token decomposition names why

The "encoded knowledge means the next session just works" claim got its direct test (workflow
`wf_10428f91-e4b`): fresh maintainers with no build context inherited each finished workspace —
5 comprehension questions with evidence, port renumbered to 9142 everywhere, a Release Notes
page wired into every page's nav — then two blind judges on a pre-registered rubric.

**Quality: a tie at the ceiling.** Both arms 10/10 from both judges, first pass, zero
contradicted claims, zero stale ports, every claim mechanically reproduced. "Just works"
happened — in both arms.

**Cost: dx +60% output tokens (14,567 vs 9,128), +37% wall (212s vs 155s), same 17 tool
calls.** The decomposition (tool payload chars per arm): dx pushed 15,990 chars through Bash
(vs 8,312) because **`dx set` retypes a whole block body to change one string** — nav/footer
rewiring across 4 documents paid payload ∝ block size where `Edit` pays old+new strings ∝
change size — and the workspace's dist/*.html export tree meant every edit also paid `dx
render` ×5 regeneration. More turns (38 vs 32) carried the method's extra decisions (sync,
adoption, export freshness). The read-economy win never appeared: a 5-page corpus fits in two
Reads, so grep ≈ dx search at this scale.

**Honest ledger at n=4 experiments: no measured scale yet shows dx under 1.0× on tokens**
(greenfield ~1.1×, fd maintenance +13%, session-2 here +60%). What dx measurably wins is
verification quality — zero-iteration convergence on fd, judges reproducing every claim here.
Two engine candidates queued on `index.dx#now-worklist`: a surgical within-block
`--replace OLD --with NEW` (the single biggest lever — turns cross-cutting edit cost from
block-sized to change-sized), and a one-command whole-workspace export so a site deliverable
stops double-bookkeeping. Next measurement that could show <1.0×: orientation-dominated task
on a corpus large enough that index+search beats grep+Read (≥tens of files), with the
surgical edit shipped first.

### 52b: the autosetup gap closed — one command now writes the setup the experiment had to hand-build

All three part-52 worklist items shipped the same day (ticked on `index.dx#now-worklist`):

- **`dx index` scaffolds the harness it preaches.** Build-system detection (Cargo at
  root or one level down — the `rust/` monorepo layout; `package.json` with a real test
  script; pytest projects; `go.mod`; a `Makefile` with `test:`) writes `dev.dx` beside
  `index.dx`: test/clippy/fmt gates for cargo, a test gate for the rest, every gate
  carrying the sandbox pattern (rustup `CARGO_HOME` resolved from PATH,
  `--offline --locked`, `reads=` listing only paths that exist, `writes=target`).
  Scaffolding never approves — `dx run dev.dx --approve` is the review, and the index's
  new Verification section says so. An existing `dev.dx` is kept even under `--force`.
- **The index scaffold carries real signal now.** A bounded static survey (≤600 files,
  ≤512KB each, identifier tokens only) ranks each area's files by fan-in and size and
  says why — on fd, `output.rs — 182 lines, referenced by 11 files` tops `src/`, entry
  points are flagged, `Entry points seen:` lands in each area's TODO, the lead paragraph
  is seeded from the README's first prose (badge/link lines skipped), and the method
  skeleton (`How a change flows` recipes section) is scaffolded in. Grammar fixed
  (`1 file`).
- **`edit::remove_block` takes the `::output` records reporting on the removed block** —
  a document no longer keeps output for code it doesn't carry. One implementation, every
  surface (CLI `dx remove`, MCP `dx_edit`).

Proof: cargo test all 14 binaries green (doc-core 365, doc-cli 302), clippy `-D
warnings`, fmt, `editor/build.sh` rebuilt both wasm engines, both JS suites 34/34,
DOC's own `dev.dx` gates re-ran green. End-to-end on a **virgin** fd clone: `dx index .`
alone wrote the ranked index + 3-gate harness, and `dx run dev.dx --approve` ran green
first try — 264 tests, clippy, fmt, zero debugging, where the morning's hand-built
attempt needed a probe session to discover the sandbox HOME. Uncommitted in the working
tree alongside part 51's fixes.

## This wave, part 52: experiment #2 — autosetup judged, proper setup built, maintenance A/B re-run on the fresh revision

The queued maintenance-task experiment ran (workflow `wf_697ebf93-a64`, transcripts in the
session's subagents dir): a never-dx'd real project (sharkdp/fd @ 0f1f967, ~8.5k LOC,
264-test suite), `dx index` judged raw, the setup then built properly, and two fresh
builders on an identical task — 5 comprehension questions with file:line evidence, add a
`--count` flag with ≥3 integration tests, iterate until test/clippy/fmt green, honest
REPORT.md — one standard-tools, one dx-methodology, then three judges re-verifying every
claim mechanically.

**Autosetup verdict.** `dx index` is honest and instant: correct 6-area map, complete
file lists, Now/Findings doctrine embedded, self-instruction to improve first. But it
knows nothing the tree doesn't say, ranks nothing, and — the real gap — ships zero
runnable gates while its own Findings prose preaches them. Naive `cargo test` gates then
fail in the sandbox (fresh HOME, no network); the working pattern (rustup `CARGO_HOME` +
`--offline --locked`) exists only in DOC's own dev.dx. Proper setup (true pipeline map,
change recipes, dev.dx with 3 gates recorded green) cost ~15 operator minutes. Three
part-52 items are on `index.dx#now-worklist` (harness scaffolding, orphaned-output on
`dx remove`, `(1 files)` grammar).

**Measured result (n=1, one caveat below).** Both arms fully green, feature verified
live by all judges, comprehension tied 4.5/5, one contradicted claim each. **3/3 judges
preferred the dx build** (blind labels normalized by directory; judge 3's A/B were
swapped): feature score avg 9.0 vs 7.83, decided on spec conformance — dx shipped 4
separate tests with exact clap-conflict-message assertions and needed **0 fix
iterations** (gates + index recipes carried it); standard shipped 1 bundled test
function and needed 2 iterations. Cost was near-parity again, dx slightly heavier:
25,045 vs 22,117 output tokens (+13%), 6m00s vs 4m49s wall, 38 vs 37 tool calls. The
efficiency doctrine holds: single-session task on a small repo ≈ 1.0–1.15×; the win the
proper setup bought was *quality and zero-iteration verification*, not tokens.

**Caveat.** The Workflow args arrived stringified, so both builder prompts said their
workdir was `undefined`. The dx builder recovered by finding the arm's workspace (the
index made it discoverable); the standard builder cloned fd fresh (same revision, so
results comparable, but cost symmetry is approximate). Lesson recorded: pass Workflow
args as real JSON values, and the recovery asymmetry is itself a data point.

## Part 51: the A/B methodology experiment — parity on cost, a design gap with names

A controlled test (workflow `wf_aa2eef67-b44`, transcripts in the session's subagents dir):
one identical greenfield spec (4-page "Meridian" doc site: home, getting started,
architecture + diagram, FAQ, revision pass, verified REPORT), two fresh builders — one
standard workflow, one dx-only — then three blind judges scoring both against a
pre-registered rubric (Spec /10, Design /10, design judged only from rendered pixels).

**Measured result.** Cost was parity: standard 29,923 output tokens / 202k fresh input;
dx 30,783 / 225k (and 59% more cache reads). Wall time ~7–8 min each. Scores (3-judge
avg): standard **spec 9.0, design 9.33**; dx **spec 9.67, design 7.17**. dx won
verification (BUILD-OK harness + link census reproduced exactly by judges; zero
contradicted claims — the standard build had one) and lost design, on defects all three
judges found independently. No order-of-magnitude efficiency appeared on a greenfield
single-session build — the read-economy savings need an existing corpus and inherited
verdicts to bite, and the dx builder spent its whole revision pass fixing *tool*
defects (below) instead of content.

**The fix list (each cost the site design points, cited by 3/3 or 2/3 judges):**

1. **Fold stubs are illegible in a rendered site.** Getting Started's install/query code
   collapsed to faint ~8px `▸ sh` / `▸ sql` slivers — every judge read the page as
   content-missing. The fold label must render at body scale in page ink; a summary a
   reader cannot find is the "label where the listing should be" failure in new clothes.
2. **Board edge ink and edge labels don't survive rendering.** Hairline low-contrast
   edges, ~6px labels (`points`, `flush`, `history`), on dark near-invisible. Edge
   labels need a legibility floor; edge ink should come from the page's ink scale.
3. **Mermaid→layout emitted an unlabeled duplicate arrow** into one node and bunched
   arrowheads on it — check edge dedup and the per-side spread under conversion.
4. **In-flow board frame carries dead space** around a small canvas (384px TD board
   centered in a mostly-empty bordered frame). `block_page` trims lone captures;
   the in-flow frame should hug the canvas too.
5. **LR layout blew up**: a 7-node LR flowchart laid out at 1,870px (~0.37 fit, ~5px
   text); the builder had to re-author TD by hand. Layout should prefer compactness or
   the fit notice should name the re-orientation.
6. **Default sheet reads flat as a *site*.** "Scale contrast … close to the sheet
   default" (2/3 judges). The blank-sheet law holds for documents; a doc-site's display
   scale may need one stronger step in the default type ramp.
7. **No shared identity mechanism**: the same `::style` block was pasted into all four
   pages — token overhead and drift risk. A confined `::style src=` (path law, like
   `::code src=`) would make one theme the single source.
8. **`.dx-doc a` default link color ignores the document's accent** — needed a manual
   override on every page.

**RESOLVED (same day, workflow `wf_136dcfa0-d17`): all eight fixes shipped and re-judged PASS.**
- Fold summaries render at the listing's own mono scale in visible ink (`theme.rs`
  `.dx-code-folded > summary`), confirmed legible in a 4× crop of the rendered page.
- Board edge ink rose `--dx-faint`→`--dx-muted`, edge labels 0.62→0.72rem in `--dx-text`,
  and a render-side floor (`EDGE_LABEL_MIN` 9px displayed: a label on a fitted board takes
  an inline compensating font-size) keeps them legible at any fit scale.
- Mermaid dedups identical `(from,to)` edges at read time (a labeled repeat donates its
  label to a bare earlier one), and `drawn_edges` skips duplicates in hand-written boards.
- The in-flow board frame hugs its canvas (`hug = scale·width + 2·FIT_PADDING`, centered,
  1px jitter tolerance) and `flow_height` dropped the 480 floor for short canvases —
  measured ~24 CSS px padding per side on the re-rendered test board.
- `arrange` re-orients a horizontal placement wider than `REORIENT_WIDTH` (1896 = 3× the
  legible column budget) down the column; `fit_notice` advises by aspect.
- Default H1 takes a deliberate display step (~2.3× body); `.dx-doc a` defaults to the
  page's own ink treatment — no browser blue anywhere.
- `::style src=` landed additively: parse/normalize/stringify carry it (unset serializes
  to nothing — no fixture regenerated, verified by grep), `resolve` treats it as a fifth
  reference kind (path law, `references`, re-rooting, `hydrate` fills from the file; a
  miss records Unresolved without writing a sentence into a CSS body), rendered CSS still
  crosses `escape_style`. Contract doc updated. Bonus: `model.rs`'s stale image-only `src`
  doc comment corrected.
- Proof: cargo test all crates, clippy `-D warnings`, fmt, `editor/build.sh` (both wasm
  builds), both JS suites 34/34 — all green first run; visual re-judge on the experiment's
  own site-b with the fresh binary: PASS on every named defect, adversarial standard.
  Uncommitted in the working tree — commit is the next step.

**Method lesson from the operator (queued as experiment #2):** the A/B prompt said
"revision pass", which reads as one internal cycle — it never invited dx's actual loop
(constant iteration, every change immediately viewable, a fix costing a read+grep+edit).
Re-run the experiment with both arms told to iterate until the rendered page satisfies
them; the per-iteration cost difference is where the methodology gap should appear.

**Efficiency doctrine, restated from measurement:** greenfield build ≈ 1.0×; honest
end-to-end range on real tasks stays 2–5×; the famous 1500× is a read-byte ratio that
only appears when a session answers from index + recorded verdicts instead of re-reading
and re-verifying. The multiple grows with corpus size × session count, not with one
build — the next experiment worth running is a *maintenance* task on a large existing
site, where orientation and re-verification dominate.

## Part 50: every capture pays one launch, and a frame is photographed only settled

The last per-page browser launch died, and the review loop caught two real regressions
in my first cut before they could ship:

- **One launch, one load.** `capture_pages`/`capture`/`capture_block` joined the batch
  route on `doc_shot::cdp`; a paginated read loads the document **once** and scrolls a
  page-sized window over it (boards keep their own self-sized pages, second pass). The
  plan document's 12-page read: 22.0s (reload-per-page) → ~13s settled, from one launch —
  the old route paid **thirteen launches**. `Cdp::open` now closes the previous target,
  so a long batch cannot pile pages up inside the browser.
- **A lone-block capture is the block, not the block plus a sheet.** `render::block_page`
  drops the sheet's margins and takes the page's own content measure (680);
  `BLOCK_MIN_HEIGHT` (24) replaces the 200px floor. A one-line quote arrives as a
  680×61 strip — the ~40% dark-margin waste the operator flagged in part 46b is gone.
- **The reviewer failed my first cut twice, and both failures became engine law.**
  (1) Clipping past the viewport (`captureBeyondViewport`) delivered nine `::view`
  frames as empty paper — sandboxed frames only paint inside the viewport — so flow
  pages scroll instead of clip. (2) Virtual time (`Cdp::settle`, the DevTools form of
  the `--virtual-time-budget` the rewrite had dropped) was still not enough: the
  compositor rasters in **real** time, and the first-met frames shipped empty while
  later ones painted. The fix is `stable_screenshot`: a page is photographed only when
  two consecutive captures agree (120ms breath, 12-look cap) — "settled" is now a
  definition, not a hope. Final re-judge: pagination PASS (visit-phone whole on its own
  page), all nine view frames carrying the real site.

Worklist: the tier past one-launch is a capture session held **across** calls — with the
design constraint recorded (the session must be owned by a host that drops it; MCP's
`handle()` is pure by design, and a process-wide static would orphan a Chromium).

## Part 49: the smear can never ship silently again — and the feature tier landed

Two commits. First, the in-flight visual-loop tier (built by the part-46b workflow, green
in the tree for two sessions) was validated on the host shell and committed: batch
captures (`dx png --block A,B,…`, one browser session for the list), golden-PNG verdicts
in text (`--against`, `doc_shot::diff`, 3/255 antialiasing tolerance), `dx play --node`
reading x,y inside the clipped block (a `::view`'s own controls are now targetable),
`dx_board` over MCP, and the handshake doctrine (pixels in subagents, the review bar is
standing, batch every capture).

Second, part 48's two engine candidates shipped, so the board smear is now mechanically
impossible to miss:

- **A board fitted below 1:3 says so on its own frame** (`render::board::FIT_LEGIBLE` →
  `fit_notice`): the notice names the ratio and the canvas and the way out, drawn on the
  page's own paper along the frame's bottom edge. Only a failure is called out — a board
  that reads carries nothing.
- **An unstated board height takes the frame its canvas needs** (`flow_height`): the
  column's width already fixes the scale, so the frame shows the whole canvas at that
  scale, clamped between the classic 480 and 1,600. The plan board now renders at
  scale 0.409 in a 690px frame — legible, no notice — where the old fixed 480 forced
  0.27; only a true filmstrip hits the cap and earns the notice.
- **`h=fit` stopped double-charging list margins**: sibling `li` margins collapse, so
  `FIT_ITEM_LEAD` dropped 10→6 — the ~80px band under a sixteen-item checklist was
  exactly the double charge. The remaining breath of room is the constants' stated
  clip-protection bias, kept on purpose.

Tests pin all three (`an_illegible_fit_is_called_out_on_the_board_itself`,
`an_unstated_height_grows_the_frame_to_the_width_scale`,
`a_legible_fit_carries_no_notice`, and the rewritten both-axes fit test). Wasm rebuilt
for both surfaces; parity held.

## Part 48: the viewer caught the smear — the plan board holds only the plan

The user opened the document in DX.app and the canvas was a dark smear. Root cause read
straight off the board's own body: ten `::view` frames (two of them 6,100px-tall
whole-page filmstrips) had grown the plan canvas to ~1,410 × 9,100, and the in-flow fit —
which by design shows the whole arrangement — scaled everything to ~0.1. Not an engine
bug: a modeling error the engine faithfully rendered.

- **Restructure, through dx surfaces only:** the eleven view frames, the gallery
  paragraph, and the measure quote dropped `hidden` (via `dx set --header`, bodies
  passed explicitly — the erase-refusal from part 45b did its job once) and now live in
  the page flow; the board body was rewritten to the nine plan nodes, desk-bound edges
  dropped. `lead` and `how` prose updated to tell the true story: plan on the canvas,
  screens below it.
- **Re-judged by a reviewer subagent: PASS on all four areas** — board legible at page
  width (labels, edges, edge-labels all readable), all eleven frames at sensible sizes
  with clean page breaks, reading order coherent, story lands for a first-time reader.
  One cosmetic finding filed on the worklist: `h=fit` overestimates a checklist ~100px.
- **The lesson landed where projects inherit it:** `GETTING_STARTED.dx#method-economy`
  gained the board law — *a plan board holds the plan and only the plan*; screens and
  full-height content belong in the flow. Two engine candidates on
  `index.dx#now-worklist`: a legibility floor `render::board::fit` could name, and the
  checklist fit-estimate.

## Part 47: the loop ran again — the reviewer failed the phone, the budget paid honestly

The standing request re-ran ("revise the example site to be absolutely incredible"),
worked entirely through dx surfaces, and this time the loop improved the *method* where
it creaked, not just the site:

- **The byte law now states its balance.** Last wave's top friction — 10 bytes of
  headroom forcing byte-archaeology — is fixed at the root: the weight claim prints
  `29,978 of 30,000 — 22 in hand`, the plan states the payment rule (delete real
  redundancy, never readability), and the budget proved itself mid-wave by FAILing a
  48-byte overdraft before it could ship. Headroom was *bought*, not shaved: the torch
  hero gradient collapsed into `--night-rgb` (the remap block is now the single source
  of truth), print styles stopped restating what the variable remap already says,
  `og:type=website` (the protocol's own default) and a duplicated `strong` rule went.
- **A reviewer subagent failed five of ten frames; every failure was fixed and
  re-judged PASS.** Phone section headers now stack (no more "Tonight's / sky" wraps),
  the hero copy sits on a real scrim in full starlight ink, the booking select leads
  with scarcity ("nine of twelve places") and wears an ember chevron, instrument labels
  rose to 10–11px dim ink, the journal stagger reads deliberate, and four `::view`
  frames were resized to their content (`visit-view` no longer clips its fine print
  mid-sentence). Verify grew to **33 claims** pinning the mechanical ones; the five
  polish boxes were ticked by `dx check` only after the re-judge.
- **The lesson landed in the teaching doc.** `GETTING_STARTED.dx` gained
  `method-economy`: pixels are judged in a subagent that answers in a sentence, a
  `::view` at phone width is the phone check, and a budget is a law only when it
  states its balance. Frames burned in the reviewer's context, never the operator's —
  the lean loop measured 17% of naive, per `site-economics.dx`'s own run.

## Part 46: the launch boxes closed — the form is the calendar

The site test continued from the document's own two open launch boxes; everything ran
through the dx surface (`dx source`/`set`/`check`/`run`/`png`), which is the test:

- **"Wire the booking form to the calendar" shipped as dated nights carrying their
  moon.** The select's options are the almanac's own calendar — `value="2026-11-…"`,
  each stating the night's moon in the page's voice, and the lunar month is real:
  tonight's waning gibbous (moonrise 01:40, agreeing with the hero and the almanac)
  rises later the next night, the new-moon night is the one that sold out, the crescent
  night after sets by seven. **"Cap each night at twelve, waitlist the rest"** is the
  full night's option (`joins the waitlist`) plus the aside's rule: book a full night
  and you hold the first place freed. The DNS box stays honestly unchecked.
- **Verify grew to 28 claims** (`the form offers the almanac's own nights`, `a full
  night joins the waitlist, never oversold`), edited with `dx set` — the edit recorded
  the approval, `dx run` executed the new code with no `--force`, staleness alone
  re-proved the page. Weight law held: 29,915 of 30,000 bytes, the additions paid for
  by tightening four CSS comments (words only; no rule, no voice).
- **Surfaces observed working:** the board's `::view src=` frames showed the edited
  file with no sync step; `dx png --block visit-view` was the design review (the dated
  option reads cleanly in the closed select on the rendered page).
- **Finding:** this Claude Code session had no `dx` MCP server registered — the CLI
  carried the method, but check `dx setup`/client registration for why `dx mcp` was
  absent from the session's tool surface.

### 46b: the visual pass, and the review-agent bar

Looking at the frames (not just verifying claims) found what no claim covered: masthead
nav wrapped mid-label on desk and clipped the torch control off a phone (now wraps at
every width — the wrap rule moved to the base and the phone duplicate died); the
journal's staggered column was dead in modern Chromium because the scroll-driven `rise`
animation's fill owns `transform` and cancelled the static `translateY` (stagger moved
to `margin-top`, a property the motion system doesn't touch). An independent
review agent judging against the mission then FAILED the site on four more: nav
overflow at 820–1300px, `.visit-aside` below AA contrast (→ `--ink-dim`),
`aria-hidden` on the `<figure>`s swallowing their visible captions (→ moved to the
`.band` divs), and a flag character inside the waitlist option's date value (→ clean
`2026-11-13`). All fixed, two new verify claims pin the mechanical ones —
**30 of 30 hold, weight 29,971** — and the re-judge returned PASS.

Product gaps this loop surfaced (on `index.dx#now-worklist`): `dx play` cannot target
inside a `::view` frame; every `dx png` pays a Chromium launch (persistent capture
session + delta frames + golden-frame text verdicts are the 100× efficiency tier); and
the handshake should teach that pixel judgment belongs in a subagent — the operator's
context carries verdicts, never frames.

**In flight (workflow `wf_fbc80e2a-ca2`):** the feature tier above is being built —
in-view play targeting, batch one-launch captures, `--against` text diff verdicts,
handshake doctrine, `examples/site-economics.dx` (measured token counts for a whole
site generation) — with gates, live browser proof, and adversarial review phases.
**Queued for the fix round after it returns (operator directive, 2026-08-07):**
single-block captures must be trimmed to the block's own box — a board node, an
`::image`, any lone `--block` render delivers content as the entire picture, no page
margin around it (this session's view-frame captures were ~40% dark margin, pure
token waste at reading sizes).

## Open beyond this wave

- **Store distribution** (money, not code): submit `packaging/build/dx-firefox.xpi` to
  addons.mozilla.org as unlisted → `packaging/signed/`, upload `dx-chrome.zip` to the
  Chrome Web Store and set `CHROME_WEB_STORE` in `extension.rs`, Apple Developer for
  Safari distribution.
- **One claim still resting on reasoning:** a `.dx` on github.com in a signed-in browser on
  a **private** repository (the same-origin raw route should carry the reader's session).
- **Driving DX.app from automation:** post real `CGEvent`s — AppleScript's System Events
  click never reaches the DOM and makes a broken editor look like a working one.

Next step: the postscript's remaining tier, one down — text-first frame description,
delta reads after edits, golden-frame verdicts in text, symbol-level source sections.
The gallery-rebuild field test from part 42 still stands, now with supersampled reads
and the `now`/cheap-write loop to run it through; `index.dx#now-worklist` is the live
worklist.
