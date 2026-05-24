import test from 'node:test';
import assert from 'node:assert/strict';
import { renderDocumentViewHtml } from '#runtime-src/doc-view.js';

test('renderDocumentViewHtml handles blocks with null/missing fields (branch coverage)', () => {
  const doc = {
    title: 'Branch Test',
    source: '',
    blocks: [
      // heading with no text and no level (|| '' and || 1 branches)
      { type: 'heading' },
      // bulleted-list with non-array items (|| [] branch)
      { type: 'bulleted-list', items: null },
      // bulleted-list with string items (item?.text || item branch)
      { type: 'bulleted-list', items: [{ checked: false }] },
      // checklist with non-array items
      { type: 'checklist', items: null },
      // checklist with item that has no text (item?.text || item || '' branch)
      { type: 'checklist', items: [{ checked: true }] },
      // quote with no text
      { type: 'quote' },
      // code with no text
      { type: 'code' },
      // image with no src and no alt (alt || '' and alt ? figcaption : '' branches)
      { type: 'image' },
      // image with alt but no src
      { type: 'image', alt: 'Alt text', src: '' },
      // rule with no className (attrs.length === 0 branch)
      { type: 'rule' },
      // paragraph with no text (block?.text || '' branch)
      { type: 'paragraph' },
      // unknown type (falls through to paragraph render)
      { type: 'unknown' },
    ],
  };

  const html = renderDocumentViewHtml(doc);
  assert.ok(typeof html === 'string');
  assert.ok(html.includes('<html'));
});

test('renderDocumentViewHtml escapes null/undefined text values', () => {
  const doc = {
    title: null,
    source: '',
    blocks: [{ type: 'paragraph', text: null }],
  };

  const html = renderDocumentViewHtml(doc);
  assert.ok(typeof html === 'string');
});

  test('renderDocumentViewHtml handles null items and missing type in blocks', () => {
    const doc = {
      // no title and no relativePath → || 'Untitled Document' fallback
      source: '',
      // not an array → [] fallback (the Array.isArray false branch at line 148)
      blocks: 'not-an-array',
    };
    const html = renderDocumentViewHtml(doc);
    assert.ok(typeof html === 'string');
  });

  test('renderDocumentViewHtml handles null list items and empty-type blocks', () => {
    const doc = {
      title: 'Null Items',
      source: '',
      blocks: [
        // bulleted-list with null items → || '' branch in toStringItems (line 36)
        { type: 'bulleted-list', items: [null, ''] },
        // block with null type → || 'paragraph' fallback in renderBlock (line 76)
        // and also inside blocksMarkup (line 157)
        { type: null },
        // checklist with null item → || '' branch (line 102)
        { type: 'checklist', items: [null] },
      ],
    };
    const html = renderDocumentViewHtml(doc);
    assert.ok(typeof html === 'string');
    assert.ok(html.includes('<html'));
  });

test('renderDocumentViewHtml renders all major block types and escapes content', () => {
  const doc = {
    title: 'All Blocks',
    relativePath: 'examples/all.dx',
    source: [
      '::heading level=3 id=h class=hero hero',
      'A <Title>',
      '::end',
      '::bulleted-list id=ul class=list list',
      '- one',
      '- two',
      '::end',
      '::numbered-list id=ol',
      '1. first',
      '2. second',
      '::end',
      '::quote id=q',
      'quoted <text>',
      '::end',
      '::code id=c language=js',
      'const x = "<x>";',
      '::end',
      '::image id=img class=photo src=cat.png',
      'cat <alt>',
      '::end',
      '::rule id=sep',
      '::end',
      '::paragraph id=p class=copy',
      'hello <world>',
      '::end',
    ].join('\n'),
  };

  const html = renderDocumentViewHtml(doc, {
    theme: 'auto',
    resolvedTheme: 'light',
    appearance: { paper: 'gray', density: 'compact', scale: 999 },
    effectiveCss: 'p{color:red;}',
  });

  assert.match(html, /A &lt;Title&gt;/);
  assert.match(html, /id="h"/);
  assert.match(html, /<ul[^>]*>/);
  assert.match(html, /<ol[^>]*>/);
  assert.match(html, /<blockquote[^>]*>/);
  assert.match(html, /<pre[^>]*>/);
  assert.match(html, /<figure[^>]*>/);
  assert.match(html, /<li>one<\/li>/);
  assert.match(html, /<li>two<\/li>/);
  assert.match(html, /<li>first<\/li>/);
  assert.match(html, /<li>second<\/li>/);
  assert.match(html, /quoted &lt;text&gt;/);
  assert.match(html, /const x = &quot;&lt;x&gt;&quot;;/);
  assert.match(html, /src="cat\.png"/);
  assert.match(html, /cat &lt;alt&gt;/);
  assert.match(html, /<hr[^>]*\/>/);
  assert.match(html, /<p[^>]*>hello &lt;world&gt;<\/p>/);
  assert.match(html, /--editor-scale:1.15/);
});

test('renderDocumentViewHtml falls back to document.blocks when source is empty', () => {
  const doc = {
    title: 'Fallback Blocks',
    relativePath: 'examples/fallback.dx',
    source: '',
    blocks: [
      { type: 'checklist', id: 'cl', items: [{ checked: true, text: 'done' }, { checked: false, text: 'todo' }] },
      { type: 'paragraph', id: 'p1', text: 'fallback paragraph' },
      { type: 'unknown-type', id: 'u1', text: 'unknown fallback' },
    ],
  };

  const html = renderDocumentViewHtml(doc, {
    appearance: { scale: 10 },
  });

  assert.match(html, /id="cl"/);
  assert.match(html, /type="checkbox" disabled checked/);
  assert.match(html, /<span class="check-done">done<\/span>/);
  assert.match(html, /<span>todo<\/span>/);
  assert.match(html, /fallback paragraph/);
  assert.match(html, /unknown fallback/);
  assert.match(html, /--editor-scale:0.9/);
});

test('renderDocumentViewHtml renders object-backed list items without [object Object]', () => {
  const doc = {
    title: 'Object Lists',
    source: '',
    blocks: [
      {
        type: 'numbered-list',
        id: 'steps',
        items: [
          { text: 'First step' },
          { text: 'Second step' },
        ],
      },
    ],
  };

  const html = renderDocumentViewHtml(doc);
  assert.equal(html.includes('[object Object]'), false);
  assert.match(html, /<li>First step<\/li>/);
  assert.match(html, /<li>Second step<\/li>/);
});

test('renderDocumentViewHtml renders svg/html/graph blocks and sanitizes script handlers', () => {
  const doc = {
    title: 'Rich Render',
    source: [
      '::svg id=logo',
      '<svg viewBox="0 0 10 10" onload="alert(1)"><circle cx="5" cy="5" r="4" /></svg>',
      '::end',
      '',
      '::html id=hero',
      '<div onclick="evil()">Hero</div><script>alert(1)</script>',
      '::end',
      '',
      '::graph id=plot',
      '<svg viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" /></svg>',
      '::end',
      '',
      '::code language=svg',
      '<svg viewBox="0 0 5 5"><rect x="1" y="1" width="3" height="3" /></svg>',
      '::end',
      '',
      '::code language=html',
      '<section>Inline HTML</section>',
      '::end',
    ].join('\n'),
  };

  const html = renderDocumentViewHtml(doc);
  assert.match(html, /class="svg-wrap"/);
  assert.match(html, /class="html-wrap"/);
  assert.match(html, /class="graph-wrap"/);
  assert.equal(html.includes('onclick='), false);
  assert.equal(html.includes('onload='), false);
  assert.equal(html.includes('<script>'), false);
  assert.match(html, /<svg viewBox="0 0 10 10"/);
  assert.match(html, /<section>Inline HTML<\/section>/);
});

test('renderDocumentViewHtml marks hidden blocks with hidden attributes', () => {
  const doc = {
    title: 'Hidden Blocks',
    source: [
      '::paragraph id=shown',
      'Shown paragraph',
      '::end',
      '',
      '::paragraph id=hidden-block hidden',
      'Hidden paragraph',
      '::end',
    ].join('\n'),
  };

  const html = renderDocumentViewHtml(doc);
  assert.match(html, /data-block-id="hidden-block"/);
  assert.match(html, /data-block-hidden="true"/);
  assert.match(html, /class="[^"]*is-hidden[^"]*"/);
});

test('renderDocumentViewHtml interpolates template vars from script json blocks', () => {
  const doc = {
    title: 'Template Vars',
    source: [
      '::script id=vars type=application/json',
      '{"kpi":"47.50%","label":"Compaction"}',
      '::end',
      '',
      '::code id=summary language=html',
      '<table><tbody><tr><td>{{label}}</td><td>{{kpi}}</td></tr></tbody></table>',
      '::end',
      '',
      '::paragraph id=caption',
      '{{label}} result: {{kpi}}',
      '::end',
    ].join('\n'),
  };

  const html = renderDocumentViewHtml(doc);
  assert.match(html, /<td>Compaction<\/td>/);
  assert.match(html, /<td>47\.50%<\/td>/);
  assert.match(html, /<p[^>]*>Compaction result: 47\.50%<\/p>/);
  assert.equal(html.includes('::script'), false);
});
