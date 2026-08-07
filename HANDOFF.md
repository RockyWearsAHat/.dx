# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–39) lives in this file's own git history —
`git log -p HANDOFF.md`._

Last updated: 2026-08-07

## Current state

Everything green, everything current, proven mechanically:

- `cargo test` green every crate (doc-core **347**, doc-cli **285** among them), clippy
  `-D warnings` clean, `fmt --check` clean, both node suites 34/34 against rebuilt wasm,
  fixture corpus and drift guard green.
- `dx run dev.dx` after this wave's edits: engine (347+49), lints, and surfaces re-ran —
  exactly the gates staled by the rust and editor trees — and passed; corpus and
  page-contract cached.
- Release binary rebuilt and installed to `~/.local/bin/dx` on a fresh inode (see the
  part-45 operational note); `examples/example_site_plan.dx` verify: **28 of 28 claims
  hold**, verdict written by `dx run` after the part-46 edits.

## This wave, part 46: the launch boxes closed — the form is the calendar

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

## Part 45: the site test — the document's own records drove the revision

The example-site field test ("write the example site with the method, revise if there are
issues"), run entirely through `examples/example_site_plan.dx`:

- **The document said what was missing, and the revision shipped it.** The unchecked
  audience box (a teacher planning a term-time visit) and the sitemap's "school nights
  still to sketch" were the spec. Shipped: a School Nights section — the year-bar as the
  night-bar's sibling instrument (months below, an ember "dark by six" bracket, gradient
  as data), free-for-valley-schools, a mailto letter route — plus a waning-gibbous phase
  glyph on the almanac's moonrise line. Board gained `school-view`; audience box ticked,
  sitemap and polish updated in the same sweep. Verify grew to **26 claims, all holding**,
  including the 30 KB weight cap the page was trimmed back under (29,986 bytes).
- **The verify block had silently lost its `reads=` declaration** — site edits no longer
  staled its verdict, the exact dishonesty the doc brags it prevents. Restored
  (`reads=site/index.html,site/site.css`); the very next file edit re-ran the proof by
  itself, no `--force`. Watch for this: a header retype that drops attrs is invisible
  until staleness stops working.
- **Engine defect found on the way, fixed at the root:** `section()` sliced a hidden
  board-node block in, but the text renderer skipped it — so MCP `dx_source section=<node>`
  answered *empty where content exists* while CLI `--block` answered. Now a non-heading
  block named by the selector is revealed in the slice (naming it is the ask; the slice is
  a render view, never serialized — heading sections keep their nodes hidden). Pinned:
  `a_section_naming_a_hidden_block_reveals_it`,
  `a_heading_section_keeps_its_board_nodes_hidden`. Proven over a fresh `dx mcp` stdio
  session: the brief answers its text.
- **Operational note that cost a server:** `cp` over the *running* `~/.local/bin/dx`
  inode gets the process SIGKILLed on macOS (stale code-signature cache) and poisons the
  file until replaced on a fresh inode. **The product already guards this** —
  `setup.rs::replace` unlinks before copying and its comment names the SIGKILL — the
  session's mistake was bypassing `dx setup` with a raw `cp`. Install a rebuilt binary
  with the build's own `dx setup`, or at minimum `rm` before `cp`.
- **`dx set --header` no longer erases silently** (part 40's watch-out closed): a header
  retype whose body arrives empty from anywhere but an explicit `--text ''` is refused
  when the block has words to lose; `--text ''` stays the deliberate way. Pinned:
  `a_header_retype_with_nothing_on_stdin_keeps_the_words`,
  `an_explicit_empty_text_still_empties_on_a_header_retype`.
- **Design QA through the document:** `school-view`/`school-phone` frames on the board
  (a phone frame must state its viewport — `width=390`, the miss that made the first
  capture desk-wide); phone width verified by eye, year-bar gradient corrected to peak
  at midsummer, the orphaned label line shortened. Weight held at the 30 KB law through
  every trim (final: 26/26).
- Gates: doc-core 347 (two new), doc-cli 287 (two new), clippy `-D warnings` clean, fmt
  clean, both node suites 34/34 against rebuilt wasm, `dx run dev.dx` green with only
  staled gates re-running. `index.dx#now-worklist` records all of it; the three standing
  items (store submission, gallery field test, private-repo proof) remain open.

## Part 44: the inversion — work lives in the document, and small thoughts are cheap

The operator's scratchbook doctrine ("context is a cache; the document is the memory"),
built so it is the path of least resistance rather than discipline:

- **`dx_append`** — the cheap write. `block` grows a block's body by the new lines alone
  (a finding onto a ledger, a line onto the worklist — no resending what's there; lands
  on the same replace `dx_edit` performs, so edit-is-the-review holds for runnable code);
  `after` inserts a new block after an id; neither appends at the document's end. Prose
  kinds only — code enters through dx_edit/dx_write where its powers are reviewed.
- **`dx_check`** — tick/untick one box by position, `edit::toggle_check` underneath, every
  other byte identical.
- **Fixed a real dx_edit defect found on the way:** the plain path wrote `.text` directly —
  a silent no-op for checklists and lists (content lives in items only `set_body`
  rebuilds) and it skipped board hidden-node creation. Now routed through the engine's
  `set_block`. Pinned: `editing_a_checklist_body_through_dx_edit_actually_changes_it`.
- **The `now` convention.** `dx_index` scaffolds a `Now` section (note + `now-worklist`
  checklist) — the designated first read of a turn and its last write, the task's program
  counter (found / fixing / verified). Handshake gained WORK IN THE DOCUMENT, NOT IN
  CONTEXT: think by writing, the thin-conversation loop, scratch-promotes, first-call-
  reads/last-call-writes — all assertion-pinned. This repo's own `index.dx` now carries
  its `now` section (inserted via `dx insert`, the CLI twin).
- **Proven live over stdio:** a real `dx mcp` process appended a two-line thought for two
  lines and ticked item 2; the wrong-item call refused with the counting sentence.
  doc-cli 285/285, clippy zero, fmt clean; `dev.dx` re-ran only what each edit staled
  (engine+lints for the rust edits, corpus for the index.dx edit).

## Part 43: pixels that survive the budget, and text that stays text

A field session judging UI from `dx_read` pages (Self-Host console, `DX-FEEDBACK.md` on
the operator's desktop) hit the two remaining read-quality defects; both closed at the root:

- **Reading captures are supersampled, then compressed smart.** `ShotOptions.oversample`
  (new, default 1; `for_reading` sets 2): the browser rasterizes at `scale × oversample`
  and `doc_shot::png` — the platform's one PNG codec, hand-rolled on the `miniz_oxide`
  already in-tree — averages it back down **in linear light** to the same vision budget.
  Near-threshold ink (hairlines, faint marks, small edge labels) now arrives as gray
  instead of vanishing at rasterization; sRGB-naive averaging (the "just whatever"
  resize) is exactly what the codec's checker test pins against (linear average of
  black/white = 188, not 127). Codec: 8-bit, four plain color types, non-interlaced,
  bounded decode (64 MP cap, hostile chunk lengths refused), opaque images written as
  RGB, per-row min-sum filter heuristic. Real-Chromium proof:
  `a_real_browser_reading_capture_arrives_at_its_stated_size`.
- **`dx_source` on an image block no longer dumps base64.** Root cause: every reading
  tool hydrates (`document_at`), so an `::image src=` arrives as a multi-MB `data:` URI
  and `render::text` printed it raw — 3.5M characters at a reader who asked for words.
  `image_text` now renders an embedded image as a one-line stand-in (media type, ~KB,
  where to look), same trade as `view_text`; a file `src` stays a Markdown link. The
  stale comment claiming dx_source skips hydration corrected.
- **The method is now the stated default, and the doctrine is complete.** The MCP
  handshake opens with the standing rule: every connected agent works in the dx
  methodology, always, unless specifically requested not to — convenience is not the
  exemption, an explicit request is. It now also carries the operator's full doctrine:
  speak minimally and work constantly, acting on what the task implies from everything
  understood so far, not only the latest message; every codebase gets indexed, always;
  THE DOCUMENTS ARE THE MEMORY (a linked, factual knowledge base — decisions,
  constraints, findings written as they form, claims recorded as run-block verdicts,
  staled documents updated in the same sweep as the change); and planning, creation, and
  verification are one three-call motion — dx_edit writes intent and code, the edit runs
  it, dx_read shows the result. All pinned in
  `the_handshake_advertises_tools_resources_and_guidance`; README's agent section states
  the same rule and points at the handshake as the enforcing surface.
- Feedback triage, recorded: pipefail (#4) was already fixed; folder `reads=` and
  `::image`/8 MB warnings landed in parts 40/42. Still open from the postscript, next
  tier: cheap-write ergonomics (append/insert-after/check over MCP), a blessed `::now`
  bootstrap section convention, scratch-that-promotes.

## Part 42: the picture that names its producer, and the method said out loud

A field session (screenshots, 2026-08-07) built its harness around a hand-managed PNG
gallery — 2× captures against the embed limit, freshness by path convention — and its
own postmortem named four product gaps. All four closed, plus the root cause:

- **`dx_edit` takes `header`.** Lands on `edit::replace_block` (same op as
  `dx set --header`), so changing a block's kind or one attribute over MCP is one call —
  the review→full-`dx_write`→re-grant dance is gone. Focus id returned; a header that
  renames says so in the reply.
- **`::image for=<run-block-id>`** ties a picture to the run that produces its file.
  Parse/stringify/normalize carry it (additive — unset writes nothing);
  `render::html::image_doubt` judges it from the document alone: producer missing /
  never ran / failed → the figure itself is called out (`dx-image-doubt`, red marginal
  note, same dress as a failed run). Green producer → plain picture. Freshness is the
  engine's claim now, not adjacency. Contract § image attrs + README updated.
- **The 8 MB embed limit surfaces at save.** `resolve::MAX_IMAGE_BYTES` is public;
  `workspace::oversized_image_warnings` warns from `dx_write`, `dx_edit`, and `dx set`
  the moment a stored `::image` names an oversized local file — the render-time refusal
  stays, but nobody meets it first anymore. Limit documented in the contract and the
  `dx_write` schema.
- **The handshake teaches the method, not just the tools.** `initialize()` instructions
  rewritten: dx is the working method — index, harness, proof, memory — with named
  sections (ORIENT / READ ECONOMY / THE HARNESS / RESULTS LIVE IN THE DOCUMENT), the
  for= rule, the header rule, and "a gallery of hand-managed images proves nothing and
  is the smell that you have left the method." Assertions pin every load-bearing phrase.

## Part 41: the server that stays current, and the search that answers

The field sessions' efficiency complaints (screenshots, 2026-08-07), fixed at the root:

- **`dx mcp` re-execs when its binary is updated** (`mcp::serve_buffered`). Fingerprint =
  length + mtime of `current_exe`, recorded at startup, checked after each answer; on
  drift the server re-execs itself — same args, same stdio — and the next request is
  served by the new engine. The check only runs while the reader's own buffer is empty
  (bytes in the kernel pipe survive an exec; buffered bytes would not), so pipelined
  requests are never dropped. Proven live: touch the binary mid-session → answer, stderr
  notice, second banner, next request answered by the new process. This closes the
  "restart the assistant" class part 40 misfiled as operational, including stale handshake
  instructions steering agents to page-image reads.
- **A search hit carries its answer.** `doc_core::search::best_block_id` ranks blocks by
  the same rule as documents (shared `score_against` — one scoring implementation);
  `workspace::Hit.block` names the winner; MCP `dx_search` hands over `block` +
  `excerpt` (the block's section text, capped at 700 bytes on a char boundary) and
  `dx search` prints an indented `#id first-line` answer line. Find-then-read is one call.
- **Steering: images one block at a time.** Handshake instructions now say a search that
  lands *is* the read, and `dx_read` images are spent per-block (`block`), never a page
  sweep; `dx_search`'s description no longer says "follow up with dx_read". README agent
  section updated with all three.

## Part 40: the harness, folder reads, and the publish cleanup

"Rewrite all documentation, set the project up with dx properly, test every surface by
automating inside dx, make it publish-worthy" — plus a field review (7.5/10) asking for
documented grant laws, directory `reads=`, and an in-document image loop.

- **`dev.dx` — the development harness, at the root.** Five run blocks execute the gates
  inside the sandbox and record their verdicts: engine suites (`cargo test` offline against
  the shared incremental `rust/target` via a `writes=` grant, `CARGO_HOME` derived from the
  rustup cargo's own path because the sandbox redirects `$HOME`), lints, both JS suites,
  corpus resolution/canonicality (enumerated by `dx ls`, so it needs no upkeep), and a
  page-contract gate that mechanizes what was previously an eyeballed checklist (fold,
  scripts, external fetches, view sandbox). What cannot run confined is stated, not hidden:
  attacks (a kernel boundary is proven from outside), doc-shot (Chromium), and the
  execution-path tests (a sandbox does not nest) stay the host-shell `cargo test`. Proven
  live: an edit to `rust/` re-ran exactly the two cargo gates in ~4 s, everything else
  cached, no re-review demanded.
- **Directory `reads=`** (the field ask, and what makes the harness stale honestly).
  A `reads=` path may name a folder: `Resolver::files_under` (new, defaulted) expands it to
  every file under it — sorted, hidden entries and `target`/`node_modules` skipped, and
  anything under the block's own `writes=` grant excluded, because a result inside the
  fingerprint would stale the block's own verdict forever. Approval keeps naming the
  *declared* paths — new data re-runs, new powers re-review (`run_document` now derives the
  approval identity from `declared_read_paths`, not the expansion). Pinned:
  `a_reads_folder_expands_to_its_files_and_stales_with_them`,
  `a_reads_folder_leaves_out_what_the_block_writes`,
  `a_folder_read_walks_sorted_and_skips_hidden_entries_and_build_caches`. Documented in the
  format contract, README's grant-language section, `dx help` RUN, the `dx_run` schema, and
  the MCP handshake.
- **Fabricated measurements deleted.** `documents/compact-proof.dx`,
  `examples/compactness-comparison.{dx,md}`, and `examples/footprint-pair.{dx,md}` carried
  hand-typed numbers presented as measurements — the exact lie the verify-block doctrine
  exists to prevent (`route-economics.dx` and `dx stats` are the mechanical replacements).
  Their fixture pairs and all three mirror lists updated with them. The prose-only
  `documents/` checks (`final-validation`, `browser-check`, `virtual-check`) are superseded
  by `dev.dx` and the docs they duplicated; `documents/` is gone. The store deliberately
  keeps the deleted documents' content: manifests are what let `git show`/`git log -p`
  render old revisions forever.
- **Docs rewritten in the same wave.** README: the grant language (`reads=`/`writes=`, the
  approval asymmetry), the harness as the development loop, `::image` named in the agent
  section (it already existed — the field ask was discoverability), docs table updated.
  CLAUDE.md: workspace section, approval bullet (folder law), quality bar leads with the
  harness. `docs/grill-me.md`: execution questions updated to the exactly-three paths and
  the grant laws; the stale `document_css` question replaced. Format contract § Executable
  Code: folder expansion. HANDOFF compressed to this resume point.
- **Watch out for `dx set --header` with no body source** — it replaces the whole block, so
  an empty stdin silently empties the body (documented behavior, bit this session; part-31
  fixed the analogous bare-word case with a refusal). Candidate for the same treatment.

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
