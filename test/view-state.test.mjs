import test from 'node:test';
import assert from 'node:assert/strict';
import { createDatabase, upsertDocument } from '../src/database.js';
import { normalizeDocInput } from '../src/doc-format.js';
import { mergeDocumentViewState, normalizeDocumentViewState, readDocumentViewState } from '../src/view-state.js';
import { cleanupTempWorkspace, createTempWorkspace } from './test-utils.mjs';

test('normalizeDocumentViewState applies defensive defaults and clamps nested fields', () => {
  const normalized = normalizeDocumentViewState({
    theme: 'bad-value',
    resolvedTheme: 'shadow',
    appearance: { paper: 'cream', density: 'dense', scale: 999 },
    viewport: { width: -10, height: 720.8, pixelRatio: 0.5, zoomLevel: 1 },
    effectiveCss: 42,
    sourceText: null,
  });

  assert.equal(normalized.theme, 'auto');
  assert.equal(normalized.resolvedTheme, 'dark');
  assert.equal(normalized.appearance.paper, 'cream');
  assert.equal(normalized.appearance.density, 'comfortable');
  assert.equal(normalized.appearance.scale, 115);
  assert.equal(normalized.viewport.width, null);
  assert.equal(normalized.viewport.height, 721);
  assert.equal(normalized.viewport.pixelRatio, 0.5);
  assert.equal(normalized.effectiveCss, '42');
  assert.equal(normalized.sourceText, '');
});

test('mergeDocumentViewState applies partial patch without dropping existing settings', () => {
  const base = normalizeDocumentViewState({
    theme: 'dark',
    resolvedTheme: 'dark',
    appearance: { paper: 'slate', density: 'compact', scale: 96 },
    viewport: { width: 1440, height: 900, pixelRatio: 2, zoomLevel: 0, zoomFactor: 1 },
    effectiveCss: '.old { color: red; }',
    sourceText: '::paragraph\nold\n::end\n',
  });

  const merged = mergeDocumentViewState(base, {
    theme: 'light',
    appearance: { scale: 110 },
    viewport: { width: 1280 },
  });

  assert.equal(merged.theme, 'light');
  assert.equal(merged.resolvedTheme, 'dark');
  assert.equal(merged.appearance.paper, 'slate');
  assert.equal(merged.appearance.density, 'compact');
  assert.equal(merged.appearance.scale, 110);
  assert.equal(merged.viewport.width, 1280);
  assert.equal(merged.viewport.height, 900);
  assert.equal(merged.effectiveCss, '.old { color: red; }');
  assert.equal(merged.sourceText, '::paragraph\nold\n::end\n');
});

test('readDocumentViewState sanitizes stored view state fields', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-view-state-tests-');

  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/examples/x.dx`, {
      title: 'x',
      blocks: [{ type: 'paragraph', text: 'ok' }],
    });

    const id = upsertDocument(db, rootDir, doc, Date.now());

    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run(JSON.stringify({
      theme: 'invalid',
      resolvedTheme: 'invalid',
      appearance: { paper: 'cream', density: 'compact', scale: 130 },
      viewport: { width: -1, height: 800.2, pixelRatio: 0, zoomLevel: 2 },
      effectiveCss: '.a{}',
      sourceText: '::paragraph id=p\ntext\n::end\n',
    }), id);

    const state = readDocumentViewState(db, id);
    assert.ok(state);
    assert.equal(state.theme, 'auto');
    assert.equal(state.resolvedTheme, 'dark');
    assert.equal(state.appearance.paper, 'cream');
    assert.equal(state.appearance.density, 'compact');
    assert.equal(state.appearance.scale, 115);
    assert.equal(state.viewport.width, null);
    assert.equal(state.viewport.height, 800);
    assert.equal(state.viewport.pixelRatio, null);
    assert.equal(state.viewport.zoomLevel, 2);
    assert.ok(state.viewport.zoomFactor > 1);

    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run('{bad', id);
    assert.equal(readDocumentViewState(db, id), null);

    assert.equal(readDocumentViewState(db, 999999), null);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('readDocumentViewState uses auto/dark defaults when theme fields absent', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('doc-view-state-defaults-');
  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/defaults.dx`, {
      title: 'defaults',
      blocks: [{ type: 'paragraph', text: 'ok' }],
    });
    const id = upsertDocument(db, rootDir, doc, Date.now());
    // Store JSON with no theme or resolvedTheme fields → triggers || 'auto' and || 'dark'
    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run(
      JSON.stringify({ sourceText: '', effectiveCss: '' }),
      id
    );
    const state = readDocumentViewState(db, id);
    assert.ok(state);
    assert.equal(state.theme, 'auto');
    assert.equal(state.resolvedTheme, 'dark');
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});
