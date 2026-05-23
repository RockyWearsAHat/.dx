import {
  computeAutocompleteSuggestions,
  computeNextSelectedIndex,
  DEFAULT_BLOCK_AUTOCOMPLETE,
  getLineContextFromValue,
} from './webview-autocomplete-core.js';
import {
  buildAutocompleteSchemaFromHeaders,
  createAutocompleteHistory,
  mergeAutocompleteSchemas,
} from './webview-autocomplete-history.js';

const AUTOCOMPLETE_HISTORY_STORAGE_KEY = 'docdb.autocomplete-history.v1';

export {
  computeAutocompleteSuggestions,
  computeNextSelectedIndex,
  DEFAULT_BLOCK_AUTOCOMPLETE,
  getLineContextFromValue,
};

function defaultEscapeHtml(value) {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function createAutocompleteController(options) {
  const config = options || {};
  const blockAutocomplete = Array.isArray(config.blockAutocomplete)
    ? config.blockAutocomplete
    : DEFAULT_BLOCK_AUTOCOMPLETE;
  const collectKnownIds = typeof config.collectKnownIds === 'function'
    ? config.collectKnownIds
    : () => [];
  const collectKnownClasses = typeof config.collectKnownClasses === 'function'
    ? config.collectKnownClasses
    : () => [];
  const collectKnownImageSources = typeof config.collectKnownImageSources === 'function'
    ? config.collectKnownImageSources
    : () => [];
  const collectAutocompleteHeaders = typeof config.collectAutocompleteHeaders === 'function'
    ? config.collectAutocompleteHeaders
    : () => [];
  const escapeHtml = typeof config.escapeHtml === 'function'
    ? config.escapeHtml
    : defaultEscapeHtml;
  const storage = config.storage && typeof config.storage.getItem === 'function' && typeof config.storage.setItem === 'function'
    ? config.storage
    : null;
  const storageKey = String(config.storageKey || AUTOCOMPLETE_HISTORY_STORAGE_KEY);
  const raf = typeof config.requestAnimationFrame === 'function'
    ? config.requestAnimationFrame
    : (callback) => callback();

  const history = createAutocompleteHistory(storage, storageKey);

  const state = {
    textarea: null,
    suggestions: [],
    selectedIndex: 0,
    replaceStart: 0,
    replaceEnd: 0,
    typed: '',
  };

  let measurementCanvas = null;

  function getAutocompleteSuggestions(textarea, forceOpen = false) {
    if (!textarea) {
      return { suggestions: [], replaceStart: 0, replaceEnd: 0, typed: '' };
    }

    const context = getLineContextFromValue(textarea.value || '', textarea.selectionStart || 0);
    const runtimeSchema = buildAutocompleteSchemaFromHeaders(collectAutocompleteHeaders());
    const schema = mergeAutocompleteSchemas(runtimeSchema, history.getSchema());
    return computeAutocompleteSuggestions({
      context,
      forceOpen,
      blockAutocomplete,
      knownIds: collectKnownIds(),
      knownClasses: collectKnownClasses(),
      knownImageSources: collectKnownImageSources(),
      knownBlockTypes: schema.blockTypes,
      knownAttributeKeys: schema.attributeKeys,
      knownAttributeValuesByKey: schema.attributeValuesByKey,
    });
  }

  function getAutocompleteEls(textarea) {
    const srcWrap = textarea ? textarea.closest('.block-src-wrapper') : null;
    if (!srcWrap) {
      return { menuEl: null, mirrorEl: null };
    }

    return {
      menuEl: srcWrap.querySelector('.autocomplete-menu'),
      mirrorEl: srcWrap.querySelector('.block-src-mirror'),
    };
  }

  function renderGhostText(textarea, mirrorEl) {
    if (!mirrorEl || !state.suggestions.length) return;

    const selected = state.suggestions[state.selectedIndex];
    if (!selected) {
      return;
    }

    const value = String(textarea.value || '');
    const ghostSuffix = selected.insertText.startsWith(state.typed)
      ? selected.insertText.slice(state.typed.length)
      : '';

    mirrorEl.textContent = '';
    mirrorEl.appendChild(document.createTextNode(value.slice(0, state.replaceStart) + state.typed));
    if (ghostSuffix) {
      const span = document.createElement('span');
      span.className = 'ghost-suffix';
      span.textContent = ghostSuffix;
      mirrorEl.appendChild(span);
    }
    mirrorEl.appendChild(document.createTextNode(value.slice(state.replaceEnd)));
    mirrorEl.scrollTop = textarea.scrollTop;

    const srcWrap = textarea.closest('.block-src-wrapper');
    if (srcWrap) {
      srcWrap.classList.add('ghost-active');
    }
    textarea.classList.add('ghost-active');
  }

  function closeAutocomplete() {
    const textarea = state.textarea;
    if (!textarea) {
      return;
    }

    const { menuEl, mirrorEl } = getAutocompleteEls(textarea);
    const srcWrap = textarea.closest('.block-src-wrapper');

    if (menuEl) {
      menuEl.innerHTML = '';
      menuEl.style.display = 'none';
    }

    if (mirrorEl) {
      mirrorEl.textContent = '';
    }

    if (srcWrap) {
      srcWrap.classList.remove('ghost-active');
    }

    textarea.classList.remove('ghost-active');

    state.textarea = null;
    state.suggestions = [];
    state.selectedIndex = 0;
    state.replaceStart = 0;
    state.replaceEnd = 0;
    state.typed = '';
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

    state.textarea = textarea;
    state.suggestions = suggestions;
    state.selectedIndex = 0;
    state.replaceStart = model.replaceStart;
    state.replaceEnd = model.replaceEnd;
    state.typed = model.typed;

    if (suggestions.length === 0) {
      closeAutocomplete();
      return false;
    }

    renderGhostText(textarea, mirrorEl);

    menuEl.innerHTML = suggestions
      .map((item, index) => {
        const selectedClass = index === 0 ? ' selected' : '';
        return '<button type="button" class="autocomplete-item' + selectedClass + '" data-index="' + index + '"><span class="autocomplete-item-label">' + escapeHtml(item.label) + '</span><span class="autocomplete-item-detail">' + escapeHtml(item.detail) + '</span></button>';
      })
      .join('');
    menuEl.style.display = 'block';

    raf(() => {
      positionAutocompleteMenu(textarea, menuEl);
    });

    return true;
  }

  function positionAutocompleteMenu(textarea, menuEl) {
    if (!textarea || !menuEl) {
      return;
    }

    const computed = window.getComputedStyle(textarea);
    const lineHeight = Number.parseFloat(computed.lineHeight) || 20;
    const paddingTop = Number.parseFloat(computed.paddingTop) || 0;
    const paddingLeft = Number.parseFloat(computed.paddingLeft) || 0;
    const beforeCursor = String(textarea.value || '').slice(0, textarea.selectionStart || 0);
    const currentLine = beforeCursor.slice(beforeCursor.lastIndexOf('\n') + 1);
    const lineIndex = beforeCursor.split('\n').length - 1;

    if (!measurementCanvas) {
      measurementCanvas = document.createElement('canvas');
    }

    const context = measurementCanvas.getContext('2d');
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
    if (!state.textarea || state.suggestions.length === 0) {
      return false;
    }

    state.selectedIndex = computeNextSelectedIndex(state.selectedIndex, delta, state.suggestions.length);

    const textarea = state.textarea;
    const { menuEl, mirrorEl } = getAutocompleteEls(textarea);

    renderGhostText(textarea, mirrorEl);

    if (menuEl) {
      const items = menuEl.querySelectorAll('.autocomplete-item');
      items.forEach((item, index) => {
        item.classList.toggle('selected', index === state.selectedIndex);
      });

      const activeItem = menuEl.querySelector('.autocomplete-item.selected');
      if (activeItem) {
        activeItem.scrollIntoView({ block: 'nearest' });
      }
    }

    return true;
  }

  function acceptAutocomplete(textarea, explicitIndex) {
    if (!textarea || state.textarea !== textarea || state.suggestions.length === 0) {
      return false;
    }

    const index = Number.isInteger(explicitIndex)
      ? Math.max(0, Math.min(explicitIndex, state.suggestions.length - 1))
      : state.selectedIndex;
    const choice = state.suggestions[index];
    if (!choice) {
      return false;
    }

    if (choice.kind === 'attribute-value') {
      const context = getLineContextFromValue(textarea.value || '', textarea.selectionStart || 0);
      const valueMatch = /\b([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]*))$/i.exec(context.beforeCursor);
      history.rememberToken('attribute-value', choice.insertText, valueMatch ? valueMatch[1] : '');
    } else {
      history.rememberToken(choice.kind, choice.insertText);
    }

    const value = String(textarea.value || '');
    const insertText = String(choice.insertText || '');
    const before = value.slice(0, state.replaceStart);
    const after = value.slice(state.replaceEnd);
    textarea.value = before + insertText + after;

    const nextCursor = state.replaceStart + insertText.length;
    textarea.setSelectionRange(nextCursor, nextCursor);
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
    closeAutocomplete();
    return true;
  }

  return {
    state,
    acceptAutocomplete,
    closeAutocomplete,
    getAutocompleteEls,
    getAutocompleteSuggestions,
    positionAutocompleteMenu,
    renderAutocomplete,
    renderGhostText,
    updateAutocompleteSelection,
  };
}
