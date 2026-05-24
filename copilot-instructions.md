# DOC Platform Context

## Stack
- Node.js ESM app (`type: module`), Node 23+
- MCP server in [src/mcp-server.ts](src/mcp-server.ts)
- DOC service engine in [src/doc-service.ts](src/doc-service.ts)
- Bundle/archive engine in [src/doc-archive.ts](src/doc-archive.ts)
- Dxlite sidecar search index in [src/dxlite.ts](src/dxlite.ts)
- VS Code extension in [vscode-extension/extension.ts](vscode-extension/extension.ts)

## Storage Model
- SQLite is removed from active runtime paths.
- `.dx` files on disk are stubs/pointers.
- Canonical content is packed DOC binary blocks in bundle artifacts.
- Repository-tracked docs live in `.doc/.repo-docs.bin`.
- Local-only docs live in `.doc/.local-docs.bin`.
- Dxlite sidecars (`*.dxlite.bin`) provide fast token->doc lookup.
- View state is persisted in `.doc/view-state.json`.

## Build and Test
- Build runtime: `npm run build:ts`
- Build tests: `npm run build:test`
- Run tests: `npm test`
- Coverage gate: `npm run test:coverage`
- MCP start: `npm run mcp`
- MCP dev: `npm run mcp:dev`

## CLI Workflows
- Setup/ingest docs: `npm run setup`, `npm run ingest`
- Reconstruct by relative path: `npm run reconstruct -- <path.dx>`
- Maintain command is bundle-engine informational only: `npm run maintain`

## MCP Tooling
- Tools include list/search/get/create/save/ingest and viewer session controls.
- Resources expose `doc:///` (source) and `docview:///` (rendered HTML).
- Runtime is bundle+dxlite only; no DB maintenance operations.

## Key Directories
- Runtime source: [src](src)
- Extension host/webview: [vscode-extension](vscode-extension)
- Tests: [test](test)
- Build outputs: [build](build)
- Scripts: [scripts](scripts)

## Engineering Rules
- Keep behavior deterministic and explicit.
- Prefer narrow helpers over monolithic orchestration.
- Validate end-to-end behavior before declaring complete.
- Keep compatibility arguments only when needed for API stability.

