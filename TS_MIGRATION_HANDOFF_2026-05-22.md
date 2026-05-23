# TypeScript Migration Handoff (2026-05-22)

## Objective Completed
The runtime and webview JavaScript modules were converted to TypeScript source files while preserving JavaScript runtime artifacts.

## What Was Converted
- `src/`: 14 modules converted from `.js` source to `.ts` source.
- `vscode-extension/media/`: 17 modules converted from `.js` source to `.ts` source.
- Existing emitted/runtime `.js` files remain present and were regenerated from TypeScript.

## Build System Changes
- Added root TypeScript project config: `tsconfig.json`.
- Added script: `npm run build:ts` (`npx tsc --project tsconfig.json`).
- Kept `build:surface` as alias to `build:ts` for compatibility.
- Updated `tsconfig.surface.json` and `vscode-extension/media/tsconfig.surface.json` to extend the root config.
- Added/updated Node type dependency in dev dependencies (`@types/node`).

## Validation Performed
- TypeScript compile invoked with emit enabled:
  - `npm run build:ts`
  - Log path: `/tmp/doc-ts-build.log`
- Test suite run after conversion:
  - `npm test`
  - Result: pass 206, fail 0

## Current TypeScript Error Backlog
- Total diagnostics: **1534** (`grep -c "error TS" /tmp/doc-ts-build.log`).
- Dominant error categories:
  - `TS7006` implicit `any` parameters
  - `TS2339` missing property on broad types (especially `EventTarget` in DOM handlers)
  - `TS18047` possible `null` references
  - `TS7053` indexed access on untyped object maps
  - `TS18046` `unknown` narrowing gaps

Top files by error count:
1. `vscode-extension/media/webview.ts` (402)
2. `vscode-extension/media/webview-edit-controllers.ts` (129)
3. `vscode-extension/media/webview-events.ts` (119)
4. `src/mcp-server.ts` (87)
5. `vscode-extension/media/webview-state-core.ts` (80)
6. `src/doc-format.ts` (78)
7. `vscode-extension/media/webview-doc-model.ts` (74)
8. `vscode-extension/media/doc-pipeline.ts` (71)
9. `vscode-extension/media/webview-document-lifecycle.ts` (62)
10. `src/database.ts` (61)

## Recommended Next Steps For Next Agent

### Phase 1: Unblock strict typing in core backend first
- Start with `src/database.ts`, `src/doc-service.ts`, `src/doc-format.ts`, `src/doc-archive.ts`, `src/doc-binary.ts`.
- Add foundational shared types in one place (document model, section model, db row shapes, metadata maps).
- Replace implicit-any function signatures with explicit input/output interfaces.
- Add index signatures or `Record<string, ...>` where dynamic object maps are intentional.

### Phase 2: Fix webview DOM typing seams
- In `vscode-extension/media/webview.ts` and related controllers:
  - Narrow `event.target` via `instanceof` checks or typed helper guards.
  - Replace broad `EventTarget` assumptions with `HTMLTextAreaElement`, `HTMLElement`, `HTMLInputElement`, etc.
  - Add null guards for query-selected nodes (`if (!node) return`).
  - Tighten autocomplete state shape and action signatures.

### Phase 3: Stabilize module contracts
- Extract cross-module interfaces into a dedicated type module (for example `src/types.ts` and `vscode-extension/media/types.ts`).
- Ensure exported functions have explicit return types where inference currently leaks `any`.
- Resolve all `unknown` diagnostics by schema validation and narrowing helpers.

### Phase 4: Raise quality gates progressively
- Keep `strict: true`.
- Triage by file and commit in vertical slices:
  1. backend persistence slice
  2. doc parse/format slice
  3. webview state/events slice
  4. webview rendering/controller slice
- After each slice:
  - `npm run build:ts`
  - `npm test`

## Notes
- This migration intentionally prioritized full source conversion and runtime continuity over immediate diagnostic cleanup.
- Runtime behavior remains validated by tests, but static typing debt is now explicit and ready for incremental elimination.
