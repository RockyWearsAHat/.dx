/**
 * Turning a rendered page into something a VS Code webview will display.
 *
 * A webview runs under a strict content-security policy, so the page it is given must
 * carry no external references and no scripts of the *document's* own. That is exactly what
 * the renderer already produces, so this module has three jobs: add the small toolbar the
 * reader interacts with, pick a palette that matches the editor, and put the shared editing
 * surface on the page. The policy itself is `policy.ts`.
 *
 * The editing surface is `editor/surface/edit.js`, the same file DX.app loads. Nothing about
 * *how editing behaves* is decided here — only how its four calls reach the extension host,
 * which on this surface is `postMessage` and on that one is a `WKScriptMessageHandler`.
 */

import * as vscode from 'vscode';

import { engine, resourcesFor } from './engine';
import { scriptPolicy } from './policy';

/** The shared editing surface's files, read once from the extension's own directory. */
export interface Surface {
  /** `editor/surface/edit.js`. */
  readonly js: string;
  /** `editor/surface/edit.css`. */
  readonly css: string;
  /**
   * `editor/surface/doc_wasm.js` — the geometry engine's glue, the `no-modules` build that
   * defines a plain `wasm_bindgen` global. Static code, unlike the engine's own bytes: it
   * ships with `edit.js` the same way, inlined once per page rather than fetched, and never
   * crosses the `postMessage` bridge — only the `.wasm` bytes do, on request.
   */
  readonly glue: string;
}

/**
 * The bridge from the shared editor to the extension host.
 *
 * Each of the editor's calls becomes one message and one reply, correlated by a sequence
 * number because a webview's `postMessage` is one-way. Nothing else crosses: an edit answers
 * with the re-rendered document and the page swaps that element in, so the webview's HTML is
 * built once and there is no position to hand back afterwards.
 */
const BRIDGE = `
const dxPending = new Map();
let dxSequence = 0;

function dxCall(payload) {
  const call = ++dxSequence;
  return new Promise((resolve, reject) => {
    dxPending.set(call, { resolve, reject });
    vscodeApi.postMessage({ dxEdit: Object.assign({ call }, payload) });
  });
}

window.addEventListener('message', (event) => {
  const message = event.data || {};
  if (message.dxReply) {
    const waiting = dxPending.get(message.dxReply.call);
    if (waiting) {
      dxPending.delete(message.dxReply.call);
      if (message.dxReply.error) waiting.reject(new Error(message.dxReply.error));
      else waiting.resolve(message.dxReply.value);
    }
  }
});

if (window.dxEditor) {
  window.dxEditor.attach({
    source: (id) => dxCall({ op: 'source', id }),
    parts: (id) => dxCall({ op: 'parts', id }),
    decorate: (text) => dxCall({ op: 'decorate', text }),
    commit: (id, text, then) => dxCall({ op: 'commit', id, text, then }),
    replace: (id, header, body, then) => dxCall({ op: 'replace', id, header, body, then }),
    remove: (id) => dxCall({ op: 'remove', id }),
    run: (id) => dxCall({ op: 'run', id }),
    check: (id, item) => dxCall({ op: 'check', id, item }),
    board: (id, action, spec) => dxCall({ op: 'board', id, action, spec }),
    engine: () => dxCall({ op: 'engine' }),
  });
  // The geometry engine loads once, after attach: its bytes come over this same bridge
  // (one request, not inlined into the page), and a board is fully usable before it lands
  // — the fitted page's own curves are the honest answer until the truer, measured ones
  // are ready to replace them.
  window.dxEditor.engine();
}
`;

/**
 * The toolbar shown above every rendered document.
 *
 * It is written in the document's own margin, not in a strip of interface bolted above it: the
 * same paper tone, the same hairline, the marginal mono the renderer uses for its own pencil
 * notes. Only the palette variables `doc-core` actually publishes may appear here — a name the
 * stylesheet does not define is not a fallback, it is an invalid declaration the browser drops,
 * which is how this bar spent a release rendering unstyled.
 *
 * A sticky bar has to carry `--dx-bg`, because the page scrolls under it. That is the paper's
 * own tone, so it stays one surface rather than becoming a second.
 */
const TOOLBAR = `
<div class="dx-bar">
  <span class="dx-bar-title" id="dx-title"></span>
  <span class="dx-bar-spacer"></span>
  <button data-command="dx.editSource">edit source</button>
  <button data-command="dx.run">run</button>
  <button data-command="dx.exportPng">export image</button>
</div>
<style>
  .dx-bar {
    position: sticky; top: 0; z-index: 10;
    display: flex; align-items: baseline; gap: 1.15rem;
    padding: 0.5rem 1.75rem;
    background: var(--dx-bg);
    border-bottom: 1px solid var(--dx-rule);
    font: 400 0.68rem/1.6 var(--dx-mono);
    letter-spacing: 0.02em;
  }
  .dx-bar-title { color: var(--dx-faint); }
  .dx-bar-spacer { flex: 1; }
  .dx-bar button {
    font: inherit; cursor: pointer;
    padding: 0; border: 0; background: none;
    color: var(--dx-muted);
    text-decoration: underline;
    text-decoration-color: transparent;
    text-underline-offset: 3px;
  }
  .dx-bar button:hover,
  .dx-bar button:focus-visible {
    color: var(--dx-text);
    text-decoration-color: var(--dx-rule);
    outline: none;
  }
</style>
<script>
  const vscodeApi = acquireVsCodeApi();
  for (const button of document.querySelectorAll('.dx-bar button')) {
    button.addEventListener('click', () => {
      vscodeApi.postMessage({ command: button.dataset.command });
    });
  }
</script>
`;

/**
 * Render `source` into a complete webview page.
 *
 * `nonce` authorizes the toolbar, the shared editor, and the bridge between them — and
 * nothing else. The document's own content stays script-free, which is what lets the policy
 * name its scripts one by one: markup that came from the document carries no nonce and so
 * cannot run even if it somehow arrived with a `<script>` in it.
 *
 * `surface` is optional; without it the page renders exactly as it did before, read-only.
 * `documentDir` is the document's folder; with it, the document's references — sibling
 * files behind `::code src=`, sibling documents behind board nodes — render as their
 * current content instead of sentences.
 */
export function previewHtml(
  source: string,
  title: string,
  nonce: string,
  surface?: Surface,
  documentDir?: string
): string {
  const configuration = vscode.workspace.getConfiguration('dx');
  const page = engine().render_html(
    source,
    resolveTheme(configuration.get<string>('theme', 'match-editor')),
    false,
    documentDir === undefined ? undefined : resourcesFor(source, documentDir)
  );

  const policy = scriptPolicy(nonce);
  const toolbar = TOOLBAR.replace('<script>', `<script nonce="${nonce}">`).replace(
    'id="dx-title"></span>',
    `id="dx-title">${escapeHtml(title)}</span>`
  );
  // The editor goes after the toolbar's script, because the bridge it installs calls
  // `acquireVsCodeApi()` — which may be called exactly once per page, and the toolbar
  // already did. The geometry glue goes before `edit.js`, in its own nonce'd script: it
  // defines the `wasm_bindgen` global `edit.js`'s `window.dxEditor.engine()` instantiates
  // once the bridge hands it the `.wasm` bytes.
  const editing = surface
    ? `<style>${surface.css}</style>\n` +
      `<script nonce="${nonce}">${surface.glue}</script>\n` +
      `<script nonce="${nonce}">${surface.js}\n${BRIDGE}</script>`
    : '';

  return injectHead(page, `<meta http-equiv="Content-Security-Policy" content="${policy}">`)
    .replace('<body>', `<body>\n${toolbar}`)
    .replace('</body>', `${editing}\n</body>`);
}

/**
 * The document's blocks, without the page around them — what an edit hands back.
 *
 * The webview replaces this one element instead of rebuilding its HTML, so the reader keeps
 * their scroll position, their caret, and the page they were already looking at.
 */
export function sheetHtml(source: string, documentDir?: string): string {
  const configuration = vscode.workspace.getConfiguration('dx');
  return engine().render_html(
    source,
    resolveTheme(configuration.get<string>('theme', 'match-editor')),
    true,
    documentDir === undefined ? undefined : resourcesFor(source, documentDir)
  );
}

/** Map the configured theme onto a palette the renderer understands. */
/**
 * The theme an *exported* page takes: the reader's `dx.theme`, and `auto` when that setting
 * says `match-editor`.
 *
 * A page written to disk is read outside VS Code, where there is no editor to match — so
 * baking in whichever theme happened to be active would hand someone a dark page because
 * the author was working at night. `auto` lets the file follow its own reader's system.
 */
export function exportTheme(): string {
  const configured = vscode.workspace
    .getConfiguration('dx')
    .get<string>('theme', 'match-editor');
  return configured === 'match-editor' ? 'auto' : configured;
}

function resolveTheme(configured: string): string {
  if (configured !== 'match-editor') {
    return configured;
  }
  const kind = vscode.window.activeColorTheme.kind;
  const dark =
    kind === vscode.ColorThemeKind.Dark || kind === vscode.ColorThemeKind.HighContrast;
  return dark ? 'dark' : 'light';
}

/** Insert markup at the start of a page's `<head>`. */
function injectHead(page: string, markup: string): string {
  const index = page.indexOf('<head>');
  if (index < 0) {
    return `${markup}${page}`;
  }
  const cut = index + '<head>'.length;
  return `${page.slice(0, cut)}\n${markup}${page.slice(cut)}`;
}

/** Escape text destined for HTML. */
function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}
