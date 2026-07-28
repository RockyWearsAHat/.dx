# Manual test: webview editor on the Rust `doc-core` wasm parser (Step 5b)

This verifies the VS Code webview editor renders and edits `.dx` documents through the new
Rust `doc-core` engine compiled to WebAssembly, with the TypeScript parser as a fallback.

**Status: NOT YET RUN.** This requires the live VS Code Extension Development Host (F5), which
cannot be automated from the agent environment. The automated parity test and the full build +
test suite are green (see the Step 5b report), but the live-editor checks below are unverified.

## Prerequisites

```bash
export PATH="$HOME/.cargo/bin:$PATH"
wasm-pack build rust/doc-wasm --target web --out-dir ../../vscode-extension/media/wasm
npm run build:ts        # bundles the webview, wiring in media/wasm/doc_wasm.js
```

The `media/wasm/` directory is gitignored; rerun the `wasm-pack` command after a fresh clone.
If it is missing, the webview bundle still builds (a stub disables the wasm path) and the editor
silently uses the TypeScript parser — so the steps below also implicitly check that wasm is
actually active.

## Launch

1. Open `vscode-extension/` in VS Code and press **F5** (Extension Development Host).
2. In the dev host, run **DOC DB: Mount Virtual Files** if the `.dx` editor does not auto-mount.

## Checks

1. **wasm is active (not the fallback).** Open the webview Developer Tools
   (Command Palette → *Developer: Open Webview Developer Tools*). Confirm there is **no**
   `doc-core wasm init failed` warning in the console. If you see it, wasm failed and the editor
   fell back to TS — investigate before trusting the rest.
2. **Open + render.** Open `examples/welcome.dx`. Confirm all blocks render: the title heading,
   paragraphs, the chart/table blocks, and the checklist (4 items, first three checked). Nothing
   should render as a raw `::checklist … ::end` paragraph.
3. **Open `examples/tutorial.dx` and `examples/block-reference.dx`.** Confirm headings, lists,
   code blocks, and rules all render normally.
4. **Edit a block.** Double-click a paragraph to open its source editor. Confirm the source box
   shows the block's canonical `.dx` text (e.g. `::paragraph … ::end`). Make a small edit and
   commit it. Confirm the rendered block updates.
5. **Edit a list/checklist.** Edit one checklist item's text and toggle one checkbox. Confirm the
   change renders and the item boundaries are preserved (no merged/split items).
6. **Save round-trip.** Save the document (the custom editor's save). Reopen it. Confirm the
   content is unchanged and no block degraded into a literal `::heading … ::end` paragraph.
7. **Malformed input.** Temporarily type a malformed block (e.g. an unterminated `::heading`
   with no `::end`) and confirm the editor degrades gracefully the same way it did before this
   change (compare against `docs/dx-format-contract.md`).
8. **Diff view.** Open a `.dx` diff (modify a tracked `.dx`, open the SCM diff). Confirm both
   sides render through the editor without errors.

## Known, by-design difference

The wasm engine is canonical: it does not preserve verbatim per-block original text. The adapter
(`vscode-extension/media/doc-core-wasm.ts`) regenerates each block's `rawSource` via the webview's
own `stringifyBlock`. So when you open a block's source editor, you may see the *canonical*
serialization rather than the exact bytes that were on disk (e.g. normalized attribute order or
whitespace). This is expected for the single-engine model and matches what the document would
save to anyway. Verify it does not surprise you during step 4.
