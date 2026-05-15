import path from 'node:path';

const BLOCK_TYPES = new Set([
  'heading',
  'paragraph',
  'bulleted-list',
  'numbered-list',
  'quote',
  'code',
  'image',
]);

function clampHeadingLevel(value) {
  const level = Number(value);

  if (!Number.isFinite(level)) {
    return 1;
  }

  return Math.min(4, Math.max(1, Math.trunc(level)));
}

function parseValue(raw) {
  const value = String(raw || '').trim();

  if (value === 'true') {
    return true;
  }

  if (value === 'false') {
    return false;
  }

  if (/^-?\d+$/.test(value)) {
    return Number(value);
  }

  if ((value.startsWith('{') && value.endsWith('}')) || (value.startsWith('[') && value.endsWith(']'))) {
    try {
      return JSON.parse(value);
    } catch {
      return value;
    }
  }

  return value;
}

function parseFrontmatter(text) {
  if (!text.startsWith('---\n')) {
    return { metadata: {}, body: text };
  }

  const end = text.indexOf('\n---\n', 4);

  if (end === -1) {
    return { metadata: {}, body: text };
  }

  const metadata = {};
  const frontmatterBlock = text.slice(4, end);
  const body = text.slice(end + 5);

  for (const line of frontmatterBlock.split('\n')) {
    if (!line.trim()) {
      continue;
    }

    const separatorIndex = line.indexOf(':');

    if (separatorIndex === -1) {
      continue;
    }

    const key = line.slice(0, separatorIndex).trim();
    const value = line.slice(separatorIndex + 1);
    metadata[key] = parseValue(value);
  }

  return { metadata, body };
}

function slugifyHeading(heading) {
  return String(heading || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'block';
}

function normalizeMeta(input) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return {};
  }

  return { ...input };
}

function normalizeTags(tags) {
  if (!Array.isArray(tags)) {
    return [];
  }

  return tags
    .map((tag) => String(tag).trim())
    .filter(Boolean);
}

function blockText(block) {
  if (block.type === 'image') {
    const alt = String(block.alt || '').trim();
    const src = String(block.src || '').trim();
    return alt || src;
  }

  if (block.type === 'bulleted-list' || block.type === 'numbered-list') {
    return block.items.join('\n');
  }

  return block.text || '';
}

function ensureUniqueId(seed, registry) {
  const base = slugifyHeading(seed);
  const seen = registry.get(base) || 0;
  registry.set(base, seen + 1);
  return seen === 0 ? base : `${base}-${seen + 1}`;
}

function normalizeClassName(value) {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

function parseAttributeString(rawAttributes) {
  const attributes = {};
  const text = String(rawAttributes || '');
  const pattern = /([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g;
  let match = pattern.exec(text);

  while (match) {
    const key = String(match[1] || '').trim();
    const value = match[2] ?? match[3] ?? match[4] ?? '';

    if (key) {
      attributes[key] = value;
    }

    match = pattern.exec(text);
  }

  return attributes;
}

function formatAttributeValue(value) {
  const text = String(value || '').trim();

  if (!text) {
    return '';
  }

  if (/^[^\s"'=]+$/.test(text)) {
    return text;
  }

  return `"${text.replace(/"/g, '')}"`;
}

function normalizeBlock(block, index, registry) {
  const blockType = BLOCK_TYPES.has(block?.type) ? block.type : 'paragraph';
  const idSeed = block?.id || (blockType === 'heading' ? block.text : `${blockType}-${index + 1}`);
  const id = ensureUniqueId(idSeed, registry);
  const className = normalizeClassName(block?.className || block?.class);

  if (blockType === 'heading') {
    return {
      id,
      className,
      type: 'heading',
      level: clampHeadingLevel(block?.level),
      text: String(block?.text || `Section ${index + 1}`).trim(),
    };
  }

  if (blockType === 'bulleted-list' || blockType === 'numbered-list') {
    const items = Array.isArray(block?.items)
      ? block.items.map((item) => String(item).trim()).filter(Boolean)
      : String(block?.text || '')
          .split('\n')
          .map((item) => item.trim())
          .filter(Boolean);

    return {
      id,
      className,
      type: blockType,
      items: items.length > 0 ? items : ['List item'],
    };
  }

  if (blockType === 'image') {
    return {
      id,
      className,
      type: 'image',
      src: String(block?.src || '').trim(),
      alt: String(block?.alt || '').trim(),
    };
  }

  const normalized = {
    id,
    className,
    type: blockType,
    text: String(block?.text || '').trim(),
  };

  if (blockType === 'code') {
    normalized.language = String(block?.language || '').trim();
  }

  return normalized;
}

function normalizeBlocks(blocks) {
  const sourceBlocks = Array.isArray(blocks) ? blocks : [];
  const registry = new Map();
  const normalized = sourceBlocks.map((block, index) => normalizeBlock(block, index, registry));

  if (normalized.length > 0) {
    return normalized;
  }

  return [normalizeBlock({ type: 'paragraph', text: 'Start writing here.' }, 0, registry)];
}

function parseDocsrcHeader(text) {
  const marker = '\n---\n';
  const separatorIndex = text.indexOf(marker);

  if (separatorIndex === -1) {
    return null;
  }

  const header = text.slice(0, separatorIndex).split('\n');

  if (!header[0] || !header[0].startsWith('@doc')) {
    return null;
  }

  const payload = {
    title: '',
    summary: '',
    tags: [],
    meta: {},
  };

  for (const line of header.slice(1)) {
    const trimmed = line.trim();

    if (!trimmed) {
      continue;
    }

    const separator = trimmed.indexOf(':');

    if (separator === -1) {
      continue;
    }

    const key = trimmed.slice(0, separator).trim();
    const value = trimmed.slice(separator + 1).trim();

    if (key === 'title') {
      payload.title = value;
      continue;
    }

    if (key === 'summary') {
      payload.summary = value;
      continue;
    }

    if (key === 'tags') {
      payload.tags = value.split(',').map((tag) => tag.trim()).filter(Boolean);
      continue;
    }

    if (key.startsWith('meta.')) {
      const metaKey = key.slice(5).trim();

      if (metaKey) {
        payload.meta[metaKey] = parseValue(value);
      }
    }
  }

  payload.body = text.slice(separatorIndex + marker.length);
  return payload;
}

function parseBlockHeader(headerLine) {
  const match = /^::([a-z-]+)(?:\s+(.*))?$/.exec(headerLine.trim());

  if (!match) {
    return null;
  }

  const type = match[1];
  const attributes = parseAttributeString(match[2] || '');

  return { type, attributes };
}

function parseDocsrcBlocks(body) {
  const lines = body.replace(/^\n+/, '').split('\n');
  const blocks = [];
  let current = null;

  function flushCurrent() {
    if (!current) {
      return;
    }

    const content = current.lines.join('\n').trim();
    const { type, attributes } = current.header;

    if (type === 'heading') {
      blocks.push({
        type: 'heading',
        level: clampHeadingLevel(attributes.level || 1),
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: content || 'Section',
      });
    } else if (type === 'paragraph') {
      blocks.push({
        type: 'paragraph',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: content,
      });
    } else if (type === 'quote') {
      blocks.push({
        type: 'quote',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: content,
      });
    } else if (type === 'code') {
      blocks.push({
        type: 'code',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        language: attributes.lang || '',
        text: current.lines.join('\n').replace(/\n+$/, ''),
      });
    } else if (type === 'bulleted-list' || type === 'numbered-list') {
      blocks.push({
        type,
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        items: current.lines.map((line) => line.trim()).filter(Boolean),
      });
    } else if (type === 'image') {
      blocks.push({
        type,
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        src: String(attributes.src || '').trim(),
        alt: current.lines.join('\n').trim(),
      });
    }

    current = null;
  }

  for (const line of lines) {
    if (!current) {
      const header = parseBlockHeader(line);

      if (header) {
        current = { header, lines: [] };
      }

      continue;
    }

    if (line.trim() === '::end') {
      flushCurrent();
      continue;
    }

    current.lines.push(line);
  }

  flushCurrent();
  return blocks;
}

function flushBufferedParagraph(paragraphLines, blocks) {
  if (paragraphLines.length === 0) {
    return;
  }

  blocks.push({
    type: 'paragraph',
    text: paragraphLines.join(' ').trim(),
  });
  paragraphLines.length = 0;
}

function flushBufferedList(listState, blocks) {
  if (!listState.type || listState.items.length === 0) {
    listState.type = null;
    listState.items = [];
    return;
  }

  blocks.push({
    type: listState.type,
    items: [...listState.items],
  });

  listState.type = null;
  listState.items = [];
}

function flushBufferedQuote(quoteLines, blocks) {
  if (quoteLines.length === 0) {
    return;
  }

  blocks.push({
    type: 'quote',
    text: quoteLines.join('\n').trim(),
  });

  quoteLines.length = 0;
}

function parseLegacyBlocks(body) {
  const blocks = [];
  const paragraphLines = [];
  const quoteLines = [];
  const listState = { type: null, items: [] };
  const lines = body.replace(/^\n+/, '').split('\n');
  let codeFence = null;
  let codeLines = [];

  for (const line of lines) {
    const headingMatch = /^(#{1,4})\s+(.*)$/.exec(line);
    const bulletListMatch = /^[-*]\s+(.*)$/.exec(line);
    const numberedListMatch = /^\d+\.\s+(.*)$/.exec(line);
    const quoteMatch = /^>\s?(.*)$/.exec(line);
    const codeFenceMatch = /^```(.*)$/.exec(line.trim());

    if (codeFence) {
      if (codeFenceMatch) {
        blocks.push({
          type: 'code',
          language: codeFence,
          text: codeLines.join('\n').trimEnd(),
        });
        codeFence = null;
        codeLines = [];
      } else {
        codeLines.push(line);
      }

      continue;
    }

    if (codeFenceMatch) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedList(listState, blocks);
      flushBufferedQuote(quoteLines, blocks);
      codeFence = codeFenceMatch[1].trim();
      codeLines = [];
      continue;
    }

    if (headingMatch) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedList(listState, blocks);
      flushBufferedQuote(quoteLines, blocks);
      blocks.push({
        type: 'heading',
        level: headingMatch[1].length,
        text: headingMatch[2].trim(),
      });
      continue;
    }

    if (bulletListMatch) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedQuote(quoteLines, blocks);

      if (listState.type && listState.type !== 'bulleted-list') {
        flushBufferedList(listState, blocks);
      }

      listState.type = 'bulleted-list';
      listState.items.push(bulletListMatch[1].trim());
      continue;
    }

    if (numberedListMatch) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedQuote(quoteLines, blocks);

      if (listState.type && listState.type !== 'numbered-list') {
        flushBufferedList(listState, blocks);
      }

      listState.type = 'numbered-list';
      listState.items.push(numberedListMatch[1].trim());
      continue;
    }

    if (quoteMatch) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedList(listState, blocks);
      quoteLines.push(quoteMatch[1]);
      continue;
    }

    if (!line.trim()) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedList(listState, blocks);
      flushBufferedQuote(quoteLines, blocks);
      continue;
    }

    paragraphLines.push(line.trim());
  }

  flushBufferedParagraph(paragraphLines, blocks);
  flushBufferedList(listState, blocks);
  flushBufferedQuote(quoteLines, blocks);

  if (codeFence) {
    blocks.push({
      type: 'code',
      language: codeFence,
      text: codeLines.join('\n').trimEnd(),
    });
  }

  return blocks;
}

function buildSections(blocks) {
  const sections = [];
  let currentSection = {
    id: 'overview',
    depth: 0,
    heading: 'Overview',
    content: [],
  };

  for (const block of blocks) {
    if (block.type === 'heading') {
      if (currentSection.content.length || currentSection.depth > 0) {
        sections.push({
          ...currentSection,
          content: currentSection.content.join('\n\n').trim(),
        });
      }

      currentSection = {
        id: block.id,
        depth: block.level,
        heading: block.text,
        content: [],
      };
      continue;
    }

    const content = blockText(block).trim();

    if (content) {
      currentSection.content.push(content);
    }
  }

  if (currentSection.content.length || currentSection.depth > 0) {
    sections.push({
      ...currentSection,
      content: currentSection.content.join('\n\n').trim(),
    });
  }

  return sections;
}

function extractSummary(blocks) {
  for (const block of blocks) {
    if (block.type === 'heading') {
      continue;
    }

    const content = blockText(block).trim();

    if (content) {
      return content.split('\n')[0];
    }
  }

  return '';
}

function parseDocSource(text) {
  const docsrcHeader = parseDocsrcHeader(text);

  if (docsrcHeader) {
    return {
      title: docsrcHeader.title,
      summary: docsrcHeader.summary,
      tags: docsrcHeader.tags,
      meta: docsrcHeader.meta,
      blocks: parseDocsrcBlocks(docsrcHeader.body),
      source: text,
    };
  }

  const trimmed = text.trim();

  if (trimmed.startsWith('{')) {
    const parsed = JSON.parse(trimmed);
    return {
      title: parsed.title,
      summary: parsed.summary,
      tags: parsed.tags,
      meta: parsed.meta,
      blocks: parsed.blocks,
      source: text,
    };
  }

  const { metadata, body } = parseFrontmatter(text);

  return {
    metadata,
    body,
    source: text,
  };
}

export function normalizeDocInput(filePath, input = {}) {
  const legacyMetadata = normalizeMeta(input.metadata);
  const title = String(input.title || legacyMetadata.title || path.basename(filePath, path.extname(filePath))).trim();
  const tags = normalizeTags(input.tags || legacyMetadata.tags);
  const meta = {
    ...normalizeMeta(input.meta),
  };

  for (const [key, value] of Object.entries(legacyMetadata)) {
    if (key === 'title' || key === 'summary' || key === 'tags') {
      continue;
    }

    meta[key] = value;
  }

  const blocks = normalizeBlocks(input.blocks || parseLegacyBlocks(String(input.body || '')));
  const summary = String(input.summary || legacyMetadata.summary || extractSummary(blocks)).trim();
  const sections = buildSections(blocks);

  const document = {
    filePath,
    title,
    summary,
    tags,
    meta,
    metadata: {
      title,
      summary,
      tags,
      ...meta,
    },
    blocks,
    sections,
    source: '',
  };

  document.source = typeof input.source === 'string' && input.source.trim()
    ? `${input.source.trimEnd()}\n`
    : stringifyDocFile(document);

  return document;
}

export function parseDocFile(filePath, text) {
  const parsed = parseDocSource(text);
  return normalizeDocInput(filePath, parsed);
}

export function createDefaultBlocks(title) {
  return normalizeBlocks([
    {
      type: 'heading',
      level: 1,
      text: title,
    },
    {
      type: 'paragraph',
      text: 'Start writing without markup syntax. Add blocks from the editor toolbar.',
    },
  ]);
}

function encodeMetaLines(meta) {
  return Object.entries(meta || {})
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `meta.${key}: ${typeof value === 'string' ? value : JSON.stringify(value)}`);
}

function blockHeader(block) {
  const blockId = block.id || slugifyHeading(block.text || block.type || 'block');
  const attributes = [`id=${formatAttributeValue(blockId)}`];

  if (normalizeClassName(block.className)) {
    attributes.push(`class=${formatAttributeValue(normalizeClassName(block.className))}`);
  }

  if (block.type === 'heading') {
    attributes.unshift(`level=${clampHeadingLevel(block.level)}`);
    return `::heading ${attributes.join(' ')}`;
  }

  if (block.type === 'code') {
    if (block.language) {
      attributes.push(`lang=${formatAttributeValue(block.language)}`);
    }
    return `::code ${attributes.join(' ')}`;
  }

  if (block.type === 'image') {
    if (block.src) {
      attributes.push(`src=${formatAttributeValue(block.src)}`);
    }
    return `::image ${attributes.join(' ')}`;
  }

  return `::${block.type} ${attributes.join(' ')}`;
}

function blockBody(block) {
  if (block.type === 'bulleted-list' || block.type === 'numbered-list') {
    return block.items.join('\n');
  }

  if (block.type === 'image') {
    return block.alt || '';
  }

  return block.text || '';
}

export function stringifyDocFile(document) {
  const headerLines = [
    '@doc 3',
    `title: ${document.title || ''}`,
    `summary: ${document.summary || ''}`,
    `tags: ${normalizeTags(document.tags).join(', ')}`,
    ...encodeMetaLines(normalizeMeta(document.meta)),
    '---',
  ];

  const blockChunks = (document.blocks || []).map((block) => {
    const lines = [blockHeader(block), blockBody(block), '::end'];
    return lines.join('\n');
  });

  const body = blockChunks.join('\n\n');
  return `${headerLines.join('\n')}\n${body}\n`;
}
