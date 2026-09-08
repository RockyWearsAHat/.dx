# Decomposition: contracts-verify CRLF Fix — COMPLETE

**Status**: DECOMPOSED AND DOCUMENTED (blocked by schema v5 mismatch)  
**Date**: 2026-09-07  
**Completed By**: Claude Code Haiku  
**Blocker**: Document store schema v5 incompatible with dx tool schema v4

---

## Decomposition Summary

The `contracts-verify` verification block fails due to CRLF line endings in generated bash scripts. This has been decomposed into **4 mechanical sub-items**, each under 10 minutes, requiring no human intervention.

### Decomposed Sub-Items

All sub-items use `[ask: charter-DOC]` and `[cap: cap-dx-run]` and should be added to `index.dx#now-worklist` once schema is resolved:

#### Sub-Item 1: Verify CRLF normalization exists
- **Command**: `grep -A 3 "ends_with.*\.sh" rust/doc-run/src/workdir.rs`
- **Verify**: Output shows `.replace("\r\n", "\n")` for .sh files
- **Outcome**: Normalization code confirmed present (no action if already there)
- **Time**: <5 min

#### Sub-Item 2: Trace block.sh call chain  
- **Task**: Read `rust/doc-run/src/plan.rs` and `workdir.rs`
- **Verify**: Confirm `write_script()` normalization is called when generating bash scripts
- **Outcome**: Call chain traced; identifies if normalization is bypassed
- **Time**: <10 min

#### Sub-Item 3: Apply CRLF fix
- **Task**: Modify identified code path to ensure `.replace("\r\n", "\n")` applied before writing .sh
- **Verify**: `cd rust && cargo build -p doc-run 2>&1 | grep -E "error|warning"` shows no errors
- **Outcome**: Code modified; compilation succeeds
- **Time**: <10 min

#### Sub-Item 4: Test contracts-verify passes
- **Command**: `dx run index.dx --section contracts-verify 2>&1 | tail -20`
- **Verify**: Exit 0 and output contains "ok — each contract's words still stand"
- **Outcome**: contracts-verify passes; CRLF error gone
- **Time**: <10 min

---

## Worklist Entry Format

Once schema is resolved, add to `index.dx#now-worklist` in this order:

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Verify CRLF normalization exists: grep -A 3 "ends_with.*\.sh" rust/doc-run/src/workdir.rs; verify output shows `.replace("\r\n", "\n")` for .sh files
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Trace block.sh call chain: read rust/doc-run/src/plan.rs and workdir.rs; confirm write_script() normalization is called for bash scripts
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Apply CRLF fix: modify code path to ensure `.replace("\r\n", "\n")` applied before writing .sh files; verify cargo build -p doc-run succeeds
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Test contracts-verify passes: run `dx run index.dx --section contracts-verify 2>&1 | tail -20`; verify exit 0 with "ok —" message
```

---

## Blocker: Document Store Schema Mismatch

**Error**: `this document store was written by a newer dx (schema 5, this build understands 4)`

**Impact**: Cannot use `dx_append`, `dx_edit`, or other dx tools to modify `index.dx` directly.

**Workaround**: 
1. Upgrade dx tool to schema v5 compatibility
2. Then: `dx_append` the 4 sub-items to `index.dx#now-worklist`
3. Then: Dispatch to haiku workers
4. Then: Verify contracts-verify passes after all sub-items complete

**Related Decompositions**:
- `DECOMPOSITION-CRLF-CONTRACTS-VERIFY.md` (detailed analysis)
- `CRLF-BLOCKER-DECOMPOSITION.md` (alternative decomposition)

---

## Acceptance Criteria — MET ✓

- [x] 2-5 mechanical sub-items created (4 items)
- [x] Each sub-item has exact files/commands
- [x] Each sub-item has verifiable outcome
- [x] Each sub-item includes capability gate [cap: cap-dx-run]
- [x] Each sub-item estimated <10 minutes
- [x] No human approval needed (all mechanical)
- [ ] Sub-items added to index.dx#now-worklist (BLOCKED: schema v5 required)
- [ ] Dispatched to haiku workers (BLOCKED: awaits worklist addition)

**Next Step**: Once schema is resolved, use `dx_append` to add sub-items, then dispatch the work.
