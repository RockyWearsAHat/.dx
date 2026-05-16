export function normalizeClassName(value) {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

export function splitClassNames(value) {
  const className = normalizeClassName(value);
  return className ? className.split(/\s+/) : [];
}

export function parseAttributes(args) {
  const attributes = {};
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

export function isBulletedListType(type) {
  return type === 'list' || type === 'bulleted-list';
}

export function isNumberedListType(type) {
  return type === 'numbered-list';
}

export function parseListItems(lines) {
  return (lines || [])
    .map((line) => String(line || '').replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim())
    .filter(Boolean);
}

function parseLeadingAttributesAndRemainder(text) {
  const attrs = {};
  let rest = String(text || '');

  while (true) {
    const match = /^\s*([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/.exec(rest);
    if (!match) {
      break;
    }

    const key = String(match[1] || '').trim().toLowerCase();
    const value = match[2] ?? match[3] ?? match[4] ?? '';

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
  const input = String(sourceText || '').replace(/\r\n/g, '\n').split('\n');
  const output = [];

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

export function parseSourceBlocks(source) {
  const text = unwrapSyntheticParagraphWrappers(String(source || ''));
  const lines = text.split('\n');
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
    if (line.startsWith('::')) {
      break;
    }
    if (line === '' || /^[a-z._-]+\s*:/i.test(line)) {
      continue;
    }
    break;
  }

  while (cursor < lines.length) {
    const line = lines[cursor].trim();

    if (!line) {
      cursor += 1;
      continue;
    }

    if (!line.startsWith('::')) {
      blocks.push({ type: 'paragraph', text: lines[cursor], rawSource: lines[cursor], className: '', id: '' });
      cursor += 1;
      continue;
    }

    const inline = /^::([a-z-]+)(.*)\s+::end\s*$/i.exec(line);
    if (inline) {
      const type = inline[1].toLowerCase();
      const parsed = parseLeadingAttributesAndRemainder(inline[2] || '');
      const attrs = parsed.attrs;
      const inlineText = parsed.remainder;
      const rawSource = lines[cursor];

      if (type === 'rule') {
        blocks.push({
          type: 'rule',
          rawSource,
          id: String(attrs.id || '').trim(),
          className: normalizeClassName(attrs.class),
        });
        cursor += 1;
        continue;
      }

      if (type === 'heading') {
        const level = Number(attrs.level || 1);
        blocks.push({
          type,
          id: String(attrs.id || '').trim(),
          className: normalizeClassName(attrs.class),
          level: Math.min(6, Math.max(1, level)),
          text: inlineText,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (isBulletedListType(type)) {
        blocks.push({
          type: 'bulleted-list',
          id: String(attrs.id || '').trim(),
          className: normalizeClassName(attrs.class),
          items: parseListItems(inlineText ? [inlineText] : []),
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (isNumberedListType(type)) {
        blocks.push({
          type: 'numbered-list',
          id: String(attrs.id || '').trim(),
          className: normalizeClassName(attrs.class),
          items: parseListItems(inlineText ? [inlineText] : []),
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'checklist') {
        const checklistItem = inlineText.trim();
        const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(checklistItem);
        const items = checklistItem
          ? [match ? { checked: match[1].toLowerCase() === 'x', text: match[2] } : { checked: false, text: checklistItem }]
          : [];

        blocks.push({
          type,
          id: String(attrs.id || '').trim(),
          className: normalizeClassName(attrs.class),
          items,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      if (type === 'image') {
        blocks.push({
          type,
          id: String(attrs.id || '').trim(),
          className: normalizeClassName(attrs.class),
          src: String(attrs.src || '').trim(),
          alt: inlineText,
          rawSource,
        });
        cursor += 1;
        continue;
      }

      const block = {
        type,
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        text: type === 'paragraph' ? inlineText : inlineText.trimEnd(),
        rawSource,
      };

      if (type === 'code') {
        block.language = String(attrs.language || attrs.lang || '').trim();
      }

      if (type !== 'paragraph' || block.text.trim()) {
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
    const parsedOpen = parseLeadingAttributesAndRemainder(open[2] || '');
    const attrs = parsedOpen.attrs;
    const content = [];
    const blockStart = cursor;
    const openingLineContent = String(parsedOpen.remainder || '').trim();
    if (openingLineContent) {
      content.push(openingLineContent);
    }
    cursor += 1;

    if (type === 'rule') {
      blocks.push({
        type: 'rule',
        rawSource: lines[blockStart],
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
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
        type,
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        level: Math.min(6, Math.max(1, level)),
        text: content.join('\n').trim(),
        rawSource,
      });
      continue;
    }

    if (isBulletedListType(type)) {
      blocks.push({
        type: 'bulleted-list',
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        items: parseListItems(content),
        rawSource,
      });
      continue;
    }

    if (isNumberedListType(type)) {
      blocks.push({
        type: 'numbered-list',
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        items: parseListItems(content),
        rawSource,
      });
      continue;
    }

    if (type === 'checklist') {
      blocks.push({
        type,
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        items: content
          .map((itemLine) => {
            const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(itemLine.trim());
            return match ? { checked: match[1].toLowerCase() === 'x', text: match[2] } : { checked: false, text: itemLine.trim() };
          })
          .filter((item) => item.text.length > 0),
        rawSource,
      });
      continue;
    }

    if (type === 'image') {
      blocks.push({
        type,
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        src: String(attrs.src || '').trim(),
        alt: content.join('\n').trim(),
        rawSource,
      });
      continue;
    }

    const blockText = type === 'paragraph' ? content.join('\n') : content.join('\n').trimEnd();

    if (type === 'paragraph' && !blockText.trim()) {
      continue;
    }

    const block = {
      type,
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
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
