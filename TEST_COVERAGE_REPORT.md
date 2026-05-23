# Comprehensive Test Coverage Report

Last verified: 2026-05-22

## Executive Summary

- `npm test`: pass (150 tests, 0 failures)
- `npm run test:coverage`: pass (enforced `--100` on covered surface)
- Enforced coverage result: 100% statements, 100% branches, 100% functions, 100% lines on the configured include list

The repo now enforces a strict coverage gate for critical runtime files and currently meets it fully.

## What Is Enforced

`test:coverage` in `package.json` enforces `--100` for this exact set of files:

- `src/database.js`
- `src/doc-service.js`
- `src/doc-format.js`
- `src/doc-binary.js`
- `src/doc-view.js`
- `src/view-state.js`
- `src/global-db-path.js`
- `src/sqlite-native.js`
- `vscode-extension/media/doc-pipeline.js`

If any one of those files drops below 100% in statements/branches/functions/lines, the coverage command fails.

## What The Tests Prove In Practice

- End-to-end document lifecycle works: ingest `.dx`, parse/normalize, persist to SQLite, pack archive payloads, reconstruct, and search.
- Canonical DOCSRC parsing is robust across normal and many edge inputs (headers, lists, checklist variants, quote/code/image/rule blocks, legacy markdown-like input, malformed boundary cases).
- Binary codec pack/unpack stays deterministic and rejects or safely handles corrupted payloads.
- Render pipeline emits expected HTML structure with escaping/sanitization and paper/theme/density/view-state attributes.
- View-state sanitization clamps/normalizes invalid values and safely handles corrupt JSON.
- Native SQLite bridge is active and usable via basic create/insert/select workflows.
- Global DB path fallback behavior is covered for set, blank, and unset env var states.

## Current Test Inventory

Primary test files:

- `test/database-docservice.integration.test.mjs`
- `test/doc-format.test.mjs`
- `test/doc-binary.test.mjs`
- `test/doc-view-and-pipeline.test.mjs`
- `test/doc-view-extra.test.mjs`
- `test/view-state.test.mjs`
- `test/global-db-path.test.mjs`
- `test/sqlite-native.test.mjs`
- `test/mcp-operations.unit.test.mjs`
- `test/coverage-final-1.test.mjs`
- `test/coverage-final-2.test.mjs`
- `test/coverage-gaps.test.mjs`
- `test/coverage-gaps-extended.test.mjs`

Observed run summary:

- tests: 150
- pass: 150
- fail: 0

## Known Non-Enforced / Not Fully Covered Areas

These areas are not part of the strict `--100` include list and can regress without failing `test:coverage`:

- `src/mcp-server.js` (tool protocol wiring and session lifecycle)
- `src/cli.js` (command parsing/output/exit behavior)
- `src/doc-view-capture.js` (Playwright and Quick Look fallback paths)
- `src/git-doc-state.js` and `src/file-discovery.js` git-shell integration behavior
- `vscode-extension/extension.js`, `vscode-extension/media/webview-main.js`, and `vscode-extension/media/webview.js`
- native C++ implementation internals in `native/sqlite_bridge.cc` beyond JS-level smoke behavior

## Documentation Alignment Notes

This file previously described an older state (15 tests and partial coverage). It now reflects the current enforced gate and run output.

## Run Commands

```bash
npm test
npm run test:coverage
```
