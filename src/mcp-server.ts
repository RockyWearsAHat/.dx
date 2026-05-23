#!/usr/bin/env node
/**
 * MCP Server for Document Platform
 * Exposes document operations to AI tools and agents over stdio.
 */

import path from 'node:path';
import readline from 'node:readline';
import { mkdir } from 'node:fs/promises';
import {
  createDatabase,
  migrateLegacyWorkspace,
  saveDocumentViewState,
} from './database.js';
import {
  createDocument,
  getDocument,
  getDocumentByRelativePath,
  ingestWorkspace,
  listOrSearchDocuments,
  saveDocumentSourceByRelativePath,
} from './doc-service.js';
import { renderDocumentViewHtml } from './doc-view.js';
import { captureDocumentViewPng } from './doc-view-capture.js';
import { stringifyDocFile } from './doc-format.js';
import { resolveDocDbPath } from './global-db-path.js';
import { mergeDocumentViewState, normalizeDocumentViewState, readDocumentViewState } from './view-state.js';

const PROTOCOL_VERSION = '2024-11-05';
const runtimeCache = new Map();
const viewerSessions = new Map();

const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
  terminal: false,
});

function toSlug(value) {
  return String(value || 'untitled')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'untitled';
}

function safeJsonStringify(value) {
  return JSON.stringify(value, (_key, item) => {
    if (typeof item === 'bigint') {
      return item.toString();
    }
    return item;
  });
}

function resolveWorkspaceRoot(workspacePath) {
  const candidate = String(workspacePath || process.env.DOC_WORKSPACE_ROOT || process.cwd());
  return path.resolve(candidate);
}

async function getRuntime(workspacePath) {
  const workspaceRoot = resolveWorkspaceRoot(workspacePath);

  if (runtimeCache.has(workspaceRoot)) {
    return runtimeCache.get(workspaceRoot);
  }

  const dbPath = resolveDocDbPath();
  await mkdir(path.dirname(dbPath), { recursive: true });

  const db = createDatabase(dbPath);
  migrateLegacyWorkspace(db, workspaceRoot, path.join(workspaceRoot, 'data', 'doc-index.sqlite'));

  const runtime = { workspaceRoot, db };
  runtimeCache.set(workspaceRoot, runtime);
  return runtime;
}

const TOOLS = [
  {
    name: 'list-documents',
    description: 'List documents in the workspace database',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        query: { type: 'string', description: 'Optional search query' },
        limit: { type: 'number', description: 'Maximum results (default 50)' },
      },
      required: [],
    },
  },
  {
    name: 'get-document',
    description: 'Get a document by relative path or ID',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path' },
        id: { type: 'number', description: 'Document ID' },
      },
      required: [],
      oneOf: [
        { required: ['path'] },
        { required: ['id'] },
      ],
    },
  },
  {
    name: 'search-documents',
    description: 'Search documents by title/content/tags',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        query: { type: 'string', description: 'Search query' },
        limit: { type: 'number', description: 'Maximum results (default 20)' },
      },
      required: ['query'],
    },
  },
  {
    name: 'create-document',
    description: 'Create a new document and optionally seed source content',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path (default under documents/)' },
        title: { type: 'string', description: 'Document title' },
        summary: { type: 'string', description: 'Document summary' },
        tags: { type: 'array', items: { type: 'string' }, description: 'Document tags' },
        content: { type: 'string', description: 'Optional DOC source text to save after create' },
      },
      required: [],
    },
  },
  {
    name: 'save-document',
    description: 'Save full source text for a document path',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path' },
        content: { type: 'string', description: 'Full document source text' },
      },
      required: ['path', 'content'],
    },
  },
  {
    name: 'open-document-viewer',
    description: 'Open a built-in interactive viewer session for a document and return current view state.',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path' },
        id: { type: 'number', description: 'Document ID' },
      },
      required: [],
      oneOf: [
        { required: ['path'] },
        { required: ['id'] },
      ],
    },
  },
  {
    name: 'interact-document-viewer',
    description: 'Interact with an active document viewer session: inspect, click, scroll, edit block text, append paragraph, and save.',
    inputSchema: {
      type: 'object',
      properties: {
        sessionId: { type: 'string', description: 'Viewer session id returned by open-document-viewer' },
        action: {
          type: 'string',
          enum: ['inspect', 'click-block', 'scroll-to', 'set-block-text', 'append-paragraph', 'save', 'set-view-settings', 'reset-view-settings', 'close', 'undo-state', 'redo-state', 'undo-document', 'redo-document'],
          description: 'Interaction action to apply',
        },
        index: { type: 'number', description: 'Block index for click/set/scroll actions' },
        text: { type: 'string', description: 'Text payload for set-block-text or append-paragraph' },
        settings: {
          type: 'object',
          description: 'Optional viewer settings patch for set-view-settings',
          properties: {
            theme: { type: 'string' },
            resolvedTheme: { type: 'string' },
            appearance: { type: 'object' },
            viewport: { type: 'object' },
            effectiveCss: { type: 'string' },
            sourceText: { type: 'string' },
          },
        },
      },
      required: ['sessionId', 'action'],
    },
  },
  {
    name: 'capture-document-view',
    description: 'Capture a real PNG screenshot for a .dx document rendered in the simple browser workflow.',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path' },
        id: { type: 'number', description: 'Document ID' },
        size: { type: 'number', description: 'Capture width hint in pixels (default 1492)' },
      },
      required: [],
      oneOf: [
        { required: ['path'] },
        { required: ['id'] },
      ],
    },
  },
  {
    name: 'use-document-viewer',
    description: 'Single-call document viewer operation: open/resume a session, apply interactions, and return updated state with rendered screenshot.',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path' },
        id: { type: 'number', description: 'Document ID' },
        sessionId: { type: 'string', description: 'Existing viewer session id. If omitted, a new session opens from path or id.' },
        size: { type: 'number', description: 'Screenshot width hint in pixels (default 1492)' },
        actions: {
          type: 'array',
          description: 'Optional list of actions to run in order. If omitted, defaults to inspect.',
          items: {
            type: 'object',
            properties: {
              action: {
                type: 'string',
                enum: ['inspect', 'click-block', 'scroll-to', 'set-block-text', 'append-paragraph', 'save', 'set-view-settings', 'reset-view-settings', 'close', 'undo-state', 'redo-state', 'undo-document', 'redo-document'],
              },
              index: { type: 'number' },
              text: { type: 'string' },
              settings: { type: 'object' },
            },
            required: ['action'],
          },
        },
      },
      required: [],
    },
  },
  {
    name: 'ingest-workspace',
    description: 'Scan workspace .dx files and ingest them into SQLite',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
      },
      required: [],
    },
  },
];

function createViewerSessionId() {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function toBlockPreview(block) {
  const type = String(block?.type || 'paragraph');
  if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') {
    const items = Array.isArray(block?.items) ? block.items : [];
    return items
      .map((item) => String(item?.text || item || '').trim())
      .filter(Boolean)
      .join(' | ')
      .slice(0, 180);
  }

  if (type === 'image') {
    return String(block?.alt || block?.src || '').slice(0, 180);
  }

  return String(block?.text || '').slice(0, 180);
}

function buildViewerState(session) {
  const document = session.document;
  const viewState = normalizeDocumentViewState(session.activeViewState);
  const blocks = Array.isArray(document?.blocks) ? document.blocks : [];
  const historyDepth = Array.isArray(session.history) ? session.history.length : 0;
  const redoDepth = Array.isArray(session.future) ? session.future.length : 0;

  return {
    sessionId: session.sessionId,
    document: {
      id: document.id,
      title: document.title,
      relativePath: document.relativePath,
      updatedAt: document.updatedAt,
    },
    view: {
      uri: `docview:///${document.relativePath}`,
      activeBlockIndex: session.activeBlockIndex,
      scrollTop: session.scrollTop,
      saveState: session.isDirty ? 'dirty' : 'clean',
      historyDepth,
      redoDepth,
      blockCount: blocks.length,
      blocks: blocks.map((block, index) => ({
        index,
        id: block.id,
        type: block.type,
        preview: toBlockPreview(block),
      })),
      viewSettings: viewState,
      html: renderDocumentViewHtml(document, {
        theme: viewState.theme,
        resolvedTheme: viewState.resolvedTheme,
        appearance: viewState.appearance,
        effectiveCss: viewState.effectiveCss,
      }),
    },
  };
}

function findViewerSessionForDocument(workspaceRoot, documentId) {
  const targetId = Number(documentId);
  if (!Number.isFinite(targetId)) {
    return null;
  }

  for (const session of viewerSessions.values()) {
    if (!session || session.workspaceRoot !== workspaceRoot) {
      continue;
    }

    if (Number(session?.document?.id) === targetId) {
      return session;
    }
  }

  return null;
}

function getActiveCaptureViewState(runtime, document) {
  const liveSession = findViewerSessionForDocument(runtime.workspaceRoot, document?.id);
  if (liveSession?.activeViewState) {
    return normalizeDocumentViewState(liveSession.activeViewState);
  }

  return readDocumentViewState(runtime.db, document?.id);
}

function setBlockText(block, text) {
  const value = String(text || '');
  const type = String(block?.type || 'paragraph');

  if (type === 'bulleted-list' || type === 'numbered-list') {
    block.items = value
      .split('\n')
      .map((line) => line.replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim())
      .filter(Boolean);
    return;
  }

  if (type === 'checklist') {
    block.items = value
      .split('\n')
      .map((line) => {
        const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(line.trim());
        if (match) {
          return { checked: match[1].toLowerCase() === 'x', text: match[2] };
        }
        return { checked: false, text: line.trim() };
      })
      .filter((item) => item.text);
    return;
  }

  if (type === 'image') {
    block.alt = value;
    return;
  }

  block.text = value;
}

async function resolveDocumentFromArgs(runtime, args) {
  if (args.path) {
    return getDocumentByRelativePath(runtime.workspaceRoot, runtime.db, String(args.path));
  }

  if (Number.isFinite(Number(args.id))) {
    return getDocument(runtime.workspaceRoot, runtime.db, Number(args.id));
  }

  return null;
}

async function captureRendered({ document, size, viewState }) {
  const captured = await captureDocumentViewPng(document, { size, viewState });
  return { captured };
}

function cloneSessionDocument(document) {
  return JSON.parse(safeJsonStringify(document || {}));
}

function createInitialViewerViewState(db, document) {
  const fromDb = readDocumentViewState(db, document?.id);
  return normalizeDocumentViewState(fromDb || {});
}

function resetSessionViewState(session) {
  session.activeViewState = normalizeDocumentViewState(session.initialViewState);
  session.settingsDirty = false;
}

function createSessionSnapshot(session) {
  return {
    document: cloneSessionDocument(session.document),
    activeBlockIndex: session.activeBlockIndex,
    scrollTop: session.scrollTop,
  };
}

function pushSessionHistory(session, action) {
  if (!session || typeof session !== 'object') {
    return;
  }

  if (!Array.isArray(session.history)) {
    session.history = [];
  }

  if (!Array.isArray(session.future)) {
    session.future = [];
  }

  session.future = [];

  session.history.push({
    action: String(action || '').toLowerCase(),
    at: new Date().toISOString(),
    snapshot: createSessionSnapshot(session),
  });

  if (session.history.length > 32) {
    session.history = session.history.slice(-32);
  }
}

function syncSessionDirtyState(session) {
  if (!session || typeof session !== 'object') {
    return;
  }

  const currentSource = stringifyDocFile(session.document);
  session.document.source = currentSource;
  session.isDirty = currentSource !== String(session.savedSource || '');
}

function undoSessionState(session) {
  if (!session || !Array.isArray(session.history) || session.history.length === 0) {
    return false;
  }

  if (!Array.isArray(session.future)) {
    session.future = [];
  }

  session.future.push({
    action: 'redo-state',
    at: new Date().toISOString(),
    snapshot: createSessionSnapshot(session),
  });

  const previous = session.history.pop();
  if (!previous || !previous.snapshot) {
    return false;
  }

  session.document = cloneSessionDocument(previous.snapshot.document);
  session.activeBlockIndex = Number.isInteger(previous.snapshot.activeBlockIndex)
    ? previous.snapshot.activeBlockIndex
    : 0;
  session.scrollTop = Number.isFinite(previous.snapshot.scrollTop)
    ? previous.snapshot.scrollTop
    : 0;
  syncSessionDirtyState(session);
  return true;
}

function redoSessionState(session) {
  if (!session || !Array.isArray(session.future) || session.future.length === 0) {
    return false;
  }

  if (!Array.isArray(session.history)) {
    session.history = [];
  }

  session.history.push({
    action: 'undo-state',
    at: new Date().toISOString(),
    snapshot: createSessionSnapshot(session),
  });

  const next = session.future.pop();
  if (!next || !next.snapshot) {
    return false;
  }

  session.document = cloneSessionDocument(next.snapshot.document);
  session.activeBlockIndex = Number.isInteger(next.snapshot.activeBlockIndex)
    ? next.snapshot.activeBlockIndex
    : 0;
  session.scrollTop = Number.isFinite(next.snapshot.scrollTop)
    ? next.snapshot.scrollTop
    : 0;
  syncSessionDirtyState(session);
  return true;
}

async function applyViewerAction(session, actionSpec) {
  const action = String(actionSpec?.action || '').toLowerCase();
  const blocks = Array.isArray(session.document?.blocks) ? session.document.blocks : [];
  const index = Number(actionSpec?.index);

  if (action === 'inspect') {
    return;
  }

  if (action === 'undo-state' || action === 'undo-document') {
    if (!undoSessionState(session)) {
      throw new Error('No viewer state transition available to undo');
    }
    return;
  }

  if (action === 'redo-state' || action === 'redo-document') {
    if (!redoSessionState(session)) {
      throw new Error('No viewer state transition available to redo');
    }
    return;
  }

  if (action === 'click-block') {
    if (!Number.isInteger(index) || index < 0 || index >= blocks.length) {
      throw new Error('index is required and must reference an existing block');
    }
    pushSessionHistory(session, action);
    session.activeBlockIndex = index;
    return;
  }

  if (action === 'scroll-to') {
    if (!Number.isFinite(index)) {
      throw new Error('index is required for scroll-to');
    }
    pushSessionHistory(session, action);
    session.scrollTop = Math.max(0, Math.trunc(index));
    return;
  }

  if (action === 'set-block-text') {
    if (!Number.isInteger(index) || index < 0 || index >= blocks.length) {
      throw new Error('index is required and must reference an existing block');
    }

    pushSessionHistory(session, action);
    setBlockText(blocks[index], String(actionSpec?.text || ''));
    syncSessionDirtyState(session);
    return;
  }

  if (action === 'append-paragraph') {
    const text = String(actionSpec?.text || '').trim();
    if (!text) {
      throw new Error('text is required for append-paragraph');
    }

    pushSessionHistory(session, action);
    blocks.push({
      id: `paragraph-${blocks.length + 1}`,
      className: '',
      type: 'paragraph',
      text,
    });
    syncSessionDirtyState(session);
    return;
  }

  if (action === 'save') {
    const runtime = await getRuntime(session.workspaceRoot);
    const saved = await saveDocumentSourceByRelativePath(
      runtime.workspaceRoot,
      runtime.db,
      session.document.relativePath,
      stringifyDocFile(session.document)
    );
    session.document = saved;
    session.savedSource = stringifyDocFile(saved);
    session.isDirty = false;
    session.future = [];
    return;
  }

  if (action === 'set-view-settings') {
    const patch = actionSpec?.settings;
    if (!patch || typeof patch !== 'object') {
      throw new Error('settings object is required for set-view-settings');
    }

    session.activeViewState = mergeDocumentViewState(session.activeViewState, patch);
    session.settingsDirty = safeJsonStringify(session.activeViewState) !== safeJsonStringify(session.initialViewState);
    return;
  }

  if (action === 'reset-view-settings') {
    resetSessionViewState(session);
    return;
  }

  if (action === 'close') {
    if (session.settingsDirty) {
      resetSessionViewState(session);
    }

    return {
      closed: true,
      restoredViewSettings: normalizeDocumentViewState(session.initialViewState),
    };
  }

  throw new Error(`Unknown interaction action: ${action}`);
}

async function handleTool(id, toolName, args = {}) {
  try {
    let result;
    let content = null;

    switch (toolName) {
      case 'list-documents': {
        const runtime = await getRuntime(args.workspacePath);
        const query = String(args.query || '');
        const limit = Math.max(1, Number(args.limit || 50));
        result = (await listOrSearchDocuments(runtime.workspaceRoot, runtime.db, query)).slice(0, limit);
        break;
      }

      case 'get-document': {
        const runtime = await getRuntime(args.workspacePath);

        if (args.path) {
          result = await getDocumentByRelativePath(runtime.workspaceRoot, runtime.db, args.path);
        } else if (Number.isFinite(Number(args.id))) {
          result = await getDocument(runtime.workspaceRoot, runtime.db, Number(args.id));
        } else {
          return sendError(id, -32602, 'Either path or id is required');
        }

        if (!result) {
          return sendError(id, -32602, 'Document not found');
        }

        break;
      }

      case 'search-documents': {
        const runtime = await getRuntime(args.workspacePath);
        const query = String(args.query || '').trim();

        if (!query) {
          return sendError(id, -32602, 'query is required');
        }

        const limit = Math.max(1, Number(args.limit || 20));
        result = (await listOrSearchDocuments(runtime.workspaceRoot, runtime.db, query)).slice(0, limit);
        break;
      }

      case 'create-document': {
        const runtime = await getRuntime(args.workspacePath);
        const title = String(args.title || 'Untitled').trim() || 'Untitled';
        const docPath = String(args.path || `documents/${toSlug(title)}.dx`);

        await createDocument(runtime.workspaceRoot, runtime.db, {
          path: docPath,
          title,
          summary: args.summary,
          tags: Array.isArray(args.tags) ? args.tags : [],
        });

        if (String(args.content || '').trim()) {
          result = await saveDocumentSourceByRelativePath(runtime.workspaceRoot, runtime.db, docPath, String(args.content));
        } else {
          result = await getDocumentByRelativePath(runtime.workspaceRoot, runtime.db, docPath);
        }

        break;
      }

      case 'save-document': {
        const runtime = await getRuntime(args.workspacePath);
        result = await saveDocumentSourceByRelativePath(
          runtime.workspaceRoot,
          runtime.db,
          String(args.path || ''),
          String(args.content || '')
        );
        break;
      }

      case 'ingest-workspace': {
        const runtime = await getRuntime(args.workspacePath);
        result = await ingestWorkspace(runtime.workspaceRoot, runtime.db);
        break;
      }

      case 'open-document-viewer': {
        const runtime = await getRuntime(args.workspacePath);
        const document = await resolveDocumentFromArgs(runtime, args);

        if (!document) {
          return sendError(id, -32602, 'Document not found (provide valid path or id)');
        }

        const sessionId = createViewerSessionId();
        const savedSource = stringifyDocFile(document);
        const initialViewState = createInitialViewerViewState(runtime.db, document);
        const session = {
          sessionId,
          workspaceRoot: runtime.workspaceRoot,
          db: runtime.db,
          document,
          initialViewState,
          activeViewState: normalizeDocumentViewState(initialViewState),
          settingsDirty: false,
          activeBlockIndex: 0,
          scrollTop: 0,
          history: [],
          future: [],
          savedSource,
          isDirty: false,
        };

        viewerSessions.set(sessionId, session);
        result = buildViewerState(session);
        break;
      }

      case 'capture-document-view': {
        const runtime = await getRuntime(args.workspacePath);
        const document = await resolveDocumentFromArgs(runtime, args);

        if (!document) {
          return sendError(id, -32602, 'Document not found (provide valid path or id)');
        }

        const viewState = getActiveCaptureViewState(runtime, document);
        const { captured } = await captureRendered({ document, size: args.size, viewState });

        result = {
          document: {
            id: document.id,
            title: document.title,
            relativePath: document.relativePath,
            updatedAt: document.updatedAt,
          },
          capture: {
            mimeType: captured.mimeType,
            bytes: captured.bytes,
            mode: captured.mode,
            engine: captured.engine,
            viewport: captured.viewport,
          },
        };

        content = [
          { type: 'text', text: safeJsonStringify(result) },
          { type: 'image', mimeType: captured.mimeType, data: captured.base64 },
        ];
        break;
      }

      case 'interact-document-viewer': {
        const sessionId = String(args.sessionId || '');
        const action = String(args.action || '').toLowerCase();
        const session = viewerSessions.get(sessionId);

        if (!session) {
          return sendError(id, -32602, 'Viewer session not found. Call open-document-viewer first.');
        }

        try {
          const actionResult = await applyViewerAction(session, {
            action,
            index: args.index,
            text: args.text,
            settings: args.settings,
          });

          if (actionResult?.closed) {
            viewerSessions.delete(sessionId);
            result = {
              sessionId,
              closed: true,
              restoredViewSettings: actionResult.restoredViewSettings,
            };
            break;
          }
        } catch (err) {
          return sendError(id, -32602, err.message);
        }

        result = buildViewerState(session);
        break;
      }

      case 'use-document-viewer': {
        const runtime = await getRuntime(args.workspacePath);
        let session = null;

        if (args.sessionId) {
          session = viewerSessions.get(String(args.sessionId || ''));
          if (!session) {
            return sendError(id, -32602, 'Viewer session not found for sessionId');
          }
        } else {
          const document = await resolveDocumentFromArgs(runtime, args);
          if (!document) {
            return sendError(id, -32602, 'Provide path/id to open a new viewer session');
          }

          const sessionId = createViewerSessionId();
          const savedSource = stringifyDocFile(document);
          const initialViewState = createInitialViewerViewState(runtime.db, document);
          session = {
            sessionId,
            workspaceRoot: runtime.workspaceRoot,
            db: runtime.db,
            document,
            initialViewState,
            activeViewState: normalizeDocumentViewState(initialViewState),
            settingsDirty: false,
            activeBlockIndex: 0,
            scrollTop: 0,
            history: [],
            future: [],
            savedSource,
            isDirty: false,
          };
          viewerSessions.set(sessionId, session);
        }

        const actions = Array.isArray(args.actions) && args.actions.length > 0
          ? args.actions
          : [{ action: 'inspect' }];

        for (const actionSpec of actions) {
          try {
            const actionResult = await applyViewerAction(session, actionSpec);
            if (actionResult?.closed) {
              viewerSessions.delete(session.sessionId);
              result = {
                sessionId: session.sessionId,
                closed: true,
                restoredViewSettings: actionResult.restoredViewSettings,
              };
              content = [{ type: 'text', text: safeJsonStringify(result) }];
              return sendResponse(id, { content });
            }
          } catch (err) {
            return sendError(id, -32602, err.message);
          }
        }

        const state = buildViewerState(session);
        const { captured } = await captureRendered({
          document: session.document,
          size: args.size,
          viewState: normalizeDocumentViewState(session.activeViewState),
        });
        result = {
          ...state,
          capture: {
            mimeType: captured.mimeType,
            bytes: captured.bytes,
            mode: captured.mode,
            engine: captured.engine,
            viewport: captured.viewport,
          },
        };
        content = [
          { type: 'text', text: safeJsonStringify(result) },
          { type: 'image', mimeType: captured.mimeType, data: captured.base64 },
        ];
        break;
      }

      default:
        return sendError(id, -32601, `Unknown tool: ${toolName}`);
    }

    return sendResponse(id, {
      content: content || [{ type: 'text', text: safeJsonStringify(result) }],
    });
  } catch (err) {
    console.error(`Tool ${toolName} error:`, err);
    return sendError(id, -32603, `Tool execution failed: ${err.message}`);
  }
}

async function handleRequest(message) {
  try {
    if (!message.jsonrpc || message.jsonrpc !== '2.0') {
      return sendError(message.id, -32600, 'Invalid Request');
    }

    const { method, params, id } = message;

    if (method === 'initialize') {
      return sendResponse(id, {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: {
          tools: {},
          resources: {},
        },
        serverInfo: {
          name: 'doc-platform-mcp',
          version: '0.1.0',
        },
      });
    }

    if (method === 'tools/list') {
      return sendResponse(id, { tools: TOOLS });
    }

    if (method === 'tools/call') {
      const name = params?.name;
      const callArgs = params?.arguments || {};
      return handleTool(id, name, callArgs);
    }

    if (method === 'resources/list') {
      const runtime = await getRuntime(params?.workspacePath);
      const docs = await listOrSearchDocuments(runtime.workspaceRoot, runtime.db, '');
      const resources = docs.flatMap((doc) => ([
        {
          uri: `doc:///${doc.relativePath}`,
          name: `${doc.title || doc.relativePath} (source)`,
          mimeType: 'text/plain',
        },
        {
          uri: `docview:///${doc.relativePath}`,
          name: `${doc.title || doc.relativePath} (view)`,
          mimeType: 'text/html',
        },
      ]));
      return sendResponse(id, { resources });
    }

    if (method === 'resources/read') {
      const uri = String(params?.uri || '');
      const runtime = await getRuntime(params?.workspacePath);
      const relativePath = uri
        .replace(/^doc:\/\/\//, '')
        .replace(/^docview:\/\/\//, '');
      const doc = await getDocumentByRelativePath(runtime.workspaceRoot, runtime.db, relativePath);

      if (!doc) {
        return sendError(id, -32602, 'Document not found');
      }

      if (uri.startsWith('docview:///')) {
        return sendResponse(id, {
          contents: [
            {
              uri,
              mimeType: 'text/html',
              text: renderDocumentViewHtml(doc),
            },
          ],
        });
      }

      return sendResponse(id, {
        contents: [
          {
            uri,
            mimeType: 'text/plain',
            text: doc.source,
          },
        ],
      });
    }

    return sendError(id, -32601, 'Method not found');
  } catch (err) {
    console.error('Request handling error:', err);
    return sendError(message.id, -32603, `Internal error: ${err.message}`);
  }
}

function sendResponse(id, result) {
  console.log(safeJsonStringify({ jsonrpc: '2.0', id, result }));
}

function sendError(id, code, message) {
  console.log(safeJsonStringify({ jsonrpc: '2.0', id, error: { code, message } }));
}

rl.on('line', async (line) => {
  try {
    const message = JSON.parse(line);
    await handleRequest(message);
  } catch (err) {
    console.error('Parse error:', err.message);
    sendError(null, -32700, 'Parse error');
  }
});

rl.on('close', () => {
  process.exit(0);
});

console.error('[MCP Server] Document Platform MCP Server started');
