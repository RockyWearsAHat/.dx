# Decomposition: Failed Gate Implementation Dispatch

**Blocker**: Document store schema mismatch (v5 vs v4) prevents `dx_append` from working. These sub-items must be appended to `index.dx#now-worklist` once schema is resolved via `dx sync` or version upgrade.

## Sub-items to append to now-worklist (in order)

```
- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Fix cap-doc-cli-build gate stub: replace `# Gate verification block` comment in gates/cap-doc-cli-build.sh with `set -euo pipefail; cd rust && cargo build -p doc-cli >/dev/null 2>&1`; verify: `bash gates/cap-doc-cli-build.sh` exits 0

- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-render] Fix cap-dx-render gate stub: replace `# Gate verification block` comment in gates/cap-dx-render.sh with `set -euo pipefail; dx render index.dx >/dev/null 2>&1`; verify: `bash gates/cap-dx-render.sh` exits 0

- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Fix cap-dx-run gate stub: replace `# Gate verification block` comment in gates/cap-dx-run.sh with `set -euo pipefail; dx run --review index.dx >/dev/null 2>&1`; verify: `bash gates/cap-dx-run.sh` exits 0

- [ ] [scout] [ask: charter-DOC] [cap: cap-vscode] Fix cap-vscode gate stub: replace `# Gate verification block` comment in gates/cap-vscode.sh with `set -euo pipefail; cd editor/vscode && npm run build >/dev/null 2>&1`; verify: `bash gates/cap-vscode.sh` exits 0

- [ ] [scout] [ask: charter-DOC] [cap: cap-archives] Fix cap-archives gate stub: replace `# Gate verification block` comment in gates/cap-archives.sh with `set -euo pipefail; bash ./packaging/build-stores.sh --dry-run >/dev/null 2>&1`; verify: `bash gates/cap-archives.sh` exits 0
```

## Why this decomposition

The original dispatch failed because:
1. Multiple duplicate worklist items (cap-doc-cli-build, cap-dx-render, etc. appear 2+ times each)
2. Team team-general-0 already dispatched (previous attempt stuck or incomplete)
3. Gate stubs do not verify anything — they only echo `# Gate verification block` and exit 0

Each sub-item above is:
- **Mechanical**: One file, one replace operation, one verification command
- **Under 10 minutes**: Bash string replacement + single test run
- **Verifiable**: Exit code check confirms gate now validates real capability
- **Non-overlapping**: Each sub-item touches a different gate file

## Integration

After schema upgrade, merge these items into `index.dx#now-worklist` via:
```bash
dx_append D:\SARA\Desktop\DOC\index.dx now-worklist < DECOMPOSITION-STUB-GATES.md
```

Or manually copy the checklist lines above into the worklist block.

## Status

- [ ] Tick when sub-items are added to index.dx and this file is deleted
