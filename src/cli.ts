import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { getDocumentByRelativePath, ingestWorkspace, listOrSearchDocuments, reconstructDocument } from './doc-service.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');
const runtime = null;

const command = process.argv[2];

if (command === 'setup') {
  const documents = await ingestWorkspace(rootDir, runtime);
  console.log('DOC setup complete.');
  console.log(`Indexed ${documents.length} document(s).`);
  console.log('Behavior tutorial:');
  console.log('1. Keep .dx files in the repo; stubs now point into one hidden shared archive at .doc/.repo-docs.bin.');
  console.log('2. Bundle + dxlite are the live engine for search and editing.');
  console.log('3. The VS Code editor has two modes: Edit and Read. Toggle with the pen button.');
  console.log('4. Open controls with / and format help with ?.');
  console.log('5. Use Option/Alt+Click on id/class attributes in source mode to edit scoped CSS quickly.');
  process.exit(0);
}

if (command === 'ingest') {
  const documents = await ingestWorkspace(rootDir, runtime);
  console.log(`Indexed ${documents.length} document(s).`);
  for (const document of documents) {
    console.log(`- ${document.relativePath}`);
  }
  process.exit(0);
}

if (command === 'reconstruct') {
  const target = String(process.argv[3] || '').trim();

  if (!target || !target.endsWith('.dx')) {
    console.error('Usage: npm run reconstruct -- <workspace-relative-path.dx>');
    process.exit(1);
  }

  const doc = await getDocumentByRelativePath(rootDir, runtime, target);
  if (!doc) {
    console.error(`Document not found: ${target}`);
    process.exit(1);
  }

  console.log(await reconstructDocument(rootDir, runtime, doc.id));
  process.exit(0);
}

if (command === 'maintain') {
  console.log('Maintenance complete. Active engine is bundle + dxlite; no SQLite vacuum/checkpoint required.');
  process.exit(0);
}

console.log('Available commands:');
console.log('- npm run setup');
console.log('- npm run ingest');
console.log('- npm run reconstruct -- <workspace-relative-path.dx>');
console.log('- npm run maintain');
console.log(`Indexed documents currently available: ${(await listOrSearchDocuments(rootDir, runtime, '')).length}`);