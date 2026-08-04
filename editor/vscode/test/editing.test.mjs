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
    'block_header',
    'set_block',
    'replace_block',
    'insert_block',
    'remove_block',
    'preview_block',
    'field_html',
    'board_place',
    'board_arrange',
    'board_detach',
    'board_link',
    'toggle_check',
  ]) {
    assert.equal(typeof engine[name], 'function', `${name} is not exported`);
  }
});

const BOARD =
  '::board id=plan height=520\n- ideas x=40 y=40 w=280 h=160\n::end\n\n' +
  '::paragraph id=ideas hidden\nRough ideas.\n::end\n';

test('a board node moves, reshapes, links, and detaches across the boundary', () => {
  const placed = JSON.parse(engine.board_place(BOARD, 'plan', 'ideas', 200, 80, 0, 240));
  assert.equal(placed.id, 'ideas');
  assert.match(placed.source, /- ideas x=200 y=80 w=280 h=240/);

  const added = JSON.parse(engine.board_place(placed.source, 'plan', '', 500, 40, 0, 0));
  assert.notEqual(added.id, '');
  assert.match(added.source, new RegExp(`::paragraph id=${added.id} hidden`));

  const linked = engine.board_link(added.source, 'plan', added.id, 'ideas', true, 'b', 't');
  assert.match(linked, new RegExp(`- ${added.id} x=500 y=40 w=280 h=180 to=ideas:b-t`));

  const detached = engine.board_detach(linked, 'plan', added.id);
  assert.doesNotMatch(detached, new RegExp(`- ${added.id}`));
  assert.doesNotMatch(detached, new RegExp(`id=${added.id}`));
});

const REVIEW =
  '::code id=listing src=src/lib.rs lang=rust\n::end\n\n' +
  '::board id=map height=300\n- plan.dx#step x=20 y=20 w=200 h=90\n::end\n';

test('references cross the boundary: listed, then resolved from provided resources', () => {
  const refs = JSON.parse(engine.references(REVIEW));
  assert.deepEqual(refs, [
    { kind: 'file', path: 'src/lib.rs' },
    { kind: 'document', path: 'plan.dx' },
  ]);

  const resources = JSON.stringify({
    files: { 'src/lib.rs': 'pub fn answer() -> u32 { 42 }\n' },
    documents: { 'plan.dx': '::paragraph id=step\nShip it.\n::end\n' },
  });
  const page = engine.render_html(REVIEW, 'light', true, resources);
  assert.match(page, /pub fn answer/, 'the listing is the file, not the empty body');
  assert.match(page, /Ship it\./, 'the board node shows the sibling document’s block');

  const bare = engine.render_html(REVIEW, 'light', true);
  assert.match(bare, /could not be read here/, 'no resources means a sentence, never silence');
});

const TODO = '::checklist id=todo\n[ ] sketch it\n[x] ship it\n::end\n';

test('a box ticks and unticks across the boundary, and nothing else moves', () => {
  const ticked = engine.toggle_check(TODO, 'todo', 0);
  assert.equal(ticked, TODO.replace('[ ] sketch it', '[x] sketch it'));
  assert.equal(engine.toggle_check(ticked, 'todo', 0), TODO);
});

test('every box on the page says which item it is, so a click can name one', () => {
  const page = engine.render_html(TODO, 'light', true);
  assert.match(page, /class="dx-mark" data-check="0"/);
  assert.match(page, /class="dx-mark" data-check="1"/);
});

test('ticking a box that is not there arrives as a thrown sentence', () => {
  assert.throws(() => engine.toggle_check(TODO, 'todo', 7), /has 2 items/);
  assert.throws(() => engine.toggle_check(SAMPLE, 'intro', 0), /only a checklist/);
});

test('a board renders as a viewport whose nodes carry the document blocks', () => {
  const page = engine.render_html(BOARD, 'light', true);
  assert.match(page, /class="dx-board"/);
  // A node is drawn at the box its line states — all four numbers, nothing measured.
  assert.match(page, /data-node-id="ideas" style="left:40px;top:40px;width:280px;height:160px"/);
  assert.match(page, /Rough ideas\./);
});

test('a node placed on top of another pushes it down, so nothing is left covered', () => {
  const two = engine.board_arrange(BOARD, 'plan', 'ideas,0,0,280,200 notes,0,400,280,200');
  const dropped = JSON.parse(engine.board_place(two, 'plan', 'notes', 0, 40, 280, 200));
  // `notes` keeps exactly where it was put; `ideas` falls 28px clear of its bottom.
  assert.match(dropped.source, /- notes x=0 y=40 w=280 h=200/);
  assert.match(dropped.source, /- ideas x=0 y=268 w=280 h=200/);
});

test('a board is laid out in one call, and its edges keep the sides they were drawn between', () => {
  // Every box in one edit — and a node the board had no line for arrives with its block.
  const settled = engine.board_arrange(BOARD, 'plan', 'ideas,0,0,300,160 sketch,0,340,280,200');
  assert.match(settled, /- ideas x=0 y=0 w=300 h=160/);
  assert.match(settled, /- sketch x=0 y=340 w=280 h=200/);
  assert.match(settled, /::paragraph id=sketch hidden/);

  // The two sides reach the page only where the line pinned them; an unpinned end is left
  // for the surface to route against the boxes it can measure.
  const linked = engine.board_link(settled, 'plan', 'ideas', 'sketch', true, 'bottom', '');
  assert.match(linked, /to=sketch:b-\./);
  const page = engine.render_html(linked, 'light', true);
  assert.match(page, /data-from-side="b"/);
  assert.doesNotMatch(page, /data-to-side/);
});

test('a refused board call arrives as a thrown sentence', () => {
  assert.throws(
    () => engine.board_place(BOARD, 'ideas', 'x', 0, 0, 0, 0),
    /not a board/
  );
  assert.throws(() => engine.board_arrange(BOARD, 'plan', 'ideas,0'), /node,x,y/);
});

test('a field decorates with every character kept', () => {
  const out = engine.field_html('**loud** and `code`');
  // The marks are visible and the meaning is styled — both, not either.
  assert.match(out, /<strong>loud<\/strong>/);
  assert.match(out, /dx-mark/);
  assert.match(out, /<code>code<\/code>/);
  // Stripping the tags yields the exact source: the invariant caret math stands on.
  assert.equal(out.replace(/<[^>]*>/g, ''), '**loud** and `code`');
});

test('a block hands back the header line the writer put in the file', () => {
  assert.equal(engine.block_header(SAMPLE, 'title'), '::heading level=1 id=title');
  assert.equal(engine.block_header(SAMPLE, 'intro'), '::paragraph id=intro');
});

test('replacing with the shown header and body changes nothing', () => {
  const canonical = engine.set_block(SAMPLE, 'intro', engine.block_source(SAMPLE, 'intro'));
  for (const id of ['title', 'intro', 'points']) {
    const replaced = JSON.parse(
      engine.replace_block(canonical, id, engine.block_header(canonical, id), engine.block_source(canonical, id))
    );
    assert.equal(replaced.source, canonical, `round trip moved \`${id}\``);
    assert.equal(replaced.id, id);
  }
});

test('a rewritten header retypes the block and keeps its id', () => {
  const replaced = JSON.parse(engine.replace_block(SAMPLE, 'intro', '::quote', 'The opening line.'));
  assert.equal(replaced.id, 'intro');
  assert.match(replaced.source, /::quote id=intro/);
});

test('an unknown kind in a header is refused with a sentence, not folded to prose', () => {
  assert.throws(
    () => engine.replace_block(SAMPLE, 'intro', '::codee', 'x'),
    /::codee/
  );
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
  const drawn = engine.preview_block(SAMPLE, 'intro', 'A *new* line.', 'light');
  assert.match(drawn, /<em>new<\/em>/);
  assert.doesNotMatch(drawn, /Guide/, 'the preview carried the document around it');
  assert.doesNotMatch(drawn, /dx-doc/, 'the preview carried the sheet it sits on');
  // A preview is a read. The source it was given is the source it leaves behind.
  assert.equal(engine.block_source(SAMPLE, 'intro'), 'The opening line.');
});

test('a previewed block is drawn exactly as the page draws it', () => {
  const saved = engine.set_block(SAMPLE, 'points', 'one\ntwo\nthree');
  const page = engine.render_html(saved, 'light', true);
  const drawn = engine.preview_block(SAMPLE, 'points', 'one\ntwo\nthree', 'light');
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
