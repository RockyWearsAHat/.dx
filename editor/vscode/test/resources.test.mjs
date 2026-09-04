/**
 * Gathering a document's references off the filesystem, without an editor.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * `src/engine.ts`'s `resourcesFor` is the half of rendering that only the host can do: the
 * engine says what is still missing and this fetches it, round after round, until nothing is.
 * Everything about "a reader sees the listing, not an empty block" arrives through it — and
 * the rule it exists to keep is the repository's first one: a pointer is never shown where a
 * document belongs. A `.dx` that turns out to be one line into the store is resolved through
 * the committed pack, and when no pack holds it the block becomes the engine's sentence,
 * never the pointer line itself.
 *
 * These tests build a real folder and let the real `fs` read it. Nothing is stubbed, because
 * the thing being tested is exactly the reading.
 */

import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { resourcesFor } from '../dist/engine.js';

const here = dirname(fileURLToPath(import.meta.url));
const dx = createRequire(import.meta.url)(join(here, '..', 'wasm', 'doc_wasm.js'));
const FIXTURE_PACK = join(here, '..', '..', 'github', 'test', 'fixture', 'repo.dxcp');

/** A scratch folder holding `files`, removed when the test ends. */
function folder(t, files) {
  const root = mkdtempSync(join(tmpdir(), 'dx-resources-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const [path, content] of Object.entries(files)) {
    const target = join(root, path);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, content);
  }
  return root;
}

test('a document that references nothing gathers nothing', (t) => {
  const root = folder(t, {});
  assert.equal(resourcesFor('::paragraph id=p\nPlain.\n::end\n', root, dx), undefined);
});

test('a listing is gathered as the file’s current text', (t) => {
  const root = folder(t, { 'src/lib.rs': 'pub fn answer() -> u32 { 42 }\n' });
  const held = JSON.parse(
    resourcesFor('::code id=listing src=src/lib.rs lang=rust\n::end\n', root, dx)
  );
  assert.equal(held.files['src/lib.rs'], 'pub fn answer() -> u32 { 42 }\n');
  assert.deepEqual(held.absent, []);
});

/**
 * The second round, and the reason the gathering is a loop: the stylesheet is named by the
 * fetched page, not by the document, so a host that read once would frame the view undressed.
 */
test('a gathered page brings the stylesheets it names in turn', (t) => {
  const root = folder(t, {
    'site/index.html': '<link rel="stylesheet" href="look.css">\n<h1>Hello</h1>\n',
    'site/look.css': 'body { color: #222 }\n',
  });
  const held = JSON.parse(resourcesFor('::view id=page src=site/index.html\n::end\n', root, dx));
  assert.ok(held.files['site/index.html']);
  assert.equal(held.files['site/look.css'], 'body { color: #222 }\n');
});

test('a reference that cannot be read is given up on, and the walk still ends', (t) => {
  const root = folder(t, { 'site/index.html': '<link rel="stylesheet" href="gone.css">\n' });
  const held = JSON.parse(resourcesFor('::view id=page src=site/index.html\n::end\n', root, dx));
  assert.deepEqual(held.absent, ['site/gone.css']);
  assert.equal(held.files['site/gone.css'], undefined);
});

const BOARD = '::board id=map height=300\n- welcome.dx#title x=20 y=20 w=200 h=90\n::end\n';
const POINTER = '~ dx1 0000000000000000000000000000000000000000000000000000000000000000\n';

test('a sibling pointer is resolved through the committed pack', (t) => {
  const root = folder(t, { 'welcome.dx': POINTER });
  mkdirSync(join(root, '.doc'));
  copyFileSync(FIXTURE_PACK, join(root, '.doc', 'repo.dxcp'));

  const held = JSON.parse(resourcesFor(BOARD, root, dx));
  const source = held.documents['welcome.dx'];
  assert.ok(source, 'the pointer resolved to nothing');
  assert.doesNotMatch(source, /^~ dx1 /, 'the pointer line was handed back as the document');
  assert.match(source, /^::/, 'the pack answered with something that is not DOCSRC');
});

/**
 * The failure that must not become silence. With no pack there is no document — and the one
 * thing the host may not do is pass the pointer line off as content, which would render a
 * board node reading `~ dx1 …` to someone who asked for a document.
 */
test('a pointer no pack holds is absent, never the pointer line', (t) => {
  const root = folder(t, { 'welcome.dx': POINTER });
  assert.equal(resourcesFor(BOARD, root, dx), undefined);
});

test('a sibling document that is plain text is gathered as itself', (t) => {
  const root = folder(t, { 'welcome.dx': '::heading level=1 id=title\nWelcome\n::end\n' });
  const held = JSON.parse(resourcesFor(BOARD, root, dx));
  assert.match(held.documents['welcome.dx'], /Welcome/);
});
