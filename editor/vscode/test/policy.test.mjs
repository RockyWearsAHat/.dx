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

test('the geometry engine may compile, and nothing gains the power to eval a string', () => {
  const policy = scriptPolicy('abc');

  // The one widening this webview carries, and it is asserted explicitly rather than left
  // to a silent diff: the geometry engine (`editor/surface/doc_wasm.js`, inlined the same
  // way `edit.js` is) needs `'wasm-unsafe-eval'` to compile at all.
  assert.match(
    policy,
    /script-src [^;]*'wasm-unsafe-eval'/,
    'the geometry engine is WebAssembly, which will not compile without it'
  );
  // `'wasm-unsafe-eval'` is not a script source and grants no origin, no inline script, and
  // none of `eval`/`Function(string)`/`setTimeout(string)` — those need the unrelated
  // `'unsafe-eval'`, which a policy carrying only the wasm grant must still refuse. The
  // substring `'unsafe-eval'` (quote to quote) never appears inside `'wasm-unsafe-eval'`,
  // so this is the same check as the plain-`'unsafe-eval'` assertion above, stated again
  // beside the grant it is guarding.
  assert.doesNotMatch(
    policy,
    /script-src [^;]*'unsafe-eval'/,
    'compiling wasm is not evaluating strings, and the page must get neither'
  );
});

test('each render authorizes its scripts with a nonce nobody can guess', () => {
  const nonce = makeNonce();

  assert.match(nonce, /^[0-9a-f]{32}$/, 'a nonce is 16 random bytes, written as hex');
  assert.notEqual(nonce, makeNonce(), 'two renders were given the same nonce');
});
