/**
 * Talking to the `dx` command line for the two things a webview cannot do itself:
 * executing code blocks and writing image files.
 *
 * Viewing never needs the CLI — rendering happens in WebAssembly inside the editor. So a
 * missing `dx` binary degrades one feature rather than breaking the extension, and the
 * message says exactly how to fix it.
 */

import { execFile } from 'child_process';
import * as path from 'path';
import * as vscode from 'vscode';

/** What a CLI invocation produced. */
export interface CliResult {
  ok: boolean;
  output: string;
}

/** The configured path to the `dx` binary. */
export function cliPath(): string {
  return vscode.workspace.getConfiguration('dx').get<string>('cliPath', 'dx');
}

/**
 * Run `dx` with `args` in the document's directory.
 *
 * Resolves rather than rejects on failure: the caller shows the message to the reader, and
 * a non-zero exit from `dx run` (a code block that failed) is a result, not a crash.
 */
export function runCli(args: string[], documentPath: string): Promise<CliResult> {
  return new Promise((resolve) => {
    execFile(
      cliPath(),
      args,
      { cwd: path.dirname(documentPath), maxBuffer: 16 * 1024 * 1024 },
      (error, stdout, stderr) => {
        const output = `${stdout}${stderr}`.trim();
        if (error && (error as NodeJS.ErrnoException).code === 'ENOENT') {
          resolve({ ok: false, output: missingCliMessage() });
          return;
        }
        resolve({ ok: !error, output: output || (error ? String(error) : '') });
      }
    );
  });
}

/** The message shown when the `dx` binary cannot be found. */
export function missingCliMessage(): string {
  return (
    `Could not find the "${cliPath()}" command. ` +
    'Install the dx CLI and run `dx install`, or set "dx.cliPath" to its full path. ' +
    'Viewing documents does not need it — only running code and exporting images do.'
  );
}
