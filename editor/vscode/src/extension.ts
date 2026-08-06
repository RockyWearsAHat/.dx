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

import { documentText, revisionText } from './changes';
import { runCli } from './cli';
import { engine, outlineOf } from './engine';
import { makeNonce } from './policy';
import { blockHtml, previewHtml, sheetHtml, Surface } from './preview';

/** View type of the rendered document editor. */
const VIEW_TYPE = 'dx.document';

/**
 * Scheme of the read-only text one side of a comparison is shown as.
 *
 * A `dx-text:` URI names a document and the revision to read it at, and resolves to what the
 * pointer stands for. It is a URI rather than a temporary file so VS Code's own diff editor
 * can open it, with its own history, folding, and word wrap.
 */
const TEXT_SCHEME = 'dx-text';

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
  const sides = new DxTextProvider();
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider(VIEW_TYPE, new DxEditorProvider(loadSurface(context)), {
      webviewOptions: { retainContextWhenHidden: true },
      supportsMultipleEditorsPerDocument: true,
    }),
    vscode.workspace.registerTextDocumentContentProvider(TEXT_SCHEME, sides),
    // A comparison the reader keeps typing into has to keep up, or it is quietly showing
    // them a document they no longer have.
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document.uri.scheme === 'file' && event.document.uri.fsPath.endsWith('.dx')) {
        sides.workingCopyChanged(event.document.uri);
      }
    }),
    vscode.commands.registerCommand('dx.editSource', editSource),
    vscode.commands.registerCommand('dx.openChanges', openChanges),
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
        this.surface,
        document.uri.scheme === 'file' ? path.dirname(document.uri.fsPath) : undefined
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

/**
 * Answers `dx-text:` URIs with the document the named revision holds.
 *
 * The URI carries the file's path and, in its query, the revision to read — empty for the
 * working copy, which is taken from the editor rather than the disk so an unsaved change is
 * part of the comparison. Both sides come back through `dx textconv`, so neither can be a
 * pointer, and a side that cannot be resolved shows the sentence saying why: a comparison
 * that quietly displays an empty document is a reader told their work vanished.
 */
class DxTextProvider implements vscode.TextDocumentContentProvider {
  /** Raised for a side whose text has moved on. */
  private readonly changed = new vscode.EventEmitter<vscode.Uri>();

  /** VS Code asks for a side's text again whenever this fires for it. */
  public readonly onDidChange = this.changed.event;

  /** Say that the working copy of `file` is no longer what it was. */
  public workingCopyChanged(file: vscode.Uri): void {
    this.changed.fire(sideUri(file, ''));
  }

  /** The text of one side of a comparison. */
  public async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const file = vscode.Uri.file(uri.path).fsPath;
    const revision = uri.query;
    try {
      const held = revision
        ? await revisionText(file, revision)
        : (await vscode.workspace.openTextDocument(vscode.Uri.file(file))).getText();
      return await documentText(held, path.dirname(file), runCli);
    } catch (failure) {
      return failure instanceof Error ? failure.message : String(failure);
    }
  }
}

/** One call from the shared editor: an operation, a block, and what to do with it. */
interface EditCall {
  /** Correlates this call with its reply. */
  readonly call: number;
  /**
   * `source`, `parts`, `draw`, `decorate`, `commit`, `replace`, `remove`, `run`, `check`, or
   * `board`.
   */
  readonly op: string;
  /** The block the call is about. */
  readonly id?: string;
  /** The characters being written, for `draw`, `decorate`, and `commit`. */
  readonly text?: string;
  /** The `::kind attrs` opening line being written, for `replace`. */
  readonly header?: string;
  /** The body being written, for `replace`. */
  readonly body?: string;
  /** `insert` when Return should start the next paragraph after this one. */
  readonly then?: string;
  /** For `check`, the box's position in its checklist, counting from zero. */
  readonly item?: number;
  /** A board call's verb: `place`, `add`, `arrange`, `detach`, `link`, or `unlink`. */
  readonly action?: string;
  /** A board call's details: which node, where, and which edge. */
  readonly spec?: BoardSpec;
}

/** What one board operation needs to know. */
interface BoardSpec {
  /** The node a `place` or `detach` is about. */
  readonly node?: string;
  /** Canvas coordinates for `place` and `add`. */
  readonly x?: number;
  readonly y?: number;
  /** A node's box for `place`; 0 or absent keeps what the line already says. */
  readonly w?: number;
  readonly h?: number;
  /** Every node an `arrange` places, as `node,x,y,width,height` items separated by spaces. */
  readonly arrangement?: string;
  /** The ends of the edge a `link`/`unlink` draws or erases. */
  readonly from?: string;
  readonly to?: string;
  /** The sides of those two nodes the edge meets; empty leaves the end to the renderer. */
  readonly fromSide?: string;
  readonly toSide?: string;
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
  value?: string | { document: string; focus?: string } | { header: string; body: string };
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
  // The folder the document's references resolve against, so a re-rendered sheet shows
  // `::code src=` listings and cross-document board nodes as their current content.
  const dir = document.uri.scheme === 'file' ? path.dirname(document.uri.fsPath) : undefined;

  try {
    switch (call.op) {
      case 'source':
        return { value: engine().block_source(source, id) };

      // The block split the way the editing surface shows it: the writer's own
      // `::kind attrs` line above, the editable body beneath.
      case 'parts':
        return {
          value: {
            header: engine().block_header(source, id),
            body: engine().block_source(source, id),
          },
        };

      // A read, and the only call that changes nothing: what the block would look like with
      // the characters currently in the field.
      case 'draw':
        return { value: blockHtml(source, id, call.text ?? '') };

      // The characters currently in a prose field, decorated by the engine — marks styled
      // in place, every byte kept. A pure function of the text; the document is not read.
      case 'decorate':
        return { value: engine().field_html(call.text ?? '') };

      // Execute one `run` block through the CLI, which is the only thing that runs code.
      // The file is saved first — `dx run` reads from disk — and the buffer is brought back
      // in step with what the run wrote before the page is re-rendered from it.
      //
      // `--approve` is the reader's click: they are looking at this block's code in their
      // own editor and pressed `run` on it, which is the review the approval gate asks for,
      // and `--only` keeps the approval to that one block. Without it a block they just
      // typed would always be refused — editing changes the fingerprint — and the surface
      // would send them to a terminal to run what the page already offers.
      case 'run': {
        if (document.isDirty) {
          await document.save();
        }
        const file = document.uri.fsPath;
        const before = document.getText();
        const result = await runCli(
          ['run', file, '--only', id, '--approve'],
          path.dirname(file)
        );
        const disk = fs.readFileSync(file, 'utf8');
        if (disk === before) {
          // Nothing was written: the run never happened (a missing CLI, a refused
          // sandbox). The sentence explaining why is the only thing there is to show.
          return { error: result.output || `dx run wrote nothing for \`${id}\`` };
        }
        // A failed *block* also lands here, deliberately: the failure was written into
        // the document as an output block, and the page showing it is the report.
        wrote(disk);
        await replaceAll(document, disk);
        await document.save();
        return { value: { document: sheetHtml(disk, dir) } };
      }

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
        return { value: { document: sheetHtml(updated, dir), focus } };
      }

      // A retype: the reader rewrote the tag line over the block, so the whole block is
      // replaced — kind, attributes, and body — through the same `doc-core` rule
      // `dx set --header` is.
      case 'replace': {
        const replaced = JSON.parse(
          engine().replace_block(source, id, call.header ?? '', call.body ?? '')
        ) as { source: string; id: string };
        let updated = replaced.source;
        let focus: string | undefined;
        if (call.then === 'insert') {
          const inserted = JSON.parse(
            engine().insert_block(updated, replaced.id, 'paragraph', '')
          ) as { source: string; id: string };
          updated = inserted.source;
          focus = inserted.id;
        }
        wrote(updated);
        await replaceAll(document, updated);
        return { value: { document: sheetHtml(updated, dir), focus } };
      }

      case 'remove': {
        const updated = engine().remove_block(source, id);
        wrote(updated);
        await replaceAll(document, updated);
        return { value: { document: sheetHtml(updated, dir) } };
      }

      // A box on a checklist, ticked by clicking it. The marker is the format's, so flipping
      // it is `doc-core`'s — the same `dx check` an agent runs.
      case 'check': {
        const updated = engine().toggle_check(source, id, Math.max(0, Math.round(call.item ?? 0)));
        wrote(updated);
        await replaceAll(document, updated);
        return { value: { document: sheetHtml(updated, dir) } };
      }

      // A board operation: dragging, linking, adding, or deleting a node on the canvas.
      // The line rewriting is entirely `doc-core`'s — the same `dx board` an agent runs —
      // so the surface never grows a copy of the node-line grammar.
      case 'board': {
        const spec = call.spec ?? {};
        const action = call.action ?? '';
        let updated: string;
        let focus: string | undefined;
        if (action === 'place' || action === 'add') {
          const placed = JSON.parse(
            engine().board_place(
              source,
              id,
              action === 'add' ? '' : spec.node ?? '',
              Math.round(spec.x ?? 0),
              Math.round(spec.y ?? 0),
              Math.max(0, Math.round(spec.w ?? 0)),
              Math.max(0, Math.round(spec.h ?? 0))
            )
          ) as { source: string; id: string };
          updated = placed.source;
          // A fresh node is a block waiting for its first words: open it.
          if (action === 'add') {
            focus = placed.id;
          }
        } else if (action === 'arrange') {
          updated = engine().board_arrange(source, id, spec.arrangement ?? '');
        } else if (action === 'detach') {
          updated = engine().board_detach(source, id, spec.node ?? '');
        } else if (action === 'link' || action === 'unlink') {
          updated = engine().board_link(
            source,
            id,
            spec.from ?? '',
            spec.to ?? '',
            action === 'link',
            spec.fromSide ?? '',
            spec.toSide ?? ''
          );
        } else {
          return { error: `unknown board action \`${action}\`` };
        }
        wrote(updated);
        await replaceAll(document, updated);
        return { value: { document: sheetHtml(updated, dir), focus } };
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
 * Compare the document with the version git's `HEAD` holds — both as documents.
 *
 * This is the one thing the editor's own "Open Changes" cannot do: git applies the `dx`
 * diff driver `dx git-setup` installed, VS Code's git integration does not, so its comparison
 * is two pointer lines whose digests differ — true, and of no use to anyone. Here each side is
 * resolved first, and what the reader sees is the prose that changed.
 */
async function openChanges(uri?: vscode.Uri): Promise<void> {
  const target = await resolveUri(uri);
  if (!target) {
    return;
  }
  await vscode.commands.executeCommand(
    'vscode.diff',
    sideUri(target, 'HEAD'),
    sideUri(target, ''),
    `${path.basename(target.fsPath)} — HEAD ↔ working copy`
  );
}

/** One side of a comparison: the document at `revision`, or the working copy when empty. */
function sideUri(file: vscode.Uri, revision: string): vscode.Uri {
  return vscode.Uri.from({ scheme: TEXT_SCHEME, path: file.path, query: revision });
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
    () => runCli(['run', target.fsPath], path.dirname(target.fsPath))
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
  const page = engine().render_html(document.getText(), 'auto', false);
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

  const result = await runCli(['png', target.fsPath], path.dirname(target.fsPath));
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
