# DOC Platform Context

## Stack
- Node.js ESM app (`type: module`), requires Node 23+
- MCP (Model Context Protocol) server in `src/mcp-server.js` for AI tool integration
- VS Code companion extension in `vscode-extension/` with virtual `docdb:/` filesystem
- Native C++ SQLite bridge for high-performance database access

## Data Model
- SQLite is authoritative storage for documents, sections, and token index
- `.dx` files on disk are lightweight stubs, not canonical source
- A single hidden repository artifact at `.doc/.repo-docs.bin` stores Brotli-compressed DOC binary payloads keyed by document path
- Canonical DB path resolved by `src/global-db-path.js`

## SQLite Interface
- Primary runtime interface is a native C++ Node-API bridge:
  - Source: `native/sqlite_bridge.cc`
  - Build config: `binding.gyp`
  - JS loader: `src/sqlite-native.js`
- `src/database.js` imports `DatabaseSync` from `src/sqlite-native.js`
- Loader falls back to `node:sqlite` when native binary is unavailable

## Build and Run
- Install deps/build native module: `npm install`
- Rebuild native bridge manually: `npm run build:native`
- Start MCP server: `npm run mcp`
- MCP server dev mode: `npm run mcp:dev`

## Document Workflows
- Guided setup + ingest tutorial: `npm run setup`
- Ingest workspace docs into SQLite: `npm run ingest`
- Reconstruct source by document id: `npm run reconstruct -- <document-id>`

## Dual Storage Behavior
- Every save/ingest writes two on-disk artifacts per document:
  - `.dx` stub (pointer + archive metadata)
  - shared hidden artifact `.doc/.repo-docs.bin` (for cross-user repository transport)
- Ingest can hydrate SQLite from `.doc/.repo-docs.bin` when the local DB is empty/missing

## Key Directories
- Services and database: `src/`
- Native addon: `native/`
- VS Code extension: `vscode-extension/`
- Sample docs: `documents/`, `examples/`, `research/`

## MCP Server
The MCP server exposes document operations to AI tools and agents:
- Tools: list-documents, search-documents, get-document, create-document, save-document, ingest-workspace, get-document-visual
- Resources: Virtual `doc:///` URIs for each document
- Transport: stdio (standard MCP protocol)

## Visual Processing
- `src/doc-visual.js` converts document blocks into a DX-native visual hierarchy and surface model for AI reasoning
- `get-document-visual` returns structured visual/design data directly from `.dx` content (no external webview/browser preview output)

