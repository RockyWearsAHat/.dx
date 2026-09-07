# CRLF Line Ending Blocker - Decomposed Sub-Items

## Problem
The `contracts-verify` and `verify` verification blocks are failing with bash syntax errors:
```
syntax error near unexpected token `$'do\r''
```

This indicates that bash scripts are being generated with Windows CRLF line endings (`\r\n`) but executed on Unix/macOS which only expects LF (`\n`).

## Root Cause
Bash scripts generated in `doc-run/src/confine.rs` (or related code path) are writing content that includes carriage returns. When bash executes these scripts on macOS/Linux, it fails to parse the CRLF line endings.

## Impact
- `index.dx#contracts-verify` cannot run verification checks
- `index.dx#verify` cannot run verification checks
- Capability gates remain in "suspect" status
- Document verification system is blocked

## Technical Blocker
- Document store schema mismatch (v5 vs v4) prevents `dx_append` from working
- Cannot directly append sub-items to `index.dx#now-worklist` via MCP tools
- Workaround: Add items directly via git/commit after schema upgrade

## Decomposed Sub-Items

These sub-items should be added to `index.dx#now-worklist` once schema issue is resolved:

### Sub-Item 1: Locate bash script generation code
- **Capability**: [cap: cap-dx-run]
- **Task**: Find where bash scripts are generated in doc-run
- **Command**: `grep -rn 'fs::write\|write_all' rust/doc-run/src/ | grep -v test`
- **Verify**: Output identifies the file and line number where block.sh content is written (expected: rust/doc-run/src/confine.rs or similar)
- **Time**: <5 minutes

### Sub-Item 2: Identify line ending issue location
- **Capability**: [cap: cap-dx-run]
- **Task**: Inspect the exact line that writes bash script and check if input includes CRLF
- **Command**: `sed -n '<line>,<line+5>p' rust/doc-run/src/confine.rs` (after finding line from Sub-Item 1)
- **Verify**: Code shows where script content is written; likely uses `.as_bytes()` or `.to_string()` without stripping line endings
- **Time**: <5 minutes

### Sub-Item 3: Apply LF-only conversion
- **Capability**: [cap: cap-dx-run]
- **Task**: Modify the script generation to strip Windows line endings
- **File**: rust/doc-run/src/confine.rs (or identified location)
- **Change**: Before writing, add `.replace("\r\n", "\n")` to the script content
- **Example**: `script.replace("\r\n", "\n").as_bytes()` instead of `script.as_bytes()`
- **Verify**: `cd rust && cargo build -p doc-run 2>&1 | tail -1` shows no compilation errors
- **Time**: <5 minutes

### Sub-Item 4: Verify contracts-verify passes
- **Capability**: [cap: cap-dx-run]
- **Task**: Run the contracts verification block and confirm no CRLF errors
- **Command**: `cd D:\SARA\Desktop\DOC && dx run --review index.dx --section contracts-verify 2>&1 | tail -20`
- **Verify**: Output shows "ok" or successful completion, NOT "syntax error near unexpected token"
- **Time**: <10 minutes

### Sub-Item 5: Verify verify block passes
- **Capability**: [cap: cap-dx-run]
- **Task**: Run the main verification block and confirm no CRLF errors
- **Command**: `cd D:\SARA\Desktop\DOC && dx run --review index.dx --section verify 2>&1 | tail -20`
- **Verify**: Output shows successful verification, NOT "syntax error near unexpected token"
- **Time**: <10 minutes

## Next Steps

1. **Resolve schema mismatch**: Upgrade dx or downgrade document store to compatible versions
2. **Add sub-items to worklist**: Once schema is resolved, use `dx_append` to add these 5 sub-items to `index.dx#now-worklist`
3. **Dispatch to Haiku worker**: Once added to worklist, each item can be implemented independently
4. **Verify all pass**: After implementation, all verification blocks should pass

## Notes
- Each sub-item is mechanical and under 10 minutes
- Sub-items can be done in parallel (1-3) then sequentially (4-5)
- No human interaction required (no account signups, payments, etc.)
- Fixes the core blocker that prevents verification gates from running
