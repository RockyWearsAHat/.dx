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

## Parser Parity

1. Do backend and webview parse the same malformed inline forms the same way?
2. Do both parsers recover from `... ::end` on the same line?
3. Do both parsers ignore orphan `::end` safely?
4. Do both parsers normalize wrappers the same way?
5. If one parser recovers a form and the other does not, why is that acceptable?

## CSS Safety

1. Can in-document CSS affect rendered output without selector-targeted activation?
2. Is `effectiveCss` empty when no scoped CSS surface is active?
3. Does closing scoped CSS clear active style injection?
4. Can persisted view state accidentally re-enable global CSS on reopen?
5. Can capture pipeline infer CSS from source text and bypass safety policy?

## Storage and Canonicalization

1. Does SQLite remain canonical for source/view state after this change?
2. Is `.doc` transport mirrored from DB, not treated as source of truth?
3. Can ingest from stub/archive produce different block semantics than direct save?
4. Can source reconstruction produce a non-canonical form?

## UX Failure Prevention

1. If parser recovery is triggered, do we visibly converge back to canonical source on next save?
2. Is there any path where malformed source remains sticky after successful save?
3. Can a user copy/paste malformed one-line block syntax and keep the editor stable?
4. Are failures explicit, or silently converted into hard-to-debug state?

## Done Criteria

A change is not done until all are true:

- Block list remains typed (no command-text paragraphs).
- Save/reopen roundtrip preserves block semantics.
- Render capture respects CSS safety contract.
- Recovery paths normalize malformed input to canonical output.
- Behavior matches docs in `docs/dx-format-contract.md`.
