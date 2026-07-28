// Parity harness for Step 5b: proves the wasm doc-core parser + the webview adapter mapping
// produce the SAME block structure as the TypeScript reference `parseSourceBlocks`
// (vscode-extension/media/doc-pipeline.ts) for real `.dx` documents.
//
// The adapter mapping replicated here is identical to `media/doc-core-wasm.ts#toPipelineBlock`.
// (That module imports the *web*-target wasm glue, which needs `fetch`; in Node we drive the
// *nodejs*-target pkg directly and apply the same mapping, so the logic under test is the same.)
//
// We compare the SEMANTIC block fields (type/id/className/level/text/items/...). We deliberately
// EXCLUDE `rawSource`: the TS parser preserves verbatim original per-block text, whereas the
// canonical engine regenerates it via `stringifyBlock`. That divergence is by design and is
// documented in the adapter; it does not affect what the editor renders.
//
// Run: node rust/doc-wasm/validate-webview-parity.mjs   (after `npm run build:ts`)

import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const require = createRequire(import.meta.url);
const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..');

const wasm = require(path.join(here, 'pkg', 'doc_wasm.js'));
const runtimeMedia = path.join(repoRoot, 'build', 'runtime', 'vscode-extension', 'media');
const { parseSourceBlocks } = await import(path.join(runtimeMedia, 'doc-pipeline.js'));
const { stringifyBlock } = await import(path.join(runtimeMedia, 'webview-doc-model.js'));

// --- adapter mapping (mirror of media/doc-core-wasm.ts#toPipelineBlock) --------------------
function toPipelineBlock(block) {
  const base = {
    type: block.type,
    id: String(block.id || ''),
    className: String(block.className || ''),
    hidden: Boolean(block.hidden),
  };
  switch (block.type) {
    case 'heading':
      base.level = Number(block.level || 1);
      base.text = String(block.text || '');
      break;
    case 'bulleted-list':
    case 'numbered-list':
      base.items = block.items.map((it) => String(it.text || ''));
      break;
    case 'checklist':
      base.items = block.items.map((it) => ({ checked: Boolean(it.checked), text: String(it.text || '') }));
      break;
    case 'image':
      base.src = String(block.src || '');
      base.alt = String(block.alt || '');
      break;
    case 'stylesheet':
      base.href = String(block.href || '');
      base.media = String(block.media || '');
      break;
    case 'style':
      base.text = String(block.text || '');
      break;
    case 'script':
      base.scriptType = String(block.scriptType || '');
      base.src = String(block.src || '');
      base.module = Boolean(block.module);
      base.text = String(block.text || '');
      break;
    case 'code':
      base.language = String(block.language || '');
      base.text = String(block.text || '');
      break;
    case 'rule':
      break;
    default:
      base.text = String(block.text || '');
      break;
  }
  base.rawSource = stringifyBlock(base);
  return base;
}

function wasmParseSourceBlocks(source) {
  const doc = JSON.parse(wasm.parse('', String(source ?? '')));
  return doc.blocks.map(toPipelineBlock);
}

// --- normalization for comparison (drop rawSource; coerce sparse/empty to a canonical form) -
const SEMANTIC_KEYS = [
  'type', 'id', 'className', 'hidden', 'level', 'text', 'language',
  'src', 'alt', 'href', 'media', 'scriptType', 'module', 'items',
];
function normalizeItem(item) {
  if (typeof item === 'string') return { checked: false, text: item, list: true };
  return { checked: Boolean(item.checked), text: String(item.text || '') };
}
function normalizeBlock(block) {
  const out = {};
  for (const key of SEMANTIC_KEYS) {
    let v = block[key];
    if (key === 'items') {
      out.items = Array.isArray(v) ? v.map(normalizeItem) : [];
      continue;
    }
    if (v === undefined || v === '' || v === false || v === 0) v = null;
    out[key] = v;
  }
  return out;
}
function normalizeBlocks(blocks) {
  return blocks.map(normalizeBlock);
}

// --- documents -----------------------------------------------------------------------------
const docs = [
  'examples/block-reference.dx',
  'examples/compactness-comparison.dx',
  'examples/footprint-pair.dx',
  'examples/tutorial.dx',
  'examples/welcome.dx',
  'documents/browser-check.dx',
  'documents/final-validation.dx',
  'documents/compact-proof.dx',
  'documents/virtual-check.dx',
];

let passed = 0;
const failures = [];
for (const rel of docs) {
  let text;
  try {
    text = readFileSync(path.join(repoRoot, rel), 'utf8');
  } catch {
    continue; // tolerate missing optional docs
  }
  const ts = normalizeBlocks(parseSourceBlocks(text));
  const wa = normalizeBlocks(wasmParseSourceBlocks(text));
  const a = JSON.stringify(ts, null, 1);
  const b = JSON.stringify(wa, null, 1);
  if (a === b) {
    passed += 1;
    console.log(`  ok   ${rel}  (${ts.length} blocks)`);
  } else {
    failures.push(rel);
    console.log(`  FAIL ${rel}`);
    // show first differing block
    const n = Math.max(ts.length, wa.length);
    for (let i = 0; i < n; i++) {
      const ja = JSON.stringify(ts[i]);
      const jb = JSON.stringify(wa[i]);
      if (ja !== jb) {
        console.log(`       block[${i}] TS:   ${ja}`);
        console.log(`       block[${i}] WASM: ${jb}`);
        break;
      }
    }
  }
}

console.log(`\n${passed} passed, ${failures.length} failed (rawSource excluded by design).`);
if (failures.length) {
  process.exitCode = 1;
}
