# Decomposition: Failed Dispatch - Capability Gate Verification

## Original Failed Item
Team dispatch failed with: "dispatch team team-general-0: team team-general-0 already dispatched"

This was an attempt to delegate verification of all 7 capability gates to the haiku worker team.

## Root Cause
The dispatch tried to send a single large item when the team was already handling it, causing a duplication conflict.

## Decomposed Sub-Items (2–5 mechanical tasks, <10 min each)

### Sub-item 1: Verify cap-doc-cli-build
- **Capability**: cap-doc-cli-build
- **Command**: cd rust && cargo build -p doc-cli
- **Verifiable outcome**: Binary exists at rust/target/debug/doc-cli (exit 0 on success)
- **Time**: ~3 minutes
- **Description**: [scout] [ask: charter-DOC] [cap: cap-doc-cli-build] Build Rust CLI to verify doc-cli binary compiles without errors; exit 0 if build succeeds

### Sub-item 2: Verify cap-dx-render
- **Capability**: cap-dx-render  
- **Command**: cd rust && ./target/debug/dx render examples/showcase.dx --out /tmp/test.html
- **Verifiable outcome**: HTML file exists at /tmp/test.html with size > 1000 bytes (exit 0 on success)
- **Time**: ~2 minutes
- **Description**: [scout] [ask: charter-DOC] [cap: cap-dx-render] Render a sample document to verify dx render command produces valid HTML output; exit 0 if output file created

### Sub-item 3: Verify cap-dx-run
- **Capability**: cap-dx-run
- **Command**: cd rust && ./target/debug/dx run --review tests/fixtures/cap-run.dx
- **Verifiable outcome**: Code block executes without error and output is recorded (exit 0 on success)
- **Time**: ~2 minutes
- **Description**: [scout] [ask: charter-DOC] [cap: cap-dx-run] Run a test document with executable code blocks to verify dx run command executes code in sandboxed environment; exit 0 if run succeeds

### Sub-item 4: Verify cap-vscode
- **Capability**: cap-vscode
- **Command**: test -f editor/wasm/doc_wasm.js && file editor/wasm/doc_wasm.js | grep -q "WebAssembly"
- **Verifiable outcome**: WASM file exists and is valid WebAssembly (exit 0 on success)
- **Time**: ~1 minute
- **Description**: [scout] [ask: charter-DOC] [cap: cap-vscode] Verify VS Code extension wasm artifact exists and is valid WebAssembly; exit 0 if file verified

### Sub-item 5: Verify cap-archives + cap-version + cap-dx-doctor-store
- **Capability**: cap-archives, cap-version, cap-dx-doctor-store (combined)
- **Commands**: 
  - Archive check: test -f packaging/signed/dx-chrome.zip && file packaging/signed/dx-chrome.zip | grep -q "Zip"
  - Version check: ./target/debug/dx version | grep -E '^[0-9]+\.[0-9]+\.[0-9]+'
  - Doctor check: ./target/debug/dx doctor | grep -q "Status:"
- **Verifiable outcome**: All 3 checks pass (exit 0 on success)
- **Time**: ~2 minutes  
- **Description**: [scout] [ask: charter-DOC] [cap: cap-archives] Verify signed browser archives exist, version command works, and doctor tool validates store; exit 0 if all 3 checks pass

## Schedule
- Each sub-item assigned to haiku worker with 10-minute budget
- Items processed in parallel to restore capability verification
- Original dispatch conflict resolved by breaking into independent, non-overlapping tasks

## Commit Strategy
- Mark original dispatch failure item as complete [x]
- Add these 5 sub-items as new [scout] items to now-worklist
- Format: `- [ ] [scout] [ask: <askId>] [cap: <capId>] <text>`
