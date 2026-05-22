import test from 'node:test';
import assert from 'node:assert/strict';
import { parseDocFile, normalizeDocInput } from '../src/doc-format.js';
import { parseSourceBlocks } from '../vscode-extension/media/doc-pipeline.js';

test('parseDocFile handles unclosed quote at end of file', () => {
  const source = [
    '# Title',
    '',
    '> This is a quote',
    '> That continues',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const quoteBlock = parsed.blocks.find((b) => b.type === 'quote');
  assert.ok(quoteBlock);
  assert.ok(quoteBlock.text.includes('This is a quote'));
  assert.ok(quoteBlock.text.includes('That continues'));
});

test('parseDocFile handles transition from quote to paragraph', () => {
  const source = [
    '> Quote line',
    'Paragraph text',
    '> Another quote',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const quoteBlocks = parsed.blocks.filter((b) => b.type === 'quote');
  assert.ok(quoteBlocks.length >= 1);
  const paraBlock = parsed.blocks.find((b) => b.type === 'paragraph' && b.text === 'Paragraph text');
  assert.ok(paraBlock);
});

test('parseDocFile handles list with quote transition', () => {
  const source = [
    '- List item',
    '- Another item',
    '',
    '> Quote after list',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const listBlock = parsed.blocks.find((b) => b.type === 'bulleted-list');
  const quoteBlock = parsed.blocks.find((b) => b.type === 'quote');
  assert.ok(listBlock);
  assert.ok(quoteBlock);
});

test('parseSourceBlocks handles quote blocks', () => {
  const source = [
    '::quote',
    'This is a quoted passage',
    'that spans lines',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const quoteBlock = blocks.find((b) => b.type === 'quote');
  assert.ok(quoteBlock);
  assert.ok(quoteBlock.text.includes('quoted passage'));
});

test('parseSourceBlocks handles rule blocks', () => {
  const source = '::rule id=sep1\n::end';
  const blocks = parseSourceBlocks(source);
  const ruleBlock = blocks.find((b) => b.type === 'rule');
  assert.ok(ruleBlock);
  assert.equal(ruleBlock.id, 'sep1');
});

test('parseSourceBlocks handles code blocks with no language', () => {
  const source = [
    '::code',
    'console.log("hello");',
    'function test() {}',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const codeBlock = blocks.find((b) => b.type === 'code');
  assert.ok(codeBlock);
  assert.ok(codeBlock.text.includes('console.log'));
});

test('parseSourceBlocks handles numbered lists', () => {
  const source = [
    '::numbered-list id=steps',
    '1. First step',
    '2. Second step',
    '3. Third step',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const listBlock = blocks.find((b) => b.type === 'numbered-list');
  assert.ok(listBlock);
  assert.ok(Array.isArray(listBlock.items));
  assert.equal(listBlock.items.length, 3);
});

test('normalizeDocInput with tags array', () => {
  const doc = normalizeDocInput('/tmp/test.dx', {
    title: 'Tagged Doc',
    tags: ['tag1', 'tag2', 'tag3'],
    blocks: [],
  });

  assert.ok(Array.isArray(doc.tags));
  assert.equal(doc.tags.length, 3);
  assert.ok(doc.tags.includes('tag1'));
});

test('parseDocFile with inline code fence closure', () => {
  const source = [
    'Before code',
    '',
    '```',
    'const x = 5;',
    '```',
    '',
    'After code',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const codeBlock = parsed.blocks.find((b) => b.type === 'code');
  assert.ok(codeBlock);
  assert.ok(codeBlock.text.includes('const x'));
  const afterBlock = parsed.blocks.find((b) => b.type === 'paragraph' && b.text.includes('After code'));
  assert.ok(afterBlock);
});

test('parseSourceBlocks handles empty numbered list', () => {
  const source = [
    '::numbered-list',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  // Should not create block if empty
  assert.ok(blocks);
});

test('parseSourceBlocks handles heading blocks', () => {
  const source = [
    '::heading level=2 id=sec1 class=fancy',
    'Section Title',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const headingBlock = blocks.find((b) => b.type === 'heading');
  assert.ok(headingBlock);
  assert.equal(headingBlock.level, 2);
  assert.equal(headingBlock.id, 'sec1');
  assert.equal(headingBlock.className, 'fancy');
});

test('parseDocFile handles multiple paragraphs', () => {
  const source = [
    'Paragraph one',
    'continues here',
    '',
    'Paragraph two',
    'also continues',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const paraBlocks = parsed.blocks.filter((b) => b.type === 'paragraph');
  assert.ok(paraBlocks.length >= 1);
});

test('parseSourceBlocks preserves rawSource on blocks', () => {
  const source = [
    '::paragraph id=p1',
    'Test content',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const paraBlock = blocks.find((b) => b.type === 'paragraph' && b.id === 'p1');
  assert.ok(paraBlock);
  assert.ok(paraBlock.rawSource);
  assert.ok(paraBlock.rawSource.includes('::paragraph'));
});

test('parseDocFile handles leading/trailing blank lines', () => {
  const source = [
    '',
    '',
    'Content',
    '',
    '',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  assert.ok(parsed.blocks.length >= 1);
  const contentBlock = parsed.blocks.find((b) => b.type === 'paragraph' && b.text === 'Content');
  assert.ok(contentBlock);
});

test('parseSourceBlocks handles paragraph with class', () => {
  const source = [
    '::paragraph class=intro',
    'This is introductory text.',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const paraBlock = blocks.find((b) => b.type === 'paragraph' && b.className === 'intro');
  assert.ok(paraBlock);
  assert.equal(paraBlock.text, 'This is introductory text.');
});

test('normalizeDocInput extracts summary from content', () => {
  const doc = normalizeDocInput('/tmp/test.dx', {
    title: 'Test',
    blocks: [
      { type: 'heading', level: 1, text: 'Title' },
      { type: 'paragraph', text: 'This is the first paragraph with content.' },
    ],
  });

  assert.ok(doc.summary.length > 0);
});

test('parseDocFile canonical mode ignores legacy markdown headings', () => {
  const source = [
    '# Heading 1',
    '',
    '::paragraph',
    'Canonical block',
    '::end',
    '',
    '# Heading 2',
  ].join('\n');

  const parsed = parseDocFile('/tmp/test.dx', source);
  const paraBlock = parsed.blocks.find((b) => b.type === 'paragraph' && b.text.includes('Canonical'));
  assert.ok(paraBlock);
  assert.equal(parsed.blocks.some((b) => b.type === 'heading'), false);
});
