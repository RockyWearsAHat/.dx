/**
 * DX Documents — open a `.dx` file and see the document, not the markup.
 *
 * The extension registers a custom editor for `*.dx`, so double-clicking one in the
 * explorer shows the rendered page. The source is one click away and is an ordinary text
 * editor on an ordinary text file: `.dx` is plain text, and nothing here hides that.
 *
 * # Why this works the same on every machine
 * Rendering runs in WebAssembly inside the extension host, so the package is
 * platform-independent — the same `.vsix` on macOS, Windows, and Linux, with no native
 * build step and no background server. The `dx` command line is optional and used only for
 * running code blocks and exporting images.
 */

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';

import { runCli } from './cli';
import { engine, outlineOf } from './engine';
import { blockHtml, makeNonce, previewHtml, sheetHtml, Surface } from './preview';

/** View type of the rendered document editor. */
const VIEW_TYPE = 'dx.document';

/**
 * Read the shared editing surface out of the packaged extension.
 *
 * `editor/build.sh` copies `editor/surface` in beside the wasm, for the same reason it
 * copies the same two files into DX.app: one source, packed into each thing that ships it.
 * A package built without them still opens documents — read-only, and saying nothing about
 * it, because a reader who cannot edit is no worse off than they were before.
 */
function loadSurface(context: vscode.ExtensionContext): Surface | undefined {
  try {
    const directory = path.join(context.extensionPath, 'surface');
    return {
      js: fs.readFileSync(path.join(directory, 'edit.js'), 'utf8'),
      css: fs.readFileSync(path.join(directory, 'edit.css'), 'utf8'),
    };
  } catch {
    return undefined;
  }
}

/** Activate the extension: register the editor and the commands. */
export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider(VIEW_TYPE, new DxEditorProvider(loadSurface(context)), {
      webviewOptions: { retainContextWhenHidden: true },
      supportsMultipleEditorsPerDocument: true,
    }),
    vscode.commands.registerCommand('dx.editSource', editSource),
    vscode.commands.registerCommand('dx.showRendered', showRendered),
    vscode.commands.registerCommand('dx.run', runCodeBlocks),
    vscode.commands.registerCommand('dx.exportHtml', exportHtml),
    vscode.commands.registerCommand('dx.exportPng', exportPng),
    vscode.commands.registerCommand('dx.copyMarkdown', copyMarkdown)
  );
}

/** Deactivate the extension. Nothing is held open, so there is nothing to release. */
export function deactivate(): void {
  // No background processes, watchers, or servers to shut down.
}

/**
 * Shows a `.dx` file as a rendered page that follows the file as it changes.
 *
 * The webview is a *view*: every edit still goes through the text document, so undo,
 * source control, and external edits all behave normally.
 */
class DxEditorProvider implements vscode.CustomTextEditorProvider {
  /** The shared editing surface, read once from the extension's own directory. */
  public constructor(private readonly surface?: Surface) {}

  /** Render `document` into `panel` and keep the two in step. */
  public resolveCustomTextEditor(
    document: vscode.TextDocument,
    panel: vscode.WebviewPanel
  ): void {
    panel.webview.options = { enableScripts: true };

    // The text this webview last wrote. An edit made on the page comes back as a change to
    // the document, and rebuilding the page for it would throw away the caret the reader is
    // still typing into — the page has already shown that edit, from the same engine. A
    // change that is *not* this one came from somewhere else, and does need a fresh page.
    let written: string | undefined;

    const refresh = (): void => {
      panel.webview.html = previewHtml(
        document.getText(),
        path.basename(document.fileName),
        makeNonce(),
        this.surface
      );
    };
    refresh();

    const onChange = vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document.uri.toString() !== document.uri.toString()) {
        return;
      }
      if (event.document.getText() === written) {
        written = undefined;
        return;
      }
      refresh();
    });
    const onTheme = vscode.window.onDidChangeActiveColorTheme(refresh);
    const onMessage = panel.webview.onDidReceiveMessage(
      (message: { command?: string; dxEdit?: EditCall }) => {
        if (typeof message.command === 'string') {
          void vscode.commands.executeCommand(message.command, document.uri);
          return;
        }
        if (message.dxEdit) {
          const call = message.dxEdit;
          void applyEdit(document, call, (text) => {
            written = text;
          }).then((outcome) => {
            // The change event has already fired by now, if it was going to; an edit that
            // produced the same text does not raise one. Either way the note has served its
            // purpose, and a stale one would swallow a later change that happened to match.
            written = undefined;
            void panel.webview.postMessage({
              dxReply: { call: call.call, value: outcome.value, error: outcome.error },
            });
          });
        }
      }
    );

    panel.onDidDispose(() => {
      onChange.dispose();
      onTheme.dispose();
      onMessage.dispose();
    });
  }
}

/** One call from the shared editor: an operation, a block, and what to do with it. */
interface EditCall {
  /** Correlates this call with its reply. */
  readonly call: number;
  /** `source`, `draw`, `commit`, or `remove`. */
  readonly op: string;
  /** The block the call is about. */
  readonly id?: string;
  /** The body being written, for `draw` and `commit`. */
  readonly text?: string;
  /** `insert` when Return should start the next paragraph after this one. */
  readonly then?: string;
}

/** What an edit produced: an answer for the caller, or a failure to show on the page. */
interface EditOutcome {
  /**
   * The answer, for calls that have one.
   *
   * `source` answers with characters; `draw` with one block's HTML; `commit` and `remove`
   * with `{ document, focus }` — the re-rendered document for the page to swap in, and the
   * block to open once it is there.
   */
  value?: string | { document: string; focus?: string };
  /** A sentence the editor shows on the page. */
  error?: string;
}

/**
 * Perform one editing call against `document`.
 *
 * Every change goes through a `WorkspaceEdit` rather than writing the file, so undo, dirty
 * state, and source control all behave exactly as they do for text a person typed — which is
 * the whole reason this surface edits a `TextDocument` instead of calling the `dx` binary.
 *
 * The document operations themselves are `doc-core`'s, through the wasm engine: the same
 * code `dx set` and DX.app run, so a block cannot come out of an edit shaped differently
 * depending on which surface made it.
 */
async function applyEdit(
  document: vscode.TextDocument,
  call: EditCall,
  wrote: (text: string) => void
): Promise<EditOutcome> {
  const source = document.getText();
  const id = call.id ?? '';

  try {
    switch (call.op) {
      case 'source':
        return { value: engine().block_source(source, id) };

      // A read, and the only call that changes nothing: what the block would look like with
      // the characters currently in the field.
      case 'draw':
        return { value: blockHtml(source, id, call.text ?? '') };

      case 'commit': {
        let updated = engine().set_block(source, id, call.text ?? '');
        let focus: string | undefined;
        if (call.then === 'insert') {
          const inserted = JSON.parse(
            engine().insert_block(updated, id, 'paragraph', '')
          ) as { source: string; id: string };
          updated = inserted.source;
          focus = inserted.id;
        }
        wrote(updated);
        await replaceAll(document, updated);
        return { value: { document: sheetHtml(updated), focus } };
      }

      case 'remove': {
        const updated = engine().remove_block(source, id);
        wrote(updated);
        await replaceAll(document, updated);
        return { value: { document: sheetHtml(updated) } };
      }

      default:
        return { error: `unknown editor operation \`${call.op}\`` };
    }
  } catch (failure) {
    return { error: failure instanceof Error ? failure.message : String(failure) };
  }
}

/** Replace a document's whole text with `text`, as one undoable edit. */
async function replaceAll(document: vscode.TextDocument, text: string): Promise<void> {
  if (document.getText() === text) {
    return;
  }
  const edit = new vscode.WorkspaceEdit();
  const whole = new vscode.Range(
    document.positionAt(0),
    document.positionAt(document.getText().length)
  );
  edit.replace(document.uri, whole, text);
  await vscode.workspace.applyEdit(edit);
}

/** Open the raw DOCSRC of the active document in a normal text editor. */
async function editSource(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (target) {
    await vscode.commands.executeCommand('vscode.openWith', target, 'default');
  }
}

/** Open the rendered view of the active document. */
async function showRendered(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (target) {
    await vscode.commands.executeCommand('vscode.openWith', target, VIEW_TYPE);
  }
}

/**
 * Run the document's code blocks, then reload it so the new output is visible.
 *
 * The file is saved first: `dx run` reads from disk, and running stale bytes would report
 * results for code the reader is no longer looking at.
 */
async function runCodeBlocks(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (!target) {
    return;
  }
  const document = await vscode.workspace.openTextDocument(target);
  if (document.isDirty) {
    await document.save();
  }

  const runnable = outlineOf(document.getText()).filter((entry) => entry.runnable);
  if (runnable.length === 0) {
    void vscode.window.showInformationMessage(
      'Nothing to run: mark a code block with `run` to make it executable.'
    );
    return;
  }

  const result = await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: `Running ${runnable.length} code block${runnable.length === 1 ? '' : 's'}…`,
    },
    () => runCli(['run', target.fsPath], target.fsPath)
  );

  // `dx run` writes results into the file; reload so the rendered view shows them.
  await vscode.commands.executeCommand('workbench.action.files.revert');

  if (result.ok) {
    void vscode.window.showInformationMessage(result.output || 'Ran the document.');
  } else {
    void vscode.window.showWarningMessage(result.output || 'Some blocks failed.');
  }
}

/** Write the rendered page beside the document as an HTML file. */
async function exportHtml(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (!target) {
    return;
  }
  const document = await vscode.workspace.openTextDocument(target);
  const page = engine().render_html(document.getText(), 'auto', false, false);
  const output = target.with({ path: `${stripExtension(target.path)}.html` });

  await vscode.workspace.fs.writeFile(output, Buffer.from(page, 'utf8'));
  void vscode.window.showInformationMessage(`Exported ${path.basename(output.fsPath)}`);
}

/** Render the document to an image using the `dx` command line. */
async function exportPng(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (!target) {
    return;
  }
  const document = await vscode.workspace.openTextDocument(target);
  if (document.isDirty) {
    await document.save();
  }

  const result = await runCli(['png', target.fsPath], target.fsPath);
  if (result.ok) {
    void vscode.window.showInformationMessage(result.output || 'Exported an image.');
  } else {
    void vscode.window.showWarningMessage(result.output);
  }
}

/** Put the document's Markdown rendering on the clipboard. */
async function copyMarkdown(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (!target) {
    return;
  }
  const document = await vscode.workspace.openTextDocument(target);
  await vscode.env.clipboard.writeText(engine().render_text(document.getText(), false));
  void vscode.window.showInformationMessage('Copied the document as Markdown.');
}

/** The document a command should act on: the one passed in, or the active editor's. */
async function resolveUri(uri?: vscode.Uri): Promise<vscode.Uri | undefined> {
  if (uri) {
    return uri;
  }
  const active = vscode.window.activeTextEditor?.document.uri;
  if (active?.fsPath.endsWith('.dx')) {
    return active;
  }
  const tab = vscode.window.tabGroups.activeTabGroup.activeTab?.input;
  if (tab && typeof tab === 'object' && 'uri' in tab) {
    const candidate = (tab as { uri: vscode.Uri }).uri;
    if (candidate.fsPath.endsWith('.dx')) {
      return candidate;
    }
  }
  void vscode.window.showWarningMessage('Open a .dx document first.');
  return undefined;
}

/** A path with its extension removed. */
function stripExtension(value: string): string {
  const dot = value.lastIndexOf('.');
  return dot > value.lastIndexOf('/') ? value.slice(0, dot) : value;
}
