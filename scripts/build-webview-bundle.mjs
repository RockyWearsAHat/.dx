import fs from 'node:fs';
import path from 'node:path';
import * as esbuild from 'esbuild';

const repoRoot = process.cwd();
const entryPoint = path.join(repoRoot, 'build', 'runtime', 'vscode-extension', 'media', 'webview-main.js');
const bundleOutput = path.join(repoRoot, 'build', 'docdb-webview.bundle.min.js');

// The compiled webview imports the Rust doc-core wasm glue from `./wasm/doc_wasm.js`. That
// generated module lives in source under `vscode-extension/media/wasm/` (gitignored, produced by
// `wasm-pack build --target web`), not under `build/runtime`, so esbuild cannot resolve it from
// the compiled tree. A resolver plugin redirects the bare specifier to the real source file. If
// the wasm build was never run, it falls back to a stub that disables the wasm path so the bundle
// still builds and the editor uses the TypeScript parser.
const wasmSource = path.join(repoRoot, 'vscode-extension', 'media', 'wasm', 'doc_wasm.js');
const wasmStub = path.join(repoRoot, 'scripts', 'doc-wasm-stub.js');
const wasmGlue = fs.existsSync(wasmSource) ? wasmSource : wasmStub;
if (wasmGlue === wasmStub) {
  console.warn('[build-webview-bundle] media/wasm/doc_wasm.js not found; bundling wasm stub (TS parser fallback).');
}

const wasmResolverPlugin = {
  name: 'doc-core-wasm-resolver',
  setup(build) {
    build.onResolve({ filter: /(^|\/)wasm\/doc_wasm\.js$/ }, () => ({ path: wasmGlue }));
  },
};

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
  plugins: [wasmResolverPlugin],
});
