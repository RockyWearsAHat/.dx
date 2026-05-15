import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { brotliCompressSync, brotliDecompressSync, constants as zlibConstants } from 'node:zlib';

import { packDocument, unpackDocument } from './doc-binary.js';
import { normalizeDocInput } from './doc-format.js';

const ARCHIVE_MAGIC = Buffer.from('DXAR1', 'ascii');
const ARCHIVE_CODEC = 'brotli-docbin-v1';
const REPO_ARCHIVE_MAGIC = Buffer.from('DXRA1', 'ascii');
const REPO_ARCHIVE_RELATIVE_PATH = '.doc/.repo-docs.bin';

function readUInt32LE(buffer, offset) {
  if (offset + 4 > buffer.length) {
    throw new Error('Archive payload is truncated.');
  }

  return buffer.readUInt32LE(offset);
}

function legacyCompanionArchivePath(absoluteDocPath) {
  const docPath = String(absoluteDocPath || '');

  if (!docPath.endsWith('.dx')) {
    return `${docPath}.dxz`;
  }

  return `${docPath.slice(0, -3)}.dxz`;
}

function toRelativePath(rootDir, absolutePath) {
  const relative = path.relative(rootDir, absolutePath);

  if (relative.startsWith('..') || path.isAbsolute(relative)) {
    throw new Error('Archive path must stay inside the workspace root.');
  }

  return relative;
}

function encodeArchivePayload(document) {
  const packed = packDocument(document);
  const compressed = brotliCompressSync(packed, {
    params: {
      [zlibConstants.BROTLI_PARAM_MODE]: zlibConstants.BROTLI_MODE_GENERIC,
      [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
      [zlibConstants.BROTLI_PARAM_LGWIN]: 24,
      [zlibConstants.BROTLI_PARAM_SIZE_HINT]: packed.length,
    },
  });

  const header = Buffer.alloc(ARCHIVE_MAGIC.length + 4);
  ARCHIVE_MAGIC.copy(header, 0);
  header.writeUInt32LE(packed.length, ARCHIVE_MAGIC.length);

  const archiveBuffer = Buffer.concat([header, compressed]);
  const sha256 = createHash('sha256').update(archiveBuffer).digest('hex');

  return {
    archiveBuffer,
    sha256,
    packedBytes: packed.length,
    archiveBytes: archiveBuffer.length,
  };
}

function decodeArchivePayload(filePath, archiveBuffer) {
  const buffer = Buffer.isBuffer(archiveBuffer) ? archiveBuffer : Buffer.from(archiveBuffer || []);

  if (buffer.length < ARCHIVE_MAGIC.length + 4) {
    throw new Error('Archive payload is too small.');
  }

  const magic = buffer.subarray(0, ARCHIVE_MAGIC.length);

  if (!magic.equals(ARCHIVE_MAGIC)) {
    throw new Error('Archive payload has an invalid header.');
  }

  const packedLength = readUInt32LE(buffer, ARCHIVE_MAGIC.length);
  const compressed = buffer.subarray(ARCHIVE_MAGIC.length + 4);
  const packed = brotliDecompressSync(compressed);

  if (packed.length !== packedLength) {
    throw new Error('Archive payload failed integrity validation.');
  }

  const unpacked = unpackDocument(packed);
  return normalizeDocInput(filePath, unpacked);
}

function getRepoArchiveAbsolutePath(rootDir) {
  return path.resolve(rootDir, REPO_ARCHIVE_RELATIVE_PATH);
}

function decodeRepoArchive(buffer) {
  const payload = Buffer.isBuffer(buffer) ? buffer : Buffer.from(buffer || []);

  if (payload.length < REPO_ARCHIVE_MAGIC.length) {
    throw new Error('Repository archive is invalid or truncated.');
  }

  if (!payload.subarray(0, REPO_ARCHIVE_MAGIC.length).equals(REPO_ARCHIVE_MAGIC)) {
    throw new Error('Repository archive header is invalid.');
  }

  const compressed = payload.subarray(REPO_ARCHIVE_MAGIC.length);
  const jsonText = brotliDecompressSync(compressed).toString('utf8');
  const parsed = JSON.parse(jsonText);

  if (!parsed || typeof parsed !== 'object' || !parsed.documents || typeof parsed.documents !== 'object') {
    throw new Error('Repository archive has an invalid schema.');
  }

  return parsed;
}

function encodeRepoArchive(container) {
  const normalized = {
    version: 1,
    codec: ARCHIVE_CODEC,
    updatedAt: new Date().toISOString(),
    documents: container.documents || {},
  };
  const jsonBuffer = Buffer.from(JSON.stringify(normalized), 'utf8');
  const compressed = brotliCompressSync(jsonBuffer, {
    params: {
      [zlibConstants.BROTLI_PARAM_MODE]: zlibConstants.BROTLI_MODE_GENERIC,
      [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
      [zlibConstants.BROTLI_PARAM_LGWIN]: 24,
      [zlibConstants.BROTLI_PARAM_SIZE_HINT]: jsonBuffer.length,
    },
  });

  return Buffer.concat([REPO_ARCHIVE_MAGIC, compressed]);
}

async function readRepoArchiveContainer(rootDir) {
  const absolutePath = getRepoArchiveAbsolutePath(rootDir);

  try {
    const payload = await readFile(absolutePath);
    return decodeRepoArchive(payload);
  } catch {
    return {
      version: 1,
      codec: ARCHIVE_CODEC,
      documents: {},
      updatedAt: new Date().toISOString(),
    };
  }
}

async function writeRepoArchiveContainer(rootDir, container) {
  const absolutePath = getRepoArchiveAbsolutePath(rootDir);
  await mkdir(path.dirname(absolutePath), { recursive: true });
  const payload = encodeRepoArchive(container);
  await writeFile(absolutePath, payload);

  return {
    absolutePath,
    relativePath: REPO_ARCHIVE_RELATIVE_PATH,
    bytes: payload.length,
  };
}

export function getDocArchiveMetadata(rootDir, absoluteDocPath) {
  const relativeDocPath = toRelativePath(rootDir, absoluteDocPath);

  return {
    absolutePath: getRepoArchiveAbsolutePath(rootDir),
    relativePath: REPO_ARCHIVE_RELATIVE_PATH,
    key: relativeDocPath,
    codec: ARCHIVE_CODEC,
  };
}

export async function writeDocArchive(rootDir, absoluteDocPath, document) {
  const archiveMeta = getDocArchiveMetadata(rootDir, absoluteDocPath);
  const payload = encodeArchivePayload(document);
  const container = await readRepoArchiveContainer(rootDir);

  container.documents[archiveMeta.key] = {
    sha256: payload.sha256,
    packedBytes: payload.packedBytes,
    archiveBytes: payload.archiveBytes,
    payload: payload.archiveBuffer.toString('base64'),
  };

  const repoArtifact = await writeRepoArchiveContainer(rootDir, container);

  return {
    codec: archiveMeta.codec,
    relativePath: repoArtifact.relativePath,
    key: archiveMeta.key,
    sha256: payload.sha256,
    packedBytes: payload.packedBytes,
    archiveBytes: payload.archiveBytes,
    repoArtifactBytes: repoArtifact.bytes,
  };
}

export async function readDocArchive(rootDir, absoluteDocPath, explicitRelativeArchivePath = '') {
  const archiveMeta = getDocArchiveMetadata(rootDir, absoluteDocPath);
  const container = await readRepoArchiveContainer(rootDir);
  const key = archiveMeta.key;
  const entry = container.documents && container.documents[key];

  if (entry && entry.payload) {
    const archiveBuffer = Buffer.from(String(entry.payload || ''), 'base64');
    return decodeArchivePayload(absoluteDocPath, archiveBuffer);
  }

  if (explicitRelativeArchivePath) {
    const legacyPath = path.resolve(rootDir, String(explicitRelativeArchivePath || ''));
    const legacyBuffer = await readFile(legacyPath);
    return decodeArchivePayload(absoluteDocPath, legacyBuffer);
  }

  throw new Error(`Archive entry not found for document key: ${key}`);
}

export function getLegacyCompanionArchivePath(absoluteDocPath) {
  return legacyCompanionArchivePath(absoluteDocPath);
}
