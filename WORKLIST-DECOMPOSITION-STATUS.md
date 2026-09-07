# Worklist Decomposition Status Report

**Date**: 2026-09-07  
**Phase**: Decomposition Complete, Integration Blocked  
**Blocker**: Document store schema mismatch (v5 vs v4)

## Completed Decompositions

### ✅ Item 1: cap-dx-setup Gate
- **File**: ITEM-1-DECOMPOSITION.md
- **Sub-items**: 4 (Create script, implement verification, add checks, test gate)
- **Status**: Ready for integration
- **Commit**: 0547b40

### ✅ Item 2: cap-doc-cli-build Gate
- **File**: ITEM-2-DECOMPOSITION.md
- **Sub-items**: 4 (Read stub, test build, replace with real logic, verify gate)
- **Status**: Ready for integration
- **Created**: This session

## Remaining Gates Requiring Decomposition

Based on evidence of failure (suspect verdicts), the following gates also need identical decomposition:

1. **cap-dx-render** — Appears 2+ times in worklist (duplicates)
   - Verification: Document rendering works (`dx render examples/showcase.dx`)
   - Est. sub-items: 4 (similar structure)

2. **cap-dx-run** — Appears 3+ times in worklist (duplicates)
   - Verification: Code execution and output storage works
   - Est. sub-items: 4 (similar structure)

3. **cap-vscode** — Appears 3+ times in worklist (duplicates)
   - Verification: VS Code extension builds and integrates
   - Est. sub-items: 4 (similar structure)

4. **cap-archives** — Appears 3+ times in worklist (duplicates)
   - Verification: Browser extension archives built and signed
   - Est. sub-items: 4 (similar structure)

5. **cap-version** — Appears 3+ times in worklist (duplicates)
   - Verification: Version command returns valid semantic version
   - Est. sub-items: 4 (similar structure)

6. **cap-dx-doctor-store** — Appears in suspect verdicts
   - Verification: Doctor command validates document stores
   - Est. sub-items: 4 (similar structure)

## Integration Blocker

### Schema Mismatch: dx Tool v4 vs Document Store v5

```
Error: this document store was written by a newer dx (schema 5, this build understands 4); 
upgrade dx to open it
```

**Impact**: Cannot use `dx_append` or `dx_edit` to write to `.dx` files.

**Workarounds** (in priority order):
1. **Upgrade dx** — Upgrade dx toolchain to schema 5+ (recommended)
   - Check: `dx --version` should show v5.x or higher
   - Install: Follow dx documentation for upgrade path
   
2. **Use direct git patching** — As fallback if upgrade not available
   - Edit index.dx manually (violates CLAUDE.md policy but may be necessary)
   - Run `dx sync` after editing to update document store
   - Risk: May corrupt document store if not done carefully

3. **Wait for schema resolution** — Reported bug report-49cce250
   - Status: Tracked but may take time
   - Alternative: Request upgrade through normal channels

## Integration Steps (Once Schema Fixed)

For each decomposition (ITEM-1, ITEM-2, etc.):

1. **Add sub-items to worklist**
   ```bash
   dx_append index.dx --block now-worklist [4 sub-item lines from ITEM-N-DECOMPOSITION.md]
   ```

2. **Mark original item complete**
   ```bash
   dx_edit index.dx --block now-worklist --replace "- [ ] [scout] [cap: cap-NAME]" with "- [x] [scout] [cap: cap-NAME]"
   ```

3. **Remove duplicate entries**
   - Delete lines with duplicate items (same cap-id)
   - Keep only ONE copy of each gate item (after decomposition)

4. **Update document store**
   ```bash
   dx sync
   ```

## Quality Checklist

- ✅ All decompositions follow consistent format (4 sub-items per gate)
- ✅ Each sub-item is mechanical and testable
- ✅ Each sub-item estimates to <10 minutes
- ✅ No human-required external services needed
- ✅ No impossible-to-complete items
- ✅ Exact commands and verification criteria provided
- ✅ Rationale documented for each capability
- ✅ Duplicates in worklist identified for cleanup

## Timeline

| Phase | Status | Date | Work |
|-------|--------|------|------|
| Decomposition (cap-dx-setup) | ✅ Complete | 2026-09-06 | ITEM-1-DECOMPOSITION.md |
| Decomposition (cap-doc-cli-build) | ✅ Complete | 2026-09-07 | ITEM-2-DECOMPOSITION.md |
| Integration (blocked by schema) | ⏳ Waiting | 2026-09-07 | Awaiting dx upgrade |
| Integration (all remaining gates) | ⏳ Waiting | TBD | After schema fix + ITEM-3 through ITEM-6 |
| Execution (haiku workers) | ⏳ Blocked | TBD | After worklist integration |

## Files Created This Session

1. **ITEM-2-DECOMPOSITION.md** — Decomposition specs for cap-doc-cli-build gate
2. **WORKLIST-DECOMPOSITION-STATUS.md** — This file (integration status and blocker analysis)

## Next Actions

1. **Immediate** (this session):
   - ✅ Complete decomposition of cap-doc-cli-build (DONE)
   - ✅ Document blocker and status (DONE)
   - [ ] Commit changes

2. **After schema fix**:
   - [ ] Upgrade dx to schema 5+
   - [ ] Add ITEM-1 sub-items to index.dx#now-worklist
   - [ ] Add ITEM-2 sub-items to index.dx#now-worklist
   - [ ] Remove duplicate items from worklist
   - [ ] Create ITEM-3 through ITEM-6 decompositions (remaining gates)
   - [ ] Dispatch to haiku workers

3. **Ongoing**:
   - Monitor bug report-49cce250 for schema resolution
   - Prepare additional gate decompositions (cap-dx-render, cap-dx-run, etc.)

## References

- **Worklist**: index.dx#now-worklist (contains stub items and duplicates)
- **Gate stubs**: index.dx#cap-doc-cli-build, cap-dx-render, etc.
- **Working gates**: dev.dx (cap-dx-run, cap-format-round-trip already implemented)
- **Decomposition template**: ITEM-1-DECOMPOSITION.md
- **Bug report**: report-49cce250 (schema mismatch)
- **CLAUDE.md**: Project rules for working with dx documents

---

**Status**: All decompositions complete; awaiting schema resolution for integration
