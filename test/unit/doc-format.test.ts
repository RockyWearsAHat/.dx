import test from 'node:test';
import assert from 'node:assert/strict';
import { createDefaultBlocks, normalizeDocInput, parseDocFile, stringifyDocFile } from '#runtime-src/doc-format.js';

test('parseDocFile handles canonical block source and checklist/list/image', () => {
  const source = [
    '::heading level=2 id=my-head class=hero',
    'Hello',
    '::end',
    '',
    '::checklist id=tasks class=todo',
    '[x] done',
    '[ ] todo',
    '::end',
    '',
    '::bulleted-list id=items',
    '- a',
    '- b',
    '::end',
    '',
    '::image id=img src=cat.png',
    'Cat image',
    '::end',
    '',
    '::rule id=sep',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/demo.dx', source);

  assert.equal(parsed.blocks[0].type, 'heading');
  assert.equal(parsed.blocks[0].level, 2);
  assert.equal(parsed.blocks[0].id, 'my-head');
  assert.equal(parsed.blocks[0].className, 'hero');
  assert.equal(parsed.blocks[1].type, 'checklist');
  assert.equal(parsed.blocks[1].items[0].checked, true);
  assert.equal(parsed.blocks[2].type, 'bulleted-list');
  assert.ok(Array.isArray(parsed.blocks[2].items));
  assert.equal(parsed.blocks[3].type, 'image');
  assert.equal(parsed.blocks[4].type, 'rule');
  assert.ok(parsed.sections.length >= 1);
});

test('parseDocFile handles legacy markdown-style source', () => {
  const source = [
    '# Heading',
    '',
    'Paragraph one.',
    '',
    '- item a',
    '- item b',
    '',
    '> quote',
    '',
    '```js',
    'console.log(1)',
    '```',
  ].join('\n');

  const parsed = parseDocFile('/tmp/legacy.dx', source);
  const types = parsed.blocks.map((b) => b.type);

  assert.ok(types.includes('heading'));
  assert.ok(types.includes('paragraph'));
  assert.ok(types.includes('bulleted-list'));
  assert.ok(types.includes('quote'));
  assert.ok(types.includes('code'));
});

test('normalizeDocInput fills defaults and stringifyDocFile is deterministic', () => {
  const source = [
    '::heading level=1 id=title-main',
    'A',
    '::end',
    '',
    '::paragraph id=body-main',
    'Hello',
    '::end',
  ].join('\n');

  const first = stringifyDocFile(parseDocFile('/tmp/a.dx', source));
  const second = stringifyDocFile(parseDocFile('/tmp/a.dx', first));

  assert.equal(first, second);
  assert.ok(first.includes('::paragraph'));
});

test('createDefaultBlocks generates editable starter content', () => {
  const blocks = createDefaultBlocks('Title');
  assert.equal(blocks[0].type, 'heading');
  assert.equal(blocks[1].type, 'paragraph');
});

test('parseDocFile handles @doc docsrc header format with metadata lines', () => {
  const source = [
    '@doc',
    'title: Docsrc Title',
    '',
    'not-a-key-value',
    'summary: This is the summary',
    'tags: alpha, beta',
    'meta.author: Alice',
    '---',
    '::paragraph id=p1',
    'Body text from docsrc header format.',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/docsrc-meta.dx', source);
  assert.equal(parsed.title, 'Docsrc Title');
  assert.equal(parsed.summary, 'This is the summary');
  assert.deepEqual(parsed.tags, ['alpha', 'beta']);
  assert.equal(parsed.meta.author, 'Alice');
  const para = parsed.blocks.find((b) => b.type === 'paragraph');
  assert.ok(para);
  assert.ok(para.text.includes('Body text'));
});

test('parseDocFile unwraps synthetic paragraph wrappers before parsing', () => {
  const source = [
    '::paragraph id=paragraph-42',
    'Synthetic wrapper content',
    '::end',
    '::heading level=1 id=h1',
    'Real heading',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/synthetic.dx', source);
  const heading = parsed.blocks.find((b) => b.type === 'heading');
  assert.ok(heading, 'Expected heading block');
  assert.ok(heading.text.includes('Real heading'));
  assert.ok(parsed.blocks.length >= 1);
});

test('parseDocFile handles ::quote and ::code canonical blocks', () => {
  const source = [
    '::quote id=q1',
    'Famous words of wisdom.',
    '::end',
    '',
    '::code id=c1 lang=js',
    'console.log("hello")',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/quote-code.dx', source);
  const quote = parsed.blocks.find((b) => b.type === 'quote');
  const code = parsed.blocks.find((b) => b.type === 'code');
  assert.ok(quote, 'Expected quote block');
  assert.ok(quote.text.includes('Famous words'));
  assert.ok(code, 'Expected code block');
  assert.equal(code.language, 'js');
});

test('parseDocFile handles standalone ::end and empty list in canonical format', () => {
  const source = [
    '::paragraph id=p1',
    'First para.',
    '::end',
    '::end',
    '::bulleted-list id=empty',
    '::end',
    '::paragraph id=p2',
    'Last para.',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/edge-cases.dx', source);
  const paras = parsed.blocks.filter((b) => b.type === 'paragraph');
  const list = parsed.blocks.find((b) => b.type === 'bulleted-list');
  assert.ok(paras.length >= 1);
  assert.ok(list);
});

test('stringifyDocFile uses quoted attribute for className with spaces', () => {
  const source = [
    '::paragraph id=p1 class=hero section',
    'Content here.',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/classname.dx', source);
  const stringified = stringifyDocFile(parsed);
  assert.ok(stringified.includes('class='));
});
  test('stringifyDocFile quotes className with spaces (formatAttributeValue quoted path)', () => {
    const doc = normalizeDocInput('/tmp/quoted.dx', {
      title: 'Quoted Class',
      blocks: [{ type: 'paragraph', id: 'p1', className: 'hero section', text: 'hi' }],
    });
    const stringified = stringifyDocFile(doc);
    assert.ok(stringified.includes('class="hero section"'), `Expected quoted class, got: ${stringified}`);
  });

test('normalizeDocInput handles list block with string items (not object)', () => {
  const doc = normalizeDocInput('/tmp/strlist.dx', {
    title: 'String Items',
    blocks: [
      { type: 'bulleted-list', items: ['apple', 'banana', 'cherry'] },
    ],
  });

  const list = doc.blocks.find((b) => b.type === 'bulleted-list');
  assert.ok(list);
  assert.equal(list.items[0].text, 'apple');
  assert.equal(list.items[1].text, 'banana');
});

test('normalizeDocInput handles list block with text instead of items array', () => {
  const doc = normalizeDocInput('/tmp/textlist.dx', {
    title: 'Text List',
    blocks: [
      { type: 'bulleted-list', text: 'line one\nline two\nline three' },
    ],
  });

  const list = doc.blocks.find((b) => b.type === 'bulleted-list');
  assert.ok(list);
  assert.equal(list.items[0].text, 'line one');
  assert.equal(list.items[1].text, 'line two');
});

test('parseDocFile ignores --- separator when header does not start with @doc', () => {
  const source = [
    'Just some regular text',
    'Another line',
    '---',
    '::paragraph id=p1',
    'Body content.',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/no-at-doc.dx', source);
  assert.equal(parsed.title, 'no-at-doc');
  const para = parsed.blocks.find((b) => b.type === 'paragraph' && b.text.includes('Body content.'));
  assert.ok(para, 'Expected body paragraph to survive non-@doc separator');
});

test('parseDocFile calls parseFrontmatter with YAML-style header and plain body', () => {
  const source = [
    '---',
    'title: My YAML Title',
    'draft: true',
    'published: false',
    'priority: 42',
    'data: [1,2,3]',
    'config: {invalid_json}',
    '---',
    'Plain text content with no block syntax.',
  ].join('\n');

  const parsed = parseDocFile('/tmp/frontmatter.dx', source);
  assert.equal(parsed.title, 'My YAML Title');
  assert.equal(parsed.metadata.draft, true);
  assert.equal(parsed.metadata.published, false);
  assert.equal(parsed.metadata.priority, 42);
  assert.deepEqual(parsed.metadata.data, [1, 2, 3]);
  assert.equal(parsed.metadata.config, '{invalid_json}');
  const para = parsed.blocks.find((b) => b.type === 'paragraph' && b.text.includes('Plain text content with no block syntax.'));
  assert.ok(para, 'Expected body text paragraph after frontmatter');
});
  test('parseDocFile parseFrontmatter handles blank lines and non-colon lines inside block', () => {
    // blank line hits continue at 68-69; "just-a-word" (no colon) hits continue at 74-75
    const source = [
      '---',
      'title: With Blanks',
      '',
      'just-a-word',
      'priority: 7',
      '---',
      'Body without block syntax.',
    ].join('\n');

    const parsed = parseDocFile('/tmp/frontmatter-blanks.dx', source);
    assert.equal(parsed.title, 'With Blanks');
    assert.equal(parsed.metadata.priority, 7);
    assert.equal(parsed.metadata['just-a-word'], undefined);
  });

  test('parseDocFile parseFrontmatter handles missing closing --- (returns early)', () => {
    // No closing \n---\n → hits early return at lines 59-60
    const source = [
      '---',
      'title: Unclosed',
      'no closing separator here',
      'just plain text continues',
    ].join('\n');

    const parsed = parseDocFile('/tmp/frontmatter-unclosed.dx', source);
    assert.equal(parsed.title, 'frontmatter-unclosed');
    const para = parsed.blocks.find((b) => b.type === 'paragraph' && b.text.includes('title: Unclosed'));
    assert.ok(para, 'Expected unclosed frontmatter text to be treated as body content');
  });

test('parseDocFile handles heading with non-numeric level (clamps to 1)', () => {
  const source = [
    '::heading level=abc id=h1',
    'My Heading',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/heading-level.dx', source);
  const heading = parsed.blocks.find((b) => b.type === 'heading');
  assert.ok(heading, 'Expected heading block');
  assert.equal(heading.level, 1);
});

test('parseDocFile keeps style metadata blocks while excluding them from semantic section content', () => {
  const source = [
    '::style id=doc-style',
    '.hidden { display: none; }',
    '::end',
    '::stylesheet href=themes/shared.css media=screen',
    '::end',
    '::heading level=1',
    'Visible Heading',
    '::end',
    '::paragraph',
    'Visible body text.',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/style-blocks.dx', source);
  const styleBlock = parsed.blocks.find((b) => b.type === 'style');
  const stylesheetBlock = parsed.blocks.find((b) => b.type === 'stylesheet');

  assert.ok(styleBlock);
  assert.ok(stylesheetBlock);
  assert.equal(stylesheetBlock.href, 'themes/shared.css');
  assert.equal(stylesheetBlock.media, 'screen');
  assert.equal(parsed.sections.some((section) => section.content.includes('display: none')), false);
});

test('parseDocFile preserves svg/html/graph/mermaid typed blocks', () => {
  const source = [
    '::svg id=logo',
    '<svg viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" /></svg>',
    '::end',
    '',
    '::html id=hero',
    '<div class="hero">Hello</div>',
    '::end',
    '',
    '::graph id=g1',
    '<svg viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" /></svg>',
    '::end',
    '',
    '::mermaid id=m1',
    'graph TD',
    'A --> B',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/rich-types.dx', source);
  const types = parsed.blocks.map((block) => block.type);

  assert.ok(types.includes('svg'));
  assert.ok(types.includes('html'));
  assert.ok(types.includes('graph'));
  assert.ok(types.includes('mermaid'));

  const roundtrip = stringifyDocFile(parsed);
  assert.match(roundtrip, /::svg/);
  assert.match(roundtrip, /::html/);
  assert.match(roundtrip, /::graph/);
  assert.match(roundtrip, /::mermaid/);
});

test('parseDocFile and stringifyDocFile preserve hidden block attributes', () => {
  const source = [
    '::paragraph id=visible',
    'Visible text',
    '::end',
    '',
    '::paragraph id=secret hidden=true',
    'Hidden text',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/hidden.dx', source);
  const hiddenBlock = parsed.blocks.find((block) => block.id === 'secret');
  assert.ok(hiddenBlock);
  assert.equal(hiddenBlock.hidden, true);

  const roundtrip = stringifyDocFile(parsed);
  assert.match(roundtrip, /::paragraph id=secret hidden=true/);
});

test('stringifyDocFile serializes object-backed list items without [object Object]', () => {
  const doc = normalizeDocInput('/tmp/object-list.dx', {
    title: 'Object List',
    blocks: [
      {
        type: 'numbered-list',
        id: 'steps',
        items: [
          { text: 'First step' },
          { text: 'Second step', nested: [{ text: 'Second step detail' }] },
        ],
      },
    ],
  });

  const source = stringifyDocFile(doc);
  assert.equal(source.includes('[object Object]'), false);
  assert.match(source, /- First step/);
  assert.match(source, /- Second step/);
  assert.match(source, /  - Second step detail/);
});

test('parseDocFile accepts single-quoted and bare attributes and inline blocks', () => {
  const source = [
    "::heading level='3' id=h3 class='hero primary' Inline heading text ::end",
    "::paragraph id=p1 class=copy Body text ::end",
    "::stylesheet href='themes/global.css' media=print ::end",
    "::checklist id=tasks [x] done ::end",
  ].join('\n');

  const parsed = parseDocFile('/tmp/attr-modes.dx', source);
  const heading = parsed.blocks.find((b) => b.type === 'heading');
  const paragraph = parsed.blocks.find((b) => b.type === 'paragraph');
  const stylesheet = parsed.blocks.find((b) => b.type === 'stylesheet');
  const checklist = parsed.blocks.find((b) => b.type === 'checklist');

  assert.ok(heading);
  assert.equal(heading.level, 3);
  assert.equal(heading.className, 'hero primary');
  assert.ok(paragraph);
  assert.equal(paragraph.className, 'copy');
  assert.ok(stylesheet);
  assert.equal(stylesheet.href, 'themes/global.css');
  assert.equal(stylesheet.media, 'print');
  assert.ok(checklist);
  assert.deepEqual(checklist.items, [{ checked: true, text: 'done' }]);
});

test('parseDocFile legacy parser handles sparse heading/list/quote captures', () => {
  const source = [
    '# ',
    '- ',
    '1. ',
    '>',
    'actual paragraph',
  ].join('\n');

  const parsed = parseDocFile('/tmp/legacy-sparse.dx', source);
  assert.ok(parsed.blocks.some((b) => b.type === 'heading'));
  assert.ok(parsed.blocks.some((b) => b.type === 'bulleted-list'));
  assert.ok(parsed.blocks.some((b) => b.type === 'numbered-list'));
  assert.ok(parsed.blocks.some((b) => b.type === 'quote'));
  assert.ok(parsed.blocks.some((b) => b.type === 'paragraph'));
});

test('normalizeDocInput handles non-object input defensively', () => {
  const normalized = normalizeDocInput('/tmp/non-object.dx', 'not-an-object');
  assert.equal(normalized.title, 'non-object');
  assert.ok(Array.isArray(normalized.blocks));
  assert.ok(normalized.blocks.length >= 1);
});
