# HANDOFF

_Resume point after `/compact` or `/clear`. Update after every task or wave (see CLAUDE.md)._

Last updated: 2026-07-31

## This wave: full-history audit, five defects fixed, in-editor diff restored, everything committed

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
