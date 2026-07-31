/**
 * Comparing two versions of a `.dx` document *as documents*.
 *
 * A `.dx` file on disk is a one-line pointer into the store, so VS Code's own "Open Changes"
 * shows two digests differing: git applies the `dx` diff driver, and the editor's git
 * integration does not. This module is the resolving half of the answer — given the bytes of
 * one side, hand back the text those bytes stand for.
 *
 * Resolution is `dx textconv`, the very driver git runs, so nothing here knows what a pointer
 * looks like or where the store keeps content. Plain document text passes through it
 * unchanged, and a pointer whose content is missing comes back as dx's own sentence saying
 * what to run — never as an empty document.
 *
 * Nothing in this file imports the editor, which is what lets it be tested under plain node.
 */

import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';

import type { CliResult } from './cli';

/**
 * A program run with arguments in a directory, resolving with what it printed.
 *
 * The shape `runCli` already has, so `dx` reaches this module as the extension's one
 * CLI seam — and a test can hand it a runner that spawns nothing.
 */
export type Run = (args: string[], directory: string) => Promise<CliResult>;

/**
 * Run `git` with `args` in `directory`.
 *
 * Resolves rather than rejects on failure: git's own sentence — "does not exist in 'HEAD'" —
 * is what the reader needs to see, and it arrives on standard error with a non-zero exit.
 */
export const runGit: Run = (args, directory) =>
  new Promise((resolve) => {
    execFile(
      'git',
      args,
      { cwd: directory, maxBuffer: 16 * 1024 * 1024 },
      (error, stdout, stderr) => {
        const failure = `${stderr}`.trim() || (error ? String(error) : '');
        resolve({ ok: !error, output: error ? failure : stdout });
      }
    );
  });

/**
 * The bytes `file` had at `revision`, straight out of git — a pointer, most likely.
 *
 * The revision is read with git's own `<rev>:./<name>` form from the file's directory, so no
 * path arithmetic here has to agree with where the work tree begins. Rejects with git's
 * sentence when that revision does not carry the file, because a document that is not there
 * is a thing to say, not an empty page to show.
 */
export async function revisionText(
  file: string,
  revision: string,
  git: Run = runGit
): Promise<string> {
  const result = await git(['show', `${revision}:./${basename(file)}`], dirname(file));
  if (!result.ok) {
    throw new Error(result.output || `git could not read ${basename(file)} at ${revision}.`);
  }
  return result.output;
}

/**
 * The document `text` stands for, resolved against the store nearest `directory`.
 *
 * `dx textconv` resolves by the digest written inside the pointer rather than by path — the
 * reason it can read a version that is no longer anyone's working copy — so the bytes are
 * handed to it in a throwaway file exactly as git does. `--out` carries the answer back in a
 * file too, so the document arrives byte for byte instead of through a trimmed pipe.
 *
 * Rejects with dx's own sentence when the pointer cannot be resolved.
 */
export async function documentText(
  text: string,
  directory: string,
  dx: Run
): Promise<string> {
  const folder = await mkdtemp(join(tmpdir(), 'dx-changes-'));
  const input = join(folder, 'pointer.dx');
  const output = join(folder, 'document.dx');
  try {
    await writeFile(input, text, 'utf8');
    const result = await dx(['textconv', input, '--root', directory, '--out', output], directory);
    if (!result.ok) {
      throw new Error(result.output || `dx could not resolve this document in ${directory}.`);
    }
    return await readFile(output, 'utf8');
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
}
