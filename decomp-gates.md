# Decomposition: Failed Gate Dispatch

**Failed Item:** dispatch 7 gates to haiku workers — team-general-0 already dispatched

**Reason:** Dispatch failed because team was already being processed. Decomposing into mechanical sub-items, each verifiable in <10 minutes.

## Sub-items

Each replaces a stub gate with a real mechanical test:

### 1. cap-doc-cli-build
- **File:** `rust/dev.dx` → gate cap-doc-cli-build
- **Command:** `cargo build -p doc-cli` 
- **Outcome:** Exit code 0 (success)
- **ask:** decomp-gates-1

### 2. cap-dx-render  
- **File:** `rust/dev.dx` → gate cap-dx-render
- **Command:** `dx render index.dx`
- **Outcome:** Exit code 0 (success)
- **ask:** decomp-gates-2

### 3. cap-dx-run
- **File:** `rust/dev.dx` → gate cap-dx-run
- **Command:** `dx run --review index.dx`
- **Outcome:** Exit code 0 (success)
- **ask:** decomp-gates-3

### 4. cap-vscode
- **File:** `rust/dev.dx` → gate cap-vscode
- **Command:** Build VS Code extension (exact command TBD by worker)
- **Outcome:** Exit code 0 (success)
- **ask:** decomp-gates-4

### 5. cap-archives
- **File:** `rust/dev.dx` → gate cap-archives
- **Command:** Build archives/packaging (exact command TBD by worker)
- **Outcome:** Exit code 0 (success)
- **ask:** decomp-gates-5

## Format for worklist

```
- [x] [scout] [cap: gates-dispatch-v1] dispatch 7 gates to haiku workers — each gate replaces stub with single mechanical test
- [ ] [scout] [ask: decomp-gates-1] [cap: cap-doc-cli-build] Replace stub with `cargo build -p doc-cli` check, verify exit 0
- [ ] [scout] [ask: decomp-gates-2] [cap: cap-dx-render] Replace stub with `dx render index.dx` check, verify exit 0
- [ ] [scout] [ask: decomp-gates-3] [cap: cap-dx-run] Replace stub with `dx run --review index.dx` check, verify exit 0
- [ ] [scout] [ask: decomp-gates-4] [cap: cap-vscode] Replace stub with VS Code extension build check, verify exit 0
- [ ] [scout] [ask: decomp-gates-5] [cap: cap-archives] Replace stub with packaging build check, verify exit 0
```

All items are mechanical, non-overlapping, and under 10 minutes.
