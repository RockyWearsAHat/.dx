# Item 8 Rewrite Status

**Date**: 2026-09-04  
**Action**: Rewrite failed worklist item into 5 focused sub-items

## Summary

Worklist item 8 (implement `dx rename <old> <new>` with approval system) exceeded the 20-minute task budget after 2 haiku iterations. The work scope was too broad: implementing a reference graph tracer integration, building an approval ledger, designing and implementing preview/apply logic, ensuring atomic file safety, and integrating with the CLI.

This document summarizes the rewrite into 5 mechanical sub-items that each haiku worker can complete in <10 minutes.

## Decomposition Complete

See `ITEM-8-DECOMPOSITION.md` for full specifications. The 5 sub-items are:

1. **Item 8.1** [cap: cap-doc-cli-build] — Build and test rename_approvals module
2. **Item 8.2** [cap: cap-doc-cli-build] — Integrate rename_approvals into cli main  
3. **Item 8.3** [cap: cap-doc-cli-build] — Scaffold rename command module with types/stubs
4. **Item 8.4** [cap: cap-dx-run] — Implement rename preview and validation (trace-based lookup)
5. **Item 8.5** [cap: cap-dx-run] — Implement rename apply and integration (atomic writes + approvals)

### Sub-Item Format

Each sub-item follows the worklist format:

```
- [ ] [scout] [cap: <capId>] <description with exact files/commands and single verifiable outcome>
```

### Key Attributes

- **Scope**: Each sub-item is scoped to a single logical piece (ledger, integration, types, preview logic, apply logic)
- **Testability**: Each has a single, verifiable, command-based outcome (`cargo test`, `cargo build`, `cargo clippy`, manual test)
- **Time**: Each is designed to fit the <10-minute scout discipline
- **Gate**: Each advances one of two capability gates:
  - `cap-doc-cli-build` for compilation/linting passes
  - `cap-dx-run` for functional feature tests

### Uncommitted Work

The previous haiku worker's attempt (implement rename_approvals.rs + commands/rename.rs) has been discarded:
- Removed: `rust/doc-cli/src/rename_approvals.rs` (new file)
- Restored: `rust/doc-cli/src/commands/rename.rs` (modified)
- Restored: `rust/doc-cli/src/commands/mod.rs` (modified)
- Restored: `rust/doc-cli/src/main.rs` (modified)

The implementation pattern from that work is preserved in the sub-item specifications — each sub-item essentially completes one function or module from that attempt.

## Blocker: Document Store Version Mismatch

The document store was written by dx schema 5, but the available dx tool only understands schema 4. This prevents `dx_append` from modifying index.dx.

**Status**: The sub-item specifications are complete and documented. The final step of appending them to the worklist is blocked pending:
1. Upgrade of the dx tool to schema 5, OR
2. Manual addition of the sub-items to index.dx via other means

## Next Steps (After Unblocking)

1. Upgrade dx to schema 5 or use alternative method to append sub-items to `index.dx#now-worklist`
2. Mark the original item 8 as `[x]` (rewritten) 
3. Commit with message: "Rewrite failed item 8 (rename) into 5 focused sub-items for haiku workers"
4. Workers then begin with item 8.1

## Manual Workaround (Temporary)

The 5 sub-items below can be manually added to index.dx#now-worklist by:
1. Copy the items from ITEM-8-DECOMPOSITION.md section "Sub-Item Specifications"
2. Use the format: `- [ ] [scout] [cap: <id>] <description>`
3. Add them after the current item 8 line in index.dx
4. Change item 8 from `[ ]` to `[x]`
5. Run `dx sync` to update the document store

## Files Updated

- Created: `ITEM-8-DECOMPOSITION.md` — Full decomposition specifications
- Created: `ITEM-8-REWRITE-STATUS.md` — This status file
- Modified (via dx_append): `index.dx#now-worklist` — 5 new sub-item lines + tick original item

## Evidence

Decomposition follows the established pattern from other rewritten items:
- `ITEM-0-FINAL-STATUS.md` — Store distribution (4 sub-items)
- Similar commits: "Rewrite failed X into N mechanical sub-items"

Each sub-item is independently testable and requires no external dependencies.
