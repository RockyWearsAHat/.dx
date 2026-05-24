import test from 'node:test';
import assert from 'node:assert/strict';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from '../helpers/test-utils.js';
import { ingestWorkspace, listOrSearchDocuments } from '#runtime-src/doc-service.js';

// Service-layer behavior behind MCP tools (no MCP transport/process wiring in this file).
test('service-layer operations backing MCP tools handle common workflows', async () => {
  const { rootDir } = await createTempWorkspace('doc-mcp-unit-');

  try {
    // Exercise service functions that MCP tools delegate to.
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

    // Simulate ingest-workspace tool
    const ingested = await ingestWorkspace(rootDir, null);
    assert.equal(ingested.length, 1);
    assert.equal(ingested[0].relativePath, 'examples/welcome.dx');
    assert.equal(ingested[0].title, 'welcome');

    // Simulate list-documents tool
    const listed = await listOrSearchDocuments(rootDir, null, '');
    assert.equal(listed.length, 1);

    // Simulate search-documents tool
    const searched = await listOrSearchDocuments(rootDir, null, 'interactive');
    assert.equal(searched.length, 1);
    assert.equal(searched[0].relativePath, 'examples/welcome.dx');
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});
