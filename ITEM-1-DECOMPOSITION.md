# Decomposition of Worklist Item 1: cap-dx-setup Gate

**Date**: 2026-09-07  
**Blocker**: Document store schema 5 vs dx tool schema 4 mismatch prevents `dx_append` from working  
**Status**: Sub-items created and ready for worklist integration

## Sub-Item Specifications

These 4 sub-items should be added to `index.dx#now-worklist` immediately after the original item 1 line, then the original item should be marked [x]:

### Sub-Item 1: Create gates/cap-dx-setup.sh with bash header
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Create gates/cap-dx-setup.sh with bash header: write file with `#!/bin/bash` and `set -euo pipefail` on lines 1-2, add comment describing gate purpose on line 3; verify: file exists and is executable (`test -x gates/cap-dx-setup.sh`)
```

**Why**: Establishes the bash script foundation with proper error handling setup.  
**Verifiable outcome**: File `gates/cap-dx-setup.sh` exists and `test -x gates/cap-dx-setup.sh` returns true.  
**Estimated time**: 5 minutes

---

### Sub-Item 2: Implement dx setup verification
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Implement dx setup verification in gates/cap-dx-setup.sh: add lines to invoke `dx setup --help` and capture exit code; gate exits 0 only if command succeeds (no `|| true` escapes)
```

**Why**: Verifies that dx setup command is callable and completes without error.  
**Verifiable outcome**: Run `bash gates/cap-dx-setup.sh` and confirm it exits with status 0 when dx setup succeeds.  
**Estimated time**: 7 minutes

---

### Sub-Item 3: Add PATH and MCP checks
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Add PATH and MCP checks to gates/cap-dx-setup.sh: verify `dx` binary is on PATH after setup (`which dx >/dev/null`) and MCP config exists (`test -f ~/.config/dx/mcp.json` or equivalent); both checks must pass for gate to exit 0
```

**Why**: Verifies that setup actually installs the required components (dx on PATH and MCP configuration).  
**Verifiable outcome**: Script checks both PATH and MCP config; returns exit 0 only when both exist.  
**Estimated time**: 8 minutes

---

### Sub-Item 4: Test complete gate
```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Test cap-dx-setup gate: run `bash gates/cap-dx-setup.sh` from repo root; must exit 0; if exit code is non-zero, record actual error and do not mark complete
```

**Why**: Final verification that the gate works end-to-end.  
**Verifiable outcome**: `bash gates/cap-dx-setup.sh` exits with status 0.  
**Estimated time**: 5 minutes

---

## Integration Instructions

Once schema mismatch is resolved, add these 4 lines to `index.dx#now-worklist` immediately after the first item (currently marked `[ ]`), then mark the first item as `[x]`:

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Create gates/cap-dx-setup.sh with bash header: write file with `#!/bin/bash` and `set -euo pipefail` on lines 1-2, add comment describing gate purpose on line 3; verify: file exists and is executable (`test -x gates/cap-dx-setup.sh`)
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Implement dx setup verification in gates/cap-dx-setup.sh: add lines to invoke `dx setup --help` and capture exit code; gate exits 0 only if command succeeds (no `|| true` escapes)
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Add PATH and MCP checks to gates/cap-dx-setup.sh: verify `dx` binary is on PATH after setup (`which dx >/dev/null`) and MCP config exists (`test -f ~/.config/dx/mcp.json` or equivalent); both checks must pass for gate to exit 0
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Test cap-dx-setup gate: run `bash gates/cap-dx-setup.sh` from repo root; must exit 0; if exit code is non-zero, record actual error and do not mark complete
```

Then run `dx sync` to update the document store.

## Rationale

The cap-dx-setup capability is critical because:
- `dx setup` is the core user entry point emphasized in README as "One command, once, per device"
- It is essential for first-time user experience and adoption
- Setup must verify all components (PATH, MCP, service configs) are installed correctly
- Gate must exit 0 only when setup succeeds and installation is complete

## Related Issues

- Schema mismatch (dx tool v4 vs document store v5) reported as bug report-49cce250
- Previous decompositions: ITEM-8-DECOMPOSITION.md, SCOUT-ITEMS-TO-ADD.md
- Team dispatch error: "team team-general-0 already dispatched" (duplicate items in worklist)
