/**
 * The host contract, against the hosts that implement it.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * `editor/surface/edit.js` is the one editor, and its module doc states the calls it asks
 * of a host. Two hosts answer: the VS Code webview (`editor/vscode/src/preview.ts`) and
 * DX.app (`packaging/app/Editor.swift`). Three statements of one contract, in three
 * languages, in three places nothing linked together — so it drifted: both hosts carried a
 * `draw` op the contract never listed and the editor never called, dead surface in a Swift
 * file and a TypeScript file at once.
 *
 * This file holds them equal. The contract is what `edit.js` *calls* — the ground truth,
 * read out of the source rather than its prose — and each host must offer exactly that: no
 * missing call, which is a control that silently does nothing on one surface, and no extra
 * one, which is code no reader will ever reach.
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..', '..');

/** Read one repository file as text. */
const source = (...parts) => readFileSync(join(root, ...parts), 'utf8');

const EDITOR = source('editor', 'surface', 'edit.js');

/** Everything below the module doc: the code, without the prose that describes it. */
const CODE = EDITOR.slice(EDITOR.indexOf('(function ()'))
  .split('\n')
  .filter((line) => !/^\s*(\*|\/\/|\/\*)/.test(line))
  .join('\n');

/**
 * The calls `edit.js` makes on its host — `host.commit(…)`, the `host.run &&` guards that
 * ask whether an optional one is offered, and the ones written `host\n  .remove(id)`.
 *
 * The lookbehind is what keeps `ghost.style` out: this file has a `ghost` element, and a
 * looser pattern reports its properties as host calls.
 */
function contract() {
  return new Set(Array.from(CODE.matchAll(/(?<![\w$.])host\s*\.\s*([a-z]+)\b/g), (m) => m[1]));
}

/** The op names one host's `attach({…})` block offers, in whatever language it is written. */
function offered(text) {
  const at = text.indexOf('attach({');
  assert.ok(at >= 0, 'a host must attach the editor');
  const block = text.slice(at, text.indexOf('});', at));
  return new Set(Array.from(block.matchAll(/op:\s*'([a-z]+)'/g), (m) => m[1]));
}

const CONTRACT = contract();

test('the editor asks for the calls its own documentation states', () => {
  // The prose is what a person implementing a host reads, so it is held to the code too.
  const doc = EDITOR.slice(0, EDITOR.indexOf('(function ()'));
  const documented = new Set(Array.from(doc.matchAll(/^ \* - `([a-z]+)\(/gm), (m) => m[1]));
  assert.deepEqual([...documented].sort(), [...CONTRACT].sort());
});

for (const [host, file] of [
  ['the VS Code webview', ['editor', 'vscode', 'src', 'preview.ts']],
  ['DX.app', ['packaging', 'app', 'Editor.swift']],
]) {
  test(`${host} offers exactly the calls the editor makes`, () => {
    assert.deepEqual([...offered(source(...file))].sort(), [...CONTRACT].sort());
  });
}
