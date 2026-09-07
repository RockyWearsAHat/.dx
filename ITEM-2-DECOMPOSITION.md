# Decomposition of Worklist Item 2: cap-doc-cli-build Gate

**Date**: 2026-09-07  
**Blocker**: Document store schema 5 vs dx tool schema 4 mismatch prevents `dx_append` from working  
**Status**: Sub-items created and ready for worklist integration

## Sub-Item Specifications

These 4 sub-items should be added to `index.dx#now-worklist` immediately after the first cap-doc-cli-build item (line 10), then the original item should be marked [x]:

### Sub-Item 1: Read and understand current stub
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Examine cap-doc-cli-build gate in index.dx: run `dx_source index.dx section=cap-doc-cli-build` to read current stub; verify: confirm block contains only a comment and no real verification logic
```

**Why**: Establishes baseline understanding of what the gate currently does (nothing).  
**Verifiable outcome**: Read output shows stub is comment-only (e.g., `# Gate verification block - cap-doc-cli-build`).  
**Estimated time**: 3 minutes

---

### Sub-Item 2: Verify local doc-cli build works
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Test doc-cli build locally: run `cd rust && cargo build -p doc-cli && cargo clippy -p doc-cli -- -D warnings` to verify build and linting succeeds; verify: both commands exit 0 with no errors
```

**Why**: Confirms that doc-cli is buildable before writing the gate logic.  
**Verifiable outcome**: Commands run without errors; exit status is 0.  
**Estimated time**: 7 minutes

---

### Sub-Item 3: Replace stub with verification logic
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Update cap-doc-cli-build gate block in index.dx using dx_edit: replace comment-only stub with real verification script using `set -euo pipefail` that runs `cd rust && cargo build -p doc-cli && cargo clippy -p doc-cli -- -D warnings && echo "ok - doc-cli builds and lints clean"` with no escape hatches; verify: gate block contains proper bash script with error handling
```

**Why**: Installs the real verification logic that will actually test the capability.  
**Verifiable outcome**: Gate block now contains real bash script (not just a comment); script has `set -euo pipefail` header.  
**Estimated time**: 5 minutes

---

### Sub-Item 4: Run gate and confirm it passes
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Test complete gate: run `dx run index.dx section=cap-doc-cli-build` (or equivalent); must exit 0 and output "ok" message; if gate fails, record actual error and diagnose
```

**Why**: Final verification that the gate works end-to-end and provides meaningful feedback.  
**Verifiable outcome**: `dx run` command exits 0; output contains "ok - doc-cli builds" message.  
**Estimated time**: 5 minutes

---

## Integration Instructions

Once schema mismatch is resolved, add these 4 lines to `index.dx#now-worklist` immediately after line 10 (the first cap-doc-cli-build item), then mark that original item as `[x]`:

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Examine cap-doc-cli-build gate in index.dx: run `dx_source index.dx section=cap-doc-cli-build` to read current stub; verify: confirm block contains only a comment and no real verification logic
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Test doc-cli build locally: run `cd rust && cargo build -p doc-cli && cargo clippy -p doc-cli -- -D warnings` to verify build and linting succeeds; verify: both commands exit 0 with no errors
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Update cap-doc-cli-build gate block in index.dx using dx_edit: replace comment-only stub with real verification script using `set -euo pipefail` that runs `cd rust && cargo build -p doc-cli && cargo clippy -p doc-cli -- -D warnings && echo "ok - doc-cli builds and lints clean"` with no escape hatches; verify: gate block contains proper bash script with error handling
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Test complete gate: run `dx run index.dx section=cap-doc-cli-build` (or equivalent); must exit 0 and output "ok" message; if gate fails, record actual error and diagnose
```

Then run `dx sync` to update the document store.

## Rationale

The cap-doc-cli-build capability is critical because:
- The `doc-cli` Rust binary is the primary user-facing tool for the dx project
- It must build successfully with zero compiler errors and warnings (clippy -D warnings)
- Gate must exit 0 only when doc-cli is fully buildable and passes all lints
- This blocks any other capability that depends on doc-cli functioning

## Related Issues

- Schema mismatch (dx tool v4 vs document store v5) reported as bug report-49cce250
- Previous decomposition: ITEM-1-DECOMPOSITION.md (cap-dx-setup)
- Duplicate items in worklist causing dispatch conflict (cap-doc-cli-build appears 2+ times)
- Similar gates need same treatment: cap-dx-render, cap-dx-run, cap-vscode, cap-archives, cap-version, cap-dx-doctor-store

## Duplicates in Worklist

Note: cap-doc-cli-build appears multiple times in the worklist (lines 10, 14 at minimum). After decomposing the first occurrence, remove duplicate entries to avoid re-dispatching the same work to the team.
