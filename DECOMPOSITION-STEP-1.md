# Step 1: Decompose Failed Worklist Item

## Problem
Item with `ask: wk-now-worklist-7/5` combines 3 independent capabilities into one:
- cap-archives
- cap-version  
- cap-dx-doctor-store

This caused "team already dispatched" error because multiple independent capabilities were in one dispatch.

## Solution
Decompose into 3 separate sub-items, each with unique ask ID and single capability:

### Sub-Item 1: cap-archives verification
- **Ask ID**: `wk-now-worklist-0/5`
- **Capability**: `cap-archives`
- **Task**: Verify archive build: run `cargo build --release && cd packaging && npm run build`; check .xpi and .zip exist and are non-empty
- **Verifiable outcome**: Both archive files exist with non-zero byte count

### Sub-Item 2: cap-version verification
- **Ask ID**: `wk-now-worklist-0/6`
- **Capability**: `cap-version`
- **Task**: Verify version consistency: grep "version" from Cargo.toml and editor/package.json; confirm same version string in both
- **Verifiable outcome**: Version strings match between files

### Sub-Item 3: cap-dx-doctor-store verification
- **Ask ID**: `wk-now-worklist-0/7`
- **Capability**: `cap-dx-doctor-store`
- **Task**: Run `dx doctor` in repo root; confirm exit code 0 and all checks pass
- **Verifiable outcome**: Exit code 0 with no errors

## Change to Apply
In `index.dx#now-worklist`, replace the single line (ask: wk-now-worklist-7/5):
```
- [ ] [scout] [ask: wk-now-worklist-7/5] [cap: cap-archives] Verify cap-archives and cap-version and cap-dx-doctor-store: Run packaging build, confirm .xpi and .zip archives exist, run dx doctor and confirm all checks pass
```

With 3 separate lines:
```
- [ ] [scout] [ask: wk-now-worklist-0/5] [cap: cap-archives] Verify archive build: run cargo build --release && cd packaging && npm run build; check .xpi and .zip exist and are non-empty
- [ ] [scout] [ask: wk-now-worklist-0/6] [cap: cap-version] Verify version consistency: grep "version" Cargo.toml and editor/package.json; confirm same version string in both files
- [ ] [scout] [ask: wk-now-worklist-0/7] [cap: cap-dx-doctor-store] Run dx doctor: execute `dx doctor` in repo root; confirm exit code 0 and all checks pass with no errors
```

## Why This Decomposition Works
1. **Mechanically completable**: Each sub-item has a single clear command/verification
2. **Under 10 minutes each**: A haiku worker can run the command and verify the outcome in < 10 minutes
3. **No human intervention needed**: All items are automated checks
4. **Independent dispatch**: Each with unique ask ID prevents "already dispatched" error
5. **Follows the pattern**: Matches existing sub-items in format and specificity
