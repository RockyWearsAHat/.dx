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

import * as path from 'path';
import * as vscode from 'vscode';

import { runCli } from './cli';
import { engine, outlineOf } from './engine';
import { makeNonce, previewHtml } from './preview';

/** View type of the rendered document editor. */
const VIEW_TYPE = 'dx.document';

/** Activate the extension: register the editor and the commands. */
export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider(VIEW_TYPE, new DxEditorProvider(), {
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
  /** Render `document` into `panel` and keep the two in step. */
  public resolveCustomTextEditor(
    document: vscode.TextDocument,
    panel: vscode.WebviewPanel
  ): void {
    panel.webview.options = { enableScripts: true };

    const refresh = (): void => {
      panel.webview.html = previewHtml(
        document.getText(),
        path.basename(document.fileName),
        makeNonce()
      );
    };
    refresh();

    const onChange = vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document.uri.toString() === document.uri.toString()) {
        refresh();
      }
    });
    const onTheme = vscode.window.onDidChangeActiveColorTheme(refresh);
    const onMessage = panel.webview.onDidReceiveMessage((message: { command?: string }) => {
      if (typeof message.command === 'string') {
        void vscode.commands.executeCommand(message.command, document.uri);
      }
    });

    panel.onDidDispose(() => {
      onChange.dispose();
      onTheme.dispose();
      onMessage.dispose();
    });
  }
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
