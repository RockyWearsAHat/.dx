export const DEFAULT_BLOCK_AUTOCOMPLETE = [
  '::paragraph',
  '::heading level=1',
  '::bulleted-list',
  '::numbered-list',
  '::checklist',
  '::quote',
  '::code language=',
  '::image src=',
  '::rule',
  '::end',
];

export function getLineContextFromValue(value, cursor) {
  const text = String(value || '');
  const safeCursor = Math.max(0, Math.min(Number(cursor || 0), text.length));
  const lineStart = text.lastIndexOf('\n', Math.max(0, safeCursor - 1)) + 1;
  const lineEndIndex = text.indexOf('\n', safeCursor);
  const lineEnd = lineEndIndex === -1 ? text.length : lineEndIndex;
  const lineText = text.slice(lineStart, lineEnd);
  const beforeCursor = text.slice(lineStart, safeCursor);
  const indent = (lineText.match(/^\s*/) || [''])[0];

  return {
    value: text,
    cursor: safeCursor,
    lineStart,
    lineEnd,
    lineText,
    beforeCursor,
    indent,
  };
}

export function computeAutocompleteSuggestions(options) {
  const config = options || {};
  const forceOpen = Boolean(config.forceOpen);
  const context = config.context || getLineContextFromValue(config.value || '', config.cursor || 0);
  const blockAutocomplete = Array.isArray(config.blockAutocomplete)
    ? config.blockAutocomplete
    : DEFAULT_BLOCK_AUTOCOMPLETE;
  const knownIds = Array.isArray(config.knownIds) ? config.knownIds : [];
  const knownClasses = Array.isArray(config.knownClasses) ? config.knownClasses : [];
  const knownImageSources = Array.isArray(config.knownImageSources) ? config.knownImageSources : [];
  const knownBlockTypes = Array.isArray(config.knownBlockTypes) ? config.knownBlockTypes : [];
  const knownAttributeKeys = Array.isArray(config.knownAttributeKeys) ? config.knownAttributeKeys : [];
  const knownAttributeValuesByKey = config.knownAttributeValuesByKey && typeof config.knownAttributeValuesByKey === 'object'
    ? config.knownAttributeValuesByKey
    : {};

  const suggestions = [];
  let replaceStart = context.cursor;
  let replaceEnd = context.cursor;
  let typed = '';

  const commandMatch = /::[a-z-]*$/i.exec(context.beforeCursor);
  if (commandMatch) {
    typed = commandMatch[0];
    replaceStart = context.cursor - typed.length;
    replaceEnd = context.cursor;

    const blockCandidates = new Set([...blockAutocomplete, ...knownBlockTypes.map((type) => `::${type}`)]);

    for (const candidate of blockCandidates) {
      if (!candidate.startsWith(typed)) continue;
      suggestions.push({
        label: candidate,
        insertText: candidate,
        kind: 'block',
        detail: 'Block',
      });
    }

    return { suggestions, replaceStart, replaceEnd, typed };
  }

  const headerMatch = /^\s*::([a-z-]+)\b/i.exec(context.lineText);
  if (headerMatch) {
    const blockType = String(headerMatch[1] || '').toLowerCase();
    const before = context.beforeCursor;

    const valueMatch = /\b([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]*))$/i.exec(before);
    if (valueMatch) {
      const key = String(valueMatch[1] || '').toLowerCase();
      typed = valueMatch[2] ?? valueMatch[3] ?? valueMatch[4] ?? '';
      replaceStart = context.cursor - typed.length;

      const afterLine = context.value.slice(context.cursor, context.lineEnd);
      const right = /^([^\s"']*)/.exec(afterLine);
      replaceEnd = context.cursor + (right ? right[1].length : 0);

      const keyValues = Array.isArray(knownAttributeValuesByKey[key]) ? knownAttributeValuesByKey[key] : [];
      const candidates = new Set(keyValues);

      if (key === 'id') {
        for (const value of knownIds) candidates.add(value);
      } else if (key === 'class') {
        for (const value of knownClasses) candidates.add(value);
      } else if (key === 'src') {
        for (const value of knownImageSources) candidates.add(value);
      } else if (key === 'type') {
        for (const value of knownBlockTypes) candidates.add(value);
      }

      for (const candidate of candidates) {
        if (!candidate || !candidate.startsWith(typed) || candidate === typed) continue;
        suggestions.push({
          label: candidate,
          insertText: candidate,
          kind: 'attribute-value',
          detail: `${key} value`,
        });
      }

      return { suggestions, replaceStart, replaceEnd, typed };
    }

    const keyMatch = /(?:\s|::[a-z-]+\s+)([a-zA-Z0-9._-]*)$/i.exec(before);
    if (keyMatch) {
      typed = keyMatch[1] || '';
      replaceStart = context.cursor - typed.length;
      replaceEnd = context.cursor;

      const keys = new Set(knownAttributeKeys);
      if (blockType === 'heading') keys.add('level');
      if (blockType === 'code') keys.add('language');
      if (blockType === 'image') keys.add('src');
      if (blockType === 'stylesheet') {
        keys.add('href');
        keys.add('media');
      }
      keys.add('id');
      keys.add('class');

      for (const key of keys) {
        if (!key.startsWith(typed) || key === typed) continue;
        suggestions.push({
          label: `${key}=`,
          insertText: `${key}=`,
          kind: 'attribute-key',
          detail: `${blockType} attribute`,
        });
      }

      return { suggestions, replaceStart, replaceEnd, typed };
    }
  }

  const idMatch = /\bid=([^\s]*)$/i.exec(context.beforeCursor);
  if (idMatch) {
    typed = idMatch[1] || '';
    replaceStart = context.cursor - typed.length;
    const afterLine = context.value.slice(context.cursor, context.lineEnd);
    const right = /^([^\s]*)/.exec(afterLine);
    replaceEnd = context.cursor + (right ? right[1].length : 0);

    for (const id of knownIds) {
      if (!id.startsWith(typed)) continue;
      suggestions.push({
        label: id,
        insertText: id,
        kind: 'id',
        detail: 'Known id',
      });
    }

    return { suggestions, replaceStart, replaceEnd, typed };
  }

  const classMatch = /\bclass=(?:"([^"]*)"|'([^']*)'|([^\s]*))$/i.exec(context.beforeCursor);
  if (classMatch) {
    const classValue = classMatch[1] ?? classMatch[2] ?? classMatch[3] ?? '';
    const currentToken = classValue.split(/\s+/).pop() || '';
    typed = currentToken;
    replaceStart = context.cursor - currentToken.length;
    const afterClass = context.value.slice(context.cursor, context.lineEnd);
    const rightClass = /^([^\s"']*)/.exec(afterClass);
    replaceEnd = context.cursor + (rightClass ? rightClass[1].length : 0);

    for (const token of knownClasses) {
      if (!token.startsWith(currentToken) || token === currentToken) continue;
      suggestions.push({
        label: token,
        insertText: token,
        kind: 'class',
        detail: 'Known class',
      });
    }

    return { suggestions, replaceStart, replaceEnd, typed };
  }

  const imageSrcMatch = /::image\s+[^\n]*\bsrc=([^\s]*)$/i.exec(context.beforeCursor);
  if (imageSrcMatch) {
    typed = imageSrcMatch[1] || '';
    replaceStart = context.cursor - typed.length;
    const afterSrc = context.value.slice(context.cursor, context.lineEnd);
    const rightSrc = /^([^\s]*)/.exec(afterSrc);
    replaceEnd = context.cursor + (rightSrc ? rightSrc[1].length : 0);

    for (const source of knownImageSources) {
      if (!source.startsWith(typed)) continue;
      suggestions.push({
        label: source,
        insertText: source,
        kind: 'src',
        detail: 'Image source',
      });
    }

    return { suggestions, replaceStart, replaceEnd, typed };
  }

  if (forceOpen) {
    for (const candidate of blockAutocomplete) {
      suggestions.push({
        label: candidate,
        insertText: candidate,
        kind: 'block',
        detail: 'Block',
      });
    }
  }

  return { suggestions, replaceStart, replaceEnd, typed };
}

export function computeNextSelectedIndex(currentIndex, delta, count) {
  if (!Number.isFinite(count) || count <= 0) {
    return 0;
  }

  const normalizedCurrent = Number.isFinite(currentIndex) ? currentIndex : 0;
  const normalizedDelta = Number.isFinite(delta) ? delta : 0;
  return (normalizedCurrent + normalizedDelta + count) % count;
}
