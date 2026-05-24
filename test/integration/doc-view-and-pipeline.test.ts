import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

import {
  isBulletedListType,
  isNumberedListType,
  normalizeClassName,
  parseAttributes,
  parseListItems,
  parseSourceBlocks,
  splitClassNames,
} from '#runtime-media/doc-pipeline.js';
import { renderDocumentViewHtml } from '#runtime-src/doc-view.js';

test('doc-pipeline helpers parse attributes and lists', () => {
  assert.equal(normalizeClassName(' a   b  '), 'a b');
  assert.deepEqual(splitClassNames(' a b  c '), ['a', 'b', 'c']);
  assert.deepEqual(parseAttributes('id=x class="a b" level=2'), { id: 'x', class: 'a b', level: '2' });
  assert.deepEqual(parseListItems(['- one', '2. two', '* three']), ['one', 'two', 'three']);
  assert.equal(isBulletedListType('list'), true);
  assert.equal(isNumberedListType('numbered-list'), true);
});

test('parseSourceBlocks parses block syntax and inline forms', () => {
  const source = [
    '::heading level=2 id=h class=hero',
    'Hello',
    '::end',
    '::paragraph id=p class=copy Hello inline ::end',
    '::paragraph id=ph hidden Hidden inline ::end',
    '::script id=vars type=application/json {"region":"us-east"} ::end',
    '::bulleted-list id=l item-a ::end',
    '::checklist id=cl [x] done ::end',
    '::image id=img src=cat.png cute ::end',
    '::rule id=r ::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  assert.equal(blocks[0].type, 'heading');
  assert.equal(blocks[0].className, 'hero');
  assert.equal(blocks[1].type, 'paragraph');
  assert.equal(blocks[2].type, 'paragraph');
  assert.equal(blocks[2].hidden, true);
  assert.equal(blocks[3].type, 'script');
  assert.equal(blocks[3].scriptType, 'application/json');
  assert.equal(blocks[4].type, 'bulleted-list');
  assert.equal(blocks[5].type, 'checklist');
  assert.equal(blocks[6].type, 'image');
  assert.equal(blocks[7].type, 'rule');
});

test('parseSourceBlocks supports style and stylesheet blocks', () => {
  const source = [
    '::style id=doc-theme',
    '.hero { color: #fff; }',
    '::end',
    '::stylesheet href=themes/global.css media=screen ::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const style = blocks.find((b) => b.type === 'style');
  const stylesheet = blocks.find((b) => b.type === 'stylesheet');

  assert.ok(style);
  assert.match(style.text, /hero/);
  assert.ok(stylesheet);
  assert.equal(stylesheet.href, 'themes/global.css');
  assert.equal(stylesheet.media, 'screen');
});

test('parseSourceBlocks supports inline style and stylesheet source fallback', () => {
  const source = [
    '::style .title { color: tomato; } ::end',
    '::stylesheet src=assets/global.css ::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const style = blocks.find((b) => b.type === 'style');
  const stylesheet = blocks.find((b) => b.type === 'stylesheet');

  assert.ok(style);
  assert.match(style.text, /tomato/);
  assert.ok(stylesheet);
  assert.equal(stylesheet.href, 'assets/global.css');
});

test('parseSourceBlocks supports inline and block stylesheet href fallback from body text', () => {
  const source = [
    '::stylesheet https://cdn.example.com/theme.css ::end',
    '::stylesheet',
    'themes/local.css',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const hrefs = blocks.filter((b) => b.type === 'stylesheet').map((b) => b.href);
  assert.deepEqual(hrefs, ['https://cdn.example.com/theme.css', 'themes/local.css']);
});

  test('parseSourceBlocks handles inline heading and code blocks', () => {
    const source = [
      '::heading level=3 id=h3 Inline Heading Text ::end',
      '::code lang=js console.log("hi") ::end',
      ':: invalid-no-type',
      '::',
    ].join('\n');

    const blocks = parseSourceBlocks(source);
    const heading = blocks.find((b) => b.type === 'heading');
    assert.ok(heading, 'Expected inline heading');
    assert.equal(heading.level, 3);
    assert.ok(heading.text.includes('Inline Heading Text'));

    const code = blocks.find((b) => b.type === 'code');
    assert.ok(code, 'Expected inline code block');
    assert.equal(code.language, 'js');
  });

  test('parseSourceBlocks handles @doc header with --- separator and metadata', () => {
    const source = [
      '@doc',
      'title: Pipeline Title',
      'summary: Pipeline summary',
      '---',
      '::paragraph',
      'Body after @doc header.',
      '::end',
    ].join('\n');

    const blocks = parseSourceBlocks(source);
    const para = blocks.find((b) => b.type === 'paragraph');
    assert.ok(para, 'Expected paragraph block after @doc header');
    assert.ok(para.text.includes('Body after @doc header'));
  });

  test('parseSourceBlocks handles @doc header with :: block before ---', () => {
    const source = [
      '@doc',
      'title: Partial Header',
      '::paragraph',
      'Reached without --- separator.',
      '::end',
    ].join('\n');

    const blocks = parseSourceBlocks(source);
    const para = blocks.find((b) => b.type === 'paragraph');
    assert.ok(para, 'Expected paragraph block');
  });

  test('parseSourceBlocks handles plain text lines (non-:: content)', () => {
    const source = [
      '::heading level=1',
      'Section Title',
      '::end',
      '',
      'Just a plain text paragraph line',
      'Another plain text line',
    ].join('\n');

    const blocks = parseSourceBlocks(source);
    const textBlock = blocks.find((b) => b.type === 'paragraph' && b.text.includes('Just a plain'));
    assert.ok(textBlock, 'Expected plain text to be wrapped in a paragraph block');
  });

test('renderDocumentViewHtml emits block wrappers and paper-mode attributes', () => {
  const doc = {
    title: 'View Test',
    relativePath: 'examples/view.dx',
    source: [
      '::heading level=1 id=head class=hero',
      'Title',
      '::end',
      '',
      '::paragraph id=p class=copy',
      'Body',
      '::end',
      '',
      '::checklist id=cl',
      '[x] done',
      '::end',
    ].join('\n'),
  };

  const html = renderDocumentViewHtml(doc, {
    theme: 'dark',
    resolvedTheme: 'dark',
    appearance: { paper: 'cream', density: 'compact', scale: 110 },
    effectiveCss: '.x{color:red;}',
  });

  assert.match(html, /data-theme="dark"/);
  assert.match(html, /data-paper="cream"/);
  assert.match(html, /data-density="compact"/);
  assert.match(html, /class="block-wrap hero head"/);
  assert.match(html, /type="checkbox" disabled checked/);
  assert.match(html, /<style>\.x\{color:red;\}<\/style>/);
});

test('renderDocumentViewHtml mounts style and stylesheet blocks without visible text', () => {
  const doc = {
    title: 'Styled View',
    source: [
      '::style',
      '.copy { letter-spacing: 0.02em; }',
      '::end',
      '::stylesheet href=https://example.com/global.css media=screen',
      '::end',
      '::paragraph',
      'Visible text',
      '::end',
    ].join('\n'),
  };

  const html = renderDocumentViewHtml(doc);
  assert.match(html, /data-doc-style="1"/);
  assert.match(html, /rel="stylesheet" href="https:\/\/example\.com\/global\.css"/);
  assert.match(html, /<p>Visible text<\/p>/);
});

test('renderDocumentViewHtml escapes closing style tags and accepts stylesheet src fallback', () => {
  const doc = {
    title: 'Styled View 2',
    blocks: [
      { type: 'style', text: '.x::after { content: "</style>"; }' },
      { type: 'stylesheet', src: 'assets/print.css' },
      { type: 'paragraph', text: 'Body' },
    ],
  };

  const html = renderDocumentViewHtml(doc);
  assert.match(html, /<style data-doc-style="1">/);
  assert.match(html, /<\\\/style>/);
  assert.match(html, /href="assets\/print\.css"/);
});

test('renderDocumentViewHtml ignores empty style and stylesheet entries', () => {
  const doc = {
    title: 'Styled View 3',
    blocks: [
      { type: 'style', text: '' },
      { type: 'stylesheet', href: '' },
      { type: 'paragraph', text: 'Only body text should render.' },
    ],
  };

  const html = renderDocumentViewHtml(doc);
  assert.equal(/data-doc-style=/.test(html), false);
  assert.equal(/rel="stylesheet"/.test(html), false);
  assert.match(html, /Only body text should render\./);
});

test('styles.css contains interactive paper-mode and chrome/edit affordances', async () => {
  const cssPath = path.join(process.cwd(), 'vscode-extension/media/styles.css');
  const css = await readFile(cssPath, 'utf8');

  assert.match(css, /Strict paper-first mode/);
  assert.match(css, /\.page\s*\{/);
  assert.match(css, /\.block-view/);
  assert.match(css, /\.ui-chrome-edit-toggle/);
  assert.match(css, /body\[data-paper="cream"\]/);
  assert.match(css, /body\[data-density="compact"\]/);
});

test('parseSourceBlocks and helpers cover defensive fallbacks', () => {
  // parseAttributes fallback paths for args||'' and empty capture groups
  assert.deepEqual(parseAttributes(null), {});
  assert.deepEqual(parseAttributes('ID='), {});

  // parseListItems fallback paths for lines||[] and line||''
  assert.deepEqual(parseListItems(null), []);
  assert.deepEqual(parseListItems([null, '   ']), []);

  const source = [
    // Inline heading with no attrs id; parseLeadingAttributesAndRemainder gets empty text
    '::heading Inline title only ::end',
    // Inline bulleted list with no item text -> inlineText ? [inlineText] : [] false branch
    '::bulleted-list ::end',
    // Inline numbered list with no item text
    '::numbered-list ::end',
    // Inline checklist with no text -> checklistItem false branch yields []
    '::checklist ::end',
    // Inline checklist with non-checkbox text -> match false branch in ternary
    '::checklist plain text item ::end',
    // Inline image with no attrs src/id
    '::image alt only ::end',
    // Inline code with lang but no language attr (attrs.language || attrs.lang)
    '::code lang=js console.log(1) ::end',
    // Inline paragraph with empty text should be skipped
    '::paragraph    ::end',
    // Block rule with no id to exercise attrs.id || '' on block path
    '::rule',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);

  const heading = blocks.find((b) => b.type === 'heading');
  assert.ok(heading);
  assert.equal(heading.id, '');

  const bullet = blocks.find((b) => b.type === 'bulleted-list');
  assert.ok(bullet);
  assert.deepEqual(bullet.items, []);
  assert.equal(bullet.id, '');

  const numbered = blocks.find((b) => b.type === 'numbered-list');
  assert.ok(numbered);
  assert.deepEqual(numbered.items, []);

  const checklistBlocks = blocks.filter((b) => b.type === 'checklist');
  assert.equal(checklistBlocks.length, 2);
  assert.deepEqual(checklistBlocks[0].items, []);
  assert.deepEqual(checklistBlocks[1].items, [{ checked: false, text: 'plain text item' }]);

  const image = blocks.find((b) => b.type === 'image');
  assert.ok(image);
  assert.equal(image.id, '');
  assert.equal(image.src, '');

  const code = blocks.find((b) => b.type === 'code');
  assert.ok(code);
  assert.equal(code.language, 'js');

  // Empty inline paragraph should be skipped entirely.
  assert.equal(blocks.some((b) => b.type === 'paragraph' && !b.text), false);

  const rule = blocks.find((b) => b.type === 'rule');
  assert.ok(rule);
  assert.equal(rule.id, '');
});

test('parseSourceBlocks parses single-quoted and bare attribute values', () => {
  const source = [
    "::heading level='4' id=h4 class='hero primary' Inline title ::end",
    "::stylesheet href='themes/shared.css' media=print ::end",
    '::image id=img src=cat.png Alt text ::end',
    "::paragraph id=p class=copy Body ::end",
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const heading = blocks.find((b) => b.type === 'heading');
  const stylesheet = blocks.find((b) => b.type === 'stylesheet');
  const image = blocks.find((b) => b.type === 'image');
  const paragraph = blocks.find((b) => b.type === 'paragraph');

  assert.ok(heading);
  assert.equal(heading.level, 4);
  assert.equal(heading.className, 'hero primary');
  assert.ok(stylesheet);
  assert.equal(stylesheet.href, 'themes/shared.css');
  assert.equal(stylesheet.media, 'print');
  assert.ok(image);
  assert.equal(image.src, 'cat.png');
  assert.ok(paragraph);
  assert.equal(paragraph.className, 'copy');
});
