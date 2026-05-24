import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { readFile } from 'node:fs/promises';

import { parseDocFile } from '#runtime-src/doc-format.js';
import { renderDocumentViewHtml } from '#runtime-src/doc-view.js';

const DX_FILES = [
  'documents/browser-check.dx',
  'documents/compact-proof.dx',
  'documents/final-validation.dx',
  'documents/virtual-check.dx',
  'examples/block-reference.dx',
  'examples/compactness-comparison.dx',
  'examples/footprint-pair.dx',
  'examples/tutorial.dx',
  'examples/welcome.dx',
] as const;

async function loadDx(relativePath: string) {
  const absolutePath = path.join(process.cwd(), relativePath);
  const source = await readFile(absolutePath, 'utf8');
  return { absolutePath, source };
}

test('all tracked .dx documents parse and render rich content safely', async () => {
  for (const relativePath of DX_FILES) {
    const { absolutePath, source } = await loadDx(relativePath);

    assert.notEqual(source.trim(), '~', `${relativePath} must not remain a placeholder`);

    const parsed = parseDocFile(absolutePath, source);
    assert.ok(parsed.blocks.length > 0, `${relativePath} should contain blocks`);
    assert.ok(parsed.blocks.some((block) => block.type === 'heading'), `${relativePath} should include a heading`);

    const html = renderDocumentViewHtml({
      title: parsed.title,
      relativePath,
      source,
    });

    assert.match(html, /<main class="page"/);
    assert.equal(html.toLowerCase().includes('<script'), false, `${relativePath} render output must not contain script tags`);

    if (source.includes('language=svg')) {
      assert.match(html, /<svg|class="svg-wrap"/);
    }

    if (source.includes('language=html')) {
      assert.match(html, /<table|class="html-wrap"/);
    }
  }
});

function assertMetricRowsAreConsistent(rows: Array<Record<string, number | string>>, label: string) {
  for (const row of rows) {
    const sample = String(row.sample);
    const mdBytes = Number(row.md_bytes);
    const packedBytes = Number(row.packed_bytes);
    const reductionBytes = Number(row.reduction_bytes);
    const reductionPct = Number(row.reduction_pct);

    assert.equal(reductionBytes, mdBytes - packedBytes, `${label}:${sample} reduction_bytes must equal md_bytes - packed_bytes`);

    const expectedPct = Number((((mdBytes - packedBytes) / mdBytes) * 100).toFixed(2));
    assert.equal(reductionPct, expectedPct, `${label}:${sample} reduction_pct must match derived percentage`);
  }
}

test('compaction metric JSON blocks have correct arithmetic', async () => {
  const metricFiles = [
    'examples/compactness-comparison.dx',
    'examples/footprint-pair.dx',
    'documents/compact-proof.dx',
  ];

  for (const relativePath of metricFiles) {
    const { absolutePath, source } = await loadDx(relativePath);
    const parsed = parseDocFile(absolutePath, source);
    const metricBlock = parsed.blocks.find((block) => {
      return block.type === 'code' && String(block.language || '').toLowerCase() === 'json';
    });

    assert.ok(metricBlock, `${relativePath} should include a json metrics block`);

    const rows = JSON.parse(String(metricBlock?.text || '[]')) as Array<Record<string, number | string>>;
    assert.ok(rows.length > 0, `${relativePath} metrics must include at least one row`);
    assertMetricRowsAreConsistent(rows, relativePath);
  }
});
