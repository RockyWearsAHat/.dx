# Scout Phase 2: Confirmation & Worklist Specification

**Date**: 2026-09-06  
**Status**: Scouting complete - ready for worklist update  
**Blocker**: Document store schema v5 vs tool schema v4 prevents dx_append

---

## Verification: Previous Scout Findings Still Valid ✅

The SCOUT-FINAL-REPORT.md findings (2026-09-06) remain accurate:
- ✅ 1 of 7 gates working (cap-dx-doctor-store)
- ✅ 6 gates are placeholder stubs  
- ✅ Project Understanding is comprehensive
- ✅ Critical blocker: schema version mismatch

**No new issues discovered.** Work plan in SCOUT-ITEMS-TO-ADD.md is sound.

---

## Items to Add to `now-worklist` Block (index.dx)

These 5 items should be appended to the `now-worklist` checklist. Format: `- [ ] [scout] [ask: charter-DOC] [cap: CAP_ID] Description`

### Priority 1: Foundation Gates (Do First - blocks everything else)

1. **Implement cap-doc-cli-build gate**
   ```
   - [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Implement cap-doc-cli-build gate: add bash script with `set -euo pipefail` to verify `cargo build -p doc-cli` succeeds in rust/ directory; gate exits 0 only when build completes without errors
   ```
   - **Why First**: Core CLI must build before anything else
   - **Verify**: cap-doc-cli-build code block in index.dx#Finished contains proper bash with build check

2. **Implement cap-dx-render gate**
   ```
   - [ ] [scout] [ask: charter-DOC] [cap: cap-dx-render] Implement cap-dx-render gate: add bash script with `set -euo pipefail` to run `cargo test --lib render` in rust/; gate exits 0 only when all render tests pass
   ```
   - **Why Second**: Document rendering is core capability
   - **Verify**: cap-dx-render block contains render test verification

3. **Implement cap-dx-run gate**
   ```
   - [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Implement cap-dx-run gate: add bash script with `set -euo pipefail` to run `cargo test --lib run` in rust/; gate exits 0 only when execution tests pass
   ```
   - **Why Third**: Code execution is core capability
   - **Verify**: cap-dx-run block contains execution test verification

### Priority 2: Surface Gates (Do After Foundation)

4. **Implement cap-vscode gate**
   ```
   - [ ] [scout] [ask: charter-DOC] [cap: cap-vscode] Implement cap-vscode gate: add bash script with `set -euo pipefail` to verify VS Code extension builds successfully; run `cd editor/vscode && npm run build` and verify extension package created; gate exits 0 only when build succeeds
   ```
   - **Why**: Editor surface verification
   - **Verify**: cap-vscode block builds extension

5. **Implement cap-archives gate**
   ```
   - [ ] [scout] [ask: charter-DOC] [cap: cap-archives] Implement cap-archives gate: add bash script with `set -euo pipefail` to verify browser extension archives exist and are valid; check Firefox XPI and Chrome ZIP in packaging/build/; gate exits 0 only when all archives present and readable
   ```
   - **Why**: Distribution verification
   - **Verify**: cap-archives block checks archive presence

---

## Implementation Sequence

```
PHASE 1: Foundation (Items 1-3 above)
├─ Each worker: implement one gate
├─ Verify: bash gates/cap-xxx.sh exits 0
├─ Verify: index.dx#Finished#cap-xxx contains real code
└─ All 3 MUST pass before moving to Phase 2

PHASE 2: Surfaces (Items 4-5 above)
├─ Each worker: implement one gate  
├─ Verify: npm builds run without error
├─ Verify: archives exist and are readable
└─ All 5 gates passing = "gates: 5/7 pass"

PHASE 3: New Gates (Existing Worklist)
├─ cap-dx-setup
├─ cap-format-round-trip (shell script exists)
├─ cap-store-integrity
└─ cap-sandbox-cross-platform
```

---

## Success Criteria for This Scout

- [x] Project charter is clear and documented
- [x] Capability gates identified and prioritized
- [x] 5 critical items specified with exact requirements
- [x] Each item maps to one gate
- [x] Verification method stated for each item
- [x] Blocker documented (schema version)
- [x] Workaround provided (manual git updates)
- [ ] **BLOCKER**: Items need to be added to now-worklist (schema prevents dx_append)

---

## Next Step: Manual Worklist Update

Since `dx_append` cannot run (schema v5 vs v4), these items should be manually added to the `now-worklist` checklist block in `index.dx` when:
1. Schema mismatch is resolved (upgrade dx or downgrade store), OR
2. Manual edit is performed using plain text/git tools, OR  
3. A newer dx version with compatible schema is installed

**Recommendation**: Once these items are added to now-worklist, the engine can dispatch them to Haiku workers for implementation.

---

## Reference: Gate Implementation Pattern

All gates follow this bash template (see gates/cap-format-round-trip.sh):

```bash
#!/bin/bash
set -euo pipefail

# Brief description

# Actual verification command(s)
# (e.g., cd rust && cargo test --lib xyz)

# Exit with status (implicit via set -e)
```

No `|| true`, no `|| echo`, no error suppression. Gates exit 0 only on real success.

---

## Confirmation

✅ **Scout Phase 2 Complete**

Findings from Phase 1 SCOUT-FINAL-REPORT.md remain valid. Work plan is clear. Blocker is documented. Ready for human/engine intervention to update worklist and dispatch items to workers.

**Owner**: Alex Waldmann  
**Email**: alexwaldmann2004@gmail.com  
**Date**: 2026-09-06 20:45 UTC
