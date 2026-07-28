/**
 * Talking to the `dx` command line for the two things a webview cannot do itself:
 * executing code blocks and writing image files.
 *
 * Viewing never needs the CLI — rendering happens in WebAssembly inside the editor. So a
 * missing `dx` binary degrades one feature rather than breaking the extension, and the
 * message says exactly how to fix it.
 */

import { execFile } from 'child_process';
import { existsSync } from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';

/** What a CLI invocation produced. */
export interface CliResult {
  ok: boolean;
  output: string;
}

/**
 * Places `dx install` puts the binary, checked when it is not on `PATH`.
 *
 * An editor launched from the desktop does not always inherit the `PATH` a terminal has, so
 * `dx` can be installed and working in a shell yet invisible here. Looking in the handful of
 * locations the installer actually uses turns that puzzle into a non-event.
 */
function installLocations(): string[] {
  const home = os.homedir();
  if (process.platform === 'win32') {
    const local = process.env.LOCALAPPDATA ?? path.join(home, 'AppData', 'Local');
    return [path.join(local, 'Programs', 'dx', 'dx.exe')];
  }
  return [
    path.join(home, '.local', 'bin', 'dx'),
    '/usr/local/bin/dx',
    '/opt/homebrew/bin/dx',
  ];
}

/** The configured path to the `dx` binary. */
export function cliPath(): string {
  return vscode.workspace.getConfiguration('dx').get<string>('cliPath', 'dx');
}

/**
 * The command to actually spawn: the configured value if it names a real file or is meant
 * to be resolved through `PATH`, otherwise the first known install location that exists.
 */
function resolveCli(): string {
  const configured = cliPath();
  if (configured.includes(path.sep) || existsSync(configured)) {
    return configured;
  }
  return installLocations().find(existsSync) ?? configured;
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
      resolveCli(),
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
    `Could not find the "${cliPath()}" command, or any dx binary in ` +
    `${installLocations().join(', ')}. ` +
    'Install the dx CLI and run `dx install`, or set "dx.cliPath" to its full path. ' +
    'Viewing documents does not need it — only running code and exporting images do.'
  );
}
