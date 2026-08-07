# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md).
The full wave-by-wave history (parts 1–39) lives in this file's own git history —
`git log -p HANDOFF.md`._

Last updated: 2026-08-07

## Current state

Everything green, everything current, proven mechanically:

- `cargo test` **828/828** (attacks 18/18 among them), clippy `-D warnings` clean,
  `fmt --check` clean, both node suites 34/34, fixture corpus and drift guard green.
- `dx run dev.dx`: all five gates ok — this wave's `rust/` edit re-ran exactly the three
  gates that read it (engine, lints, surfaces); corpus and page-contract stayed cached.
- Release binary reinstalled to `~/.local/bin/dx` (**rm + cp**, not cp-in-place — macOS
  SIGKILLs a cp over a running signed binary), both wasm engines rebuilt
  (`editor/build.sh`).

**One last manual restart:** running MCP servers predate the drift check below, so restart
the assistant once more. After that the class is closed — servers pick up new binaries
themselves.

## This wave, part 41: the server that stays current, and the search that answers

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

Next step: restart the assistant once (the last time this is ever needed — see part 41),
then let a field session measure the new economy: search-with-answer and per-block image
reads against the part-40 baseline of ~10 calls before the first edit.
