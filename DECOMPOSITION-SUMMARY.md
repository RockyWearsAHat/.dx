# Decomposition Summary: Failed Dispatch Resolution

## Task Completed: Decompose Failed Team Dispatch into Mechanical Sub-Items

### Original Failed Item
**Status**: [x] COMPLETE (decomposed)
**Error**: `team dispatch failed: dispatch team team-general-0: team team-general-0 already dispatched`
**Root Cause**: Attempted to send all 7 capability gate verifications as a single dispatch request, creating a conflict when the team was already handling related items.

## Decomposed Sub-Items (5 Mechanical Tasks)

All sub-items follow the format:
```
- [ ] [scout] [ask: charter-DOC] [cap: <capability_id>] <task description>
```

### Sub-Item 1: cap-doc-cli-build
- **Exact Command**: `cd rust && cargo build -p doc-cli`
- **Verifiable Outcome**: Binary exists at `rust/target/debug/doc-cli`
- **Exit Code**: 0 on success (compile succeeds)
- **Time Budget**: ~3 minutes
- **Worklist Entry**: `- [ ] [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Build Rust CLI to verify doc-cli binary compiles without errors; exact command: cd rust && cargo build -p doc-cli; exit 0 if build succeeds`

### Sub-Item 2: cap-dx-render
- **Exact Command**: `dx render examples/showcase.dx --out /tmp/test.html`
- **Verifiable Outcome**: HTML file created with size > 1000 bytes
- **Exit Code**: 0 on success (render completes)
- **Time Budget**: ~2 minutes
- **Worklist Entry**: `- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-render] Render sample document to verify dx render produces valid HTML output; exact command: dx render examples/showcase.dx --out /tmp/test.html; exit 0 if HTML file created with size > 1KB`

### Sub-Item 3: cap-dx-run
- **Exact Command**: `dx run --review tests/fixtures/cap-run.dx`
- **Verifiable Outcome**: Code executes and output is recorded in document
- **Exit Code**: 0 on success (execution completes)
- **Time Budget**: ~2 minutes
- **Worklist Entry**: `- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-run] Execute test document with code blocks in sandbox; exact command: dx run --review tests/fixtures/cap-run.dx; exit 0 if code executes and output recorded`

### Sub-Item 4: cap-vscode
- **Exact Command**: `test -f editor/wasm/doc_wasm.js && file editor/wasm/doc_wasm.js | grep -q "WebAssembly"`
- **Verifiable Outcome**: WASM file exists and is valid WebAssembly format
- **Exit Code**: 0 on success (file verified)
- **Time Budget**: ~1 minute
- **Worklist Entry**: `- [ ] [scout] [ask: charter-DOC] [cap: cap-vscode] Verify VS Code extension WASM artifact exists and is valid; exact command: test -f editor/wasm/doc_wasm.js && file editor/wasm/doc_wasm.js | grep -q "WebAssembly"; exit 0 if WASM file verified`

### Sub-Item 5: cap-archives + cap-version + cap-dx-doctor-store (Combined)
- **Exact Commands** (all must pass):
  1. `test -f packaging/signed/dx-chrome.zip && file packaging/signed/dx-chrome.zip | grep -q "Zip"`
  2. `dx version | grep -E '^[0-9]+\.[0-9]+\.[0-9]+'`
  3. `dx doctor | grep -q "Status:"`
- **Verifiable Outcome**: All three commands pass (exit codes are all 0)
- **Exit Code**: 0 on success (all checks pass)
- **Time Budget**: ~2 minutes
- **Worklist Entry**: `- [ ] [scout] [ask: charter-DOC] [cap: cap-archives] Verify signed browser archives and doctor tool; exact commands: (1) test -f packaging/signed/dx-chrome.zip && file packaging/signed/dx-chrome.zip | grep -q "Zip", (2) dx version | grep -E '^[0-9]+\.[0-9]+\.[0-9]+', (3) dx doctor | grep -q "Status:"; exit 0 if all three pass`

## Dispatch Strategy Change
- **Previous Approach**: Single team dispatch with all items
- **New Approach**: 5 independent sub-items, each with distinct capability gate
- **Benefit**: Eliminates dispatch conflicts; allows parallel processing; each item fits <10-minute time budget
- **Capability Coverage**: All 7 gates covered (5 items group 2 gates together)

## Files Created
- `DECOMP-GATE-DISPATCH.md` - Detailed decomposition with rationale
- `WORKLIST-DECOMPOSITION.txt` - Worklist format specification
- `DECOMPOSITION-SUMMARY.md` - This file

## Commit
Commit: e77043f
Message: "scout: decompose failed dispatch into 5 mechanical sub-items"
Includes complete decomposition documentation and exact command specifications for each sub-item.

## Status
✅ Decomposition complete and documented
⏳ Waiting for worklist integration (dx_append to index.dx#now-worklist may require schema update)
⏳ Sub-items ready for assignment to haiku workers

## Next Steps
1. Update index.dx#now-worklist with the 5 sub-items (each as a new checklist line)
2. Mark original dispatch failure item as complete [x]
3. Commit worklist changes
4. Dispatch updated worklist items to team
