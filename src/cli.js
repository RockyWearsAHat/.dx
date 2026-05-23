import path from 'node:path';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { createDatabase, listDocuments, migrateLegacyWorkspace } from './database.js';
import { ingestWorkspace, reconstructDocument } from './doc-service.js';
import { resolveDocDbPath } from './global-db-path.js';
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');
const dbPath = resolveDocDbPath();
const legacyDbPath = path.join(rootDir, 'data', 'doc-index.sqlite');
await mkdir(path.dirname(dbPath), { recursive: true });
const db = createDatabase(dbPath);
migrateLegacyWorkspace(db, rootDir, legacyDbPath);
const command = process.argv[2];
if (command === 'setup') {
    const documents = await ingestWorkspace(rootDir, db);
    console.log('DOC setup complete.');
    console.log(`Indexed ${documents.length} document(s).`);
    console.log('Behavior tutorial:');
    console.log('1. Keep .dx files in the repo; stubs now point into one hidden shared archive at .doc/.repo-docs.bin.');
    console.log('2. SQLite remains the live index for fast search and editing.');
    console.log('3. The VS Code editor has two modes: Edit and Read. Toggle with the pen button.');
    console.log('4. Open controls with / and format help with ?.');
    console.log('5. Use Option/Alt+Click on id/class attributes in source mode to edit scoped CSS quickly.');
    process.exit(0);
}
if (command === 'ingest') {
    const documents = await ingestWorkspace(rootDir, db);
    console.log(`Indexed ${documents.length} document(s).`);
    for (const document of documents) {
        console.log(`- ${document.relativePath}`);
    }
    process.exit(0);
}
if (command === 'reconstruct') {
    const identifier = Number(process.argv[3]);
    if (!identifier) {
        console.error('Usage: npm run reconstruct -- <document-id>');
        process.exit(1);
    }
    console.log(await reconstructDocument(rootDir, db, identifier));
    process.exit(0);
}
console.log('Available commands:');
console.log('- npm run setup');
console.log('- npm run ingest');
console.log('- npm run reconstruct -- <document-id>');
console.log(`Indexed documents currently available: ${listDocuments(db, rootDir).length}`);
