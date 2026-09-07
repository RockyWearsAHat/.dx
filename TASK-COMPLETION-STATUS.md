# Task Completion Status: Decompose Failed Item

## Task Summary
**Goal**: Decompose a failed worklist item into 2-5 mechanical sub-items (each <10 min, verifiable)

**Status**: PARTIAL - Decomposition complete, worklist update blocked by schema mismatch

## What Was Completed

### 1. Problem Identification ✅
- **Root cause identified**: Bash scripts are written with Windows CRLF line endings but executed on Unix/macOS where only LF is expected
- **Failing blocks**: 
  - `index.dx#contracts-verify` - exits with bash syntax error
  - `index.dx#verify` - exits with bash syntax error
- **Error message**: `syntax error near unexpected token $'do\r''`
- **Impact**: Contract verification system is blocked; capability gates remain in "suspect" status

### 2. Decomposition ✅
Created `CRLF-BLOCKER-DECOMPOSITION.md` with 5 mechanical sub-items:
1. **Find bash script generation**: `grep -rn 'fs::write\|write_all' rust/doc-run/src/` 
   - Verify: Output identifies file and line number (expected: confine.rs)
   - Time: <5 min

2. **Identify line ending issue**: Inspect code that writes bash scripts for CRLF handling
   - Verify: Code shows where content is written without stripping `\r`
   - Time: <5 min

3. **Apply LF-only conversion**: Add `.replace("\r\n", "\n")` before writing script
   - File: rust/doc-run/src/confine.rs
   - Verify: `cargo build -p doc-run` exits 0 with no errors
   - Time: <5 min

4. **Verify contracts-verify**: Run block and confirm no CRLF syntax errors
   - Command: `dx run --review index.dx --section contracts-verify`
   - Verify: Output shows "ok" not "syntax error"
   - Time: <10 min

5. **Verify verify block**: Run block and confirm no CRLF syntax errors
   - Command: `dx run --review index.dx --section verify`
   - Verify: Output shows success not "syntax error"
   - Time: <10 min

### 3. Git Commit ✅
Committed decomposition work:
- `commit b19164c`: "scout: decompose CRLF bash script blocker into 5 mechanical sub-items"
- File: `CRLF-BLOCKER-DECOMPOSITION.md`

## Technical Blocker: Schema Mismatch

### Issue Description
- Document store (`repo.dxcp`) was written with schema version 5
- MCP tools (`dx_append`, `dx_edit`) only understand schema version 4
- Cannot modify `index.dx#now-worklist` to add decomposed sub-items

### Error Message
```
this document store was written by a newer dx (schema 5, this build understands 4); 
upgrade dx to open it
```

### Attempted Workarounds
1. ✅ `dx sync` - confirmed documents resolve correctly, schema issue is with MCP tools
2. ❌ `dx_append` - failed with schema mismatch error
3. ❌ `dx_edit` - failed to locate text in block body (likely due to schema issue)
4. ❌ Direct pack file editing - not feasible without compatible dx binary

## What Needs to Happen Next

### Prerequisite: Resolve Schema Mismatch
- **Option A**: Upgrade dx binary to version that understands schema 5
- **Option B**: Downgrade document store to schema 4 (if possible)
- **Recommended**: Check project setup/docs for dx version requirements

### After Schema Resolution
1. Use `dx_append D:\SARA\Desktop\DOC\index.dx now-worklist` to add 5 sub-items
2. Tick the original cap-dx-setup item's box (mark complete)
3. Commit the changes
4. Dispatch to Haiku worker for implementation

### Sub-Items to Add (Once Schema Fixed)
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Fix bash script CRLF issue: find where bash scripts are generated in doc-run/src/confine.rs; run `grep -rn 'fs::write\|write_all' rust/doc-run/src/` to locate; verify: grep output shows file path

- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Strip CRLF from bash scripts: edit rust/doc-run/src/confine.rs to add `.replace("\r\n", "\n")` before writing; verify: `cd rust && cargo build -p doc-run` exits 0

- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Verify contracts-verify passes: run `dx run --review index.dx --section contracts-verify`; verify: no "syntax error" in output

- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Verify verify block passes: run `dx run --review index.dx --section verify`; verify: no "syntax error" in output
```

## Summary

**Decomposition Work**: ✅ COMPLETE
- Problem identified and root cause understood
- Sub-items documented with exact commands and verifiable outcomes
- Committed to git with clear commit message

**Worklist Integration**: ❌ BLOCKED
- Requires resolving document store schema mismatch (v5 vs v4)
- MCP tools cannot write to document store in this state
- Awaiting schema compatibility fix or tool upgrade

**Next Action**: 
- Resolve schema mismatch (check dx version requirements in docs)
- Once resolved, run: `dx_append` to add sub-items to now-worklist
- Then dispatch to Haiku worker for implementation

**Time Spent**: Decomposition complete and documented; schema issue is environmental, not task-related.
