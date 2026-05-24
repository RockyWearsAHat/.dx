export interface ChecklistItem {
  checked: boolean;
  text: string;
}

export interface PipelineBlock {
  type: string;
  id: string;
  className: string;
  hidden?: boolean;
  rawSource?: string;
  text?: string;
  level?: number;
  items?: Array<string | ChecklistItem>;
  src?: string;
  alt?: string;
  href?: string;
  media?: string;
  language?: string;
}

type Attributes = Record<string, string>;

function parseBooleanAttribute(value: string | number | boolean | null | undefined | object): boolean {
  const normalized = String(value || '').trim().toLowerCase();
  return normalized === 'true' || normalized === '1' || normalized === 'yes' || normalized === 'on';
}

export function normalizeClassName(value: string | number | boolean | null | undefined | object): string {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

export function splitClassNames(value: string | number | boolean | null | undefined | object): string[] {
  const className = normalizeClassName(value);
  return className ? className.split(/\s+/) : [];
}

export function parseAttributes(args: string | number | boolean | null | undefined | object): Attributes {
  const attributes: Attributes = {};
  const pattern = /([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g;
  const text = String(args || '');
  let match = pattern.exec(text);

  while (match) {
    const key = String(match[1]).trim().toLowerCase();
    const value = (match[2] ?? match[3] ?? match[4]) as string;

    if (key) {
      attributes[key] = value;
    }

    match = pattern.exec(text);
  }

  return attributes;
}

export function isBulletedListType(type: string): boolean {
  return type === 'list' || type === 'bulleted-list';
}

export function isNumberedListType(type: string): boolean {
  return type === 'numbered-list';
}

export function parseListItems(lines: string[]): string[] {
  return (lines || [])
    .map((line) => String(line || '').replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim())
    .filter(Boolean);
}

function parseLeadingAttributesAndRemainder(text: string | number | boolean | null | undefined | object): { attrs: Attributes; remainder: string } {
  const attrs: Attributes = {};
  let rest = String(text || '');

  while (true) {
    const match = /^\s*([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/.exec(rest);
    if (!match) {
      break;
    }

    const key = String(match[1]).trim().toLowerCase();
    const value = (match[2] ?? match[3] ?? match[4]) as string;

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

function unwrapSyntheticParagraphWrappers(sourceText: string | number | boolean | null | undefined | object): string {
  const input = String(sourceText || '').replace(/\r\n/g, '\n').split('\n');
  const output: string[] = [];

  for (let i = 0; i < input.length; i += 1) {
    const line = String(input[i] || '');
    const trimmed = line.trim();
    const isSyntheticParagraphOpen = /^::paragraph\s+id=paragraph-\d+\s*$/i.test(trimmed);

    if (isSyntheticParagraphOpen && i + 2 < input.length) {
      const wrappedLine = String(input[i + 1] || '');
      const closeLine = String(input[i + 2] || '').trim();

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

function makeBaseBlock(type: string, attrs: Attributes): PipelineBlock {
  return {
    type,
    id: String(attrs.id || '').trim(),
    className: normalizeClassName(attrs.class),
    hidden: parseBooleanAttribute(attrs.hidden),
  };
}

function parseChecklistLine(itemLine: string): ChecklistItem {
  const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(itemLine.trim());
  if (match) {
    const checkedToken = match[1];
    const text = match[2];
    return { checked: checkedToken.toLowerCase() === 'x', text };
  }
  return { checked: false, text: itemLine.trim() };
}

export function parseSourceBlocks(source: string | number | boolean | null | undefined | object): PipelineBlock[] {
  const text = unwrapSyntheticParagraphWrappers(String(source || ''));
  const lines = text.split('\n');
  const blocks: PipelineBlock[] = [];
  let cursor = 0;

  const firstLine = lines[0];
  if (firstLine.startsWith('@doc')) {
    cursor = 1;
  }

  for (; cursor < lines.length; cursor += 1) {
    const line = lines[cursor].trim();
    if (line === '---') {
      cursor += 1;
      break;
    }
    if (line.startsWith('::')) {
      break;
    }
    if (line === '' || /^[a-z._-]+\s*:/i.test(line)) {
      continue;
    }
    break;
  }

  while (cursor < lines.length) {
    const currentLine = lines[cursor];
    const line = currentLine.trim();

    if (!line) {
      cursor += 1;
      continue;
    }

    if (!line.startsWith('::')) {
      blocks.push({ type: 'paragraph', text: currentLine, rawSource: currentLine, className: '', id: '' });
      cursor += 1;
      continue;
    }

    const inline = /^::([a-z-]+)(.*)\s+::end\s*$/i.exec(line);
    if (inline) {
      const type = inline[1].toLowerCase();
      const parsed = parseLeadingAttributesAndRemainder(inline[2]);
      const attrs = parsed.attrs;
      const inlineText = parsed.remainder;
      const rawSource = currentLine;

      if (type === 'rule') {
        blocks.push({
          ...makeBaseBlock('rule', attrs),
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'style') {
        blocks.push({
          ...makeBaseBlock('style', attrs),
          text: inlineText,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'stylesheet') {
        blocks.push({
          ...makeBaseBlock('stylesheet', attrs),
          href: String(attrs.href || attrs.src || inlineText || '').trim(),
          media: String(attrs.media || '').trim(),
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'heading') {
        const level = Number(attrs.level || 1);
        blocks.push({
          ...makeBaseBlock(type, attrs),
          level: Math.min(6, Math.max(1, level)),
          text: inlineText,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (isBulletedListType(type)) {
        blocks.push({
          ...makeBaseBlock('bulleted-list', attrs),
          items: parseListItems(inlineText ? [inlineText] : []),
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (isNumberedListType(type)) {
        blocks.push({
          ...makeBaseBlock('numbered-list', attrs),
          items: parseListItems(inlineText ? [inlineText] : []),
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'checklist') {
        const checklistItem = inlineText.trim();
        const items = checklistItem ? [parseChecklistLine(checklistItem)] : [];

        blocks.push({
          ...makeBaseBlock(type, attrs),
          items,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'image') {
        blocks.push({
          ...makeBaseBlock(type, attrs),
          src: String(attrs.src || '').trim(),
          alt: inlineText,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      const block: PipelineBlock = {
        ...makeBaseBlock(type, attrs),
        text: type === 'paragraph' ? inlineText : inlineText.trimEnd(),
        rawSource,
      };

      if (type === 'code') {
        block.language = String(attrs.language || attrs.lang || '').trim();
      }

      if (type !== 'paragraph' || String(block.text || '').trim()) {
        blocks.push(block);
      }

      cursor += 1;
      continue;
    }

    const open = /^::([a-z-]+)(.*)$/i.exec(line);

    if (!open) {
      cursor += 1;
      continue;
    }

    const type = open[1].toLowerCase();
    if (type === 'end') {
      cursor += 1;
      continue;
    }
    const parsedOpen = parseLeadingAttributesAndRemainder(open[2]);
    const attrs = parsedOpen.attrs;
    const content: string[] = [];
    const blockStart = cursor;
    const openingLineContent = String(parsedOpen.remainder || '').trim();
    if (openingLineContent) {
      content.push(openingLineContent);
    }
    cursor += 1;

    if (type === 'rule') {
      blocks.push({
        ...makeBaseBlock('rule', attrs),
        rawSource: lines[blockStart],
      });
      continue;
    }

    while (cursor < lines.length) {
      const rawLine = String(lines[cursor] || '');
      const trimmedLine = rawLine.trim();
      const endIndex = rawLine.indexOf('::end');

      if (trimmedLine === '::end') {
        break;
      }

      if (endIndex !== -1) {
        const beforeEnd = rawLine.slice(0, endIndex).trimEnd();
        if (beforeEnd) {
          content.push(beforeEnd);
        }
        break;
      }

      content.push(rawLine);
      cursor += 1;
    }

    const blockEnd = cursor < lines.length ? cursor : cursor - 1;

    if (cursor < lines.length) {
      cursor += 1;
    }

    const rawSource = lines.slice(blockStart, blockEnd + 1).join('\n');

    if (type === 'heading') {
      const level = Number(attrs.level || 1);
      blocks.push({
        ...makeBaseBlock(type, attrs),
        level: Math.min(6, Math.max(1, level)),
        text: content.join('\n').trim(),
        rawSource,
      });
      continue;
    }

    if (isBulletedListType(type)) {
      blocks.push({
        ...makeBaseBlock('bulleted-list', attrs),
        items: parseListItems(content),
        rawSource,
      });
      continue;
    }

    if (isNumberedListType(type)) {
      blocks.push({
        ...makeBaseBlock('numbered-list', attrs),
        items: parseListItems(content),
        rawSource,
      });
      continue;
    }

    if (type === 'checklist') {
      blocks.push({
        ...makeBaseBlock(type, attrs),
        items: content
          .map((itemLine) => parseChecklistLine(itemLine))
          .filter((item) => item.text.length > 0),
        rawSource,
      });
      continue;
    }

    if (type === 'image') {
      blocks.push({
        ...makeBaseBlock(type, attrs),
        src: String(attrs.src || '').trim(),
        alt: content.join('\n').trim(),
        rawSource,
      });
      continue;
    }

    if (type === 'style') {
      blocks.push({
        ...makeBaseBlock('style', attrs),
        text: content.join('\n').trimEnd(),
        rawSource,
      });
      continue;
    }

    if (type === 'stylesheet') {
      blocks.push({
        ...makeBaseBlock('stylesheet', attrs),
        href: String(attrs.href || attrs.src || content.join('\n').trim() || '').trim(),
        media: String(attrs.media || '').trim(),
        rawSource,
      });
      continue;
    }

    const blockText = type === 'paragraph' ? content.join('\n') : content.join('\n').trimEnd();

    if (type === 'paragraph' && !blockText.trim()) {
      continue;
    }

    const block: PipelineBlock = {
      ...makeBaseBlock(type, attrs),
      text: blockText,
      rawSource,
    };

    if (type === 'code') {
      block.language = String(attrs.language || attrs.lang || '').trim();
    }

    blocks.push(block);
  }

  return blocks;
}
