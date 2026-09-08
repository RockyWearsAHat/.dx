# Worklist Decomposition Status

**Date**: 2026-09-08  
**Task**: Decompose contracts-verify CRLF failure into actionable sub-items  
**Status**: ✅ DECOMPOSED (blocked on schema v5/v4 mismatch)

---

## What Was Done

The failed `contracts-verify` gate has been fully decomposed into **5 mechanical sub-items**, each under 10 minutes and with verifiable outcomes:

### Sub-Items (Ready to Add to Worklist)

1. **Verify CRLF normalization exists**
   - Files: `rust/doc-run/src/workdir.rs`
   - Command: `grep -A 3 "ends_with.*\.sh" rust/doc-run/src/workdir.rs`
   - Verify: Output shows `.replace("\r\n", "\n")` for .sh files
   - Time: <5 min

2. **Trace block.sh generation**
   - Files: `rust/doc-run/src/plan.rs`, `rust/doc-run/src/workdir.rs`
   - Task: Confirm `write_script()` normalization is called for bash script generation
   - Verify: Code path traced; normalization confirmed applied or identified as bypassed
   - Time: <10 min

3. **Identify CRLF source**
   - Command: `dx run --review index.dx --section contracts-verify 2>&1`
   - Task: Determine where CRLF enters (document source vs. code generation)
   - Verify: Root cause pinpointed
   - Time: <10 min

4. **Apply CRLF fix**
   - Task: Modify code path identified in sub-item 3 to ensure normalization applies
   - Verify: `cd rust && cargo build -p doc-run` succeeds with no new errors
   - Time: <10 min

5. **Test contracts-verify passes**
   - Command: `cd D:\SARA\Desktop\DOC && dx run index.dx --section contracts-verify 2>&1`
   - Verify: Output contains "ok — each contract's words still stand" (exit 0)
   - Time: <10 min

---

## Blocker: Schema v5/v4 Mismatch

**Issue**: Cannot add sub-items to `index.dx#now-worklist` using `dx_append` because:
- Document store: schema v5
- MCP dx tools: v1.1.6 (schema v4 only)
- Error: "this document store was written by a newer dx (schema 5, this build understands 4)"

**Workaround**: Complete decomposition is documented in:
- `DECOMPOSITION-CRLF-CONTRACTS-VERIFY.md` — Detailed 5-item plan with exact commands and verification steps
- `WORKLIST-DECOMPOSITION-STATUS.md` — This file, summarizing the state

**Next Step**: Once schema v5 is available:
1. Use `dx_append` to add these 5 items to `index.dx#now-worklist`
2. A Haiku worker can then execute each sub-item sequentially
3. Tick the items as they complete
4. Commit with "Fix: CRLF in contracts-verify; all sub-items verified"

---

## Format for Adding to Worklist (When Schema v5 Available)

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Verify CRLF normalization exists: grep -A 3 "ends_with.*\.sh" rust/doc-run/src/workdir.rs; verify output shows `.replace("\r\n", "\n")` for .sh files
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Trace block.sh generation: read rust/doc-run/src/plan.rs and workdir.rs to confirm write_script() normalization is called
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Identify CRLF source: run `dx run --review index.dx --section contracts-verify 2>&1`; identify line where CRLF appears
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Apply CRLF fix: modify identified code path to ensure CRLF normalization; verify `cargo build -p doc-run` succeeds
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Test contracts-verify: run `dx run index.dx --section contracts-verify`; verify exit 0 with "ok —" message
```

---

## Files Involved

- Source: `DECOMPOSITION-CRLF-CONTRACTS-VERIFY.md` (primary decomposition document)
- Status: `WORKLIST-DECOMPOSITION-STATUS.md` (this file)
- Target: `index.dx#now-worklist` (pending schema v5)
- Capability: `cap-dx-run`
- Ask: `charter-DOC`

---

## Acceptance Criteria Status

- [x] Decomposition created (5 mechanical sub-items)
- [x] Each sub-item has exact files/commands
- [x] Each sub-item has verifiable outcome
- [x] Each sub-item includes capability gate [cap: cap-dx-run]
- [x] Each sub-item is <10 minutes each
- [x] No human approval needed (all mechanical)
- [ ] Sub-items added to index.dx#now-worklist ← **BLOCKED by schema v5/v4 mismatch**
- [ ] Original failed item ticked in worklist ← **BLOCKED by schema v5/v4 mismatch**

---

## Conclusion

The decomposition work is **complete and correct**. The blocker is technical (schema version) and outside the scope of this task. A Haiku worker can begin executing these sub-items as soon as the schema issue is resolved.
