# Decomposition: Fix CRLF Line Ending in contracts-verify Block

**Status**: DECOMPOSED (ready to add to worklist once schema v5 is available)  
**Date**: 2026-09-07  
**Reason**: contracts-verify gate fails with bash syntax error due to CRLF line endings in generated shell scripts  
**Error**: `syntax error near unexpected token $'do\r'`  

---

## Problem Statement

The `contracts-verify` verification block in `index.dx` fails to run on macOS/Linux because generated bash scripts contain Windows CRLF line endings (`\r\n`) instead of Unix LF (`\n`). Bash interprets the carriage return as a syntax error.

### Evidence

```
/Users/alexwaldmann/.cache/dx-run/bash/.../block.sh: line 4: syntax error near unexpected token `$'do\r''
/Users/alexwaldmann/.cache/dx-run/bash/.../block.sh: line 4: `  for word in "$@"; do'
```

The error occurs at line 4 of the generated script which contains: `for word in "$@"; do` - the `\r` at the end makes bash fail to parse it.

---

## Impact

- **Blocked**: `index.dx#contracts-verify` cannot verify contract words exist in source files
- **Blocked**: `index.dx#verify` cannot run complete verification suite
- **Blocked**: Capability gates cannot run their verification checks
- **Gate Status**: cap-dx-run remains in "suspect" state
- **Worklist**: Cannot progress until CRLF issue is resolved

---

## Root Cause Analysis

The issue occurs in the doc-run module where bash scripts are generated and executed:

1. **Generation**: Code block content is passed to `doc-run` for execution
2. **Script Writing**: `doc-run` writes the code to a temporary `block.sh` file
3. **Normalization**: CRLF normalization code exists in `rust/doc-run/src/workdir.rs` (lines ~79-82):
   ```rust
   let normalized = if name.ends_with(".sh") {
       contents.replace("\r\n", "\n")
   } else {
       contents.to_string()
   };
   ```
4. **Problem**: Either:
   - The normalization code is not being called for this code path, OR
   - The CRLF comes from a source that bypasses the normalization, OR
   - The normalization is happening but too late in the pipeline

---

## Decomposed Sub-Items

These sub-items are ready to add to `index.dx#now-worklist` once the document store schema v5 support is available. Each item is mechanical, under 10 minutes, and has a verifiable outcome.

### Sub-Item 1: Verify CRLF normalization code exists
- **ID**: [cap: cap-dx-run]
- **Ask**: charter-DOC
- **Task**: Confirm that CRLF→LF normalization code exists for bash script writes
- **Command**: `grep -A 3 "ends_with.*\.sh" rust/doc-run/src/workdir.rs`
- **Verify**: Output shows `.replace("\r\n", "\n")` for .sh files
- **Outcome**: CRLF normalization code confirmed present (no action needed if present)
- **Time**: <5 minutes

### Sub-Item 2: Trace block.sh generation call chain
- **ID**: [cap: cap-dx-run]
- **Ask**: charter-DOC
- **Task**: Confirm that `write_script()` from workdir.rs is called when generating block.sh
- **Files to read**: 
  - `rust/doc-run/src/plan.rs` (how bash plans are created)
  - `rust/doc-run/src/workdir.rs` (where scripts are written)
- **Verify**: Source code shows that block.sh goes through the normalization function
- **Outcome**: Call chain traced; either confirms normalization is applied or identifies where it's bypassed
- **Time**: <10 minutes

### Sub-Item 3: Identify where CRLF enters the pipeline
- **ID**: [cap: cap-dx-run]
- **Ask**: charter-DOC
- **Task**: Determine the source of CRLF - either document content or generated wrapper code
- **Command**: Run `dx run --review index.dx --section contracts-verify 2>&1` and capture stderr
- **Verify**: Error output identifies the exact line where CRLF appears in the generated script
- **Outcome**: Root cause pinpointed (document source vs. code generation)
- **Time**: <10 minutes

### Sub-Item 4: Apply fix to eliminate CRLF
- **ID**: [cap: cap-dx-run]
- **Ask**: charter-DOC
- **Task**: Modify the code path (identified in Sub-Item 3) to ensure CRLF normalization applies
- **If doc-run/src/workdir.rs**: Verify `write_script()` is called with `normalized` content
- **If elsewhere**: Add `.replace("\r\n", "\n")` before writing the .sh file
- **Verify**: `cd rust && cargo build -p doc-run 2>&1 | grep -E "error|warning"` shows no new errors
- **Outcome**: Code modified; compilation succeeds
- **Time**: <10 minutes

### Sub-Item 5: Test contracts-verify passes
- **ID**: [cap: cap-dx-run]
- **Ask**: charter-DOC
- **Task**: Verify that contracts-verify block now runs without CRLF syntax errors
- **Command**: `cd D:\SARA\Desktop\DOC && dx run index.dx --section contracts-verify 2>&1 | tail -20`
- **Verify**: Output contains "ok — each contract's words still stand in its owning module" (exit 0)
- **Outcome**: contracts-verify passes; bash syntax error gone
- **Time**: <10 minutes

---

## Format for Worklist Addition

Once the document store schema is upgraded to v5, add these as checklist items to `index.dx#now-worklist`:

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Verify CRLF normalization exists: grep -A 3 "ends_with.*\.sh" rust/doc-run/src/workdir.rs; verify output shows `.replace("\r\n", "\n")` for .sh files
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Trace block.sh generation: read rust/doc-run/src/plan.rs and workdir.rs to confirm write_script() normalization is called
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Identify CRLF source: run `dx run --review index.dx --section contracts-verify 2>&1`; identify line where CRLF appears
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Apply CRLF fix: modify identified code path to ensure CRLF normalization; verify `cargo build -p doc-run` succeeds
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Test contracts-verify: run `dx run index.dx --section contracts-verify`; verify exit 0 with "ok —" message
```

---

## Notes

- **Parallel**: Sub-items 1-2 can run in parallel
- **Sequential**: Sub-item 3 depends on fix results from Sub-item 2
- **Dependencies**: Sub-item 5 depends on Sub-items 3-4 being complete
- **No human interaction**: All sub-items are mechanical code/verification tasks
- **Assigned ask**: All items use `ask: charter-DOC` (consistent with existing worklist items)
- **Capability gate**: All items advance `cap-dx-run` (the verification gate capability)

---

## Blocker Status

**Schema v5 requirement**: This decomposition cannot be added to the worklist via `dx_append` until the document store schema is upgraded from v4 to v5. A separate issue has been filed (report-66799993) documenting this schema mismatch.

**Workaround**: This decomposition document serves as the authoritative record until the worklist can be updated. Once the schema is resolved, these items should be added to `index.dx#now-worklist` in the order listed above.

---

## Acceptance Criteria

- [x] Decomposition created (2-5 mechanical sub-items)
- [x] Each sub-item has exact files/commands
- [x] Each sub-item has verifiable outcome
- [x] Each sub-item includes capability gate [cap: cap-dx-run]
- [x] Each sub-item is <10 minutes
- [x] No human approval needed (all mechanical)
- [ ] Sub-items added to index.dx#now-worklist (blocked by schema v5)
- [ ] Original failed item ticked in worklist (blocked by schema v5)
