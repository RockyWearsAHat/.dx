import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';

function checkSyntax(relativePath) {
  const target = path.join(process.cwd(), relativePath);
  const result = spawnSync(process.execPath, ['--check', target], {
    encoding: 'utf8',
  });

  assert.equal(
    result.status,
    0,
    `Syntax check failed for ${relativePath}\nstdout:\n${result.stdout || ''}\nstderr:\n${result.stderr || ''}`,
  );
}

test('extension webview module files parse without syntax errors', () => {
  checkSyntax('vscode-extension/media/webview-autocomplete-controller.js');
  checkSyntax('vscode-extension/media/webview-autocomplete-core.js');
  checkSyntax('vscode-extension/media/webview-autocomplete-history.js');
  checkSyntax('vscode-extension/media/webview-block-presentation.js');
  checkSyntax('vscode-extension/media/webview-block-renderer.js');
  checkSyntax('vscode-extension/media/webview-block-source-normalizer.js');
  checkSyntax('vscode-extension/media/webview-document-lifecycle.js');
  checkSyntax('vscode-extension/media/doc-pipeline.js');
  checkSyntax('vscode-extension/media/webview-doc-model.js');
  checkSyntax('vscode-extension/media/webview-edit-controllers.js');
  checkSyntax('vscode-extension/media/webview-events.js');
  checkSyntax('vscode-extension/media/webview-fsm.mjs');
  checkSyntax('vscode-extension/media/webview-state-core.js');
  checkSyntax('vscode-extension/media/webview-surface-controller.js');
  checkSyntax('vscode-extension/media/webview.js');
  checkSyntax('vscode-extension/media/webview-main.js');
});

test('custom editor provider parses without syntax errors', () => {
  checkSyntax('vscode-extension/extension.js');
});
