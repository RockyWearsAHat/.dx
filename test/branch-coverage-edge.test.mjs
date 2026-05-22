import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { chmod, mkdir, writeFile } from 'node:fs/promises';
import { DatabaseSync } from 'node:sqlite';

import {
  createDatabase,
  getDocumentByPath,
  getWorkspaceDocumentById,
  migrateLegacyWorkspace,
  upsertDocument,
} from '../src/database.js';
import { normalizeDocInput, parseDocFile, stringifyDocFile } from '../src/doc-format.js';
import { createDocument, ingestWorkspace } from '../src/doc-service.js';
import { mergeDocumentViewState, normalizeDocumentViewState } from '../src/view-state.js';
import { packDocument, unpackDocument } from '../src/doc-binary.js';
import { parseAttributes, parseSourceBlocks } from '../vscode-extension/media/doc-pipeline.js';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from './test-utils.mjs';

function skipString(buffer, state) {
  const length = readVarint(buffer, state);
  state.offset += length;
}

function readVarint(buffer, state) {
  let shift = 0;
  let value = 0;

  while (state.offset < buffer.length) {
    const byte = buffer[state.offset];
    state.offset += 1;
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) break;
    shift += 7;
  }

  return value;
}

test('doc-pipeline covers attribute parsing and empty stylesheet/code edge paths', () => {
  const attrs = parseAttributes("one='1' two=2 three=\"3\"");
  assert.deepEqual(attrs, { one: '1', two: '2', three: '3' });

  const malformedWrapped = parseSourceBlocks([
    '::paragraph id=paragraph-1',
    '',
    '',
  ].join('\n'));
  assert.equal(Array.isArray(malformedWrapped), true);

  const blocks = parseSourceBlocks([
    '::code lang=ts console.log(1) ::end',
    '::stylesheet ::end',
    '::stylesheet',
    '::end',
  ].join('\n'));

  const code = blocks.find((block) => block.type === 'code');
  const stylesheets = blocks.filter((block) => block.type === 'stylesheet');

  assert.ok(code, 'Expected code block');
  assert.equal(code.language, 'ts');
  assert.equal(stylesheets.length, 2);
  assert.equal(stylesheets[0].href, '');
  assert.equal(stylesheets[1].href, '');
});

test('view-state normalizers handle non-object patches and null input', () => {
  const normalized = normalizeDocumentViewState(null);
  assert.equal(normalized.theme, 'auto');
  assert.equal(normalized.resolvedTheme, 'dark');

  const mergedWithPrimitivePatch = mergeDocumentViewState({
    theme: 'light',
    appearance: { paper: 'cream', density: 'compact', scale: 101 },
    viewport: { width: 1000, height: 700, zoomLevel: 1, zoomFactor: 1.2 },
  }, 7);
  assert.equal(mergedWithPrimitivePatch.theme, 'light');
  assert.equal(mergedWithPrimitivePatch.appearance.paper, 'cream');

  const mergedWithInvalidNestedPatch = mergeDocumentViewState({
    appearance: { paper: 'white', density: 'comfortable', scale: 100 },
    viewport: { width: 800, height: 600, zoomLevel: 0, zoomFactor: 1 },
  }, {
    appearance: 'invalid',
    viewport: 'invalid',
  });

  assert.equal(mergedWithInvalidNestedPatch.appearance.paper, 'white');
  assert.equal(mergedWithInvalidNestedPatch.viewport.width, 800);
});

test('doc-binary handles defensive encode defaults and unknown block type fallback', () => {
  const packed = packDocument({
    title: 't',
    summary: 's',
    tags: { not: 'an-array' },
    meta: null,
    blocks: [{ type: 'paragraph', text: 'body' }],
  });

  const unpacked = unpackDocument(packed);
  assert.deepEqual(unpacked.tags, []);
  assert.deepEqual(unpacked.meta, {});

  const mutated = Buffer.from(packed);
  const state = { offset: 5 };

  // schema version
  readVarint(mutated, state);
  skipString(mutated, state); // title
  skipString(mutated, state); // summary

  // tags
  const tagsCount = readVarint(mutated, state);
  for (let index = 0; index < tagsCount; index += 1) {
    skipString(mutated, state);
  }

  // meta count
  const metaCount = readVarint(mutated, state);
  for (let index = 0; index < metaCount; index += 1) {
    skipString(mutated, state);
    skipString(mutated, state);
  }

  // block count varint
  const blockCount = readVarint(mutated, state);
  assert.equal(blockCount > 0, true);

  // First block type code byte.
  mutated[state.offset] = 255;
  const decoded = unpackDocument(mutated);
  assert.equal(decoded.blocks[0].type, 'paragraph');
});

test('database hydration and migration cover defensive source/mtime fallbacks', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-db-branch-');
  const db = createDatabase(dbPath);

  try {
    const absolutePath = path.join(rootDir, 'edge-fallback.dx');
    const parsed = parseDocFile(absolutePath, '');
    const id = upsertDocument(db, rootDir, parsed, Date.now());

    db.prepare('UPDATE documents SET body = ? WHERE id = ?').run('', id);
    db.prepare('UPDATE document_storage SET source_bytes = ?, packed_bytes = ? WHERE document_id = ?').run(0, 10, id);

    const hydrated = getWorkspaceDocumentById(db, rootDir, id);
    assert.equal(hydrated.sourceBytes, 0);
    assert.equal(hydrated.compressionRatio, 0);

    const legacyPath = path.join(rootDir, 'legacy.sqlite');
    const legacyDb = new DatabaseSync(legacyPath);
    legacyDb.exec('CREATE TABLE documents (path TEXT, body TEXT, source_mtime_ms INTEGER, updated_at TEXT);');
    legacyDb.prepare('INSERT INTO documents(path, body, source_mtime_ms, updated_at) VALUES(?, ?, ?, ?)').run(
      path.join(rootDir, 'legacy-null-mtime.dx'),
      '',
      null,
      new Date().toISOString(),
    );
    legacyDb.close();

    const migration = migrateLegacyWorkspace(db, rootDir, legacyPath);
    assert.equal(migration.imported >= 1, true);

      const legacyDir = path.join(rootDir, 'legacy-dir');
      await mkdir(legacyDir, { recursive: true });
      const directoryMigration = migrateLegacyWorkspace(db, rootDir, legacyDir);
      assert.deepEqual(directoryMigration, { imported: 0, skipped: 0 });

      const unreadableLegacy = path.join(rootDir, 'legacy-unreadable.sqlite');
      await writeFile(unreadableLegacy, 'not-sqlite', 'utf8');
      await chmod(unreadableLegacy, 0);
      const unreadableMigration = migrateLegacyWorkspace(db, rootDir, unreadableLegacy);
      assert.deepEqual(unreadableMigration, { imported: 0, skipped: 0 });
      await chmod(unreadableLegacy, 0o644);
  } finally {
    db.close();
    await cleanupTempWorkspace(rootDir);
  }
});

test('doc-service ingest handles empty existing source and v3 archive stub branch', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-service-branch-');
  const db = createDatabase(dbPath);

  try {
    await assert.rejects(
      () => createDocument(rootDir, db, { title: 'missing path' }),
      /valid \.dx path is required/,
    );

    const existingAbs = path.join(rootDir, 'examples/existing-empty.dx');
    const parsed = parseDocFile(existingAbs, '');
    const existingId = upsertDocument(db, rootDir, parsed, Date.now());
    db.prepare('UPDATE documents SET body = ? WHERE id = ?').run('', existingId);

    await writeDxFile(rootDir, 'examples/existing-empty.dx', [
      '@docstub 3',
      'path: examples/existing-empty.dx',
      'archive: archives/ignored-v3-path.dxbun',
    ].join('\n'));

    await writeDxFile(rootDir, 'examples/v3-reconstruct-miss.dx', [
      '@docstub 3',
      'path: examples/v3-reconstruct-miss.dx',
      'archive: archives/missing-file.dxbun',
    ].join('\n'));

    const ingested = await ingestWorkspace(rootDir, db);
    const existing = getDocumentByPath(db, rootDir, existingAbs);

    assert.ok(existing, 'Expected existing stub-backed document to remain addressable');
    assert.equal(Array.isArray(ingested), true);
  } finally {
    db.close();
    await cleanupTempWorkspace(rootDir);
  }
});

test('doc-format parser and stringifier edge cases cover inline and fallback branches', () => {
  const parsed = parseDocFile('edge.dx', [
    '::rule ::end',
    '::paragraph inline text ::end',
    '::stylesheet',
    '::end',
    '::paragraph',
    'Body',
    '::end',
    '::heading',
    '::end',
  ].join('\n'));

  const stylesheet = parsed.blocks.find((block) => block.type === 'stylesheet');
  assert.ok(stylesheet);
  assert.equal(stylesheet.href, '');

  const heading = parsed.blocks.find((block) => block.type === 'heading');
  assert.ok(heading);

  const source = stringifyDocFile({
    title: 'Stringify branch coverage',
    blocks: [
      {},
      { type: 'paragraph', text: '', id: '' },
      {
        type: 'bulleted-list',
        id: '',
        items: [0, { text: 'kept' }],
      },
    ],
  });

  assert.match(source, /::paragraph\s+id=/);
  assert.match(source, /- kept/);
});

test('doc-format and doc-pipeline handle empty inputs and code language fallbacks', () => {
  assert.throws(() => parseDocFile('empty.dx'), /indexOf/);

  const parsed = parseDocFile('code-and-lists.dx', [
    '::code lang=ts',
    'console.log(1)',
    '::end',
    '::code language=js',
    'console.log(2)',
    '::end',
    '::code',
    'console.log(3)',
    '::end',
    '::list',
    '',
    '- item',
    '::end',
    '::checklist',
    '',
    '[x] done',
    '::end',
  ].join('\n'));

  const codeBlocks = parsed.blocks.filter((block) => block.type === 'code');
  assert.equal(codeBlocks.length, 3);
  assert.equal(codeBlocks[0].language, 'ts');
  assert.equal(codeBlocks[1].language, 'js');
  assert.equal(codeBlocks[2].language, '');

  const pipelineBlocks = parseSourceBlocks([
    '::code language=jsx const x = 1; ::end',
    '::code const y = 2; ::end',
  ].join('\n'));
  const pipelineCodeBlocks = pipelineBlocks.filter((block) => block.type === 'code');
  assert.equal(pipelineCodeBlocks.length, 2);
  assert.equal(pipelineCodeBlocks[0].language, 'jsx');
  assert.equal(pipelineCodeBlocks[1].language, '');
});

test('normalizeDocInput covers block-type/list/checklist fallback branches', () => {
  const normalized = normalizeDocInput('branches.dx', {
    title: 'Branches',
    blocks: [
      { type: 'unknown' },
      { type: 'heading', text: '' },
      {
        type: 'bulleted-list',
        items: [
          { text: '' },
          { text: 'kept', nested: [{ text: 'child' }] },
        ],
      },
      {
        type: 'numbered-list',
        text: '\nfirst\nsecond\n',
      },
      {
        type: 'bulleted-list',
        items: 'invalid-items-shape',
      },
      {
        type: 'checklist',
        items: 'invalid-items-shape',
      },
      {
        type: 'checklist',
        items: [0],
      },
      {
        type: 'checklist',
        items: [null],
      },
    ],
  });

  const heading = normalized.blocks.find((block) => block.type === 'heading');
  assert.ok(heading);
  assert.match(heading.text, /Section/);

  const numbered = normalized.blocks.find((block) => block.type === 'numbered-list');
  assert.ok(numbered);
  assert.equal(numbered.items.length >= 2, true);

  const checklists = normalized.blocks.filter((block) => block.type === 'checklist');
  assert.equal(checklists.length >= 2, true);
  assert.equal(checklists[0].items.length > 0, true);
  assert.equal(checklists[1].items.length > 0, true);
});
