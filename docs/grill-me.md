# /grill-me

Use this checklist to challenge any .dx parser, renderer, or save-path change.

## Format Integrity

1. Can this change turn a valid block into a paragraph containing literal `::heading ... ::end` text?
2. Can this change emit single-line inline blocks on save?
3. Can this change emit synthetic `paragraph-N` wrappers?
4. Can this change lose list item boundaries during parse/save roundtrip?
5. Can this change strip `id` or `class` attributes from blocks?
6. Can this change mutate block order in roundtrip save?
7. Can this change parse `lang=css` but write back `language=` inconsistently?
8. Can this change silently swallow unknown block types?

9. Does a new attribute serialize to nothing when unset, so existing documents do not
   reformat on their next save?

## One Engine

There is a single parser and a single renderer (`rust/doc-core`), compiled natively for the
`dx` CLI and to wasm for the VS Code extension. Every question here is about keeping it that way.

1. Does this add a second place that parses or renders `.dx`?
2. Does the extension still render byte-identically to `dx render`?
3. Does any host (CLI, MCP, editor, screenshotter) reach around `doc-core` to format
   something itself?
4. If a view needs new information, is it exposed from `doc-core` rather than recomputed?

## Rendering Safety

1. Can prose reach the page without being escaped?
2. Can author markup (`::html`, `::svg`) keep a `<script>`, an `on*` handler, or a
   `javascript:` URL?
3. Can in-document CSS (`::style`, `::stylesheet`) affect rendering when `document_css` is
   off?
4. Does the rendered page reference anything external — a font, an image, a stylesheet?
5. Does non-ASCII prose (em dashes, accents, emoji, CJK) render, or does byte-index slicing
   panic on it?

## Execution Safety

1. Can reading, rendering, outlining, or screenshotting a document execute anything?
2. Is execution still reachable only through `dx run` and the `dx_run` tool?
3. Does `DX_NO_EXEC=1` still stop every block?
4. Can a block outlive its timeout, or wedge the process by filling a pipe?
5. Is a missing toolchain reported as a sentence naming what to install, rather than a crash?
6. Does the output fingerprint still change when the code or its dependencies change?
7. Is an `::output` block still inert data — never a thing that can run?

## Storage and Canonicalization

1. Is the on-disk `.dx` file still the whole content — readable with `cat`, searchable with
   `grep`, diffable in a pull request?
2. Does this introduce any state a `.dx` file must be kept in sync with?
3. Does every write go through the canonical writer, so two tools cannot fight over
   formatting?
4. Can a recovery path produce a non-canonical form?

## UX Failure Prevention

1. If parser recovery is triggered, do we visibly converge back to canonical source on next save?
2. Is there any path where malformed source remains sticky after successful save?
3. Can a user paste malformed one-line block syntax and keep the editor stable?
4. Are failures explicit, or silently converted into hard-to-debug state?
5. When something is missing (a browser, a runtime, a block id), does the message say what
   to do — and, for a bad id, list the ids that do exist?

## Done Criteria

A change is not done until all are true:

- `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` are clean.
- The byte-identical round-trip fixtures still pass, untouched.
- Block list remains typed (no command-text paragraphs).
- Save/reopen round-trip preserves block semantics.
- Reading a document still executes nothing.
- The editor and the CLI still render identically.
- Behavior matches `docs/dx-format-contract.md`, and that file was updated in the same edit
  if the behavior changed.
