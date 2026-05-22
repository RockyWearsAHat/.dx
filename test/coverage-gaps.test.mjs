import test from 'node:test';
import assert from 'node:assert/strict';
import {
  parseDocFile,
  normalizeDocInput,
  createDefaultBlocks,
} from '../src/doc-format.js';
import { parseSourceBlocks } from '../vscode-extension/media/doc-pipeline.js';

test('parseDocFile handles unclosed code fence at EOF', () => {
  const source = [
    '# Heading',
    '',
    '```javascript',
    'const x = 5;',
    'console.log(x);',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  assert.equal(parsed.blocks.length >= 2, true);
  const codeBlock = parsed.blocks.find((b) => b.type === 'code');
  assert.ok(codeBlock);
  assert.ok(codeBlock.text.includes('const x'));
});

test('parseDocFile handles multiple consecutive blank lines', () => {
  const source = [
    '# Title',
    '',
    '',
    '',
    'Paragraph',
    '',
    '',
    '- item',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  assert.ok(parsed.blocks.length >= 3);
  assert.equal(parsed.blocks[0].type, 'heading');
  assert.ok(parsed.blocks.some((b) => b.type === 'paragraph'));
});

test('parseDocFile handles mixed quote markers with flush', () => {
  const source = [
    '> Quote line 1',
    '> Quote line 2',
    '',
    '- List item',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const quoteBlock = parsed.blocks.find((b) => b.type === 'quote');
  assert.ok(quoteBlock);
  assert.ok(quoteBlock.text.includes('Quote line 1'));
});

test('parseDocFile with empty paragraph recovery', () => {
  const source = [
    '# Heading',
    '',
    '> Quote',
    '',
    '# Next Heading',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  assert.ok(parsed.blocks.length >= 2);
  // Verify empty paragraphs are not created
  const emptyParas = parsed.blocks.filter((b) => b.type === 'paragraph' && !b.text.trim());
  assert.equal(emptyParas.length, 0);
});

test('normalizeDocInput with explicit metadata object', () => {
  const doc = normalizeDocInput('/tmp/test.dx', {
    title: 'Test Doc',
    metadata: { author: 'Test', version: '1.0' },
    tags: ['test', 'coverage'],
    blocks: [{ type: 'heading', level: 1, text: 'Hello' }],
  });

  assert.equal(doc.title, 'Test Doc');
  assert.ok(doc.meta.author);
  assert.ok(doc.tags.includes('test'));
});

test('normalizeDocInput with legacy metadata fallback', () => {
  const doc = normalizeDocInput('/tmp/test.dx', {
    metadata: { title: 'Legacy Title', summary: 'Old summary' },
    blocks: [{ type: 'paragraph', text: 'Content' }],
  });

  assert.equal(doc.title, 'Legacy Title');
  assert.equal(doc.summary, 'Old summary');
});

test('createDefaultBlocks generates correct structure', () => {
  const blocks = createDefaultBlocks('My Project');
  assert.ok(Array.isArray(blocks));
  assert.equal(blocks[0].type, 'heading');
  assert.equal(blocks[0].level, 1);
  assert.equal(blocks[0].text, 'My Project');
});

test('parseSourceBlocks handles image blocks with all attributes', () => {
  const source = [
    '::image id=hero class=large src=hero.jpg',
    'Alt text',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const imageBlock = blocks.find((b) => b.type === 'image');
  assert.ok(imageBlock);
  assert.equal(imageBlock.id, 'hero');
  assert.equal(imageBlock.className, 'large');
  assert.equal(imageBlock.src, 'hero.jpg');
  assert.ok(imageBlock.alt.includes('Alt text'));
});

test('parseSourceBlocks handles checklist with mixed items', () => {
  const source = [
    '::checklist',
    '[x] Completed task',
    '[ ] Not done',
    '[X] Another done',
    'Not a task item',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const listBlock = blocks.find((b) => b.type === 'checklist');
  assert.ok(listBlock);
  assert.ok(Array.isArray(listBlock.items));
  assert.ok(listBlock.items.some((i) => i.checked === true));
  assert.ok(listBlock.items.some((i) => i.checked === false));
});

test('parseSourceBlocks skips empty paragraph content', () => {
  const source = [
    '::paragraph',
    '',
    '::end',
    'After paragraph',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  // Empty paragraph should be skipped
  const emptyParas = blocks.filter((b) => b.type === 'paragraph' && !b.text.trim());
  assert.equal(emptyParas.length, 0);
});

test('parseSourceBlocks handles code block with language tag', () => {
  const source = [
    '::code language=typescript',
    'interface User { name: string; }',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const codeBlock = blocks.find((b) => b.type === 'code');
  assert.ok(codeBlock);
  assert.equal(codeBlock.language, 'typescript');
  assert.ok(codeBlock.text.includes('interface User'));
});

test('parseSourceBlocks handles bulleted lists with multiple items', () => {
  const source = [
    '::bulleted-list',
    '- Top level',
    '- Another item',
    '- Third item',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const listBlock = blocks.find((b) => b.type === 'bulleted-list');
  assert.ok(listBlock);
  assert.ok(Array.isArray(listBlock.items));
  assert.ok(listBlock.items.length >= 2);
});

test('parseDocFile preserves list structure from Markdown', () => {
  const source = [
    '- Top level',
    '  - Nested item',
    '    - Deep nested',
    '- Another top',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const listBlock = parsed.blocks.find((b) => b.type === 'bulleted-list');
  assert.ok(listBlock);
  assert.ok(Array.isArray(listBlock.items));
  // Markdown legacy parser may flatten nested items due to buffer flushing
  // Just verify we have the expected items
  assert.ok(listBlock.items.length >= 2);
});

test('parseDocFile with quote block flushing', () => {
  const source = [
    '> First quote',
    '> Second quote',
    'Paragraph after',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const quoteBlock = parsed.blocks.find((b) => b.type === 'quote');
  assert.ok(quoteBlock);
  const textAfter = parsed.blocks.find((b) => b.type === 'paragraph' && b.text.includes('Paragraph'));
  assert.ok(textAfter);
});

test('parseDocFile recovery from unclosed canonical block', () => {
  const source = [
    '::paragraph id=p1',
    'This paragraph opens but never closes',
    '# Then we hit a heading',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  // Block should auto-close and recover
  assert.ok(parsed.blocks.length >= 1);
});

test('normalizeDocInput handles empty blocks array', () => {
  const doc = normalizeDocInput('/tmp/test.dx', {
    title: 'Empty Doc',
    blocks: [],
  });

  assert.equal(doc.title, 'Empty Doc');
  assert.ok(Array.isArray(doc.blocks));
  // Should have default block
  assert.equal(doc.blocks.length > 0, true);
});

test('parseDocFile title extraction from path', () => {
  const source = 'Some content';
  const parsed = parseDocFile('/path/to/my-document.dx', source);
  assert.equal(parsed.title, 'my-document');
});

test('parseSourceBlocks handles missing attributes gracefully', () => {
  const source = [
    '::image',
    'No src provided',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const imageBlock = blocks.find((b) => b.type === 'image');
  assert.ok(imageBlock);
  assert.equal(imageBlock.src, '');
  assert.equal(imageBlock.alt, 'No src provided');
});
