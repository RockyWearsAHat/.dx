# Scout Status Report

## Current State
- **Understanding**: ✅ Complete
- **Capability Gates**: 1/7 working (6 stubs need implementation)
- **Worklist Items**: ✅ Identified (5 priority items documented below)
- **Blocker**: ❌ Schema version mismatch prevents dx_append updates

## Schema Blocker Details
Document store is at schema v5, but available dx tools understand v4. This prevents using `dx_append` or `dx_edit` to update the now-worklist block.

**Resolution required**: Upgrade dx installation to v5 or downgrade document store to v4

## Ready-to-Dispatch Worklist Items

Once schema is resolved, add these 5 items to `index.dx#now-worklist` in order:

```
- [scout] [ask: charter-DOC] [cap: cap-format-round-trip] Implement cap-format-round-trip gate: verify documents round-trip byte-for-byte through parse/stringify cycle; add `::code run` block with `set -euo pipefail`, run test fixtures through parse→stringify, exit 0 only when all remain identical

- [scout] [ask: charter-DOC] [cap: cap-store-integrity] Implement cap-store-integrity gate: verify storage cannot lose bytes in pack/unpack cycles; add `::code run` block with `set -euo pipefail`, run `dx sync` full cycle, verify all documents survive with matching checksums, exit 0 only on success

- [scout] [ask: charter-DOC] [cap: cap-dx-render] Replace stub gate cap-dx-render with real check: currently contains only a comment and exits 0 without verifying; rewrite block with `set -euo pipefail`, run `cargo test --lib render` or equivalent real rendering tests, exit 0 only when tests pass

- [scout] [ask: charter-DOC] [cap: cap-dx-run] Replace stub gate cap-dx-run with real check: currently contains only a comment and exits 0 without verifying; rewrite block with `set -euo pipefail`, run `cargo test --lib run` or equivalent execution tests, exit 0 only when tests pass

- [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Replace stub gate cap-doc-cli-build with real check: currently contains only a comment and exits 0 without verifying; rewrite block with `set -euo pipefail`, run `cargo build -p doc-cli` from rust/ directory, verify binary exists and is executable, exit 0 only on success
```

## Full Details
See `SCOUT-PHASE-3-SUMMARY.md` for detailed specifications of each item, secondary items, and notes on prior cleanup work.

## Next Steps
1. **Human action**: Resolve schema version (upgrade dx or downgrade store)
2. **Add items to worklist**: Use `dx_append` or `dx_edit` to add the 5 items to now-worklist
3. **Dispatch**: Items will be available for workers immediately after worklist update
4. **Verify**: Run `dx run dev.dx` to check gate status as items are implemented

## Success Criteria
- All 5 items in now-worklist block
- Items map to failing gates (cap-format-round-trip through cap-doc-cli-build)
- Gates transition from stub (exit 0 always) to real verification (exit 0 only on success)
- Final state: gates 6/7 pass (or 5/7 if sandbox-cross-platform is deferred)
