import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { readFile } from 'node:fs/promises';

import {
  createDatabase,
  getDocumentByPath,
  getDocumentSectionBySlug,
  getDocumentSections,
  listDocuments,
  searchDocuments,
  upsertDocument,
} from '../src/database.js';
import { normalizeDocInput, parseDocFile } from '../src/doc-format.js';
import {
  createDocument,
  ingestWorkspace,
  saveDocument,
} from '../src/doc-service.js';
import { readDocumentViewState } from '../src/view-state.js';
import { packDocument, unpackDocument } from '../src/doc-binary.js';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from './test-utils.mjs';

// ─── view-state.js ────────────────────────────────────────────────────────────

test('readDocumentViewState returns null when db is null', () => {
  assert.equal(readDocumentViewState(null, 1), null);
});

test('readDocumentViewState returns null when documentId is NaN', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('vs-nan-');
  try {
    const db = createDatabase(dbPath);
    assert.equal(readDocumentViewState(db, NaN), null);
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('readDocumentViewState returns null when view_state_json is a non-object', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('vs-noobj-');
  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/x.dx`, { title: 'x', blocks: [] });
    const id = upsertDocument(db, rootDir, doc, Date.now());

    // Store a JSON string (not an object) — e.g. the number 42
    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run('"not-an-object"', id);
    assert.equal(readDocumentViewState(db, id), null);

    // Store null (typeof null === 'object' but !null === true)
    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run('null', id);
    assert.equal(readDocumentViewState(db, id), null);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

// ─── database.js ─────────────────────────────────────────────────────────────

test('getDocumentByPath returns null when workspace does not exist', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('db-noworkspace-');
  try {
    const db = createDatabase(dbPath);
    const result = getDocumentByPath(db, '/nonexistent/root', '/nonexistent/root/doc.dx');
    assert.equal(result, null);
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('searchDocuments with empty query returns all docs with score 0', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('db-search-empty-');
  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/a.dx`, { title: 'Alpha', blocks: [{ type: 'paragraph', text: 'content' }] });
    upsertDocument(db, rootDir, doc, Date.now());

    const results = searchDocuments(db, rootDir, '');
    assert.ok(results.length >= 1);
    assert.equal(results[0].score, 0);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('getDocumentSectionBySlug returns null when slug not found', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('db-slug-');
  try {
    const db = createDatabase(dbPath);
    const result = getDocumentSectionBySlug(db, 999999, 'no-such-slug');
    assert.equal(result, null);
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('listDocuments returns empty array when workspace does not exist', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('db-list-empty-');
  try {
    const db = createDatabase(dbPath);
    const results = listDocuments(db, '/no/such/workspace');
    assert.deepEqual(results, []);
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

// ─── doc-service.js ───────────────────────────────────────────────────────────

test('createDocument rejects path without .dx extension', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('svc-badpath-');
  try {
    const db = createDatabase(dbPath);
    await assert.rejects(
      () => createDocument(rootDir, db, { path: 'notes/readme.txt' }),
      /valid .dx path/i
    );
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('createDocument rejects path that escapes workspace root', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('svc-escape-');
  try {
    const db = createDatabase(dbPath);
    await assert.rejects(
      () => createDocument(rootDir, db, { path: '../outside.dx' }),
      /workspace root/i
    );
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('saveDocument rejects unknown document id', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('svc-notfound-');
  try {
    const db = createDatabase(dbPath);
    await assert.rejects(
      () => saveDocument(rootDir, db, 999999, { title: 'Ghost' }),
      /Document not found/
    );
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('ingestWorkspace handles stub files on second pass (existing in DB)', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('svc-stub-');
  try {
    const source = [
      '::heading level=1 id=h1',
      'Stub Test',
      '::end',
      '',
      '::paragraph id=p1',
      'Test content for stub ingestion.',
      '::end',
    ].join('\n');
    await writeDxFile(rootDir, 'docs/test.dx', source);

    const db = createDatabase(dbPath);

    // First ingest: .dx → creates archive + stub
    const first = await ingestWorkspace(rootDir, db);
    assert.equal(first.length, 1);

    // Verify stub was written
    const stubText = await readFile(path.join(rootDir, 'docs/test.dx'), 'utf8');
    assert.ok(stubText.startsWith('@docstub'), 'Expected stub file after first ingest');

    // Second ingest: stub file → if (stub) && if (existing) path
    const second = await ingestWorkspace(rootDir, db);
    assert.equal(second.length, 1);
    // Title is derived from filename (canonical format has no embedded title field)
    assert.ok(second[0].id > 0);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('ingestWorkspace handles stub with archive but no DB entry', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('svc-stub-noentry-');
  try {
    const source = [
      '::heading level=1 id=h2',
      'Archive Stub',
      '::end',
      '',
      '::paragraph id=p2',
      'Archive reconstruction test.',
      '::end',
    ].join('\n');
    await writeDxFile(rootDir, 'docs/archive.dx', source);

    const db = createDatabase(dbPath);

    // First ingest: creates archive + stub + DB entry
    await ingestWorkspace(rootDir, db);

    // Wipe DB so the document is gone but archive still exists on disk
    db.exec('DELETE FROM tokens');
    db.exec('DELETE FROM sections');
    db.exec('DELETE FROM workspace_documents');
    db.exec('DELETE FROM documents');
    db.exec('DELETE FROM workspaces');

    // Third ingest: stub exists, archive exists, but no DB entry → reconstructs from archive
    const reconstructed = await ingestWorkspace(rootDir, db);
    assert.equal(reconstructed.length, 1);
    assert.ok(reconstructed[0].id > 0);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

// ─── doc-binary.js ────────────────────────────────────────────────────────────

test('doc-binary decodeMeta falls back to raw string on invalid JSON value', () => {
  // Build a document with a string meta value, then corrupt the JSON encoding
  // Encoding: meta value 'hello' is stored as JSON.stringify('hello') = '"hello"' (7 bytes)
  // We change it to 'hello' (5 bytes, not valid JSON) → catch branch fires
  const packed = packDocument({
    title: '',
    summary: '',
    tags: [],
    meta: { k: 'hello' },
    blocks: [],
  });

  // The packed buffer layout for this specific input:
  // [0..4] = MAGIC 'DOCB1'
  // [5]    = varint version (1)
  // [6]    = empty title (varint 0)
  // [7]    = empty summary (varint 0)
  // [8]    = empty tags count (varint 0)
  // [9]    = meta count (varint 1)
  // [10]   = key length (varint 1)
  // [11]   = 'k' (107)
  // [12]   = value length (varint 7)
  // [13..19] = '"hello"' ([34,104,101,108,108,111,34])
  // [20]   = block count (varint 0)

  // Replace '"hello"' (8 bytes at offset 12) with 'hello' (6 bytes at offset 12)
  const corrupt = Buffer.concat([
    packed.subarray(0, 12),
    Buffer.from([5, 104, 101, 108, 108, 111]),
    packed.subarray(20),
  ]);

  const result = unpackDocument(corrupt);
  assert.equal(result.meta.k, 'hello');
});

test('doc-binary decodeBlock throws on truncated buffer (no block data)', () => {
  const packed = packDocument({ title: '', summary: '', tags: [], meta: {}, blocks: [] });
  // Change the block count from 0 to 1, creating a claim of 1 block with no data
  const corrupt = Buffer.from(packed);
  corrupt[corrupt.length - 1] = 1;
  assert.throws(() => unpackDocument(corrupt), /Unexpected end of binary document/);
});

test('doc-binary decodeBlock throws on truncated heading (missing level byte)', () => {
  const packed = packDocument({ title: '', summary: '', tags: [], meta: {}, blocks: [] });
  // Append: blockCount=1, type=heading(1), id='' (len 0) — but no level byte
  const corrupt = Buffer.concat([
    packed.subarray(0, packed.length - 1),   // everything except trailing block count 0
    Buffer.from([1, 1, 0]),                  // block count=1, type=heading, id=empty
  ]);
  assert.throws(() => unpackDocument(corrupt), /Unexpected end of binary document/);
});

test('doc-binary decodeBlock throws on truncated checklist item', () => {
  const packed = packDocument({ title: '', summary: '', tags: [], meta: {}, blocks: [] });
  // Append: blockCount=1, type=checklist(9), id='' (len 0), itemCount=1 — but no checked byte
  const corrupt = Buffer.concat([
    packed.subarray(0, packed.length - 1),   // everything except trailing block count 0
    Buffer.from([1, 9, 0, 1]),               // blockCount=1, type=checklist, id=empty, itemCount=1
  ]);
  assert.throws(() => unpackDocument(corrupt), /Unexpected end of binary document/);
});
