import test from 'node:test';
import assert from 'node:assert/strict';
import { packDocument, unpackDocument } from '../src/doc-binary.js';

test('doc-binary pack/unpack roundtrip preserves blocks and metadata', () => {
  const doc = {
    title: 'Binary Doc',
    summary: 'Roundtrip test',
    tags: ['alpha', 'beta'],
    meta: { owner: 'alex', count: 2 },
    blocks: [
      { type: 'heading', id: 'h-1', level: 2, text: 'Heading' },
      { type: 'paragraph', id: 'p-1', text: 'Paragraph text' },
      { type: 'bulleted-list', id: 'l-1', items: ['a', 'b'] },
      { type: 'numbered-list', id: 'l-2', items: ['1', '2'] },
      { type: 'quote', id: 'q-1', text: 'Quote' },
      { type: 'code', id: 'c-1', language: 'js', text: 'const a = 1;' },
      { type: 'image', id: 'i-1', src: 'x.png', alt: 'an image' },
      { type: 'rule', id: 'r-1' },
      { type: 'checklist', id: 'cl-1', items: [{ checked: true, text: 'done' }, { checked: false, text: 'todo' }] },
    ],
  };

  const packed = packDocument(doc);
  const unpacked = unpackDocument(packed);

  assert.equal(unpacked.title, doc.title);
  assert.equal(unpacked.summary, doc.summary);
  assert.deepEqual(unpacked.tags, doc.tags);
  assert.deepEqual(unpacked.meta, doc.meta);
  assert.deepEqual(unpacked.blocks, doc.blocks);
});

test('doc-binary rejects invalid blob header', () => {
  assert.throws(() => unpackDocument(Buffer.from('not-a-doc-binary')), /valid DOC binary payload/);
});

test('doc-binary handles fallback values for optional fields and non-array inputs', () => {
  const packed = packDocument({
    title: 'Edge',
    // Exercise tags||[] and meta||{} fallbacks in packDocument.
    tags: undefined,
    meta: null,
    // Exercise Array.isArray(document.blocks) false branch.
    blocks: null,
  });

  const unpacked = unpackDocument(new Uint8Array(packed));
  assert.equal(unpacked.title, 'Edge');
  assert.deepEqual(unpacked.tags, []);
  assert.deepEqual(unpacked.meta, {});
  assert.deepEqual(unpacked.blocks, []);
});

test('doc-binary normalizes unknown/missing block fields during roundtrip', () => {
  const doc = {
    title: 'Fallback Blocks',
    summary: '',
    tags: null,
    meta: undefined,
    blocks: [
      // Unknown type falls back to paragraph; id/text fallback to ''.
      { type: 'mystery' },
      // Heading level/text fallback.
      { type: 'heading', id: null, level: 0, text: null },
      // List items fallback from non-array.
      { type: 'bulleted-list', id: null, items: null },
      // Code/image fallback fields.
      { type: 'code', id: null, language: null, text: null },
      { type: 'image', id: null, src: null, alt: null },
      // Checklist fallback from non-array and non-object items.
      { type: 'checklist', id: null, items: [null, 'raw-item'] },
    ],
  };

  const unpacked = unpackDocument(packDocument(doc));

  assert.deepEqual(unpacked.blocks[0], { type: 'paragraph', id: '', text: '' });
  assert.deepEqual(unpacked.blocks[1], { type: 'heading', id: '', level: 1, text: '' });
  assert.deepEqual(unpacked.blocks[2], { type: 'bulleted-list', id: '', items: [] });
  assert.deepEqual(unpacked.blocks[3], { type: 'code', id: '', language: '', text: '' });
  assert.deepEqual(unpacked.blocks[4], { type: 'image', id: '', src: '', alt: '' });
  assert.deepEqual(unpacked.blocks[5], {
    type: 'checklist',
    id: '',
    items: [
      { checked: false, text: '' },
      { checked: false, text: 'raw-item' },
    ],
  });
});

test('doc-binary checklist encoder handles null items and object items with null text', () => {
  const doc = {
    title: 'Checklist Edge',
    summary: '',
    tags: [],
    meta: {},
    blocks: [
      { type: 'checklist', id: 'a', items: null },
      { type: 'checklist', id: 'b', items: [{ checked: true, text: null }] },
    ],
  };

  const unpacked = unpackDocument(packDocument(doc));
  assert.deepEqual(unpacked.blocks[0], { type: 'checklist', id: 'a', items: [] });
  assert.deepEqual(unpacked.blocks[1], {
    type: 'checklist',
    id: 'b',
    items: [{ checked: true, text: '' }],
  });
});
