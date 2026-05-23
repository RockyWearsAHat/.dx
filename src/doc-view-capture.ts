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

function toFiniteNumber(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function resolveViewport({ size = 1000, viewState = null } = {}) {
  const stateViewport = viewState && typeof viewState === 'object' ? viewState.viewport : null;
  const stateWidth = toFiniteNumber(stateViewport?.width);
  const stateHeight = toFiniteNumber(stateViewport?.height);
  const statePixelRatio = toFiniteNumber(stateViewport?.pixelRatio);

  const width = clamp(Math.round(stateWidth || toFiniteNumber(size) || 1000), 400, 4096);
  const aspectRatio = stateHeight && stateWidth && stateWidth > 0
    ? stateHeight / stateWidth
    : 0.613;
  const height = clamp(Math.round(stateHeight || (width * aspectRatio)), 500, 8192);
  const deviceScaleFactor = clamp(statePixelRatio || 2, 1, 3);

  return { width, height, deviceScaleFactor };
}

async function captureRenderedWithPlaywright(html, { size = 1000, viewState = null } = {}) {
  const { chromium } = await import('playwright');
  const { width, height, deviceScaleFactor } = resolveViewport({ size, viewState });
  const browser = await chromium.launch({ headless: true });

  try {
    const page = await browser.newPage({
      viewport: { width, height },
      deviceScaleFactor,
    });

    await page.setContent(String(html || ''), { waitUntil: 'networkidle' });

    // Match the visible editor state by waiting for runtime assets to settle.
    await page.evaluate(async () => {
      if (document.fonts && typeof document.fonts.ready?.then === 'function') {
        try {
          await document.fonts.ready;
        } catch {
        }
      }

      const images = Array.from(document.images || []);
      await Promise.all(images.map((image) => {
        if (image.complete) {
          return null;
        }

        return new Promise((resolve) => {
          const done = () => resolve();
          image.addEventListener('load', done, { once: true });
          image.addEventListener('error', done, { once: true });
        });
      }));
    });

    // Give layout a beat to settle after CSS variables/theme attributes apply.
    await page.waitForTimeout(150);

    const bytes = await page.screenshot({ type: 'png', fullPage: true });

    return {
      mimeType: 'image/png',
      base64: bytes.toString('base64'),
      bytes: bytes.length,
      mode: 'rendered',
      engine: 'playwright-chromium',
      viewport: { width, height, deviceScaleFactor },
    };
  } finally {
    await browser.close();
  }
}

async function captureRenderedWithQuickLook(html, { size = 1000, stem = 'document' } = {}) {
  if (process.platform !== 'darwin') {
    throw new Error('Rendered fallback capture currently supports macOS only (requires qlmanage).');
  }

  const tempDir = await mkdtemp(path.join(os.tmpdir(), 'docview-capture-'));
  const htmlPath = path.join(tempDir, `${stem}.html`);

  try {
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
      mode: 'rendered',
      engine: 'quicklook-fallback',
    };
  } finally {
    await rm(tempDir, { recursive: true, force: true });
  }
}

/**
 * Capture a rendered .dx document surface as a PNG.
 * Primary path: Playwright Chromium (closest to VS Code webview rendering).
 * Fallback: Quick Look on macOS if Playwright is unavailable.
 */
export async function captureDocumentViewPng(document, { size = 1000, viewState = null } = {}) {
  const stem = sanitizeStem(document?.title || document?.relativePath || 'document');
  const hasInlineSource = String(document?.source || '').trim().length > 0;
  const captureDocument = hasInlineSource
    ? document
    : (viewState?.sourceText ? { ...document, source: String(viewState.sourceText || '') } : document);

  const html = renderDocumentViewHtml(captureDocument, {
    theme: viewState?.theme || 'auto',
    resolvedTheme: viewState?.resolvedTheme || 'dark',
    appearance: viewState?.appearance || null,
    effectiveCss: viewState?.effectiveCss || '',
  });

  try {
    return await captureRenderedWithPlaywright(html, { size, viewState });
  } catch {
    return captureRenderedWithQuickLook(html, { size, stem });
  }
}