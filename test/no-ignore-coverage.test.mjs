import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';

import {
  createDatabase,
  getDocumentById,
  getDocumentByPath,
  getDocumentSections,
  getDocumentViewState,
  getUserConfigValue,
  listWorkspaceProjects,
  migrateLegacyWorkspace,
  saveDocumentViewState,
  setUserConfigValue,
  upsertDocument,
} from '../src/database.js';
import { packDocument } from '../src/doc-binary.js';
import { normalizeDocInput, parseDocFile, stringifyDocFile } from '../src/doc-format.js';
import {
  ingestWorkspace,
  saveDocumentSourceToDbAndArchive,
} from '../src/doc-service.js';
import { parseAttributes, parseSourceBlocks } from '../vscode-extension/media/doc-pipeline.js';
import { DatabaseSync } from '../src/sqlite-native.js';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from './test-utils.mjs';

test('stringifyDocFile exercises list and block-body fallback branches', () => {
  const doc = {
    blocks: [
      { type: 'bulleted-list', id: 'list-a', items: null },
      { type: 'numbered-list', id: 'list-b', items: ['first', { text: 'second' }, { nested: [] }] },
      { type: 'image', id: 'img-no-alt' },
      { type: 'style', id: 'style-empty' },
      { type: 'stylesheet', id: 'sheet-empty' },
      { type: 'rule', id: 'rule-a' },
      { type: 'paragraph', id: 'p-empty' },
    ],
  };

  const source = stringifyDocFile(doc);
  assert.ok(source.includes('::bulleted-list id=list-a'));
  assert.ok(source.includes('::numbered-list id=list-b'));
  assert.ok(source.includes('- first'));
  assert.ok(source.includes('- second'));
  assert.ok(source.includes('::image id=img-no-alt'));
  assert.ok(source.includes('::style id=style-empty'));
  assert.ok(source.includes('::stylesheet id=sheet-empty'));
  assert.ok(source.includes('::rule id=rule-a'));

  const emptySource = stringifyDocFile({});
  assert.equal(emptySource, '\n');
});

test('doc-pipeline exercises attribute and stylesheet/code fallback variants', () => {
  const attrs = parseAttributes("href='a.css' media=screen data-id=hero");
  assert.equal(attrs.href, 'a.css');
  assert.equal(attrs.media, 'screen');
  assert.equal(attrs['data-id'], 'hero');

  const blocks = parseSourceBlocks([
    '::rule class=separator ::end',
    '::stylesheet src=theme.css ::end',
    '::stylesheet',
    'https://cdn.example/styles.css',
    '::end',
    '::code lang=js const v = 1; ::end',
    '::heading',
    'Implicit level heading',
    '::end',
    '::paragraph class=intro',
    '',
    '::end',
  ].join('\n'));

  const inlineRule = blocks.find((b) => b.type === 'rule');
  assert.equal(inlineRule.id, '');

  const inlineStylesheet = blocks.find((b) => b.type === 'stylesheet' && b.href === 'theme.css');
  assert.ok(inlineStylesheet);

  const blockStylesheet = blocks.find((b) => b.type === 'stylesheet' && b.href.includes('cdn.example'));
  assert.ok(blockStylesheet);

  const code = blocks.find((b) => b.type === 'code');
  assert.equal(code.language, 'js');

  const heading = blocks.find((b) => b.type === 'heading');
  assert.equal(heading.level, 1);

  assert.equal(blocks.some((b) => b.type === 'paragraph' && !b.text.trim()), false);
});

test('doc-format exercises canonical parser and normalization fallbacks', () => {
  const canonical = [
    '::heading',
    '::end',
    '::code language=ts',
    'const x = 1;',
    '::end',
    '::list',
    '-',
    '  - nested item',
    '::end',
    '::image',
    'image alt text',
    '::end',
    '::checklist',
    '[ ]',
    'task',
    '::end',
    '::stylesheet',
    'theme.css',
    '::end',
    '::style',
    '.a{color:red;}',
    '::end',
    '::paragraph id=paragraph-1',
    'wrapped paragraph',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/fallbacks.dx', canonical);
  assert.ok(parsed.blocks.some((b) => b.type === 'heading'));
  assert.ok(parsed.blocks.some((b) => b.type === 'code' && b.language === 'ts'));
  assert.ok(parsed.blocks.some((b) => b.type === 'bulleted-list'));
  assert.ok(parsed.blocks.some((b) => b.type === 'image' && b.alt === 'image alt text'));
  assert.ok(parsed.blocks.some((b) => b.type === 'checklist'));
  assert.ok(parsed.blocks.some((b) => b.type === 'stylesheet' && b.href === 'theme.css'));
  assert.ok(parsed.blocks.some((b) => b.type === 'style'));

  const normalized = normalizeDocInput('/tmp/normalize.dx', {
    blocks: {
      not: 'an-array',
    },
  });
  assert.ok(Array.isArray(normalized.blocks));

  const derived = normalizeDocInput('/tmp/derive.dx', {
    blocks: [
      { type: 'image', src: 'cover.png', alt: '' },
      { type: 'paragraph', text: '' },
      { type: 'checklist', items: [{ checked: true, text: '' }, 'todo'] },
      { type: 'stylesheet', src: 'shared.css', media: '' },
      { type: 'style' },
    ],
  });
  assert.ok(derived.summary.length > 0);
});

test('database and doc-service fallback branches are exercised without c8 ignores', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-no-ignore-');
  const db = createDatabase(dbPath);

  try {
    const mainDocPath = path.join(rootDir, 'docs/main.dx');
    const normalized = normalizeDocInput(mainDocPath, {
      title: 'Main',
      blocks: [
        { type: 'heading', level: 1, text: 'Main' },
        { type: 'paragraph', text: 'Body' },
      ],
    });

    normalized.summary = '';
    normalized.source = '';
    const insertedId = upsertDocument(db, rootDir, normalized, Date.now());

    delete normalized.source;
    normalized.summary = undefined;
    upsertDocument(db, rootDir, normalized, Date.now());

    setUserConfigValue(db, undefined, null);
    assert.equal(getUserConfigValue(db, undefined, 'fallback'), '');

    saveDocumentViewState(db, insertedId, 'not-an-object');
    assert.equal(getDocumentViewState(db, insertedId), null);

    const body = '::paragraph id=p\nX\n::end\n';
    const parsed = parseDocFile(path.join(rootDir, 'docs/malformed.dx'), body);
    const packed = packDocument(parsed);
    const now = new Date().toISOString();

    const malformedInsert = db.prepare(`
      INSERT INTO documents (path, title, summary, metadata_json, body, outline_json, source_mtime_ms, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      path.join(rootDir, 'docs/malformed.dx'),
      'Malformed',
      '',
      '{}',
      body,
      '[]',
      Date.now(),
      now,
      now
    );

    const malformedId = Number(malformedInsert.lastInsertRowid);

    db.prepare(`
      INSERT INTO document_storage (document_id, packed_blob, packed_bytes, source_bytes, updated_at)
      VALUES (?, ?, ?, ?, ?)
    `).run(malformedId, packed, packed.length, 0, now);

    const hydrated = getDocumentById(db, malformedId);
    assert.ok(hydrated.sourceBytes > 0);
    assert.ok(typeof hydrated.compressionRatio === 'number');

    db.prepare(`
      INSERT INTO sections (document_id, slug, heading, depth, position, content)
      VALUES (?, ?, ?, ?, ?, ?)
    `).run(malformedId, 'empty', 'Empty', 1, 0, '');

    const sections = getDocumentSections(db, malformedId);
    assert.equal(sections[0].excerpt, '');

    db.prepare(`
      INSERT INTO workspaces (root_path, name, created_at, updated_at)
      VALUES (?, ?, ?, ?)
    `).run('/tmp/no-docs-workspace', 'no-docs-workspace', now, now);

    const projects = listWorkspaceProjects(db);
    assert.ok(projects.some((project) => project.rootPath === '/tmp/no-docs-workspace' && project.documentCount === 0));

    const slashDoc = normalizeDocInput('/workspace/slash.dx', {
      title: 'Slash',
      blocks: [{ type: 'paragraph', text: 'slash workspace root' }],
    });
    upsertDocument(db, '/', slashDoc, Date.now());

    const legacyPath = path.join(rootDir, '.tmp/legacy.sqlite');
    const legacyDb = new DatabaseSync(legacyPath);
    legacyDb.exec(`
      CREATE TABLE documents (
        path TEXT,
        body TEXT,
        source_mtime_ms INTEGER,
        updated_at TEXT
      );
    `);

    legacyDb.prepare('INSERT INTO documents(path, body, source_mtime_ms, updated_at) VALUES (?, ?, ?, ?)').run(
      path.join(rootDir, 'legacy/imported.dx'),
      null,
      0,
      now
    );
    legacyDb.close();

    const migration = migrateLegacyWorkspace(db, rootDir, legacyPath);
    assert.equal(migration.imported, 1);

    const emptyPathMigration = migrateLegacyWorkspace(db, rootDir, undefined);
    assert.deepEqual(emptyPathMigration, { imported: 0, skipped: 0 });

    const badLegacyPath = path.join(rootDir, '.tmp/not-a-db.txt');
    await writeDxFile(rootDir, '.tmp/not-a-db.txt', 'this is not sqlite');
    const badMigration = migrateLegacyWorkspace(db, rootDir, badLegacyPath);
    assert.deepEqual(badMigration, { imported: 0, skipped: 0 });

    await writeDxFile(rootDir, 'docs/empty-source.dx', '');

    const existingSourceDocPath = path.join(rootDir, 'docs/stubbed.dx');
    const existingSourceDoc = parseDocFile(existingSourceDocPath, '');
    upsertDocument(db, rootDir, existingSourceDoc, Date.now());

    await writeDxFile(rootDir, 'docs/stub-pointer.dx', [
      '@docstub 3',
      'path: docs/stubbed.dx',
      'archive: missing.bin',
    ].join('\n'));

    await ingestWorkspace(rootDir, db);

    const saved = await saveDocumentSourceToDbAndArchive(rootDir, db, '/docs/new-source.dx', undefined);
    assert.ok(saved.stubText.startsWith('@docstub'));

    const hydratedByPath = getDocumentByPath(db, rootDir, mainDocPath);
    assert.ok(hydratedByPath);
  } finally {
    db.close();
    await cleanupTempWorkspace(rootDir);
  }
});

test('sqlite-native falls back to node:sqlite when native binding path is invalid', async () => {
  const previousPath = process.env.DOC_SQLITE_NATIVE_BINDING_PATH;

  try {
    process.env.DOC_SQLITE_NATIVE_BINDING_PATH = '../build/Release/does-not-exist.node';
    const sqliteModule = await import(`../src/sqlite-native.js?fallback=${Date.now()}`);

    assert.equal(sqliteModule.isNativeSQLiteBridge, false);

    const db = new sqliteModule.DatabaseSync(':memory:');
    db.exec('CREATE TABLE t(id INTEGER PRIMARY KEY, value TEXT NOT NULL);');
    db.prepare('INSERT INTO t(value) VALUES (?)').run('fallback-ok');
    const row = db.prepare('SELECT value FROM t WHERE id = 1').get();
    assert.equal(row.value, 'fallback-ok');
    db.close();
  } finally {
    if (previousPath === undefined) {
      delete process.env.DOC_SQLITE_NATIVE_BINDING_PATH;
    } else {
      process.env.DOC_SQLITE_NATIVE_BINDING_PATH = previousPath;
    }
  }
});
