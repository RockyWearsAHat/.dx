import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeBlockSourceInput } from '#runtime-media/webview-block-source-normalizer.js';

test('normalizer converts markdown heading to heading block', () => {
  const next = normalizeBlockSourceInput('# Launch Plan');
  assert.equal(next, '::heading level=1\nLaunch Plan\n::end');
});

test('normalizer converts fenced code with language to code block', () => {
  const next = normalizeBlockSourceInput('```js\nconst x = 1;\n```');
  assert.equal(next, '::code language=js\nconst x = 1;\n::end');
});

test('normalizer converts checklist markdown to checklist block', () => {
  const next = normalizeBlockSourceInput('- [x] done\n- [ ] next');
  assert.equal(next, '::checklist\n[x] done\n[ ] next\n::end');
});

test('normalizer converts html paragraph to paragraph block', () => {
  const next = normalizeBlockSourceInput('<p>Hello world</p>');
  assert.equal(next, '::paragraph\nHello world\n::end');
});

test('normalizer preserves canonical block sources', () => {
  const next = normalizeBlockSourceInput('::quote\nHi\n::end');
  assert.equal(next, '::quote\nHi\n::end');
});
