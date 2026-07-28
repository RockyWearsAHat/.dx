// Adapter that lets the webview parse `.dx` source with the Rust `doc-core` engine compiled
// to WebAssembly, instead of the TypeScript reference parser in `doc-pipeline.ts`.
//
// WHY THIS EXISTS
// ---------------
// DOC historically had a DUAL-PARSER problem: the backend (`src/doc-format.ts`) and the
// webview (`doc-pipeline.ts`) each parsed `.dx` independently and had to be kept in lock-step.
// The Rust `doc-core` crate is now the single source of truth; `doc-wasm` exposes it to JS.
// This module is the thin, browser-safe bridge that wires the wasm `parse` into the exact
// `PipelineBlock[]` shape the webview renderer/editor already consumes, so the webview can
// route parsing through the Rust engine while the TS parser remains an untouched fallback.
//
// SHAPE MAPPING (wasm DocumentDTO block -> webview PipelineBlock)
// --------------------------------------------------------------
// The wasm `parse(path, text)` returns a `DocumentDto` JSON (camelCase). Its blocks already
// use `type`/`className`/`scriptType`, so most fields cross unchanged. The adapter only needs
// to normalize a few shape differences:
//   * `items`: the DTO always carries `{ checked, text, nested }` objects. The webview expects
//     PLAIN STRINGS for `bulleted-list`/`numbered-list` items and `{ checked, text }` objects
//     for `checklist` items. We flatten accordingly and drop `nested` (the webview block model
//     is flat; nested lists are not part of `PipelineBlock`).
//   * default/empty fields: the DTO always emits every field; the webview block shape is sparse
//     (omitted when empty). We let `stringifyBlock` see the full object and only attach the
//     fields each block type actually uses, matching `parseSourceBlocks`'s output.
//   * `rawSource`: the wasm parser is canonical and does NOT preserve per-block original text.
//     The editor needs `rawSource` for in-place block editing, so we synthesize it with the
//     webview's own canonical serializer (`stringifyBlock`). This yields deterministic, save-
//     stable source — the same text the document would round-trip to — which is exactly what
//     the single-engine model wants.
//
// LIFECYCLE
// ---------
// `parse` is synchronous, but wasm instantiation is async. `initDocCore()` performs a one-time,
// idempotent async init; callers must await it once at webview boot before invoking
// `wasmParseSourceBlocks`. If init fails (or is never called), `isDocCoreReady()` stays false
// and callers should fall back to the TS `parseSourceBlocks`.

import init, { parse as wasmParse } from './wasm/doc_wasm.js';
import type { ChecklistItem, PipelineBlock } from './doc-pipeline.js';
import { stringifyBlock } from './webview-doc-model.js';

/** A single list/checklist item as it crosses the wasm boundary. */
interface WasmItem {
  checked: boolean;
  text: string;
  nested: WasmItem[];
}

/** A block as emitted by the wasm `DocumentDto` (every field always present). */
interface WasmBlock {
  type: string;
  id: string;
  className: string;
  hidden: boolean;
  scriptType: string;
  module: boolean;
  level: number;
  text: string;
  language: string;
  src: string;
  alt: string;
  href: string;
  media: string;
  items: WasmItem[];
}

/** The full document as emitted by the wasm `parse`. */
interface WasmDocument {
  title: string;
  summary: string;
  tags: string[];
  meta: Array<[string, string]>;
  blocks: WasmBlock[];
}

/** Module-level init state. `wasm-bindgen`'s `init` is itself idempotent, but we guard the
 * promise so concurrent callers share one instantiation and a failure is observable. */
let initPromise: Promise<boolean> | null = null;
let ready = false;

/**
 * Initialize the `doc-core` wasm module exactly once.
 *
 * Idempotent: repeated calls return the same in-flight (or settled) promise. Pass the
 * webview-safe URL of `doc_wasm_bg.wasm` (obtained via `webview.asWebviewUri`) because the
 * esbuild bundle flattens `import.meta.url`, so the module cannot locate the `.wasm` on its own.
 *
 * Resolves to `true` when the engine is ready and `false` if instantiation failed; it never
 * rejects, so callers can branch to the TS fallback without a try/catch.
 *
 * @param wasmUrl absolute webview URI of the `doc_wasm_bg.wasm` binary
 */
export function initDocCore(wasmUrl: string): Promise<boolean> {
  if (initPromise) {
    return initPromise;
  }
  initPromise = (async () => {
    try {
      await init({ module_or_path: wasmUrl });
      ready = true;
      return true;
    } catch (error) {
      // Swallow-to-fallback is intentional: a wasm failure must NOT break the editor.
      // We surface it on the console so the regression is diagnosable.
      // eslint-disable-next-line no-console
      console.warn('doc-core wasm init failed; falling back to TS parser:', error);
      ready = false;
      return false;
    }
  })();
  return initPromise;
}

/** Whether the wasm engine finished initializing successfully and `wasmParseSourceBlocks`
 * is safe to call. */
export function isDocCoreReady(): boolean {
  return ready;
}

/** Map one wasm DTO item to the webview's checklist-item shape. */
function toChecklistItem(item: WasmItem): ChecklistItem {
  return { checked: Boolean(item.checked), text: String(item.text || '') };
}

/** Build the sparse `PipelineBlock` for one wasm block, mirroring `parseSourceBlocks` output
 * (only the fields a given block type carries) and synthesizing a canonical `rawSource`. */
function toPipelineBlock(block: WasmBlock): PipelineBlock {
  const base: PipelineBlock = {
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
      base.items = block.items.map((item) => String(item.text || ''));
      break;
    case 'checklist':
      base.items = block.items.map(toChecklistItem);
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
      // rule carries no body; `parseSourceBlocks` emits only base fields.
      break;
    default:
      base.text = String(block.text || '');
      break;
  }

  // The editor reads `rawSource` for in-place editing; the canonical engine drops it, so we
  // regenerate it from the webview's own serializer (deterministic, save-stable).
  base.rawSource = stringifyBlock(base);
  return base;
}

/**
 * Parse `.dx` source with the wasm `doc-core` engine and return the same `PipelineBlock[]`
 * structure as `doc-pipeline.parseSourceBlocks`.
 *
 * Precondition: `initDocCore` has resolved `true` (`isDocCoreReady()` is `true`). Callers that
 * cannot guarantee that must check `isDocCoreReady()` and fall back to `parseSourceBlocks`.
 *
 * The parameter type matches `parseSourceBlocks` (it accepts any value and coerces to string)
 * so this function is a drop-in replacement via `setSourceBlockParser`.
 *
 * @param source raw `.dx` document text (coerced to a string)
 * @returns canonical blocks adapted to the webview block shape
 */
export function wasmParseSourceBlocks(source: string | number | boolean | null | undefined | object): PipelineBlock[] {
  const doc = JSON.parse(wasmParse('', String(source ?? ''))) as WasmDocument;
  return doc.blocks.map(toPipelineBlock);
}
