# .doc/ — Repository Document Archive

This folder contains the ultra-compressed document artifact for the DOC editor system.

## Contents

- **`.repo-docs.bin`** — Single Brotli-compressed container with all .dx document payloads, keyed by document path. This is the portable artifact that makes documents transportable across repositories and users while maintaining SQLite as the authoritative index.

## For Best Experience

Install the **docdb VS Code extension** to:
- Automatically hide this folder in the Explorer sidebar
- Open `.dx` stub files with the rich document editor
- Enable inline CSS editing, scoped to individual blocks
- See real-time rendering and block-level editing

The extension will automatically hide this folder by updating `.vscode/settings.json` with `files.exclude` rules.

Without the extension, the folder remains visible but is not required for functionality—stub files still resolve their content from the archive. However, the custom editor and visual experience are significantly enhanced with the extension installed.

## Storage Format

Documents are stored in a binary container format:
- Each document is compressed individually with Brotli
- The container is indexed by document path (e.g., `examples/welcome.dx`)
- SHA256 integrity check on read to ensure data consistency
- Container is transparent to the stub editor—all reads/writes are automatic

## Development

To manually inspect the archive:
```bash
node -e "
  const archive = require('./src/doc-archive.js');
  const fs = require('fs');
  const data = fs.readFileSync('.doc/.repo-docs.bin');
  const docs = archive.decodeArchiveContainer(data);
  console.log(Object.keys(docs));
"
```

This folder is part of version control and should be committed to ensure documents are available to all users of the repository.
