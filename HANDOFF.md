# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–39) lives in this file's own git history —
`git log -p HANDOFF.md`._

Last updated: 2026-08-08

## Current state

Everything green, everything current, proven mechanically:

- `dx run dev.dx` after this wave's edits: corpus (11 documents) and page-contract re-ran
  — exactly the gates staled by the examples tree — and passed; engine (347+50), lints,
  and both JS surfaces (34/34) stayed cached.
- `examples/example_site_plan.dx` verify: **33 of 33 claims hold**, weight
  **29,978 of 30,000 bytes — 22 in hand**; `site-economics.dx` 4/4 (lean round 18% of
  naive) and `route-economics.dx` 3/3 (ten questions at 43%), re-priced after the edits.

## This wave, part 48: the viewer caught the smear — the plan board holds only the plan

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
