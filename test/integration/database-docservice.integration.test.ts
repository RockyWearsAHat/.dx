import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { writeFileSync } from 'node:fs';

import {
  createDatabase,
  getDocumentByPath,
  getDocumentById,
  getDocumentSectionBySlug,
  getDocumentSections,
  getDocumentViewState,
  getUserConfigValue,
  listDocuments,
  listWorkspaceProjects,
  migrateLegacyWorkspace,
  saveDocumentViewState,
  searchDocuments,
  setUserConfigValue,
} from '#runtime-src/database.js';
  import { upsertDocument } from '#runtime-src/database.js';
import {
  createDocument,
  getDocument,
  getDocumentByRelativePath,
  ingestWorkspace,
  listOrSearchDocuments,
  reconstructDocument,
  saveDocument,
  saveDocumentSourceByRelativePath,
  saveDocumentSourceToDbAndArchive,
} from '#runtime-src/doc-service.js';
import { getLegacyCompanionArchivePath } from '#runtime-src/doc-archive.js';
import { DatabaseSync } from '#runtime-src/sqlite-native.js';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from '../helpers/test-utils.js';

function createDocSource(title, body = 'Paragraph') {
  return [
    `::heading level=1 id=${title.toLowerCase()}`,
    title,
    '::end',
    '',
    `::paragraph id=${title.toLowerCase()}-p`,
    body,
    '::end',
    '',
  ].join('\n');
}

test('database + doc-service workflow from ingest through save/search/reconstruct', async () => {
  const { rootDir, dbPath } = await createTempWorkspace();

  try {
    await writeDxFile(rootDir, 'examples/welcome.dx', createDocSource('Welcome', 'interactive paper body'));
    const db = createDatabase(dbPath);

    const ingested = await ingestWorkspace(rootDir, db);
    assert.equal(ingested.length, 1);

    const stubText = await readFile(path.join(rootDir, 'examples/welcome.dx'), 'utf8');
    assert.ok(stubText.startsWith('@docstub'));
    assert.match(stubText, /artifact:\s+\.doc\//);
    assert.match(stubText, /codec:\s+brotli-docbin-v1/);

    const listed = listDocuments(db, rootDir);
    assert.equal(listed.length, 1);

    const found = searchDocuments(db, rootDir, 'interactive paper');
    assert.equal(found.length, 1);
    assert.ok(found[0].score > 0);

    const byPath = getDocumentByPath(db, rootDir, path.join(rootDir, 'examples/welcome.dx'));
    assert.ok(byPath);

    const byId = getDocumentById(db, byPath.id);
    assert.ok(byId);

    const sections = getDocumentSections(db, byPath.id);
    assert.ok(sections.length >= 1);
    const firstSection = getDocumentSectionBySlug(db, byPath.id, sections[0].slug);
    assert.ok(firstSection);

    saveDocumentViewState(db, byPath.id, { theme: 'dark' });
    assert.deepEqual(getDocumentViewState(db, byPath.id), { theme: 'dark' });

    setUserConfigValue(db, 'preferred_theme', 'dark');
    assert.equal(getUserConfigValue(db, 'preferred_theme', 'auto'), 'dark');
    assert.equal(getUserConfigValue(db, 'missing', 'auto'), 'auto');

    const created = await createDocument(rootDir, db, {
      path: 'notes/new.dx',
      title: 'New Doc',
      summary: 'summary',
      tags: ['a'],
    });

    assert.equal(created.relativePath, 'notes/new.dx');

    const updated = await saveDocument(rootDir, db, created.id, {
      title: 'New Doc 2',
      blocks: [{ type: 'paragraph', text: 'changed' }],
    });
    assert.equal(updated.title, 'New Doc 2');

    const sourceSaved = await saveDocumentSourceByRelativePath(
      rootDir,
      db,
      'notes/new.dx',
      createDocSource('Saved', 'saved body')
    );
    assert.equal(sourceSaved.relativePath, 'notes/new.dx');

    const sourceRoundTrip = await saveDocumentSourceToDbAndArchive(
      rootDir,
      db,
      'notes/new.dx',
      createDocSource('Saved Again', 'saved body 2')
    );
    assert.ok(sourceRoundTrip.stubText.startsWith('@docstub'));

    const gotDoc = await getDocument(rootDir, db, created.id);
    assert.ok(gotDoc);

    const gotByRelative = await getDocumentByRelativePath(rootDir, db, 'notes/new.dx');
    assert.ok(gotByRelative);

    const allDocs = await listOrSearchDocuments(rootDir, db, 'saved');
    assert.ok(allDocs.length >= 1);

    const reconstructed = await reconstructDocument(rootDir, db, created.id);
    assert.ok(reconstructed.includes('::paragraph'));

    const projects = listWorkspaceProjects(db);
    assert.ok(projects.some((p) => p.rootPath === rootDir));

    await assert.rejects(() => reconstructDocument(rootDir, db, 999999), /Document not found/);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('migrateLegacyWorkspace imports missing docs only', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-legacy-tests-');

  try {
    const db = createDatabase(dbPath);
    const legacyPath = path.join(rootDir, 'data', 'doc-index.sqlite');
    await mkdir(path.dirname(legacyPath), { recursive: true });

    const legacy = new DatabaseSync(legacyPath);
    legacy.exec(`
      CREATE TABLE documents (
        id INTEGER PRIMARY KEY,
        path TEXT NOT NULL,
        body TEXT NOT NULL,
        source_mtime_ms INTEGER NOT NULL,
        updated_at TEXT
      );
    `);

    const legacyDocPath = path.join(rootDir, 'examples/legacy.dx');
    legacy.prepare('INSERT INTO documents(path, body, source_mtime_ms, updated_at) VALUES (?, ?, ?, ?)').run(
      legacyDocPath,
      createDocSource('Legacy', 'from legacy db'),
      Date.now(),
      new Date().toISOString()
    );
    legacy.close();

    const first = migrateLegacyWorkspace(db, rootDir, legacyPath);
    assert.equal(first.imported, 1);

    const second = migrateLegacyWorkspace(db, rootDir, legacyPath);
    assert.equal(second.skipped, 1);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

  test('migrateLegacyWorkspace returns early when args are missing', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-migrate-empty-');
    try {
      const db = createDatabase(dbPath);
      // Empty root
      assert.deepEqual(migrateLegacyWorkspace(db, '', '/some/path'), { imported: 0, skipped: 0 });
      // Non-existent legacy path
      assert.deepEqual(migrateLegacyWorkspace(db, rootDir, '/no/such/file.sqlite'), { imported: 0, skipped: 0 });
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('migrateLegacyWorkspace returns early when legacy db has no documents table', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-migrate-notable-');
    try {
      const db = createDatabase(dbPath);
      const legacyPath = path.join(rootDir, 'empty.sqlite');
      const emptyDb = new DatabaseSync(legacyPath);
      emptyDb.exec('CREATE TABLE other_table (id INTEGER PRIMARY KEY)');
      emptyDb.close();

      const result = migrateLegacyWorkspace(db, rootDir, legacyPath);
      assert.deepEqual(result, { imported: 0, skipped: 0 });
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('migrateLegacyWorkspace handles corrupt legacy db gracefully', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-migrate-corrupt-');
    try {
      const db = createDatabase(dbPath);
      const corruptPath = path.join(rootDir, 'corrupt.sqlite');
      writeFileSync(corruptPath, 'this is not a valid sqlite file at all');

      const result = migrateLegacyWorkspace(db, rootDir, corruptPath);
      assert.deepEqual(result, { imported: 0, skipped: 0 });
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('getDocumentByPath returns null when workspace does not exist', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-getbypath-nowkspc-');
    try {
      const db = createDatabase(dbPath);
      const result = getDocumentByPath(db, '/no/such/workspace', '/no/such/workspace/doc.dx');
      assert.equal(result, null);
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('getDocumentByPath returns null when document not in workspace', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-getbypath-nodoc-');
    try {
      const db = createDatabase(dbPath);
      await writeDxFile(rootDir, 'exists.dx', createDocSource('Existing'));
      await ingestWorkspace(rootDir, db);
      // Workspace exists but path is wrong
      const result = getDocumentByPath(db, rootDir, path.join(rootDir, 'nonexistent.dx'));
      assert.equal(result, null);
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

    test('getDocumentById returns null when document does not exist', async () => {
      const { rootDir, dbPath } = await createTempWorkspace('db-getbyid-notfound-');
      try {
        const db = createDatabase(dbPath);
        const result = getDocumentById(db, 999999999);
        assert.equal(result, null);
        db.close();
      } finally {
        await cleanupTempWorkspace(rootDir);
      }
    });

    test('listDocuments returns empty array when workspace root is empty string', async () => {
      const { rootDir, dbPath } = await createTempWorkspace('db-list-emptyroot-');
      try {
        const db = createDatabase(dbPath);
        const result = listDocuments(db, '');
        assert.deepEqual(result, []);
        db.close();
      } finally {
        await cleanupTempWorkspace(rootDir);
      }
    });

      test('getDocumentViewState returns null when document does not exist', async () => {
        const { rootDir, dbPath } = await createTempWorkspace('db-viewstate-null-');
        try {
          const db = createDatabase(dbPath);
          const result = getDocumentViewState(db, 999999999);
          assert.equal(result, null);
          db.close();
        } finally {
          await cleanupTempWorkspace(rootDir);
        }
      });

      test('getDocumentViewState returns null when view_state_json is corrupt', async () => {
        const { rootDir, dbPath } = await createTempWorkspace('db-viewstate-corrupt-');
        try {
          const db = createDatabase(dbPath);
          await writeDxFile(rootDir, 'test.dx', createDocSource('View State'));
          await ingestWorkspace(rootDir, db);
          const docs = listDocuments(db, rootDir);
          const id = docs[0].id;
          db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run('not-valid-json{{{', id);
          const result = getDocumentViewState(db, id);
          assert.equal(result, null);
          db.close();
        } finally {
          await cleanupTempWorkspace(rootDir);
        }
      });

      test('upsertDocument throws when workspace root is empty string', async () => {
        const { rootDir, dbPath } = await createTempWorkspace('db-upsert-emptyroot-');
        try {
          const db = createDatabase(dbPath);
          const doc = { filePath: `${rootDir}/test.dx`, title: 'Test', source: 'placeholder', blocks: [], sections: [], metadata: {}, tags: [], summary: '' };
          assert.throws(() => upsertDocument(db, '', doc), /Workspace root is required/);
          db.close();
        } finally {
          await cleanupTempWorkspace(rootDir);
        }
      });

  test('hydrateDocument falls back to parseDocFile when document_storage row is absent (lines 81-82)', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-no-storage-');
    try {
      const db = createDatabase(dbPath);
      await writeDxFile(rootDir, 'fallback.dx', createDocSource('Fallback', 'No storage row'));
      await ingestWorkspace(rootDir, db);
      const docs = listDocuments(db, rootDir);
      const id = docs[0].id;
      db.prepare('DELETE FROM document_storage WHERE document_id = ?').run(id);
      const doc = getDocumentById(db, id);
      assert.ok(doc, 'Expected document to hydrate via parseDocFile fallback');
      assert.match(doc.source, /No storage row/);
      assert.match(doc.source, /::paragraph/);
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('hydrateDocument falls back to parseDocFile when packed_blob is corrupt (lines 78-79)', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-corrupt-blob-');
    try {
      const db = createDatabase(dbPath);
      await writeDxFile(rootDir, 'corrupt.dx', createDocSource('Corrupt', 'Corrupt blob'));
      await ingestWorkspace(rootDir, db);
      const docs = listDocuments(db, rootDir);
      const id = docs[0].id;
      db.prepare('UPDATE document_storage SET packed_blob = ? WHERE document_id = ?').run(Buffer.from('not-valid-msgpack'), id);
      const doc = getDocumentById(db, id);
      assert.ok(doc, 'Expected document to hydrate via catch fallback');
      assert.match(doc.source, /Corrupt blob/);
      assert.match(doc.source, /::paragraph/);
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('createDatabase runs ALTER TABLE migration when view_state_json column is missing (lines 191-192)', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-migration-');
    try {
      // Create the DB with old schema (no view_state_json column)
      const oldDb = new DatabaseSync(dbPath);
      oldDb.exec(`
        CREATE TABLE IF NOT EXISTS documents (
          id INTEGER PRIMARY KEY,
          path TEXT NOT NULL UNIQUE,
          title TEXT NOT NULL,
          summary TEXT NOT NULL DEFAULT '',
          tags TEXT NOT NULL DEFAULT '[]',
          meta TEXT NOT NULL DEFAULT '{}',
          body TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workspaces (
          id INTEGER PRIMARY KEY,
          root_path TEXT NOT NULL UNIQUE,
          name TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
      `);
      oldDb.close();
      // Now call createDatabase — should trigger migration (ALTER TABLE ADD COLUMN)
      const db = createDatabase(dbPath);
      // Verify the column now exists by inspecting schema.
      const columns = db.prepare('PRAGMA table_info(documents)').all();
      const hasViewStateColumn = columns.some((column) => column.name === 'view_state_json');
      assert.equal(hasViewStateColumn, true);
      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('doc-service covers stub parsing and fallback branches', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('doc-service-branches-');
    try {
      const db = createDatabase(dbPath);

      // path: . forces assertWithinRoot(relative||'.') fallback.
      await writeDxFile(rootDir, 'root-stub.dx', [
        '@docstub',
        'path: .',
      ].join('\n'));

      // Missing optional lines exercises parseStubTarget defaults for artifact/key/archive/etc.
      await writeDxFile(rootDir, 'minimal-stub.dx', '@docstub\n');

      const ingested = await ingestWorkspace(rootDir, db);
      assert.deepEqual(ingested, []);

      // createDocument title fallback and blocks fallback.
      const created = await createDocument(rootDir, db, {
        path: 'fallbacks/no-title.dx',
      });
      assert.equal(created.title, 'no-title');

      // Ensure unlink success path runs by creating a legacy companion file.
      const legacyCompanion = getLegacyCompanionArchivePath(path.join(rootDir, 'fallbacks/no-title.dx'));
      await mkdir(path.dirname(legacyCompanion), { recursive: true });
      await writeFile(legacyCompanion, 'legacy', 'utf8');

      // saveDocument title fallback and blocks ternary false branch.
      const saved = await saveDocument(rootDir, db, created.id, {
        title: '',
        summary: 'updated summary',
      });
      assert.equal(saved.title, created.title);

      // Null source text paths for String(sourceText||'').
      const byRelative = await saveDocumentSourceByRelativePath(rootDir, db, 'fallbacks/no-title.dx', null);
      assert.equal(byRelative.relativePath, 'fallbacks/no-title.dx');

      const byArchive = await saveDocumentSourceToDbAndArchive(rootDir, db, 'fallbacks/no-title.dx', null);
      assert.ok(byArchive.stubText.startsWith('@docstub'));

      // Null-return branches.
      const missingByRelative = await getDocumentByRelativePath(rootDir, db, 'missing.dx');
      assert.equal(missingByRelative, null);
      const missingById = await getDocument(rootDir, db, 99999999);
      assert.equal(missingById, null);

      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('migrateLegacyWorkspace falls back source_mtime_ms and body when legacy row is sparse', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-legacy-sparse-');
    try {
      const db = createDatabase(dbPath);
      const legacyPath = path.join(rootDir, 'legacy-sparse.sqlite');

      const legacy = new DatabaseSync(legacyPath);
      legacy.exec(`
        CREATE TABLE documents (
          id INTEGER PRIMARY KEY,
          path TEXT NOT NULL,
          body TEXT,
          source_mtime_ms INTEGER,
          updated_at TEXT
        );
      `);

      const legacyDocPath = path.join(rootDir, 'examples', 'sparse.dx');
      legacy
        .prepare('INSERT INTO documents(path, body, source_mtime_ms, updated_at) VALUES (?, ?, ?, ?)')
        .run(legacyDocPath, null, null, new Date().toISOString());
      legacy.close();

      const result = migrateLegacyWorkspace(db, rootDir, legacyPath);
      assert.equal(result.imported, 1);

      const imported = getDocumentByPath(db, rootDir, legacyDocPath);
      assert.ok(imported);

      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

  test('migrateLegacyWorkspace handles legacy path that is a directory', async () => {
    const { rootDir, dbPath } = await createTempWorkspace('db-legacy-dir-');
    try {
      const db = createDatabase(dbPath);
      const legacyDirPath = path.join(rootDir, 'legacy-dir.sqlite');
      await mkdir(legacyDirPath, { recursive: true });

      const result = migrateLegacyWorkspace(db, rootDir, legacyDirPath);
      assert.deepEqual(result, { imported: 0, skipped: 0 });

      db.close();
    } finally {
      await cleanupTempWorkspace(rootDir);
    }
  });

