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

/** The subset of the WebAssembly module this extension uses. */
export interface Engine {
  /** Render `.dx` source to a self-contained HTML page. */
  render_html(text: string, theme: string, fragment: boolean, documentCss: boolean): string;
  /** Render `.dx` source to Markdown. */
  render_text(text: string, includeIds: boolean): string;
  /** Outline `.dx` source as a JSON array of block entries. */
  render_outline(text: string): string;
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

/** Parse a document's outline into typed entries, tolerating an unreadable result. */
export function outlineOf(text: string): OutlineEntry[] {
  try {
    const parsed: unknown = JSON.parse(engine().render_outline(text));
    return Array.isArray(parsed) ? (parsed as OutlineEntry[]) : [];
  } catch {
    return [];
  }
}
