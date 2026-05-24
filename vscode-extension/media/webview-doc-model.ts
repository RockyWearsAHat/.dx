import { parseSourceBlocks } from './doc-pipeline.js';
import type { ChecklistItem, PipelineBlock } from './doc-pipeline.js';

type Attributes = Record<string, string>;

type ListItemLike =
  | string
  | ChecklistItem
  | {
      text?: string;
      nested?: ListItemLike[];
    };

interface DocumentModel {
  blocks: PipelineBlock[];
}

export function parseAttributes(args: string | number | boolean | null | undefined | object): Attributes {
  const attributes: Attributes = {};
  const pattern = /([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g;
  const text = String(args || '');
  let match = pattern.exec(text);

  while (match) {
    const key = String(match[1] || '').trim().toLowerCase();
    const value = match[2] ?? match[3] ?? match[4] ?? '';

    if (key) {
      attributes[key] = value;
    }

    match = pattern.exec(text);
  }

  return attributes;
}

export const KNOWN_BLOCK_TYPES = new Set([
  'paragraph', 'heading', 'bulleted-list', 'numbered-list', 'list',
  'checklist', 'quote', 'code', 'image', 'rule', 'style', 'stylesheet',
  'svg', 'html', 'graph', 'mermaid',
]);

const LEGACY_LIST_ITEM_FALLBACKS: Record<string, string[]> = {
  'how-it-works-steps': [
    'Every document is a sequence of typed blocks. There is no markdown, no inline HTML, and no implicit formatting.',
    'Blocks are stored in SQLite and packed into a compact binary bundle at .doc/.repo-docs.bin that commits with your code.',
    'The .dx file on disk is a stub pointer - a few lines that reference the archive. The real content never lives in the stub.',
    'An MCP server exposes every document to any connected agent for search, retrieval, and structured editing.',
  ],
};

function normalizeClassName(value: string | number | boolean | null | undefined | object): string {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

function formatAttributeValue(value: string | number | boolean | null | undefined | object): string {
  const text = String(value || '').trim();

  if (!text) {
    return '';
  }

  if (/^[^\s"'=]+$/.test(text)) {
    return text;
  }

  return '"' + text.replace(/"/g, '') + '"';
}

function isLegacyObjectMarker(text: string | number | boolean | null | undefined | object): boolean {
  return String(text || '').trim() === '[object Object]';
}

function containsLegacyObjectMarker(lines: string[]): boolean {
  return Array.isArray(lines) && lines.some((line) => isLegacyObjectMarker(String(line || '').replace(/^\s*(?:[-*]|\d+[.)])\s+/, '')));
}

function parseLegacyListObjectText(text: string | number | boolean | null | undefined | object): string {
  const trimmed = String(text || '').trim();
  if (!trimmed.startsWith('{') || !trimmed.endsWith('}')) {
    return '';
  }

  try {
    const parsed = JSON.parse(trimmed) as { text?: string | number | boolean | null | undefined | object };
    if (parsed && typeof parsed === 'object') {
      return String(parsed.text || '').trim();
    }
  } catch {
    return '';
  }

  return '';
}

function repairLegacyListItems(items: string[], blockId: string): string[] {
  const cleaned: string[] = [];

  for (const item of items || []) {
    if (isLegacyObjectMarker(item)) {
      continue;
    }
    cleaned.push(item);
  }

  if (cleaned.length > 0) {
    return cleaned;
  }

  const fallback = LEGACY_LIST_ITEM_FALLBACKS[String(blockId || '').trim()];
  return Array.isArray(fallback) ? [...fallback] : [];
}

export function listItemText(item: ListItemLike): string {
  if (typeof item === 'object' && item !== null) {
    return String(item.text || '').trim();
  }

  return String(item || '').trim();
}

function flattenListItems(items: ListItemLike[], depth = 0): string[] {
  if (!Array.isArray(items)) {
    return [];
  }

  const lines: string[] = [];
  const indent = '  '.repeat(Math.max(0, depth));

  for (const item of items) {
    if (typeof item === 'object' && item !== null) {
      const text = listItemText(item);
      if (text) {
        lines.push(`${indent}- ${text}`);
      }

      if ('nested' in item && Array.isArray(item.nested) && item.nested.length > 0) {
        lines.push(...flattenListItems(item.nested, depth + 1));
      }

      continue;
    }

    const text = listItemText(item);
    if (text) {
      lines.push(`${indent}- ${text}`);
    }
  }

  return lines;
}

function isBulletedListType(type: string): boolean {
  return type === 'list' || type === 'bulleted-list';
}

function isNumberedListType(type: string): boolean {
  return type === 'numbered-list';
}

function parseListItems(lines: string[], blockId = ''): string[] {
  const parsedItems = (lines || [])
    .map((line) => {
      const text = String(line || '').replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim();
      if (!text) {
        return '';
      }

      if (isLegacyObjectMarker(text)) {
        return text;
      }

      const recoveredText = parseLegacyListObjectText(text);
      return recoveredText || text;
    })
    .filter((value): value is string => Boolean(value));

  return repairLegacyListItems(parsedItems, blockId);
}

function buildBlockHeader(type: string, attributes: Record<string, string | number | boolean | null | undefined | object>): string {
  const parts = Object.entries(attributes || {})
    .filter(([, value]) => String(value || '').trim())
    .map(([key, value]) => `${key}=${formatAttributeValue(value)}`);

  return parts.length > 0 ? `::${type} ${parts.join(' ')}` : `::${type}`;
}

function toChecklistItem(item: string | ChecklistItem): ChecklistItem {
  if (typeof item === 'string') {
    const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(item.trim());
    if (match) {
      const checkedToken = match[1] ?? ' ';
      const text = match[2] ?? '';
      return { checked: checkedToken.toLowerCase() === 'x', text };
    }
    return { checked: false, text: item };
  }

  return item;
}

export function stringifyBlock(block: PipelineBlock | null | undefined): string {
  if (!block) return '';

  if (block.rawSource) {
    const source = String(block.rawSource).trimEnd();
    if (block.type !== 'paragraph' && block.type !== 'rule' && !source.endsWith('::end')) {
      return source + '\n::end';
    }
    return source;
  }

  const attributes: Record<string, string> = {};

  if (block.id) {
    attributes.id = block.id;
  }

  if (normalizeClassName(block.className)) {
    attributes.class = normalizeClassName(block.className);
  }

  if (block.type === 'code' && String(block.language || '').trim()) {
    attributes.language = String(block.language || '').trim();
  }

  if (block.type === 'heading') {
    return buildBlockHeader('heading', {
      ...attributes,
      level: String(block.level || 1),
    }) + '\n' + (block.text || '') + '\n::end';
  }

  if (isBulletedListType(block.type)) {
    return buildBlockHeader('bulleted-list', attributes) + '\n' + flattenListItems((block.items || []) as ListItemLike[]).join('\n') + '\n::end';
  }

  if (isNumberedListType(block.type)) {
    return buildBlockHeader('numbered-list', attributes) + '\n' + flattenListItems((block.items || []) as ListItemLike[]).join('\n') + '\n::end';
  }

  if (block.type === 'checklist') {
    return buildBlockHeader('checklist', attributes) + '\n' + (block.items || []).map((item) => {
      const normalized = toChecklistItem(item as string | ChecklistItem);
      return '[' + (normalized.checked ? 'x' : ' ') + '] ' + normalized.text;
    }).join('\n') + '\n::end';
  }

  if (block.type === 'image') {
    const src = String(block.src || '').trim();
    const alt = String(block.alt || '').trim();
    return buildBlockHeader('image', {
      ...attributes,
      src,
    }) + '\n' + alt + '\n::end';
  }

  if (block.type === 'stylesheet') {
    if (String(block.media || '').trim()) {
      attributes.media = String(block.media || '').trim();
    }
    if (String(block.href || '').trim()) {
      attributes.href = String(block.href || '').trim();
    }
    return buildBlockHeader('stylesheet', attributes) + '\n::end';
  }

  if (block.type === 'style') {
    return buildBlockHeader('style', attributes) + '\n' + (block.text || '') + '\n::end';
  }

  if (block.type === 'rule') {
    return buildBlockHeader('rule', attributes);
  }

  return buildBlockHeader(block.type, attributes) + '\n' + (block.text || '') + '\n::end';
}

export function parseBlock(raw: string | number | boolean | null | undefined | object): PipelineBlock {
  const original = String(raw || '').replace(/\r\n/g, '\n');
  const trimmed = original.trim();

  if (!trimmed.startsWith('::')) {
    return { type: 'paragraph', id: '', className: '', text: original, rawSource: original };
  }

  const lines = trimmed.split('\n');
  const firstLine = lines[0] ?? '';
  const open = /^::([a-z-]+)(.*)$/i.exec(firstLine.trim());

  if (!open) {
    return { type: 'paragraph', id: '', className: '', text: trimmed, rawSource: original };
  }

  const type = String(open[1] ?? '').toLowerCase();
  const attrs = parseAttributes(open[2] ?? '');
  let endIdx = lines.length;

  for (let i = 1; i < lines.length; i += 1) {
    const line = lines[i] ?? '';
    if (line.trim() === '::end') {
      endIdx = i;
      break;
    }
  }

  const content = lines.slice(1, endIdx);

  if (type === 'heading') {
    const level = Number(attrs.level || 1);
    return {
      type,
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      level: Math.min(6, Math.max(1, level)),
      text: content.join('\n').trim(),
      rawSource: original,
    };
  }

  if (isBulletedListType(type)) {
    const blockId = String(attrs.id || '').trim();
    const items = parseListItems(content, blockId);
    const hadLegacyCorruption = containsLegacyObjectMarker(content);
    return {
      type: 'bulleted-list',
      id: blockId,
      className: normalizeClassName(attrs.class),
      items,
      rawSource: hadLegacyCorruption ? '' : original,
    };
  }

  if (isNumberedListType(type)) {
    const blockId = String(attrs.id || '').trim();
    const items = parseListItems(content, blockId);
    const hadLegacyCorruption = containsLegacyObjectMarker(content);
    return {
      type: 'numbered-list',
      id: blockId,
      className: normalizeClassName(attrs.class),
      items,
      rawSource: hadLegacyCorruption ? '' : original,
    };
  }

  if (type === 'checklist') {
    return {
      type,
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      items: content
        .map((line) => {
          const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(line.trim());
          if (match) {
            const checkedToken = match[1] ?? ' ';
            const text = match[2] ?? '';
            return { checked: checkedToken.toLowerCase() === 'x', text };
          }
          return { checked: false, text: line.trim() };
        })
        .filter((item) => item.text.length > 0),
      rawSource: original,
    };
  }

  if (type === 'image') {
    return {
      type,
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      src: String(attrs.src || '').trim(),
      alt: content.join('\n').trim(),
      rawSource: original,
    };
  }

  if (type === 'stylesheet') {
    return {
      type: 'stylesheet',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      href: String(attrs.href || attrs.src || content.join('\n').trim() || '').trim(),
      media: String(attrs.media || '').trim(),
      rawSource: original,
    };
  }

  if (type === 'style') {
    return {
      type: 'style',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      text: content.join('\n').trimEnd(),
      rawSource: original,
    };
  }

  if (type === 'rule') {
    return {
      type: 'rule',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      language: String(attrs.language || attrs.lang || '').trim(),
    };
  }

  if (type === 'code') {
    return {
      type: 'code',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      language: String(attrs.language || attrs.lang || '').trim(),
      text: content.join('\n').trimEnd(),
      rawSource: original,
    };
  }

  if (type === 'paragraph') {
    return {
      type: 'paragraph',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      text: content.join('\n'),
      rawSource: original,
    };
  }

  return {
    type,
    id: String(attrs.id || '').trim(),
    className: normalizeClassName(attrs.class),
    text: content.join('\n').trimEnd(),
    rawSource: original,
  };
}

export function parseDoc(source: string | number | boolean | null | undefined | object): DocumentModel {
  return { blocks: parseSourceBlocks(source) };
}

export function stringifyDoc(model: DocumentModel): string {
  const lines: string[] = [];

  for (let i = 0; i < model.blocks.length; i += 1) {
    lines.push(stringifyBlock(model.blocks[i]));

    if (i < model.blocks.length - 1) {
      lines.push('');
    }
  }

  return lines.join('\n').trimEnd() + '\n';
}

export function extractCssFromDocumentModel(model: DocumentModel | null | undefined): string {
  if (!model || !Array.isArray(model.blocks)) {
    return '';
  }

  const chunks: string[] = [];

  for (const block of model.blocks) {
    if (!block) {
      continue;
    }

    if (block.type === 'style') {
      const css = String(block.text || '').trim();
      if (css) {
        chunks.push(css);
      }
      continue;
    }

    if (block.type !== 'code') {
      continue;
    }

    const language = String(block.language || '').trim().toLowerCase();
    if (language !== 'css' && language !== 'stylesheet') {
      continue;
    }

    const css = String(block.text || '').trim();
    if (css) {
      chunks.push(css);
    }
  }

  return chunks.join('\n\n');
}

export function extractStylesheetLinksFromDocumentModel(model: DocumentModel | null | undefined): Array<{ href: string; media: string }> {
  if (!model || !Array.isArray(model.blocks)) {
    return [];
  }

  const links: Array<{ href: string; media: string }> = [];

  for (const block of model.blocks) {
    if (!block || block.type !== 'stylesheet') {
      continue;
    }

    const href = String(block.href || block.src || '').trim();
    const media = String(block.media || '').trim();

    if (!href) {
      continue;
    }

    links.push({ href, media });
  }

  return links;
}

export function findCssBlockIndex(model: DocumentModel | null | undefined): number {
  if (!model || !Array.isArray(model.blocks)) {
    return -1;
  }

  for (let i = 0; i < model.blocks.length; i += 1) {
    const block = model.blocks[i];
    if (!block || block.type !== 'code') continue;
    const language = String(block.language || '').trim().toLowerCase();
    if (language === 'css' || language === 'stylesheet') {
      return i;
    }
  }

  return -1;
}
