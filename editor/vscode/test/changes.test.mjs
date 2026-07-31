/**
 * Resolving the two sides of an in-editor comparison, without spawning anything.
 *
 *   node --test "editor/vscode/test/*.test.mjs"
 *
 * `src/changes.ts` is the half of "DX: Open Changes" that has no editor in it: bytes in, the
 * document those bytes stand for out. Both programs it needs — `dx` and `git` — arrive as a
 * seam, so these tests hand it runners that spawn nothing and assert what it asked them for.
 *
 * What matters here is that this module never decides what a pointer *is*: every side goes to
 * `dx textconv`, the same driver git runs, and a side that cannot be resolved comes back as
 * dx's own sentence rather than as an empty document.
 */

import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import test from 'node:test';

import { documentText, revisionText } from '../src/changes.ts';

const POINTER = '~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823\n';
const DOCUMENT = '::heading level=1 id=title\nGuide\n::end\n';

/**
 * A `dx` that resolves every pointer to `text`, recording each call in `calls`.
 *
 * It reads the file it was handed while that file still exists — the scratch directory is
 * gone by the time the caller has its answer, which is the point of the last test below.
 */
function dxWriting(text, calls) {
  return async (args, directory) => {
    const output = args[args.indexOf('--out') + 1];
    calls.push({ args, directory, handed: await readFile(args[1], 'utf8') });
    await writeFile(output, text, 'utf8');
    return { ok: true, output: `wrote ${output}\n` };
  };
}

test('a side is resolved by the same driver git runs, against the document’s own workspace', async () => {
  const calls = [];
  const resolved = await documentText(POINTER, '/work/notes', dxWriting(DOCUMENT, calls));

  assert.equal(resolved, DOCUMENT, 'the document did not come back byte for byte');
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args[0], 'textconv');
  assert.equal(calls[0].args[calls[0].args.indexOf('--root') + 1], '/work/notes');
  assert.equal(calls[0].directory, '/work/notes');
});

test('the bytes reach dx in a file, exactly as they were handed in', async () => {
  const calls = [];
  await documentText(POINTER, '/work/notes', dxWriting(DOCUMENT, calls));

  // A throwaway file, because that is how git hands a blob to the same driver.
  assert.equal(calls[0].handed, POINTER);
});

test('nothing is left behind on disk, on either outcome', async () => {
  const calls = [];
  await documentText(POINTER, '/work/notes', dxWriting(DOCUMENT, calls));
  assert.equal(existsSync(dirname(calls[0].args[1])), false, 'a resolved side kept its scratch');

  const failed = [];
  const refusing = async (args, directory) => {
    failed.push({ args, directory });
    return { ok: false, output: 'no' };
  };
  await assert.rejects(() => documentText(POINTER, '/work/notes', refusing));
  assert.equal(existsSync(dirname(failed[0].args[1])), false, 'a failed side kept its scratch');
});

test('text that is already a document is dx’s to recognize, not this module’s', async () => {
  const calls = [];
  const resolved = await documentText(DOCUMENT, '/work/notes', dxWriting(DOCUMENT, calls));

  assert.equal(calls.length, 1, 'a plain document was answered without asking dx');
  assert.equal(resolved, DOCUMENT);
});

test('a side that cannot be resolved is dx’s sentence, never an empty document', async () => {
  const sentence =
    'this dx pointer names version c939d5be…, which is not in /work/notes’s store or packs; ' +
    'run `dx sync` there, or restore .doc/repo.dxcp';
  const refusing = async () => ({ ok: false, output: sentence });

  await assert.rejects(
    () => documentText(POINTER, '/work/notes', refusing),
    (error) => error.message === sentence
  );
});

test('the older side is read out of git, from the document’s own directory', async () => {
  const calls = [];
  const git = async (args, directory) => {
    calls.push({ args, directory });
    return { ok: true, output: POINTER };
  };

  assert.equal(await revisionText('/work/notes/guide.dx', 'HEAD', git), POINTER);
  // `<rev>:./name` is git's own relative form, so no path arithmetic here has to agree with
  // where the work tree begins.
  assert.deepEqual(calls[0].args, ['show', 'HEAD:./guide.dx']);
  assert.equal(calls[0].directory, '/work/notes');
});

test('a revision that does not carry the file answers with git’s sentence', async () => {
  const sentence = "fatal: path 'guide.dx' does not exist in 'HEAD'";
  const git = async () => ({ ok: false, output: sentence });

  await assert.rejects(
    () => revisionText('/work/notes/guide.dx', 'HEAD', git),
    (error) => error.message === sentence
  );
});
