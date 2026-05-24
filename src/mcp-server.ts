#!/usr/bin/env node
/**
 * MCP Server for Document Platform
 * Exposes document operations to AI tools and agents over stdio.
 */

import path from 'node:path';
import readline from 'node:readline';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
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
import { computeSourceHash, mergeDocumentViewState, normalizeDocumentViewState } from './view-state.js';
import type { DocumentViewState } from './view-state.js';

const PROTOCOL_VERSION = '2024-11-05';

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | undefined | object | JsonValue[];

interface Runtime {
  workspaceRoot: string;
  viewStatePath: string;
  viewStateDocuments: Record<string, JsonValue>;
}

interface DocBlock {
  id?: string;
  className?: string;
  type?: string;
  text?: string;
  alt?: string;
  src?: string;
  language?: string;
  href?: string;
  media?: string;
  items?: Array<string | { text?: string; checked?: boolean }>;
}

interface DocRecord {
  id: number;
  title: string;
  relativePath: string;
  updatedAt: string;
  source?: string;
  blocks?: DocBlock[];
}

interface ViewerActionSpec {
  action?: string;
  index?: number;
  text?: string;
  settings?: Record<string, JsonValue>;
}

interface ToolArgs extends ViewerActionSpec {
  workspacePath?: string;
  query?: string;
  limit?: number;
  path?: string;
  id?: number | string;
  title?: string;
  summary?: string;
  tags?: JsonValue[];
  content?: string;
  size?: number;
  sessionId?: string;
  actions?: ViewerActionSpec[];
}

interface ViewerSessionSnapshot {
  document: DocRecord;
  activeBlockIndex: number;
  scrollTop: number;
}

interface ViewerHistoryEntry {
  action: string;
  at: string;
  snapshot: ViewerSessionSnapshot;
}

interface ViewerSession {
  sessionId: string;
  workspaceRoot: string;
  document: DocRecord;
  initialViewState: NormalizedViewState;
  activeViewState: NormalizedViewState;
  settingsDirty: boolean;
  activeBlockIndex: number;
  scrollTop: number;
  history: ViewerHistoryEntry[];
  future: ViewerHistoryEntry[];
  savedSource: string;
  isDirty: boolean;
}

interface JsonRpcRequest {
  jsonrpc?: string;
  method?: string;
  params?: {
    name?: string;
    arguments?: ToolArgs;
    workspacePath?: string;
    uri?: string;
  };
  id?: JsonValue;
}

interface CaptureResult {
  mimeType: string;
  base64: string;
  bytes: number;
  mode: string;
  engine: string;
  viewport?: JsonValue;
}

type NormalizedViewState = DocumentViewState;

const runtimeCache = new Map<string, Runtime>();
const viewerSessions = new Map<string, ViewerSession>();

function getViewStatePath(workspaceRoot: string): string {
  return path.resolve(workspaceRoot, '.doc/view-state.json');
}

async function readViewStateDocuments(viewStatePath: string): Promise<Record<string, JsonValue>> {
  try {
    const raw = await readFile(viewStatePath, 'utf8');
    const parsed = JSON.parse(raw) as { documents?: Record<string, JsonValue> };
    if (parsed && typeof parsed === 'object' && parsed.documents && typeof parsed.documents === 'object') {
      return parsed.documents;
    }
  } catch {
    // Missing or invalid view-state files default to empty in-memory state.
  }
  return {};
}

async function persistViewStateDocuments(runtime: Runtime): Promise<void> {
  await mkdir(path.dirname(runtime.viewStatePath), { recursive: true });
  const payload = {
    version: 1,
    documents: runtime.viewStateDocuments,
  };
  await writeFile(runtime.viewStatePath, safeJsonStringify(payload), 'utf8');
}

const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
  terminal: false,
});

function toSlug(value: JsonValue): string {
  return String(value || 'untitled')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'untitled';
}

function safeJsonStringify(value: JsonValue): string {
  return JSON.stringify(value, (_key, item) => {
    if (typeof item === 'bigint') {
      return item.toString();
    }
    return item;
  });
}

function resolveWorkspaceRoot(workspacePath: JsonValue): string {
  const candidate = String(workspacePath || process.env.DOC_WORKSPACE_ROOT || process.cwd());
  return path.resolve(candidate);
}

async function getRuntime(workspacePath: JsonValue): Promise<Runtime> {
  const workspaceRoot = resolveWorkspaceRoot(workspacePath);

  const cached = runtimeCache.get(workspaceRoot);
  if (cached) {
    return cached;
  }

  const viewStatePath = getViewStatePath(workspaceRoot);
  const viewStateDocuments = await readViewStateDocuments(viewStatePath);
  const runtime = { workspaceRoot, viewStatePath, viewStateDocuments };
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
            sourceHash: { type: 'string' },
            editBuffer: { type: 'string' },
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
    name: 'maintain-database',
    description: 'Run WAL checkpoint(TRUNCATE) + VACUUM maintenance for SQLite compaction.',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
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

function createViewerSessionId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function toBlockPreview(block: DocBlock | null | undefined): string {
  const type = String(block?.type || 'paragraph');
  if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') {
    const items = Array.isArray(block?.items) ? block.items : [];
    return items
      .map((item) => {
        if (typeof item === 'object' && item !== null) {
          return String(item.text || '').trim();
        }
        return String(item || '').trim();
      })
      .filter(Boolean)
      .join(' | ')
      .slice(0, 180);
  }

  if (type === 'image') {
    return String(block?.alt || block?.src || '').slice(0, 180);
  }

  return String(block?.text || '').slice(0, 180);
}

function buildViewerState(session: ViewerSession): Record<string, string | number | boolean | null | undefined | object> {
  const document = session.document;
  const viewState = normalizeDocumentViewState(session.activeViewState) as NormalizedViewState;
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
      html: renderDocumentViewHtml({
        title: document.title,
        relativePath: document.relativePath,
        source: document.source,
      }),
    },
  };
}

function findViewerSessionForDocument(workspaceRoot: string, documentId: string | number | boolean | null | undefined | object): ViewerSession | null {
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

function getActiveCaptureViewState(runtime: Runtime, document: DocRecord | null): NormalizedViewState {
  const liveSession = findViewerSessionForDocument(runtime.workspaceRoot, document?.id);
  if (liveSession?.activeViewState) {
    return normalizeDocumentViewState(liveSession.activeViewState) as NormalizedViewState;
  }

  const key = String(document?.relativePath || '');
  return normalizeDocumentViewState((runtime.viewStateDocuments && runtime.viewStateDocuments[key]) || {}) as NormalizedViewState;
}

function setBlockText(block: DocBlock, text: string | number | boolean | null | undefined | object): void {
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
          const checkedToken = match[1] ?? ' ';
          const itemText = match[2] ?? '';
          return { checked: checkedToken.toLowerCase() === 'x', text: itemText };
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

async function resolveDocumentFromArgs(runtime: Runtime, args: ToolArgs): Promise<DocRecord | null> {
  if (args.path) {
    return getDocumentByRelativePath(runtime.workspaceRoot, null, String(args.path));
  }

  if (Number.isFinite(Number(args.id))) {
    return getDocument(runtime.workspaceRoot, null, Number(args.id));
  }

  return null;
}

async function captureRendered({ document, size, viewState }: { document: DocRecord; size?: number; viewState?: NormalizedViewState }) {
  const captured = await captureDocumentViewPng(document, { size, viewState }) as CaptureResult;
  return { captured };
}

function cloneSessionDocument(document: DocRecord): DocRecord {
  return JSON.parse(safeJsonStringify(document || {})) as DocRecord;
}

function createInitialViewerViewState(runtime: Runtime, document: DocRecord): NormalizedViewState {
  const key = String(document?.relativePath || '');
  const fromStore = runtime.viewStateDocuments && runtime.viewStateDocuments[key];
  return normalizeDocumentViewState(fromStore || {}) as NormalizedViewState;
}

function resetSessionViewState(session: ViewerSession): void {
  session.activeViewState = normalizeDocumentViewState(session.initialViewState);
  session.settingsDirty = false;
}

function createSessionSnapshot(session: ViewerSession): ViewerSessionSnapshot {
  return {
    document: cloneSessionDocument(session.document),
    activeBlockIndex: session.activeBlockIndex,
    scrollTop: session.scrollTop,
  };
}

function pushSessionHistory(session: ViewerSession, action: string | number | boolean | null | undefined | object): void {
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

function syncSessionDirtyState(session: ViewerSession): void {
  if (!session || typeof session !== 'object') {
    return;
  }

  const currentSource = stringifyDocFile(session.document);
  session.document.source = currentSource;
  session.isDirty = currentSource !== String(session.savedSource || '');
  session.activeViewState = mergeDocumentViewState(session.activeViewState, {
    sourceHash: computeSourceHash(currentSource),
    editBuffer: session.isDirty ? currentSource : '',
  });
}

function undoSessionState(session: ViewerSession): boolean {
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

function redoSessionState(session: ViewerSession): boolean {
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

async function applyViewerAction(session: ViewerSession, actionSpec: ViewerActionSpec): Promise<{ closed: true; restoredViewSettings: NormalizedViewState } | void> {
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
    const block = blocks[index];
    if (!block) {
      throw new Error('index is required and must reference an existing block');
    }
    setBlockText(block, String(actionSpec?.text || ''));
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
      null,
      session.document.relativePath,
      stringifyDocFile(session.document)
    );
    session.document = saved;
    session.savedSource = stringifyDocFile(saved);
    session.isDirty = false;
    session.future = [];
    session.activeViewState = mergeDocumentViewState(session.activeViewState, {
      sourceHash: computeSourceHash(session.savedSource),
      editBuffer: '',
    });
    return;
  }

  if (action === 'set-view-settings') {
    const patch = actionSpec?.settings;
    if (!patch || typeof patch !== 'object') {
      throw new Error('settings object is required for set-view-settings');
    }

    session.activeViewState = mergeDocumentViewState(session.activeViewState, patch);
    session.settingsDirty = safeJsonStringify(session.activeViewState) !== safeJsonStringify(session.initialViewState);
    runtime.viewStateDocuments[session.document.relativePath] = session.activeViewState;
    await persistViewStateDocuments(runtime);
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

function toErrorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

async function handleTool(id: JsonValue, toolName: string, args: ToolArgs = {}) {
  try {
    let result;
    let content = null;

    switch (toolName) {
      case 'list-documents': {
        const runtime = await getRuntime(args.workspacePath);
        const query = String(args.query || '');
        const limit = Math.max(1, Number(args.limit || 50));
        result = (await listOrSearchDocuments(runtime.workspaceRoot, null, query)).slice(0, limit);
        break;
      }

      case 'get-document': {
        const runtime = await getRuntime(args.workspacePath);

        if (args.path) {
          result = await getDocumentByRelativePath(runtime.workspaceRoot, null, args.path);
        } else if (Number.isFinite(Number(args.id))) {
          result = await getDocument(runtime.workspaceRoot, null, Number(args.id));
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
        result = (await listOrSearchDocuments(runtime.workspaceRoot, null, query)).slice(0, limit);
        break;
      }

      case 'create-document': {
        const runtime = await getRuntime(args.workspacePath);
        const title = String(args.title || 'Untitled').trim() || 'Untitled';
        const docPath = String(args.path || `documents/${toSlug(title)}.dx`);

        await createDocument(runtime.workspaceRoot, null, {
          path: docPath,
          title,
          summary: args.summary,
          tags: Array.isArray(args.tags)
            ? args.tags.map((tag) => String(tag || '').trim()).filter(Boolean)
            : [],
        });

        if (String(args.content || '').trim()) {
          result = await saveDocumentSourceByRelativePath(runtime.workspaceRoot, null, docPath, String(args.content));
        } else {
          result = await getDocumentByRelativePath(runtime.workspaceRoot, null, docPath);
        }

        break;
      }

      case 'save-document': {
        const runtime = await getRuntime(args.workspacePath);
        result = await saveDocumentSourceByRelativePath(
          runtime.workspaceRoot,
          null,
          String(args.path || ''),
          String(args.content || '')
        );
        break;
      }

      case 'ingest-workspace': {
        const runtime = await getRuntime(args.workspacePath);
        result = await ingestWorkspace(runtime.workspaceRoot, null);
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
        const initialViewState = createInitialViewerViewState(runtime, document);
        const session = {
          sessionId,
          workspaceRoot: runtime.workspaceRoot,
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

      case 'maintain-database': {
        result = {
          status: 'removed',
          message: 'SQLite maintenance is no longer applicable. Active engine is bundle + dxlite.',
        };
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
          return sendError(id, -32602, toErrorMessage(err));
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
          const initialViewState = createInitialViewerViewState(runtime, document);
          session = {
            sessionId,
            workspaceRoot: runtime.workspaceRoot,
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
            return sendError(id, -32602, toErrorMessage(err));
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
    return sendError(id, -32603, `Tool execution failed: ${toErrorMessage(err)}`);
  }
}

async function handleRequest(message: JsonRpcRequest) {
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
      const name = String(params?.name || '');
      const callArgs = params?.arguments || {};
      return handleTool(id, name, callArgs);
    }

    if (method === 'resources/list') {
      const runtime = await getRuntime(params?.workspacePath);
      const docs = await listOrSearchDocuments(runtime.workspaceRoot, null, '');
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
      const doc = await getDocumentByRelativePath(runtime.workspaceRoot, null, relativePath);

      if (!doc) {
        return sendError(id, -32602, 'Document not found');
      }

      if (uri.startsWith('docview:///')) {
        const renderDocument = {
          title: doc.title,
          relativePath: doc.relativePath,
          source: doc.source,
        };

        return sendResponse(id, {
          contents: [
            {
              uri,
              mimeType: 'text/html',
              text: renderDocumentViewHtml(renderDocument),
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
    return sendError(message.id, -32603, `Internal error: ${toErrorMessage(err)}`);
  }
}

function sendResponse(id: JsonValue, result: JsonValue): void {
  console.log(safeJsonStringify({ jsonrpc: '2.0', id, result }));
}

function sendError(id: JsonValue, code: number, message: string): void {
  console.log(safeJsonStringify({ jsonrpc: '2.0', id, error: { code, message } }));
}

rl.on('line', async (line) => {
  try {
    const message = JSON.parse(line);
    await handleRequest(message);
  } catch (err) {
    console.error('Parse error:', toErrorMessage(err));
    sendError(null, -32700, 'Parse error');
  }
});

rl.on('close', () => {
  process.exit(0);
});

console.error('[MCP Server] Document Platform MCP Server started');
