# DOC Platform

DOC is a canonical block-document system for human and AI collaboration.

It keeps rendering and authoring separate. Humans edit visual blocks in the browser instead of hand-writing markup syntax. AI systems get one deterministic DOCSRC serialization instead of ambiguous Markdown variants.

Canonical document content lives in compressed bundle archives. On-disk `.dx` files are stubs/pointers; the bundle archives hold the packed payloads for transport and rebuild.

## Storage model

- On-disk `.dx` files are minimal stubs, for example:

```text
@docstub 3
path: research/grill-with-docs.dx
```

- Canonical content is packed DOC-binary blocks inside bundle archives:
  - `.doc/.repo-docs.bin` — repository-tracked docs (commit this; it rebuilds state in a fresh clone/shared repo).
  - `.doc/.local-docs.bin` — local-only docs (gitignored).
- Which archive a doc belongs to is driven by its git tracking state.
- Existing non-stub `.dx` files are migrated into the bundle during ingest, then replaced by stubs + archive entries.
- Search runs on a zero-dependency custom token index: `dxlite` sidecars (`*.dxlite.bin`).
- View state (theme, appearance, edit buffer) is persisted in `.doc/view-state.json`.

## Quick start

1. **Run guided setup:** `npm run setup` to ingest docs and print behavior-focused editor tips.
2. **(Optional) Seed example docs:** `npm run docs:seed` to rewrite baseline welcome/tutorial/reference docs.
3. **Ingest workspace (repeat when needed):** `npm run ingest` to migrate/reindex all `.dx` files into the bundle + dxlite index.
4. **Compile TypeScript runtime artifacts:** `npm run build:ts`.
5. **Run strict TypeScript diagnostics for the stabilized surface:** `npm run typecheck`.
6. **Run full migration diagnostics across all TypeScript files:** `npm run typecheck:full`.
7. **Run MCP server (fast start):** `npm run mcp` to start the MCP server immediately from built runtime.
8. **Edit in VS Code:** Open `vscode-extension/` and press `F5` to launch the extension with virtual `docdb:/` filesystem.
9. **Reconstruct:** `npm run reconstruct -- <path.dx>` to emit DOCSRC source from the bundle.
10. **Re-clean generated outputs (when needed):** `npm run clean`.

## Project layout

- `src/` core runtime services, storage, parser, and MCP server logic.
- `vscode-extension/` extension host and webview source.
- `test/unit/` focused unit tests.
- `test/integration/` cross-module workflow and integration tests.
- `test/coverage/` edge and threshold-focused coverage tests.
- `test/helpers/` shared test utilities.
- `test/types/` ambient type declarations used by TypeScript tests.
- `build/runtime/` compiled runtime JavaScript.
- `build/test/` compiled test JavaScript.
- `build/coverage/` generated coverage reports and c8 temp output.

This structure keeps authored sources in `src/`, test sources in `test/`, and all generated artifacts under `build/`.

The MCP server is the standard interface for AI agents and tools to query and manipulate documents. The VS Code extension reads/writes the same bundle archives directly — no HTTP server required.

## Tutorial and setup behaviors

- Press `/` to open the control panel quickly.
- Use the pen button to switch between **Editing** and **Read only** mode.
- Use `?` to open the in-editor setup + format tutorial.
- Use `Option/Alt + click` on `id=` or `class=` attributes while source-editing to open scoped CSS editing.
- Share `.dx` files plus `.doc/.repo-docs.bin` in git to keep portable, rebuildable docs for other users.

## VS Code integration

The workspace includes a local extension at `vscode-extension/` that provides a virtual filesystem:

- `docdb:/` is a virtual filesystem provider backed by the bundle archives.
- Virtual docs appear like normal files in Explorer once mounted.
- Opening a `.dx` stub uses the custom DOC DB editor and loads full content from the bundle.
- The extension reads/writes the bundle archives directly (no HTTP backend).

To run the extension locally:

1. Open `vscode-extension/` in VS Code.
2. Press `F5` to launch Extension Development Host.
3. Run `DOC DB: Mount Virtual Files` if it does not auto-mount.

## MCP Server

The project exposes document operations via a Model Context Protocol (MCP) server. This is the standard interface for AI tools, agents, and LLMs to interact with the knowledge base:

```bash
npm run mcp
```

`npm run mcp` intentionally does not rebuild runtime artifacts. The server starts quickly from `build/runtime/`. Use `npm run mcp:prepare` (alias for `npm run build:ts`) when you want to refresh runtime artifacts first.

**Available Tools:**
- `list-documents` — List all documents with optional search query
- `get-document` — Retrieve a specific document by path or ID
- `search-documents` — Full-text search across all documents
- `create-document` — Create a new document
- `save-document` — Update an existing document
- `open-document-viewer` — Open a stateful, built-in document viewer session
- `interact-document-viewer` — Interact with a viewer session (inspect/click/scroll/edit/save)
- `ingest-workspace` — Index documents from a workspace directory

**Available Resources:**
- `doc:///path/to/document.dx` — Raw document source
- `docview:///path/to/document.dx` — Built-in rendered document view (HTML)

The MCP server reads/writes the same bundle archives as the VS Code extension, ensuring consistency.

## Canonical DOCSRC shape

```text
@doc 3
title: Architecture Notes
summary: What this document covers.
tags: architecture, docs
meta.owner: alex
---
::heading level=1 id=architecture-notes
Architecture Notes
::end

::paragraph id=paragraph-2
This document is edited visually, not with Markdown syntax.
::end
```

## Block syntax reference

- `::paragraph` text `::end` or plain text without a block wrapper
- `::heading level=1..4` text `::end`
- `::bulleted-list` newline-separated items `::end`
- `::numbered-list` newline-separated items `::end`
- `::checklist` items as `[x] done` or `[ ] pending` `::end`
- `::quote` text `::end`
- `::code` text `::end` (optional `lang=` or `language=` attribute)
- `::image src=...` alt text body `::end`
- `::style` CSS declarations `::end` (applies style, not semantic content)
- `::stylesheet href=...` `::end` (external stylesheet link, not semantic content)

For list blocks, each newline is one item. Leading `-`, `*`, or `1.` prefixes are optional and normalized away.

Style and stylesheet blocks affect rendering only. They are intentionally excluded from section text extraction/search context so AI text workflows focus on document meaning rather than presentation rules.

## DX Contract and Review Checklist

- DX format and safety contract: `docs/dx-format-contract.md`
- Parser/render change grilling checklist: `docs/grill-me.md`

These docs define the non-negotiable behavior for parsing, canonicalization, and CSS safety.

## Why this matches the video better

The reparsed transcript makes the core complaint clear: Markdown is attractive because it renders well, but it has too many overlapping syntaxes, too much inline escape hatch behavior, and too much grammar pollution in the source text itself. This implementation fixes that by:

- using one file grammar
- moving humans onto a visual block editor
- keeping AI-facing storage deterministic
- indexing semantic sections into a custom token index instead of parsing ad hoc markup every time

## Architecture

- `src/doc-format.ts` handles DOCSRC parsing, legacy migration, block normalization, and reconstruction.
- `src/doc-binary.ts` packs normalized documents into compact binary blocks.
- `src/doc-archive.ts` reads/writes the brotli-compressed bundle archives (repo + local).
- `src/dxlite.ts` builds and queries the custom token search index over bundle contents.
- `src/git-doc-state.ts` resolves git tracking state to route docs between repo and local archives.
- `src/doc-service.ts` orchestrates bundle-first storage and writes tiny link stubs to disk.
- `src/mcp-server.ts` defines stdio MCP tools/resources for document read/write/search/view workflows.
- `src/doc-view-capture.ts` captures rendered document surfaces as PNG via Playwright (Quick Look fallback on macOS).
- Runtime `.js` files are emitted from TypeScript into `build/runtime/` for Node/webview execution compatibility.
- `vscode-extension/` provides the `docdb:/` virtual filesystem and `.dx` stub custom editor.

## Limits

- This extension is local to this repo and is not packaged/published yet.
- Delete and rename operations for virtual docs are not implemented in the extension yet.