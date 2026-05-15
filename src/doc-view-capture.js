import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { renderDocumentViewHtml } from './doc-view.js';

const execFileAsync = promisify(execFile);

function sanitizeStem(value) {
  const stem = String(value || 'document')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return stem || 'document';
}

/**
 * Render document view HTML and capture a PNG screenshot using macOS Quick Look.
 */
export async function captureDocumentViewPng(document, { size = 1400 } = {}) {
  if (process.platform !== 'darwin') {
    throw new Error('capture-document-view currently supports macOS only (requires qlmanage).');
  }

  const tempDir = await mkdtemp(path.join(os.tmpdir(), 'docview-capture-'));
  const stem = sanitizeStem(document?.title || document?.relativePath || 'document');
  const htmlPath = path.join(tempDir, `${stem}.html`);

  try {
    const html = renderDocumentViewHtml(document);
    await writeFile(htmlPath, html, 'utf8');

    await execFileAsync('qlmanage', ['-t', '-s', String(Math.max(400, Number(size) || 1400)), '-o', tempDir, htmlPath]);

    const entries = await readdir(tempDir);
    const pngName = entries.find((name) => name.toLowerCase().endsWith('.png'));

    if (!pngName) {
      throw new Error('Quick Look did not produce a PNG capture.');
    }

    const pngPath = path.join(tempDir, pngName);
    const bytes = await readFile(pngPath);

    return {
      mimeType: 'image/png',
      base64: bytes.toString('base64'),
      bytes: bytes.length,
    };
  } finally {
    await rm(tempDir, { recursive: true, force: true });
  }
}