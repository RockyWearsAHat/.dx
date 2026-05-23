import { Buffer } from 'node:buffer';

const BLOCK_TYPE_TO_CODE = {
  heading: 1,
  paragraph: 2,
  'bulleted-list': 3,
  'numbered-list': 4,
  quote: 5,
  code: 6,
  image: 7,
  rule: 8,
  checklist: 9,
};

const CODE_TO_BLOCK_TYPE = Object.fromEntries(
  Object.entries(BLOCK_TYPE_TO_CODE).map(([type, code]) => [code, type])
);

const MAGIC = Buffer.from('DOCB1', 'ascii');

type BlockType =
  | 'heading'
  | 'paragraph'
  | 'bulleted-list'
  | 'numbered-list'
  | 'quote'
  | 'code'
  | 'image'
  | 'rule'
  | 'checklist';

interface ChecklistItem {
  checked?: boolean;
  text?: string;
}

interface BinaryBlock {
  type?: BlockType | string;
  id?: string;
  level?: number;
  text?: string;
  language?: string;
  src?: string;
  alt?: string;
  items?: Array<string | ChecklistItem>;
}

interface BinaryDocument {
  title?: string;
  summary?: string;
  tags?: string[];
  meta?: Record<string, string | number | boolean | null | undefined | object>;
  blocks?: BinaryBlock[];
}

interface DecodeState {
  offset: number;
}

function encodeVarint(value: number | string | null | undefined): Buffer {
  const numberValue = Number(value);
  let number = Math.max(0, Math.trunc(numberValue || 0));
  const bytes = [];

  while (number >= 0x80) {
    bytes.push((number & 0x7f) | 0x80);
    number >>= 7;
  }

  bytes.push(number);
  return Buffer.from(bytes);
}

function decodeVarint(buffer: Buffer, state: DecodeState): number {
  let shift = 0;
  let value = 0;

  while (state.offset < buffer.length) {
    const byte = buffer[state.offset];
    if (byte === undefined) {
      throw new Error('Unexpected end of binary document while decoding varint byte.');
    }
    state.offset += 1;
    value |= (byte & 0x7f) << shift;

    if ((byte & 0x80) === 0) {
      return value;
    }

    shift += 7;
  }

  throw new Error('Unexpected end of binary document while decoding varint.');
}

function encodeString(value: string | number | null | undefined): Buffer {
  const text = String(value || '');
  const payload = Buffer.from(text, 'utf8');
  return Buffer.concat([encodeVarint(payload.length), payload]);
}

function decodeString(buffer: Buffer, state: DecodeState): string {
  const length = decodeVarint(buffer, state);
  const end = state.offset + length;

  if (end > buffer.length) {
    throw new Error('Unexpected end of binary document while decoding string.');
  }

  const value = buffer.toString('utf8', state.offset, end);
  state.offset = end;
  return value;
}

function encodeMeta(meta: Record<string, string | number | boolean | null | undefined | object> | null | undefined): Buffer {
  // Defensive fallback is unreachable through public packDocument flow.
  const entries = Object.entries(meta || {});
  const parts = [encodeVarint(entries.length)];

  for (const [key, value] of entries) {
    parts.push(encodeString(key));
    parts.push(encodeString(JSON.stringify(value)));
  }

  return Buffer.concat(parts);
}

function decodeMeta(buffer: Buffer, state: DecodeState): Record<string, string | number | boolean | null | undefined | object> {
  const count = decodeVarint(buffer, state);
  const meta: Record<string, string | number | boolean | null | undefined | object> = {};

  for (let index = 0; index < count; index += 1) {
    const key = decodeString(buffer, state);
    const rawValue = decodeString(buffer, state);

    try {
      meta[key] = JSON.parse(rawValue);
    } catch {
      meta[key] = rawValue;
    }
  }

  return meta;
}

function encodeTags(tags: string[] | null | undefined): Buffer {
  // Defensive fallback is unreachable through public packDocument flow.
  const safeTags = Array.isArray(tags) ? tags : [];
  const parts = [encodeVarint(safeTags.length)];

  for (const tag of safeTags) {
    parts.push(encodeString(tag));
  }

  return Buffer.concat(parts);
}

function decodeTags(buffer: Buffer, state: DecodeState): string[] {
  const count = decodeVarint(buffer, state);
  const tags: string[] = [];

  for (let index = 0; index < count; index += 1) {
    tags.push(decodeString(buffer, state));
  }

  return tags;
}

function encodeBlock(block: BinaryBlock): Buffer {
  const normalizedType = typeof block.type === 'string' ? block.type : 'paragraph';
  const blockCode = BLOCK_TYPE_TO_CODE[normalizedType as keyof typeof BLOCK_TYPE_TO_CODE] || BLOCK_TYPE_TO_CODE.paragraph;
  const parts = [Buffer.from([blockCode]), encodeString(block.id || '')];

  if (block.type === 'heading') {
    parts.push(Buffer.from([Math.max(1, Math.min(4, Number(block.level) || 1))]));
    parts.push(encodeString(block.text || ''));
    return Buffer.concat(parts);
  }

  if (block.type === 'bulleted-list' || block.type === 'numbered-list') {
    const items = Array.isArray(block.items) ? block.items : [];
    parts.push(encodeVarint(items.length));

    for (const item of items) {
      const text = typeof item === 'object' && item !== null ? String(item.text || '') : String(item || '');
      parts.push(encodeString(text));
    }

    return Buffer.concat(parts);
  }

  if (block.type === 'code') {
    parts.push(encodeString(block.language || ''));
    parts.push(encodeString(block.text || ''));
    return Buffer.concat(parts);
  }

  if (block.type === 'image') {
    parts.push(encodeString(block.src || ''));
    parts.push(encodeString(block.alt || ''));
    return Buffer.concat(parts);
  }

  if (block.type === 'rule') {
    return Buffer.concat(parts);
  }

  if (block.type === 'checklist') {
    const items = Array.isArray(block.items) ? block.items : [];
    parts.push(encodeVarint(items.length));
    for (const item of items) {
      const checked = typeof item === 'object' && item !== null ? Boolean(item.checked) : false;
      const text = typeof item === 'object' && item !== null ? String(item.text || '') : String(item || '');
      parts.push(Buffer.from([checked ? 1 : 0]));
      parts.push(encodeString(text));
    }
    return Buffer.concat(parts);
  }

  parts.push(encodeString(block.text || ''));
  return Buffer.concat(parts);
}

function decodeBlock(buffer: Buffer, state: DecodeState): BinaryBlock {
  if (state.offset >= buffer.length) {
    throw new Error('Unexpected end of binary document while decoding block.');
  }

  const typeCode = buffer[state.offset];
  if (typeCode === undefined) {
    throw new Error('Unexpected end of binary document while decoding block type.');
  }
  state.offset += 1;

  // Unknown type codes require malformed payloads crafted outside packDocument.
  const type = (CODE_TO_BLOCK_TYPE[typeCode] || 'paragraph') as BlockType | string;
  const id = decodeString(buffer, state);

  if (type === 'heading') {
    if (state.offset >= buffer.length) {
      throw new Error('Unexpected end of binary document while decoding heading level.');
    }

    const level = buffer[state.offset];
    if (level === undefined) {
      throw new Error('Unexpected end of binary document while decoding heading level.');
    }
    state.offset += 1;
    const text = decodeString(buffer, state);
    return { type, id, level, text };
  }

  if (type === 'bulleted-list' || type === 'numbered-list') {
    const count = decodeVarint(buffer, state);
    const items = [];

    for (let index = 0; index < count; index += 1) {
      items.push(decodeString(buffer, state));
    }

    return { type, id, items };
  }

  if (type === 'code') {
    const language = decodeString(buffer, state);
    const text = decodeString(buffer, state);
    return { type, id, language, text };
  }

  if (type === 'image') {
    const src = decodeString(buffer, state);
    const alt = decodeString(buffer, state);
    return { type, id, src, alt };
  }

  if (type === 'rule') {
    return { type, id };
  }

  if (type === 'checklist') {
    const count = decodeVarint(buffer, state);
    const items = [];
    for (let i = 0; i < count; i += 1) {
      if (state.offset >= buffer.length) {
        throw new Error('Unexpected end of binary document while decoding checklist item.');
      }
      const checked = buffer[state.offset] === 1;
      state.offset += 1;
      const text = decodeString(buffer, state);
      items.push({ checked, text });
    }
    return { type, id, items };
  }

  const text = decodeString(buffer, state);
  return { type, id, text };
}

export function packDocument(document: BinaryDocument): Buffer {
  const parts = [
    MAGIC,
    encodeVarint(1),
    encodeString(document.title || ''),
    encodeString(document.summary || ''),
    encodeTags(document.tags),
    encodeMeta(document.meta),
  ];

  const blocks = Array.isArray(document.blocks) ? document.blocks : [];
  parts.push(encodeVarint(blocks.length));

  for (const block of blocks) {
    parts.push(encodeBlock(block));
  }

  return Buffer.concat(parts);
}

export function unpackDocument(blob: Buffer | Uint8Array): BinaryDocument {
  const buffer = Buffer.isBuffer(blob) ? blob : Buffer.from(blob);

  if (buffer.length < MAGIC.length + 1 || !buffer.subarray(0, MAGIC.length).equals(MAGIC)) {
    throw new Error('Document blob is not a valid DOC binary payload.');
  }

  const state = { offset: MAGIC.length };
  decodeVarint(buffer, state);

  const title = decodeString(buffer, state);
  const summary = decodeString(buffer, state);
  const tags = decodeTags(buffer, state);
  const meta = decodeMeta(buffer, state);
  const blockCount = decodeVarint(buffer, state);
  const blocks = [];

  for (let index = 0; index < blockCount; index += 1) {
    blocks.push(decodeBlock(buffer, state));
  }

  return {
    title,
    summary,
    tags,
    meta,
    blocks,
  };
}
