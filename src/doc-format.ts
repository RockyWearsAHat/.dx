import path from 'node:path';

const BLOCK_TYPES = new Set([
  'heading',
  'paragraph',
  'bulleted-list',
  'numbered-list',
  'quote',
  'code',
  'image',
  'checklist',
  'rule',
  'style',
  'stylesheet',
]);

function clampHeadingLevel(value) {
  const level = Number(value);

  if (!Number.isFinite(level)) {
    return 1;
  }

  return Math.min(4, Math.max(1, Math.trunc(level)));
}

function parseValue(raw) {
  const value = String(raw).trim();

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
  if (block.type === 'style' || block.type === 'stylesheet') {
    return '';
  }

  if (block.type === 'image') {
    const alt = String(block.alt || '').trim();
    const src = String(block.src || '').trim();
    return alt || src;
  }

  if (block.type === 'checklist') {
    return block.items.map((item) => String(item.text).trim()).join('\n');
  }

  if (block.type === 'rule') {
    return '';
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
    const key = String(match[1]).trim();
    const value = match[2] ?? match[3] ?? match[4];

    if (key) {
      attributes[key] = value;
    }

    match = pattern.exec(text);
  }

  return attributes;
}

function formatAttributeValue(value) {
  const text = String(value).trim();

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
      ? block.items.map((item) => {
          // Preserve nested structure: if item is an object with text/nested, keep it
          if (typeof item === 'object' && item !== null && item.text !== undefined) {
            return {
              text: String(item.text || '').trim(),
              ...(item.nested && Array.isArray(item.nested) && { nested: item.nested }),
            };
          }
          // Fallback: treat as string
          return { text: String(item).trim() };
        }).filter((item) => item.text.length > 0)
      : String(block?.text ?? '')
          .split('\n')
          .map((item) => ({ text: item.trim() }))
          .filter((item) => item.text.length > 0);

    return {
      id,
      className,
      type: blockType,
      items: items.length > 0 ? items : [{ text: 'List item' }],
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

  if (blockType === 'checklist') {
    const rawItems = Array.isArray(block?.items) ? block.items : [];
    const items = rawItems.map((item) => {
      if (typeof item === 'object' && item !== null) {
        return { checked: Boolean(item.checked), text: String(item.text || '').trim() };
      }
        return { checked: false, text: String(item ?? '').trim() };
    }).filter((item) => item.text.length > 0);
    return {
      id,
      className,
      type: 'checklist',
      items: items.length > 0 ? items : [{ checked: false, text: 'Item' }],
    };
  }

  if (blockType === 'rule') {
    return { id, className, type: 'rule' };
  }

  if (blockType === 'style') {
    return {
      id,
      className,
      type: 'style',
      text: String(block?.text || '').trimEnd(),
    };
  }

  if (blockType === 'stylesheet') {
    return {
      id,
      className,
      type: 'stylesheet',
      href: String(block?.href || block?.src || '').trim(),
      media: String(block?.media || '').trim(),
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

function parseLeadingAttributesAndRemainder(text) {
  const attrs = {};
  let rest = String(text || '');

  while (true) {
    const match = /^\s*([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/.exec(rest);

    if (!match) {
      break;
    }

    const key = String(match[1]).trim().toLowerCase();
    const value = match[2] ?? match[3] ?? match[4];

    if (key) {
      attrs[key] = value;
    }

    rest = rest.slice(match[0].length);
  }

  return {
    attrs,
    remainder: rest.trim(),
  };
}

function unwrapSyntheticParagraphWrappers(sourceText) {
  const input = String(sourceText).replace(/\r\n/g, '\n').split('\n');
  const output = [];

  for (let i = 0; i < input.length; i += 1) {
    const line = String(input[i] || '');
    const trimmed = line.trim();
    const isSyntheticParagraphOpen = /^::paragraph\s+id=paragraph-\d+\s*$/i.test(trimmed);

    if (isSyntheticParagraphOpen && i + 2 < input.length) {
      const wrappedLine = String(input[i + 1]);
      const closeLine = String(input[i + 2]).trim();

      if (closeLine === '::end') {
        output.push(wrappedLine);
        i += 2;
        continue;
      }
    }

    output.push(line);
  }

  return output.join('\n');
}

function parseDocsrcBlocks(body) {
  const lines = unwrapSyntheticParagraphWrappers(body).replace(/^\n+/, '').split('\n');
  const blocks = [];

  function pushBlock(type, attributes, contentLines) {
    const content = contentLines.join('\n').trim();

    if (type === 'heading') {
      blocks.push({
        type: 'heading',
        level: clampHeadingLevel(attributes.level || 1),
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: content || 'Section',
      });
      return;
    }

    if (type === 'paragraph') {
      blocks.push({
        type: 'paragraph',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: content,
      });
      return;
    }

    if (type === 'quote') {
      blocks.push({
        type: 'quote',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: content,
      });
      return;
    }

    if (type === 'code') {
      blocks.push({
        type: 'code',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        language: attributes.lang || attributes.language || '',
        text: contentLines.join('\n').replace(/\n+$/, ''),
      });
      return;
    }

    if (type === 'bulleted-list' || type === 'list' || type === 'numbered-list') {
      const normalizedType = type === 'list' ? 'bulleted-list' : type;
      // Parse items with indentation to preserve nesting
      const itemsWithIndent = contentLines
        .map((line) => {
          // Match: optional indent, then list marker (-, *, or digit.), then text
          const bulletMatch = /^(\s*)[-*]\s+(.*)$/.exec(line);
          const numberedMatch = /^(\s*)\d+\.\s+(.*)$/.exec(line);
          const match = bulletMatch || numberedMatch;
          if (!match) {
            // Fallback: no list marker, treat as-is
            const fallback = /^(\s*)(.+)$/.exec(line);
            if (!fallback) return null;
            return { text: fallback[2].trim(), indent: fallback[1].length };
          }
          return { text: match[2].trim(), indent: match[1].length };
        })
        .filter(Boolean);
      
      const nested = buildNestedListStructure(itemsWithIndent);
      blocks.push({
        type: normalizedType,
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        items: nested,
      });
      return;
    }

    if (type === 'image') {
      blocks.push({
        type: 'image',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        src: String(attributes.src || '').trim(),
        alt: content,
      });
      return;
    }

    if (type === 'checklist') {
      const items = contentLines
        .map((line) => {
          const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(line.trim());
          if (match) {
            return { checked: match[1].toLowerCase() === 'x', text: match[2].trim() };
          }
          const trimmed = line.trim();
          return trimmed ? { checked: false, text: trimmed } : null;
        })
        .filter(Boolean)
        .filter((item) => item.text.length > 0);
      blocks.push({
        type: 'checklist',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        items,
      });
      return;
    }

    if (type === 'rule') {
      blocks.push({
        type: 'rule',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
      });
      return;
    }

    if (type === 'style') {
      blocks.push({
        type: 'style',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        text: contentLines.join('\n').trimEnd(),
      });
      return;
    }

    if (type === 'stylesheet') {
      blocks.push({
        type: 'stylesheet',
        id: attributes.id,
        className: normalizeClassName(attributes.class),
        href: String(attributes.href || attributes.src || contentLines.join('\n').trim() || '').trim(),
        media: String(attributes.media || '').trim(),
      });
      return;
    }
  }

  let cursor = 0;
  while (cursor < lines.length) {
    const rawLine = String(lines[cursor] || '');
    const line = rawLine.trim();

    if (!line) {
      cursor += 1;
      continue;
    }

    const inline = /^::([a-z-]+)(.*)\s+::end\s*$/i.exec(line);
    if (inline) {
      const type = inline[1].toLowerCase();
      if (type !== 'end') {
          const parsed = parseLeadingAttributesAndRemainder(inline[2]);
        const inlineContent = parsed.remainder ? [parsed.remainder] : [];
        pushBlock(type, parsed.attrs, inlineContent);
      }
      cursor += 1;
      continue;
    }

    const header = parseBlockHeader(line);
    if (!header) {
      cursor += 1;
      continue;
    }

    const type = header.type.toLowerCase();
    if (type === 'end') {
      cursor += 1;
      continue;
    }

    const parsedOpen = parseLeadingAttributesAndRemainder(rawLine.replace(/^::[a-z-]+/i, ''));
    const contentLines = [];

    if (parsedOpen.remainder) {
      contentLines.push(parsedOpen.remainder);
    }

    cursor += 1;
    let foundEnd = false;

    while (cursor < lines.length) {
      const bodyLine = String(lines[cursor]);
      const bodyTrimmed = bodyLine.trim();
      const endIndex = bodyLine.indexOf('::end');

      if (bodyTrimmed === '::end') {
        foundEnd = true;
        break;
      }

      if (endIndex !== -1) {
        const beforeEnd = bodyLine.slice(0, endIndex).trimEnd();
        if (beforeEnd) {
          contentLines.push(beforeEnd);
        }
        foundEnd = true;
        break;
      }

      contentLines.push(bodyLine);
      cursor += 1;
    }

    pushBlock(type, parsedOpen.attrs, contentLines);

    // Auto-close: if we didn't find ::end, that's okay—block is still valid
    // (unclosed blocks are auto-closed at EOF)
    if (cursor < lines.length && foundEnd) {
      cursor += 1;
    }
  }

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

function buildNestedListStructure(flatItems) {
  // Convert flat list with indent levels into nested structure
  // Each item is { text: string, indent: number }
  // Returns array of items with 'nested' field for children
  
  if (!flatItems || flatItems.length === 0) {
    return [];
  }

  const result = [];
  const stack = [];

  for (const item of flatItems) {
    const indent = item.indent || 0;
    
    // Pop items from stack until we find the correct parent level
    while (stack.length > 0 && stack[stack.length - 1].indent >= indent) {
      stack.pop();
    }

    const processed = { text: item.text };

    if (stack.length > 0) {
      // This item should be nested under the last item in stack
      const parent = stack[stack.length - 1];
      if (!parent.nested) {
        parent.nested = [];
      }
      parent.nested.push(processed);
    } else {
      // Top-level item
      result.push(processed);
    }

    stack.push(Object.assign({}, processed, { indent }));
  }

  return result;
}

function flushBufferedList(listState, blocks) {
  if (!listState.type || listState.items.length === 0) {
    listState.type = null;
    listState.items = [];
    return;
  }

  const nested = buildNestedListStructure(listState.items);
  blocks.push({
    type: listState.type,
    items: nested,
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
    const bulletListMatch = /^(\s*)[-*]\s+(.*)$/.exec(line);
    const numberedListMatch = /^(\s*)\d+\.\s+(.*)$/.exec(line);
    const quoteMatch = /^>\s?(.*)$/.exec(line);
    const codeFenceMatch = /^```(.*)$/.exec(line.trim());

    if (codeFence !== null) {
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

      const indent = bulletListMatch[1].length;
      listState.type = 'bulleted-list';
      listState.items.push({ text: bulletListMatch[2].trim(), indent });
      continue;
    }

    if (numberedListMatch) {
      flushBufferedParagraph(paragraphLines, blocks);
      flushBufferedQuote(quoteLines, blocks);

      if (listState.type && listState.type !== 'numbered-list') {
        flushBufferedList(listState, blocks);
      }

      const indent = numberedListMatch[1].length;
      listState.type = 'numbered-list';
      listState.items.push({ text: numberedListMatch[2].trim(), indent });
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

  if (codeFence !== null) {
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

  // Canonical DOC source is block syntax without required frontmatter header.
  if (/^::[a-z-]+(?:\s|$)/m.test(trimmed)) {
    return {
      title: '',
      summary: '',
      tags: [],
      meta: {},
      blocks: parseDocsrcBlocks(text),
      source: text,
    };
  }

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

  // Canonical storage format is block-only source (no metadata preamble).
  document.source = stringifyDocFile(document);

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

  if (block.type === 'stylesheet') {
    if (block.href) {
      attributes.push(`href=${formatAttributeValue(block.href)}`);
    }

    if (block.media) {
      attributes.push(`media=${formatAttributeValue(block.media)}`);
    }

    return `::stylesheet ${attributes.join(' ')}`;
  }

  return `::${block.type} ${attributes.join(' ')}`;
}

function blockBody(block) {
  if (block.type === 'bulleted-list' || block.type === 'numbered-list') {
    const toLines = (items, depth = 0) => {
      if (!Array.isArray(items)) {
        return [];
      }

      const lines = [];
      const indent = '  '.repeat(Math.max(0, depth));

      for (const item of items) {
        if (typeof item === 'object' && item !== null) {
          const text = String(item.text || '').trim();
          if (text) {
            lines.push(`${indent}- ${text}`);
          }

          if (Array.isArray(item.nested) && item.nested.length > 0) {
            lines.push(...toLines(item.nested, depth + 1));
          }

          continue;
        }

        const text = String(item || '').trim();
        if (text) {
          lines.push(`${indent}- ${text}`);
        }
      }

      return lines;
    };

    return toLines(block.items).join('\n');
  }

  if (block.type === 'image') {
    return block.alt || '';
  }

  if (block.type === 'style') {
    return block.text || '';
  }

  if (block.type === 'stylesheet' || block.type === 'rule') {
    return '';
  }

  return block.text || '';
}

export function stringifyDocFile(document) {
  const blockChunks = (document.blocks || []).map((block) => {
    const lines = [blockHeader(block), blockBody(block), '::end'];
    return lines.join('\n');
  });

  const body = blockChunks.join('\n\n');
  return `${body}\n`;
}
