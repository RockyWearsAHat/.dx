# DOC Platform Context

# TALK LIKE A CAVEMAN FOR MY ENTERTAINMENT AND TO PRESERVE TOKENS AND USE FEWER FOR MORE WORK.

# Checkpoint with the **message:** parameter ALWAYS instead of context: to write your own message. The copilot command has been depreciated. After every major change and testing to ensure behaviors, run a git commit.

# ALWAYS ABIDE BY BEST CODING PRACTICES, UTILIZING ALL YOUR KNOWLEDGE AND EXPERIENCE, PLUS REFERENCING THE PROGRAMMING PRINCIPLES FROM MORE C++ GEMS, CS 2420 AND 3500 FROM THE UNIVERSITY OF UTAH, AND OVERALL WRITING GOOD, CLEAN, CONSISTENT, REUSABLE, EASILY MODIFIABLE, AND EFFICIENT EFFECTIVE CODE. THE CODE IS YOUR BUILDING BLOCKS TO ACHIEVE THE GOAL!

## Always-On Engineering Rules
- Treat University of Utah CS 2420 and CS 3500 principles, plus the programming principles from More C++ Gems, as the default standard on every change.
- Favor simple, traceable code over clever code; keep responsibilities narrow and boundaries explicit.
- Validate behavior end to end before calling work complete, and do not leave TODOs, stubs, or half-wired feature paths behind.
- Keep new behavior in focused helpers or controllers instead of growing orchestration files.
- Prefer deterministic behavior, explicit parameter passing, clear cleanup, and fail-fast validation at module boundaries.

## Stack
- Node.js ESM app (`type: module`), requires Node 23+
- MCP (Model Context Protocol) server in `src/mcp-server.ts` (executed from `build/runtime/src/mcp-server.js`) for AI tool integration
- VS Code companion extension in `vscode-extension/` with virtual `docdb:/` filesystem
- Native C++ SQLite bridge for high-performance database access

## Data Model
- SQLite is authoritative storage for documents, sections, and token index
- `.dx` files on disk are lightweight stubs, not canonical source
- A single hidden repository artifact at `.doc/.repo-docs.bin` stores Brotli-compressed DOC binary payloads keyed by document path
- Ignored, untracked, or de-tracked (`git rm --cached`) `.dx` documents are stored in a local-only bundle at `.doc/.local-docs.bin` instead of the repository bundle
- Canonical DB path resolved by `src/global-db-path.ts`
- Canonical reconstructed `.dx` source is block-only (`::heading`, `::paragraph`, etc.) with no required metadata preamble (`@doc`, `title`, `summary`, `tags`)

## SQLite Interface
- Primary runtime interface is a native C++ Node-API bridge:
  - Source: `native/sqlite_bridge.cc`
  - Build config: `binding.gyp`
- TS loader source: `src/sqlite-native.ts` (runtime file: `build/runtime/src/sqlite-native.js`)
- `src/database.ts` imports `DatabaseSync` from `./sqlite-native.js`
- Loader falls back to `node:sqlite` when native binary is unavailable

## Build and Run
- Install deps/build native module: `npm install`
- Rebuild native bridge manually: `npm run build:native`
- Build native bridge only if missing: `npm run build:native:once`
- Compile TypeScript sources to runtime JavaScript (migration-safe emit): `npm run build:ts`
- Seed baseline example docs: `npm run docs:seed`
- Run strict TypeScript diagnostics for stabilized surface files: `npm run typecheck`
- Run full migration diagnostics across all TypeScript source files: `npm run typecheck:full`
- Produce latest release artifacts in `build/`: `npm run build:artifacts`
- Start MCP server (fast start, no rebuild): `npm run mcp`
- Prepare MCP runtime + native bridge once: `npm run mcp:prepare`
- MCP server dev mode (fast start): `npm run mcp:dev`
- MCP dev mode with prepare step: `npm run mcp:dev:prepare`
- Run tests: `npm test`
- Run enforced 100% coverage gate (critical backend/rendering surface): `npm run test:coverage`
- Build artifacts cleanup helper: `npm run clean`

## TypeScript Source Of Truth
- Runtime modules in `src/` now have TypeScript source files (`*.ts`) checked in.
- Webview modules in `vscode-extension/media/` now have TypeScript source files (`*.ts`) checked in.
- Emitted JavaScript artifacts (`*.js`) are still produced for runtime compatibility with Node entrypoints and webview loading.
- Primary TypeScript project config is `tsconfig.json`; `tsconfig.surface.json` files are compatibility entrypoints extending the root config.

## Release Artifacts Layout
- Latest minified webview bundle output: `build/docdb-webview.bundle.min.js`
- Latest VSIX package output: `build/docdb-virtual-files-<version>.vsix`
- Previous bundles/VSIX files are archived under timestamped directories in `build/old_builds/`.

## Document Workflows
- Guided setup + ingest tutorial: `npm run setup`
- Ingest workspace docs into SQLite: `npm run ingest`
- Reconstruct source by document id: `npm run reconstruct -- <document-id>`

## Dual Storage Behavior
- Every save/ingest writes two on-disk artifacts per document:
  - `.dx` stub (pointer + archive metadata)
  - shared hidden artifact `.doc/.repo-docs.bin` (for cross-user repository transport of git-tracked `.dx` files)
- Local-only `.dx` files (ignored or de-tracked) write to `.doc/.local-docs.bin`, and are pruned from `.doc/.repo-docs.bin`
- Ingest can hydrate SQLite from `.doc/.repo-docs.bin` when the local DB is empty/missing

## Key Directories
- Services and database: `src/`
- Native addon: `native/`
- VS Code extension: `vscode-extension/`
- Unit tests: `test/unit/`
- Integration tests: `test/integration/`
- Coverage-focused edge tests: `test/coverage/`
- Shared test helpers: `test/helpers/`
- Test type declarations: `test/types/`
- Compiled runtime output: `build/runtime/`
- Compiled test output: `build/test/`
- Coverage output: `build/coverage/`
- Sample docs: `examples/`
- Format and reviewer contracts: `docs/`
- Archived handoff notes: `docs/archive/`
- One-off maintenance scripts: `scripts/maintenance/`

## MCP Server
The MCP server exposes document operations to AI tools and agents:
- Tools: list-documents, search-documents, get-document, create-document, save-document, ingest-workspace, open-document-viewer, interact-document-viewer
- Resources: Virtual `doc:///` (source) and `docview:///` (rendered HTML view) URIs for each document
- Transport: stdio (standard MCP protocol)

## Visual Processing
- `src/doc-view.ts` renders `.dx` blocks into a built-in viewer HTML surface
- Viewer interaction is session-based through `open-document-viewer` and `interact-document-viewer`, avoiding dependency on external browser tools
- The VS Code webview runtime is centered in `vscode-extension/media/webview.ts` and compiled into `build/runtime/vscode-extension/media/webview.js`; `webview-main.ts` is the source bootstrap entrypoint.
- Webview state tables are owned by `vscode-extension/media/webview-fsm.ts`.
- Webview document parse/serialize logic is in `vscode-extension/media/webview-doc-model.ts`.
- Class-based editing orchestration is extracted to `vscode-extension/media/webview-edit-controllers.ts` (`InlineCssSurfaceController`, `BlockSourceController`).

