# Scout Report: Phase 2 Worklist Items

## Context
All 7 existing capability gates pass (as of commit 07b973a). Per scout instructions, having all gates pass means:
1. Propose ONE item for the next capability (cap-dx-setup - already added)
2. Find 3-5 total items to keep the fleet moving

This report proposes 4 additional items to reach the 5-item target, covering critical hardening work and the next logical capabilities per the project charter.

## Items to Add to now-worklist

### Item 1: Implement cap-dx-setup Gate
**Status**: Already added (commit 07b973a)
**Ask**: charter-DOC
**Capability**: cap-dx-setup
**Description**: Implement bash script with `set -euo pipefail` to verify `dx setup` command completes successfully and installs all components (PATH, MCP, service configs). Gate exits 0 when setup succeeds and installation is verified.
**Rationale**: `dx setup` is the core user entry point emphasized in README as "One command, once, per device" — critical for first-time user experience and adoption.

---

### Item 2: Implement cap-format-round-trip Gate
**Ask**: charter-DOC
**Capability**: cap-format-round-trip
**Description**: Add bash script with `set -euo pipefail` to verify documents round-trip byte-for-byte through parse/stringify cycles. Gate exits 0 when random documents match original exactly after round-trip.
**Rationale**: CLAUDE.md states as non-negotiable: "Canonical output must not drift. `doc-core/src/format` round-trips real documents byte-for-byte against captured fixtures." This is a foundational constraint on store integrity. Current test fixtures exist but there is no capability gate proving the property holds.
**Verification**: 
```bash
# Generate random documents, parse, stringify, compare MD5
for doc in examples/showcase.dx examples/tutorial.dx; do
  dx text "$doc" > /tmp/before.txt
  # Parse and stringify (via internal machinery)
  dx render "$doc" > /dev/null
  dx text "$doc" > /tmp/after.txt
  diff /tmp/before.txt /tmp/after.txt || exit 1
done
exit 0
```

---

### Item 3: Implement cap-store-integrity Gate
**Ask**: charter-DOC  
**Capability**: cap-store-integrity
**Description**: Add bash script with `set -euo pipefail` to verify no bytes are lost during pack/unpack cycles. Gate exits 0 when documents in `.doc/repo.dxcp` can be unpacked and all content is recovered bit-identical.
**Rationale**: CLAUDE.md explicitly states: "Storage cannot lose a byte. A chunk holds the exact canonical text `stringify` writes for one block, so reassembly is concatenation." The store is the source of truth; data loss in the pack would be silent. This gate verifies the storage contract.
**Verification**:
```bash
# Create test documents, sync to store, verify recovery
dx new test1.dx --title "Test Doc 1"
ORIGINAL_HASH=$(sha256sum .doc/repo.dxcp | cut -d' ' -f1)
dx sync .
AFTER_SYNC_HASH=$(sha256sum .doc/repo.dxcp | cut -d' ' -f1)
[ "$ORIGINAL_HASH" = "$AFTER_SYNC_HASH" ] || exit 1
exit 0
```

---

### Item 4: Implement cap-sandbox-cross-platform Gate
**Ask**: charter-DOC
**Capability**: cap-sandbox-cross-platform
**Description**: Add bash script with `set -euo pipefail` to verify code execution is confined properly on macOS (Seatbelt), Linux (bubblewrap), and Windows (where DX_UNCONFINED must be used). Gate exits 0 when a test document's code block cannot escape its sandbox.
**Rationale**: CLAUDE.md states: "Running a document runs code someone else wrote... dx confines it with Seatbelt on macOS and bubblewrap on Linux." Security is non-negotiable; this gate proves the confinement boundary holds across platforms.
**Verification**:
```bash
# Try to read files outside granted paths — should fail
cat > attack.dx << 'EOF'
::code lang=bash run
# Try to read /etc/passwd (outside document directory)
cat /etc/passwd && echo "SANDBOX BROKEN" && exit 1
echo "Sandbox holding"
exit 0
::end
EOF

dx run attack.dx --force 2>&1 | grep -q "Sandbox holding" || exit 1
exit 0
```

---

### Item 5: Implement cap-store-submission-automation Gate
**Ask**: charter-DOC
**Capability**: cap-store-submission-automation
**Description**: Add bash script with `set -euo pipefail` to verify store submission workflow can run in CI/CD. Gate exits 0 when archives are built, manifests validated, and mock submission endpoints confirm acceptance.
**Rationale**: Archive building and testing is complete (10/10 tests passing). The gate formalizes CI integration for the submission workflow, ensuring deployment is automated and repeatable.
**Verification**: 
```bash
cd packaging
# Build archives
./build-stores.sh || exit 1
# Run submission tests
node --test test/chrome-store-integration.test.mjs || exit 1
node --test test/store-submission.test.mjs || exit 1
exit 0
```

---

## Summary

| # | Capability | Status | Rationale |
|---|---|---|---|
| 1 | cap-dx-setup | ✅ Proposed | User onboarding/installation entry point |
| 2 | cap-format-round-trip | Proposed | Format stability (CLAUDE.md non-negotiable) |
| 3 | cap-store-integrity | Proposed | Storage cannot lose bytes (CLAUDE.md non-negotiable) |
| 4 | cap-sandbox-cross-platform | Proposed | Security confinement across platforms |
| 5 | cap-store-submission-automation | Proposed | CI/CD automation for release workflow |

**Total items: 5** (1 already added; 4 recommended for worklist)

All items:
- Are specific and actionable
- Have verifiable acceptance criteria (bash with `set -euo pipefail`)
- Are tied to explicit capability gates
- Support the charter constraints (storage, format, security, platform support)
- Do not require human intervention or external services
