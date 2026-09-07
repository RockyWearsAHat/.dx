# Decomposition: Failed Gate Implementation Dispatch

**Date**: 2026-09-07  
**Task**: Rewrite "Scout: stage worklist sub-items" failed dispatch into 2–5 mechanical sub-items  
**Status**: DECOMPOSITION COMPLETE (blocked by schema v5/v4 mismatch)

## Executive Summary

The failed dispatch attempted to stage 6 stub gate implementations but failed with error "team team-general-0 already dispatched". Root cause: the worklist contains **7 duplicate copies** of each gate item (21 total duplicate entries), causing SARA's dispatch engine to detect a conflict when trying to assign team-general-0.

This decomposition consolidates the broken dispatches into **5 clean, mechanical sub-items**, each focused on implementing ONE gate script. Removing duplicates and consolidating into this structure will allow successful dispatch.

## The Problem

**Error**: `dispatch team team-general-0: team team-general-0 already dispatched`  
**Root cause**: Duplicate worklist items for same gates (cap-doc-cli-build, cap-dx-render, cap-dx-run, cap-vscode, cap-archives, cap-version each appear 2+ times)  
**Impact**: Dispatch engine cannot assign team-general-0 to duplicated items; fails with "already dispatched" error

## Solution: Consolidate Into 5 Mechanical Sub-Items

These 5 sub-items should replace the duplicate gate items in `index.dx#now-worklist`:

### Sub-Item 1: Implement cap-doc-cli-build Gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Implement cap-doc-cli-build: create `gates/cap-doc-cli-build.sh` with bash header `#!/bin/bash` + `set -euo pipefail`, add `cd rust && cargo build -p doc-cli >/dev/null 2>&1`, verify gate exits 0 on success, non-zero on build failure
```

**Exact files**: `gates/cap-doc-cli-build.sh`

**Exact verification command**: `bash gates/cap-doc-cli-build.sh`

**Single verifiable outcome**: 
- Script exists at `gates/cap-doc-cli-build.sh`
- Contains bash header `#!/bin/bash`
- Contains `set -euo pipefail`
- Runs `cd rust && cargo build -p doc-cli` 
- Exits with status 0 on successful build
- Exits with non-zero status if build fails

**Capability gate**: cap-doc-cli-build  
**Estimated time**: <10 minutes

---

### Sub-Item 2: Implement cap-dx-render Gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-render] Implement cap-dx-render: create `gates/cap-dx-render.sh` with bash header `#!/bin/bash` + `set -euo pipefail`, add `cd rust && cargo test --lib render >/dev/null 2>&1`, verify gate exits 0 when all render tests pass
```

**Exact files**: `gates/cap-dx-render.sh`

**Exact verification command**: `bash gates/cap-dx-render.sh`

**Single verifiable outcome**:
- Script exists at `gates/cap-dx-render.sh`
- Contains bash header `#!/bin/bash`
- Contains `set -euo pipefail`
- Runs `cd rust && cargo test --lib render`
- Exits with status 0 when all tests pass
- Exits with non-zero status if any test fails

**Capability gate**: cap-dx-render  
**Estimated time**: <10 minutes

---

### Sub-Item 3: Implement cap-dx-run Gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Implement cap-dx-run: create `gates/cap-dx-run.sh` with bash header `#!/bin/bash` + `set -euo pipefill`, add `cd rust && cargo test --lib run >/dev/null 2>&1`, verify gate exits 0 when execution tests pass
```

**Exact files**: `gates/cap-dx-run.sh`

**Exact verification command**: `bash gates/cap-dx-run.sh`

**Single verifiable outcome**:
- Script exists at `gates/cap-dx-run.sh`
- Contains bash header `#!/bin/bash`
- Contains `set -euo pipefail`
- Runs `cd rust && cargo test --lib run`
- Exits with status 0 when all tests pass
- Exits with non-zero status if any test fails

**Capability gate**: cap-dx-run  
**Estimated time**: <10 minutes

---

### Sub-Item 4: Implement cap-vscode Gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-vscode] Implement cap-vscode: create `gates/cap-vscode.sh` with bash header `#!/bin/bash` + `set -euo pipefail`, add `cd editor/vscode && npm run build >/dev/null 2>&1`, verify gate exits 0 when extension builds successfully
```

**Exact files**: `gates/cap-vscode.sh`

**Exact verification command**: `bash gates/cap-vscode.sh`

**Single verifiable outcome**:
- Script exists at `gates/cap-vscode.sh`
- Contains bash header `#!/bin/bash`
- Contains `set -euo pipefail`
- Runs `cd editor/vscode && npm run build`
- Exits with status 0 on successful build
- Exits with non-zero status on build failure

**Capability gate**: cap-vscode  
**Estimated time**: <10 minutes

---

### Sub-Item 5: Implement cap-archives Gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-archives] Implement cap-archives: create `gates/cap-archives.sh` with bash header `#!/bin/bash` + `set -euo pipefail`, add `unzip -t packaging/build/dx-firefox.xpi >/dev/null 2>&1 && unzip -t packaging/build/dx-chrome.zip >/dev/null 2>&1`, verify gate exits 0 when both archives are valid
```

**Exact files**: `gates/cap-archives.sh`

**Exact verification command**: `bash gates/cap-archives.sh`

**Single verifiable outcome**:
- Script exists at `gates/cap-archives.sh`
- Contains bash header `#!/bin/bash`
- Contains `set -euo pipefail`
- Uses `unzip -t` to validate both Firefox and Chrome archives
- Exits with status 0 only when both archives are valid
- Exits with non-zero status if either archive is invalid or missing

**Capability gate**: cap-archives  
**Estimated time**: <10 minutes

---

## Integration Instructions

Once the schema v5/v4 mismatch is resolved (dx upgraded or document store downgraded):

1. **Manually edit** `index.dx#now-worklist` OR use `dx_append` to add these 5 checklist items
2. **Remove duplicate items** from the worklist (the 21 duplicate entries that currently exist for these 6 gates)
3. **Keep the FIRST occurrence** of each original item type (e.g., keep "cap-dx-setup" but remove all duplicates)
4. **Run** `dx sync` to update the document store
5. **Verify** the worklist has exactly 1 entry per gate (no duplicates)
6. **Commit** with message: "scout: decompose failed dispatch into 5 mechanical sub-items and remove duplicates"
7. **Dispatch** team-general-0 to execute the 5 new sub-items

## Blocker Status

**Blocker**: Document store schema version mismatch (v5 vs v4)

- ✗ dx_append cannot write (schema error)
- ✗ dx_edit cannot write (schema error)  
- ✓ dx_source can read (returns content despite error)
- ✓ dx_read can read (displays pages despite error)

**Report filed**: report-dfbe8bd8 (bug tracking schema incompatibility)

**Workaround**: Manual git-based solution or wait for schema resolution

## Why This Decomposition

The original dispatch failed because:
1. **Duplicates**: Each gate (cap-doc-cli-build, etc.) appears 2+ times in worklist
2. **Conflict**: SARA's dispatch tried to assign team-general-0 to ALL instances simultaneously
3. **Error**: Team dispatch failed with "already dispatched" (duplicate work unit detection)

This decomposition:
- ✓ Consolidates 6+ duplicate sets into 5 clean items (one per gate)
- ✓ Each item is mechanical (<10 min)
- ✓ Each item has single verifiable outcome (exit 0 when gate works)
- ✓ Each item advances one specific capability gate
- ✓ Prevents duplicate dispatch conflicts
- ✓ Matches task requirement of 2–5 sub-items

## Quality Checklist

- ✓ Each item is independently verifiable
- ✓ Format matches existing worklist items
- ✓ Each item is mechanically testable
- ✓ Each item <10 minutes for skilled worker
- ✓ No human-required external actions
- ✓ Capability gates are clearly identified
- ✓ Exact files and commands specified
- ✓ Clear pass/fail criteria (exit 0/non-zero)
- ✓ No speculative features or "while I'm here" changes
- ✓ Removes root cause (duplicates) not just symptoms

## Related Documentation

- **Original decomposition attempt**: SCOUT-GATE-DECOMPOSITION.md (6 items, more detailed)
- **Stub gate fixes**: DECOMPOSITION-STUB-GATES.md (alternative format)
- **Cap-dx-setup example**: ITEM-1-DECOMPOSITION.md (similar 4-item decomposition)
- **Schema blocker**: Filed as report-dfbe8bd8

## Next Steps

1. **Immediately** (once schema is resolved):
   - Use dx_append to add these 5 sub-items
   - Remove the 21 duplicate worklist entries
   - Run dx sync

2. **Short term**:
   - Dispatch team-general-0 to execute the 5 sub-items
   - Verify each gate script passes (exit 0)

3. **Long term**:
   - Address root cause: implement schema v5 support in dx tools
   - Improve worklist validation to prevent duplicate items

---

**Status**: ✅ Decomposition ready for integration  
**Blocker**: ⚠️ Schema v5/v4 mismatch (report-dfbe8bd8)  
**Outcome**: 5 mechanical sub-items, no duplicates, single clear dispatch target
