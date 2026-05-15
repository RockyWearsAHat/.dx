import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { renderDocumentViewHtml } from './doc-view.js';

const execFileAsync = promisify(execFile);

function parseAppleScriptList(output) {
  const values = String(output || '')
    .trim()
    .split(',')
    .map((part) => Number(part.trim()))
    .filter((value) => Number.isFinite(value));

  if (values.length < 2) {
    throw new Error(`Unable to parse AppleScript output: ${output}`);
  }

  return values;
}

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
export async function captureDocumentViewPng(document, { size = 1400, viewState = null } = {}) {
  if (process.platform !== 'darwin') {
    throw new Error('capture-document-view currently supports macOS only (requires qlmanage).');
  }

  const tempDir = await mkdtemp(path.join(os.tmpdir(), 'docview-capture-'));
  const stem = sanitizeStem(document?.title || document?.relativePath || 'document');
  const htmlPath = path.join(tempDir, `${stem}.html`);

  try {
    const captureDocument = viewState?.sourceText
      ? { ...document, source: String(viewState.sourceText || '') }
      : document;

    const html = renderDocumentViewHtml(captureDocument, {
      theme: viewState?.theme || 'auto',
      resolvedTheme: viewState?.resolvedTheme || 'dark',
      appearance: viewState?.appearance || null,
      effectiveCss: viewState?.effectiveCss || '',
    });
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

/**
 * Capture the visible VS Code window itself (user view) on macOS.
 */
export async function captureWindowViewPng({ appName = 'Code - Insiders' } = {}) {
  if (process.platform !== 'darwin') {
    throw new Error('window capture currently supports macOS only.');
  }

  const tempDir = await mkdtemp(path.join(os.tmpdir(), 'docview-window-capture-'));
  const pngPath = path.join(tempDir, 'window-capture.png');

  try {
    const cleanedAppName = String(appName).replace(/"/g, '');
    const windowIdScript = `tell application "System Events" to tell process "${cleanedAppName}" to get value of attribute "AXWindowNumber" of front window`;
    const posScript = `tell application "System Events" to tell process "${String(appName).replace(/"/g, '')}" to get position of front window`;
    const sizeScript = `tell application "System Events" to tell process "${String(appName).replace(/"/g, '')}" to get size of front window`;

    let captured = false;

    try {
      const idResult = await execFileAsync('osascript', ['-e', windowIdScript]);
      const windowId = Number(String(idResult.stdout || '').trim());
      if (Number.isFinite(windowId) && windowId > 0) {
        await execFileAsync('screencapture', ['-x', '-l', String(Math.trunc(windowId)), pngPath]);
        captured = true;
      }
    } catch {
      captured = false;
    }

    if (!captured) {
      try {
        const posResult = await execFileAsync('osascript', ['-e', posScript]);
        const sizeResult = await execFileAsync('osascript', ['-e', sizeScript]);
        const [x, y] = parseAppleScriptList(posResult.stdout);
        const [width, height] = parseAppleScriptList(sizeResult.stdout);

        await execFileAsync('screencapture', ['-x', '-R', `${Math.max(0, x)},${Math.max(0, y)},${Math.max(1, width)},${Math.max(1, height)}`, pngPath]);
        captured = true;
      } catch {
        captured = false;
      }
    }

    if (!captured) {
      // Last-resort fallback: full-screen capture still represents real user view.
      await execFileAsync('screencapture', ['-x', pngPath]);
    }

    const bytes = await readFile(pngPath);
    return {
      mimeType: 'image/png',
      base64: bytes.toString('base64'),
      bytes: bytes.length,
      mode: 'window',
      appName,
    };
  } finally {
    await rm(tempDir, { recursive: true, force: true });
  }
}