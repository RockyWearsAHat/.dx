# Worklist Item 8: Implement `dx rename` — Decomposition

**Date**: 2026-09-04  
**Status**: REWRITTEN INTO 5 SUB-ITEMS

## Summary

Item 8 (implement `dx rename <old> <new>` with approval system) exceeded the 20-minute task wall after 2 haiku iterations due to scope: integrating a reference graph tracer, building an approval ledger, implementing preview/apply logic, and ensuring atomic safety.

Rewritten as 5 mechanical sub-items, each advancing `cap-doc-cli-build`, each completable in <10 minutes by a scout worker.

## Sub-Item Specifications

### Item 8.1: Build and test rename_approvals module [cap: cap-doc-cli-build]

**Files**: `rust/doc-cli/src/rename_approvals.rs`

**Description**: Implement the local approval ledger for rename fingerprints — a marker file per approved rename in the cache root, mirroring `doc-run::approvals` but for renames, not code execution.

**Verifiable Outcome**:
```bash
cd rust
cargo test --lib rename_approvals
```
Must pass: all 5 tests in rename_approvals (recorded, unrecorded, survive, malformed, well_formed).

**Capability Gate**: `cap-doc-cli-build` — ledger module builds and tests pass.

---

### Item 8.2: Integrate rename_approvals into cli main [cap: cap-doc-cli-build]

**Files**: `rust/doc-cli/src/main.rs`

**Description**: Add the rename_approvals module as a top-level CLI module.

**Changes**:
- Add `mod rename_approvals;` to main.rs

**Verifiable Outcome**:
```bash
cd rust
cargo build -p doc-cli 2>&1 | grep -c "error"
```
Must return 0 (no compilation errors).

**Capability Gate**: `cap-doc-cli-build` — main builds with rename_approvals available.

---

### Item 8.3: Scaffold rename command module [cap: cap-doc-cli-build]

**Files**:
- `rust/doc-cli/src/commands/mod.rs`
- `rust/doc-cli/src/commands/rename.rs`

**Description**: Define the rename command module with types and basic structure:
- `RenamePreview` struct (old_name, new_name, definitions, references, graph_digest)
- `RenameLocation` struct (file, line)
- Stub `run()`, `preview()`, `apply()`, and helper functions
- Module documentation

**Verifiable Outcome**:
```bash
cd rust
cargo build -p doc-cli
cargo clippy -p doc-cli -- -D warnings
cargo fmt --check
```
All three must pass (compiles, no clippy warnings, formatted correctly).

**Capability Gate**: `cap-doc-cli-build` — rename command module structure compiles and lints clean.

---

### Item 8.4: Implement rename preview and validation [cap: cap-dx-run]

**Files**: `rust/doc-cli/src/commands/rename.rs`

**Description**: Implement the preview pipeline:
- `build_trace()` — call doc_core::trace over the root directory
- `preview()` — use the trace to find definitions_of() and references_to() for old_name
- Validation: refuse if old == new, empty new, no definitions, definitions/references in skipped paths
- `compute_graph_digest()` — hash the trace state for this symbol
- Error messages for each validation failure

**Verifiable Outcome**:
```bash
cd rust
cargo test --lib commands::rename::tests
```
Must pass: tests for preview (valid rename, no definitions, skipped path, identical names).

**Capability Gate**: `cap-dx-run` — preview correctly finds and validates renames against the reference graph.

---

### Item 8.5: Implement rename apply and integration [cap: cap-dx-run]

**Files**: `rust/doc-cli/src/commands/rename.rs`

**Description**: Complete the rename CLI:
- `compute_rename_fingerprint()` — hash (old_name, new_name, sorted sites, graph_digest)
- Approval ledger check: `ledger.is_approved(fingerprint)`
- `apply()` — re-check graph state, read each file, verify old_name at each line, replace in memory, write atomically
- `run()` — orchestrate preview, check approval, apply, and report results
- Test end-to-end: approve a rename, apply it, verify it fails idempotently (graph has changed)

**Verifiable Outcome**:
```bash
cd rust
cargo test --lib commands::rename
cargo build --release -p doc-cli
# Manual: run `./target/release/doc-cli rename old_sym new_sym` in a test project
```
Must pass: all tests, binary builds, and a manual end-to-end test in a 2-file project shows the rename was applied.

**Capability Gate**: `cap-dx-run` — full rename command executes, checks approvals, applies atomically, and reports correctly.

---

## Total Scope

| Item | Scope | Gate | Difficulty |
|------|-------|------|------------|
| 8.1  | Ledger module (113 lines) | cap-doc-cli-build | Build mechanics |
| 8.2  | Main integration (1 line) | cap-doc-cli-build | Linker test |
| 8.3  | Scaffold types/stubs (~50 lines) | cap-doc-cli-build | Type design |
| 8.4  | Preview & trace (~80 lines) | cap-dx-run | Reference graph |
| 8.5  | Apply & approve (~80 lines) | cap-dx-run | File I/O safety |

Each is independently testable. Items 8.1-8.3 build the foundation; 8.4-8.5 wire in the logic.

---

## Worklist Checklist Items (For Manual Entry)

If using dx_append is blocked, these items can be manually added to `index.dx#now-worklist`:

```
- [ ] [scout] [cap: cap-doc-cli-build] Build and test rename_approvals ledger module: `cargo test --lib rename_approvals` in rust/ must pass all 5 tests
- [ ] [scout] [cap: cap-doc-cli-build] Integrate rename_approvals into cli main: add `mod rename_approvals;` to rust/doc-cli/src/main.rs, `cargo build -p doc-cli` must succeed
- [ ] [scout] [cap: cap-doc-cli-build] Scaffold rename command with types and stubs: define RenamePreview, RenameLocation, run/preview/apply signatures in rust/doc-cli/src/commands/rename.rs, `cargo clippy` must pass
- [ ] [scout] [cap: cap-dx-run] Implement rename preview and validation: trace-based lookup, definitions_of/references_to, validation (no skipped paths, old!=new), graph_digest; `cargo test --lib commands::rename::tests` passes
- [ ] [scout] [cap: cap-dx-run] Implement rename apply and integration: compute_rename_fingerprint, ledger approval check, atomic file writes, end-to-end test; manual rename in 2-file project succeeds
```

Then tick the original item 8 line: `[x] Implement dx rename...`

## Handoff

Once all 5 sub-items are complete and passing:
- `cargo test` passes across all doc-cli tests
- `cargo clippy` and `cargo fmt` pass
- Manual end-to-end rename in a test project succeeds
- Item 8 worklist line is ticked [x]

The full feature is then ready for integration into the main CLI command table and documented in CLAUDE.md as the stated exception to "reading never writes."
