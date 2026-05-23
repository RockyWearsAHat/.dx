import { DatabaseSync } from './sqlite-native.js';
import { existsSync } from 'node:fs';
import { packDocument, unpackDocument } from './doc-binary.js';
import { normalizeDocInput, parseDocFile, stringifyDocFile } from './doc-format.js';
const STOP_WORDS = new Set([
    'a',
    'an',
    'and',
    'are',
    'as',
    'at',
    'be',
    'but',
    'by',
    'for',
    'from',
    'has',
    'have',
    'in',
    'into',
    'is',
    'it',
    'of',
    'on',
    'or',
    'that',
    'the',
    'their',
    'this',
    'to',
    'we',
    'with',
]);
function tokenize(text) {
    const counts = new Map();
    const normalized = String(text || '').toLowerCase().match(/[a-z0-9][a-z0-9_-]{1,}/g) || [];
    for (const token of normalized) {
        if (token.length < 3 || STOP_WORDS.has(token)) {
            continue;
        }
        counts.set(token, (counts.get(token) || 0) + 1);
    }
    return counts;
}
function mergeTokenCounts(target, tokenCounts, multiplier = 1) {
    for (const [token, count] of tokenCounts.entries()) {
        target.set(token, (target.get(token) || 0) + count * multiplier);
    }
}
function getDocumentStorageRow(db, documentId) {
    return db.prepare(`
    SELECT packed_blob, packed_bytes, source_bytes
    FROM document_storage
    WHERE document_id = ?
  `).get(documentId);
}
function hydrateDocumentFromRow(db, row) {
    const storage = getDocumentStorageRow(db, row.id);
    let normalized;
    if (storage?.packed_blob) {
        try {
            const unpacked = unpackDocument(storage.packed_blob);
            normalized = normalizeDocInput(row.path, {
                ...unpacked,
                source: row.body,
            });
        }
        catch {
            normalized = parseDocFile(row.path, row.body);
        }
    }
    else {
        normalized = parseDocFile(row.path, row.body);
    }
    const packedBytes = storage?.packed_bytes || 0;
    // Hydrated rows always have source_bytes; fallback is defensive for legacy/corrupt rows.
    const sourceBytes = storage?.source_bytes || Buffer.byteLength(row.body || '', 'utf8');
    // Zero-byte branch is defensive for malformed legacy rows.
    const compressionRatio = sourceBytes > 0 ? Number((packedBytes / sourceBytes).toFixed(4)) : 0;
    return {
        id: row.id,
        path: row.path,
        title: normalized.title,
        summary: normalized.summary,
        tags: normalized.tags,
        meta: normalized.meta,
        metadata: normalized.metadata,
        source: row.body,
        blocks: normalized.blocks,
        sections: normalized.sections,
        outline: normalized.sections.map((section) => ({
            id: section.id,
            heading: section.heading,
            depth: section.depth,
        })),
        sourceMtimeMs: row.source_mtime_ms,
        createdAt: row.created_at,
        updatedAt: row.updated_at,
        packedBytes,
        sourceBytes,
        compressionRatio,
    };
}
export function createDatabase(dbPath) {
    const db = new DatabaseSync(dbPath);
    db.exec(`
    CREATE TABLE IF NOT EXISTS workspaces (
      id INTEGER PRIMARY KEY,
      root_path TEXT NOT NULL UNIQUE,
      name TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS documents (
      id INTEGER PRIMARY KEY,
      path TEXT NOT NULL UNIQUE,
      title TEXT NOT NULL,
      summary TEXT NOT NULL,
      metadata_json TEXT NOT NULL,
      body TEXT NOT NULL,
      outline_json TEXT NOT NULL,
      view_state_json TEXT,
      source_mtime_ms INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS document_storage (
      document_id INTEGER PRIMARY KEY,
      packed_blob BLOB NOT NULL,
      packed_bytes INTEGER NOT NULL,
      source_bytes INTEGER NOT NULL,
      updated_at TEXT NOT NULL,
      FOREIGN KEY(document_id) REFERENCES documents(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS sections (
      id INTEGER PRIMARY KEY,
      document_id INTEGER NOT NULL,
      slug TEXT NOT NULL,
      heading TEXT NOT NULL,
      depth INTEGER NOT NULL,
      position INTEGER NOT NULL,
      content TEXT NOT NULL,
      FOREIGN KEY(document_id) REFERENCES documents(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS tokens (
      id INTEGER PRIMARY KEY,
      document_id INTEGER NOT NULL,
      section_id INTEGER,
      token TEXT NOT NULL,
      hits INTEGER NOT NULL,
      FOREIGN KEY(document_id) REFERENCES documents(id) ON DELETE CASCADE,
      FOREIGN KEY(section_id) REFERENCES sections(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS workspace_documents (
      workspace_id INTEGER NOT NULL,
      document_id INTEGER NOT NULL,
      PRIMARY KEY (workspace_id, document_id),
      FOREIGN KEY(workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE,
      FOREIGN KEY(document_id) REFERENCES documents(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS user_config (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_documents_updated_at ON documents(updated_at DESC);
    CREATE INDEX IF NOT EXISTS idx_sections_document_id ON sections(document_id, position);
  `);
    // Schema migration: add view_state_json column if it doesn't exist
    try {
        db.prepare(`SELECT view_state_json FROM documents LIMIT 0`).all();
    }
    catch {
        db.exec(`ALTER TABLE documents ADD COLUMN view_state_json TEXT;`);
    }
    db.exec(`
    CREATE INDEX IF NOT EXISTS idx_tokens_token ON tokens(token);
    CREATE INDEX IF NOT EXISTS idx_tokens_document_id ON tokens(document_id);
    CREATE INDEX IF NOT EXISTS idx_workspaces_root_path ON workspaces(root_path);
    CREATE INDEX IF NOT EXISTS idx_workspace_documents_workspace_id ON workspace_documents(workspace_id);
    CREATE INDEX IF NOT EXISTS idx_workspace_documents_document_id ON workspace_documents(document_id);
  `);
    return db;
}
export function getDocumentViewState(db, documentId) {
    const row = db.prepare(`SELECT view_state_json FROM documents WHERE id = ?`).get(documentId);
    if (!row || !row.view_state_json) {
        return null;
    }
    try {
        return JSON.parse(row.view_state_json);
    }
    catch {
        return null;
    }
}
export function saveDocumentViewState(db, documentId, viewState) {
    // Non-object values are intentionally normalized to null.
    const json = viewState && typeof viewState === 'object' ? JSON.stringify(viewState) : null;
    db.prepare(`UPDATE documents SET view_state_json = ? WHERE id = ?`).run(json, documentId);
}
export function getUserConfigValue(db, key, fallbackValue = null) {
    // Keys should always be strings; fallback is defensive.
    const row = db.prepare(`SELECT value FROM user_config WHERE key = ?`).get(String(key || ''));
    if (!row) {
        return fallbackValue;
    }
    return row.value;
}
export function setUserConfigValue(db, key, value) {
    const now = new Date().toISOString();
    db.prepare(`
    INSERT INTO user_config (key, value, updated_at)
    VALUES (?, ?, ?)
    ON CONFLICT(key) DO UPDATE SET
      value = excluded.value,
      updated_at = excluded.updated_at
  `).run(
    // Keys should always be strings; fallback is defensive.
    String(key || ''), 
    // Nullish value normalization is defensive.
    String(value ?? ''), now);
}
function ensureWorkspace(db, workspaceRoot) {
    const normalizedRoot = String(workspaceRoot || '').trim();
    if (!normalizedRoot) {
        throw new Error('Workspace root is required.');
    }
    const existing = db.prepare(`SELECT id FROM workspaces WHERE root_path = ?`).get(normalizedRoot);
    if (existing?.id) {
        return Number(existing.id);
    }
    const now = new Date().toISOString();
    // Roots should always contain at least one segment.
    const name = normalizedRoot.split('/').filter(Boolean).pop() || normalizedRoot;
    const result = db.prepare(`
    INSERT INTO workspaces (root_path, name, created_at, updated_at)
    VALUES (?, ?, ?, ?)
  `).run(normalizedRoot, name, now, now);
    return Number(result.lastInsertRowid);
}
function getWorkspaceId(db, workspaceRoot) {
    const normalizedRoot = String(workspaceRoot || '').trim();
    if (!normalizedRoot) {
        return null;
    }
    const row = db.prepare(`SELECT id FROM workspaces WHERE root_path = ?`).get(normalizedRoot);
    return row?.id ? Number(row.id) : null;
}
function insertTokensForDocument(db, documentId, document) {
    const insertToken = db.prepare(`
    INSERT INTO tokens (document_id, section_id, token, hits)
    VALUES (?, ?, ?, ?)
  `);
    const documentTokens = new Map();
    mergeTokenCounts(documentTokens, tokenize(document.title), 6);
    // Summary defaults are defensive for legacy callers.
    mergeTokenCounts(documentTokens, tokenize(document.summary || ''), 3);
    for (const [token, hits] of documentTokens.entries()) {
        insertToken.run(documentId, null, token, hits);
    }
    const sectionRows = db.prepare(`
    SELECT id, heading, content
    FROM sections
    WHERE document_id = ?
    ORDER BY position ASC
  `).all(documentId);
    for (const section of sectionRows) {
        const sectionTokens = new Map();
        mergeTokenCounts(sectionTokens, tokenize(section.heading), 3);
        mergeTokenCounts(sectionTokens, tokenize(section.content), 1);
        for (const [token, hits] of sectionTokens.entries()) {
            insertToken.run(documentId, section.id, token, hits);
        }
    }
}
export function upsertDocument(db, workspaceRoot, document, sourceMtimeMs) {
    const now = new Date().toISOString();
    const existing = db.prepare(`SELECT id FROM documents WHERE path = ?`).get(document.filePath);
    // Normalized docs should already include source.
    const sourceText = document.source || stringifyDocFile(document);
    const packedBlob = packDocument(document);
    const sourceBytes = Buffer.byteLength(sourceText, 'utf8');
    const packedBytes = packedBlob.length;
    const metadataJson = JSON.stringify(document.metadata);
    const outlineJson = JSON.stringify(document.sections.map((section) => ({
        id: section.id,
        heading: section.heading,
        depth: section.depth,
    })));
    db.exec('BEGIN');
    try {
        const workspaceId = ensureWorkspace(db, workspaceRoot);
        let documentId = existing?.id;
        if (documentId) {
            db.prepare(`DELETE FROM tokens WHERE document_id = ?`).run(documentId);
            db.prepare(`DELETE FROM sections WHERE document_id = ?`).run(documentId);
            db.prepare(`
        UPDATE documents
        SET title = ?, summary = ?, metadata_json = ?, body = ?, outline_json = ?, source_mtime_ms = ?, updated_at = ?
        WHERE id = ?
      `).run(document.title, 
            // Summary fallback is defensive for legacy callers.
            document.summary || '', metadataJson, sourceText, outlineJson, sourceMtimeMs, now, documentId);
        }
        else {
            const result = db.prepare(`
        INSERT INTO documents (
          path,
          title,
          summary,
          metadata_json,
          body,
          outline_json,
          source_mtime_ms,
          created_at,
          updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      `).run(document.filePath, document.title, 
            // Summary fallback is defensive for legacy callers.
            document.summary || '', metadataJson, sourceText, outlineJson, sourceMtimeMs, now, now);
            documentId = Number(result.lastInsertRowid);
        }
        db.prepare(`
      INSERT INTO document_storage (document_id, packed_blob, packed_bytes, source_bytes, updated_at)
      VALUES (?, ?, ?, ?, ?)
      ON CONFLICT(document_id) DO UPDATE SET
        packed_blob = excluded.packed_blob,
        packed_bytes = excluded.packed_bytes,
        source_bytes = excluded.source_bytes,
        updated_at = excluded.updated_at
    `).run(documentId, packedBlob, packedBytes, sourceBytes, now);
        db.prepare(`
      INSERT INTO workspace_documents (workspace_id, document_id)
      VALUES (?, ?)
      ON CONFLICT(workspace_id, document_id) DO NOTHING
    `).run(workspaceId, documentId);
        const insertSection = db.prepare(`
      INSERT INTO sections (document_id, slug, heading, depth, position, content)
      VALUES (?, ?, ?, ?, ?, ?)
    `);
        document.sections.forEach((section, position) => {
            insertSection.run(documentId, section.id, section.heading, section.depth, position, section.content);
        });
        insertTokensForDocument(db, documentId, document);
        db.exec('COMMIT');
        return documentId;
    }
    catch (error) {
        db.exec('ROLLBACK');
        throw error;
    }
}
export function getDocumentById(db, documentId) {
    const row = db.prepare(`SELECT * FROM documents WHERE id = ?`).get(documentId);
    if (!row) {
        return null;
    }
    return hydrateDocumentFromRow(db, row);
}
export function getWorkspaceDocumentById(db, workspaceRoot, documentId) {
    const workspaceId = getWorkspaceId(db, workspaceRoot);
    if (!workspaceId) {
        return null;
    }
    const row = db.prepare(`
    SELECT d.*
    FROM documents d
    JOIN workspace_documents wd ON wd.document_id = d.id
    WHERE wd.workspace_id = ? AND d.id = ?
  `).get(workspaceId, documentId);
    if (!row) {
        return null;
    }
    return hydrateDocumentFromRow(db, row);
}
export function getDocumentByPath(db, workspaceRoot, documentPath) {
    const workspaceId = getWorkspaceId(db, workspaceRoot);
    if (!workspaceId) {
        return null;
    }
    const row = db.prepare(`
    SELECT d.*
    FROM documents d
    JOIN workspace_documents wd ON wd.document_id = d.id
    WHERE wd.workspace_id = ? AND d.path = ?
  `).get(workspaceId, documentPath);
    if (!row) {
        return null;
    }
    return hydrateDocumentFromRow(db, row);
}
export function listDocuments(db, workspaceRoot) {
    const workspaceId = getWorkspaceId(db, workspaceRoot);
    if (!workspaceId) {
        return [];
    }
    const rows = db.prepare(`
    SELECT d.id, d.path, d.title, d.summary, d.metadata_json, d.body, d.outline_json, d.source_mtime_ms, d.created_at, d.updated_at
    FROM documents d
    JOIN workspace_documents wd ON wd.document_id = d.id
    WHERE wd.workspace_id = ?
    ORDER BY updated_at DESC, title COLLATE NOCASE ASC
  `).all(workspaceId);
    return rows.map((row) => hydrateDocumentFromRow(db, row));
}
export function searchDocuments(db, workspaceRoot, query) {
    const workspaceId = getWorkspaceId(db, workspaceRoot);
    if (!workspaceId) {
        return [];
    }
    const terms = Array.from(tokenize(query).keys());
    if (terms.length === 0) {
        return listDocuments(db, workspaceRoot).map((document) => ({
            ...document,
            score: 0,
            matches: [],
        }));
    }
    const placeholders = terms.map(() => '?').join(', ');
    const scoredRows = db.prepare(`
    SELECT
      t.document_id,
      COUNT(DISTINCT token) AS matched_terms,
      SUM(hits) AS total_hits
    FROM tokens t
    JOIN workspace_documents wd ON wd.document_id = t.document_id
    WHERE wd.workspace_id = ? AND token IN (${placeholders})
    GROUP BY t.document_id
    ORDER BY matched_terms DESC, total_hits DESC
  `).all(workspaceId, ...terms);
    return scoredRows.map((row) => {
        const document = getWorkspaceDocumentById(db, workspaceRoot, row.document_id);
        const matches = db.prepare(`
      SELECT s.heading, s.slug, s.content, SUM(t.hits) AS score
      FROM tokens t
      JOIN sections s ON s.id = t.section_id
      JOIN workspace_documents wd ON wd.document_id = t.document_id
      WHERE wd.workspace_id = ? AND t.document_id = ? AND t.token IN (${placeholders})
      GROUP BY s.id
      ORDER BY score DESC, s.position ASC
      LIMIT 3
    `).all(workspaceId, row.document_id, ...terms);
        return {
            ...document,
            score: Number(row.total_hits),
            matches: matches.map((match) => ({
                heading: match.heading,
                slug: match.slug,
                excerpt: match.content.slice(0, 220),
                score: match.score,
            })),
        };
    });
}
export function getDocumentSections(db, documentId) {
    return db.prepare(`
    SELECT id, slug, heading, depth, position, content
    FROM sections
    WHERE document_id = ?
    ORDER BY position ASC
  `).all(documentId).map((row) => ({
        id: Number(row.id),
        slug: row.slug,
        heading: row.heading,
        depth: Number(row.depth),
        position: Number(row.position),
        content: row.content,
        // Content fallback is defensive for legacy rows.
        excerpt: String(row.content || '').slice(0, 280),
    }));
}
export function getDocumentSectionBySlug(db, documentId, slug) {
    const row = db.prepare(`
    SELECT id, slug, heading, depth, position, content
    FROM sections
    WHERE document_id = ? AND slug = ?
  `).get(documentId, slug);
    if (!row)
        return null;
    return {
        id: Number(row.id),
        slug: row.slug,
        heading: row.heading,
        depth: Number(row.depth),
        position: Number(row.position),
        content: row.content,
    };
}
export function listWorkspaceProjects(db) {
    const rows = db.prepare(`
    SELECT w.id, w.root_path, w.name, w.created_at, w.updated_at, COUNT(wd.document_id) AS document_count
    FROM workspaces w
    LEFT JOIN workspace_documents wd ON wd.workspace_id = w.id
    GROUP BY w.id
    ORDER BY w.updated_at DESC
  `).all();
    return rows.map((row) => ({
        id: Number(row.id),
        rootPath: row.root_path,
        name: row.name,
        // COUNT should always be numeric; fallback is defensive.
        documentCount: Number(row.document_count || 0),
        createdAt: row.created_at,
        updatedAt: row.updated_at,
    }));
}
export function migrateLegacyWorkspace(db, workspaceRoot, legacyDbPath) {
    // Public callers should pass non-empty roots; fallback is defensive.
    const root = String(workspaceRoot || '').trim();
    // Public callers should pass non-empty paths; fallback is defensive.
    const legacyPath = String(legacyDbPath || '').trim();
    if (!root || !legacyPath || !existsSync(legacyPath)) {
        return { imported: 0, skipped: 0 };
    }
    let legacyDb;
    try {
        legacyDb = new DatabaseSync(legacyPath);
        const tables = legacyDb.prepare(`SELECT name FROM sqlite_master WHERE type = ?`).all('table');
        const hasDocuments = tables.some((row) => row.name === 'documents');
        if (!hasDocuments) {
            return { imported: 0, skipped: 0 };
        }
        const rows = legacyDb.prepare(`
      SELECT path, body, source_mtime_ms
      FROM documents
      WHERE path LIKE ?
      ORDER BY updated_at DESC
    `).all(`${root}/%`);
        let imported = 0;
        let skipped = 0;
        for (const row of rows) {
            // Legacy body fallback for incomplete rows.
            const parsed = parseDocFile(row.path, row.body || '');
            const existing = getDocumentByPath(db, root, row.path);
            if (existing) {
                skipped += 1;
                continue;
            }
            upsertDocument(db, root, parsed, 
            // source_mtime_ms fallback is defensive for malformed legacy rows.
            Number(row.source_mtime_ms || Date.now()));
            imported += 1;
        }
        return { imported, skipped };
    }
    catch {
        return { imported: 0, skipped: 0 };
        /* c8 ignore next */
    }
    finally {
        legacyDb?.close();
    }
}
