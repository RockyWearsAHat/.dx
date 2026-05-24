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
- The writer does not emit synthetic `paragraph-N` wrappers.
- The writer does not emit single-line `::... ::end` blocks.

## Supported Blocks

- `heading` attrs: `level`, `id`, `class`
- `paragraph` attrs: `id`, `class`
- `quote` attrs: `id`, `class`
- `bulleted-list` attrs: `id`, `class`
- `numbered-list` attrs: `id`, `class`
- `code` attrs: `id`, `class`, `lang` or `language`
- `image` attrs: `id`, `class`, `src`
- `rule` attrs: `id`, `class`
- `script` attrs: `id`, `class`, `type`, `src`, `module`

All block types also support boolean presence attrs:

- `hidden` (equivalent to `hidden=true`)
- `module` (for `script` blocks, equivalent to `module=true`)

## Parsing and Recovery Rules

Parser must accept and normalize these non-canonical forms for recovery only:

- Single-line inline blocks: `::heading ... text ::end`
- Trailing close token on same line as content: `... } ::end`
- Synthetic paragraph wrappers from old broken state:
  - `::paragraph id=paragraph-123`
  - `<wrapped line>`
  - `::end`

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

View state in SQLite may include:

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
2. Inspect viewer block list: no synthetic `paragraph-*` wrappers for command lines.
3. Capture rendered view: base style applies, in-doc CSS does not auto-apply globally.
4. Open scoped CSS editor via id/class interaction and verify scoped CSS applies only while active.

## If Corruption Is Detected

1. Restore canonical source through `saveDocumentSourceByRelativePath`.
2. Re-open viewer and confirm typed blocks are reconstructed.
3. Re-run capture and verify CSS safety contract.
4. Do not hand-edit `.doc` transport artifacts directly.
