/**
 * The webview's security boundary: what a rendered document may reach, and the nonce that
 * authorizes the few scripts the extension itself puts on the page.
 *
 * It sits apart from `preview.ts` because it depends on nothing but node. The policy that
 * actually ships is the string this module builds, so a test can pin it — a boundary nobody
 * checks is a boundary that quietly widens. Everything else about the page — the palette, the
 * toolbar, the editing surface — needs the editor, and stays in `preview.ts`.
 */

import { randomBytes } from 'node:crypto';

/**
 * Content-security policy for the rendered document: styles and inline images only.
 *
 * `img-src` grants `data:` and nothing remote, matching DX.app — a document must not reach
 * the network, and the two hosts agreeing on that boundary is what makes it a boundary.
 */
const POLICY =
  "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:;";

/**
 * The policy as the page carries it, with `nonce` the one thing permitted to run.
 *
 * The document's own content stays script-free, which is what lets the policy name its
 * scripts one by one: markup that came from the document carries no nonce and so cannot run
 * even if it somehow arrived with a `<script>` in it.
 *
 * `'wasm-unsafe-eval'` is the one addition beyond a script source, and it authorizes no
 * script of its own: it lifts WebAssembly's compile restriction *only* for code already
 * authorized to run by its nonce — the geometry engine's glue, `editor/surface/doc_wasm.js`,
 * inlined the same way `edit.js` is. It grants nothing to the document's own markup (still
 * script-free), no origin (`img-src`/`style-src`/`font-src`/`default-src 'none'` are
 * untouched), and none of `eval`, `Function(string)`, or `setTimeout(string)` — those need
 * the unrelated `'unsafe-eval'`, which this policy still refuses.
 */
export function scriptPolicy(nonce: string): string {
  return POLICY.replace(
    "default-src 'none'",
    `default-src 'none'; script-src 'nonce-${nonce}' 'wasm-unsafe-eval'`
  );
}

/**
 * A fresh nonce for one webview render.
 *
 * Cryptographically random: the nonce is what authorizes scripts under the policy above,
 * and `Math.random` is guessable.
 */
export function makeNonce(): string {
  return randomBytes(16).toString('hex');
}
