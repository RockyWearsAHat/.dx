/**
 * The document engine, loaded from WebAssembly.
 *
 * The parser and renderer are the same compiled Rust the `dx` command line and the MCP
 * server use, so a document looks identical in the editor, in a browser, and in the image
 * an AI agent is shown. Nothing here re-implements the format; if these two ever disagreed,
 * documents would silently drift between the tools that touch them.
 *
 * WebAssembly is also what makes the extension portable: one `.vsix` runs unchanged on
 * macOS, Windows, and Linux, with no native module to compile and no `dx` binary required
 * for viewing.
 */

import * as fs from 'fs';
import * as path from 'path';

/** The subset of the WebAssembly module this extension uses. */
export interface Engine {
  /**
   * Render `.dx` source to a self-contained HTML page. `resources` answers the
   * document's references (see {@link Engine.references}) as JSON
   * `{"files": {path: text}, "documents": {path: source}}`; omitted, every reference
   * renders as its honest sentence.
   */
  render_html(text: string, theme: string, fragment: boolean, resources?: string): string;
  /**
   * Every reference the source makes past its own edge, as JSON
   * `[{kind: 'file' | 'document', path}, …]` — the prefetch list for `render_html`.
   */
  references(text: string): string;
  /** The canonical source of one document held in a `DXCP1` pack. */
  pack_document(pack: Uint8Array, path: string): string;
  /** Render `.dx` source to Markdown. */
  render_text(text: string, includeIds: boolean): string;
  /** Outline `.dx` source as a JSON array of block entries. */
  render_outline(text: string): string;

  // Editing. These are `doc_core::edit`, the same operations `dx source`, `dx set`,
  // `dx insert`, `dx remove`, and `dx render --block` perform — so a block edited here and a
  // block edited in DX.app or by an agent goes through one implementation. Each throws with a
  // sentence naming the ids that exist when the one asked for does not.

  /** The editable text of one block: what goes in the field when a reader clicks it. */
  block_source(text: string, id: string): string;
  /** The canonical `::kind attrs` opening line of one block — the editing header. */
  block_header(text: string, id: string): string;
  /** Replace one block's body, returning the whole document's canonical source. */
  set_block(text: string, id: string, body: string): string;
  /** Replace one block wholesale — header and body. Returns `{source, id}` as JSON. */
  replace_block(text: string, id: string, header: string, body: string): string;
  /** Add a block after `after` (or at the top when empty). Returns `{source, id}` as JSON. */
  insert_block(text: string, after: string, kind: string, body: string): string;
  /** Take one block out, returning the canonical source without it. */
  remove_block(text: string, id: string): string;
  /**
   * Tick or untick the box at position `item` of a checklist, counting from zero — the
   * position the renderer writes on every mark as `data-check`.
   */
  toggle_check(text: string, id: string, item: number): string;
  /**
   * Move and reshape (or add) a node on a `::board`. An empty `node` creates a fresh one
   * with a hidden paragraph ready to write; a `w` or `h` of 0 keeps what the line already
   * says. The board settles afterwards, so nothing is left covering anything. Returns
   * `{source, id}` as JSON.
   */
  board_place(
    text: string,
    board: string,
    node: string,
    x: number,
    y: number,
    w: number,
    h: number
  ): string;
  /**
   * Place every node an arrangement names, in one edit — `"node,x,y,width,height …"`: a
   * group a reader moved, or a whole board an agent has just laid out.
   */
  board_arrange(text: string, board: string, spec: string): string;
  /** Take a node off a board — its line, its edges, and its orphaned hidden block. */
  board_detach(text: string, board: string, node: string): string;
  /**
   * Draw (`linked` true) or erase the edge from `from` to `to` on a board. `fromSide` and
   * `toSide` are the edges of the two nodes the line joins (`left`, `right`, `top`,
   * `bottom`, or their initials); empty leaves that end to the renderer.
   */
  board_link(
    text: string,
    board: string,
    from: string,
    to: string,
    linked: boolean,
    fromSide: string,
    toSide: string
  ): string;
  /** One block's HTML with `body` in it, saving nothing — the live half of writing. */
  preview_block(text: string, id: string, body: string, theme: string): string;
  /**
   * A field's text as decorated HTML — the source with its marks styled in place, every
   * character kept. What the editing surface shows inside a prose field between keystrokes.
   */
  field_html(text: string): string;
}

/** One row of a document outline. */
export interface OutlineEntry {
  id: string;
  kind: string;
  level: number;
  preview: string;
  chars: number;
  runnable: boolean;
}

let cached: Engine | undefined;

/**
 * Load the engine, reusing the instance across calls.
 *
 * Throws with an actionable message if the WebAssembly module is missing from the
 * installed extension, which can only happen if the package was built incorrectly.
 */
export function engine(): Engine {
  if (cached) {
    return cached;
  }
  try {
    // Required lazily and by relative path so the .wasm file resolves inside the
    // installed extension directory on every platform.
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    cached = require('../wasm/doc_wasm.js') as Engine;
    return cached;
  } catch (error) {
    throw new Error(
      `The DX rendering engine failed to load (${String(error)}). ` +
        'Reinstall the DX Documents extension.'
    );
  }
}

/**
 * The marker that opens a `.dx` pointer, matching `doc-store`'s strict stub reader —
 * the same recognition the github extension's resolver uses, so nothing that is not
 * exactly a pointer is ever mistaken for one.
 */
const POINTER = /^~ dx1 ([0-9a-f]{64})\s*$/;

/**
 * Gather what `source` references into the JSON `render_html` takes, or `undefined`
 * when it references nothing.
 *
 * The engine decides *what* is referenced and what becomes of the bytes; this gathers
 * them, which only the host can: files are read from the document's own folder, and a
 * sibling `.dx` that turns out to be a pointer is resolved through the committed pack
 * (`.doc/repo.dxcp`) — the reader never sees a pointer where a document belongs. A path
 * that cannot be read is left out, and the engine renders its sentence in place.
 */
export function resourcesFor(source: string, documentDir: string): string | undefined {
  let refs: unknown;
  try {
    refs = JSON.parse(engine().references(source));
  } catch {
    return undefined;
  }
  if (!Array.isArray(refs) || refs.length === 0) {
    return undefined;
  }
  const files: Record<string, string> = {};
  const documents: Record<string, string> = {};
  for (const ref of refs as { kind?: string; path?: string }[]) {
    if (typeof ref.path !== 'string') {
      continue;
    }
    const target = path.join(documentDir, ref.path);
    let text: string;
    try {
      text = fs.readFileSync(target, 'utf8');
    } catch {
      continue;
    }
    if (ref.kind === 'file') {
      files[ref.path] = text;
    } else if (ref.kind === 'document') {
      const resolved = POINTER.test(text.trim()) ? packedDocument(target) : text;
      if (resolved !== undefined) {
        documents[ref.path] = resolved;
      }
    }
  }
  return JSON.stringify({ files, documents });
}

/**
 * The true content of the pointer at `target`, read from the workspace pack — the file
 * beside `.doc/` nearest above it. `undefined` when no pack holds it; the engine then
 * says so on the page, which beats pretending the pointer line was the document.
 */
function packedDocument(target: string): string | undefined {
  for (let dir = path.dirname(target); ; dir = path.dirname(dir)) {
    const pack = path.join(dir, '.doc', 'repo.dxcp');
    if (fs.existsSync(pack)) {
      try {
        const relative = path.relative(dir, target).split(path.sep).join('/');
        return engine().pack_document(fs.readFileSync(pack), relative);
      } catch {
        return undefined;
      }
    }
    if (path.dirname(dir) === dir) {
      return undefined;
    }
  }
}

/** Parse a document's outline into typed entries, tolerating an unreadable result. */
export function outlineOf(text: string): OutlineEntry[] {
  try {
    const parsed: unknown = JSON.parse(engine().render_outline(text));
    return Array.isArray(parsed) ? (parsed as OutlineEntry[]) : [];
  } catch {
    return [];
  }
}
