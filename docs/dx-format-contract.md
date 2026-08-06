# DX Format Contract

This document is the canonical behavior spec for .dx source in this repository.

## Goals

- Keep .dx deterministic and block-structured.
- Prevent parser drift between extension/webview and backend/database ingestion.
- Let a document dress itself identically on every surface, without letting its CSS fetch.
- Guarantee malformed intermediate states do not become persisted canonical state.

## Canonical Source Shape

A .dx document is a sequence of explicit block sections.

```text
::heading level=1 id=doc-hero
Welcome to DOC DB
::end

::paragraph id=intro
Body text
::end
```

### Canonical Writer Rules

- The writer emits one opening line per block (`::type ...`).
- The writer emits block body lines as-is.
- The writer emits one standalone `::end` line.
- The writer emits one blank line between blocks.
- The writer does not emit single-line `::... ::end` blocks.
- The writer assigns an unnamed block the id `<type>-<position>`, so an unnamed paragraph
  becomes `::paragraph id=paragraph-2`. **The reader must treat that as an ordinary block.**
  It used to be recognized as a legacy "synthetic wrapper" and unwrapped, which destroyed the
  writer's own output: writing a document with an unnamed paragraph and reading it back
  replaced the paragraph with a placeholder.
- The writer must not lose authored content. Two block bodies were previously written empty or
  truncated, silently erasing them on save:
  - `checklist` writes one `[x] text` / `[ ] text` line per item.
  - `bulleted-list` / `numbered-list` write nested items at two spaces of indent per level, and
    every item survives whatever its depth.

## Supported Blocks

Every block accepts `id` and `class`; the attributes below are what each adds.

- `heading` attrs: `level`
- `paragraph` attrs: none
- `quote` attrs: none
- `bulleted-list` attrs: none
- `numbered-list` attrs: none
- `checklist` attrs: none — one `[x] text` / `[ ] text` line per item
- `code` attrs: `lang` or `language`, `src`, `run`, `deps`, `reads`, `writes`, `timeout`,
  `format` — `src` names a sibling file whose **current text is the listing** (see
  References below); the stored body stays as written and unset `src` serializes to
  nothing. `reads` is a comma-separated list of sibling files the block's code reads at
  run time, and `writes` a comma-separated list of folders inside the document's folder
  the block may write; each is held to the path law, and unset either serializes to
  nothing
- `output` attrs: `for`, `status`, `exit`, `hash`, `format`
- `image` attrs: `src`
- `nav` attrs: `label`
- `rule` attrs: none
- `html` attrs: none — author markup, rendered through the allow-list in `render::escape`
- `svg` attrs: none — a drawing, sanitized the same way
- `graph` / `mermaid` attrs: none — kept verbatim and shown as its own source
- `board` attrs: `height` (viewport CSS px; unset means the renderer's default) — the body
  is one reference line per node, `- <block-id> x=N y=N w=N h=N to=a,b`, kept **verbatim**:
  the line grammar belongs to the renderer (`render::board`), unknown keys survive a
  round-trip, and each line names a block of the *same document* (usually one marked
  `hidden`, so it appears on the board and not in the page flow) — or, as a
  **cross-document reference**, one block of a *sibling document*: `- plan.dx#step-one
  x=20 y=20` shows `step-one` of `plan.dx`, current at every render (see References
  below). Saving a board body creates a hidden paragraph for any line naming a block the
  document does not have — but never for a cross-document reference, whose block lives
  in its own document.
  - `x y w h` are the node's **whole box** in canvas pixels, and the node is drawn at exactly
    that size. Nothing measures content to lay a board out, so a browser, a PNG, and `dx
    board` all place the same rectangles; content longer than the box scrolls inside it.
    `w`/`h` default to 280×180.
  - A dimension may also state a **rule** instead of a number: `w=page` / `h=page` take the
    page's own measures (the 680px column; the 480px default viewport), and `w=fit` / `h=fit`
    size the node to the block it shows by the engine's own deterministic estimate — resolved
    from nothing but the document, so every consumer still draws the identical box, and
    re-resolved as the block is rewritten. The words survive every rewrite (`w=page` never
    becomes `w=680`), and an edit that restates a rule's own resolved number keeps the rule;
    only a differing number replaces it.
  - A `to=` target may name the two sides the edge joins: `to=steps:b-t` leaves the bottom
    and arrives at the top (`l`, `r`, `t`, `b`, or `.` for "unpinned"). A bare `to=steps` is
    unpinned at both ends and takes whichever facing pair reads best, so every board written
    before sides existed still draws.
- `view` attrs: `src` — a sibling coded page, shown as the **page it renders to** (see
  References below); `width`/`height` — the framed viewport in its own CSS pixels
  (defaults 1180×760), so `width=390` is the page genuinely laid out at phone width. The
  frame is scaled uniformly into the space it is shown in — the page column, or a board
  node's stated box, which a view fills edge to edge. The stored body stays as written
  (empty for a `src=` view), and unset attributes serialize to nothing. The page renders
  inside an `<iframe sandbox="">` — an opaque origin allowed nothing — so a view is only
  ever *shown*: nothing in it runs, submits, navigates, or reads the document's page. A
  view may also carry an inline body instead of `src`: the body is framed the same way.
- `script` attrs: `type`, `src`, `module`
- `style` attrs: `media` — wraps the block's CSS in that media query
- `stylesheet` attrs: `href`, `media` — imported ahead of every `::style` rule

An unrecognized `::type` folds to `paragraph` and keeps its text, so a document written by a
newer `dx` never loses content when an older one reads it.

All block types also support boolean presence attrs:

- `hidden` (equivalent to `hidden=true`)
- `module` (for `script` blocks, equivalent to `module=true`)
- `run` (for `code` blocks, equivalent to `run=true`)

## Navigation

A `nav` block is a list of places to go. Its body holds one target per line, written like a
list item, and an indented line nests under the one above it:

```text
::nav id=side class=sidebar label="{n}. {name}"
- [Install first](setup.dx)
- api.dx#errors
- #results
::end
```

Two forms, and the difference is where the name comes from:

| Entry | Name shown |
|-------|------------|
| `[Install first](setup.dx)` | `Install first` — the author wrote it, and `label` does not apply |
| `api.dx#errors` | Expanded from `label`, which defaults to `{name}` |

`label` tokens, expanded per entry:

| Token | Value |
|-------|-------|
| `{name}` | The most specific name the target carries: for `#id` in this document, the text of the heading it addresses; otherwise the fragment, then the file stem |
| `{target}` | The target exactly as written |
| `{path}` | The path half of the target, without the fragment |
| `{n}` | The entry's 1-based position |

An unknown token is left as written rather than blanked — a name is prose the author chose, and
deleting part of it silently is worse than showing a brace.

**An empty body means this document's own contents:** every heading below the title, in order,
nested by level. That is the whole feature for most documents — one block, nothing to maintain:

```text
::nav id=contents

::end
```

Two consequences of that rule are part of the contract. Normalization never invents an entry for
an empty `nav` — unlike a list, which gets a placeholder item — because the empty body *is* the
instruction. And a `nav` that resolves to nothing renders as nothing, not as an empty list.

Resolution is a pure function of the document it sits in: it never reads another file. A
cross-document entry is named from its target text alone. This is why the editor, the CLI, an
agent, and the GitHub extension all show the same navigation — the renderer that compiles to
wasm has no filesystem to consult, so there is nothing to disagree about.

## References — one source of truth, shown current everywhere

A document may name four things it does not itself carry, so content lives once and every
page showing it stays current:

- **A sibling file**: `::code id=listing src=src/lib.rs lang=rust` renders the file's
  current text as its listing, and `run` executes that text. The file is the source of
  truth; the document reviews it.
- **A sibling coded page**: `::view id=screen src=site/index.html width=390` shows the
  page the file renders to, framed live at the stated viewport — the same reference the
  other way up. Hydration also inlines the page's own *relative* stylesheets
  (`resolve::file_references` tells a prefetching host about them), because the sandboxed
  frame has no folder to fetch from; absolute stylesheets and images stay the frame's own
  to fetch, exactly as a browser would. The `src` may carry a fragment
  (`src=site/index.html#visit`): the file before the `#` is read, and the frame shows
  just that element — one screen per section of a page too tall for one frame. Only a
  fragment that is a plain id (`[A-Za-z0-9_-]+`) is one; any other `#` stays part of the
  filename. The crop is a selector appended to the hydrated page, never a script — a
  sandboxed frame has no URL to scroll, and no render may need a script.
- **One block of a sibling document**: a board node line `- plan.dx#step-one x= y=`
  draws that block on this board, resolved fresh at every render.
- **A sibling picture**: `::image id=shot src=shots/frame.png` embeds the file's current
  bytes as a `data:` URI at hydration, so the rendered page carries its own artwork
  wherever it is shown — a capture made from a scratch directory, an editor webview, a
  PNG export. Raster formats only (`png`, `jpg`/`jpeg`, `gif`, `webp` — the same
  allow-list as `render::escape`'s `data:` images); SVG is a document that can script,
  so it is never inlined and the reference stays as written. A remote URL or a `data:`
  URI in `src` travels exactly as written. An image file the resolver cannot produce is
  a sentence in the image's alt text, naming the path.

The rules (`doc_core::resolve` is the implementation, and there is exactly one):

- **The path law (`resolve::confined`).** A reference is a relative path walking downward
  from the document's own folder. Absolute paths, `~`, anything containing `:` or `\`,
  and any `.` or `..` segment are refused before any resolver is asked — a document is
  something you were handed, and rendering it must not read a file it has no claim to.
- **Hydration is a read, and is never saved.** `resolve::hydrate` fills references into a
  parsed, in-memory copy at view/run time. The stored document keeps the reference and an
  empty body; unset `src` serializes to nothing, so the change is additive. Serializing a
  hydrated document is a defect — it would turn references back into copies.
- **The engine decides, hosts transport.** Every surface hands bytes to the same
  hydration through a `Resolver`: the CLI and MCP read the document's folder (sibling
  `.dx` through the store, so a pointer resolves to true content), VS Code reads the
  workspace (pointers through `.doc/repo.dxcp`), the GitHub extension gathers what
  `references` lists — documents from the committed pack, files from the repository's own
  session-carrying raw route — and passes them into `render_html`. No host re-implements
  the grammar.
- **A reference that resolves to nothing is a sentence in the block's place** naming the
  path — never silence, never an empty block. `dx run` refuses to execute such a listing:
  the block is recorded `blocked`, because executing a sentence about a missing file
  helps nobody.
- **Fingerprints track the file.** A `src` listing's output hash is computed over the
  file's current text, so editing the file makes the recorded output stale exactly as
  editing an inline body would — the documentation is the test surface.
- **A foreign block's own references stay its own.** A board node showing
  `sub/plan.dx#listing` whose block says `src=main.rs` resolves that file as
  `sub/main.rs` — relative to the document that owns the listing, re-checked against the
  path law.

## Executable Code and Captured Output

A `code` block marked `run` is executable. `deps` names the libraries to install before
running, `timeout` caps its runtime in seconds, and `format` (`svg` or `html`) declares
that the block prints markup which should be rendered rather than quoted. `reads` names
the sibling files the code reads (comma-separated, path-law-confined): their current text
joins the run fingerprint (`hash=`), so editing a declared file makes the recorded output
stale exactly like editing the code would — the record never claims "no changes" about
content it read. Review's approval is a separate, narrower identity: the code and its
powers (runner, deps, code, the `reads=` *paths*, the `writes=` grant), never the declared
files' text — so an edited input re-runs approved code by itself, while an edited block or
header re-opens review. A declared file that cannot be resolved blocks the run with a
sentence, never a silent fingerprint that omits it.

`writes` names the folders of the document's own the block may write (comma-separated,
path-law-confined, created if missing) — the sandbox otherwise keeps everything but the
block's scratch directory read-only. The grant joins the fingerprint, so review prints it
and widening a grant re-opens review exactly like editing the code; the `.doc` store and
any path that resolves outside the document's folder (a symlink on the way) are refused
with a sentence. The grant never includes the network.

Running a document writes one `output` block immediately after each code block it ran:

```text
::code id=stats lang=python run deps=numpy
print(1)
::end

::output id=stats-output for=stats status=ok hash=b1323d9097e9ba05
1
::end
```

Contract for `output` blocks:

- `for` names the `code` block that produced it; the writer places it directly after that
  block, and re-running replaces it rather than appending a second one.
- `status` is `ok`, `error`, or `blocked` (no toolchain, or execution disabled).
- `exit` is the process exit code and is omitted when it is `0`.
- `hash` fingerprints the code, its dependencies, the current text of every file the
  block declares with `reads`, and the `writes` grant. A re-run whose fingerprint still
  matches is skipped and the recorded output is left untouched.
- `format` is copied from the code block, and only affects rendering: a failed block always
  renders as text so the error is legible.
- An `output` block is data, never executable: nothing in the parser, renderer, or
  screenshotter runs code. Only `dx run` / the `dx_run` tool does.

## Parsing and Recovery Rules

Parser must accept and normalize these non-canonical forms for recovery only:

- Single-line inline blocks: `::heading ... text ::end`
- Trailing close token on same line as content: `... } ::end`
- An unrecognized `::type` opening, which folds to `paragraph` without leaving literal
  `::type` markers in the text

Recovery behavior:

- Parse into proper typed blocks.
- Normalize IDs/classes.
- Persist back to canonical multi-line block form on next save.

**Recovery must never cost content.** The trailing close token is recognized only when
whitespace precedes it and nothing but whitespace follows — the same shape a single-line block
has. Matching `::end` anywhere in a line instead meant a document that *described* the format
lost text: a sentence reading "a block ends with `::end` on its own line" was cut at the
backtick, and an SVG label containing the token truncated the drawing and everything under it.
A block closes on a line of its own, or at the end of a line, never mid-sentence.

## Placeholders

A `{{key}}` in any block's text is replaced with the value a JSON `script` block declared for
that key (see [Supported Blocks](#supported-blocks)). Only top-level scalars participate;
nested objects and arrays are skipped rather than stringified.

**An unknown key is left exactly as written.** Blanking it deletes the author's characters with
nothing to show that it happened — a typo in a key erases the word it stood for, and a document
explaining `{{name}}` loses the token mid-sentence. This is the rule `nav` labels already
follow, and a malformed `{{ not a key }}` has always survived; the well-formed unknown was the
odd case out.

## CSS Safety Contract

A document's own CSS **always applies**, on every surface: `dx render`, `dx png`, DX.app, the
VS Code preview, the local daemon, and the github.com extension. There is no flag, because a
`::style` block a reader has to know a switch for is a block that silently does nothing.

Where it goes is part of the contract. The `<style data-dx-document-css>` element is emitted
**inside `.dx-doc`**, ahead of the blocks, never in the page `<head>` — `.dx-doc` is the unit
every editing host swaps on save, and a stylesheet stranded in a `<head>` the host never
re-reads is a document that loses its dress the moment anyone touches it. Page and fragment
renders therefore carry byte-identical CSS.

What CSS may do is bounded, because a `.dx` is something you were handed:

- **It may dress.** Selectors, properties, `@media`, and `url(data:image/…)` artwork the
  document carries itself all pass through untouched.
- **It may not fetch.** A `url(…)` naming a host is rewritten to an empty `url()`. A
  `background: url(https://…)` fires on render, which turns reading a document into telling
  its author you read it — and on github.com, telling a stranger your IP.
- **It may not execute.** `@import`, `expression(`, `behavior:`, and `-moz-binding` are
  neutralized in `::style` bodies, and `</style` can never close the element early.

Neutralized, not deleted: a mangled property is a rule the browser ignores, so the
surrounding stylesheet still parses and the author sees one thing not work rather than their
whole dress falling off at the first bad line.

A `::stylesheet` block is the one deliberate exception to "may not fetch" — requesting a
remote sheet is the block's entire stated purpose, so an author who writes one gets one. Its
`href` is held to relative paths and `http(s)` only; `javascript:`, `data:`, and
scheme-relative `//host` are refused, since an `@import` runs whatever it resolves to with
the document's own privileges. Imports are emitted ahead of every rule, as CSS requires.

`render::escape::escape_style` is the authority for all of the above, and
`document_css_may_dress_a_page_but_not_fetch_from_it` in `render::html` is its evidence.

## Non-Negotiable Invariants

- Backend parser and webview parser must accept the same block grammar.
- Save path must never downgrade block syntax into plain paragraphs containing `::...` text.
- Reingest must preserve typed block semantics.
- Corrupted intermediate forms must be recoverable and normalized.

## Operational Checks

Before shipping parser/render changes:

1. Save and reopen `examples/welcome.dx`.
2. Inspect viewer block list: command lines appear as their own blocks, and a
   `::paragraph id=paragraph-N` block round-trips unchanged.
3. Capture rendered view: base style applies, and so does the document's own `::style`.
4. Confirm a `::style` carrying `url(https://…)` renders with an empty `url()`.

## If Corruption Is Detected

1. Run `dx sync` in the repository: it adopts plain-text `.dx` files, restores documents from
   the packs when the index is missing, and reports what it cannot resolve.
2. Re-open the document and confirm typed blocks are reconstructed.
3. Do not hand-edit `.doc` pack artifacts directly.

## Storage

The format above describes a document's **canonical text**. That text is what is stored, and
how it is stored is part of the contract because it is what makes storage lossless.

- A document is split into one **chunk** per block. A chunk's payload is byte-for-byte the
  canonical text the writer emits for that block, with no trailing newline.
- A chunk is addressed by the lowercase hex SHA-256 of those bytes. Identical blocks — across
  documents, or across versions of one document — are one chunk.
- Reassembly is concatenation: join a document's chunk texts with a blank line between them and
  append one trailing newline, and the result equals the canonical file exactly. Storage
  therefore cannot drop a block attribute it does not know about, which a structured binary
  encoding both can and did.
- A **manifest** is one version of a document: its source digest and its ordered chunk list.
- On disk a `.dx` file is a one-line pointer, `~ dx1 <64 hex digits>`, naming the digest of the
  document's canonical source. A file is a pointer only if its first line matches that form
  exactly; anything else is content to be adopted, never discarded.
- A **pack** (`DXCP1`) is a set of documents in one file: distinct chunk bodies once each,
  entries referencing them by index, and the whole payload compressed as a single stream so
  redundancy *between* blocks compresses too. `.doc/repo.dxcp` is committed; it is the content.

### Compression frames

The pack's payload is wrapped in a `dxz` frame whose first four bytes name the codec that wrote
it. That indirection is what lets compaction improve without a migration, and what guarantees a
pack written by an older build still reads.

| Magic | Codec | Written | Read |
|-------|-------|---------|------|
| `DXZ1` | Self-contained LZSS | no longer | **always** — packs in real repositories contain it |
| `DXZ2` | DEFLATE, pure Rust (`miniz_oxide`) | when smallest | yes |
| `DXZ3` | Stored, uncompressed | when nothing beats it | yes |

Two properties follow, and both are tested:

- **Compression never inflates.** The smallest encoding wins, and storing the bytes as they are
  is always one of the candidates, so the stored form never exceeds the input plus an 8-byte
  header — however incompressible the content.
- **A decoded payload is checked against the length its frame declares.** A payload that
  decodes to the wrong size is an error, never a short read.

Measured on this repository's six examples: 17,381 bytes of canonical source become a
5,982-byte pack — 34.4%, byte-for-byte recoverable. The previous LZSS-only codec produced
49.3% of source from the same corpus.

A heavier codec (brotli, LZMA) would save a few points more. DEFLATE was chosen because it is
pure Rust with no `unsafe`, compiles to `wasm32` with the rest of the engine, and keeps the
editor's bundle small. Adding one later means adding a magic, not converting anything.
