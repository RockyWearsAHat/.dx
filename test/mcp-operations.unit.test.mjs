import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from './test-utils.mjs';
import { createDatabase } from '../src/database.js';
import { ingestWorkspace, listOrSearchDocuments } from '../src/doc-service.js';

// Unit-level MCP message handling tests (no subprocess spawning)
test('mcp document operations are callable and handle common workflows', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-mcp-unit-');

  try {
    // Simulate MCP tool environment by calling the service layer directly
    await writeDxFile(
      rootDir,
      'examples/welcome.dx',
      [
        '::heading level=1 id=welcome',
        'Welcome to interactive paper',
        '::end',
        '',
        '::paragraph id=body',
        'This is an interactive sheet.',
        '::end',
      ].join('\n')
    );

    const db = createDatabase(dbPath);

    // Simulate ingest-workspace tool
    const ingested = await ingestWorkspace(rootDir, db);
    assert.equal(ingested.length, 1);
    assert.equal(ingested[0].relativePath, 'examples/welcome.dx');
    assert.equal(ingested[0].title, 'welcome');

    // Simulate list-documents tool
    const listed = await listOrSearchDocuments(rootDir, db, '');
    assert.equal(listed.length, 1);

    // Simulate search-documents tool
    const searched = await listOrSearchDocuments(rootDir, db, 'interactive');
    assert.equal(searched.length, 1);
    assert.ok(searched[0].score > 0);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});
