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

- `heading` attrs: `level`, `id`, `class`
- `paragraph` attrs: `id`, `class`
- `quote` attrs: `id`, `class`
- `bulleted-list` attrs: `id`, `class`
- `numbered-list` attrs: `id`, `class`
- `code` attrs: `id`, `class`, `lang` or `language`, `run`, `deps`, `timeout`, `format`
- `output` attrs: `id`, `class`, `for`, `status`, `exit`, `hash`, `format`
- `image` attrs: `id`, `class`, `src`
- `rule` attrs: `id`, `class`
- `script` attrs: `id`, `class`, `type`, `src`, `module`

All block types also support boolean presence attrs:

- `hidden` (equivalent to `hidden=true`)
- `module` (for `script` blocks, equivalent to `module=true`)
- `run` (for `code` blocks, equivalent to `run=true`)

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
