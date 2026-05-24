import { createHash } from 'node:crypto';
import { brotliCompressSync, brotliDecompressSync, constants as zlibConstants } from 'node:zlib';
import { mkdir, readFile, stat, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { unpackDocument } from './doc-binary.js';

const DXLITE_MAGIC = Buffer.from('DXLIT1', 'ascii');
const BUNDLE_MAGIC_V2 = Buffer.from('DXBUN2', 'ascii');
const BUNDLE_MAGIC_V3 = Buffer.from('DXBUN3', 'ascii');

interface DxliteArchiveGitFlags {
  tracked?: boolean;
  untracked?: boolean;
  ignored?: boolean;
  modified?: boolean;
  staged?: boolean;
}

interface DxliteArchiveEntry {
  sha256: string;
  packedBytes: number;
  _packed?: Buffer | Uint8Array;
  payload?: string;
  git?: DxliteArchiveGitFlags;
}

interface DxliteArchiveContainer {
  version: number;
  documents: Record<string, DxliteArchiveEntry>;
}

interface DxliteDocRow {
  id: number;
  path: string;
  packedBytes: number;
  sha256: string;
  git: DxliteArchiveGitFlags;
}

interface DxlitePayload {
  version: 1;
  builtAt: string;
  sourceArchive: string;
  docs: DxliteDocRow[];
  postings: Record<string, number[]>;
}

interface DxliteSearchHit {
  documentPath: string;
  score: number;
}

interface DxliteMemoryIndex {
  archiveMtimeMs: number;
  payload: DxlitePayload;
}

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

const dxliteSidecarFileCache = new Map<string, { mtimeMs: number; payload: DxlitePayload }>();
const dxliteMemoryIndexByArchive = new Map<string, DxliteMemoryIndex>();
const persistDxliteSidecars = process.env.DOC_DXLITE_PERSIST === '1';

function toDxliteSidecarRelativePath(archiveRelativePath: string): string {
  const normalized = String(archiveRelativePath || '').trim();
  if (!normalized) {
    throw new Error('Archive relative path is required to build dxlite sidecar path.');
  }

  if (normalized.endsWith('.bin')) {
    return `${normalized.slice(0, -4)}.dxlite.bin`;
  }

  return `${normalized}.dxlite.bin`;
}

function tokenize(text: string): string[] {
  const tokens = String(text || '').toLowerCase().match(/[a-z0-9][a-z0-9_-]{1,}/g) || [];
  return tokens.filter((token) => token.length >= 3 && !STOP_WORDS.has(token));
}

function resolvePackedBytes(entry: DxliteArchiveEntry): Buffer {
  if (entry._packed) {
    return Buffer.isBuffer(entry._packed) ? entry._packed : Buffer.from(entry._packed);
  }

  if (entry.payload) {
    const payloadBuffer = Buffer.from(String(entry.payload || ''), 'base64');
    const packedLength = payloadBuffer.readUInt32LE(5);
    const compressed = payloadBuffer.subarray(9);
    return brotliDecompressSync(compressed).subarray(0, packedLength);
  }

  throw new Error('Archive entry is missing packed bytes and payload data.');
}

function buildDxlitePayload(container: DxliteArchiveContainer, sourceArchive: string): DxlitePayload {
  const docs: DxliteDocRow[] = [];
  const postingsMap = new Map<string, Set<number>>();
  const entries = Object.entries(container.documents || {}).sort(([a], [b]) => a.localeCompare(b));

  entries.forEach(([relativePath, entry], index) => {
    const packed = resolvePackedBytes(entry);
    const unpacked = unpackDocument(packed);
    const docId = index + 1;

    docs.push({
      id: docId,
      path: relativePath,
      packedBytes: entry.packedBytes || packed.length,
      sha256: entry.sha256,
      git: entry.git || {},
    });

    const parts: string[] = [
      String(unpacked.title || ''),
      String(unpacked.summary || ''),
    ];

    for (const block of unpacked.blocks || []) {
      if (!block || typeof block !== 'object') {
        continue;
      }

      const candidate = block as Record<string, string | number | boolean | null | undefined | object>;
      if (candidate.text) {
        parts.push(String(candidate.text || ''));
      }
      if (candidate.href) {
        parts.push(String(candidate.href || ''));
      }
      if (candidate.alt) {
        parts.push(String(candidate.alt || ''));
      }
      if (Array.isArray(candidate.items)) {
        for (const item of candidate.items) {
          if (typeof item === 'string') {
            parts.push(item);
            continue;
          }
          if (item && typeof item === 'object') {
            parts.push(String((item as { text?: string }).text || ''));
          }
        }
      }
    }

    const uniqueTokens = new Set(tokenize(parts.join(' ')));
    for (const token of uniqueTokens) {
      if (!postingsMap.has(token)) {
        postingsMap.set(token, new Set());
      }
      postingsMap.get(token)?.add(docId);
    }
  });

  const postings: Record<string, number[]> = {};
  for (const [token, docSet] of postingsMap.entries()) {
    postings[token] = Array.from(docSet).sort((a, b) => a - b);
  }

  return {
    version: 1,
    builtAt: new Date().toISOString(),
    sourceArchive,
    docs,
    postings,
  };
}

function encodeDxlitePayload(payload: DxlitePayload): Buffer {
  const jsonBuffer = Buffer.from(JSON.stringify(payload), 'utf8');
  const compressed = brotliCompressSync(jsonBuffer, {
    params: {
      [zlibConstants.BROTLI_PARAM_MODE]: zlibConstants.BROTLI_MODE_GENERIC,
      [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
      [zlibConstants.BROTLI_PARAM_LGWIN]: 24,
      [zlibConstants.BROTLI_PARAM_SIZE_HINT]: jsonBuffer.length,
    },
  });

  const header = Buffer.alloc(DXLITE_MAGIC.length + 4);
  DXLITE_MAGIC.copy(header, 0);
  header.writeUInt32LE(compressed.length, DXLITE_MAGIC.length);
  return Buffer.concat([header, compressed]);
}

function decodeDxlitePayload(buffer: Buffer): DxlitePayload {
  const minBytes = DXLITE_MAGIC.length + 4;
  if (buffer.length < minBytes || !buffer.subarray(0, DXLITE_MAGIC.length).equals(DXLITE_MAGIC)) {
    throw new Error('DXLITE sidecar header is invalid.');
  }

  const compressedLength = buffer.readUInt32LE(DXLITE_MAGIC.length);
  const compressedStart = minBytes;
  if (buffer.length < compressedStart + compressedLength) {
    throw new Error('DXLITE sidecar is truncated.');
  }

  const compressed = buffer.subarray(compressedStart, compressedStart + compressedLength);
  const jsonText = brotliDecompressSync(compressed).toString('utf8');
  const parsed = JSON.parse(jsonText) as DxlitePayload;

  if (!parsed || typeof parsed !== 'object' || !Array.isArray(parsed.docs) || !parsed.postings) {
    throw new Error('DXLITE sidecar payload is invalid.');
  }

  return parsed;
}

function decodeBundleArchive(buffer: Buffer): DxliteArchiveContainer | null {
  const minLen = BUNDLE_MAGIC_V3.length + 4;
  if (buffer.length < minLen) {
    return null;
  }

  const magic = buffer.subarray(0, BUNDLE_MAGIC_V3.length);
  const isV3 = magic.equals(BUNDLE_MAGIC_V3);
  const isV2 = magic.equals(BUNDLE_MAGIC_V2);

  if (!isV2 && !isV3) {
    return null;
  }

  const compressedLen = buffer.readUInt32LE(BUNDLE_MAGIC_V3.length);
  const compressedStart = BUNDLE_MAGIC_V3.length + 4;
  if (buffer.length < compressedStart + compressedLen) {
    return null;
  }

  const compressed = buffer.subarray(compressedStart, compressedStart + compressedLen);
  const uncompressed = brotliDecompressSync(compressed);

  let off = 0;
  const entryCount = uncompressed.readUInt32LE(off);
  off += 4;

  const headers: Array<{ relativePath: string; sha256: string; flags: number; packedLen: number }> = [];
  for (let i = 0; i < entryCount; i += 1) {
    const pathLen = uncompressed.readUInt8(off);
    off += 1;
    const relativePath = uncompressed.toString('utf8', off, off + pathLen);
    off += pathLen;
    const sha256 = isV2 ? uncompressed.toString('hex', off, off + 32) : '';
    if (isV2) {
      off += 32;
    }
    const flags = uncompressed.readUInt8(off);
    off += 1;
    const packedLen = uncompressed.readUInt32LE(off);
    off += 4;
    headers.push({ relativePath, sha256, flags, packedLen });
  }

  const documents: Record<string, DxliteArchiveEntry> = {};
  for (const header of headers) {
    const packed = uncompressed.subarray(off, off + header.packedLen);
    off += header.packedLen;
    const sha256 = header.sha256 || createHash('sha256').update(packed).digest('hex');
    documents[header.relativePath] = {
      sha256,
      packedBytes: header.packedLen,
      _packed: packed,
      git: {
        tracked: Boolean(header.flags & 1),
        untracked: Boolean(header.flags & 2),
        ignored: Boolean(header.flags & 4),
        modified: Boolean(header.flags & 8),
        staged: Boolean(header.flags & 16),
      },
    };
  }

  return {
    version: 3,
    documents,
  };
}

async function loadArchiveContainerForIndex(absoluteArchivePath: string): Promise<DxliteArchiveContainer | null> {
  try {
    const archiveBuffer = await readFile(absoluteArchivePath);
    return decodeBundleArchive(archiveBuffer);
  } catch {
    return null;
  }
}

export async function writeDxliteIndex(rootDir: string, archiveRelativePath: string, container: DxliteArchiveContainer) {
  const sidecarRelativePath = toDxliteSidecarRelativePath(archiveRelativePath);
  const sidecarAbsolutePath = path.resolve(rootDir, sidecarRelativePath);
  const payload = buildDxlitePayload(container, archiveRelativePath);

  const archiveAbsolutePath = path.resolve(rootDir, archiveRelativePath);
  let archiveMtimeMs = Date.now();
  try {
    archiveMtimeMs = (await stat(archiveAbsolutePath)).mtimeMs;
  } catch {
    // Best-effort cache timestamp fallback.
  }

  dxliteMemoryIndexByArchive.set(archiveRelativePath, {
    archiveMtimeMs,
    payload,
  });

  if (!persistDxliteSidecars) {
    try {
      await unlink(sidecarAbsolutePath);
    } catch {
      // Ignore missing sidecar files.
    }

    return {
      relativePath: sidecarRelativePath,
      absolutePath: sidecarAbsolutePath,
      bytes: 0,
      docs: payload.docs.length,
    };
  }

  const encoded = encodeDxlitePayload(payload);
  await mkdir(path.dirname(sidecarAbsolutePath), { recursive: true });
  await writeFile(sidecarAbsolutePath, encoded);

  return {
    relativePath: sidecarRelativePath,
    absolutePath: sidecarAbsolutePath,
    bytes: encoded.length,
    docs: payload.docs.length,
  };
}

export async function removeDxliteIndex(rootDir: string, archiveRelativePath: string) {
  const sidecarRelativePath = toDxliteSidecarRelativePath(archiveRelativePath);
  const sidecarAbsolutePath = path.resolve(rootDir, sidecarRelativePath);

  dxliteMemoryIndexByArchive.delete(archiveRelativePath);

  try {
    await unlink(sidecarAbsolutePath);
  } catch {
    // Ignore missing sidecar files.
  }
}

async function readDxliteIndexAt(absolutePath: string): Promise<DxlitePayload | null> {
  let stats;
  try {
    stats = await stat(absolutePath);
  } catch {
    return null;
  }

  const cached = dxliteSidecarFileCache.get(absolutePath);
  if (cached && cached.mtimeMs === stats.mtimeMs) {
    return cached.payload;
  }

  try {
    const buffer = await readFile(absolutePath);
    const payload = decodeDxlitePayload(buffer);
    dxliteSidecarFileCache.set(absolutePath, {
      mtimeMs: stats.mtimeMs,
      payload,
    });
    return payload;
  } catch {
    return null;
  }
}

async function ensureDxliteIndex(rootDir: string, archiveRelativePath: string): Promise<DxlitePayload | null> {
  const archiveAbsolutePath = path.resolve(rootDir, archiveRelativePath);
  const sidecarRelativePath = toDxliteSidecarRelativePath(archiveRelativePath);
  const sidecarAbsolutePath = path.resolve(rootDir, sidecarRelativePath);

  let archiveStats;
  try {
    archiveStats = await stat(archiveAbsolutePath);
  } catch {
    return null;
  }

  const inMemory = dxliteMemoryIndexByArchive.get(archiveRelativePath);
  if (inMemory && inMemory.archiveMtimeMs === archiveStats.mtimeMs) {
    return inMemory.payload;
  }

  if (!persistDxliteSidecars) {
    try {
      await unlink(sidecarAbsolutePath);
    } catch {
      // Ignore missing sidecar files.
    }

    const container = await loadArchiveContainerForIndex(archiveAbsolutePath);
    if (!container) {
      return null;
    }

    const payload = buildDxlitePayload(container, archiveRelativePath);
    dxliteMemoryIndexByArchive.set(archiveRelativePath, {
      archiveMtimeMs: archiveStats.mtimeMs,
      payload,
    });
    return payload;
  }

  let sidecarStats = null;
  try {
    sidecarStats = await stat(sidecarAbsolutePath);
  } catch {
    sidecarStats = null;
  }

  const needsRebuild = !sidecarStats || sidecarStats.mtimeMs < archiveStats.mtimeMs;
  if (needsRebuild) {
    const container = await loadArchiveContainerForIndex(archiveAbsolutePath);
    if (container) {
      await writeDxliteIndex(rootDir, archiveRelativePath, container);
    }
  }

  const payload = await readDxliteIndexAt(sidecarAbsolutePath);
  if (payload) {
    dxliteMemoryIndexByArchive.set(archiveRelativePath, {
      archiveMtimeMs: archiveStats.mtimeMs,
      payload,
    });
  }

  return payload;
}

export async function searchDxliteIndex(rootDir: string, query: string, limit = 50): Promise<DxliteSearchHit[]> {
  const trimmed = String(query || '').trim();
  if (!trimmed) {
    return [];
  }

  const archives = [
    '.doc/.repo-docs.bin',
    '.doc/.local-docs.bin',
  ];

  const indexes = (await Promise.all(archives.map((archiveRelativePath) => ensureDxliteIndex(rootDir, archiveRelativePath)))).filter(Boolean) as DxlitePayload[];
  if (indexes.length === 0) {
    return [];
  }

  const terms = Array.from(new Set(tokenize(trimmed)));
  if (terms.length === 0) {
    return [];
  }

  const scoreByPath = new Map<string, number>();

  for (const index of indexes) {
    const docById = new Map(index.docs.map((doc) => [doc.id, doc.path]));

    for (const term of terms) {
      const postings = index.postings[term] || [];
      for (const docId of postings) {
        const relativePath = docById.get(docId);
        if (!relativePath) {
          continue;
        }

        scoreByPath.set(relativePath, (scoreByPath.get(relativePath) || 0) + 1);
      }
    }
  }

  return Array.from(scoreByPath.entries())
    .map(([documentPath, score]) => ({ documentPath, score }))
    .sort((a, b) => (b.score - a.score) || a.documentPath.localeCompare(b.documentPath))
    .slice(0, Math.max(1, Number(limit) || 50));
}
