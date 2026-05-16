# DOC Platform

DOC is a canonical block-document system for human and AI collaboration.

It keeps rendering and authoring separate. Humans edit visual blocks in the browser instead of hand-writing markup syntax. AI systems get one deterministic DOCSRC serialization instead of ambiguous Markdown variants.

SQLite is the source of truth for document content. The runtime now prefers a native C++ SQLite bridge (`native/sqlite_bridge.cc`) for predictable performance and clean typed bindings.

On-disk `.dx` files are stubs and one hidden repository artifact stores all compressed document payloads for transport.

## Storage model

- Full document source and indexes live in `data/doc-index.sqlite`.
- On-disk `.dx` files are minimal stubs, for example:

```text
@docstub 1
path: research/grill-with-docs.dx
```

- A single hidden artifact at `.doc/.repo-docs.bin` stores ultra-compressed payloads for all docs. This artifact can rebuild DB state in a fresh clone/shared repo.
- Existing non-stub `.dx` files are migrated into SQLite during ingest, then replaced by stubs + shared archive entries.
- Search continues to run on a zero-dependency custom SQLite token index.

## Quick start

1. **Run guided setup:** `npm run setup` to ingest docs and print behavior-focused editor tips.
2. **Ingest workspace (repeat when needed):** `npm run ingest` to migrate/reindex all `.dx` files into SQLite.
3. **Run MCP server:** `npm run mcp` to start the MCP server (exposes document operations to AI tools).
4. **Edit in VS Code:** Open `vscode-extension/` and press `F5` to launch the extension with virtual `docdb:/` filesystem.
5. **Reconstruct:** `npm run reconstruct -- <document-id>` to emit SQLite-backed DOCSRC source.

The MCP server is the standard interface for AI agents and tools to query and manipulate documents. The VS Code extension connects directly to the local SQLite database via the native C++ bridge — no HTTP server required.

## Tutorial and setup behaviors

- Press `/` to open the control panel quickly.
- Use the pen button to switch between **Editing** and **Read only** mode.
- Use `?` to open the in-editor setup + format tutorial.
- Use `Option/Alt + click` on `id=` or `class=` attributes while source-editing to open scoped CSS editing.
- Share `.dx` files plus `.doc/.repo-docs.bin` in git to keep portable, rebuildable docs for other users.

## VS Code integration

The workspace includes a local extension at `vscode-extension/` that provides a virtual filesystem:

- `docdb:/` is a virtual filesystem provider backed by SQLite.
- Virtual docs appear like normal files in Explorer once mounted.
- Opening a `.dx` stub uses the custom DOC DB editor and loads full content from SQLite.
- The extension connects directly to the database via the native C++ bridge (no HTTP backend).

To run the extension locally:

1. Open `vscode-extension/` in VS Code.
2. Press `F5` to launch Extension Development Host.
3. Run `DOC DB: Mount Virtual Files` if it does not auto-mount.

## MCP Server

The project exposes document operations via a Model Context Protocol (MCP) server. This is the standard interface for AI tools, agents, and LLMs to interact with the knowledge base:

```bash
npm run mcp
```

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

The MCP server reads/writes from the same SQLite database as the VS Code extension, ensuring consistency.

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

For list blocks, each newline is one item. Leading `-`, `*`, or `1.` prefixes are optional and normalized away.

## DX Contract and Review Checklist

- DX format and safety contract: `docs/dx-format-contract.md`
- Parser/render change grilling checklist: `docs/grill-me.md`

These docs define the non-negotiable behavior for parsing, canonicalization, and CSS safety.

## Why this matches the video better

The reparsed transcript makes the core complaint clear: Markdown is attractive because it renders well, but it has too many overlapping syntaxes, too much inline escape hatch behavior, and too much grammar pollution in the source text itself. This implementation fixes that by:

- using one file grammar
- moving humans onto a visual block editor
- keeping AI-facing storage deterministic
- indexing semantic sections into SQLite instead of parsing ad hoc markup every time

## Architecture

- `src/doc-format.js` handles DOCSRC parsing, legacy migration, block normalization, and reconstruction.
- `src/doc-binary.js` packs normalized documents into compact SQLite blobs.
- `src/database.js` stores source, compact storage blobs, and searchable semantic sections.
- `src/doc-service.js` enforces SQLite-first storage and writes tiny link stubs to disk.
- `src/server.js` serves editor APIs and virtual file APIs (`/api/virtual-docs`).
- `vscode-extension/` provides the `docdb:/` virtual filesystem and `.dx` stub custom editor.
- `public/` contains the browser block editor and paged reader.

## Limits

- This extension is local to this repo and is not packaged/published yet.
- Delete and rename operations for virtual docs are not implemented in the extension yet.