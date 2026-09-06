# Scout Items to Add to now-worklist

## Schema Blocker Note
**Issue**: Document store schema 5 vs dx tool schema 4 incompatibility prevents using `dx_append` to add items. These items should be manually added to the `now-worklist` block in index.dx or when schema compatibility is resolved.

## Critical Priority: Fix Broken Gates (6 items)

The 6 gates in the ## Finished section of index.dx are broken (placeholder comments). These MUST be fixed before new capabilities are added.

### Item 1: Implement cap-doc-cli-build gate
```
[ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Implement cap-doc-cli-build gate: add bash script with `set -euo pipefail` to verify `cargo build -p doc-cli` succeeds in rust/ directory; gate exits 0 only when build completes without errors
```
**Verification**: The `cap-doc-cli-build` code block in index.dx#Finished contains proper bash script that:
- Uses `set -euo pipefail`  
- Runs `cd rust && cargo build -p doc-cli`
- Exits 0 on success, non-zero on failure
- No error suppression (`|| true`, `|| echo`)

### Item 2: Implement cap-dx-render gate
```
[ ] [scout] [ask: charter-DOC] [cap: cap-dx-render] Implement cap-dx-render gate: add bash script with `set -euo pipefail` to verify document rendering produces valid output; run `cargo test --lib render` in rust/ directory; gate exits 0 only when all render tests pass
```
**Verification**: The `cap-dx-render` code block in index.dx#Finished contains proper bash script with render test verification.

### Item 3: Implement cap-dx-run gate
```
[ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Implement cap-dx-run gate: add bash script with `set -euo pipefail` to verify code execution and output storage; run `cargo test --lib run` in rust/ directory; gate exits 0 only when execution tests pass
```
**Verification**: The `cap-dx-run` code block in index.dx#Finished contains proper bash script with run/execution test verification.

### Item 4: Implement cap-vscode gate
```
[ ] [scout] [ask: charter-DOC] [cap: cap-vscode] Implement cap-vscode gate: add bash script with `set -euo pipefail` to verify VS Code extension builds successfully; run `cd editor/vscode && npm run build` and verify extension package is created; gate exits 0 only when build succeeds
```
**Verification**: The `cap-vscode` code block in index.dx#Finished contains proper bash script that builds the VS Code extension.

### Item 5: Implement cap-archives gate
```
[ ] [scout] [ask: charter-DOC] [cap: cap-archives] Implement cap-archives gate: add bash script with `set -euo pipefail` to verify browser extension archives are built and valid; verify Firefox and Chrome archive files exist and are signed/valid; gate exits 0 only when all archives are present
```
**Verification**: The `cap-archives` code block in index.dx#Finished contains proper bash script that verifies archive presence and validity.

### Item 6: Implement cap-version gate
```
[ ] [scout] [ask: charter-DOC] [cap: cap-version] Implement cap-version gate: add bash script with `set -euo pipefail` to verify version command returns valid semantic version; run `cargo run -p doc-cli -- version` and parse output as semver; gate exits 0 only when output is valid semver format
```
**Verification**: The `cap-version` code block in index.dx#Finished contains proper bash script that validates semantic version output.

---

## Secondary Priority: Implement Existing Worklist Items (Already Listed)

These are already in the worklist but listed here for completeness:

- [ ] [cap: cap-dx-setup] Implement cap-dx-setup gate
- [ ] Store distribution (sign/submit browser extensions)
- [ ] [cap: cap-format-round-trip] Implement cap-format-round-trip gate (partially done - script exists)
- [ ] [cap: cap-store-integrity] Implement cap-store-integrity gate
- [ ] [cap: cap-sandbox-cross-platform] Implement cap-sandbox-cross-platform gate

---

## Finishing Bar Checks

Current status:
- ✅ README.md with usage examples exists
- ⚠️  LICENSE file missing (item in worklist)
- ⚠️  .gitignore incomplete (item in worklist)

---

## Order of Execution (Recommended)

1. **Phase 1**: Fix the 6 broken gates (Items 1-6 above)
   - Reason: These are foundational; new gates depend on infrastructure working
   - Timeline: 2-3 days
   - Effort: Medium (each gate is ~20-40 lines of bash)

2. **Phase 2**: Implement new gates from worklist
   - cap-dx-setup
   - cap-format-round-trip (script exists, needs integration)
   - cap-store-integrity
   - cap-sandbox-cross-platform

3. **Phase 3**: Complete distribution and miscellaneous items
   - Store distribution (sign/submit)
   - LICENSE file
   - .gitignore updates

---

## Technical Notes

### Gate Implementation Pattern

All gates should follow this template:

```bash
#!/bin/bash
set -euo pipefail

# Brief description of what this gate verifies

# Verification command(s)
# e.g.: cd rust && cargo test --lib xyz

# Exit with status of last command (implicit with set -e)
```

### Reference Implementation

See `gates/cap-format-round-trip.sh` for a properly implemented gate script.

### Testing Gates

Gates can be tested locally by running:
```bash
bash gates/cap-xxx.sh
```

Must exit 0 on success, non-zero on failure.

---

## Blockers

1. **Schema Version Mismatch** (Critical)
   - Document store schema 5, dx tool schema 4
   - Blocks: dx_append, dx_edit, automated document updates
   - Workaround: Manual git commits or document reconstruction

2. **DX_UNCONFINED Environment** 
   - Current gates run with DX_UNCONFINED set
   - Prevents proper sandbox testing
   - Should be unset for real testing

---

## Success Criteria

All scouts/workers are done when:
1. [ ] All 6 broken gates have proper bash implementations with set -euo pipefail
2. [ ] All gates can be run independently and exit 0 on success
3. [ ] 4 new gates (setup, format-round-trip, store-integrity, sandbox-cross-platform) are implemented
4. [ ] Worklist items are marked complete as they're finished
5. [ ] Finishing bar checks pass (LICENSE, .gitignore, README complete)
