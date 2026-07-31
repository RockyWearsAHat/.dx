/**
 * The webview's content-security policy, and the nonce that authorizes what may run in it.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * The policy is the one thing standing between a document somebody else wrote and the reader's
 * editor, and it is a string — so it is asserted rather than read and hoped for. `src/policy.ts`
 * is loaded directly, which node does for TypeScript that is only types on top of JavaScript.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { makeNonce, scriptPolicy } from '../src/policy.ts';

test('a rendered document may show inline images and reach nothing else', () => {
  const policy = scriptPolicy('abc');

  assert.match(policy, /default-src 'none'/);
  assert.match(policy, /img-src data:/, 'inline images are how a document carries a drawing');
  assert.doesNotMatch(policy, /https:/, 'the page was allowed to fetch from the network');
  assert.doesNotMatch(policy, /http:/);
  assert.doesNotMatch(policy, /'unsafe-eval'/);
  // Styles are the document's own; scripts are only ever the extension's, named one by one.
  assert.match(policy, /script-src 'nonce-abc'/);
  assert.doesNotMatch(policy, /script-src [^;]*'unsafe-inline'/);
});

test('each render authorizes its scripts with a nonce nobody can guess', () => {
  const nonce = makeNonce();

  assert.match(nonce, /^[0-9a-f]{32}$/, 'a nonce is 16 random bytes, written as hex');
  assert.notEqual(nonce, makeNonce(), 'two renders were given the same nonce');
});
