// Parity harness: proves the wasm-compiled doc-core produces byte-for-byte identical
// output to the TypeScript reference for the operations exposed across the JS boundary.
//
// It loads the wasm pkg (CommonJS, built with `wasm-pack --target nodejs`) via
// createRequire, and the TS reference from build/runtime, then asserts equality on real
// .dx documents and on representative binary inputs.
//
// Run: node rust/doc-wasm/validate.mjs   (after `npm run build:ts` and the wasm build)

import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const require = createRequire(import.meta.url);
const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..');

// --- wasm under test -------------------------------------------------------
const wasm = require(path.join(here, 'pkg', 'doc_wasm.js'));

// --- TypeScript reference --------------------------------------------------
const runtime = path.join(repoRoot, 'build', 'runtime', 'src');
const { parseDocFile, stringifyDocFile } = await import(
  path.join(runtime, 'doc-format.js')
);
const { packDocument } = await import(path.join(runtime, 'doc-binary.js'));
const { sha256Hex } = await import(path.join(runtime, 'core', 'digest.js'));
const { compress: tsCompress } = await import(
  path.join(runtime, 'core', 'compress.js')
);

// --- tiny assertion helpers ------------------------------------------------
let passed = 0;
const failures = [];
function check(name, condition, detail = '') {
  if (condition) {
    passed += 1;
    console.log(`  ok   ${name}`);
  } else {
    failures.push(`${name}${detail ? ` — ${detail}` : ''}`);
    console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ''}`);
  }
}
const bytesEqual = (a, b) =>
  a.length === b.length && Buffer.from(a).equals(Buffer.from(b));

// --- documents: stringify(parse(...)) parity -------------------------------
const docs = [
  'examples/block-reference.dx',
  'examples/compactness-comparison.dx',
  'examples/footprint-pair.dx',
  'examples/tutorial.dx',
  'examples/welcome.dx',
  'documents/browser-check.dx',
  'documents/final-validation.dx',
];

console.log('Round-trip parity: wasm.stringify(wasm.parse) === TS.stringifyDocFile(TS.parseDocFile)');
for (const rel of docs) {
  const abs = path.join(repoRoot, rel);
  const text = readFileSync(abs, 'utf8');

  const wasmOut = wasm.stringify(wasm.parse(rel, text));
  const tsOut = stringifyDocFile(parseDocFile(rel, text));
  check(`stringify ${rel}`, wasmOut === tsOut, wasmOut === tsOut ? '' : firstDiff(wasmOut, tsOut));

  // pack parity on the SAME document. doc-core deliberately omits the host-level
  // filename-derived title fallback (it is "a host concern, not part of the format
  // core"), so we apply the TS host policy to the wasm document first to isolate the
  // codec under test. Both packers then receive byte-identical input.
  const tsDoc = parseDocFile(rel, text);
  const wasmDoc = JSON.parse(wasm.parse(rel, text));
  wasmDoc.title = tsDoc.title; // align the host-level title fallback
  const wasmPack = wasm.pack(JSON.stringify(wasmDoc));
  const tsPack = packDocument(tsDoc);
  check(`pack ${rel}`, bytesEqual(wasmPack, tsPack), `wasm=${wasmPack.length}B ts=${tsPack.length}B ${firstByteDiff(wasmPack, tsPack)}`);
}

// --- digest + compress parity on sample byte inputs ------------------------
console.log('\nDigest + compress parity on sample inputs');
const samples = [
  '',
  'hello world',
  'The quick brown fox jumps over the lazy dog.',
  'a'.repeat(5000),
  readFileSync(path.join(repoRoot, 'examples/tutorial.dx'), 'utf8'),
];
for (const [i, s] of samples.entries()) {
  const bytes = Buffer.from(s, 'utf8');
  const label = i === 4 ? 'tutorial.dx bytes' : i === 3 ? '5000x "a"' : JSON.stringify(s.slice(0, 24));

  check(`sha256_hex ${label}`, wasm.sha256_hex(bytes) === sha256Hex(bytes));
  check(`compress ${label}`, bytesEqual(wasm.compress(bytes), tsCompress(bytes)));
  // round-trip self-check: wasm.decompress(wasm.compress(x)) === x
  check(`compress->decompress ${label}`, bytesEqual(wasm.decompress(wasm.compress(bytes)), bytes));
}

// --- search smoke check (no TS equivalent asserted; verifies the export works) --
console.log('\nSearch export smoke check');
{
  const text = readFileSync(path.join(repoRoot, 'examples/tutorial.dx'), 'utf8');
  const doc = JSON.parse(wasm.parse('examples/tutorial.dx', text));
  const docsJson = JSON.stringify([['examples/tutorial.dx', doc]]);
  const hits = JSON.parse(wasm.build_index_and_search(docsJson, 'block document'));
  check('build_index_and_search returns the indexed doc', hits.length >= 1 && hits[0].path === 'examples/tutorial.dx');
  check('empty query yields no hits', JSON.parse(wasm.build_index_and_search(docsJson, '')).length === 0);
}

function firstByteDiff(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i += 1) if (a[i] !== b[i]) return `(first byte diff @${i})`;
  return a.length === b.length ? '' : `(length diff)`;
}

function firstDiff(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i += 1) {
    if (a[i] !== b[i]) return `differ at ${i}: wasm=${JSON.stringify(a.slice(i, i + 30))} ts=${JSON.stringify(b.slice(i, i + 30))}`;
  }
  return `length differs wasm=${a.length} ts=${b.length}`;
}

console.log(`\n${passed} checks passed, ${failures.length} failed.`);
if (failures.length) {
  for (const f of failures) console.error(`FAILED: ${f}`);
  process.exit(1);
}
console.log('ALL PARITY CHECKS PASSED — wasm output is byte-identical to the TypeScript reference.');
