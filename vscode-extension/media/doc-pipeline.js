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

export function parseSourceBlocks(source) {
  const text = String(source || '').replace(/\r\n/g, '\n');
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

    const open = /^::([a-z-]+)(.*)$/i.exec(line);

    if (!open) {
      cursor += 1;
      continue;
    }

    const type = open[1].toLowerCase();
    const attrs = parseAttributes(open[2] || '');
    const content = [];
    const blockStart = cursor;
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

    while (cursor < lines.length && lines[cursor].trim() !== '::end') {
      content.push(lines[cursor]);
      cursor += 1;
    }

    const blockEnd = cursor < lines.length && lines[cursor].trim() === '::end' ? cursor : cursor - 1;

    if (cursor < lines.length && lines[cursor].trim() === '::end') {
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
