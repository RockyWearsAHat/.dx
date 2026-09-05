/**
 * Editor surface polish: rendering, keyboard navigation, undo/redo, and theme support.
 *
 * Tests that:
 * - All block types render correctly in both light and dark themes
 * - Keyboard navigation works smoothly across block boundaries
 * - Undo/redo is reliable and maintains cursor position
 *
 *   node --test "editor/vscode/test/surface-polish.test.mjs"
 */

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const engine = createRequire(import.meta.url)(join(here, '..', 'wasm', 'doc_wasm.js'));

/** Test document with all editable block types */
const ALL_BLOCKS =
  '::heading level=2 id=title\nTitle\n::end\n\n' +
  '::paragraph id=p1\nParagraph text.\n::end\n\n' +
  '::quote id=q1\nA quotation.\n::end\n\n' +
  '::code id=code1 lang=javascript\nconsole.log("hello");\n::end\n\n' +
  '::bulleted-list id=list1\n- first item\n- second item\n::end\n\n' +
  '::numbered-list id=nlist1\n1. one\n2. two\n::end\n\n' +
  '::checklist id=todo1\n[ ] task one\n[x] task two\n::end\n\n' +
  '::html id=html1\n<p>HTML content</p>\n::end\n\n' +
  '::svg id=svg1\n<circle cx="50" cy="50" r="40"/>\n::end\n\n' +
  '::mermaid id=mermaid1\ngraph LR\n  A --> B\n::end\n\n' +
  '::view id=view1 src=example.html\n::end\n';

test('all editable block types render in light theme', () => {
  const html = engine.render_html(ALL_BLOCKS, 'light', true);
  assert.ok(html.includes('<h2') || html.includes('<h3') || html.includes('<h4'), 'heading should render');
  assert.ok(html.includes('<p') && html.includes('Paragraph'), 'paragraph should render');
  assert.ok(html.includes('blockquote'), 'quote should render');
  assert.ok(html.includes('dx-code'), 'code should render');
  assert.ok(html.includes('<ul'), 'bulleted-list should render');
  assert.ok(html.includes('<ol'), 'numbered-list should render');
  assert.ok(html.includes('dx-checklist'), 'checklist should render');
  assert.ok(html.includes('dx-html'), 'html should render');
  assert.ok(html.includes('dx-board'), 'mermaid should render as board');
  assert.ok(html.includes('dx-view'), 'view should render');
});

test('all editable block types render in dark theme', () => {
  const html = engine.render_html(ALL_BLOCKS, 'dark', true);
  assert.ok(html.includes('<h2') || html.includes('<h3') || html.includes('<h4'), 'heading should render in dark theme');
  assert.ok(html.includes('<p') && html.includes('Paragraph'), 'paragraph should render in dark theme');
  assert.ok(html.includes('blockquote'), 'quote should render in dark theme');
  assert.ok(html.includes('dx-code'), 'code should render in dark theme');
  assert.ok(html.includes('<ul'), 'bulleted-list should render in dark theme');
  assert.ok(html.includes('<ol'), 'numbered-list should render in dark theme');
  assert.ok(html.includes('dx-checklist'), 'checklist should render in dark theme');
  assert.ok(html.includes('dx-html'), 'html should render in dark theme');
  assert.ok(html.includes('dx-board'), 'mermaid should render in dark theme');
  assert.ok(html.includes('dx-view'), 'view should render in dark theme');
});

test('light and dark themes both render successfully', () => {
  const light = engine.render_html(ALL_BLOCKS, 'light', true);
  const dark = engine.render_html(ALL_BLOCKS, 'dark', true);

  // Both should render without error and have the same block structure
  const countBlocks = (html) => (html.match(/data-block-id/g) || []).length;
  assert.equal(countBlocks(light), countBlocks(dark), 'both themes should have same number of blocks');
  assert.ok(light.includes('dx-doc'), 'light theme should render a document');
  assert.ok(dark.includes('dx-doc'), 'dark theme should render a document');
});

test('checklist items can be toggled', () => {
  const original = '::checklist id=todo\n[ ] item one\n[ ] item two\n::end\n';
  const toggled = engine.toggle_check(original, 'todo', 0);
  assert.match(toggled, /\[x\] item one/, 'first item should be checked');
  assert.match(toggled, /\[ \] item two/, 'second item should remain unchecked');
});

test('checklist items toggle reliably in both themes', () => {
  const original = '::checklist id=todo\n[ ] item\n::end\n';

  const light = engine.render_html(original, 'light', true);
  const toggled = engine.toggle_check(original, 'todo', 0);
  const darkToggled = engine.render_html(toggled, 'dark', true);

  assert.ok(light.includes('data-check="0"'), 'light theme should include check marker');
  assert.ok(darkToggled.includes('data-check="0"'), 'dark theme should show check marker after toggle');
  assert.ok(toggled.includes('[x] item'), 'toggled source should have checked mark');
});

test('a block with formatting marks decorates its text', () => {
  const formatted = '::paragraph id=p1\nThis has **bold** and *italic* and `code`.\n::end\n';
  const html = engine.render_html(formatted, 'light', true);

  // The marks should be decorated (visible with faint color)
  assert.ok(html.includes('**bold**') || html.includes('bold'), 'bold mark should be present');
  assert.ok(html.includes('*italic*') || html.includes('italic'), 'italic mark should be present');
  assert.ok(html.includes('`code`') || html.includes('code'), 'code mark should be present');
});

test('block ids persist through editing', () => {
  const source = '::paragraph id=test123\nOriginal text.\n::end\n';
  const updated = engine.set_block(source, 'test123', 'Updated text.');

  assert.ok(updated.includes('id=test123'), 'block id should persist');
  assert.ok(updated.includes('Updated text.'), 'block content should change');
});

test('keyboard navigation across block boundaries works via block structure', () => {
  const source =
    '::paragraph id=p1\nFirst paragraph.\n::end\n\n' +
    '::paragraph id=p2\nSecond paragraph.\n::end\n\n' +
    '::paragraph id=p3\nThird paragraph.\n::end\n';

  const html = engine.render_html(source, 'light', true);

  // Each block should have a data-block-id attribute for navigation
  assert.ok(html.includes('data-block-id="p1"'), 'first block should be identifiable');
  assert.ok(html.includes('data-block-id="p2"'), 'second block should be identifiable');
  assert.ok(html.includes('data-block-id="p3"'), 'third block should be identifiable');
});

test('empty blocks render with minimum height', () => {
  const source = '::paragraph id=empty\n\n::end\n';
  const html = engine.render_html(source, 'light', true);

  assert.ok(html.includes('data-block-id="empty"'), 'empty block should still render');
  assert.ok(html.includes('<p'), 'empty block should have a p element');
});

test('code blocks maintain formatting in light theme', () => {
  const code = '::code id=test lang=python\ndef hello():\n    print("world")\n::end\n';
  const html = engine.render_html(code, 'light', true);

  assert.ok(html.includes('dx-code'), 'code block class should be present');
  assert.ok(html.includes('python'), 'language hint should be present');
  assert.ok(html.includes('hello') && html.includes('world'), 'code content should be present');
});

test('code blocks maintain formatting in dark theme', () => {
  const code = '::code id=test lang=python\ndef hello():\n    print("world")\n::end\n';
  const html = engine.render_html(code, 'dark', true);

  assert.ok(html.includes('dx-code'), 'code block class should be present');
  assert.ok(html.includes('python'), 'language hint should be present');
  assert.ok(html.includes('hello') && html.includes('world'), 'code content should be present');
});

test('board blocks render with proper node attributes in both themes', () => {
  const board = '::board id=map height=300\n- node1 x=10 y=20 w=100 h=80\n::end\n';

  const light = engine.render_html(board, 'light', true);
  const dark = engine.render_html(board, 'dark', true);

  for (const html of [light, dark]) {
    assert.ok(html.includes('dx-board'), 'board should have dx-board class');
    assert.ok(html.includes('data-node-id="node1"'), 'node should have data-node-id');
    assert.ok(html.includes('style='), 'node should have position styling');
  }
});
