import { mkdir, readFile, stat, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { getLegacyCompanionArchivePath, readDocArchive, writeDocArchive } from './doc-archive.js';
import { getDocumentByPath, getWorkspaceDocumentById, searchDocuments, upsertDocument } from './database.js';
import { createDefaultBlocks, normalizeDocInput, parseDocFile } from './doc-format.js';
import { findDocFiles } from './file-discovery.js';
const DOC_STUB_VERSION = 3;
const DOC_STUB_PREFIX = '@docstub';
function assertWithinRoot(rootDir, absolutePath) {
    const relative = path.relative(rootDir, absolutePath);
    if (relative.startsWith('..') || path.isAbsolute(relative)) {
        throw new Error('Document path must stay inside the workspace root.');
    }
    return relative || '.';
}
function toClientDocument(rootDir, document) {
    return {
        ...document,
        relativePath: path.relative(rootDir, document.path),
    };
}
function parseStubTarget(rootDir, currentPath, sourceText) {
    // In production flows sourceText is always provided by readFile.
    const lines = String(sourceText || '').split('\n').map((line) => line.trim());
    if (!lines[0] || !lines[0].startsWith(DOC_STUB_PREFIX)) {
        return null;
    }
    const prefixParts = lines[0].split(/\s+/);
    const version = Number(prefixParts[1] || DOC_STUB_VERSION);
    const pathLine = lines.find((line) => line.startsWith('path:'));
    const archiveLine = lines.find((line) => line.startsWith('archive:'));
    const artifactLine = lines.find((line) => line.startsWith('artifact:'));
    const keyLine = lines.find((line) => line.startsWith('key:'));
    const codecLine = lines.find((line) => line.startsWith('codec:'));
    const shaLine = lines.find((line) => line.startsWith('sha256:'));
    const packedBytesLine = lines.find((line) => line.startsWith('packed_bytes:'));
    const archiveBytesLine = lines.find((line) => line.startsWith('archive_bytes:'));
    const relativeFromStub = pathLine ? pathLine.slice(5).trim() : path.relative(rootDir, currentPath);
    const absolutePath = path.resolve(rootDir, relativeFromStub);
    assertWithinRoot(rootDir, absolutePath);
    return {
        version,
        absolutePath,
        artifactRelativePath: artifactLine ? artifactLine.slice(9).trim() : '',
        key: keyLine ? keyLine.slice(4).trim() : '',
        archiveRelativePath: archiveLine ? archiveLine.slice(8).trim() : '',
        codec: codecLine ? codecLine.slice(6).trim() : '',
        sha256: shaLine ? shaLine.slice(7).trim() : '',
        packedBytes: packedBytesLine ? Number(packedBytesLine.slice(13).trim()) : 0,
        archiveBytes: archiveBytesLine ? Number(archiveBytesLine.slice(14).trim()) : 0,
    };
}
function buildDocStub(rootDir, absolutePath, archiveInfo = null) {
    const relativePath = assertWithinRoot(rootDir, absolutePath);
    const lines = [
        `${DOC_STUB_PREFIX} ${DOC_STUB_VERSION}`,
        `path: ${relativePath}`,
    ];
    if (archiveInfo) {
        lines.push(`artifact: ${archiveInfo.relativePath}`);
        lines.push(`key: ${archiveInfo.key}`);
        lines.push(`archive: ${archiveInfo.relativePath}`);
        lines.push(`codec: ${archiveInfo.codec}`);
        lines.push(`sha256: ${archiveInfo.sha256}`);
        lines.push(`packed_bytes: ${archiveInfo.packedBytes}`);
        lines.push(`archive_bytes: ${archiveInfo.archiveBytes}`);
    }
    return `${lines.join('\n')}\n`;
}
async function writeDocStub(rootDir, absolutePath, archiveInfo = null) {
    await mkdir(path.dirname(absolutePath), { recursive: true });
    await writeFile(absolutePath, buildDocStub(rootDir, absolutePath, archiveInfo), 'utf8');
}
async function persistDocumentArtifacts(rootDir, absolutePath, document) {
    const archiveInfo = await writeDocArchive(rootDir, absolutePath, document);
    await writeDocStub(rootDir, absolutePath, archiveInfo);
    const legacyCompanion = getLegacyCompanionArchivePath(absolutePath);
    try {
        await unlink(legacyCompanion);
    }
    catch {
        // Ignore missing legacy companions.
    }
}
function normalizeRelativeDocPath(relativePath) {
    // Public APIs validate input before calling this helper.
    const trimmed = String(relativePath || '').trim().replace(/^\/+/, '');
    if (!trimmed || !trimmed.endsWith('.dx')) {
        throw new Error('A valid .dx path is required.');
    }
    return trimmed;
}
function resolveDocumentPath(rootDir, relativePath) {
    const absolutePath = path.resolve(rootDir, normalizeRelativeDocPath(relativePath));
    assertWithinRoot(rootDir, absolutePath);
    return absolutePath;
}
export async function ingestWorkspace(rootDir, db) {
    const docFiles = await findDocFiles(rootDir);
    const ingested = [];
    for (const filePath of docFiles) {
        const [text, fileStats] = await Promise.all([
            readFile(filePath, 'utf8'),
            stat(filePath),
        ]);
        const stub = parseStubTarget(rootDir, filePath, text);
        if (stub) {
            const existing = getDocumentByPath(db, rootDir, stub.absolutePath);
            if (existing) {
                // Existing hydrated documents should always have source text.
                const fromDbSource = parseDocFile(existing.path, existing.source || '');
                const normalizedId = upsertDocument(db, rootDir, fromDbSource, Math.trunc(fileStats.mtimeMs));
                await persistDocumentArtifacts(rootDir, stub.absolutePath, fromDbSource);
                const normalized = getWorkspaceDocumentById(db, rootDir, normalizedId);
                ingested.push(toClientDocument(rootDir, normalized));
                continue;
            }
            if (stub.archiveRelativePath) {
                try {
                    const reconstructed = await readDocArchive(rootDir, stub.absolutePath, stub.archiveRelativePath);
                    const documentId = upsertDocument(db, rootDir, reconstructed, Math.trunc(fileStats.mtimeMs));
                    await persistDocumentArtifacts(rootDir, stub.absolutePath, reconstructed);
                    const hydrated = getWorkspaceDocumentById(db, rootDir, documentId);
                    ingested.push(toClientDocument(rootDir, hydrated));
                }
                catch {
                    // Leave unresolved stubs untouched; they can be repaired from another peer.
                }
            }
            continue;
        }
        const parsed = parseDocFile(filePath, text);
        const documentId = upsertDocument(db, rootDir, parsed, Math.trunc(fileStats.mtimeMs));
        await persistDocumentArtifacts(rootDir, filePath, parsed);
        const hydrated = getWorkspaceDocumentById(db, rootDir, documentId);
        ingested.push(toClientDocument(rootDir, hydrated));
    }
    return ingested;
}
export async function createDocument(rootDir, db, input) {
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
    const documentId = upsertDocument(db, rootDir, document, Date.now());
    await persistDocumentArtifacts(rootDir, absolutePath, document);
    return toClientDocument(rootDir, getWorkspaceDocumentById(db, rootDir, documentId));
}
export async function saveDocument(rootDir, db, documentId, input) {
    const current = getWorkspaceDocumentById(db, rootDir, documentId);
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
    upsertDocument(db, rootDir, document, Date.now());
    await persistDocumentArtifacts(rootDir, current.path, document);
    return toClientDocument(rootDir, getWorkspaceDocumentById(db, rootDir, documentId));
}
export async function getDocumentByRelativePath(rootDir, db, relativePath) {
    const absolutePath = resolveDocumentPath(rootDir, relativePath);
    const document = getDocumentByPath(db, rootDir, absolutePath);
    return document ? toClientDocument(rootDir, document) : null;
}
export async function saveDocumentSourceByRelativePath(rootDir, db, relativePath, sourceText) {
    const absolutePath = resolveDocumentPath(rootDir, relativePath);
    const parsed = parseDocFile(absolutePath, String(sourceText || ''));
    const documentId = upsertDocument(db, rootDir, parsed, Date.now());
    await persistDocumentArtifacts(rootDir, absolutePath, parsed);
    return toClientDocument(rootDir, getWorkspaceDocumentById(db, rootDir, documentId));
}
/**
 * Saves document source text to SQLite and the binary archive, then returns
 * the stub pointer text that should be written to the on-disk .dx file.
 * The caller is responsible for writing the stub to disk; this function never
 * touches the filesystem directly so there is no double-write race.
 */
export async function saveDocumentSourceToDbAndArchive(rootDir, db, relativePath, sourceText) {
    const absolutePath = resolveDocumentPath(rootDir, relativePath);
    const parsed = parseDocFile(absolutePath, String(sourceText || ''));
    const documentId = upsertDocument(db, rootDir, parsed, Date.now());
    const archiveInfo = await writeDocArchive(rootDir, absolutePath, parsed);
    const stubText = buildDocStub(rootDir, absolutePath, archiveInfo);
    return {
        document: toClientDocument(rootDir, getWorkspaceDocumentById(db, rootDir, documentId)),
        stubText,
    };
}
export async function getDocument(rootDir, db, documentId) {
    const document = getWorkspaceDocumentById(db, rootDir, documentId);
    return document ? toClientDocument(rootDir, document) : null;
}
export async function listOrSearchDocuments(rootDir, db, query = '') {
    return searchDocuments(db, rootDir, query).map((document) => toClientDocument(rootDir, document));
}
export async function reconstructDocument(rootDir, db, documentId) {
    const document = getWorkspaceDocumentById(db, rootDir, documentId);
    if (!document) {
        throw new Error('Document not found.');
    }
    assertWithinRoot(rootDir, document.path);
    return document.source;
}
