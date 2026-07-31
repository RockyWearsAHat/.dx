/**
 * The editing operations, through the engine the VS Code extension actually loads.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * `doc-core` already tests these operations in Rust, thoroughly. What this file tests is the
 * part Rust cannot reach: that they survive the wasm boundary — that they are exported at
 * all, that strings cross intact, that an error arrives as a thrown `Error` rather than a
 * silent empty string, and that `insert_block`'s JSON is the shape the extension parses.
 *
 * A missing export here is an extension whose editor does nothing when a reader clicks, and
 * the first person to find out would be the reader.
 */

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const engine = createRequire(import.meta.url)(join(here, '..', 'wasm', 'doc_wasm.js'));

const SAMPLE =
  '::heading level=1 id=title\nGuide\n::end\n\n' +
  '::paragraph id=intro\nThe opening line.\n::end\n\n' +
  '::bulleted-list id=points\n- first\n- second\n::end\n';

test('every editing call the extension makes is exported', () => {
  for (const name of [
    'block_source',
    'set_block',
    'insert_block',
    'remove_block',
    'preview_block',
  ]) {
    assert.equal(typeof engine[name], 'function', `${name} is not exported`);
  }
});

test('a block hands back the text a reader would type', () => {
  assert.equal(engine.block_source(SAMPLE, 'intro'), 'The opening line.');
  assert.equal(engine.block_source(SAMPLE, 'title'), 'Guide');
  // A list edits as the writer's own body lines — markers carried, so
  // an item's text and its nesting both survive an unchanged save.
  assert.equal(engine.block_source(SAMPLE, 'points'), '- first\n- second');
});

test('showing a block and saving it unchanged changes nothing', () => {
  const canonical = engine.set_block(SAMPLE, 'intro', engine.block_source(SAMPLE, 'intro'));
  for (const id of ['title', 'intro', 'points']) {
    const shown = engine.block_source(canonical, id);
    assert.equal(engine.set_block(canonical, id, shown), canonical, `round trip moved \`${id}\``);
  }
});

test('saving one block leaves the rest of the document alone', () => {
  const after = engine.set_block(SAMPLE, 'intro', 'A replacement line.');
  assert.match(after, /A replacement line\./);
  assert.doesNotMatch(after, /The opening line\./);
  assert.match(after, /Guide/);
  assert.match(after, /first/);
});

test('non-ASCII prose crosses the boundary intact', () => {
  const written = 'Größe — “quoted”, ½, 日本語, 🎈';
  const after = engine.set_block(SAMPLE, 'intro', written);
  assert.equal(engine.block_source(after, 'intro'), written);
});

test('inserting returns the new source and the id to put the cursor in', () => {
  const inserted = JSON.parse(engine.insert_block(SAMPLE, 'intro', 'paragraph', ''));
  assert.equal(typeof inserted.source, 'string');
  assert.equal(typeof inserted.id, 'string');
  assert.notEqual(inserted.id, '', 'the new block was not named');
  // The id names a real block, which is the only reason the extension can focus it.
  assert.equal(engine.block_source(inserted.source, inserted.id), '');
});

test('removing takes out one block and only that one', () => {
  const after = engine.remove_block(SAMPLE, 'intro');
  assert.doesNotMatch(after, /The opening line\./);
  assert.match(after, /Guide/);
});

/**
 * The call the page makes between keystrokes. Everything about "the document keeps rendering
 * while you write on it" arrives through this one export, so what it must do is be there, be
 * the block and nothing around it, read the body the way the block does, and write nothing.
 */
test('a block draws itself from characters that were never saved', () => {
  const drawn = engine.preview_block(SAMPLE, 'intro', 'A *new* line.', 'light', false);
  assert.match(drawn, /<em>new<\/em>/);
  assert.doesNotMatch(drawn, /Guide/, 'the preview carried the document around it');
  assert.doesNotMatch(drawn, /dx-doc/, 'the preview carried the sheet it sits on');
  // A preview is a read. The source it was given is the source it leaves behind.
  assert.equal(engine.block_source(SAMPLE, 'intro'), 'The opening line.');
});

test('a previewed block is drawn exactly as the page draws it', () => {
  const saved = engine.set_block(SAMPLE, 'points', 'one\ntwo\nthree');
  const page = engine.render_html(saved, 'light', true, false);
  const drawn = engine.preview_block(SAMPLE, 'points', 'one\ntwo\nthree', 'light', false);
  assert.equal(drawn.match(/<li>/g).length, 3, 'a list did not preview as its lines');
  assert.ok(page.includes(drawn), `the block renders differently alone:\n${drawn}\n\n${page}`);
});

test('an unknown block throws a sentence naming the ones that exist', () => {
  assert.throws(
    () => engine.block_source(SAMPLE, 'nope'),
    (error) => String(error).includes('intro'),
    'the failure did not say what is available'
  );
});

test('an output block cannot be written by hand', () => {
  assert.throws(() => engine.insert_block(SAMPLE, 'intro', 'output', '5'));
});
