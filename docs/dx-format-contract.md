# DX Format Contract

This document is the canonical behavior spec for .dx source in this repository.

## Goals

- Keep .dx deterministic and block-structured.
- Prevent parser drift between extension/webview and backend/database ingestion.
- Prevent in-document CSS from globally mutating rendered output.
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
- `code` attrs: `lang` or `language`, `run`, `deps`, `timeout`, `format`
- `output` attrs: `for`, `status`, `exit`, `hash`, `format`
- `image` attrs: `src`
- `nav` attrs: `label`
- `rule` attrs: none
- `html` attrs: none — author markup, rendered through the allow-list in `render::escape`
- `svg` attrs: none — a drawing, sanitized the same way
- `graph` / `mermaid` attrs: none — kept verbatim and shown as its own source
- `script` attrs: `type`, `src`, `module`
- `style` attrs: `media` — applied only when the caller opts in with `--doc-css`
- `stylesheet` attrs: `href`, `media` — same opt-in

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

## Executable Code and Captured Output

A `code` block marked `run` is executable. `deps` names the libraries to install before
running, `timeout` caps its runtime in seconds, and `format` (`svg` or `html`) declares
that the block prints markup which should be rendered rather than quoted.

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
- `hash` fingerprints the code plus its dependencies. A re-run whose fingerprint still
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

In-document CSS code blocks are content by default.

- Embedded CSS is not globally injected during rendered capture.
- Embedded CSS is not globally injected during normal read rendering.
- CSS only becomes active in scoped editing mode when the user targets a selector explicitly (id/class click flow).
- Closing the scoped CSS surface removes active scoped CSS from view state/rendering.

## Persisted View State Contract

Persisted view state (`.doc/view-state.json`) may include:

- `theme`, `resolvedTheme`
- `appearance`
- `viewport`
- `effectiveCss` (scoped-only, ephemeral)
- `sourceText`

Constraints:

- `effectiveCss` must be empty when no scoped selector session is active.
- Rendered capture must not synthesize global CSS from `sourceText`.

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
3. Capture rendered view: base style applies, in-doc CSS does not auto-apply globally.
4. Open scoped CSS editor via id/class interaction and verify scoped CSS applies only while active.

## If Corruption Is Detected

1. Restore canonical source through `saveDocumentSourceByRelativePath`.
2. Re-open viewer and confirm typed blocks are reconstructed.
3. Re-run capture and verify CSS safety contract.
4. Do not hand-edit `.doc` transport artifacts directly.

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
