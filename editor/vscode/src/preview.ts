/**
 * Turning a rendered page into something a VS Code webview will display.
 *
 * A webview runs under a strict content-security policy, so the page it is given must
 * carry no external references and no scripts. That is exactly what the renderer already
 * produces, so this module only has three jobs: state the policy, add the small toolbar
 * the reader interacts with, and pick a palette that matches the editor.
 */

import * as vscode from 'vscode';

import { engine } from './engine';

/** Content-security policy for the rendered document: styles and images only. */
const POLICY =
  "default-src 'none'; style-src 'unsafe-inline'; img-src data: https:; font-src data:;";

/** The toolbar shown above every rendered document. */
const TOOLBAR = `
<div class="dx-bar">
  <span class="dx-bar-title" id="dx-title"></span>
  <span class="dx-bar-spacer"></span>
  <button data-command="dx.editSource">Edit source</button>
  <button data-command="dx.run">Run code</button>
  <button data-command="dx.exportPng">Export image</button>
</div>
<style>
  .dx-bar {
    position: sticky; top: 0; z-index: 10;
    display: flex; align-items: center; gap: 0.5rem;
    padding: 0.45rem 0.9rem;
    background: var(--dx-surface-2); border-bottom: 1px solid var(--dx-border);
    font: 500 0.78rem var(--dx-sans);
  }
  .dx-bar-title { color: var(--dx-muted); }
  .dx-bar-spacer { flex: 1; }
  .dx-bar button {
    font: inherit; cursor: pointer;
    padding: 0.2rem 0.6rem; border-radius: 6px;
    border: 1px solid var(--dx-border);
    background: var(--dx-bg); color: var(--dx-text);
  }
  .dx-bar button:hover { border-color: var(--dx-accent); color: var(--dx-accent); }
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
 * `nonce` authorizes the toolbar's own script — the only script on the page. The document's
 * own content stays script-free, which is what lets the policy stay this strict.
 */
export function previewHtml(source: string, title: string, nonce: string): string {
  const configuration = vscode.workspace.getConfiguration('dx');
  const page = engine().render_html(
    source,
    resolveTheme(configuration.get<string>('theme', 'match-editor')),
    false,
    configuration.get<boolean>('documentCss', false)
  );

  const policy = POLICY.replace("default-src 'none'", `default-src 'none'; script-src 'nonce-${nonce}'`);
  const toolbar = TOOLBAR.replace('<script>', `<script nonce="${nonce}">`).replace(
    'id="dx-title"></span>',
    `id="dx-title">${escapeHtml(title)}</span>`
  );

  return injectHead(page, `<meta http-equiv="Content-Security-Policy" content="${policy}">`)
    .replace('<body>', `<body>\n${toolbar}`);
}

/** Map the configured theme onto a palette the renderer understands. */
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

/** A fresh nonce for one webview render. */
export function makeNonce(): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  let nonce = '';
  for (let index = 0; index < 32; index += 1) {
    nonce += alphabet.charAt(Math.floor(Math.random() * alphabet.length));
  }
  return nonce;
}
