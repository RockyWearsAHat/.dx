import path from 'node:path';
import * as esbuild from 'esbuild';

const repoRoot = process.cwd();
const entryPoint = path.join(repoRoot, 'vscode-extension', 'extension.ts');
const outfile = path.join(repoRoot, 'build', 'runtime', 'vscode-extension', 'extension.cjs');

await esbuild.build({
  entryPoints: [entryPoint],
  outfile,
  bundle: false,
  minify: false,
  platform: 'node',
  format: 'cjs',
  target: 'node23',
  logLevel: 'silent',
});
