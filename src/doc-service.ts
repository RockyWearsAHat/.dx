import { createHash } from 'node:crypto';
import { mkdir, readFile, stat, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { getLegacyCompanionArchivePath, readDocArchive, writeDocArchive } from './doc-archive.js';
import { searchDxliteIndex } from './dxlite.js';
import { createDefaultBlocks, normalizeDocInput, parseDocFile } from './doc-format.js';
import { findDocFiles } from './file-discovery.js';

const DOC_STUB_VERSION = 3;
const DOC_STUB_PREFIX = '@docstub';
const DOC_STUB_TINY_PREFIX = '~';

type DbConnection = unknown;
type NormalizedDocument = ReturnType<typeof normalizeDocInput>;
type BlockInput = Array<Record<string, string | number | boolean | null | string[]>>;

export interface ClientDocument {
  id: number;
  path: string;
  relativePath: string;
  title: string;
  summary: string;
  tags: string[];
  meta: Record<string, string | number | boolean | null | undefined | object>;
  blocks: Array<Record<string, string | number | boolean | null | undefined | object>>;
  source: string;
  updatedAt: string;
}

interface DocInput {
  path: string;
  title?: string;
  summary?: string;
  tags?: string[];
  meta?: Record<string, string | number | boolean | null>;
  blocks?: BlockInput;
}

interface ArchiveInfo {
  codec: string;
  relativePath: string;
  key: string;
  sha256: string;
  packedBytes: number;
  repoArtifactBytes: number;
  localOnly?: boolean;
}

interface StubTarget {
  version: number;
  absolutePath: string;
  archiveRelativePath: string;
  isTinyFormat: boolean;
}

interface CachedWorkspaceDocument {
  mtimeMs: number;
  document: ClientDocument;
}

const workspaceDocumentCache = new Map<string, CachedWorkspaceDocument>();

function assertWithinRoot(rootDir: string, absolutePath: string): string {
  const relative = path.relative(rootDir, absolutePath);

  if (relative.startsWith('..') || path.isAbsolute(relative)) {
    throw new Error('Document path must stay inside the workspace root.');
  }

  return relative || '.';
}

function toStableDocumentId(relativePath: string): number {
  const digest = createHash('sha1').update(relativePath, 'utf8').digest('hex');
  const id = Number.parseInt(digest.slice(0, 8), 16) & 0x7fffffff;
  return id > 0 ? id : 1;
}

function parseStubTarget(rootDir: string, currentPath: string, sourceText: string): StubTarget | null {
  const lines = String(sourceText || '').split('\n').map((line) => line.trim());
  const firstLine = lines[0] || '';

  if (
    firstLine === DOC_STUB_TINY_PREFIX
    || firstLine === '@d3'
    || firstLine.startsWith('@d3 ')
    || firstLine.startsWith(`${DOC_STUB_TINY_PREFIX} `)
  ) {
    const archiveRelativePath = (firstLine === DOC_STUB_TINY_PREFIX || firstLine === '@d3')
      ? ''
      : (
        firstLine.startsWith('@d3 ')
          ? firstLine.slice(4).trim()
          : firstLine.slice(DOC_STUB_TINY_PREFIX.length).trim()
      );

    const absolutePath = currentPath;
    assertWithinRoot(rootDir, absolutePath);

    return {
      version: DOC_STUB_VERSION,
      absolutePath,
      archiveRelativePath,
      isTinyFormat: true,
    };
  }

  if (!firstLine || !firstLine.startsWith(DOC_STUB_PREFIX)) {
    return null;
  }

  const prefixParts = firstLine.split(/\s+/);
  const parsedVersion = Number(prefixParts[1]);
  const version = Number.isFinite(parsedVersion) ? parsedVersion : 0;
  const archiveLine = lines.find((line) => line.startsWith('archive:'));

  const absolutePath = currentPath;
  assertWithinRoot(rootDir, absolutePath);

  return {
    version,
    absolutePath,
    archiveRelativePath: archiveLine ? archiveLine.slice(8).trim() : '',
    isTinyFormat: false,
  };
}

function buildDocStub(rootDir: string, absolutePath: string, archiveRelativePath = ''): string {
  void rootDir;
  void absolutePath;
  if (!archiveRelativePath) {
    return DOC_STUB_TINY_PREFIX;
  }

  return `${DOC_STUB_TINY_PREFIX} ${archiveRelativePath}`;
}

async function writeDocStub(rootDir: string, absolutePath: string, archiveRelativePath = ''): Promise<void> {
  await mkdir(path.dirname(absolutePath), { recursive: true });
  await writeFile(absolutePath, buildDocStub(rootDir, absolutePath, archiveRelativePath), 'utf8');
  workspaceDocumentCache.delete(absolutePath);
}

async function migrateStubVersionToV3(rootDir: string, stub: StubTarget): Promise<void> {
  if (stub.version === DOC_STUB_VERSION && stub.isTinyFormat) {
    return;
  }

  await writeDocStub(rootDir, stub.absolutePath, '');
}

async function persistDocumentArtifacts(rootDir: string, absolutePath: string, document: NormalizedDocument): Promise<void> {
  const archiveInfo = await writeDocArchive(rootDir, absolutePath, document) as ArchiveInfo;
  void archiveInfo;
  await writeDocStub(rootDir, absolutePath, '');

  const legacyCompanion = getLegacyCompanionArchivePath(absolutePath);
  try {
    await unlink(legacyCompanion);
  } catch {
    // Ignore missing legacy companions.
  }

  workspaceDocumentCache.delete(absolutePath);
}

function normalizeParsedDocument(rootDir: string, absolutePath: string, parsed: NormalizedDocument, updatedAt: string): ClientDocument {
  const relativePath = assertWithinRoot(rootDir, absolutePath);
  return {
    id: toStableDocumentId(relativePath),
    path: absolutePath,
    relativePath,
    title: String(parsed.title || path.basename(relativePath, '.dx')),
    summary: String(parsed.summary || ''),
    tags: Array.isArray(parsed.tags) ? parsed.tags.map((tag) => String(tag || '')) : [],
    meta: (parsed.meta && typeof parsed.meta === 'object') ? parsed.meta : {},
    blocks: Array.isArray(parsed.blocks) ? parsed.blocks : [],
    source: String(parsed.source || ''),
    updatedAt,
  };
}

async function readDocumentFromWorkspace(rootDir: string, absolutePath: string): Promise<ClientDocument | null> {
  try {
    const fileStats = await stat(absolutePath);
    const cached = workspaceDocumentCache.get(absolutePath);

    if (cached && cached.mtimeMs === fileStats.mtimeMs) {
      return cached.document;
    }

    const text = await readFile(absolutePath, 'utf8');
    const updatedAt = new Date(fileStats.mtimeMs).toISOString();
    const stub = parseStubTarget(rootDir, absolutePath, text);

    if (stub) {
      try {
        await migrateStubVersionToV3(rootDir, stub);
        const parsed = await readDocArchive(rootDir, stub.absolutePath, stub.archiveRelativePath);
        const normalized = normalizeParsedDocument(rootDir, stub.absolutePath, parsed, updatedAt);
        workspaceDocumentCache.set(absolutePath, {
          mtimeMs: fileStats.mtimeMs,
          document: normalized,
        });
        return normalized;
      } catch {
        return null;
      }
    }

    const parsed = parseDocFile(absolutePath, text);
    const normalized = normalizeParsedDocument(rootDir, absolutePath, parsed, updatedAt);
    workspaceDocumentCache.set(absolutePath, {
      mtimeMs: fileStats.mtimeMs,
      document: normalized,
    });
    return normalized;
  } catch {
    return null;
  }
}

async function listWorkspaceDocuments(rootDir: string): Promise<ClientDocument[]> {
  const docFiles = await findDocFiles(rootDir);
  const loaded = await Promise.all(docFiles.map((absolutePath) => readDocumentFromWorkspace(rootDir, absolutePath)));
  return loaded.filter((doc): doc is ClientDocument => Boolean(doc));
}

function normalizeRelativeDocPath(relativePath: string): string {
  const trimmed = String(relativePath || '').trim().replace(/^\/+/, '');

  if (!trimmed || !trimmed.endsWith('.dx')) {
    throw new Error('A valid .dx path is required.');
  }

  return trimmed;
}

function resolveDocumentPath(rootDir: string, relativePath: string): string {
  const absolutePath = path.resolve(rootDir, normalizeRelativeDocPath(relativePath));
  assertWithinRoot(rootDir, absolutePath);
  return absolutePath;
}

export async function ingestWorkspace(rootDir: string, db: DbConnection): Promise<ClientDocument[]> {
  void db;
  const docFiles = await findDocFiles(rootDir);
  const ingested: ClientDocument[] = [];

  for (const filePath of docFiles) {
    const [text, fileStats] = await Promise.all([
      readFile(filePath, 'utf8'),
      stat(filePath),
    ]);

    const stub = parseStubTarget(rootDir, filePath, text);

    if (stub) {
      try {
        await migrateStubVersionToV3(rootDir, stub);
        const reconstructed = await readDocArchive(rootDir, stub.absolutePath, stub.archiveRelativePath);
        await persistDocumentArtifacts(rootDir, stub.absolutePath, reconstructed);
        ingested.push(normalizeParsedDocument(rootDir, stub.absolutePath, reconstructed, new Date(fileStats.mtimeMs).toISOString()));
      } catch {
        // Leave unresolved stubs untouched; they can be repaired from another peer.
      }

      continue;
    }

    const parsed = parseDocFile(filePath, text);
    await persistDocumentArtifacts(rootDir, filePath, parsed);
    ingested.push(normalizeParsedDocument(rootDir, filePath, parsed, new Date(fileStats.mtimeMs).toISOString()));
  }

  return ingested;
}

export async function createDocument(rootDir: string, db: DbConnection, input: DocInput): Promise<ClientDocument> {
  void db;
  const relativePath = normalizeRelativeDocPath(input.path);

  const absolutePath = resolveDocumentPath(rootDir, relativePath);
  await mkdir(path.dirname(absolutePath), { recursive: true });

  const title = input.title?.trim() || path.basename(relativePath, '.dx');
  const document = normalizeDocInput(absolutePath, {
    title,
    summary: input.summary,
    tags: input.tags,
    meta: input.meta,
    blocks: input.blocks || createDefaultBlocks(title),
  });
  await persistDocumentArtifacts(rootDir, absolutePath, document);
  return normalizeParsedDocument(rootDir, absolutePath, document, new Date().toISOString());
}

export async function saveDocument(rootDir: string, db: DbConnection, documentId: number, input: Partial<DocInput>): Promise<ClientDocument> {
  void db;
  const allDocuments = await listWorkspaceDocuments(rootDir);
  const current = allDocuments.find((doc) => doc.id === Number(documentId)) || null;

  if (!current) {
    throw new Error('Document not found.');
  }

  const document = normalizeDocInput(current.path, {
    title: input.title || current.title,
    summary: input.summary ?? current.summary,
    tags: input.tags ?? current.tags,
    meta: input.meta ?? current.meta,
    blocks: Array.isArray(input.blocks) ? input.blocks : current.blocks,
  });
  await persistDocumentArtifacts(rootDir, current.path, document);
  return normalizeParsedDocument(rootDir, current.path, document, new Date().toISOString());
}

export async function getDocumentByRelativePath(rootDir: string, db: DbConnection, relativePath: string): Promise<ClientDocument | null> {
  void db;
  const absolutePath = resolveDocumentPath(rootDir, relativePath);
  return readDocumentFromWorkspace(rootDir, absolutePath);
}

export async function saveDocumentSourceByRelativePath(rootDir: string, db: DbConnection, relativePath: string, sourceText: string): Promise<ClientDocument> {
  void db;
  const absolutePath = resolveDocumentPath(rootDir, relativePath);
  const parsed = parseDocFile(absolutePath, String(sourceText || ''));
  await persistDocumentArtifacts(rootDir, absolutePath, parsed);
  return normalizeParsedDocument(rootDir, absolutePath, parsed, new Date().toISOString());
}

/**
 * Saves document source text to SQLite and the binary archive, then returns
 * the stub pointer text that should be written to the on-disk .dx file.
 * The caller is responsible for writing the stub to disk; this function never
 * touches the filesystem directly so there is no double-write race.
 */
export async function saveDocumentSourceToDbAndArchive(rootDir: string, db: DbConnection, relativePath: string, sourceText: string): Promise<{ document: ClientDocument; stubText: string }> {
  void db;
  const absolutePath = resolveDocumentPath(rootDir, relativePath);
  const parsed = parseDocFile(absolutePath, String(sourceText || ''));
  const archiveInfo = await writeDocArchive(rootDir, absolutePath, parsed) as ArchiveInfo;
  void archiveInfo;
  const stubText = buildDocStub(rootDir, absolutePath, '');
  return {
    document: normalizeParsedDocument(rootDir, absolutePath, parsed, new Date().toISOString()),
    stubText,
  };
}

export async function getDocument(rootDir: string, db: DbConnection, documentId: number): Promise<ClientDocument | null> {
  void db;
  const allDocuments = await listWorkspaceDocuments(rootDir);
  return allDocuments.find((doc) => doc.id === Number(documentId)) || null;
}

export async function listOrSearchDocuments(rootDir: string, db: DbConnection, query = '') {
  void db;
  const trimmedQuery = String(query || '').trim();
  if (trimmedQuery) {
    const hits = await searchDxliteIndex(rootDir, trimmedQuery, 200);
    const mapped = (await Promise.all(
      hits.map((hit) => readDocumentFromWorkspace(rootDir, path.resolve(rootDir, hit.documentPath)))
    )).filter((doc): doc is ClientDocument => Boolean(doc));

    if (mapped.length > 0) {
      return mapped;
    }

    const allDocuments = await listWorkspaceDocuments(rootDir);
    const lowered = trimmedQuery.toLowerCase();
    return allDocuments.filter((doc) => {
      const haystack = [
        doc.title,
        doc.summary,
        doc.source,
        ...doc.tags,
      ].join('\n').toLowerCase();
      return haystack.includes(lowered);
    });
  }

  return listWorkspaceDocuments(rootDir);
}

export async function reconstructDocument(rootDir: string, db: DbConnection, documentId: number): Promise<string> {
  void db;
  const document = await getDocument(rootDir, null, documentId);

  if (!document) {
    throw new Error('Document not found.');
  }

  assertWithinRoot(rootDir, document.path);
  return document.source;
}