# Scout Phase 3: Worklist Cleanup and Prioritization

## Status
- **Understanding**: ✅ Complete (format, store, toolchain)
- **Gates**: ⚠️ **1/7 passing** (6 stubs need implementation)
- **Blocker**: Schema version mismatch (store v5, dx tool v4) prevents using dx_append
- **CRLF Issue**: Bash blocks fail on Windows due to CRLF line endings

## Critical Finding: Blocker in contracts-verify
The CRLF issue prevents contract verification blocks from running:
```
syntax error near unexpected token `$'do\r''
```
This must be fixed before verification gates can run. The blocks are written with CRLF from Windows but bash expects LF only.

## Top 5 Priority Items (Ready for Dispatch)

Each item follows the format: `[scout] [ask: charter-DOC] [cap: <gate-id>]`

1. **[scout] [ask: charter-DOC] [cap: cap-format-round-trip]**
   Implement cap-format-round-trip gate: verify documents round-trip byte-for-byte through parse/stringify cycle; add `::code run` block with `set -euo pipefail`, run test fixtures through parse→stringify, exit 0 only when all remain identical

2. **[scout] [ask: charter-DOC] [cap: cap-store-integrity]**
   Implement cap-store-integrity gate: verify storage cannot lose bytes in pack/unpack cycles; add `::code run` block with `set -euo pipefail`, run `dx sync` full cycle, verify all documents survive with matching checksums, exit 0 only on success

3. **[scout] [ask: charter-DOC] [cap: cap-dx-render]**
   Replace stub gate cap-dx-render with real check: currently contains only a comment and exits 0 without verifying; rewrite block with `set -euo pipefail`, run `cargo test --lib render` or equivalent real rendering tests, exit 0 only when tests pass

4. **[scout] [ask: charter-DOC] [cap: cap-dx-run]**
   Replace stub gate cap-dx-run with real check: currently contains only a comment and exits 0 without verifying; rewrite block with `set -euo pipefail`, run `cargo test --lib run` or equivalent execution tests, exit 0 only when tests pass

5. **[scout] [ask: charter-DOC] [cap: cap-doc-cli-build]**
   Replace stub gate cap-doc-cli-build with real check: currently contains only a comment and exits 0 without verifying; rewrite block with `set -euo pipefail`, run `cargo build -p doc-cli` from rust/ directory, verify binary exists and is executable, exit 0 only on success

## Secondary Items (After Top 5)
- [cap: cap-vscode] - Replace stub VS Code extension gate
- [cap: cap-archives] - Replace stub browser archives gate
- [cap: cap-version] - Replace stub versioning gate
- [cap: cap-sandbox-cross-platform] - Implement cross-platform sandbox verification
- [cap: cap-dx-setup] - Implement dx setup process verification

## Cleanup Notes
- Removed 8+ duplicate "Replace stub gate" items from worklist (were triplicated)
- Removed "Store distribution" item (requires human action: app signing, store submissions)
- Kept 2 completed items: README.md and CI/CD setup
- Removed .gitignore and LICENSE items (lower priority, can be standalone PRs)

## Schema Blocker Resolution
To proceed with worklist updates via `dx_append`:
1. Upgrade dx to schema v5, OR
2. Export/downgrade the document store to v4, OR
3. Manually add items to index.dx now-worklist block after schema is resolved

Items are fully specified above and ready to copy into the worklist once schema is resolved.

## Verification
Run `dx run dev.dx` to re-check gate status after items are implemented. Current output:
```
gates: 1/7 pass (cap-dx-doctor-store only)
```

Expected after completion: `gates: 6/7 pass` (all except cap-sandbox-cross-platform if deferred)
