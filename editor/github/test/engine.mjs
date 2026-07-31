// Loading, under node, both builds of the one engine.
//
// `doc-wasm` is compiled twice, because no single artifact loads in both hosts: the browser
// extension needs `no-modules` (a manifest content script cannot be an ES module, so the file
// defines a `wasm_bindgen` global), and the VS Code extension needs `nodejs` (CommonJS). Both
// are built together by `editor/build.sh`, and both are loadable here — which is the point.
// Whatever these tests assert about rendering, they assert about *every* build a person
// actually runs, rather than about one of two and a hope about the other.

import { createRequire } from 'node:module';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);

/// Where each build lands. `github` is what a browser runs; `vscode` is what the editor
/// requires. Both come out of `editor/build.sh`, from the same crate, in one command.
const BUILDS = {
  github: join(here, '../wasm'),
  vscode: join(here, '../../vscode/wasm'),
};

/// Every build's name, for a test that has to cover all of them.
export const BUILD_NAMES = Object.keys(BUILDS);

/// The bytes of a build's wasm, or `null` when that build has not been made.
export function wasmBytes(build = 'github') {
  const file = join(BUILDS[build], 'doc_wasm_bg.wasm');
  return existsSync(file) ? readFileSync(file) : null;
}

/// The bytes of the wasm the browser extension ships.
export function shippedWasmBytes() {
  return wasmBytes('github');
}

/// One build of the engine, initialized, or `null` when it has not been built.
///
/// Returns the module namespace — the same object `engine.js` resolves to in a browser and
/// the editor `require`s — with `pack_document`, `render_html`, `stylesheet`, and the rest as
/// plain functions. The two builds differ only in how they are loaded, and hiding that here
/// is deliberate: everything downstream then treats them as the one engine they are meant to
/// be, and a test written once covers both.
export async function loadEngine(build = 'github') {
  const dir = BUILDS[build];
  const glue = join(dir, 'doc_wasm.js');
  const wasm = join(dir, 'doc_wasm_bg.wasm');
  if (!existsSync(glue) || !existsSync(wasm)) return null;

  // The `nodejs` target is CommonJS and loads its own wasm when required.
  if (build === 'vscode') return require(glue);

  // The `no-modules` glue declares `wasm_bindgen` with `let`, which never lands on
  // `globalThis`, so it is returned out of the evaluated body instead.
  const api = new Function(`${readFileSync(glue, 'utf8')}\nreturn wasm_bindgen;`)();
  await api({ module_or_path: new Uint8Array(readFileSync(wasm)) });
  return api;
}

/// The engine the browser extension ships, initialized, or `null` when it is not built.
export function loadShippedEngine() {
  return loadEngine('github');
}
