# Scout Assessment - Current State

## Understanding Section
✅ **EXISTS**: The project has a comprehensive understanding statement in `index.dx#paragraph-2`:
> "dx is a document format, a store, and a toolchain: a .dx document is block-structured, renders to a page, and can execute the code blocks inside it. README.md is the front door, CLAUDE.md is the working contract, and docs/dx-format-contract.dx is the authority on format behavior..."

## Capability Gates - 7/7 Status Analysis

### Gate Implementation Summary
| Gate | Status | Type | Issue |
|------|--------|------|-------|
| cap-doc-cli-build | Placeholder | Code block in index.dx | No verification logic |
| cap-dx-render | Placeholder | Code block in index.dx | No verification logic |
| cap-dx-run | Placeholder | Code block in index.dx | No verification logic |
| cap-vscode | Placeholder | Code block in index.dx | No verification logic |
| cap-archives | Placeholder | Code block in index.dx | No verification logic |
| cap-version | Placeholder | Code block in index.dx | No verification logic |
| cap-dx-doctor-store | Implemented | Code block in index.dx | Works with proper `set -euo pipefail` |

**Note**: All gate code blocks show output "--- ran without a sandbox: DX_UNCONFINED is set, so this code had your own permissions ---" indicating they ran with DX_UNCONFINED, not in sandbox.

### Separate Gate Scripts
- `gates/cap-format-round-trip.sh` - Properly implemented with `set -euo pipefail`, runs cargo test

## Current Worklist
The `now-worklist` block contains 7 open items + 2 completed:

### Open Items (7)
1. [ ] [cap: cap-dx-setup] Implement cap-dx-setup gate
2. [ ] Store distribution (not a capability gate)
3. [ ] [cap: cap-format-round-trip] Implement cap-format-round-trip gate
4. [ ] [cap: cap-store-integrity] Implement cap-store-integrity gate
5. [ ] [cap: cap-sandbox-cross-platform] Implement cap-sandbox-cross-platform gate
6. [ ] Add LICENSE file
7. [ ] Create .gitignore

### Completed Items (2)
- [x] Add comprehensive README.md
- [x] Set up CI/CD pipeline

## Critical Findings

### 1. Existing Gates NOT in Worklist
The 6 broken gates (cap-doc-cli-build, cap-dx-render, cap-dx-run, cap-vscode, cap-archives, cap-version) that are in the ## Finished section of index.dx do NOT have corresponding fix items in the worklist. These gates need to be fixed but have no assigned work.

### 2. Schema Version Blocker
The document store is schema version 5, but available dx tools only understand schema 4. This blocks:
- Using `dx_append` to add new items to worklist
- Automated tool-based updates to index.dx

**Workaround**: Items must be added manually to git (not via dx tools) or the schema compatibility must be resolved.

### 3. Commits vs Reality Gap
Recent commits repeatedly state "gates: 7/7 pass (all green)" but gate verification shows:
- 6 gates are just placeholder comments
- 1 gate is properly implemented
- All gates run with DX_UNCONFINED (no sandbox)

## Assessment

**Finished Section Status**: Misleading
- Listed as complete with "7/7 passing gates"
- Reality: only 1 gate is properly implemented, 6 are placeholders

**Worklist Status**: Incomplete  
- Has 7 items but missing work items for the 6 broken gates
- New gate items (cap-dx-setup, cap-format-round-trip, cap-store-integrity, cap-sandbox-cross-platform) are present
- Core capability gates should be fixed before adding new ones

## Recommended Actions

1. **Fix 6 Broken Gates** (Priority: High)
   - Implement proper verification for: cap-doc-cli-build, cap-dx-render, cap-dx-run, cap-vscode, cap-archives, cap-version
   - Each should follow pattern of cap-format-round-trip.sh or cap-dx-doctor-store

2. **Update Worklist** (Priority: High)
   - Add items to fix the 6 broken gates
   - Prioritize: cap-doc-cli-build (foundational) first

3. **Resolve Schema Blocker** (Priority: Medium)
   - Upgrade dx tool to schema 5, or
   - Downgrade document store to schema 4, or
   - Document manual workaround for adding items

## Next Scout Item
Once gates are properly implemented: propose new capability (e.g., cap-dx-sync, cap-dx-resolve)
