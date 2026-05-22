const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { mkdir, readFile, writeFile } = require('node:fs/promises');
const vscode = require('vscode');

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

  const { db, dbModule } = await getDocRuntime();
  const absolutePath = path.resolve(workspaceRoot, rel);
  const document = dbModule.getDocumentByPath(db, workspaceRoot, absolutePath);

  if (!document) {
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
    sourceText: String(snapshot.sourceText || ''),
  };

  dbModule.saveDocumentViewState(db, document.id, normalizedSnapshot);

  const current = { version: 1, documents: {} };
  const rows = db.prepare(`
    SELECT path, view_state_json, updated_at
    FROM documents
    WHERE view_state_json IS NOT NULL
    ORDER BY updated_at DESC
  `).all();

  for (const row of rows) {
    try {
      const entry = JSON.parse(row.view_state_json);
      const key = normalizeDocPath(path.relative(workspaceRoot, row.path));

      if (!key || !entry || typeof entry !== 'object') {
        continue;
      }

      current.documents[key] = {
        ...entry,
        updatedAt: row.updated_at,
      };
    } catch {
      continue;
    }
  }

  await writeFile(viewStatePath, `${JSON.stringify(current, null, 2)}\n`, 'utf8');
}

function resetRuntime() {
  if (!runtimePromise) {
    runtimeRoot = null;
    return;
  }

  runtimePromise
    .then(({ db }) => {
      try {
        db.close();
      } catch {
      }
    })
    .catch(() => {
    });

  runtimePromise = null;
  runtimeRoot = null;
}

async function getDocRuntime() {
  const workspaceRoot = getWorkspaceRoot();

  if (!workspaceRoot) {
    throw new Error('DOC DB requires an open workspace folder.');
  }

  if (runtimePromise && runtimeRoot === workspaceRoot) {
    return runtimePromise;
  }

  runtimeRoot = workspaceRoot;
  runtimePromise = (async () => {
    const srcDir = path.join(workspaceRoot, 'src');
    const [dbModule, serviceModule, globalPathModule] = await Promise.all([
      import(pathToFileURL(path.join(srcDir, 'database.js')).href),
      import(pathToFileURL(path.join(srcDir, 'doc-service.js')).href),
      import(pathToFileURL(path.join(srcDir, 'global-db-path.js')).href),
    ]);

    const dbPath = globalPathModule.resolveDocDbPath();
    await mkdir(path.dirname(dbPath), { recursive: true });

    const db = dbModule.createDatabase(dbPath);
    dbModule.migrateLegacyWorkspace(db, workspaceRoot, path.join(workspaceRoot, 'data', 'doc-index.sqlite'));

    // Avoid re-ingesting the entire workspace on every extension startup.
    if (dbModule.listDocuments(db, workspaceRoot).length === 0) {
      await serviceModule.ingestWorkspace(workspaceRoot, db);
    }

    return {
      workspaceRoot,
      db,
      dbModule,
      serviceModule,
    };
  })();

  return runtimePromise;
}

async function readUiConfig() {
  const { db, dbModule } = await getDocRuntime();
  const configured = String(dbModule.getUserConfigValue(db, 'preferred_theme', 'auto') || 'auto');
  return { theme: THEME_OPTIONS.has(configured) ? configured : 'auto' };
}

async function writePreferredTheme(theme) {
  const { db, dbModule } = await getDocRuntime();
  const normalizedTheme = THEME_OPTIONS.has(String(theme || '').toLowerCase()) ? String(theme).toLowerCase() : 'auto';
  dbModule.setUserConfigValue(db, 'preferred_theme', normalizedTheme);
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
  const { workspaceRoot, db, serviceModule } = await getDocRuntime();
  const document = await serviceModule.getDocumentByRelativePath(workspaceRoot, db, normalizedPath);

  if (!document) {
    throw new Error(`Document not found: ${normalizedPath}`);
  }

  return String(document.source || '');
}

async function writeVirtualDocument(virtualPath, sourceText) {
  const normalizedPath = normalizeVirtualPath(virtualPath);
  const { workspaceRoot, db, serviceModule } = await getDocRuntime();
  const document = await serviceModule.saveDocumentSourceByRelativePath(workspaceRoot, db, normalizedPath, String(sourceText || ''));
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
  const { workspaceRoot, db, serviceModule } = await getDocRuntime();
  const documents = await serviceModule.listOrSearchDocuments(workspaceRoot, db, '');

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
  if (uri && uri.scheme === 'docdb') {
    const virtualPath = String(uri.path || '').replace(/^\/+/, '');
    if (!virtualPath || !virtualPath.endsWith('.dx')) {
      return null;
    }
    return virtualPath;
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

function renderEditorHtml(relativePath, sourceText, errorText = '', initialTheme = 'auto', initialAppearance = null, cspSource = "'none'", stylesUri = '', webviewUri = '', workspaceUri = '') {
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
      return `<pre>${escapeHtml(block.text || '')}</pre>`;
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
      <div id="doc-init" data-doc-path="${escapeHtml(relativePath || 'unknown.dx')}" data-doc-error="${escapeHtml(errorText || '')}" data-initial-theme="${escapeHtml(initialTheme || 'auto')}" data-initial-paper="${escapeHtml(appearance.paper)}" data-initial-density="${escapeHtml(appearance.density)}" data-initial-scale="${escapeHtml(String(appearance.scale))}" data-workspace-uri="${escapeHtml(workspaceUri || '')}" hidden></div>
      <textarea id="doc-init-source" hidden>${escapeHtml(sourceText || '')}</textarea>
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
    const stylesUri = webviewPanel.webview.asWebviewUri(vscode.Uri.joinPath(this._extensionUri, 'media', 'styles.css'));
    const webviewUri = webviewPanel.webview.asWebviewUri(vscode.Uri.joinPath(this._extensionUri, 'media', 'webview-main.js'));

    const workspaceRoot = getWorkspaceRoot();
    const workspaceUri = workspaceRoot
      ? webviewPanel.webview.asWebviewUri(vscode.Uri.file(workspaceRoot)).toString()
      : '';

    webviewPanel.webview.options = {
      enableScripts: true,
      localResourceRoots: [
        vscode.Uri.joinPath(this._extensionUri, 'media'),
        ...(workspaceRoot ? [vscode.Uri.file(workspaceRoot)] : []),
      ],
    };

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

    let sourceText = '';
    let loadError = '';

    try {
      sourceText = await readVirtualDocument(relativePath);
    } catch (error) {
      loadError = error instanceof Error ? error.message : 'Failed to load document from SQLite.';

      try {
        sourceText = document.getText();
      } catch {
        sourceText = '';
      }
    }

    webviewPanel.webview.html = renderEditorHtml(relativePath, sourceText, loadError, initialTheme, initialAppearance, webviewPanel.webview.cspSource, stylesUri, webviewUri, workspaceUri);

    let sourcePushTimer = null;
    let suppressedEchoSource = null;
    let suppressNextChangePush = false;
    let dirtySyncTimer = null;
    let pendingDirtySource = null;

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
        const latestSource = await readVirtualDocument(relativePath);

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
        await replaceWorkingCopyText(stubText ?? saveText);

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
  const provider = new DocDbFileSystemProvider();
  const customEditor = new DocDbCustomEditorProvider(context.extensionUri);

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

  if (vscode.workspace.getConfiguration().get('docdb.autoMount', false)) {
    ensureMounted();
  } 
  else {
    unmountDocDbFolder();
  }

  // Auto-configure workspace when .doc folder is detected
  ensureDocFolderConfiguration();

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
