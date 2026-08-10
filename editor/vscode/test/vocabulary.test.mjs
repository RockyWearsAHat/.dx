/**
 * The editing surface's vocabulary, against the engine that decides it.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * `editor/surface/edit.js` carries a few facts as constants — which kinds are editable,
 * what a tag line may say, which attributes are flags, the smallest box a node may be
 * dragged to — because a completion menu and a drag handle need them before any call goes
 * out. Carrying them is fine; deciding them is not. `doc-core::surface` decides them, and
 * `doc-wasm`'s `vocabulary()` is how a host reads that decision.
 *
 * This file holds the two equal. It reads the surface's own source (the one copy, before
 * any build step has copied it into a host) and compares each list to the engine's answer.
 * The pair had already drifted once: a node could be dragged no smaller than 60px tall
 * while the renderer's own `h=fit` floor was 56, so the smallest box a reader could make
 * was a box the renderer would never have chosen.
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const engine = createRequire(import.meta.url)(join(here, '..', 'wasm', 'doc_wasm.js'));
const SURFACE = readFileSync(join(here, '..', '..', 'surface', 'edit.js'), 'utf8');

const vocabulary = JSON.parse(engine.vocabulary());

/** The strings of the `const NAME = [ … ]` array literal `edit.js` declares. */
function array(name) {
  const start = SURFACE.indexOf(`const ${name} = [`);
  assert.notEqual(start, -1, `edit.js no longer declares ${name}`);
  const open = SURFACE.indexOf('[', start);
  const body = SURFACE.slice(open + 1, SURFACE.indexOf(']', open));
  return [...body.matchAll(/'([^']*)'/g)].map((match) => match[1]);
}

/** The `key: [ … ]` entries of the `const NAME = { … }` object literal `edit.js` declares. */
function object(name) {
  const start = SURFACE.indexOf(`const ${name} = {`);
  assert.notEqual(start, -1, `edit.js no longer declares ${name}`);
  const body = SURFACE.slice(SURFACE.indexOf('{', start) + 1, SURFACE.indexOf('};', start));
  const entries = {};
  for (const [, key, list] of body.matchAll(/(\w+):\s*\[([^\]]*)\]/g)) {
    entries[key] = [...list.matchAll(/'([^']*)'/g)].map((match) => match[1]);
  }
  return entries;
}

/** The number a `const NAME = 1.5;` declaration states. */
function number(name) {
  const match = SURFACE.match(new RegExp(`const ${name} = (-?[\\d.]+);`));
  assert.notEqual(match, null, `edit.js no longer declares ${name}`);
  return Number(match[1]);
}

test('the engine answers with the whole vocabulary', () => {
  for (const key of ['editable', 'readOnly', 'multiline', 'source', 'authorable', 'kinds', 'bareAttrs']) {
    assert.ok(Array.isArray(vocabulary[key]) && vocabulary[key].length > 0, `no ${key}`);
  }
  assert.equal(typeof vocabulary.attrs, 'object');
  assert.equal(typeof vocabulary.node, 'object');
});

test('the surface offers exactly the kinds the engine names', () => {
  assert.deepEqual(array('EDITABLE'), vocabulary.editable);
  assert.deepEqual(array('READ_ONLY'), vocabulary.readOnly);
  assert.deepEqual(array('MULTILINE'), vocabulary.multiline);
  assert.deepEqual(array('SOURCE'), vocabulary.source);
  assert.deepEqual(array('BARE_ATTRS'), vocabulary.bareAttrs);
});

test('the tag line completes to the headers the engine keeps', () => {
  // KINDS is written with the `::` a person types; the engine states the same lines.
  assert.deepEqual(array('KINDS'), vocabulary.kinds);
});

test('each kind is offered the attributes the engine gives it', () => {
  assert.deepEqual(object('ATTRS'), vocabulary.attrs);
});

test('the source grammar colours every flag the engine names', () => {
  // The tag line's flags are highlighted by an alternation in the tmLanguage grammar, which
  // is a third copy of the same list — it had been missing `open` since the attribute was
  // added, so a folded-open code block read as unhighlighted text in the source view.
  const grammar = JSON.parse(
    readFileSync(join(here, '..', 'syntaxes', 'dx.tmLanguage.json'), 'utf8')
  );
  const [flags] = grammar.repository.attributes.patterns;
  const highlighted = flags.match.replace(/\\b|[()]/g, '').split('|');

  assert.deepEqual(highlighted.toSorted(), [...vocabulary.bareAttrs].toSorted());
});

test('a node drags to the box the renderer would have chosen', () => {
  assert.equal(number('NODE_MIN_WIDTH'), vocabulary.node.minWidth);
  assert.equal(number('NODE_MIN_HEIGHT'), vocabulary.node.minHeight);
  assert.equal(number('FIT_MAX'), vocabulary.node.fitMax, 'a fit magnifies past the static render');
  assert.equal(number('BOARD_PADDING'), vocabulary.node.padding);
});
