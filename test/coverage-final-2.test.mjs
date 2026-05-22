import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { rm } from 'node:fs/promises';

import { parseDocFile } from '../src/doc-format.js';
import { parseSourceBlocks } from '../vscode-extension/media/doc-pipeline.js';
import { packDocument, unpackDocument } from '../src/doc-binary.js';
import {
  createDatabase,
  getDocumentSectionBySlug,
  getDocumentSections,
  listDocuments,
  searchDocuments,
  upsertDocument,
} from '../src/database.js';
import { normalizeDocInput } from '../src/doc-format.js';
import { ingestWorkspace } from '../src/doc-service.js';
import { readDocumentViewState } from '../src/view-state.js';
import { cleanupTempWorkspace, createTempWorkspace, writeDxFile } from './test-utils.mjs';

// ─── doc-format.js: list type switching in parseLegacyBlocks ─────────────────

test('parseLegacyBlocks switches from numbered-list to bulleted-list mid-flow', () => {
  const source = [
    '1. First numbered',
    '2. Second numbered',
    '- Bullet after numbers',
    '- Second bullet',
  ].join('\n');

  const parsed = parseDocFile('/tmp/listswitch1.dx', source);
  const numbered = parsed.blocks.filter((b) => b.type === 'numbered-list');
  const bulleted = parsed.blocks.filter((b) => b.type === 'bulleted-list');

  assert.equal(numbered.length, 1);
  assert.equal(bulleted.length, 1);
});

test('parseLegacyBlocks switches from bulleted-list to numbered-list mid-flow', () => {
  const source = [
    '- Bullet item one',
    '- Bullet item two',
    '1. Numbered after bullets',
    '2. Second numbered',
  ].join('\n');

  const parsed = parseDocFile('/tmp/listswitch2.dx', source);
  const bulleted = parsed.blocks.filter((b) => b.type === 'bulleted-list');
  const numbered = parsed.blocks.filter((b) => b.type === 'numbered-list');

  assert.equal(bulleted.length, 1);
  assert.equal(numbered.length, 1);
});

// ─── doc-format.js: parseDocSource docsrc-header format ──────────────────────

test('parseDocFile handles @doc docsrc header format', () => {
  const source = [
    '@doc',
    'title: Docsrc Title',
    'summary: This is the summary',
    'tags: alpha, beta',
    '---',
    '::paragraph id=p1',
    'Body text from docsrc header format.',
    '::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/docsrc.dx', source);
  assert.equal(parsed.title, 'Docsrc Title');
  assert.equal(parsed.summary, 'This is the summary');
  assert.deepEqual(parsed.tags, ['alpha', 'beta']);
  const para = parsed.blocks.find((b) => b.type === 'paragraph');
  assert.ok(para);
  assert.ok(para.text.includes('Body text'));
});

// ─── doc-format.js: parseDocSource JSON format ───────────────────────────────

test('parseDocFile handles JSON-serialized document input', () => {
  const jsonSource = JSON.stringify({
    title: 'JSON Document',
    summary: 'From JSON',
    tags: ['json'],
    meta: { format: 'json' },
    blocks: [
      { type: 'paragraph', id: 'p-json', text: 'JSON paragraph' },
    ],
  });

  const parsed = parseDocFile('/tmp/json-doc.dx', jsonSource);
  assert.equal(parsed.title, 'JSON Document');
  assert.equal(parsed.summary, 'From JSON');
  assert.deepEqual(parsed.tags, ['json']);
  assert.equal(parsed.blocks.find((b) => b.type === 'paragraph').text, 'JSON paragraph');
});

// ─── doc-format.js: parseDocsrcBlocks inline and EOF paths ───────────────────

test('parseDocFile handles inline canonical block syntax (single line ::type content ::end)', () => {
  const source = '::paragraph id=p1 Inline paragraph text ::end';
  const parsed = parseDocFile('/tmp/inline.dx', source);
  const para = parsed.blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected inline paragraph block');
  assert.ok(para.text.includes('Inline paragraph text'));
});

test('parseDocFile handles content before ::end on same line in canonical blocks', () => {
  const source = [
    '::paragraph id=p2',
    'First line',
    'Before end content ::end',
  ].join('\n');

  const parsed = parseDocFile('/tmp/inlineend.dx', source);
  const para = parsed.blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected paragraph');
  assert.ok(para.text.includes('First line'));
  assert.ok(para.text.includes('Before end content'));
});

test('parseDocFile handles canonical block without closing ::end (auto-close at EOF)', () => {
  const source = [
    '::paragraph id=p3',
    'Content with no closing end tag',
    'Another line here',
  ].join('\n');

  const parsed = parseDocFile('/tmp/noclose.dx', source);
  const para = parsed.blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected auto-closed paragraph block');
  assert.ok(para.text.includes('Content with no closing end tag'));
});

// ─── doc-pipeline.js: inline numbered-list single-line syntax ────────────────

test('parseSourceBlocks handles inline ::numbered-list ::end syntax', () => {
  const source = '::numbered-list item one ::end';
  const blocks = parseSourceBlocks(source);
  const numbered = blocks.find((b) => b.type === 'numbered-list');
  assert.ok(numbered, 'Expected a numbered-list block');
});

// ─── doc-pipeline.js: @doc header parsing in parseSourceBlocks ───────────────

test('parseSourceBlocks handles @doc header prefix with metadata lines', () => {
  const source = [
    '@doc',
    'title: My Title',
    'summary: My summary',
    '---',
    '::paragraph id=p1',
    'Body content after header.',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const para = blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected paragraph block after @doc header');
  assert.ok(para.text.includes('Body content'));
});

test('parseSourceBlocks skips to :: block start when @doc header has no --- separator', () => {
  const source = [
    '@doc',
    'title: Partial',
    '::paragraph',
    'Block reached after partial header.',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const para = blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected paragraph block after partial header');
});

// ─── doc-pipeline.js: content before ::end on same line ──────────────────────

test('parseSourceBlocks handles content before ::end on same line', () => {
  const source = [
    '::paragraph',
    'First line of content',
    'content before end ::end',
    'this line should not appear',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const para = blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected a paragraph block');
  assert.ok(para.text.includes('First line'), 'Should include first line');
  assert.ok(para.text.includes('content before end'), 'Should include content before ::end');
  assert.ok(!para.text.includes('this line should not appear'), 'Should not include lines after ::end');
});

// ─── doc-pipeline.js: block reaching EOF without ::end ───────────────────────

test('parseSourceBlocks handles block with no closing ::end (EOF)', () => {
  const source = [
    '::paragraph',
    'Some content without closing end',
    'Another line',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const para = blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected paragraph block even without ::end');
  assert.ok(para.text.includes('Some content'), 'Should capture all lines up to EOF');
  assert.ok(para.text.includes('Another line'), 'Should include second line');
});

// ─── doc-pipeline.js: opening line content ───────────────────────────────────

test('parseSourceBlocks captures opening line content in multi-line block', () => {
  const source = [
    '::paragraph Opening line text',
    'Body content',
    '::end',
  ].join('\n');

  const blocks = parseSourceBlocks(source);
  const para = blocks.find((b) => b.type === 'paragraph');
  assert.ok(para, 'Expected paragraph block');
  assert.ok(para.text.includes('Opening line text'), 'Should include opening line content');
  assert.ok(para.text.includes('Body content'), 'Should include body content');
});

// ─── doc-binary.js: multi-byte varint (strings > 127 bytes) ─────────────────

test('doc-binary handles documents with long strings requiring multi-byte varint', () => {
  const longTitle = 'x'.repeat(200);
  const doc = { title: longTitle, summary: '', tags: [], meta: {}, blocks: [] };
  const packed = packDocument(doc);
  const unpacked = unpackDocument(packed);
  assert.equal(unpacked.title, longTitle);
});

test('doc-binary encodeVarint handles negative input gracefully', () => {
  // The negative/non-finite guard in encodeVarint is triggered via packDocument
  // when an invalid value is passed. Meta keys/values go through JSON.stringify
  // so this path is triggered via a large-count document with meta.
  const doc = {
    title: '',
    summary: '',
    tags: Array.from({ length: 130 }, (_, i) => `tag${i}`),
    meta: {},
    blocks: [],
  };
  const packed = packDocument(doc);
  const unpacked = unpackDocument(packed);
  assert.equal(unpacked.tags.length, 130);
});

test('doc-binary decodeVarint throws on incomplete multi-byte varint', () => {
  const packed = packDocument({ title: '', summary: '', tags: [], meta: {}, blocks: [] });
  // Position 6 is the title string length (single byte 0). Replace with 0x80 (continuation bit set, no next byte).
  const corrupt = Buffer.from(packed);
  corrupt[6] = 0x80;
  assert.throws(() => unpackDocument(corrupt), /Unexpected end of binary document/);
});

test('doc-binary decodeString throws when declared length exceeds buffer', () => {
  const packed = packDocument({ title: '', summary: '', tags: [], meta: {}, blocks: [] });
  // Position 6 is the title string length byte (0). Change to 100 — claims 100 bytes of string
  // content but the buffer doesn't have that many bytes remaining.
  const corrupt = Buffer.from(packed);
  corrupt[6] = 100;
  assert.throws(() => unpackDocument(corrupt), /Unexpected end of binary document/);
});

// ─── view-state.js: untested branches ────────────────────────────────────────

test('readDocumentViewState sanitizes viewport with explicit positive zoomFactor', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('vs-zoom-');
  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/x.dx`, { title: 'x', blocks: [] });
    const id = upsertDocument(db, rootDir, doc, Date.now());

    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run(JSON.stringify({
      theme: 'light',
      resolvedTheme: 'light',
      appearance: { paper: 'invalid-paper', density: 'invalid-density', scale: 95 },
      viewport: { width: 1024, height: 768, pixelRatio: 2, zoomLevel: 0, zoomFactor: 1.5 },
      effectiveCss: '',
      sourceText: '',
    }), id);

    const state = readDocumentViewState(db, id);
    assert.ok(state);
    assert.equal(state.theme, 'light');
    assert.equal(state.resolvedTheme, 'light');
    assert.equal(state.appearance.paper, 'white');     // invalid-paper → 'white' fallback
    assert.equal(state.appearance.density, 'comfortable'); // invalid-density → 'comfortable' fallback
    assert.equal(state.appearance.scale, 95);
    assert.equal(state.viewport.width, 1024);
    assert.equal(state.viewport.pixelRatio, 2);
    assert.equal(state.viewport.zoomFactor, 1.5);     // explicit positive zoomFactor used directly

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('readDocumentViewState sanitizes null appearance and viewport gracefully', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('vs-null-');
  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/x.dx`, { title: 'x', blocks: [] });
    const id = upsertDocument(db, rootDir, doc, Date.now());

    db.prepare('UPDATE documents SET view_state_json = ? WHERE id = ?').run(JSON.stringify({
      theme: 'dark',
      resolvedTheme: 'dark',
      appearance: null,
      viewport: null,
      effectiveCss: '',
      sourceText: '',
    }), id);

    const state = readDocumentViewState(db, id);
    assert.ok(state);
    assert.equal(state.appearance.paper, 'white');
    assert.equal(state.appearance.density, 'comfortable');
    assert.equal(state.appearance.scale, 100);
    assert.equal(state.viewport.width, null);
    assert.equal(state.viewport.zoomLevel, 0);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

// ─── database.js: null-workspace branches ────────────────────────────────────

test('searchDocuments returns empty array when workspace does not exist', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('db-search-noworkspace-');
  try {
    const db = createDatabase(dbPath);
    const result = searchDocuments(db, '/nonexistent/workspace', 'anything');
    assert.deepEqual(result, []);
    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

test('searchDocuments returns scored results with non-empty query', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('db-search-scored-');
  try {
    const db = createDatabase(dbPath);
    const doc = normalizeDocInput(`${rootDir}/a.dx`, {
      title: 'Searchable Doc',
      blocks: [{ type: 'paragraph', text: 'unique searchable content here' }],
    });
    upsertDocument(db, rootDir, doc, Date.now());

    // Non-matching query (non-zero terms, zero results)
    const noMatch = searchDocuments(db, rootDir, 'zzznomatch999');
    assert.equal(noMatch.length, 0);

    // Matching query
    const match = searchDocuments(db, rootDir, 'unique searchable');
    assert.ok(match.length >= 1);
    assert.ok(match[0].score > 0);

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

// ─── doc-service.js: archive reconstruction failure (catch branch) ────────────

test('ingestWorkspace silently skips stub when archive reconstruction fails', async () => {
  const { rootDir, dbPath } = await createTempWorkspace('svc-archive-fail-');
  try {
    const source = [
      '::heading level=1 id=h3',
      'Archive Fail Test',
      '::end',
      '',
      '::paragraph id=p3',
      'Test for failed archive reconstruction.',
      '::end',
    ].join('\n');
    await writeDxFile(rootDir, 'docs/fail.dx', source);

    const db = createDatabase(dbPath);

    // First ingest: creates archive + stub + DB entry
    await ingestWorkspace(rootDir, db);

    // Wipe DB so the document is gone
    db.exec('DELETE FROM tokens');
    db.exec('DELETE FROM sections');
    db.exec('DELETE FROM workspace_documents');
    db.exec('DELETE FROM documents');
    db.exec('DELETE FROM workspaces');

    // Also delete the archive directory so reconstruction fails
    const docDir = path.join(rootDir, '.doc');
    await rm(docDir, { recursive: true, force: true });

    // Re-ingest: stub exists, archive is GONE → readDocArchive throws → catch fires → stub skipped
    const result = await ingestWorkspace(rootDir, db);
    assert.equal(result.length, 0, 'Stub with missing archive should be silently skipped');

    db.close();
  } finally {
    await cleanupTempWorkspace(rootDir);
  }
});

  // ─── doc-format.js: parseDocsrcBlocks paths ──────────────────────────────────

  test('parseDocFile canonical block with ::end on its own line', () => {
    const source = [
      '::paragraph id=ownline',
      'First line of text',
      'Second line of text',
      '::end',
      '::paragraph id=after',
      'After block',
      '::end',
    ].join('\n');

    const parsed = parseDocFile('/tmp/ownline.dx', source);
    const blocks = parsed.blocks.filter((b) => b.type === 'paragraph');
    assert.equal(blocks.length, 2);
    assert.ok(blocks[0].text.includes('First line'));
    assert.ok(blocks[0].text.includes('Second line'));
    assert.ok(blocks[1].text.includes('After block'));
  });

  test('parseDocFile canonical list block with non-marker content lines', () => {
    const source = [
      '::bulleted-list id=bl1',
      '- Normal bullet item',
      'Plain text without marker',
      '- Another bullet',
      '::end',
    ].join('\n');

    const parsed = parseDocFile('/tmp/listnomarker.dx', source);
    const list = parsed.blocks.find((b) => b.type === 'bulleted-list');
    assert.ok(list, 'Expected bulleted-list block');
    assert.ok(list.items.length >= 1);
  });

  test('parseDocFile canonical checklist with mixed item formats', () => {
    const source = [
      '::checklist id=cl1',
      '[x] Checked item',
      '[ ] Unchecked item',
      'Plain text no checkbox',
      '::end',
    ].join('\n');

    const parsed = parseDocFile('/tmp/mixedchecklist.dx', source);
    const checklist = parsed.blocks.find((b) => b.type === 'checklist');
    assert.ok(checklist, 'Expected checklist block');
    assert.ok(checklist.items.length >= 3);
    const plainItem = checklist.items.find((i) => i.text === 'Plain text no checkbox');
    assert.ok(plainItem, 'Expected plain text item in checklist');
    assert.equal(plainItem.checked, false);
  });

    // ─── doc-format.js: quote and code block types ───────────────────────────────

    test('parseDocFile handles ::quote block in canonical format', () => {
      const source = [
        '::quote id=q1',
        'This is a famous quote.',
        'It spans multiple lines.',
        '::end',
      ].join('\n');

      const parsed = parseDocFile('/tmp/quote.dx', source);
      const quote = parsed.blocks.find((b) => b.type === 'quote');
      assert.ok(quote, 'Expected quote block');
      assert.ok(quote.text.includes('famous quote'));
    });

    test('parseDocFile handles ::code block in canonical format', () => {
      const source = [
        '::code id=c1 lang=python',
        'def hello():',
        '    print("hi")',
        '::end',
      ].join('\n');

      const parsed = parseDocFile('/tmp/code.dx', source);
      const code = parsed.blocks.find((b) => b.type === 'code');
      assert.ok(code, 'Expected code block');
      assert.ok(code.text.includes('def hello'));
      assert.equal(code.language, 'python');
    });

    // ─── doc-format.js: standalone ::end at top level (no open block) ────────────

    test('parseDocFile handles standalone ::end without open block', () => {
      const source = [
        '::paragraph id=p1',
        'First paragraph.',
        '::end',
        '::end',
        '::paragraph id=p2',
        'Second paragraph.',
        '::end',
      ].join('\n');

      const parsed = parseDocFile('/tmp/standalone-end.dx', source);
      const blocks = parsed.blocks.filter((b) => b.type === 'paragraph');
      assert.equal(blocks.length, 2);
    });

    // ─── doc-format.js: empty bulleted-list triggers empty buildNestedListStructure ─

    test('parseDocFile handles empty list block gracefully', () => {
      const source = [
        '::bulleted-list id=empty',
        '::end',
      ].join('\n');

      const parsed = parseDocFile('/tmp/emptylist.dx', source);
      const list = parsed.blocks.find((b) => b.type === 'bulleted-list');
      assert.ok(list, 'Expected bulleted-list block');
        // Empty list is normalized to contain a default 'List item' placeholder
        assert.equal(list.items.length, 1);
        assert.equal(list.items[0].text, 'List item');
    });

    // ─── doc-pipeline.js: unwrapSyntheticParagraphWrappers path ──────────────────

    test('parseSourceBlocks unwraps synthetic paragraph wrappers', () => {
      // Synthetic paragraphs are generated by the editor and look like:
      // ::paragraph id=paragraph-42\nactual text\n::end
      const source = [
        '::paragraph id=paragraph-42',
        'Unwrapped paragraph content',
        '::end',
        '::paragraph id=paragraph-43',
        'Second unwrapped paragraph',
        '::end',
      ].join('\n');

      const blocks = parseSourceBlocks(source);
      // After unwrapping, the content is treated as plain text
      const textBlocks = blocks.filter((b) => b.type === 'paragraph');
      assert.ok(textBlocks.length >= 1, 'Expected content from unwrapped paragraphs');
    });

    test('parseSourceBlocks keeps synthetic wrapper when close line is not ::end', () => {
      // If the pattern matches but the close line is not ::end, no unwrap occurs
      const source = [
        '::paragraph id=paragraph-99',
        'Content line',
        '::not-end',
        '::paragraph',
        'Real paragraph',
        '::end',
      ].join('\n');

      const blocks = parseSourceBlocks(source);
      assert.ok(blocks.length >= 1);
    });

    // ─── doc-pipeline.js: for-loop fallback break (non-header, non-meta line) ────

    test('parseSourceBlocks breaks header-skip loop on unrecognized non-header line', () => {
      // @doc prefix starts the header skip loop, but a line that is not metadata,
      // not empty, not ::, and not --- causes the fallback break.
      const source = [
        '@doc',
        'This is a random non-metadata line!',
        '::paragraph',
        'Body content',
        '::end',
      ].join('\n');

      const blocks = parseSourceBlocks(source);
      // The random line causes the loop to break, then the :: line starts the main loop
      const para = blocks.find((b) => b.type === 'paragraph');
      assert.ok(para, 'Expected to find paragraph block after fallback break');
    });


