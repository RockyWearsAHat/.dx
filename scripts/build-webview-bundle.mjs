import path from 'node:path';
import * as esbuild from 'esbuild';

const repoRoot = process.cwd();
const entryPoint = path.join(repoRoot, 'build', 'runtime', 'vscode-extension', 'media', 'webview-main.js');
const bundleOutput = path.join(repoRoot, 'build', 'docdb-webview.bundle.min.js');

await esbuild.build({
  entryPoints: [entryPoint],
  outfile: bundleOutput,
  bundle: true,
  minify: true,
  platform: 'browser',
  format: 'iife',
  target: 'es2020',
  legalComments: 'none',
  logLevel: 'silent',
});
