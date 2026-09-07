# Status: Item 1 Decomposition Attempt

**Date**: 2026-09-07  
**Task**: Decompose failed worklist item 1 (cap-dx-setup gate) into 2-5 sub-items  
**Status**: ✅ DECOMPOSITION COMPLETE | ❌ WORKLIST INTEGRATION BLOCKED

## What Was Completed

✅ Analyzed the failed item: `- [ ] [scout] [ask: charter-DOC] [cap: cap-dx-setup] Implement cap-dx-setup gate...`

✅ Decomposed into 4 mechanical sub-items (each <10 min):
1. Create gates/cap-dx-setup.sh with bash header
2. Implement dx setup verification  
3. Add PATH and MCP checks
4. Test complete gate

✅ Documented sub-items with:
- Exact file paths (gates/cap-dx-setup.sh)
- Verifiable outcomes (file existence, exit codes, command checks)
- Capability gate advanced ([cap: cap-dx-setup])
- Proper format: `- [ ] [scout] [ask: charter-DOC] [cap: <id>] <text>`

✅ Created detailed spec: ITEM-1-DECOMPOSITION.md

## Blocker: Schema Mismatch

**Issue**: Cannot execute `dx_append` to add sub-items to index.dx#now-worklist

```
Error: this document store was written by a newer dx (schema 5, this build understands 4); 
upgrade dx to open it
```

**Root cause**: 
- Document store (.doc/repo.dxcp) uses schema 5
- MCP dx_append tool uses schema 4
- Write operations require schema compatibility

**Reported**: Bug report-49cce250 filed with dx team

## What Should Happen (Once Schema is Resolved)

The 4 sub-items from ITEM-1-DECOMPOSITION.md should be added to `index.dx#now-worklist`:

1. Insert 4 new checklist items after the first item
2. Change first item from `[ ]` to `[x]`  
3. Run `dx sync` to update document store

## Workaround Options

1. **Upgrade dx tool to schema 5**: Requires compilation (doc-cli/src/commands/setup.rs missing imports)
2. **Downgrade document store**: May lose newer features
3. **Manual git manipulation**: Risk of corruption
4. **Wait for schema compatibility fix**: Safest option

## Related Files

- ITEM-1-DECOMPOSITION.md — Full sub-item specifications
- DECOMPOSITION-ITEM-1-CAP-DX-SETUP.md — Quick reference
- ITEM-8-DECOMPOSITION.md — Previous decomposition with same schema issue
- SCOUT-ITEMS-TO-ADD.md — Earlier scout decomposition (same blocker)

## Evidence

The decomposition meets all task requirements:
- ✅ 2-5 sub-items (4 items created)
- ✅ Mechanically verifiable outcomes
- ✅ Exact files and commands named
- ✅ Each capability gate identified
- ✅ Each sub-item <10 minutes
- ✅ No human-required actions
- ✅ Proper format with [scout], [ask:], [cap:]

## Next Steps

1. ⏳ Wait for schema compatibility (dx tool upgrade or document store downgrade)
2. ⏳ Use dx_append (or manual integration) to add sub-items to worklist
3. ⏳ Dispatch items to haiku workers
4. ⏳ Close this blocker once worklist integration succeeds

**Commit**: Contains decomposition specification and this status. Item 1 remains open until worklist integration completes.
