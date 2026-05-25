const path = require('node:path');
const os = require('node:os');
const { pathToFileURL } = require('node:url');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { mkdir, readFile, writeFile, readdir } = require('node:fs/promises');
const vscode = require('vscode');

const execFileAsync = promisify(execFile);
const CHAT_SNAPSHOT_SCHEME = 'chat-editing-snapshot-text-model';

const THEME_OPTIONS = new Set(['auto', 'light', 'dark']);
const WELCOME_DOC_RELATIVE_PATH = 'examples/welcome.dx';
const WELCOME_DOC_OPENED_KEY = 'docdb.welcomeDocOpened.v1';
const IMAGE_EXT_BY_MIME = {
  'image/gif': 'gif',
  'image/jpeg': 'jpg',
  'image/jpg': 'jpg',
  'image/png': 'png',
  'image/svg+xml': 'svg',
  'image/webp': 'webp',
};
let runtimeRoot = null;
let runtimePromise = null;
const chatEditingSessionDirCache = new Map();
const chatEditingSessionStateCache = new Map();
const unifiedDiffAutoRouteInFlight = new Set();
let unifiedDiffPanelState = null;
let extensionContext = null;

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));
}

class ChatEditingSnapshotContentProvider {
  async provideTextDocumentContent(uri) {
    return await readChatEditingSnapshotSource(uri, '');
  }
}

function getVsCodeUserDataPath() {
  const home = os.homedir();
  const appName = String(vscode.env.appName || 'Code');

  if (!home) {
    return null;
  }

  if (process.platform === 'darwin') {
    return path.join(home, 'Library', 'Application Support', appName, 'User');
  }

  if (process.platform === 'win32') {
    return path.join(process.env.APPDATA || path.join(home, 'AppData', 'Roaming'), appName, 'User');
  }

  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, '.config'), appName, 'User');
}

function decodeChatEditingSessionId(value) {
  const raw = String(value || '').trim();

  if (!raw) {
    return '';
  }

  const token = raw.split('/').filter(Boolean).pop() || raw;
  const normalizedToken = token.replace(/-/g, '+').replace(/_/g, '/');
  const paddedToken = normalizedToken + '='.repeat((4 - (normalizedToken.length % 4)) % 4);

  try {
    const decoded = Buffer.from(paddedToken, 'base64').toString('utf8').trim();
    return decoded || token;
  } catch {
    return token;
  }
}

function getChatEditingSnapshotResourceUri(uri) {
  const rawPath = String(uri?.path || '').trim();

  if (!rawPath) {
    return '';
  }

  try {
    const decodedPath = decodeURIComponent(rawPath);
    return pathToFileURL(path.resolve(decodedPath)).toString();
  } catch {
    return '';
  }
}

function normalizePathForComparison(absolutePath) {
  const normalized = path.resolve(String(absolutePath || ''));
  return process.platform === 'win32' ? normalized.toLowerCase() : normalized;
}

function getAbsolutePathFromResourceUri(resourceUriText) {
  const raw = String(resourceUriText || '').trim();

  if (!raw) {
    return '';
  }

  try {
    const parsed = vscode.Uri.parse(raw);

    if (parsed.scheme === 'file' && parsed.fsPath) {
      return path.resolve(String(parsed.fsPath));
    }
  } catch {
    // Ignore parse errors and fall through to plain path decode.
  }

  try {
    return path.resolve(decodeURIComponent(raw));
  } catch {
    return '';
  }
}

function snapshotEntryMatchesFilePath(entry, filePath) {
  const entryPath = getAbsolutePathFromResourceUri(entry?.resource || '');

  if (!entryPath || !filePath) {
    return false;
  }

  return normalizePathForComparison(entryPath) === normalizePathForComparison(filePath);
}

async function findChatEditingSessionDir(sessionId) {
  const normalizedSessionId = String(sessionId || '').trim();

  if (!normalizedSessionId) {
    return null;
  }

  if (chatEditingSessionDirCache.has(normalizedSessionId)) {
    return chatEditingSessionDirCache.get(normalizedSessionId) || null;
  }

  const userDataPath = getVsCodeUserDataPath();

  if (!userDataPath) {
    return null;
  }

  const workspaceStoragePath = path.join(userDataPath, 'workspaceStorage');

  try {
    const workspaceEntries = await readdir(workspaceStoragePath, { withFileTypes: true });

    for (const workspaceEntry of workspaceEntries) {
      if (!workspaceEntry.isDirectory()) {
        continue;
      }

      const candidateDir = path.join(workspaceStoragePath, workspaceEntry.name, 'chatEditingSessions', normalizedSessionId);

      try {
        await readFile(path.join(candidateDir, 'state.json'), 'utf8');
        chatEditingSessionDirCache.set(normalizedSessionId, candidateDir);
        return candidateDir;
      } catch {
        // Keep scanning workspace storage directories.
      }
    }
  } catch {
    // Ignore missing workspace storage directories and fall back to null.
  }

  return null;
}

async function loadChatEditingSessionState(sessionId) {
  const normalizedSessionId = String(sessionId || '').trim();

  if (!normalizedSessionId) {
    return null;
  }

  const sessionDir = await findChatEditingSessionDir(normalizedSessionId);

  if (!sessionDir) {
    return null;
  }

  try {
    const parsed = JSON.parse(await readFile(path.join(sessionDir, 'state.json'), 'utf8'));
    return { sessionDir, parsed };
  } catch {
    return null;
  }
}

function findSnapshotEntryInParsedState(parsedState, snapshotUriText, resourceUri, absoluteSnapshotPath) {
  const recentEntries = Array.isArray(parsedState?.recentSnapshot?.entries) ? parsedState.recentSnapshot.entries : [];
  const initialEntries = Array.isArray(parsedState?.initialFileContents) ? parsedState.initialFileContents : [];

  const matchedEntry = recentEntries.find((entry) => String(entry?.snapshotUri || '') === snapshotUriText)
    || recentEntries.find((entry) => String(entry?.resource || '') === resourceUri)
    || recentEntries.find((entry) => snapshotEntryMatchesFilePath(entry, absoluteSnapshotPath))
    || null;

  const initialHashEntry = initialEntries.find((entry) => Array.isArray(entry) && String(entry[0] || '') === resourceUri)
    || initialEntries.find((entry) => Array.isArray(entry) && snapshotEntryMatchesFilePath({ resource: entry[0] }, absoluteSnapshotPath));
  const initialHash = String(initialHashEntry?.[1] || '').trim();

  return {
    matchedEntry,
    initialHash,
  };
}

async function resolveChatEditingSnapshotEntryByGlobalScan(uri) {
  const userDataPath = getVsCodeUserDataPath();

  if (!userDataPath) {
    return null;
  }

  const workspaceStoragePath = path.join(userDataPath, 'workspaceStorage');
  const snapshotUriText = typeof uri?.toString === 'function' ? uri.toString() : String(uri || '');
  const resourceUri = getChatEditingSnapshotResourceUri(uri);
  const absoluteSnapshotPath = decodeAbsolutePathFromSnapshotUri(uri);

  try {
    const workspaceEntries = await readdir(workspaceStoragePath, { withFileTypes: true });

    for (const workspaceEntry of workspaceEntries) {
      if (!workspaceEntry.isDirectory()) {
        continue;
      }

      const sessionsRoot = path.join(workspaceStoragePath, workspaceEntry.name, 'chatEditingSessions');
      let sessionEntries = [];

      try {
        sessionEntries = await readdir(sessionsRoot, { withFileTypes: true });
      } catch {
        continue;
      }

      for (const sessionEntry of sessionEntries) {
        if (!sessionEntry.isDirectory()) {
          continue;
        }

        const sessionDir = path.join(sessionsRoot, sessionEntry.name);
        let parsed = null;

        try {
          parsed = JSON.parse(await readFile(path.join(sessionDir, 'state.json'), 'utf8'));
        } catch {
          continue;
        }

        const match = findSnapshotEntryInParsedState(parsed, snapshotUriText, resourceUri, absoluteSnapshotPath);

        if (match.matchedEntry || match.initialHash) {
          return {
            sessionDir,
            matchedEntry: match.matchedEntry,
            initialHash: match.initialHash,
          };
        }
      }
    }
  } catch {
    return null;
  }

  return null;
}

async function resolveChatEditingSnapshotContentHash(uri) {
  const payload = parseUriQueryObject(uri?.query);
  const sessionInfo = payload?.session && typeof payload.session === 'object' ? payload.session : {};
  const sessionId = decodeChatEditingSessionId(
    sessionInfo.external || sessionInfo.path || sessionInfo.uri || sessionInfo.documentUri || ''
  );

  if (!sessionId) {
    return null;
  }

  const loadedState = await loadChatEditingSessionState(sessionId);

  if (!loadedState) {
    return null;
  }

  const { sessionDir, parsed } = loadedState;
  const resourceUri = getChatEditingSnapshotResourceUri(uri);
  const snapshotUriText = typeof uri?.toString === 'function' ? uri.toString() : String(uri || '');
  const recentEntries = Array.isArray(parsed?.recentSnapshot?.entries) ? parsed.recentSnapshot.entries : [];

  const matchedEntry = recentEntries.find((entry) => String(entry?.snapshotUri || '') === snapshotUriText)
    || recentEntries.find((entry) => String(entry?.resource || '') === resourceUri);

  const initialEntries = Array.isArray(parsed?.initialFileContents) ? parsed.initialFileContents : [];
  const initialHash = initialEntries.find((entry) => Array.isArray(entry) && String(entry[0] || '') === resourceUri)?.[1];
  const contentHash = String(matchedEntry?.originalHash || matchedEntry?.currentHash || initialHash || '').trim();

  if (!contentHash) {
    return null;
  }

  return {
    sessionDir,
    contentHash,
  };
}

async function resolveChatEditingSnapshotEntry(uri) {
  const payload = parseUriQueryObject(uri?.query);
  const sessionInfo = payload?.session && typeof payload.session === 'object' ? payload.session : {};
  const sessionId = decodeChatEditingSessionId(
    sessionInfo.external || sessionInfo.path || sessionInfo.uri || sessionInfo.documentUri || ''
  );

  if (!sessionId) {
    return null;
  }

  const loadedState = await loadChatEditingSessionState(sessionId);

  if (!loadedState) {
    return await resolveChatEditingSnapshotEntryByGlobalScan(uri);
  }

  const { sessionDir, parsed } = loadedState;
  const absoluteSnapshotPath = decodeAbsolutePathFromSnapshotUri(uri);
  const resourceUri = getChatEditingSnapshotResourceUri(uri);
  const snapshotUriText = typeof uri?.toString === 'function' ? uri.toString() : String(uri || '');
  const match = findSnapshotEntryInParsedState(parsed, snapshotUriText, resourceUri, absoluteSnapshotPath);

  if (!match.matchedEntry && !match.initialHash) {
    return await resolveChatEditingSnapshotEntryByGlobalScan(uri);
  }

  return {
    sessionDir,
    matchedEntry: match.matchedEntry,
    initialHash: String(match.initialHash || '').trim(),
  };
}

async function readChatEditingSnapshotHashSource(uri, variant = 'original', fallbackText = '') {
  try {
    const resolved = await resolveChatEditingSnapshotEntry(uri);

    if (!resolved?.sessionDir) {
      return String(fallbackText || '');
    }

    const { sessionDir, matchedEntry, initialHash } = resolved;
    const originalHash = String(matchedEntry?.originalHash || '').trim();
    const currentHash = String(matchedEntry?.currentHash || '').trim();
    const contentHash = variant === 'current'
      ? (currentHash || originalHash || initialHash)
      : (originalHash || currentHash || initialHash);

    if (!contentHash) {
      return String(fallbackText || '');
    }

    const snapshotPath = path.join(sessionDir, 'contents', contentHash);
    const snapshotText = await readFile(snapshotPath, 'utf8');
    return String(snapshotText || fallbackText || '');
  } catch {
    return String(fallbackText || '');
  }
}

async function readChatEditingDiffPairFromSnapshotUri(snapshotUri) {
  const oldSource = await readChatEditingSnapshotHashSource(snapshotUri, 'original', '');
  const newSource = await readChatEditingSnapshotHashSource(snapshotUri, 'current', '');

  return {
    oldSource: String(oldSource || ''),
    newSource: String(newSource || ''),
  };
}

async function readChatEditingSnapshotSource(uri, fallbackText = '') {
  try {
    return await readChatEditingSnapshotHashSource(uri, 'original', fallbackText);
  } catch {
    return String(fallbackText || '');
  }
}

function isSameUri(left, right) {
  return String(left || '') === String(right || '');
}

function decodeAbsolutePathFromSnapshotUri(uri) {
  try {
    const rawPath = String(uri?.path || '').trim();

    if (!rawPath) {
      return '';
    }

    return path.resolve(decodeURIComponent(rawPath));
  } catch {
    return '';
  }
}

function getSnapshotUriForFileUri(fileUri) {
  if (!fileUri || fileUri.scheme !== 'file') {
    return null;
  }

  const filePath = path.resolve(String(fileUri.fsPath || ''));

  for (const textDocument of vscode.workspace.textDocuments) {
    if (textDocument?.uri?.scheme !== CHAT_SNAPSHOT_SCHEME) {
      continue;
    }

    const snapshotPath = decodeAbsolutePathFromSnapshotUri(textDocument.uri);

    if (snapshotPath && path.resolve(snapshotPath) === filePath) {
      return textDocument.uri;
    }
  }

  return null;
}

function normalizeUnifiedDiffUris(originalUri, modifiedUri) {
  let oldUri = originalUri;
  let newUri = modifiedUri;

  if (!oldUri || !newUri) {
    return {
      originalUri: oldUri,
      modifiedUri: newUri,
    };
  }

  const oldIsSnapshot = oldUri.scheme === CHAT_SNAPSHOT_SCHEME;
  const newIsSnapshot = newUri.scheme === CHAT_SNAPSHOT_SCHEME;

  if (!oldIsSnapshot && newIsSnapshot) {
    oldUri = newUri;
    newUri = originalUri;
  }

  const oldIsFile = oldUri?.scheme === 'file';
  const newIsFile = newUri?.scheme === 'file';

  if (oldIsFile && newIsFile) {
    const oldFilePath = path.resolve(String(oldUri.fsPath || ''));
    const newFilePath = path.resolve(String(newUri.fsPath || ''));

    if (oldFilePath && newFilePath && oldFilePath === newFilePath) {
      const snapshotUri = getSnapshotUriForFileUri(newUri);

      if (snapshotUri) {
        oldUri = snapshotUri;
      }
    }
  }

  return {
    originalUri: oldUri,
    modifiedUri: newUri,
  };
}

function toUnifiedDiffPairKey(originalUri, modifiedUri) {
  const normalized = normalizeUnifiedDiffUris(originalUri, modifiedUri);
  return `${String(normalized.originalUri)}=>${String(normalized.modifiedUri)}`;
}

function findDiffCounterpartUri(documentUri) {
  if (!documentUri || typeof vscode.TabInputTextDiff !== 'function') {
    return null;
  }

  for (const tabGroup of vscode.window.tabGroups.all) {
    for (const tab of tabGroup.tabs) {
      const input = tab.input;

      if (!(input instanceof vscode.TabInputTextDiff)) {
        continue;
      }

      if (isSameUri(input.original, documentUri)) {
        return { diffRole: 'old', otherUri: input.modified };
      }

      if (isSameUri(input.modified, documentUri)) {
        return { diffRole: 'new', otherUri: input.original };
      }
    }
  }

  return null;
}

function findSnapshotCounterpartForDocument(documentUri) {
  if (!documentUri) {
    return null;
  }

  if (documentUri.scheme === CHAT_SNAPSHOT_SCHEME) {
    const absolutePath = decodeAbsolutePathFromSnapshotUri(documentUri);

    if (!absolutePath) {
      return null;
    }

    return {
      diffRole: 'old',
      otherUri: vscode.Uri.file(absolutePath),
    };
  }

  if (documentUri.scheme === 'file') {
    const snapshotUri = getSnapshotUriForFileUri(documentUri);

    if (snapshotUri) {
      return {
        diffRole: 'new',
        otherUri: snapshotUri,
      };
    }
  }

  return null;
}

function getUnifiedDiffContextForDocument(documentUri) {
  return findDiffCounterpartUri(documentUri) || findSnapshotCounterpartForDocument(documentUri);
}

function getWebviewAssetUris(webview, extensionUri) {
  const stylesUri = webview.asWebviewUri(vscode.Uri.joinPath(extensionUri, 'media', 'styles.css'));
  const workspaceRoot = getWorkspaceRoot();
  const webviewBundlePath = workspaceRoot
    ? vscode.Uri.file(path.join(workspaceRoot, 'build', 'docdb-webview.bundle.min.js'))
    : vscode.Uri.joinPath(extensionUri, 'media', 'webview-main.js');
  const webviewUri = webview.asWebviewUri(webviewBundlePath);
  const workspaceUri = workspaceRoot
    ? webview.asWebviewUri(vscode.Uri.file(workspaceRoot)).toString()
    : '';

  return {
    stylesUri,
    webviewUri,
    workspaceUri,
    workspaceRoot,
  };
}

function configureDocWebview(webview, extensionUri, workspaceRoot) {
  webview.options = {
    enableScripts: true,
    localResourceRoots: [
      vscode.Uri.joinPath(extensionUri, 'media'),
      ...(workspaceRoot ? [vscode.Uri.file(workspaceRoot)] : []),
    ],
  };
}

function isUriLike(value) {
  return Boolean(value && typeof value === 'object' && typeof value.scheme === 'string' && typeof value.path === 'string');
}

function extractDiffUrisFromInput(input) {
  if (!input || typeof input !== 'object') {
    return null;
  }

  const candidates = [
    { original: input.original, modified: input.modified },
    { original: input.left, modified: input.right },
    { original: input.base, modified: input.target },
  ];

  for (const candidate of candidates) {
    if (isUriLike(candidate.original) && isUriLike(candidate.modified)) {
      return {
        originalUri: candidate.original,
        modifiedUri: candidate.modified,
      };
    }
  }

  return null;
}

function isDxDiffInput(input) {
  const diffUris = extractDiffUrisFromInput(input);

  if (!diffUris) {
    return false;
  }

  const originalRelativePath = toWorkspaceRelativeDocPath(diffUris.originalUri);
  const modifiedRelativePath = toWorkspaceRelativeDocPath(diffUris.modifiedUri);
  return Boolean(originalRelativePath || modifiedRelativePath);
}

function getDxDiffInputUris(input) {
  const diffUris = extractDiffUrisFromInput(input);

  if (!diffUris || !isDxDiffInput(input)) {
    return null;
  }

  const relativePath = toWorkspaceRelativeDocPath(diffUris.modifiedUri) || toWorkspaceRelativeDocPath(diffUris.originalUri) || '';

  return {
    originalUri: diffUris.originalUri,
    modifiedUri: diffUris.modifiedUri,
    relativePath,
  };
}

function formatDiffSideTitle(uri, fallbackLabel) {
  const relativePath = toWorkspaceRelativeDocPath(uri);

  if (relativePath) {
    return path.basename(relativePath);
  }

  if (uri?.scheme === CHAT_SNAPSHOT_SCHEME) {
    const snapshotPath = decodeAbsolutePathFromSnapshotUri(uri);
    return snapshotPath ? path.basename(snapshotPath) : fallbackLabel;
  }

  if (uri?.scheme === 'file' && uri.fsPath) {
    return path.basename(String(uri.fsPath));
  }

  const rawPath = String(uri?.path || '').trim();
  return rawPath ? path.basename(rawPath) : fallbackLabel;
}

async function closeAllNativeDxDiffTabs() {
  const tabsToClose = [];

  for (const tabGroup of vscode.window.tabGroups.all) {
    for (const tab of tabGroup.tabs) {
      if (isDxDiffInput(tab?.input)) {
        tabsToClose.push(tab);
      }
    }
  }

  for (const tab of tabsToClose) {
    try {
      await vscode.window.tabGroups.close(tab, true);
    } catch {
      // Ignore close failures and continue.
    }
  }
}

async function openUnifiedDxDiffPanel(extensionUri, originalUri, modifiedUri) {
  const normalizedPair = normalizeUnifiedDiffUris(originalUri, modifiedUri);
  const pairKey = toUnifiedDiffPairKey(normalizedPair.originalUri, normalizedPair.modifiedUri);
  let panel = null;

  if (unifiedDiffPanelState?.panel) {
    try {
      panel = unifiedDiffPanelState.panel;
      panel.reveal(vscode.ViewColumn.Active, false);
    } catch {
      unifiedDiffPanelState = null;
      panel = null;
    }
  }

  const oldUri = normalizedPair.originalUri;
  const newUri = normalizedPair.modifiedUri;
  const relativePath = toWorkspaceRelativeDocPath(newUri)
    || toWorkspaceRelativeDocPath(oldUri)
    || toWorkspaceRelativeDocPath(originalUri)
    || '';

  if (!relativePath) {
    vscode.window.showErrorMessage('Unable to resolve DX path for unified diff viewer.');
    return;
  }

  const originalTitle = formatDiffSideTitle(oldUri, 'old');
  const modifiedTitle = formatDiffSideTitle(newUri, 'new');
  const panelTitle = `${originalTitle} (old) ↔ ${modifiedTitle} (current)`;

  if (!panel) {
    panel = vscode.window.createWebviewPanel(
      'docdb.unifiedDiff',
      panelTitle,
      vscode.ViewColumn.Active,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
      }
    );

    // Claim the singleton immediately so concurrent auto-route handlers reuse
    // this panel instead of creating a duplicate second tab.
    unifiedDiffPanelState = { panel, pairKey };
  }
  panel.title = panelTitle;

  const { stylesUri, webviewUri, workspaceUri, workspaceRoot } = getWebviewAssetUris(panel.webview, extensionUri);
  configureDocWebview(panel.webview, extensionUri, workspaceRoot);

  let initialTheme = 'auto';
  let initialAppearance = null;

  try {
    const config = await readUiConfig();
    initialTheme = String(config?.theme || 'auto');
  } catch {
    initialTheme = 'auto';
  }

  try {
    const { db, dbModule } = await getDocRuntime();
    const absolutePath = path.resolve(getWorkspaceRoot() || '', relativePath || '');
    const documentRow = dbModule.getDocumentByPath(db, getWorkspaceRoot() || '', absolutePath);
    initialAppearance = normalizeInitialAppearance(dbModule.getDocumentViewState(db, documentRow?.id));
  } catch {
    initialAppearance = null;
  }

  let oldSource = '';
  let newSource = '';
  let loadError = '';
  let baselineHeadSource = '';

  try {
    const absolutePathForRef = newUri?.scheme === 'file' && newUri.fsPath
      ? path.resolve(String(newUri.fsPath))
      : path.resolve(String(workspaceRoot || ''), relativePath);

    if (relativePath && absolutePathForRef) {
      baselineHeadSource = await readDocumentSnapshotAtRef(relativePath, absolutePathForRef, 'HEAD', '');
    }

    const baselineCurrentSource = relativePath
      ? await readVirtualDocument(relativePath).catch(() => '')
      : '';

    if (oldUri?.scheme === CHAT_SNAPSHOT_SCHEME && newUri?.scheme === 'file') {
      const snapshotPair = await readChatEditingDiffPairFromSnapshotUri(oldUri);
      oldSource = snapshotPair.oldSource || baselineHeadSource;
      newSource = snapshotPair.newSource || baselineCurrentSource;
    }

    oldSource = await readDisplaySourceForUri(relativePath, oldUri, oldSource || baselineHeadSource);
    newSource = await readDisplaySourceForUri(relativePath, newUri, newSource || baselineCurrentSource);

    if (!newSource && baselineCurrentSource) {
      newSource = baselineCurrentSource;
    }

    if (!oldSource && baselineHeadSource) {
      oldSource = baselineHeadSource;
    }

    if ((!oldSource || oldSource === newSource) && newSource && baselineHeadSource && baselineHeadSource !== newSource) {
      oldSource = baselineHeadSource;
    }
  } catch (error) {
    loadError = error instanceof Error ? error.message : 'Unable to load unified DX diff source.';
  }

  panel.webview.html = renderEditorHtml(
    relativePath,
    newSource,
    loadError,
    initialTheme,
    initialAppearance,
    panel.webview.cspSource,
    stylesUri,
    webviewUri,
    workspaceUri,
    'new',
    oldSource,
  );

  const hasResolvedDiff = Boolean(oldSource && newSource && oldSource !== newSource);

  if (!hasResolvedDiff && oldUri?.scheme === CHAT_SNAPSHOT_SCHEME && newUri?.scheme === 'file') {
    void refreshUnifiedDiffWhenSnapshotReady({
      panel,
      pairKey,
      relativePath,
      oldUri,
      newUri,
      initialTheme,
      initialAppearance,
      stylesUri,
      webviewUri,
      workspaceUri,
      workspaceRoot,
      baselineHeadSource,
    });
  }

  if (!unifiedDiffPanelState || unifiedDiffPanelState.panel !== panel) {
    panel.onDidDispose(() => {
      if (unifiedDiffPanelState?.panel === panel) {
        unifiedDiffPanelState = null;
      }
    });
  }

  unifiedDiffPanelState = { panel, pairKey };

  if (extensionContext) {
    void extensionContext.workspaceState.update('docdb.unifiedDiffLastState', { relativePath, panelTitle });
  }

  return { panel, pairKey, originalUri: oldUri, modifiedUri: newUri };
}

async function readDisplaySourceForUri(relativePath, targetUri, fallbackText = '') {
  if (!targetUri) {
    return String(fallbackText || '');
  }

  if (targetUri.scheme === CHAT_SNAPSHOT_SCHEME) {
    const preferredSnapshotText = String(fallbackText || '');
    if (preferredSnapshotText) {
      return preferredSnapshotText;
    }

    // .dx stub file snapshots hold only a pointer into the DXlite binary archive,
    // not actual document content. Decode the document path and read the
    // "before edit" source from the DXlite binary archive at git HEAD.
    const absolutePath = decodeAbsolutePathFromSnapshotUri(targetUri);
    if (absolutePath && relativePath) {
      const headSource = await readDocumentSnapshotAtRef(relativePath, absolutePath, 'HEAD', '');
      if (headSource) {
        return headSource;
      }
    }
    return String(fallbackText || '');
  }

  let candidateText = String(fallbackText || '');

  if (!candidateText) {
    try {
      const textDocument = await vscode.workspace.openTextDocument(targetUri);
      candidateText = textDocument.getText();
    } catch {
      candidateText = '';
    }
  }

  if (targetUri.scheme === 'git') {
    return await readDocumentSnapshotFromGit(relativePath, targetUri, candidateText);
  }

  const stubArchivePath = parseDocStubArchiveRelativePath(candidateText);

  if (stubArchivePath === null) {
    return candidateText;
  }

  try {
    return await readVirtualDocument(relativePath);
  } catch {
    return candidateText;
  }
}

async function refreshUnifiedDiffWhenSnapshotReady({
  panel,
  pairKey,
  relativePath,
  oldUri,
  newUri,
  initialTheme,
  initialAppearance,
  stylesUri,
  webviewUri,
  workspaceUri,
  workspaceRoot,
  baselineHeadSource,
}) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    await delay(250);

    if (!panel || unifiedDiffPanelState?.panel !== panel || unifiedDiffPanelState?.pairKey !== pairKey) {
      return;
    }

    let nextOldSource = '';
    let nextNewSource = '';

    try {
      const absolutePathForRef = newUri?.scheme === 'file' && newUri.fsPath
        ? path.resolve(String(newUri.fsPath))
        : path.resolve(String(workspaceRoot || ''), relativePath);

      const fallbackHeadSource = baselineHeadSource
        || await readDocumentSnapshotAtRef(relativePath, absolutePathForRef, 'HEAD', '');
      const fallbackCurrentSource = await readVirtualDocument(relativePath).catch(() => '');

      const snapshotPair = await readChatEditingDiffPairFromSnapshotUri(oldUri);
      nextOldSource = await readDisplaySourceForUri(relativePath, oldUri, snapshotPair.oldSource || fallbackHeadSource || '');
      nextNewSource = await readDisplaySourceForUri(relativePath, newUri, snapshotPair.newSource || fallbackCurrentSource || '');

      if (!nextOldSource && fallbackHeadSource) {
        nextOldSource = fallbackHeadSource;
      }

      if (!nextNewSource && fallbackCurrentSource) {
        nextNewSource = fallbackCurrentSource;
      }
    } catch {
      continue;
    }

    if (!nextOldSource || !nextNewSource || nextOldSource === nextNewSource) {
      continue;
    }

    panel.webview.html = renderEditorHtml(
      relativePath,
      nextNewSource,
      '',
      initialTheme,
      initialAppearance,
      panel.webview.cspSource,
      stylesUri,
      webviewUri,
      workspaceUri,
      'new',
      nextOldSource,
    );
    return;
  }
}

async function ensureDocFolderConfiguration() {
  const workspaceFolders = vscode.workspace.workspaceFolders;
  if (!workspaceFolders || workspaceFolders.length === 0) return;

  for (const folder of workspaceFolders) {
    const docPath = vscode.Uri.joinPath(folder.uri, '.doc');

    // Check if .doc folder exists
    try {
      await vscode.workspace.fs.stat(docPath);

      // .doc folder exists, apply configuration
      const config = vscode.workspace.getConfiguration();

      // 1. Auto-hide .doc folder if configured
      if (config.get('docdb.autoHideDocFolder', true)) {
        const filesExclude = config.get('files.exclude') || {};
        if (!filesExclude['.doc']) {
          filesExclude['.doc'] = true;
          await config.update('files.exclude', filesExclude, vscode.ConfigurationTarget.Workspace);
        }
      }

      // 2. Auto-create extensions.json recommendation if configured
      if (config.get('docdb.autoRecommend', true)) {
        const extJsonPath = vscode.Uri.joinPath(folder.uri, '.vscode', 'extensions.json');
        try {
          const extJsonData = await vscode.workspace.fs.readFile(extJsonPath);
          const extJson = JSON.parse(new TextDecoder().decode(extJsonData));

          if (!extJson.recommendations?.includes('alexwaldmann.docdb')) {
            if (!extJson.recommendations) extJson.recommendations = [];
            extJson.recommendations.push('alexwaldmann.docdb');
            await vscode.workspace.fs.writeFile(
              extJsonPath,
              new TextEncoder().encode(JSON.stringify(extJson, null, 2))
            );
          }
        } catch {
          // extensions.json doesn't exist or is invalid, create it
          const vscodePath = vscode.Uri.joinPath(folder.uri, '.vscode');
          try {
            await vscode.workspace.fs.stat(vscodePath);
          } catch {
            await vscode.workspace.fs.createDirectory(vscodePath);
          }

          const newExtJson = { recommendations: ['alexwaldmann.docdb'] };
          const extJsonPath = vscode.Uri.joinPath(folder.uri, '.vscode', 'extensions.json');
          await vscode.workspace.fs.writeFile(
            extJsonPath,
            new TextEncoder().encode(JSON.stringify(newExtJson, null, 2))
          );
        }
      }
    } catch {
      // .doc folder doesn't exist, skip
    }
  }
}
function normalizeVirtualPath(virtualPath) {
  const normalized = String(virtualPath || '').replace(/^\/+/, '');

  if (!normalized || !normalized.endsWith('.dx')) {
    throw new Error('A valid .dx virtual path is required.');
  }

  return normalized;
}

function ensureWithinRoot(root, targetPath) {
  const resolved = path.resolve(targetPath);
  const relative = path.relative(root, resolved);

  if (relative.startsWith('..') || path.isAbsolute(relative)) {
    throw new Error('Path must stay within workspace root.');
  }

  return resolved;
}

function sanitizeImageStem(value) {
  const stem = String(value || 'image')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return stem || 'image';
}

function normalizeDocPath(value) {
  return String(value || '').replace(/\\/g, '/').replace(/^\/+/, '');
}

async function persistViewStateSnapshot(relativePath, snapshot) {
  const workspaceRoot = getWorkspaceRoot();

  if (!workspaceRoot) {
    return;
  }

  const rel = normalizeDocPath(relativePath);

  if (!rel || !snapshot || typeof snapshot !== 'object') {
    return;
  }

  const viewStatePath = path.join(workspaceRoot, '.doc', 'view-state.json');
  await mkdir(path.dirname(viewStatePath), { recursive: true });

  const theme = String(snapshot.theme || 'auto');
  const resolvedTheme = String(snapshot.resolvedTheme || 'dark');
  const appearance = snapshot.appearance && typeof snapshot.appearance === 'object' ? snapshot.appearance : {};
  const viewport = snapshot.viewport && typeof snapshot.viewport === 'object' ? snapshot.viewport : {};
  const zoomLevelRaw = Number(vscode.workspace.getConfiguration('window').get('zoomLevel', 0));
  const zoomLevel = Number.isFinite(zoomLevelRaw) ? zoomLevelRaw : 0;
  const zoomFactor = Math.pow(1.2, zoomLevel);

  const normalizedSnapshot = {
    theme: ['auto', 'light', 'dark'].includes(theme) ? theme : 'auto',
    resolvedTheme: ['light', 'dark'].includes(resolvedTheme) ? resolvedTheme : 'dark',
    appearance: {
      paper: ['white', 'cream', 'slate'].includes(String(appearance.paper || 'white')) ? String(appearance.paper || 'white') : 'white',
      density: ['comfortable', 'compact'].includes(String(appearance.density || 'comfortable')) ? String(appearance.density || 'comfortable') : 'comfortable',
      scale: Number.isFinite(Number(appearance.scale)) ? Math.min(115, Math.max(90, Math.round(Number(appearance.scale)))) : 100,
    },
    viewport: {
      width: Number.isFinite(Number(viewport.width)) ? Math.max(1, Math.round(Number(viewport.width))) : null,
      height: Number.isFinite(Number(viewport.height)) ? Math.max(1, Math.round(Number(viewport.height))) : null,
      pixelRatio: Number.isFinite(Number(viewport.pixelRatio)) ? Number(viewport.pixelRatio) : null,
      zoomLevel,
      zoomFactor,
    },
    effectiveCss: String(snapshot.effectiveCss || ''),
    sourceHash: String(snapshot.sourceHash || ''),
    editBuffer: String(snapshot.editBuffer || ''),
  };

  let current = { version: 1, documents: {} };
  try {
    const existing = await readFile(viewStatePath, 'utf8');
    const parsed = JSON.parse(existing);
    if (parsed && typeof parsed === 'object' && parsed.documents && typeof parsed.documents === 'object') {
      current = {
        version: Number(parsed.version || 1),
        documents: parsed.documents,
      };
    }
  } catch {
    // Missing/invalid view state defaults to a fresh document map.
  }

  current.documents[rel] = {
    ...normalizedSnapshot,
    updatedAt: new Date().toISOString(),
  };

  await writeFile(viewStatePath, `${JSON.stringify(current, null, 2)}\n`, 'utf8');
}

function resetRuntime() {
  if (!runtimePromise) {
    runtimeRoot = null;
    return;
  }

  runtimePromise
    .then(() => {})
    .catch(() => {});

  runtimePromise = null;
  runtimeRoot = null;
}

async function getDocRuntime() {
  const workspaceRoot = getWorkspaceRoot();

  if (!workspaceRoot) {
    throw new Error('DOC runtime requires an open workspace folder.');
  }

  if (runtimePromise && runtimeRoot === workspaceRoot) {
    return runtimePromise;
  }

  runtimeRoot = workspaceRoot;
  runtimePromise = (async () => {
    const srcDir = path.join(workspaceRoot, 'build', 'runtime', 'src');
    const serviceModule = await import(pathToFileURL(path.join(srcDir, 'doc-service.js')).href);
    const archiveModule = await import(pathToFileURL(path.join(srcDir, 'doc-archive.js')).href);

    const docs = await serviceModule.listOrSearchDocuments(workspaceRoot, null, '');
    if (!Array.isArray(docs) || docs.length === 0) {
      await serviceModule.ingestWorkspace(workspaceRoot, null);
    }

    return {
      workspaceRoot,
      serviceModule,
      archiveModule,
    };
  })();

  return runtimePromise;
}

function parseDocStubArchiveRelativePath(sourceText) {
  const firstLine = String(sourceText || '').split(/\r?\n/, 1)[0].trim();

  if (!firstLine) {
    return null;
  }

  if (firstLine === '~' || firstLine === '@d3') {
    return '';
  }

  if (firstLine.startsWith('~ ')) {
    return firstLine.slice(2).trim();
  }

  if (firstLine.startsWith('@d3 ')) {
    return firstLine.slice(4).trim();
  }

  return null;
}

function parseUriQueryObject(rawQuery) {
  const raw = String(rawQuery || '');

  if (!raw) {
    return null;
  }

  try {
    return JSON.parse(raw);
  } catch {
    try {
      return JSON.parse(decodeURIComponent(raw));
    } catch {
      return null;
    }
  }
}

function toGitArchiveLookup(relativePath, uriQuery) {
  if (!uriQuery || typeof uriQuery !== 'object') {
    return null;
  }

  const ref = String(uriQuery.ref || uriQuery.revision || uriQuery.commit || '').trim();

  if (!ref) {
    return null;
  }

  const candidatePaths = [
    uriQuery.path,
    uriQuery.originalPath,
    uriQuery.uri,
    uriQuery.documentUri,
    relativePath,
  ];

  for (const candidate of candidatePaths) {
    const value = String(candidate || '').trim();

    if (!value) {
      continue;
    }

    if (value.startsWith('file:')) {
      try {
        const parsed = vscode.Uri.parse(value);
        if (parsed.scheme === 'file' && parsed.fsPath) {
          return { ref, absolutePath: parsed.fsPath };
        }
      } catch {
        // Ignore parse failures and continue searching candidates.
      }
    }
  }

  if (!relativePath) {
    return null;
  }

  const workspaceRoot = getWorkspaceRoot();
  if (!workspaceRoot) {
    return null;
  }

  return {
    ref,
    absolutePath: path.resolve(workspaceRoot, relativePath),
  };
}

async function readDocumentSnapshotFromGit(relativePath, documentUri, fallbackText) {
  const workspaceRoot = getWorkspaceRoot();

  if (!workspaceRoot || !relativePath || documentUri?.scheme !== 'git') {
    return String(fallbackText || '');
  }

  const uriQuery = parseUriQueryObject(documentUri.query);
  const lookup = toGitArchiveLookup(relativePath, uriQuery);

  if (!lookup) {
    return String(fallbackText || '');
  }

  const stubArchiveRelativePath = parseDocStubArchiveRelativePath(fallbackText);
  const archiveRelativePath = stubArchiveRelativePath && stubArchiveRelativePath.length > 0
    ? stubArchiveRelativePath
    : '.doc/.repo-docs.bin';

  try {
    const { stdout } = await execFileAsync('git', ['-C', workspaceRoot, 'show', `${lookup.ref}:${archiveRelativePath}`], {
      encoding: 'buffer',
      maxBuffer: 64 * 1024 * 1024,
    });

    const archiveBuffer = Buffer.isBuffer(stdout) ? stdout : Buffer.from(stdout || []);
    const { archiveModule } = await getDocRuntime();
    const parsed = archiveModule.readDocArchiveFromBuffer(workspaceRoot, lookup.absolutePath, archiveBuffer);

    return String(parsed?.source || fallbackText || '');
  } catch {
    return String(fallbackText || '');
  }
}

async function readDocumentSnapshotAtRef(relativePath, absolutePath, ref, fallbackText = '') {
  const workspaceRoot = getWorkspaceRoot();

  if (!workspaceRoot || !relativePath || !ref) {
    return String(fallbackText || '');
  }

  try {
    const { stdout } = await execFileAsync('git', ['-C', workspaceRoot, 'show', `${ref}:${relativePath}`], {
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    });

    const snapshotText = String(stdout || '');
    const stubArchiveRelativePath = parseDocStubArchiveRelativePath(snapshotText);

    if (stubArchiveRelativePath === null) {
      return snapshotText;
    }

    const archiveRelativePath = stubArchiveRelativePath && stubArchiveRelativePath.length > 0
      ? stubArchiveRelativePath
      : '.doc/.repo-docs.bin';
    const archiveResult = await execFileAsync('git', ['-C', workspaceRoot, 'show', `${ref}:${archiveRelativePath}`], {
      encoding: 'buffer',
      maxBuffer: 64 * 1024 * 1024,
    });

    const archiveBuffer = Buffer.isBuffer(archiveResult.stdout) ? archiveResult.stdout : Buffer.from(archiveResult.stdout || []);
    const { archiveModule } = await getDocRuntime();
    const parsed = archiveModule.readDocArchiveFromBuffer(workspaceRoot, absolutePath, archiveBuffer);
    return String(parsed?.source || snapshotText || fallbackText || '');
  } catch {
    return String(fallbackText || '');
  }
}

async function readUiConfig() {
  const configured = String(vscode.workspace.getConfiguration('docdb').get('preferredTheme', 'auto') || 'auto').toLowerCase();
  return { theme: THEME_OPTIONS.has(configured) ? configured : 'auto' };
}

async function writePreferredTheme(theme) {
  const normalizedTheme = THEME_OPTIONS.has(String(theme || '').toLowerCase()) ? String(theme).toLowerCase() : 'auto';
  await vscode.workspace.getConfiguration('docdb').update('preferredTheme', normalizedTheme, vscode.ConfigurationTarget.Workspace);
  return { theme: normalizedTheme };
}

function normalizeInitialAppearance(snapshot) {
  const appearance = snapshot && typeof snapshot === 'object' ? snapshot : {};
  const paper = String(appearance.paper || 'white');
  const density = String(appearance.density || 'comfortable');
  const scaleRaw = Number(appearance.scale);

  return {
    paper: ['white', 'cream', 'slate'].includes(paper) ? paper : 'white',
    density: ['comfortable', 'compact'].includes(density) ? density : 'comfortable',
    scale: Number.isFinite(scaleRaw) ? Math.min(115, Math.max(90, Math.round(scaleRaw))) : 100,
  };
}

async function uploadVirtualDocImage(virtualPath, image) {
  const { workspaceRoot } = await getDocRuntime();
  const normalizedVirtualDocPath = normalizeVirtualPath(virtualPath);
  const mimeType = String(image?.mimeType || '').toLowerCase().trim();
  const base64Data = String(image?.base64Data || '').trim();
  const originalName = String(image?.name || 'image');

  if (!mimeType.startsWith('image/')) {
    throw new Error('Only image uploads are supported.');
  }

  if (!base64Data) {
    throw new Error('Image payload is empty.');
  }

  const extension = IMAGE_EXT_BY_MIME[mimeType] || 'bin';
  const absoluteDocPath = ensureWithinRoot(workspaceRoot, path.join(workspaceRoot, normalizedVirtualDocPath));
  const docDir = path.dirname(absoluteDocPath);
  const docName = path.basename(absoluteDocPath, path.extname(absoluteDocPath));
  const assetsDir = path.join(docDir, `${docName}.assets`);
  await mkdir(assetsDir, { recursive: true });

  const stamp = new Date().toISOString().replace(/[^0-9]/g, '').slice(0, 14);
  const randomPart = Math.random().toString(36).slice(2, 8);
  const fileStem = sanitizeImageStem(path.basename(originalName, path.extname(originalName)));
  const fileName = `${stamp}-${randomPart}-${fileStem}.${extension}`;
  const absoluteImagePath = ensureWithinRoot(workspaceRoot, path.join(assetsDir, fileName));
  const bytes = Buffer.from(base64Data, 'base64');

  if (bytes.length === 0) {
    throw new Error('Invalid image payload.');
  }

  await writeFile(absoluteImagePath, bytes);
  return { path: path.relative(workspaceRoot, absoluteImagePath).replace(/\\/g, '/') };
}

async function readVirtualDocument(virtualPath) {
  const normalizedPath = normalizeVirtualPath(virtualPath);
  const { workspaceRoot, serviceModule } = await getDocRuntime();
  const document = await serviceModule.getDocumentByRelativePath(workspaceRoot, null, normalizedPath);

  if (!document) {
    throw new Error(`Document not found: ${normalizedPath}`);
  }

  return String(document.source || '');
}

async function writeVirtualDocument(virtualPath, sourceText) {
  const normalizedPath = normalizeVirtualPath(virtualPath);
  const { workspaceRoot, serviceModule } = await getDocRuntime();
  const document = await serviceModule.saveDocumentSourceByRelativePath(workspaceRoot, null, normalizedPath, String(sourceText || ''));
  return {
    document: {
      path: document.relativePath,
      title: document.title,
      updatedAt: document.updatedAt,
      sourceBytes: document.sourceBytes,
    },
  };
}

async function listVirtualDocuments() {
  const { workspaceRoot, serviceModule } = await getDocRuntime();
  const documents = await serviceModule.listOrSearchDocuments(workspaceRoot, null, '');

  return documents
    .map((document) => ({
      path: document.relativePath,
      title: document.title,
      updatedAt: document.updatedAt,
      sourceBytes: document.sourceBytes,
    }))
    .sort((left, right) => left.path.localeCompare(right.path));
}

function toWorkspaceRelativeDocPath(uri) {
  function normalizeRelativeCandidate(value) {
    const raw = String(value || '').trim();

    if (!raw) {
      return null;
    }

    if (path.isAbsolute(raw) || path.posix.isAbsolute(raw.replace(/\\/g, '/'))) {
      return null;
    }

    const normalized = raw
      .replace(/\\/g, '/')
      .replace(/^\.\/+/, '')
      .replace(/^\/+/, '');

    if (!normalized || !normalized.endsWith('.dx')) {
      return null;
    }

    if (normalized.startsWith('..') || normalized.includes('/../') || normalized.endsWith('/..')) {
      return null;
    }

    return normalized;
  }

  function fromAbsolutePath(absolutePath) {
    if (!absolutePath) {
      return null;
    }

    const resolvedPath = path.resolve(String(absolutePath));
    const workspaceFolders = vscode.workspace.workspaceFolders || [];

    for (const folder of workspaceFolders) {
      if (!folder || folder.uri.scheme !== 'file') {
        continue;
      }

      const relativePath = path.relative(folder.uri.fsPath, resolvedPath).replace(/\\/g, '/');

      if (!relativePath || relativePath.startsWith('..') || path.isAbsolute(relativePath)) {
        continue;
      }

      const normalized = normalizeRelativeCandidate(relativePath);

      if (normalized) {
        return normalized;
      }
    }

    return null;
  }

  function parseQueryObject(rawQuery) {
    const raw = String(rawQuery || '');

    if (!raw) {
      return null;
    }

    try {
      return JSON.parse(raw);
    } catch {
      try {
        return JSON.parse(decodeURIComponent(raw));
      } catch {
        return null;
      }
    }
  }

  function extractRelativeFromUriPayload(value) {
    const raw = String(value || '').trim();

    if (!raw) {
      return null;
    }

    const directRelative = normalizeRelativeCandidate(raw);
    if (directRelative) {
      return directRelative;
    }

    if (raw.startsWith('file:') || raw.startsWith('git:') || raw.startsWith('docdb:')) {
      try {
        const parsed = vscode.Uri.parse(raw);
        return toWorkspaceRelativeDocPath(parsed);
      } catch {
        return null;
      }
    }

    return fromAbsolutePath(raw);
  }

  if (uri && uri.scheme === 'docdb') {
    const virtualPath = String(uri.path || '').replace(/^\/+/, '');
    if (!virtualPath || !virtualPath.endsWith('.dx')) {
      return null;
    }
    return virtualPath;
  }

  const fromFsPath = fromAbsolutePath(uri?.fsPath);
  if (fromFsPath) {
    return fromFsPath;
  }

  const uriQuery = parseQueryObject(uri?.query);
  if (uriQuery && typeof uriQuery === 'object') {
    const queryCandidates = [
      uriQuery.path,
      uriQuery.originalPath,
      uriQuery.uri,
      uriQuery.documentUri,
      uriQuery.left,
      uriQuery.right,
    ];

    for (const candidate of queryCandidates) {
      const resolved = extractRelativeFromUriPayload(candidate);

      if (resolved) {
        return resolved;
      }
    }
  }

  const fromUriPath = extractRelativeFromUriPayload(uri?.path);
  if (fromUriPath) {
    return fromUriPath;
  }

  const workspaceFolder = vscode.workspace.getWorkspaceFolder(uri);

  if (!workspaceFolder || workspaceFolder.uri.scheme !== 'file') {
    return null;
  }

  const relativePath = path.relative(workspaceFolder.uri.fsPath, uri.fsPath).replace(/\\/g, '/');

  if (!relativePath || relativePath.startsWith('..') || path.isAbsolute(relativePath) || !relativePath.endsWith('.dx')) {
    return null;
  }

  return relativePath;
}

function getWorkspaceRoot() {
  const activeUri = vscode.window.activeTextEditor?.document?.uri;

  if (activeUri) {
    const activeFolder = vscode.workspace.getWorkspaceFolder(activeUri);

    if (activeFolder && activeFolder.uri.scheme === 'file') {
      return activeFolder.uri.fsPath;
    }
  }

  const folders = vscode.workspace.workspaceFolders || [];
  const fileFolder = folders.find((folder) => folder.uri.scheme === 'file');
  return fileFolder ? fileFolder.uri.fsPath : null;
}

function renderEditorHtml(relativePath, sourceText, errorText = '', initialTheme = 'auto', initialAppearance = null, cspSource = "'none'", stylesUri = '', webviewUri = '', workspaceUri = '', diffRole = 'none', comparisonSourceText = '') {
  const appearance = normalizeInitialAppearance(initialAppearance);
  const initialScale = String(appearance.scale / 100);
  function escapeHtml(value) {
    return String(value || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function sanitizeRichMarkup(value) {
    let text = String(value || '');
    text = text.replace(/<script[\s\S]*?>[\s\S]*?<\/script>/gi, '');
    text = text.replace(/\son[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi, '');
    text = text.replace(/\s(?:href|src|xlink:href)\s*=\s*(['"])\s*javascript:[\s\S]*?\1/gi, '');
    text = text.replace(/\s(?:href|src|xlink:href)\s*=\s*javascript:[^\s>]+/gi, '');
    return text;
  }

  function extractSvgMarkup(value) {
    const match = /<svg[\s\S]*?<\/svg>/i.exec(String(value || ''));
    return match ? match[0] : '';
  }

  function parseSourceText(text) {
    const normalized = String(text || '').replace(/\r\n/g, '\n');
    const lines = normalized.split('\n');
    const metadata = { title: '', summary: '', tags: '' };
    const blocks = [];
    let cursor = 0;

    if (lines[0] && lines[0].startsWith('@doc')) {
      cursor = 1;
    }

    for (; cursor < lines.length; cursor += 1) {
      const line = lines[cursor].trim();

      if (line === '---') {
        cursor += 1;
        break;
      }

      const colonIndex = line.indexOf(':');

      if (colonIndex !== -1) {
        const key = line.slice(0, colonIndex).trim().toLowerCase();
        const value = line.slice(colonIndex + 1).trim();

        if (key === 'title') {
          metadata.title = value;
        } else if (key === 'summary') {
          metadata.summary = value;
        } else if (key === 'tags') {
          metadata.tags = value;
        }
      }
    }

    while (cursor < lines.length) {
      const line = lines[cursor].trim();

      if (!line) {
        cursor += 1;
        continue;
      }

      if (!line.startsWith('::')) {
        blocks.push({ type: 'paragraph', text: lines[cursor] });
        cursor += 1;
        continue;
      }

      const match = /^::([a-z-]+)(.*)$/i.exec(line);

      if (!match) {
        cursor += 1;
        continue;
      }

      const type = match[1].toLowerCase();
      const args = match[2] || '';
      const content = [];
      cursor += 1;

      while (cursor < lines.length && lines[cursor].trim() !== '::end') {
        content.push(lines[cursor]);
        cursor += 1;
      }

      if (cursor < lines.length && lines[cursor].trim() === '::end') {
        cursor += 1;
      }

      if (type === 'heading') {
        const levelMatch = /level=(\d+)/.exec(args);
        blocks.push({ type, level: Math.min(6, Math.max(1, Number(levelMatch ? levelMatch[1] : '1'))), text: content.join('\n').trim() });
      } else if (type === 'list' || type === 'bulleted-list') {
        blocks.push({ type: 'bulleted-list', items: content.map((item) => item.replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim()).filter(Boolean) });
      } else if (type === 'numbered-list') {
        blocks.push({ type: 'numbered-list', items: content.map((item) => item.replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim()).filter(Boolean) });
      } else if (type === 'checklist') {
        blocks.push({
          type,
          items: content.map((item) => {
            const itemMatch = /^\s*\[(x| )\]\s*(.*)$/i.exec(item.trim());
            return itemMatch ? { checked: itemMatch[1].toLowerCase() === 'x', text: itemMatch[2] } : { checked: false, text: item.trim() };
          }).filter((item) => item.text.length > 0),
        });
      } else if (type === 'image') {
        const srcMatch = /src=([^\s]+)/.exec(args);
        blocks.push({ type, src: srcMatch ? srcMatch[1] : '', alt: content.join('\n').trim() });
      } else if (type === 'code') {
        const langMatch = /(?:lang|language)=([^\s]+)/.exec(args);
        blocks.push({ type, language: langMatch ? langMatch[1] : '', text: content.join('\n').trimEnd() });
      } else {
        blocks.push({ type, text: content.join('\n').trimEnd() });
      }
    }

    return { metadata, blocks };
  }

  function renderBlockPreview(block) {
    if (block.type === 'heading') {
      const level = Math.min(6, Math.max(1, Number(block.level || 1)));
      return `<h${level}>${escapeHtml(block.text || '')}</h${level}>`;
    }

    if (block.type === 'list' || block.type === 'bulleted-list') {
      return `<ul>${(block.items || []).map((item) => `<li>${escapeHtml(item)}</li>`).join('')}</ul>`;
    }

    if (block.type === 'numbered-list') {
      return `<ol>${(block.items || []).map((item) => `<li>${escapeHtml(item)}</li>`).join('')}</ol>`;
    }

    if (block.type === 'checklist') {
      return `<ul class="checklist-wrap">${(block.items || []).map((item) => `<li><input type="checkbox" ${item.checked ? 'checked' : ''} disabled /><span${item.checked ? ' class="check-done"' : ''}>${escapeHtml(item.text)}</span></li>`).join('')}</ul>`;
    }

    if (block.type === 'code') {
      const language = String(block.language || '').trim().toLowerCase();
      const rawText = String(block.text || '');

      if (language === 'svg') {
        const svgMarkup = extractSvgMarkup(rawText);
        if (svgMarkup) {
          return `<div class="svg-wrap">${sanitizeRichMarkup(svgMarkup)}</div>`;
        }
      }

      if (language === 'html') {
        return `<div class="html-wrap">${sanitizeRichMarkup(rawText)}</div>`;
      }

      return `<pre>${escapeHtml(rawText)}</pre>`;
    }

    if (block.type === 'quote') {
      return `<blockquote><p>${escapeHtml(block.text || '')}</p></blockquote>`;
    }

    if (block.type === 'image') {
      const src = escapeHtml(block.src || '');
      const alt = escapeHtml(block.alt || '');
      const caption = alt ? `<figcaption>${alt}</figcaption>` : '';
      return `<figure class="image-wrap"><img src="${src}" alt="${alt}" loading="lazy" />${caption}</figure>`;
    }

    if (block.type === 'svg') {
      const svgMarkup = extractSvgMarkup(block.text || '');
      if (svgMarkup) {
        return `<div class="svg-wrap">${sanitizeRichMarkup(svgMarkup)}</div>`;
      }
      return `<pre>${escapeHtml(block.text || '')}</pre>`;
    }

    if (block.type === 'html') {
      return `<div class="html-wrap">${sanitizeRichMarkup(block.text || '')}</div>`;
    }

    if (block.type === 'graph' || block.type === 'mermaid') {
      const graphText = String(block.text || '');
      const svgMarkup = extractSvgMarkup(graphText);
      if (svgMarkup) {
        return `<div class="graph-wrap">${sanitizeRichMarkup(svgMarkup)}</div>`;
      }
      return `<pre>${escapeHtml(graphText)}</pre>`;
    }

    return `<p>${escapeHtml(block.text || '')}</p>`;
  }

  const initialModel = parseSourceText(sourceText);
  const initialLoadNote = errorText
    ? `<div class="load-note error" id="load-note">Source load warning: ${escapeHtml(errorText)}</div>`
    : '';
  const initialBlocks = initialModel.blocks.map((block, index) => `
        <div class="block-wrap" data-block-index="${index}">
          <div class="block-view">${renderBlockPreview(block)}</div>
          <div class="block-src-wrapper">
            <textarea class="block-src" aria-label="Edit block source"></textarea>
          </div>
        </div>`).join('');
  const initialMarkup = `
    <div class="page" data-edit-mode="true" data-ready="false" aria-busy="true">
      <div class="loading-screen" id="loading-screen">
        <div class="loading-card" role="status" aria-live="polite" aria-label="Loading">
          <div class="loading-spinner" aria-hidden="true">
            <div class="spinner-block"></div>
            <div class="spinner-block"></div>
            <div class="spinner-block"></div>
          </div>
        </div>
      </div>
      <div id="doc-init" data-doc-path="${escapeHtml(relativePath || 'unknown.dx')}" data-doc-error="${escapeHtml(errorText || '')}" data-initial-theme="${escapeHtml(initialTheme || 'auto')}" data-initial-paper="${escapeHtml(appearance.paper)}" data-initial-density="${escapeHtml(appearance.density)}" data-initial-scale="${escapeHtml(String(appearance.scale))}" data-workspace-uri="${escapeHtml(workspaceUri || '')}" data-diff-role="${escapeHtml(diffRole || 'none')}" hidden></div>
      <textarea id="doc-init-source" hidden>${escapeHtml(sourceText || '')}</textarea>
      <textarea id="doc-init-compare-source" hidden>${escapeHtml(comparisonSourceText || '')}</textarea>
      ${initialLoadNote}

      <div class="ui-chrome" id="ui-chrome" data-open="false" data-help="false">
        <div class="ui-chrome-btns">
          <div class="mode-pill" id="mode-pill" data-mode="edit">Editing</div>
          <button class="ui-chrome-edit-toggle" id="ui-chrome-edit-toggle" type="button" aria-pressed="true" title="Toggle edit mode">✎</button>
          <button class="ui-chrome-help-btn" id="ui-chrome-help-btn" type="button" aria-expanded="false" title="Help">?</button>
          <button class="ui-chrome-toggle" id="ui-chrome-toggle" type="button" aria-expanded="false" title="Settings">/</button>
        </div>
        <div class="ui-chrome-panel" role="group" aria-label="Document appearance settings">
          <div class="ui-row">
            <label for="theme-select">Theme</label>
            <select id="theme-select">
              <option value="auto">Auto</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </div>
          <div class="ui-row">
            <label for="paper-select">Paper</label>
            <select id="paper-select">
              <option value="white">White</option>
              <option value="cream">Cream</option>
              <option value="slate">Slate</option>
            </select>
          </div>
          <div class="ui-row">
            <label for="density-select">Density</label>
            <select id="density-select">
              <option value="comfortable">Comfortable</option>
              <option value="compact">Compact</option>
            </select>
          </div>
          <div class="ui-row">
            <label for="scale-slider">Scale</label>
            <input id="scale-slider" type="range" min="90" max="115" step="1" value="100" />
          </div>
        </div>
      </div>

      <div id="blocks">${initialBlocks || ''}</div>
    </div>`;

  return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${cspSource}; script-src ${cspSource}; img-src ${cspSource} data: https:; font-src ${cspSource} https:;" />
    <link rel="stylesheet" href="${stylesUri}" />
  </head>
  <body data-theme="${escapeHtml(initialTheme || 'auto')}" data-paper="${escapeHtml(appearance.paper)}" data-density="${escapeHtml(appearance.density)}">
    ${initialMarkup}
    <script type="module" src="${webviewUri}"><\/script>
  </body>
</html>`;
}

class DocDbFileSystemProvider {
  constructor() {
    this._emitter = new vscode.EventEmitter();
    this.onDidChangeFile = this._emitter.event;
    this._cache = null;
    this._cacheTime = 0;
  }

  _virtualPath(uri) {
    return uri.path.replace(/^\/+/, '');
  }

  _isRoot(uri) {
    return uri.path === '' || uri.path === '/';
  }

  async _fetchFiles(force = false) {
    const now = Date.now();

    if (!force && this._cache && now - this._cacheTime < 1000) {
      return this._cache;
    }

    const files = await listVirtualDocuments();
    this._cache = files;
    this._cacheTime = now;
    return files;
  }

  _buildTree(files) {
    const root = {
      type: vscode.FileType.Directory,
      children: new Map(),
      mtime: Date.now(),
      size: 0,
    };

    for (const file of files) {
      const virtualPath = String(file.path || '').replace(/^\/+/, '');

      if (!virtualPath) {
        continue;
      }

      const parts = virtualPath.split('/').filter(Boolean);
      let cursor = root;

      for (let i = 0; i < parts.length; i += 1) {
        const name = parts[i];
        const isLeaf = i === parts.length - 1;

        if (!cursor.children.has(name)) {
          cursor.children.set(name, {
            type: isLeaf ? vscode.FileType.File : vscode.FileType.Directory,
            children: new Map(),
            mtime: file.updatedAt ? new Date(file.updatedAt).getTime() : Date.now(),
            size: Number(file.sourceBytes || 0),
          });
        }

        const child = cursor.children.get(name);

        if (!isLeaf) {
          child.type = vscode.FileType.Directory;
          cursor = child;
        }
      }
    }

    return root;
  }

  async _lookupNode(uri) {
    if (this._isRoot(uri)) {
      return {
        type: vscode.FileType.Directory,
        mtime: Date.now(),
        size: 0,
      };
    }

    const files = await this._fetchFiles();
    const tree = this._buildTree(files);
    const parts = this._virtualPath(uri).split('/').filter(Boolean);
    let cursor = tree;

    for (const part of parts) {
      const next = cursor.children.get(part);

      if (!next) {
        return null;
      }

      cursor = next;
    }

    return cursor;
  }

  watch() {
    return new vscode.Disposable(() => {});
  }

  async stat(uri) {
    const node = await this._lookupNode(uri);

    if (!node) {
      throw vscode.FileSystemError.FileNotFound(uri);
    }

    return {
      type: node.type,
      ctime: node.mtime,
      mtime: node.mtime,
      size: node.size,
    };
  }

  async readDirectory(uri) {
    const files = await this._fetchFiles();
    const tree = this._buildTree(files);
    let cursor = tree;

    if (!this._isRoot(uri)) {
      const parts = this._virtualPath(uri).split('/').filter(Boolean);

      for (const part of parts) {
        const next = cursor.children.get(part);

        if (!next) {
          throw vscode.FileSystemError.FileNotFound(uri);
        }

        cursor = next;
      }
    }

    if (cursor.type !== vscode.FileType.Directory) {
      throw vscode.FileSystemError.FileNotADirectory(uri);
    }

    return Array.from(cursor.children.entries()).map(([name, node]) => [name, node.type]);
  }

  async readFile(uri) {
    const virtualPath = this._virtualPath(uri);

    try {
      const text = await readVirtualDocument(virtualPath);
      return Buffer.from(text, 'utf8');
    } catch {
      throw vscode.FileSystemError.FileNotFound(uri);
    }
  }

  async writeFile(uri, content) {
    const virtualPath = this._virtualPath(uri);

    try {
      await writeVirtualDocument(virtualPath, Buffer.from(content).toString('utf8'));
    } catch {
      throw vscode.FileSystemError.Unavailable(`Unable to save ${virtualPath}`);
    }

    this._cache = null;
    this._cacheTime = 0;
    this._emitter.fire([
      { type: vscode.FileChangeType.Changed, uri },
      { type: vscode.FileChangeType.Changed, uri: uri.with({ path: path.posix.dirname(uri.path) || '/' }) },
      { type: vscode.FileChangeType.Changed, uri: vscode.Uri.parse('docdb:/') },
    ]);
  }

  createDirectory() {
    throw vscode.FileSystemError.NoPermissions('Directories are materialized from document paths in SQLite.');
  }

  delete() {
    throw vscode.FileSystemError.NoPermissions('Delete is not implemented for DOC virtual files.');
  }

  rename() {
    throw vscode.FileSystemError.NoPermissions('Rename is not implemented for DOC virtual files.');
  }
}

class DocDbCustomEditorProvider {
  constructor(extensionUri) {
    this._extensionUri = extensionUri;
  }

  async resolveCustomTextEditor(document, webviewPanel) {
    const { stylesUri, webviewUri, workspaceUri, workspaceRoot } = getWebviewAssetUris(webviewPanel.webview, this._extensionUri);
    configureDocWebview(webviewPanel.webview, this._extensionUri, workspaceRoot);

    const autoRouteToUnified = vscode.workspace.getConfiguration('docdb').get('unifiedDiffAutoOpen', true);
    const automaticDiffContext = getUnifiedDiffContextForDocument(document.uri);

    if (autoRouteToUnified && automaticDiffContext?.otherUri) {
      const normalizedPair = automaticDiffContext.diffRole === 'old'
        ? { originalUri: document.uri, modifiedUri: automaticDiffContext.otherUri }
        : { originalUri: automaticDiffContext.otherUri, modifiedUri: document.uri };
      const routeKey = toUnifiedDiffPairKey(normalizedPair.originalUri, normalizedPair.modifiedUri);

      if (!unifiedDiffAutoRouteInFlight.has(routeKey)) {
        unifiedDiffAutoRouteInFlight.add(routeKey);
        try {
          const opened = await openUnifiedDxDiffPanel(this._extensionUri, normalizedPair.originalUri, normalizedPair.modifiedUri);
          if (opened?.originalUri && opened?.modifiedUri) {
            await closeAllNativeDxDiffTabs();
          }
          webviewPanel.dispose();
          return;
        } catch {
          // Keep native editor open if unified auto-route fails.
        } finally {
          unifiedDiffAutoRouteInFlight.delete(routeKey);
        }
      }
    }

    const relativePath = toWorkspaceRelativeDocPath(document.uri);

    let initialTheme = 'auto';
    let initialAppearance = null;

    try {
      const config = await readUiConfig();
      initialTheme = String(config?.theme || 'auto');
    } catch {
      initialTheme = 'auto';
    }

    try {
      const { db, dbModule } = await getDocRuntime();
      const absolutePath = path.resolve(getWorkspaceRoot() || '', relativePath || '');
      const documentRow = dbModule.getDocumentByPath(db, getWorkspaceRoot() || '', absolutePath);
      initialAppearance = normalizeInitialAppearance(dbModule.getDocumentViewState(db, documentRow?.id));
    } catch {
      initialAppearance = null;
    }

    if (!relativePath) {
      webviewPanel.webview.html = renderEditorHtml('', '', 'Unable to map this file into workspace-relative .dx path.', initialTheme, initialAppearance, webviewPanel.webview.cspSource, stylesUri, webviewUri, workspaceUri);
      return;
    }

    const diffContext = getUnifiedDiffContextForDocument(document.uri);

    const readDisplaySourceForCurrentPane = async () => {
      return await readDisplaySourceForUri(relativePath, document.uri, document.getText());
    };

    const readComparisonSourceForCurrentPane = async (paneSourceText) => {
      if (diffContext?.otherUri) {
        return await readDisplaySourceForUri(relativePath, diffContext.otherUri, '');
      }

      const absolutePath = path.resolve(workspaceRoot || '', relativePath || '');

      if (document.uri?.scheme === 'git') {
        try {
          return await readVirtualDocument(relativePath);
        } catch {
          return '';
        }
      }

      if (document.uri?.scheme === 'file') {
        return await readDocumentSnapshotAtRef(relativePath, absolutePath, 'HEAD', '');
      }

      return String(paneSourceText || '');
    };

    let sourceText = '';
    let comparisonSourceText = '';
    let diffRole = 'none';
    let loadError = '';

    try {
      sourceText = await readDisplaySourceForCurrentPane();
      comparisonSourceText = await readComparisonSourceForCurrentPane(sourceText);

      if (comparisonSourceText && (diffContext?.diffRole === 'old' || diffContext?.diffRole === 'new')) {
        diffRole = diffContext.diffRole;
      } else if (document.uri?.scheme === 'git' && comparisonSourceText) {
        diffRole = 'old';
      } else if (document.uri?.scheme === 'file' && comparisonSourceText) {
        diffRole = 'new';
      }
    } catch (error) {
      loadError = error instanceof Error ? error.message : 'Failed to load document from SQLite.';

      try {
        sourceText = document.getText();
      } catch {
        sourceText = '';
      }
    }

    webviewPanel.webview.html = renderEditorHtml(relativePath, sourceText, loadError, initialTheme, initialAppearance, webviewPanel.webview.cspSource, stylesUri, webviewUri, workspaceUri, diffRole, comparisonSourceText);

    let sourcePushTimer = null;
    let suppressedEchoSource = null;
    let suppressNextChangePush = false;
    let dirtySyncTimer = null;
    let pendingDirtySource = null;
    // The stub pointer text last written to the on-disk working copy. Used to
    // restore the buffer when the webview signals that all edits were reverted.
    let savedStubText = document.getText();

    const replaceWorkingCopyText = async (nextText) => {
      const desiredText = String(nextText || '');
      const currentText = document.getText();

      if (currentText === desiredText) {
        return false;
      }

      const fullRange = new vscode.Range(
        document.positionAt(0),
        document.positionAt(currentText.length),
      );

      const edit = new vscode.WorkspaceEdit();
      edit.replace(document.uri, fullRange, desiredText);

      suppressNextChangePush = true;
      await vscode.workspace.applyEdit(edit);
      return true;
    };

    const flushDirtyWorkingCopySync = async () => {
      const nextText = pendingDirtySource;
      pendingDirtySource = null;

      if (typeof nextText !== 'string') {
        return;
      }

      try {
        await replaceWorkingCopyText(nextText);
      } catch {
        // Ignore dirty-sync failures; explicit save path will still persist content.
      }
    };

    const scheduleDirtyWorkingCopySync = (nextText) => {
      pendingDirtySource = String(nextText || '');

      if (dirtySyncTimer) {
        clearTimeout(dirtySyncTimer);
      }

      dirtySyncTimer = setTimeout(() => {
        dirtySyncTimer = null;
        void flushDirtyWorkingCopySync();
      }, 100);
    };

    const pushLatestSourceToWebview = async () => {
      try {
        const latestSource = await readDisplaySourceForCurrentPane();

        if (suppressedEchoSource !== null && latestSource === suppressedEchoSource) {
          suppressedEchoSource = null;
          return;
        }

        webviewPanel.webview.postMessage({ type: 'set-source', text: latestSource });
      } catch {
        // Ignore transient reload failures; next change or manual refresh can recover.
      }
    };

    const schedulePushLatestSourceToWebview = () => {
      if (sourcePushTimer) {
        clearTimeout(sourcePushTimer);
      }

      sourcePushTimer = setTimeout(() => {
        sourcePushTimer = null;
        void pushLatestSourceToWebview();
      }, 90);
    };

    const matchesOpenDocument = (targetUri) => {
      return targetUri && document.uri && targetUri.toString() === document.uri.toString();
    };

    const onSaved = vscode.workspace.onDidSaveTextDocument(async (savedDocument) => {
      if (!matchesOpenDocument(savedDocument?.uri)) return;
      schedulePushLatestSourceToWebview();
    });

    const onChanged = vscode.workspace.onDidChangeTextDocument(async (changeEvent) => {
      if (!matchesOpenDocument(changeEvent?.document?.uri)) return;

      if (suppressNextChangePush) {
        suppressNextChangePush = false;
        return;
      }

      schedulePushLatestSourceToWebview();
    });

    webviewPanel.onDidDispose(() => {
      if (sourcePushTimer) {
        clearTimeout(sourcePushTimer);
        sourcePushTimer = null;
      }
      if (dirtySyncTimer) {
        clearTimeout(dirtySyncTimer);
        dirtySyncTimer = null;
      }
      onSaved.dispose();
      onChanged.dispose();
    });

    webviewPanel.webview.onDidReceiveMessage(async (message) => {
      if (!message || !message.type) {
        return;
      }

      if (message.type === 'get-config') {
        try {
          const config = await readUiConfig();
          webviewPanel.webview.postMessage({ type: 'config', theme: String(config?.theme || 'auto') });
        } catch {
          webviewPanel.webview.postMessage({ type: 'config', theme: 'auto' });
        }
        return;
      }

      if (message.type === 'set-theme') {
        try {
          const config = await writePreferredTheme(String(message.theme || 'auto'));
          webviewPanel.webview.postMessage({ type: 'config', theme: String(config?.theme || 'auto') });
        } catch (error) {
          const messageText = error instanceof Error ? error.message : 'Unable to save theme preference.';
          webviewPanel.webview.postMessage({ type: 'status', text: messageText });
        }
        return;
      }

      if (message.type === 'upload-image') {
        try {
          const result = await uploadVirtualDocImage(relativePath, {
            name: String(message?.name || 'image'),
            mimeType: String(message?.mimeType || ''),
            base64Data: String(message?.base64Data || ''),
          });

          webviewPanel.webview.postMessage({
            type: 'image-uploaded',
            path: String(result?.path || ''),
            alt: String(message?.alt || ''),
            insertAt: typeof message?.insertAt === 'number' ? message.insertAt : -1,
          });
        } catch (error) {
          const messageText = error instanceof Error ? error.message : 'Image upload failed.';
          webviewPanel.webview.postMessage({ type: 'status', text: messageText });
        }
        return;
      }

      if (message.type === 'view-state') {
        try {
          const payload = message && message.payload && typeof message.payload === 'object' ? message.payload : {};
          const payloadPath = String(payload.docPath || relativePath || '');
          await persistViewStateSnapshot(payloadPath || relativePath, payload);
        } catch {
          // Ignore snapshot persistence issues; capture falls back to defaults.
        }
        return;
      }

      if (message.type === 'open-doc') {
        const relPath = String(message.path || '').trim();
        if (relPath && !relPath.includes('..') && /^[\w][\w.\/\-]*\.dx$/.test(relPath)) {
          const targetUri = vscode.Uri.joinPath(vscode.Uri.file(workspaceRoot), relPath);
          vscode.commands.executeCommand('vscode.open', targetUri);
        }
        return;
      }

      if (message.type === 'mark-dirty') {
        scheduleDirtyWorkingCopySync(String(message.text || ''));
        return;
      }

      if (message.type === 'mark-clean') {
        // Restore the buffer to the saved stub so VS Code's SCM status clears.
        if (dirtySyncTimer) {
          clearTimeout(dirtySyncTimer);
          dirtySyncTimer = null;
        }
        pendingDirtySource = null;
        await replaceWorkingCopyText(savedStubText);

        // VS Code dirty tracking is version-based, not text-equality-based.
        // After undo/redo churn, a document can remain dirty even when logical
        // content is back at the saved point, so persist the clean point when
        // the webview explicitly signals mark-clean.
        if (document.isDirty) {
          try {
            await document.save();
          } catch {
            // Non-fatal; clean sync still restored buffer text.
          }
        }
        return;
      }

      if (message.type !== 'save') {
        return;
      }

      try {
        const saveText = String(message.text || '');
        const saveRequestId = Number(message.requestId || 0);
        if (dirtySyncTimer) {
          clearTimeout(dirtySyncTimer);
          dirtySyncTimer = null;
        }
        pendingDirtySource = null;

        // Save the source to SQLite + archive and get back the stub pointer
        // text that belongs on disk. The .dx file is always a pointer, never
        // the raw source.
        let stubText = null;
        try {
          const { workspaceRoot, db, serviceModule } = await getDocRuntime();
          const result = await serviceModule.saveDocumentSourceToDbAndArchive(workspaceRoot, db, relativePath, saveText);
          stubText = result.stubText;
        } catch {
          // Non-fatal: fall back to replacing working copy with raw source.
        }

        // Write the stub pointer (or raw source as last resort) into the
        // VS Code document buffer, then let VS Code flush it to disk.
        const writtenStub = stubText ?? saveText;
        await replaceWorkingCopyText(writtenStub);
        savedStubText = writtenStub;

        const saved = await document.save();
        if (!saved) {
          throw new Error('Save was cancelled.');
        }

        suppressedEchoSource = saveText;
        webviewPanel.webview.postMessage({ type: 'save-complete', requestId: saveRequestId });
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : 'Save failed.';
        webviewPanel.webview.postMessage({ type: 'save-error', requestId: Number(message.requestId || 0), error: errorMsg });
      }
    });
  }
}

function ensureMounted() {
  const folder = vscode.Uri.parse('docdb:/');
  const existing = vscode.workspace.workspaceFolders || [];

  if (existing.some((item) => item.uri.scheme === 'docdb')) {
    return;
  }

  vscode.workspace.updateWorkspaceFolders(existing.length, 0, {
    uri: folder,
    name: 'DOC DB',
  });
}

function unmountDocDbFolder() {
  const existing = vscode.workspace.workspaceFolders || [];
  const index = existing.findIndex((item) => item.uri.scheme === 'docdb');

  if (index >= 0) {
    vscode.workspace.updateWorkspaceFolders(index, 1);
  }
}

async function openWelcomeDocumentOnFirstActivation(context) {
  if (context.globalState.get(WELCOME_DOC_OPENED_KEY, false)) {
    return;
  }

  const workspaceRoot = getWorkspaceRoot();
  if (!workspaceRoot) {
    return;
  }

  const welcomeUri = vscode.Uri.file(path.join(workspaceRoot, WELCOME_DOC_RELATIVE_PATH));

  try {
    await vscode.workspace.fs.stat(welcomeUri);
    await vscode.commands.executeCommand('vscode.openWith', welcomeUri, 'docdb.stubPreview');
    await context.globalState.update(WELCOME_DOC_OPENED_KEY, true);
  } catch {
    // Ignore first-run open failures; user can open the document manually.
  }
}

function activate(context) {
  extensionContext = context;
  const provider = new DocDbFileSystemProvider();
  const customEditor = new DocDbCustomEditorProvider(context.extensionUri);
  const chatSnapshotProvider = new ChatEditingSnapshotContentProvider();

  const openUnifiedDiffFromInput = async (input) => {
    const resolved = getDxDiffInputUris(input);

    if (!resolved) {
      return false;
    }

    await openUnifiedDxDiffPanel(context.extensionUri, resolved.originalUri, resolved.modifiedUri);
    return true;
  };

  const maybeRouteActiveDiffToUnified = async () => {
    const autoRoute = vscode.workspace.getConfiguration('docdb').get('unifiedDiffAutoOpen', true);

    if (!autoRoute) {
      return;
    }

    const activeTabGroup = vscode.window.tabGroups.activeTabGroup;
    const input = activeTabGroup?.activeTab?.input;

    if (!isDxDiffInput(input)) {
      return;
    }

    const resolved = getDxDiffInputUris(input);

    if (!resolved) {
      return;
    }

    const routeKey = toUnifiedDiffPairKey(resolved.originalUri, resolved.modifiedUri);

    if (unifiedDiffAutoRouteInFlight.has(routeKey)) {
      return;
    }

    unifiedDiffAutoRouteInFlight.add(routeKey);

    try {
      void closeAllNativeDxDiffTabs();
      await openUnifiedDiffFromInput(input);
    } catch {
      // If unified routing fails, keep native diff view intact.
    } finally {
      unifiedDiffAutoRouteInFlight.delete(routeKey);
    }
  };

  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(CHAT_SNAPSHOT_SCHEME, chatSnapshotProvider)
  );

  context.subscriptions.push(
    vscode.workspace.registerFileSystemProvider('docdb', provider, {
      isCaseSensitive: true,
      isReadonly: false,
    })
  );

  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider('docdb.stubPreview', customEditor, {
      webviewOptions: {
        retainContextWhenHidden: true,
      },
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand('docdb.mount', () => {
      ensureMounted();
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand('docdb.unmount', () => {
      unmountDocDbFolder();
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand('docdb.refresh', () => {
      resetRuntime();
      provider._cache = null;
      provider._cacheTime = 0;
      provider._emitter.fire([
        { type: vscode.FileChangeType.Changed, uri: vscode.Uri.parse('docdb:/') },
      ]);
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand('docdb.openUnifiedDiff', async () => {
      const activeTab = vscode.window.tabGroups.activeTabGroup?.activeTab;
      const input = activeTab?.input;

      if (!input) {
        vscode.window.showInformationMessage('No active tab to open as unified diff.');
        return;
      }

      const opened = await openUnifiedDiffFromInput(input);

      if (!opened) {
        vscode.window.showInformationMessage('Active diff does not target a DX document.');
      }
    })
  );

  context.subscriptions.push(
    vscode.window.registerWebviewPanelSerializer('docdb.unifiedDiff', {
      async deserializeWebviewPanel(webviewPanel, _serializedState) {
        const savedState = extensionContext?.workspaceState.get('docdb.unifiedDiffLastState');
        if (!savedState?.relativePath) {
          webviewPanel.webview.html = `<html><body style="padding:2rem;color:var(--vscode-foreground)">
            <p>DX diff context is no longer available. Re-open the file to compare again.</p>
          </body></html>`;
          return;
        }
        const { relativePath: rp, panelTitle: pt } = savedState;
        webviewPanel.title = String(pt || rp);
        const { stylesUri, webviewUri, workspaceUri, workspaceRoot } = getWebviewAssetUris(webviewPanel.webview, context.extensionUri);
        configureDocWebview(webviewPanel.webview, context.extensionUri, workspaceRoot);
        let initialTheme = 'auto';
        try { initialTheme = String((await readUiConfig())?.theme || 'auto'); } catch {}
        let oldSource = '';
        let newSource = '';
        try {
          const absolutePath = path.resolve(String(workspaceRoot || ''), rp);
          newSource = await readVirtualDocument(rp).catch(() => '');
          if (!newSource) {
            const td = await vscode.workspace.openTextDocument(vscode.Uri.file(absolutePath));
            newSource = td.getText();
          }
          oldSource = await readDocumentSnapshotAtRef(rp, absolutePath, 'HEAD', '');
        } catch {}
        const hasDiff = Boolean(oldSource && newSource && oldSource !== newSource);
        webviewPanel.webview.html = renderEditorHtml(
          rp,
          newSource || oldSource,
          '',
          initialTheme,
          null,
          webviewPanel.webview.cspSource,
          stylesUri,
          webviewUri,
          workspaceUri,
          hasDiff ? 'new' : 'none',
          hasDiff ? oldSource : '',
        );
        if (!unifiedDiffPanelState || unifiedDiffPanelState.panel !== webviewPanel) {
          unifiedDiffPanelState = { panel: webviewPanel, pairKey: rp };
          webviewPanel.onDidDispose(() => {
            if (unifiedDiffPanelState?.panel === webviewPanel) {
              unifiedDiffPanelState = null;
            }
          });
        }
      },
    })
  );

  context.subscriptions.push(
    vscode.window.tabGroups.onDidChangeTabs(() => {
      void maybeRouteActiveDiffToUnified();
    })
  );

  context.subscriptions.push(
    vscode.window.tabGroups.onDidChangeTabGroups(() => {
      void maybeRouteActiveDiffToUnified();
    })
  );

  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(() => {
      void maybeRouteActiveDiffToUnified();
    })
  );

  if (vscode.workspace.getConfiguration().get('docdb.autoMount', false)) {
    ensureMounted();
  } 
  else {
    unmountDocDbFolder();
  }

  // Auto-configure workspace when .doc folder is detected
  ensureDocFolderConfiguration();

  void maybeRouteActiveDiffToUnified();

  void openWelcomeDocumentOnFirstActivation(context);

  context.subscriptions.push(
    new vscode.Disposable(() => {
      resetRuntime();
    })
  );
}

function deactivate() {}

module.exports = {
  activate,
  deactivate,
};
