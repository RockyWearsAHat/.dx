const vscode = typeof acquireVsCodeApi === 'function' ? acquireVsCodeApi() : { postMessage: () => {} };

let docModel = null;
let currentTheme = 'auto';
let loadNoteEl = null;
let currentDocPath = 'unknown.dx';
const appearanceStorageKey = 'docdb.appearance.v1';
const customCssStoragePrefix = 'docdb.custom-css.v1:';
const editModeStorageKey = 'docdb.edit-mode.v1';
let currentAppearance = {
  paper: 'white',
  density: 'comfortable',
  scale: 100,
};
let chromeRevealTimer = null;
let customCssSheet = null;
let inlineCssSurfaceState = {
  source: null,
  selector: '',
  baseCssText: '',
};

let blankPagePointerDownHadOpenEditors = false;
let autosaveTimer = null;
let docSaveState = 'idle'; // idle | dirty | saving | saved | error
let lastSavedDoc = '';

function debouncedAutosave() {
  if (autosaveTimer) {
    clearTimeout(autosaveTimer);
  }
  
  if (docSaveState !== 'idle') {
    docSaveState = 'dirty';
  } else {
    docSaveState = 'dirty';
    setStatusPersistent('Unsaved changes', 'dirty');
  }
  
  autosaveTimer = setTimeout(() => {
    saveDocAuto();
  }, 200);
}

const BLOCK_AUTOCOMPLETE = [
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

let autocompleteState = {
  textarea: null,
  suggestions: [],
  selectedIndex: 0,
  replaceStart: 0,
  replaceEnd: 0,
  typed: '',
};

function setStatus(text) {
  const el = document.getElementById('status');
  if (!el) return;

  el.textContent = text;
  el.style.display = 'block';
  el.dataset.state = 'transient';

  setTimeout(() => {
    if (el.dataset.state === 'transient') {
      el.style.display = 'none';
    }
  }, 2600);
}

function setStatusPersistent(text, state = 'idle') {
  const el = document.getElementById('status');
  if (!el) return;

  el.textContent = text;
  el.style.display = 'block';
  el.dataset.state = state;
}

function clearStatusPersistent() {
  const el = document.getElementById('status');
  if (!el) return;
  el.dataset.state = 'transient';
  el.style.display = 'none';
}

function isTypingTarget(element) {
  if (!element) return false;
  const tag = String(element.tagName || '').toLowerCase();
  return tag === 'textarea' || tag === 'input' || tag === 'select' || element.isContentEditable;
}

function getEventElementTarget(event) {
  const target = event && event.target;
  if (target instanceof Element) {
    return target;
  }

  if (target && target.parentElement instanceof Element) {
    return target.parentElement;
  }

  return null;
}

function escapeHtml(value) {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function normalizeClassName(value) {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

function splitClassNames(value) {
  const className = normalizeClassName(value);
  return className ? className.split(/\s+/) : [];
}

function formatAttributeValue(value) {
  const text = String(value || '').trim();

  if (!text) {
    return '';
  }

  if (/^[^\s"'=]+$/.test(text)) {
    return text;
  }

  return '"' + text.replace(/"/g, '') + '"';
}

function getCustomCssStorageKey() {
  return customCssStoragePrefix + currentDocPath;
}

function ensureCustomCssSheet() {
  if (customCssSheet) {
    return customCssSheet;
  }

  customCssSheet = new CSSStyleSheet();
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, customCssSheet];
  return customCssSheet;
}

function applyCustomCss(cssText) {
  try {
    ensureCustomCssSheet().replaceSync(String(cssText || ''));
    return true;
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Invalid CSS';
    setStatus('CSS error: ' + message);
    return false;
  }
}

function loadCustomCss() {
  try {
    return window.localStorage.getItem(getCustomCssStorageKey()) || '';
  } catch {
    return '';
  }
}

function extractCssFromDocumentModel() {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return '';
  }

  const chunks = [];

  for (const block of docModel.blocks) {
    if (!block || block.type !== 'code') {
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

function refreshDocumentCss() {
  const documentCss = extractCssFromDocumentModel();
  const fallbackCss = loadCustomCss();
  applyCustomCss(documentCss || fallbackCss);
}

function collectKnownIds() {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const known = new Set();

  for (const block of docModel.blocks) {
    const raw = String(block && block.rawSource ? block.rawSource : '');
    const attrs = raw.match(/\bid=([^\s]+)/gi) || [];

    for (const attr of attrs) {
      const eq = attr.indexOf('=');
      if (eq !== -1) {
        const value = attr.slice(eq + 1).trim();
        if (value) {
          known.add(value);
        }
      }
    }
  }

  return Array.from(known).sort((a, b) => a.localeCompare(b));
}

function collectKnownImageSources() {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const known = new Set();

  for (const block of docModel.blocks) {
    if (!block || block.type !== 'image') continue;
    const src = String(block.src || '').trim();
    if (src) {
      known.add(src);
    }
  }

  return Array.from(known).sort((a, b) => a.localeCompare(b));
}

function collectKnownClasses() {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const known = new Set();

  for (const block of docModel.blocks) {
    for (const token of splitClassNames(block && block.className)) {
      known.add(token);
    }

    const raw = String(block && block.rawSource ? block.rawSource : '');
    const header = raw.split('\n')[0] || '';
    const attrs = parseAttributes(header.replace(/^::[a-z-]+\s*/i, ''));

    for (const token of splitClassNames(attrs.class)) {
      known.add(token);
    }
  }

  return Array.from(known).sort((a, b) => a.localeCompare(b));
}

function getLineContext(textarea) {
  const value = String(textarea.value || '');
  const cursor = Number(textarea.selectionStart || 0);
  const lineStart = value.lastIndexOf('\n', Math.max(0, cursor - 1)) + 1;
  const lineEndIndex = value.indexOf('\n', cursor);
  const lineEnd = lineEndIndex === -1 ? value.length : lineEndIndex;
  const lineText = value.slice(lineStart, lineEnd);
  const beforeCursor = value.slice(lineStart, cursor);
  const indent = (lineText.match(/^\s*/) || [''])[0];

  return {
    value,
    cursor,
    lineStart,
    lineEnd,
    lineText,
    beforeCursor,
    indent,
  };
}

function getAutocompleteSuggestions(textarea, forceOpen = false) {
  if (!textarea) {
    return { suggestions: [], replaceStart: 0, replaceEnd: 0, typed: '' };
  }

  const ctx = getLineContext(textarea);
  const suggestions = [];
  let replaceStart = ctx.cursor;
  let replaceEnd = ctx.cursor;
  let typed = '';

  const commandMatch = /::[a-z-]*$/i.exec(ctx.beforeCursor);
  if (commandMatch) {
    typed = commandMatch[0];
    replaceStart = ctx.cursor - typed.length;
    replaceEnd = ctx.cursor;

    for (const candidate of BLOCK_AUTOCOMPLETE) {
      if (!candidate.startsWith(typed)) continue;
      suggestions.push({
        label: candidate,
        insertText: candidate,
        kind: 'block',
        detail: 'Block',
      });
    }
  } else {
    const idMatch = /\bid=([^\s]*)$/i.exec(ctx.beforeCursor);
    if (idMatch) {
      typed = idMatch[1] || '';
      replaceStart = ctx.cursor - typed.length;
      const afterLine = ctx.value.slice(ctx.cursor, ctx.lineEnd);
      const right = /^([^\s]*)/.exec(afterLine);
      replaceEnd = ctx.cursor + (right ? right[1].length : 0);
      const knownIds = collectKnownIds();

      for (const id of knownIds) {
        if (id.startsWith(typed)) {
          suggestions.push({
            label: id,
            insertText: id,
            kind: 'id',
            detail: 'Known id',
          });
        }
      }
    } else {
      const classMatch = /\bclass=(?:"([^"]*)|'([^']*)'|([^\s]*))$/i.exec(ctx.beforeCursor);
      if (classMatch) {
        const classValue = classMatch[1] ?? classMatch[2] ?? classMatch[3] ?? '';
        const currentToken = classValue.split(/\s+/).pop() || '';
        typed = currentToken;
        replaceStart = ctx.cursor - currentToken.length;
        const afterClass = ctx.value.slice(ctx.cursor, ctx.lineEnd);
        const rightClass = /^([^\s"']*)/.exec(afterClass);
        replaceEnd = ctx.cursor + (rightClass ? rightClass[1].length : 0);

        for (const token of collectKnownClasses()) {
          if (!token.startsWith(currentToken) || token === currentToken) continue;
          suggestions.push({
            label: token,
            insertText: token,
            kind: 'class',
            detail: 'Known class',
          });
        }
      } else {
      const imageSrcMatch = /::image\s+[^\n]*\bsrc=([^\s]*)$/i.exec(ctx.beforeCursor);
      if (imageSrcMatch) {
        typed = imageSrcMatch[1] || '';
        replaceStart = ctx.cursor - typed.length;
        const afterSrc = ctx.value.slice(ctx.cursor, ctx.lineEnd);
        const rightSrc = /^([^\s]*)/.exec(afterSrc);
        replaceEnd = ctx.cursor + (rightSrc ? rightSrc[1].length : 0);
        const knownSources = collectKnownImageSources();

        for (const source of knownSources) {
          if (source.startsWith(typed)) {
            suggestions.push({
              label: source,
              insertText: source,
              kind: 'src',
              detail: 'Image source',
            });
          }
        }
      } else if (forceOpen) {
        for (const candidate of BLOCK_AUTOCOMPLETE) {
          suggestions.push({
            label: candidate,
            insertText: candidate,
            kind: 'block',
            detail: 'Block',
          });
        }
      }
      }
    }
  }

  return { suggestions, replaceStart, replaceEnd, typed };
}

function getAutocompleteEls(textarea) {
  const srcWrap = textarea ? textarea.closest('.block-src-wrapper') : null;
  if (!srcWrap) return { menuEl: null, mirrorEl: null };
  return {
    menuEl: srcWrap.querySelector('.autocomplete-menu'),
    mirrorEl: srcWrap.querySelector('.block-src-mirror'),
  };
}

function renderGhostText(textarea, mirrorEl) {
  if (!mirrorEl || !autocompleteState.suggestions.length) return;
  const { suggestions, selectedIndex, replaceStart, replaceEnd, typed } = autocompleteState;
  const selected = suggestions[selectedIndex];
  const value = String(textarea.value || '');
  const ghostSuffix = selected.insertText.startsWith(typed)
    ? selected.insertText.slice(typed.length)
    : '';

  mirrorEl.textContent = '';
  mirrorEl.appendChild(document.createTextNode(value.slice(0, replaceStart) + typed));
  if (ghostSuffix) {
    const span = document.createElement('span');
    span.className = 'ghost-suffix';
    span.textContent = ghostSuffix;
    mirrorEl.appendChild(span);
  }
  mirrorEl.appendChild(document.createTextNode(value.slice(replaceEnd)));
  mirrorEl.scrollTop = textarea.scrollTop;

  const srcWrap = textarea.closest('.block-src-wrapper');
  if (srcWrap) srcWrap.classList.add('ghost-active');
  textarea.classList.add('ghost-active');
}

function closeAutocomplete() {
  const textarea = autocompleteState.textarea;
  if (!textarea) return;

  const { menuEl, mirrorEl } = getAutocompleteEls(textarea);
  const srcWrap = textarea.closest('.block-src-wrapper');

  if (menuEl) {
    menuEl.innerHTML = '';
    menuEl.style.display = 'none';
  }
  if (mirrorEl) mirrorEl.textContent = '';
  if (srcWrap) srcWrap.classList.remove('ghost-active');
  textarea.classList.remove('ghost-active');

  autocompleteState = {
    textarea: null,
    suggestions: [],
    selectedIndex: 0,
    replaceStart: 0,
    replaceEnd: 0,
    typed: '',
  };
}

function renderAutocomplete(textarea, forceOpen = false) {
  if (!textarea) {
    closeAutocomplete();
    return false;
  }

  const { menuEl, mirrorEl } = getAutocompleteEls(textarea);
  if (!menuEl) {
    closeAutocomplete();
    return false;
  }

  const model = getAutocompleteSuggestions(textarea, forceOpen);
  const suggestions = model.suggestions.slice(0, 20);

  autocompleteState.textarea = textarea;
  autocompleteState.suggestions = suggestions;
  autocompleteState.selectedIndex = 0;
  autocompleteState.replaceStart = model.replaceStart;
  autocompleteState.replaceEnd = model.replaceEnd;
  autocompleteState.typed = model.typed;

  if (suggestions.length === 0) {
    closeAutocomplete();
    return false;
  }

  renderGhostText(textarea, mirrorEl);

  menuEl.innerHTML = suggestions
    .map((item, index) => {
      const cls = index === 0 ? ' selected' : '';
      return '<button type="button" class="autocomplete-item' + cls + '" data-index="' + index + '"><span class="autocomplete-item-label">' + escapeHtml(item.label) + '</span><span class="autocomplete-item-detail">' + escapeHtml(item.detail) + '</span></button>';
    })
    .join('');
  menuEl.style.display = 'block';
  requestAnimationFrame(() => {
    positionAutocompleteMenu(textarea, menuEl);
  });

  return true;
}

function positionAutocompleteMenu(textarea, menuEl) {
  if (!textarea || !menuEl) return;

  const computed = window.getComputedStyle(textarea);
  const lineHeight = Number.parseFloat(computed.lineHeight) || 20;
  const paddingTop = Number.parseFloat(computed.paddingTop) || 0;
  const paddingLeft = Number.parseFloat(computed.paddingLeft) || 0;
  const beforeCursor = String(textarea.value || '').slice(0, textarea.selectionStart || 0);
  const currentLine = beforeCursor.slice(beforeCursor.lastIndexOf('\n') + 1);
  const lineIndex = beforeCursor.split('\n').length - 1;
  const canvas = positionAutocompleteMenu.canvas || (positionAutocompleteMenu.canvas = document.createElement('canvas'));
  const context = canvas.getContext('2d');

  if (!context) {
    return;
  }

  context.font = `${computed.fontWeight} ${computed.fontSize} ${computed.fontFamily}`;
  const textWidth = context.measureText(currentLine).width;
  const srcWrap = textarea.closest('.block-src-wrapper');
  const wrapWidth = srcWrap ? srcWrap.clientWidth : textarea.clientWidth;
  const maxLeft = Math.max(0, wrapWidth - menuEl.offsetWidth);
  const left = Math.max(0, Math.min(maxLeft, paddingLeft + textWidth - textarea.scrollLeft));
  const top = Math.max(0, paddingTop + ((lineIndex + 1) * lineHeight) - textarea.scrollTop + 6);

  menuEl.style.left = `${left}px`;
  menuEl.style.top = `${top}px`;
}

function updateAutocompleteSelection(delta) {
  if (!autocompleteState.textarea || autocompleteState.suggestions.length === 0) {
    return false;
  }

  const count = autocompleteState.suggestions.length;
  autocompleteState.selectedIndex = (autocompleteState.selectedIndex + delta + count) % count;

  const textarea = autocompleteState.textarea;
  const { menuEl, mirrorEl } = getAutocompleteEls(textarea);

  renderGhostText(textarea, mirrorEl);

  if (menuEl) {
    const items = menuEl.querySelectorAll('.autocomplete-item');
    items.forEach((item, index) => {
      item.classList.toggle('selected', index === autocompleteState.selectedIndex);
    });

    const activeItem = menuEl.querySelector('.autocomplete-item.selected');
    if (activeItem) {
      activeItem.scrollIntoView({ block: 'nearest' });
    }
  }

  return true;
}

function acceptAutocomplete(textarea, explicitIndex) {
  if (!textarea || autocompleteState.textarea !== textarea || autocompleteState.suggestions.length === 0) {
    return false;
  }

  const index = Number.isInteger(explicitIndex)
    ? Math.max(0, Math.min(explicitIndex, autocompleteState.suggestions.length - 1))
    : autocompleteState.selectedIndex;
  const choice = autocompleteState.suggestions[index];
  if (!choice) {
    return false;
  }

  const value = String(textarea.value || '');
  const insertText = String(choice.insertText || '');
  const before = value.slice(0, autocompleteState.replaceStart);
  const after = value.slice(autocompleteState.replaceEnd);
  textarea.value = before + insertText + after;

  const nextCursor = autocompleteState.replaceStart + insertText.length;
  textarea.setSelectionRange(nextCursor, nextCursor);
  textarea.dispatchEvent(new Event('input', { bubbles: true }));
  closeAutocomplete();
  return true;
}

function parseAttributes(args) {
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

const KNOWN_BLOCK_TYPES = new Set([
  'paragraph', 'heading', 'bulleted-list', 'numbered-list', 'list',
  'checklist', 'quote', 'code', 'image', 'rule',
]);

function blockHasContent(block) {
  if (!block) return false;
  if (block.type === 'rule') return true;
  if (block.type === 'image') return String(block.src || '').trim().length > 0;
  if (Array.isArray(block.items)) return block.items.length > 0;
  return String(block.text || '').trim().length > 0;
}

function isBulletedListType(type) {
  return type === 'list' || type === 'bulleted-list';
}

function isNumberedListType(type) {
  return type === 'numbered-list';
}

function parseListItems(lines) {
  return (lines || [])
    .map((line) => String(line || '').replace(/^\s*(?:[-*]|\d+[.)])\s+/, '').trim())
    .filter(Boolean);
}

function buildBlockHeader(type, attributes) {
  const parts = Object.entries(attributes || {})
    .filter(([, value]) => String(value || '').trim())
    .map(([key, value]) => `${key}=${formatAttributeValue(value)}`);

  return parts.length > 0 ? `::${type} ${parts.join(' ')}` : `::${type}`;
}

function stringifyBlock(block) {
  if (!block) return '';

  if (block.rawSource) {
    return String(block.rawSource).trimEnd();
  }

  const attributes = {};

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
    return buildBlockHeader('bulleted-list', attributes) + '\n' + (block.items || []).join('\n') + '\n::end';
  }

  if (isNumberedListType(block.type)) {
    return buildBlockHeader('numbered-list', attributes) + '\n' + (block.items || []).join('\n') + '\n::end';
  }

  if (block.type === 'checklist') {
    return buildBlockHeader('checklist', attributes) + '\n' + (block.items || []).map((item) => '[' + (item.checked ? 'x' : ' ') + '] ' + item.text).join('\n') + '\n::end';
  }

  if (block.type === 'image') {
    const src = String(block.src || '').trim();
    const alt = String(block.alt || '').trim();
    return buildBlockHeader('image', {
      ...attributes,
      src,
    }) + '\n' + alt + '\n::end';
  }

  if (block.type === 'rule') {
    return buildBlockHeader('rule', attributes);
  }

  return buildBlockHeader(block.type, attributes) + '\n' + (block.text || '') + '\n::end';
}

function parseBlock(raw) {
  const original = String(raw || '').replace(/\r\n/g, '\n');
  const trimmed = original.trim();

  if (!trimmed.startsWith('::')) {
    return { type: 'paragraph', text: original, rawSource: original };
  }

  const lines = trimmed.split('\n');
  const open = /^::([a-z-]+)(.*)$/i.exec(lines[0].trim());

  if (!open) {
    return { type: 'paragraph', text: trimmed, rawSource: original };
  }

  const type = open[1].toLowerCase();
  const attrs = parseAttributes(open[2] || '');
  let endIdx = lines.length;

  for (let i = 1; i < lines.length; i += 1) {
    if (lines[i].trim() === '::end') {
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
    return {
      type: 'bulleted-list',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      items: parseListItems(content),
      rawSource: original,
    };
  }

  if (isNumberedListType(type)) {
    return {
      type: 'numbered-list',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      items: parseListItems(content),
      rawSource: original,
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
          return match ? { checked: match[1].toLowerCase() === 'x', text: match[2] } : { checked: false, text: line.trim() };
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

  if (type === 'rule') {
    return {
      type: 'rule',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      rawSource: original,
    };
  }

  if (type === 'code') {
    return {
      type: 'code',
      id: String(attrs.id || '').trim(),
      className: normalizeClassName(attrs.class),
      language: String(attrs.language || '').trim(),
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

function parseDoc(source) {
  const text = String(source || '').replace(/\r\n/g, '\n');
  const lines = text.split('\n');
  const blocks = [];
  let cursor = 0;

  // Skip @doc line if present (for backwards compat with old format)
  if (lines[0] && lines[0].startsWith('@doc')) {
    cursor = 1;
  }

  // Skip old metadata section (YAML-style key:value or --- separator)
  // Any line starting with word: or --- is metadata and should be skipped
  // But stop if we see a block (::) or plain prose (non-metadata, non-empty)
  for (; cursor < lines.length; cursor += 1) {
    const line = lines[cursor].trim();
    if (line === '---') {
      cursor += 1;
      break;
    }
    // If we hit a block before ---, it's a new-format doc with no metadata
    if (line.startsWith('::')) {
      break;
    }
    // Skip empty lines and YAML-style metadata keys (word: value, including dotted like meta.status:)
    if (line === '' || /^[a-z._-]+\s*:/i.test(line)) {
      continue;
    }
    // Anything else is content, not metadata — stop skipping
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

    // Self-closing: no content body or ::end needed
    if (type === 'rule') {
      blocks.push({ type: 'rule', rawSource: lines[blockStart], id: String(attrs.id || '').trim(), className: normalizeClassName(attrs.class) });
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
      blocks.push({ type, id: String(attrs.id || '').trim(), className: normalizeClassName(attrs.class), level: Math.min(6, Math.max(1, level)), text: content.join('\n').trim(), rawSource });
    } else if (isBulletedListType(type)) {
      blocks.push({ type: 'bulleted-list', id: String(attrs.id || '').trim(), className: normalizeClassName(attrs.class), items: parseListItems(content), rawSource });
    } else if (isNumberedListType(type)) {
      blocks.push({ type: 'numbered-list', id: String(attrs.id || '').trim(), className: normalizeClassName(attrs.class), items: parseListItems(content), rawSource });
    } else if (type === 'checklist') {
      blocks.push({
        type,
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        items: content
          .map((i) => {
            const match = /^\s*\[(x| )\]\s*(.*)$/i.exec(i.trim());
            return match ? { checked: match[1].toLowerCase() === 'x', text: match[2] } : { checked: false, text: i.trim() };
          })
          .filter((item) => item.text.length > 0),
        rawSource,
      });
    } else if (type === 'image') {
      blocks.push({
        type,
        id: String(attrs.id || '').trim(),
        className: normalizeClassName(attrs.class),
        src: String(attrs.src || '').trim(),
        alt: content.join('\n').trim(),
        rawSource,
      });
    } else {
      const text = type === 'paragraph' ? content.join('\n') : content.join('\n').trimEnd();
      if (!(type === 'paragraph' && !text.trim())) {
        const block = { type, id: String(attrs.id || '').trim(), className: normalizeClassName(attrs.class), text, rawSource };

        if (type === 'code') {
          block.language = String(attrs.language || '').trim();
        }

        blocks.push(block);
      }
    }
  }

  return { blocks };
}

function stringifyDoc(model) {
  const lines = [];

  for (let i = 0; i < model.blocks.length; i += 1) {
    lines.push(stringifyBlock(model.blocks[i]));

    if (i < model.blocks.length - 1) {
      lines.push('');
    }
  }

  return lines.join('\n').trimEnd() + '\n';
}

function buildRenderedContent(block) {
  function applyBlockDecorations(element) {
    if (!element) return element;
    if (block.id) {
      element.id = block.id;
      element.dataset.blockId = block.id;
    }
    for (const token of splitClassNames(block.className)) {
      element.classList.add(token);
    }
    return element;
  }

  if (block.type === 'heading') {
    const level = Math.min(6, Math.max(1, Number(block.level || 1)));
    const heading = document.createElement('h' + level);
    heading.textContent = block.text || '';
    return applyBlockDecorations(heading);
  }

  if (block.type === 'paragraph') {
    const paragraph = document.createElement('p');
    paragraph.textContent = block.text || '';
    return applyBlockDecorations(paragraph);
  }

  if (block.type === 'image') {
    const figure = document.createElement('figure');
    figure.className = 'image-wrap';

    const image = document.createElement('img');
    image.src = block.src || '';
    image.alt = block.alt || '';
    image.loading = 'lazy';
    figure.appendChild(image);

    if (block.alt) {
      const figcaption = document.createElement('figcaption');
      figcaption.textContent = block.alt;
      figure.appendChild(figcaption);
    }

    return applyBlockDecorations(figure);
  }

  if (block.type === 'code') {
    const pre = document.createElement('pre');
    pre.textContent = block.text || '';
    return applyBlockDecorations(pre);
  }

  if (block.type === 'rule') {
    return applyBlockDecorations(document.createElement('hr'));
  }

  if (block.type === 'quote') {
    const quote = document.createElement('blockquote');
    const paragraph = document.createElement('p');
    paragraph.textContent = block.text || '';
    quote.appendChild(paragraph);
    return applyBlockDecorations(quote);
  }

  if (isBulletedListType(block.type)) {
    const ul = document.createElement('ul');
    (block.items || []).forEach((item) => {
      const li = document.createElement('li');
      li.textContent = item;
      ul.appendChild(li);
    });
    return applyBlockDecorations(ul);
  }

  if (isNumberedListType(block.type)) {
    const ol = document.createElement('ol');
    (block.items || []).forEach((item) => {
      const li = document.createElement('li');
      li.textContent = item;
      ol.appendChild(li);
    });
    return applyBlockDecorations(ol);
  }

  if (block.type === 'checklist') {
    const ul = document.createElement('ul');
    ul.className = 'checklist-wrap';
    (block.items || []).forEach((item) => {
      const li = document.createElement('li');
      const checkbox = document.createElement('input');
      checkbox.type = 'checkbox';
      checkbox.checked = Boolean(item.checked);
      checkbox.disabled = true;
      const span = document.createElement('span');
      span.textContent = item.text;
      if (item.checked) {
        span.className = 'check-done';
      }
      li.appendChild(checkbox);
      li.appendChild(span);
      ul.appendChild(li);
    });
    return applyBlockDecorations(ul);
  }

  const fallback = document.createElement('p');
  fallback.textContent = block.text || '';
  return applyBlockDecorations(fallback);
}

function buildBlockWrap(block, index) {
  const wrap = document.createElement('div');
  wrap.className = 'block-wrap';
  wrap.dataset.blockIndex = String(index);

  const view = document.createElement('div');
  view.className = 'block-view';
  view.setAttribute('role', 'article');
  view.setAttribute('tabindex', '0');
  view.setAttribute('aria-label', `Block ${index + 1}: ${block.type}`);
  view.appendChild(buildRenderedContent(block));

  const srcWrap = document.createElement('div');
  srcWrap.className = 'block-src-wrapper';
  srcWrap.style.display = 'none';

  const mirror = document.createElement('div');
  mirror.className = 'block-src-mirror';
  mirror.setAttribute('aria-hidden', 'true');

  const source = document.createElement('textarea');
  source.className = 'block-src';
  source.setAttribute('aria-label', 'Edit block source');
  source.spellcheck = false;
  source.wrap = 'off';

  const menu = document.createElement('div');
  menu.className = 'autocomplete-menu';
  menu.setAttribute('role', 'listbox');
  menu.setAttribute('aria-label', 'Autocomplete suggestions');
  menu.style.display = 'none';

  srcWrap.appendChild(mirror);
  srcWrap.appendChild(source);
  srcWrap.appendChild(menu);

  wrap.appendChild(view);
  wrap.appendChild(srcWrap);
  return wrap;
}

function autosizeBlockSrc(textarea) {
  if (!textarea) return;

  textarea.style.height = '0px';
  textarea.style.height = textarea.scrollHeight + 'px';
}

function getRawSourceForEditor(rawSource) {
  return String(rawSource || '').replace(/\r\n/g, '\n');
}

function getRawSourceFromEditor(editorValue) {
  return String(editorValue || '').replace(/\r\n/g, '\n');
}

function findAttributeTargetAtCursor(textarea) {
  const ctx = getLineContext(textarea);
  const cursorInLine = ctx.cursor - ctx.lineStart;
  const pattern = /([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g;
  let match = pattern.exec(ctx.lineText);

  while (match) {
    const key = String(match[1] || '').toLowerCase();
    const value = match[2] ?? match[3] ?? match[4] ?? '';
    const quoted = match[2] != null || match[3] != null;
    const valueStart = match.index + match[1].length + 1 + (quoted ? 1 : 0);
    const valueEnd = valueStart + value.length;

    if (cursorInLine >= valueStart && cursorInLine <= valueEnd) {
      if (key === 'id' && value.trim()) {
        return { selector: '#' + value.trim(), kind: 'id' };
      }

      if (key === 'class' && value.trim()) {
        const offset = Math.max(0, cursorInLine - valueStart);
        const tokenPattern = /\S+/g;
        let tokenMatch = tokenPattern.exec(value);

        while (tokenMatch) {
          const tokenStart = tokenMatch.index;
          const tokenEnd = tokenStart + tokenMatch[0].length;

          if (offset >= tokenStart && offset <= tokenEnd) {
            return { selector: '.' + tokenMatch[0], kind: 'class' };
          }

          tokenMatch = tokenPattern.exec(value);
        }
      }
    }

    match = pattern.exec(ctx.lineText);
  }

  return null;
}

function findCssBlockIndex() {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return -1;
  }

  for (let i = 0; i < docModel.blocks.length; i += 1) {
    const block = docModel.blocks[i];
    if (!block || block.type !== 'code') continue;
    const language = String(block.language || '').trim().toLowerCase();
    if (language === 'css' || language === 'stylesheet') {
      return i;
    }
  }

  return -1;
}

function splitRuleSelectors(selectorText) {
  return String(selectorText || '')
    .split(',')
    .map((token) => token.trim())
    .filter(Boolean);
}

function isSelectorForTarget(selector, target) {
  const normalizedSelector = String(selector || '').trim();
  const normalizedTarget = String(target || '').trim();

  if (!normalizedSelector || !normalizedTarget) {
    return false;
  }

  return normalizedSelector === normalizedTarget
    || normalizedSelector.startsWith(normalizedTarget + ':')
    || normalizedSelector.startsWith(normalizedTarget + '::');
}

function getScopedCssForSelector(cssText, selector) {
  const source = String(cssText || '');
  const target = String(selector || '').trim();
  if (!target) {
    return '';
  }

  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  const chunks = [];
  let match = pattern.exec(source);

  while (match) {
    const selectors = splitRuleSelectors(match[1]);
    const scopedSelectors = selectors.filter((candidate) => isSelectorForTarget(candidate, target));

    if (scopedSelectors.length > 0) {
      chunks.push(`${scopedSelectors.join(', ')} {\n${String(match[2] || '').trim()}\n}`);
    }

    match = pattern.exec(source);
  }

  return chunks.join('\n\n').trim();
}

function getScopedCssDeclarations(cssText, selector) {
  const source = String(cssText || '');
  const target = String(selector || '').trim();
  if (!target) {
    return '';
  }

  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  let match = pattern.exec(source);

  while (match) {
    const selectors = splitRuleSelectors(match[1]);
    if (selectors.some((candidate) => candidate === target)) {
      return String(match[2] || '')
        .replace(/^\n+|\n+$/g, '')
        .split('\n')
        .map((line) => line.replace(/^\s{2}/, ''))
        .join('\n')
        .trimEnd();
    }

    match = pattern.exec(source);
  }

  return '';
}

function buildScopedRule(selector, declarationsText) {
  const target = String(selector || '').trim();
  if (!target) {
    return '';
  }

  const raw = String(declarationsText || '')
    .replace(/\r\n/g, '\n')
    .split('\n')
    .map((line) => line.replace(/\s+$/g, ''));
  const hasContent = raw.some((line) => line.trim().length > 0);

  if (!hasContent) {
    return '';
  }

  const body = raw
    .map((line) => (line.trim().length > 0 ? `  ${line.trim()}` : ''))
    .join('\n');

  return `${target} {\n${body}\n}`;
}

function mergeScopedCssForSelector(baseCssText, selector, declarationsText) {
  const base = String(baseCssText || '');
  const target = String(selector || '').trim();
  const scoped = buildScopedRule(target, declarationsText);

  if (!target) {
    return base;
  }

  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  let rebuilt = '';
  let lastIndex = 0;
  let match = pattern.exec(base);

  while (match) {
    const start = match.index;
    const end = pattern.lastIndex;
    rebuilt += base.slice(lastIndex, start);

    const selectors = splitRuleSelectors(match[1]);
    const remaining = selectors.filter((candidate) => !isSelectorForTarget(candidate, target));

    if (remaining.length > 0) {
      rebuilt += `${remaining.join(', ')} {${match[2]}}`;
    }

    lastIndex = end;
    match = pattern.exec(base);
  }

  rebuilt += base.slice(lastIndex);

  const compactBase = rebuilt.trim();
  if (!scoped) {
    return compactBase;
  }

  return compactBase ? `${compactBase}\n\n${scoped}` : scoped;
}

function upsertCssBlock(cssText) {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return;
  }

  const text = String(cssText || '').trim();
  let cssIndex = findCssBlockIndex();

  if (cssIndex < 0) {
    const block = {
      type: 'code',
      language: 'css',
      text,
      id: '',
      className: '',
      rawSource: '',
    };
    block.rawSource = stringifyBlock(block);
    docModel.blocks.push(block);
    return;
  }

  const block = docModel.blocks[cssIndex];
  block.language = 'css';
  block.text = text;
  block.rawSource = stringifyBlock({ ...block, rawSource: '' });
}

function ensureInlineCssSurface(source) {
  const srcWrap = source ? source.closest('.block-src-wrapper') : null;
  if (!srcWrap) {
    return null;
  }

  const sourceArea = srcWrap.querySelector('.block-src');
  if (!sourceArea) {
    return null;
  }

  let surface = srcWrap.querySelector('.inline-css-surface');

  if (surface) {
    if (surface.nextElementSibling !== sourceArea) {
      srcWrap.insertBefore(surface, sourceArea);
    }
    return surface;
  }

  surface = document.createElement('div');
  surface.className = 'inline-css-surface';
  surface.style.display = 'none';
  surface.innerHTML = '<div class="inline-css-meta inline-css-head" aria-hidden="true"></div><textarea class="inline-css-src" spellcheck="false" aria-label="Inline CSS declarations"></textarea><div class="inline-css-meta inline-css-tail" aria-hidden="true">}</div>';

  const editor = surface.querySelector('.inline-css-src');
  if (editor) {
    editor.addEventListener('input', () => {
      const cssText = mergeScopedCssForSelector(
        inlineCssSurfaceState.baseCssText,
        inlineCssSurfaceState.selector,
        editor.value,
      );
      autosizeInlineCssEditor(editor);
      if (applyCustomCss(cssText)) {
        upsertCssBlock(cssText);
        inlineCssSurfaceState.baseCssText = cssText;
      }
    });
  }

  srcWrap.insertBefore(surface, sourceArea);

  return surface;
}

function autosizeInlineCssEditor(editor) {
  if (!editor) return;

  const lineHeight = Number.parseFloat(window.getComputedStyle(editor).lineHeight) || 18;
  const minLines = 2;
  const maxLines = 16;
  const maxHeight = Math.round(lineHeight * maxLines);

  editor.style.height = '0px';
  const desiredHeight = Math.max(lineHeight * minLines, editor.scrollHeight);
  const nextHeight = Math.min(desiredHeight, maxHeight);

  editor.style.height = nextHeight + 'px';
  editor.style.maxHeight = maxHeight + 'px';
  editor.style.overflowY = desiredHeight > maxHeight ? 'auto' : 'hidden';
}

function closeInlineCssSurface() {
  const activeSource = inlineCssSurfaceState.source;
  const srcWrap = activeSource ? activeSource.closest('.block-src-wrapper') : null;
  const surface = srcWrap
    ? srcWrap.querySelector('.inline-css-surface')
    : document.querySelector('.block-src-wrapper .inline-css-surface');

  if (surface) {
    surface.style.display = 'none';
  }

  inlineCssSurfaceState = {
    source: null,
    selector: '',
    baseCssText: '',
  };
}

function openInlineCssSurface(source, selector) {
  if (!source) return;

  const surface = ensureInlineCssSurface(source);
  if (!surface) return;

  const editor = surface.querySelector('.inline-css-src');
  if (!editor) return;

  const requestedSelector = String(selector || '').trim();
  const alreadyOpen = inlineCssSurfaceState.source === source
    && inlineCssSurfaceState.selector === requestedSelector
    && surface.style.display !== 'none';

  if (alreadyOpen) {
    editor.focus();
    return;
  }

  const meta = surface.querySelector('.inline-css-meta');
  if (meta) {
    meta.textContent = requestedSelector ? `${requestedSelector} {` : '{';
  }

  const cssText = extractCssFromDocumentModel() || loadCustomCss();
  const declarations = getScopedCssDeclarations(cssText, requestedSelector);

  editor.value = declarations || '';
  surface.style.display = 'block';
  inlineCssSurfaceState = {
    source,
    selector: requestedSelector,
    baseCssText: cssText,
  };

  autosizeInlineCssEditor(editor);

  const caret = editor.value.length;
  editor.focus();
  editor.setSelectionRange(caret, caret);
  editor.scrollTop = editor.scrollHeight;
}

function closeBlockSrc(index, commitChanges) {
  const wrap = document.querySelector('.block-wrap[data-block-index="' + index + '"]');
  if (!wrap) return;

  const view = wrap.querySelector('.block-view');
  const srcWrap = wrap.querySelector('.block-src-wrapper');
  const source = wrap.querySelector('.block-src');

  if (!view || !srcWrap || !source || srcWrap.style.display !== 'block') {
    return;
  }

  if (commitChanges) {
    commitBlockSrc(index);
    return;
  }

  closeInlineCssSurface();
  srcWrap.style.display = 'none';
  view.style.display = '';
}

function commitOpenSources(exceptIndex) {
  const openWraps = Array.from(document.querySelectorAll('.block-wrap .block-src-wrapper'))
    .filter((node) => node.style.display === 'block')
    .map((node) => node.closest('.block-wrap'))
    .filter(Boolean);

  const indices = openWraps
    .map((wrap) => Number.parseInt(wrap.dataset.blockIndex, 10))
    .filter((value) => !Number.isNaN(value) && value !== exceptIndex)
    .sort((a, b) => b - a);

  for (const index of indices) {
    commitBlockSrc(index);
  }
}

function hasOpenBlockSources() {
  return Boolean(Array.from(document.querySelectorAll('.block-wrap .block-src-wrapper'))
    .find((node) => node.style.display === 'block'));
}

function isBlankPageClickTarget(target, pageEl, blocksContainer) {
  if (!target || !pageEl || !blocksContainer) return false;
  if (target.closest('.ui-chrome')) return false;
  if (target.closest('.meta-rendered') || target.closest('.meta-input')) return false;
  if (target.closest('.block-view') || target.closest('.block-src-wrapper')) return false;
  if (target.closest('button, select, input, textarea, a, label')) return false;
  return target === pageEl || target === blocksContainer;
}

function updateInlineCssAffordance(textarea) {
  if (!textarea) return;

  const target = findAttributeTargetAtCursor(textarea);
  const isActive = Boolean(target && target.selector);
  textarea.classList.toggle('css-target-active', isActive);
  if (isActive) {
    textarea.setAttribute('title', 'Option/Alt + click to edit scoped styles');
  } else {
    textarea.removeAttribute('title');
  }
}

function openBlockSrc(index) {
  const wrap = document.querySelector('.block-wrap[data-block-index="' + index + '"]');
  if (!wrap) return;

  const view = wrap.querySelector('.block-view');
  const srcWrap = wrap.querySelector('.block-src-wrapper');
  const source = wrap.querySelector('.block-src');

  if (!view || !source || !srcWrap || srcWrap.style.display === 'block') {
    return;
  }

  const block = docModel.blocks[index];
  if (!block) return;

  const rawSource = block.rawSource || stringifyBlock(block);
  source.value = getRawSourceForEditor(rawSource);
  source.dataset.originalSource = rawSource;
  view.style.display = 'none';
  srcWrap.style.display = 'block';
  autosizeBlockSrc(source);
  updateInlineCssAffordance(source);
  source.focus();
}

function commitBlockSrc(index) {
  const wrap = document.querySelector('.block-wrap[data-block-index="' + index + '"]');
  if (!wrap) return;

  const view = wrap.querySelector('.block-view');
  const srcWrap = wrap.querySelector('.block-src-wrapper');
  const source = wrap.querySelector('.block-src');

  if (!view || !source || !srcWrap) {
    return;
  }

  const originalSource = source.dataset.originalSource || '';
  const nextSource = getRawSourceFromEditor(source.value);
  const parsed = parseBlock(nextSource);
  parsed.rawSource = nextSource;

  if (parsed.type === 'paragraph' && !String(parsed.text || '').trim()) {
    docModel.blocks.splice(index, 1);
    renderDocument();
    setStatus('Block removed');
    return;
  }

  if (!KNOWN_BLOCK_TYPES.has(parsed.type)) {
    if (!originalSource) {
      docModel.blocks.splice(index, 1);
      renderDocument();
      return;
    }
    const reverted = parseBlock(originalSource);
    reverted.rawSource = originalSource;
    docModel.blocks[index] = reverted;
    view.textContent = '';
    view.appendChild(buildRenderedContent(reverted));
    srcWrap.style.display = 'none';
    view.style.display = '';
    setStatus('Reverted — unknown block type');
    return;
  }

  docModel.blocks[index] = parsed;
  view.textContent = '';
  view.appendChild(buildRenderedContent(parsed));
  srcWrap.style.display = 'none';
  view.style.display = '';
  refreshDocumentCss();
  setStatus('Unsaved changes');
}

function wireMetaField(viewId, inputId, getter, setter) {
  // Removed: metadata section no longer exists
}

function renderDocument() {
  if (!docModel) return;

  const blocksEl = document.getElementById('blocks');
  if (!blocksEl) return;

  blocksEl.textContent = '';

  docModel.blocks.forEach((block, index) => {
    blocksEl.appendChild(buildBlockWrap(block, index));
  });

  refreshDocumentCss();
}

function saveDoc() {
  if (!docModel) return;
  commitOpenSources();
  docSaveState = 'saving';
  setStatusPersistent('Saving…', 'saving');
  vscode.postMessage({ type: 'save', text: stringifyDoc(docModel) });
}

function saveDocAuto() {
  if (!docModel) return;
  commitOpenSources();
  const currentDoc = stringifyDoc(docModel);
  
  if (currentDoc === lastSavedDoc) {
    docSaveState = 'idle';
    clearStatusPersistent();
    return;
  }
  
  docSaveState = 'saving';
  setStatusPersistent('Saving…', 'saving');
  vscode.postMessage({ type: 'save', text: currentDoc });
}

function getInsertIndexForY(container, clientY) {
  const wraps = Array.from(container.querySelectorAll('.block-wrap'));

  for (let index = 0; index < wraps.length; index += 1) {
    const rect = wraps[index].getBoundingClientRect();

    if (clientY < rect.top + rect.height / 2) {
      return index;
    }
  }

  return wraps.length;
}

function insertParagraphBlock(index) {
  if (!docModel || !Array.isArray(docModel.blocks)) return;

  const next = {
    type: 'paragraph',
    text: '',
    rawSource: '',
  };

  const safeIndex = Math.max(0, Math.min(index, docModel.blocks.length));
  docModel.blocks.splice(safeIndex, 0, next);
  renderDocument();
  setStatus('New block');

  requestAnimationFrame(() => {
    openBlockSrc(safeIndex);
  });
}

function clampScale(value) {
  const numeric = Number(value);

  if (!Number.isFinite(numeric)) {
    return 100;
  }

  return Math.min(115, Math.max(90, Math.round(numeric)));
}

function loadAppearance() {
  try {
    const raw = window.localStorage.getItem(appearanceStorageKey);

    if (!raw) {
      return;
    }

    const parsed = JSON.parse(raw);
    const paper = String(parsed.paper || 'white');
    const density = String(parsed.density || 'comfortable');
    const scale = clampScale(parsed.scale);
    currentAppearance = {
      paper: ['white', 'cream', 'slate'].includes(paper) ? paper : 'white',
      density: ['comfortable', 'compact'].includes(density) ? density : 'comfortable',
      scale,
    };
  } catch {
  }
}

function persistAppearance() {
  try {
    window.localStorage.setItem(appearanceStorageKey, JSON.stringify(currentAppearance));
  } catch {
  }
}

function applyAppearance(persist = false) {
  document.body.dataset.paper = currentAppearance.paper;
  document.body.dataset.density = currentAppearance.density;
  document.documentElement.style.setProperty('--editor-scale', String(currentAppearance.scale / 100));

  const paperSelect = document.getElementById('paper-select');
  const densitySelect = document.getElementById('density-select');
  const scaleSlider = document.getElementById('scale-slider');

  if (paperSelect) {
    paperSelect.value = currentAppearance.paper;
  }

  if (densitySelect) {
    densitySelect.value = currentAppearance.density;
  }

  if (scaleSlider) {
    scaleSlider.value = String(currentAppearance.scale);
  }

  if (persist) {
    persistAppearance();
  }
}

function applyTheme(theme, persist = false) {
  const allowed = new Set(['auto', 'light', 'dark']);
  currentTheme = allowed.has(theme) ? theme : 'auto';
  document.body.dataset.theme = currentTheme;

  const prefersDark = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
  const resolved = currentTheme === 'auto' ? (prefersDark ? 'dark' : 'light') : currentTheme;
  document.body.dataset.resolvedTheme = resolved;

  const select = document.getElementById('theme-select');
  if (select) {
    select.value = currentTheme;
  }

  if (persist) {
    vscode.postMessage({ type: 'set-theme', theme: currentTheme });
  }
}

function setControlsOpen(isOpen) {
  const chrome = document.getElementById('ui-chrome');
  const toggle = document.getElementById('ui-chrome-toggle');

  if (chrome) {
    chrome.dataset.open = isOpen ? 'true' : 'false';
  }

  if (toggle) {
    toggle.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
  }

  if (isOpen) {
    document.body.classList.add('show-chrome');
  }
}

function setHelpOpen(isOpen) {
  const chrome = document.getElementById('ui-chrome');
  const btn = document.getElementById('ui-chrome-help-btn');

  if (chrome) {
    chrome.dataset.help = isOpen ? 'true' : 'false';
  }

  if (btn) {
    btn.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
    btn.setAttribute('aria-label', isOpen ? 'Hide Help' : 'Show Help');
  }

  if (isOpen) {
    document.body.classList.add('show-chrome');
  }
}

function revealChromeBriefly(durationMs = 1400) {
  document.body.classList.add('show-chrome');

  if (chromeRevealTimer) {
    clearTimeout(chromeRevealTimer);
  }

  chromeRevealTimer = setTimeout(() => {
    const chrome = document.getElementById('ui-chrome');

    if (chrome && (chrome.dataset.open === 'true' || chrome.dataset.help === 'true')) {
      return;
    }

    document.body.classList.remove('show-chrome');
  }, durationMs);
}

function toggleControls(forceOpen) {
  const chrome = document.getElementById('ui-chrome');
  const isOpen = chrome ? chrome.dataset.open === 'true' : false;
  const opening = typeof forceOpen === 'boolean' ? forceOpen : !isOpen;

  if (opening) {
    setHelpOpen(false);
  }

  setControlsOpen(opening);
}

function toggleHelp() {
  const chrome = document.getElementById('ui-chrome');
  const isOpen = chrome ? chrome.dataset.help === 'true' : false;

  if (!isOpen) {
    setControlsOpen(false);
  }

  setHelpOpen(!isOpen);
}

function isEditModeEnabled() {
  const page = document.querySelector('.page');
  return Boolean(page && page.dataset.editMode === 'true');
}

function loadEditModePreference() {
  try {
    const stored = window.localStorage.getItem(editModeStorageKey);
    if (stored === 'true') return true;
    if (stored === 'false') return false;
  } catch {
  }
  return true;
}

function persistEditModePreference(enabled) {
  try {
    window.localStorage.setItem(editModeStorageKey, enabled ? 'true' : 'false');
  } catch {
  }
}

function setEditMode(enabled) {
  const page = document.querySelector('.page');
  const toggle = document.getElementById('ui-chrome-edit-toggle');
  const modePill = document.getElementById('mode-pill');
  
  if (page) {
    page.dataset.editMode = enabled ? 'true' : 'false';
  }
  
  if (toggle) {
    toggle.setAttribute('aria-pressed', enabled ? 'true' : 'false');
  }

  if (modePill) {
    modePill.dataset.mode = enabled ? 'edit' : 'read';
    modePill.textContent = enabled ? 'Editing' : 'Read only';
  }

  persistEditModePreference(enabled);
}

function toggleEditMode() {
  const page = document.querySelector('.page');
  if (!page) return;
  const isEnabled = page.dataset.editMode === 'true';
  const nextEnabled = !isEnabled;

  if (!nextEnabled) {
    closeInlineCssSurface();
    commitOpenSources();
    closeAutocomplete();
  }

  setEditMode(nextEnabled);
  setStatus(nextEnabled ? 'Edit mode on' : 'Edit mode off');
}

function hideLoadingScreen() {
  const page = document.querySelector('.page');
  const loadingScreen = document.getElementById('loading-screen');

  if (page) {
    page.dataset.ready = 'true';
    page.setAttribute('aria-busy', 'false');
  }

  if (loadingScreen) {
    requestAnimationFrame(() => {
      loadingScreen.dataset.hidden = 'true';
      const removeLoading = () => {
        loadingScreen.style.display = 'none';
      };

      loadingScreen.addEventListener('transitionend', removeLoading, { once: true });
      setTimeout(removeLoading, 220);
    });
  }
}

function insertImageBlock(imagePath, altText) {
  if (!docModel) return;

  const block = {
    type: 'image',
    src: String(imagePath || '').trim(),
    alt: String(altText || '').trim(),
  };

  block.rawSource = stringifyBlock(block);
  docModel.blocks.push(block);
  renderDocument();
  setStatus('Image added');
}

function readFileAsPayload(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();

    reader.onload = () => {
      const result = String(reader.result || '');
      const marker = 'base64,';
      const index = result.indexOf(marker);

      if (index === -1) {
        reject(new Error('Unable to read image payload.'));
        return;
      }

      resolve({
        name: file.name,
        mimeType: file.type,
        base64Data: result.slice(index + marker.length),
      });
    };

    reader.onerror = () => reject(new Error('Unable to read image payload.'));
    reader.readAsDataURL(file);
  });
}

function wireDragAndDrop() {
  function clearDropState() {
    document.body.classList.remove('drag-active');
  }

  window.addEventListener('dragenter', (event) => {
    event.preventDefault();
    document.body.classList.add('drag-active');
  });

  window.addEventListener('dragover', (event) => {
    event.preventDefault();
    document.body.classList.add('drag-active');
  });

  window.addEventListener('dragleave', (event) => {
    if (event.relatedTarget) return;
    clearDropState();
  });

  window.addEventListener('drop', async (event) => {
    event.preventDefault();
    clearDropState();

    const files = Array.from((event.dataTransfer && event.dataTransfer.files) || []);
    const images = files.filter((file) => String(file.type || '').startsWith('image/'));

    if (images.length === 0) {
      return;
    }

    setStatus('Uploading image');

    for (const image of images) {
      try {
        const payload = await readFileAsPayload(image);
        vscode.postMessage({
          type: 'upload-image',
          name: payload.name,
          mimeType: payload.mimeType,
          base64Data: payload.base64Data,
          alt: image.name.replace(/\.[^.]+$/, ''),
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Image upload failed.';
        setStatus(message);
      }
    }
  });
}

function initializeDocument() {
  const initEl = document.getElementById('doc-init');
  const sourceEl = document.getElementById('doc-init-source');
  const docPath = initEl && initEl.dataset && initEl.dataset.docPath ? initEl.dataset.docPath : 'unknown.dx';
  const sourceText = sourceEl ? sourceEl.value : '';
  const errorText = initEl && initEl.dataset && initEl.dataset.docError ? initEl.dataset.docError : '';
  const initialTheme = initEl && initEl.dataset && initEl.dataset.initialTheme ? initEl.dataset.initialTheme : 'auto';
  currentDocPath = docPath;

  const docPathEl = document.getElementById('doc-path');
  if (docPathEl) {
    docPathEl.textContent = docPath;
  }

  loadNoteEl = document.getElementById('load-note');
  if (loadNoteEl) {
    if (errorText) {
      loadNoteEl.textContent = 'Source load warning: ' + errorText;
      loadNoteEl.classList.add('error');
    } else {
      loadNoteEl.textContent = 'Ready: ' + String(sourceText || '').length + ' chars loaded from DOC storage.';
      setTimeout(() => {
        if (loadNoteEl) {
          loadNoteEl.classList.add('quiet');
        }
      }, 900);
    }
  }

  try {
    const raw = String(sourceText || '').trim();

    if (!raw) {
      docModel = {
        metadata: {
          title: '',
          summary: '',
          tags: '',
        },
        blocks: [],
      };
    } else {
      docModel = parseDoc(sourceText);
    }

    renderDocument();
    lastSavedDoc = stringifyDoc(docModel);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus('Parse error: ' + message);
  }

  const blocksContainer = document.getElementById('blocks');

  if (blocksContainer) {
    blocksContainer.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target) return;

      const completionItem = target.closest('.autocomplete-item');
      if (completionItem) {
        event.preventDefault();
        event.stopPropagation();

        const index = Number.parseInt(completionItem.dataset.index, 10);
        const active = autocompleteState.textarea;
        if (!Number.isNaN(index) && active) {
          acceptAutocomplete(active, index);
          active.focus();
        }
        return;
      }

      const view = target.closest('.block-view');
      if (!view) return;

      if (!isEditModeEnabled()) {
        return;
      }

      // Do not hijack clicks on interactive child content in rendered blocks.
      if (target.closest('a, button, input, select, textarea, label')) {
        return;
      }

      // Preserve text selection behavior instead of forcing edit-open.
      const selection = window.getSelection ? window.getSelection() : null;
      if (selection && !selection.isCollapsed) {
        return;
      }

      // Click to open edit mode (single click activates edit)
      view.focus();

      const wrap = view.closest('.block-wrap');
      if (!wrap) return;

      const index = Number.parseInt(wrap.dataset.blockIndex, 10);
      if (Number.isNaN(index)) return;
      commitOpenSources(index);
      openBlockSrc(index);
    });

    blocksContainer.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') return;
      const view = document.activeElement?.closest('.block-view');
      if (!view) return;
      if (!isEditModeEnabled()) return;

      const wrap = view.closest('.block-wrap');
      if (!wrap) return;

      const index = Number.parseInt(wrap.dataset.blockIndex, 10);
      if (Number.isNaN(index)) return;

      event.preventDefault();
      commitOpenSources(index);
      openBlockSrc(index);
    });

    blocksContainer.addEventListener('focusout', (event) => {
      if (!event.target.classList.contains('block-src')) return;

      if (event.relatedTarget && event.relatedTarget.closest && event.relatedTarget.closest('.autocomplete-menu')) {
        return;
      }

      closeAutocomplete();
    });

    blocksContainer.addEventListener('keydown', (event) => {
      if (!event.target.classList.contains('block-src')) return;
      const textarea = event.target;

      if ((event.ctrlKey || event.metaKey) && event.code === 'Space') {
        event.preventDefault();
        renderAutocomplete(textarea, true);
        return;
      }

      if (event.key === 'ArrowDown' && autocompleteState.textarea === textarea) {
        if (autocompleteState.suggestions.length > 0) {
          event.preventDefault();
          updateAutocompleteSelection(1);
          return;
        }
      }

      if (event.key === 'ArrowUp' && autocompleteState.textarea === textarea) {
        if (autocompleteState.suggestions.length > 0) {
          event.preventDefault();
          updateAutocompleteSelection(-1);
          return;
        }
      }

      if (event.key === 'Enter' && autocompleteState.textarea === textarea && autocompleteState.suggestions.length > 0) {
        event.preventDefault();
        acceptAutocomplete(textarea);
        return;
      }

      if (event.key === 'Tab') {
        if (autocompleteState.textarea === textarea && autocompleteState.suggestions.length > 0) {
          event.preventDefault();
          acceptAutocomplete(textarea);
          return;
        }
      }

      if (event.key === 'Escape' && autocompleteState.textarea === textarea && autocompleteState.suggestions.length > 0) {
        event.preventDefault();
        event.stopPropagation();
        closeAutocomplete();
        return;
      }

      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (!wrap) return;
        const index = Number.parseInt(wrap.dataset.blockIndex, 10);
        if (Number.isNaN(index)) return;
        closeBlockSrc(index, true);
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (!wrap) return;
        const index = Number.parseInt(wrap.dataset.blockIndex, 10);
        if (Number.isNaN(index)) return;
        closeBlockSrc(index, true);
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        event.stopPropagation();
        saveDoc();
      }
    });

    blocksContainer.addEventListener('input', (event) => {
      if (!event.target.classList.contains('block-src')) return;
      autosizeBlockSrc(event.target);
      renderAutocomplete(event.target, false);
      updateInlineCssAffordance(event.target);
      debouncedAutosave();
    });

    blocksContainer.addEventListener('keyup', (event) => {
      if (!event.target.classList.contains('block-src')) return;
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') {
        renderAutocomplete(event.target, false);
      }
      updateInlineCssAffordance(event.target);
    });

    blocksContainer.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-src')) return;
      renderAutocomplete(target, false);
      updateInlineCssAffordance(target);
    });

    blocksContainer.addEventListener('mouseup', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-src')) return;
      const textarea = target;

      // Keep style editing explicit to avoid popups while cursoring/selecting text.
      const wantsScopedStyleEdit = Boolean(event.altKey || event.metaKey || event.ctrlKey);
      if (!wantsScopedStyleEdit) {
        updateInlineCssAffordance(textarea);
        return;
      }

      const selectionStart = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : -1;
      const selectionEnd = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : -1;
      if (selectionStart !== selectionEnd) {
        updateInlineCssAffordance(textarea);
        return;
      }

      requestAnimationFrame(() => {
        const target = findAttributeTargetAtCursor(textarea);
        if (target && target.selector) {
          openInlineCssSurface(textarea, target.selector);
          return;
        }
        updateInlineCssAffordance(textarea);
      });
    });

    blocksContainer.addEventListener('mousedown', (event) => {
      const target = getEventElementTarget(event);
      if (!target) return;

      const item = target.closest('.autocomplete-item');
      if (!item) return;

      event.preventDefault();
      const active = autocompleteState.textarea;
      const index = Number.parseInt(item.dataset.index, 10);
      if (active && !Number.isNaN(index)) {
        acceptAutocomplete(active, index);
        active.focus();
      }
    });

    blocksContainer.addEventListener('scroll', (event) => {
      if (!event.target.classList.contains('block-src')) return;
      const { mirrorEl } = getAutocompleteEls(event.target);
      if (mirrorEl) mirrorEl.scrollTop = event.target.scrollTop;
    }, true);

    blocksContainer.addEventListener('mousemove', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-src')) return;
      updateInlineCssAffordance(target);
    });
  }

  const pageEl = document.querySelector('.page');
  if (pageEl && blocksContainer) {
    pageEl.addEventListener('mousedown', (event) => {
      const target = getEventElementTarget(event);
      if (!target) {
        blankPagePointerDownHadOpenEditors = false;
        return;
      }

      if (!isBlankPageClickTarget(target, pageEl, blocksContainer)) {
        blankPagePointerDownHadOpenEditors = false;
        return;
      }

      blankPagePointerDownHadOpenEditors = hasOpenBlockSources() || Boolean(inlineCssSurfaceState.source);
    });

    pageEl.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target) return;

      const isBlankArea = isBlankPageClickTarget(target, pageEl, blocksContainer);
      if (!isBlankArea) return;

      const hadOpenSources = hasOpenBlockSources();
      const hadOpenInlineCss = Boolean(inlineCssSurfaceState.source);
      const shouldOnlyCloseEditors = blankPagePointerDownHadOpenEditors || hadOpenSources || hadOpenInlineCss;

      closeInlineCssSurface();
      commitOpenSources();
      blankPagePointerDownHadOpenEditors = false;

      // Blank-space clicks: first close editors, second click inserts block
      if (shouldOnlyCloseEditors) {
        return;
      }

      // Get edit mode state
      const page = document.querySelector('.page');
      const isEditMode = page && page.dataset.editMode === 'true';
      
      if (!isEditMode) {
        return;
      }

      const index = getInsertIndexForY(blocksContainer, event.clientY);
      insertParagraphBlock(index);
    });
  }

  const themeSelect = document.getElementById('theme-select');
  if (themeSelect) {
    themeSelect.addEventListener('change', (event) => {
      applyTheme(event.target.value, true);
    });
  }

  const paperSelect = document.getElementById('paper-select');
  if (paperSelect) {
    paperSelect.addEventListener('change', (event) => {
      currentAppearance.paper = String(event.target.value || 'white');
      applyAppearance(true);
    });
  }

  const densitySelect = document.getElementById('density-select');
  if (densitySelect) {
    densitySelect.addEventListener('change', (event) => {
      currentAppearance.density = String(event.target.value || 'comfortable');
      applyAppearance(true);
    });
  }

  const scaleSlider = document.getElementById('scale-slider');
  if (scaleSlider) {
    scaleSlider.addEventListener('input', (event) => {
      currentAppearance.scale = clampScale(event.target.value);
      applyAppearance(true);
    });
  }

  const controlsToggle = document.getElementById('ui-chrome-toggle');
  if (controlsToggle) {
    controlsToggle.setAttribute('aria-label', 'Settings');
    controlsToggle.setAttribute('tabindex', '0');
    controlsToggle.addEventListener('click', (event) => {
      event.preventDefault();
      toggleControls();
    });
    controlsToggle.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        toggleControls();
      }
    });
  }

  const helpBtn = document.getElementById('ui-chrome-help-btn');
  if (helpBtn) {
    helpBtn.setAttribute('aria-label', 'Tutorial, setup, and format help');
    helpBtn.setAttribute('tabindex', '0');
    helpBtn.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      toggleHelp();
    });
    helpBtn.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        event.stopPropagation();
        toggleHelp();
      }
    });
  }

  const editToggle = document.getElementById('ui-chrome-edit-toggle');
  if (editToggle) {
    editToggle.setAttribute('aria-label', 'Toggle Edit Mode');
    editToggle.setAttribute('tabindex', '0');
    editToggle.addEventListener('click', (event) => {
      event.preventDefault();
      toggleEditMode();
    });
    editToggle.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        toggleEditMode();
      }
    });
  }

  document.addEventListener('click', (event) => {
    const target = getEventElementTarget(event);
    if (!target) return;

    const chrome = document.getElementById('ui-chrome');
    if (!chrome) return;
    if (chrome.contains(target)) return;

    if (chrome.dataset.open === 'true') {
      setControlsOpen(false);
    }

    if (chrome.dataset.help === 'true') {
      setHelpOpen(false);
    }
  });

  document.addEventListener('mousedown', (event) => {
    const target = getEventElementTarget(event);
    if (!target) return;

    if (target.closest('.block-src-wrapper')) return;
    if (target.closest('.inline-css-surface')) return;
    if (target.closest('.block-view')) return;
    if (target.classList && target.classList.contains('page')) return;
    if (target.id === 'blocks') return;
    if (target.closest('.autocomplete-menu')) return;
    if (target.closest('.ui-chrome')) return;

    closeInlineCssSurface();
    commitOpenSources();
  });

  if (window.matchMedia) {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    media.addEventListener('change', () => {
      if (currentTheme === 'auto') {
        applyTheme('auto', false);
      }
    });
  }

  applyTheme(initialTheme, false);
  currentAppearance = {
    paper: 'white',
    density: 'comfortable',
    scale: 100,
  };
  applyAppearance(false);
  refreshDocumentCss();
  setControlsOpen(false);
  setHelpOpen(false);
  setEditMode(true);
  wireDragAndDrop();
  hideLoadingScreen();
  vscode.postMessage({ type: 'get-config' });

  window.addEventListener('keydown', (event) => {
    if (event.defaultPrevented) {
      return;
    }

    if (event.key === 'Escape') {
      closeInlineCssSurface();
      setControlsOpen(false);
      setHelpOpen(false);
    }

    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      saveDoc();
    }

    if (event.key === 'Tab' && !isTypingTarget(document.activeElement)) {
      const active = document.activeElement;
      const activeBlockView = active && active.closest ? active.closest('.block-view') : null;
      if (!activeBlockView) {
        return;
      }

      event.preventDefault();
      const blocksContainer = document.getElementById('blocks');
      if (!blocksContainer) return;

      const blockViews = Array.from(blocksContainer.querySelectorAll('.block-view'));
      const focused = blockViews.findIndex((block) => block.contains(document.activeElement) || block === document.activeElement);

      if (focused === -1 && blockViews.length > 0) {
        blockViews[0].focus();
      } else if (focused !== -1) {
        const nextFocus = event.shiftKey ? Math.max(0, focused - 1) : Math.min(blockViews.length - 1, focused + 1);
        blockViews[nextFocus].focus();
      }
    }
  });

  window.addEventListener('message', (event) => {
    const msg = event.data || {};

    if (msg.type === 'status') {
      setStatus(msg.text);
      return;
    }

    if (msg.type === 'set-source') {
      docModel = parseDoc(msg.text || '');
      lastSavedDoc = msg.text || '';
      renderDocument();
      applyCustomCss(loadCustomCss());
      setStatus('Refreshed');
      return;
    }

    if (msg.type === 'config') {
      applyTheme(String(msg.theme || 'auto'), false);
      return;
    }

    if (msg.type === 'image-uploaded') {
      insertImageBlock(String(msg.path || ''), String(msg.alt || ''));
    }

    if (msg.type === 'save-complete') {
      docSaveState = 'saved';
      lastSavedDoc = stringifyDoc(docModel || { metadata: {}, blocks: [] });
      setStatusPersistent('Saved', 'saved');
      setTimeout(() => {
        if (docSaveState === 'saved') {
          docSaveState = 'idle';
          clearStatusPersistent();
        }
      }, 1600);
      return;
    }

    if (msg.type === 'save-error') {
      docSaveState = 'error';
      setStatusPersistent('Save failed: ' + (msg.error || 'unknown error'), 'error');
      return;
    }
  });

  if (errorText) {
    setStatus(errorText);
  }
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initializeDocument);
} else {
  initializeDocument();
}
