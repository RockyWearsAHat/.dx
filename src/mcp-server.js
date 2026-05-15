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
  listDocuments,
} from './database.js';
import {
  createDocument,
  getDocument,
  getDocumentByRelativePath,
  ingestWorkspace,
  listOrSearchDocuments,
  saveDocumentSourceByRelativePath,
} from './doc-service.js';
import { buildVisualModel } from './doc-visual.js';
import { resolveDocDbPath } from './global-db-path.js';

const PROTOCOL_VERSION = '2024-11-05';
const runtimeCache = new Map();

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
  {
    name: 'get-document-visual',
    description: 'Get the visual surface model for a document: block layout, design quality score, issues, recommendations, headings, media, and estimated geometry. Use this before editing a document to understand its current visual structure and design state.',
    inputSchema: {
      type: 'object',
      properties: {
        workspacePath: { type: 'string', description: 'Optional workspace root path' },
        path: { type: 'string', description: 'Workspace-relative .dx path (e.g. examples/welcome.dx)' },
        id: { type: 'number', description: 'Document ID (use list-documents to find IDs)' },
      },
      required: [],
      oneOf: [
        { required: ['path'] },
        { required: ['id'] },
      ],
      description: 'Provide either path or id.',
    },
  },
];

async function handleTool(id, toolName, args = {}) {
  try {
    let result;

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

      case 'get-document-visual': {
        const runtime = await getRuntime(args.workspacePath);

        let document;
        if (args.path) {
          document = await getDocumentByRelativePath(runtime.workspaceRoot, runtime.db, String(args.path));
        } else if (Number.isFinite(Number(args.id))) {
          document = await getDocument(runtime.workspaceRoot, runtime.db, Number(args.id));
        } else {
          return sendError(id, -32602, 'Either path or id is required');
        }

        if (!document) {
          return sendError(id, -32602, 'Document not found');
        }

        const visualModel = buildVisualModel(document);

        result = {
          document: {
            id: document.id,
            title: document.title,
            relativePath: document.relativePath,
            updatedAt: document.updatedAt,
          },
          visualModel,
        };
        break;
      }

      default:
        return sendError(id, -32601, `Unknown tool: ${toolName}`);
    }

    return sendResponse(id, {
      content: [{ type: 'text', text: safeJsonStringify(result) }],
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
      const resources = listDocuments(runtime.db, runtime.workspaceRoot).map((doc) => ({
        uri: `doc:///${doc.relativePath}`,
        name: doc.title || doc.relativePath,
        mimeType: 'text/plain',
      }));
      return sendResponse(id, { resources });
    }

    if (method === 'resources/read') {
      const uri = String(params?.uri || '');
      const runtime = await getRuntime(params?.workspacePath);
      const relativePath = uri.replace(/^doc:\/\/\//, '');
      const doc = await getDocumentByRelativePath(runtime.workspaceRoot, runtime.db, relativePath);

      if (!doc) {
        return sendError(id, -32602, 'Document not found');
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
