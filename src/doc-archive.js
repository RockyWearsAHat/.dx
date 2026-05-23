import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { brotliCompressSync, brotliDecompressSync, constants as zlibConstants } from 'node:zlib';
import { packDocument, unpackDocument } from './doc-binary.js';
import { normalizeDocInput } from './doc-format.js';
import { getGitDocState } from './git-doc-state.js';
const ARCHIVE_MAGIC = Buffer.from('DXAR1', 'ascii');
const ARCHIVE_CODEC = 'brotli-docbin-v1';
const REPO_ARCHIVE_MAGIC = Buffer.from('DXRA1', 'ascii');
const BUNDLE_MAGIC = Buffer.from('DXBUN2', 'ascii');
const REPO_ARCHIVE_RELATIVE_PATH = '.doc/.repo-docs.bin';
const LOCAL_ARCHIVE_RELATIVE_PATH = '.doc/.local-docs.bin';
const LOCAL_ARCHIVE_GITIGNORE_ENTRY = '/.doc/.local-docs.bin';
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
function getLocalArchiveAbsolutePath(rootDir) {
    return path.resolve(rootDir, LOCAL_ARCHIVE_RELATIVE_PATH);
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
// ---------------------------------------------------------------------------
// DXBUN2 — single-pass bundle format (replaces DXRA1 JSON+base64 format)
//
// Layout (all multi-byte integers are little-endian):
//   [6 bytes]  magic "DXBUN2"
//   [4 bytes]  uint32: total compressed payload length
//   [N bytes]  single brotli-compressed payload containing:
//     [4 bytes]  uint32: entry count
//     per entry (header table):
//       [1 byte]   path length (uint8, max 255 chars — relative .dx paths)
//       [N bytes]  path UTF-8 bytes
//       [32 bytes] SHA-256 of the packed document bytes
//       [1 byte]   git flags: bit0=tracked bit1=untracked bit2=ignored bit3=modified bit4=staged
//       [4 bytes]  uint32: packed document byte length
//     per entry (payload section, same order as header table):
//       [N bytes]  raw packDocument() bytes (no individual brotli)
//
// All document bytes are compressed together as one corpus, giving brotli
// far more opportunity to find redundancy than per-document compression.
// ---------------------------------------------------------------------------
function encodeBundle(container) {
    const entries = Object.entries(container.documents || {});
    const pathBufs = entries.map(([k]) => Buffer.from(k, 'utf8'));
    const packedBufs = entries.map(([, v]) => Buffer.from(v._packed));
    const shas = entries.map(([, v]) => Buffer.from(v.sha256, 'hex'));
    const gitFlagsBufs = entries.map(([, v]) => {
        const g = v.git || {};
        let flags = 0;
        if (g.tracked)
            flags |= 0x01;
        if (g.untracked)
            flags |= 0x02;
        if (g.ignored)
            flags |= 0x04;
        if (g.modified)
            flags |= 0x08;
        if (g.staged)
            flags |= 0x10;
        return flags;
    });
    const headerParts = [Buffer.alloc(4)];
    headerParts[0].writeUInt32LE(entries.length, 0);
    for (let i = 0; i < entries.length; i++) {
        const pathBuf = pathBufs[i];
        const pathLen = Math.min(pathBuf.length, 255);
        const row = Buffer.alloc(1 + pathLen + 32 + 1 + 4);
        let off = 0;
        row.writeUInt8(pathLen, off++);
        pathBuf.copy(row, off, 0, pathLen);
        off += pathLen;
        shas[i].copy(row, off);
        off += 32;
        row.writeUInt8(gitFlagsBufs[i], off++);
        row.writeUInt32LE(packedBufs[i].length, off);
        headerParts.push(row);
    }
    const uncompressed = Buffer.concat([...headerParts, ...packedBufs]);
    const compressed = brotliCompressSync(uncompressed, {
        params: {
            [zlibConstants.BROTLI_PARAM_MODE]: zlibConstants.BROTLI_MODE_GENERIC,
            [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
            [zlibConstants.BROTLI_PARAM_LGWIN]: 24,
            [zlibConstants.BROTLI_PARAM_SIZE_HINT]: uncompressed.length,
        },
    });
    const lenBuf = Buffer.alloc(4);
    lenBuf.writeUInt32LE(compressed.length, 0);
    return Buffer.concat([BUNDLE_MAGIC, lenBuf, compressed]);
}
function decodeBundle(buffer) {
    const buf = Buffer.isBuffer(buffer) ? buffer : Buffer.from(buffer || []);
    const minLen = BUNDLE_MAGIC.length + 4;
    if (buf.length < minLen || !buf.subarray(0, BUNDLE_MAGIC.length).equals(BUNDLE_MAGIC)) {
        throw new Error('Bundle header is invalid.');
    }
    const compressedLen = buf.readUInt32LE(BUNDLE_MAGIC.length);
    const compressedStart = BUNDLE_MAGIC.length + 4;
    if (buf.length < compressedStart + compressedLen) {
        throw new Error('Bundle payload is truncated.');
    }
    const uncompressed = brotliDecompressSync(buf.subarray(compressedStart, compressedStart + compressedLen));
    let off = 0;
    const entryCount = uncompressed.readUInt32LE(off);
    off += 4;
    const headers = [];
    for (let i = 0; i < entryCount; i++) {
        const pathLen = uncompressed.readUInt8(off++);
        const relativePath = uncompressed.toString('utf8', off, off + pathLen);
        off += pathLen;
        const sha256 = uncompressed.toString('hex', off, off + 32);
        off += 32;
        const flags = uncompressed.readUInt8(off++);
        const packedLen = uncompressed.readUInt32LE(off);
        off += 4;
        headers.push({ relativePath, sha256, flags, packedLen });
    }
    const documents = {};
    for (const h of headers) {
        const packed = uncompressed.subarray(off, off + h.packedLen);
        off += h.packedLen;
        documents[h.relativePath] = {
            sha256: h.sha256,
            packedBytes: h.packedLen,
            _packed: packed,
            git: {
                tracked: Boolean(h.flags & 0x01),
                untracked: Boolean(h.flags & 0x02),
                ignored: Boolean(h.flags & 0x04),
                modified: Boolean(h.flags & 0x08),
                staged: Boolean(h.flags & 0x10),
            },
        };
    }
    return { version: 2, documents };
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
async function readArchiveContainerAt(absolutePath) {
    try {
        const payload = await readFile(absolutePath);
        // Prefer new DXBUN2 format; fall back to legacy DXRA1 JSON format.
        if (payload.length >= BUNDLE_MAGIC.length && payload.subarray(0, BUNDLE_MAGIC.length).equals(BUNDLE_MAGIC)) {
            return decodeBundle(payload);
        }
        // Legacy DXRA1 — migrate entries from base64+brotli to raw _packed buffers
        // so encodeBundle can write them out in the new format.
        const legacy = decodeRepoArchive(payload);
        for (const [key, entry] of Object.entries(legacy.documents || {})) {
            if (entry.payload && !entry._packed) {
                try {
                    const archiveBuffer = Buffer.from(String(entry.payload), 'base64');
                    // archiveBuffer is DXAR1 + uint32(packedLen) + brotli(packed)
                    const packedLen = archiveBuffer.readUInt32LE(ARCHIVE_MAGIC.length);
                    const compressed = archiveBuffer.subarray(ARCHIVE_MAGIC.length + 4);
                    entry._packed = brotliDecompressSync(compressed).subarray(0, packedLen);
                    delete entry.payload;
                }
                catch {
                    // If migration fails for an entry, drop it — it will re-ingest on next save.
                    delete legacy.documents[key];
                }
            }
        }
        return legacy;
    }
    catch {
        return {
            version: 2,
            documents: {},
        };
    }
}
async function writeRepoArchiveContainer(rootDir, container) {
    const absolutePath = getRepoArchiveAbsolutePath(rootDir);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    const payload = encodeBundle(container);
    await writeFile(absolutePath, payload);
    return {
        absolutePath,
        relativePath: REPO_ARCHIVE_RELATIVE_PATH,
        bytes: payload.length,
    };
}
async function readRepoArchiveContainer(rootDir) {
    return readArchiveContainerAt(getRepoArchiveAbsolutePath(rootDir));
}
async function readLocalArchiveContainer(rootDir) {
    return readArchiveContainerAt(getLocalArchiveAbsolutePath(rootDir));
}
async function writeArchiveContainer(rootDir, relativePath, container) {
    const absolutePath = path.resolve(rootDir, relativePath);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    const payload = encodeBundle(container);
    await writeFile(absolutePath, payload);
    return {
        absolutePath,
        relativePath,
        bytes: payload.length,
    };
}
async function writeLocalArchiveContainer(rootDir, container) {
    await ensureGitIgnored(rootDir, LOCAL_ARCHIVE_GITIGNORE_ENTRY);
    return writeArchiveContainer(rootDir, LOCAL_ARCHIVE_RELATIVE_PATH, container);
}
async function ensureGitIgnored(rootDir, entry) {
    const trimmedEntry = String(entry || '').trim();
    if (!trimmedEntry) {
        return;
    }
    const gitIgnorePath = path.resolve(rootDir, '.gitignore');
    let current = '';
    try {
        current = await readFile(gitIgnorePath, 'utf8');
    }
    catch {
        current = '';
    }
    const lines = current.split(/\r?\n/).map((line) => line.trim());
    if (lines.includes(trimmedEntry)) {
        return;
    }
    const suffix = current && !current.endsWith('\n') ? '\n' : '';
    await writeFile(gitIgnorePath, `${current}${suffix}${trimmedEntry}\n`, 'utf8');
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
    const repoContainer = await readRepoArchiveContainer(rootDir);
    const gitState = getGitDocState(rootDir, absoluteDocPath);
    const packed = packDocument(document);
    const sha256 = createHash('sha256').update(packed).digest('hex');
    const writeEntry = (container) => {
        container.documents[archiveMeta.key] = {
            sha256,
            packedBytes: packed.length,
            _packed: packed,
            git: {
                tracked: gitState.tracked,
                untracked: gitState.untracked,
                ignored: gitState.ignored,
                modified: gitState.modified,
                staged: gitState.staged,
            },
        };
    };
    if (!gitState.includeInRepoArchive) {
        let repoTouched = false;
        if (repoContainer.documents[archiveMeta.key]) {
            delete repoContainer.documents[archiveMeta.key];
            repoTouched = true;
        }
        if (repoTouched) {
            await writeRepoArchiveContainer(rootDir, repoContainer);
        }
        const localContainer = await readLocalArchiveContainer(rootDir);
        writeEntry(localContainer);
        const localArtifact = await writeLocalArchiveContainer(rootDir, localContainer);
        return {
            codec: ARCHIVE_CODEC,
            relativePath: localArtifact.relativePath,
            key: archiveMeta.key,
            sha256,
            packedBytes: packed.length,
            repoArtifactBytes: localArtifact.bytes,
            localOnly: true,
        };
    }
    // Restore repo-tracked mode and prune any stale local copy for this key.
    const localContainer = await readLocalArchiveContainer(rootDir);
    if (localContainer.documents[archiveMeta.key]) {
        delete localContainer.documents[archiveMeta.key];
        await writeLocalArchiveContainer(rootDir, localContainer);
    }
    writeEntry(repoContainer);
    const repoArtifact = await writeRepoArchiveContainer(rootDir, repoContainer);
    return {
        codec: ARCHIVE_CODEC,
        relativePath: repoArtifact.relativePath,
        key: archiveMeta.key,
        sha256,
        packedBytes: packed.length,
        repoArtifactBytes: repoArtifact.bytes,
    };
}
export async function readDocArchive(rootDir, absoluteDocPath, explicitRelativeArchivePath = '') {
    const archiveMeta = getDocArchiveMetadata(rootDir, absoluteDocPath);
    const container = await readRepoArchiveContainer(rootDir);
    const key = archiveMeta.key;
    const entry = container.documents && container.documents[key];
    if (entry) {
        // DXBUN2: raw packed bytes stored directly on _packed
        if (entry._packed) {
            const packed = Buffer.isBuffer(entry._packed) ? entry._packed : Buffer.from(entry._packed);
            const unpacked = unpackDocument(packed);
            return normalizeDocInput(absoluteDocPath, unpacked);
        }
        // Legacy DXRA1: payload stored as base64-encoded per-document brotli archive
        if (entry.payload) {
            const archiveBuffer = Buffer.from(String(entry.payload || ''), 'base64');
            return decodeArchivePayload(absoluteDocPath, archiveBuffer);
        }
    }
    const localContainer = await readLocalArchiveContainer(rootDir);
    const localEntry = localContainer.documents && localContainer.documents[key];
    if (localEntry) {
        if (localEntry._packed) {
            const packed = Buffer.isBuffer(localEntry._packed) ? localEntry._packed : Buffer.from(localEntry._packed);
            const unpacked = unpackDocument(packed);
            return normalizeDocInput(absoluteDocPath, unpacked);
        }
        if (localEntry.payload) {
            const archiveBuffer = Buffer.from(String(localEntry.payload || ''), 'base64');
            return decodeArchivePayload(absoluteDocPath, archiveBuffer);
        }
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
