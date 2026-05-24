import test from 'node:test';
import assert from 'node:assert/strict';
import { computeSourceHash, mergeDocumentViewState, normalizeDocumentViewState } from '#runtime-src/view-state.js';

test('normalizeDocumentViewState applies defensive defaults and clamps nested fields', () => {
  const normalized = normalizeDocumentViewState({
    theme: 'bad-value',
    resolvedTheme: 'shadow',
    appearance: { paper: 'cream', density: 'dense', scale: 999 },
    viewport: { width: -10, height: 720.8, pixelRatio: 0.5, zoomLevel: 1 },
    effectiveCss: 42,
    sourceHash: null,
    editBuffer: null,
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
  assert.equal(normalized.sourceHash, '');
  assert.equal(normalized.editBuffer, '');
});

test('mergeDocumentViewState applies partial patch without dropping existing settings', () => {
  const base = normalizeDocumentViewState({
    theme: 'dark',
    resolvedTheme: 'dark',
    appearance: { paper: 'slate', density: 'compact', scale: 96 },
    viewport: { width: 1440, height: 900, pixelRatio: 2, zoomLevel: 0, zoomFactor: 1 },
    effectiveCss: '.old { color: red; }',
    sourceHash: 'old-hash',
    editBuffer: '::paragraph\nold\n::end\n',
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
  assert.equal(merged.sourceHash, 'old-hash');
  assert.equal(merged.editBuffer, '::paragraph\nold\n::end\n');
});

test('normalizeDocumentViewState maps legacy sourceText to compact editBuffer', () => {
  const normalized = normalizeDocumentViewState({
    sourceText: 'legacy-source',
  });

  assert.equal(normalized.editBuffer, 'legacy-source');
  assert.equal(normalized.sourceHash, '');
});

test('computeSourceHash returns stable sha256 hash for source text', () => {
  const source = '::paragraph\nhello\n::end\n';
  assert.equal(computeSourceHash(source), computeSourceHash(source));
  assert.notEqual(computeSourceHash(source), computeSourceHash('different'));
});
