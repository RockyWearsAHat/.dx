# Scout Gate Decomposition: Fix Broken Gates

## Overview

This document contains the decomposition of the failed worklist item "Scout: stage worklist sub-items" into 6 mechanically verifiable sub-items for haiku worker assignment.

Each sub-item names exact files/commands, provides a single verifiable outcome, and references a capability gate that it advances.

## Blocker Notification

**Schema Mismatch**: Document store schema 5 vs dx tool schema 4 incompatibility prevents using `dx_append` to directly add these items to `index.dx#now-worklist`. These items are ready for integration once schema compatibility is resolved or a schema v5-compatible dx tool becomes available.

## Decomposed Sub-Items (6 items)

### Item 1: Implement cap-doc-cli-build gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Implement cap-doc-cli-build gate: add bash script with `set -euo pipefail` to verify `cargo build -p doc-cli` succeeds in rust/ directory; gate exits 0 only when build completes without errors
```

**Exact File**: `gates/cap-doc-cli-build.sh`

**Exact Command for Verification**: 
```bash
bash gates/cap-doc-cli-build.sh
```

**Single Verifiable Outcome**: 
- Script exists at `gates/cap-doc-cli-build.sh`
- Contains bash header with `set -euo pipefail`
- Runs `cd rust && cargo build -p doc-cli`
- Exits with code 0 on success, non-zero on failure
- No error suppression (`|| true`, `|| echo`)

**Capability Gate**: `cap-doc-cli-build`

---

### Item 2: Implement cap-dx-render gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-render] Implement cap-dx-render gate: add bash script with `set -euo pipefail` to verify document rendering produces valid output; run `cargo test --lib render` in rust/ directory; gate exits 0 only when all render tests pass
```

**Exact File**: `gates/cap-dx-render.sh`

**Exact Command for Verification**:
```bash
bash gates/cap-dx-render.sh
```

**Single Verifiable Outcome**:
- Script exists at `gates/cap-dx-render.sh`
- Contains bash header with `set -euo pipefail`
- Runs `cd rust && cargo test --lib render`
- Exits with code 0 on success, non-zero on failure
- All render-related tests in doc-core pass

**Capability Gate**: `cap-dx-render`

---

### Item 3: Implement cap-dx-run gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Implement cap-dx-run gate: add bash script with `set -euo pipefail` to verify code execution and output storage; run `cargo test --lib run` in rust/ directory; gate exits 0 only when execution tests pass
```

**Exact File**: `gates/cap-dx-run.sh`

**Exact Command for Verification**:
```bash
bash gates/cap-dx-run.sh
```

**Single Verifiable Outcome**:
- Script exists at `gates/cap-dx-run.sh`
- Contains bash header with `set -euo pipefail`
- Runs `cd rust && cargo test --lib run`
- Exits with code 0 on success, non-zero on failure
- All execution-related tests in doc-core pass

**Capability Gate**: `cap-dx-run`

---

### Item 4: Implement cap-vscode gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-vscode] Implement cap-vscode gate: add bash script with `set -euo pipefail` to verify VS Code extension builds successfully; run `cd editor/vscode && npm run build` and verify extension package is created; gate exits 0 only when build succeeds
```

**Exact File**: `gates/cap-vscode.sh`

**Exact Command for Verification**:
```bash
bash gates/cap-vscode.sh
```

**Single Verifiable Outcome**:
- Script exists at `gates/cap-vscode.sh`
- Contains bash header with `set -euo pipefail`
- Runs `cd editor/vscode && npm run build`
- Verifies extension package file is created
- Exits with code 0 on success, non-zero on failure

**Capability Gate**: `cap-vscode`

---

### Item 5: Implement cap-archives gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-archives] Implement cap-archives gate: add bash script with `set -euo pipefail` to verify browser extension archives are built and valid; verify Firefox and Chrome archive files exist and are signed/valid; gate exits 0 only when all archives are present
```

**Exact File**: `gates/cap-archives.sh`

**Exact Command for Verification**:
```bash
bash gates/cap-archives.sh
```

**Single Verifiable Outcome**:
- Script exists at `gates/cap-archives.sh`
- Contains bash header with `set -euo pipefail`
- Verifies Firefox XPI archive exists and is valid
- Verifies Chrome ZIP archive exists and is valid
- Exits with code 0 on success, non-zero on failure

**Capability Gate**: `cap-archives`

---

### Item 6: Implement cap-version gate

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-version] Implement cap-version gate: add bash script with `set -euo pipefail` to verify version command returns valid semantic version; run `cargo run -p doc-cli -- version` and parse output as semver; gate exits 0 only when output is valid semver format
```

**Exact File**: `gates/cap-version.sh`

**Exact Command for Verification**:
```bash
bash gates/cap-version.sh
```

**Single Verifiable Outcome**:
- Script exists at `gates/cap-version.sh`
- Contains bash header with `set -euo pipefail`
- Runs `cargo run -p doc-cli -- version`
- Parses output as valid semantic version (X.Y.Z format)
- Exits with code 0 on success, non-zero on failure

**Capability Gate**: `cap-version`

---

## Summary

**Total Sub-Items**: 6

**Capability Gates Advanced**:
1. cap-doc-cli-build
2. cap-dx-render
3. cap-dx-run
4. cap-vscode
5. cap-archives
6. cap-version

**Estimated Completion Time**: Each item ~5-10 minutes for haiku worker (creates/updates one bash script)

**Next Steps**:
1. Once schema v5 compatibility is available, use `dx_append` to add these items to `index.dx#now-worklist`
2. Close the original "Scout: stage worklist sub-items" worklist item by ticking its checkbox
3. Assign each sub-item to haiku workers for implementation

## Reference

See previous decomposition attempt: `SCOUT-ITEMS-TO-ADD.md` (similar item list with additional context)

See previous successful decomposition: commit 0547b40 (ITEM-1-DECOMPOSITION.md) for similar pattern
