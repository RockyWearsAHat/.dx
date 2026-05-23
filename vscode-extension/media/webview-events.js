export function registerBlockInteractionEvents(options = {}) {
  const getEventElementTarget = typeof options.getEventElementTarget === 'function'
    ? options.getEventElementTarget
    : (event) => (event && event.target instanceof Element ? event.target : null);
  const documentRef = options.documentRef || document;
  const windowRef = options.windowRef || window;
  const blocksContainer = options.blocksContainer || documentRef.getElementById('blocks');
  const pageEl = options.pageEl || documentRef.querySelector('.page');

  if (!blocksContainer) {
    return;
  }

  let blankPagePointerDownHadOpenEditors = false;

  blocksContainer.addEventListener('click', (event) => {
    const target = getEventElementTarget(event);
    if (!target) return;

    if (documentRef.activeElement && documentRef.activeElement.classList.contains('block-src')) {
      return;
    }

    const completionItem = target.closest('.autocomplete-item');
    if (completionItem) {
      event.preventDefault();
      event.stopPropagation();

      const index = Number.parseInt(completionItem.dataset.index, 10);
      const active = options.autocompleteState.textarea;
      if (!Number.isNaN(index) && active) {
        options.acceptAutocomplete(active, index);
        active.focus();
      }
      return;
    }

    const view = target.closest('.block-view');
    if (!view) return;

    if (!options.isEditModeEnabled()) {
      return;
    }

    if (target.closest('a, button, input, select, textarea, label, .autocomplete-menu')) {
      return;
    }

    const selection = windowRef.getSelection ? windowRef.getSelection() : null;
    if (selection && !selection.isCollapsed) {
      return;
    }

    view.focus();

    const wrap = view.closest('.block-wrap');
    if (!wrap) return;

    const index = Number.parseInt(wrap.dataset.blockIndex, 10);
    if (Number.isNaN(index)) return;
    options.commitOpenSources(index);
    options.openBlockSrc(index);
  });

  blocksContainer.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    const view = documentRef.activeElement?.closest('.block-view');
    if (!view) return;
    if (!options.isEditModeEnabled()) return;

    const wrap = view.closest('.block-wrap');
    if (!wrap) return;

    const index = Number.parseInt(wrap.dataset.blockIndex, 10);
    if (Number.isNaN(index)) return;

    event.preventDefault();
    options.commitOpenSources(index);
    options.openBlockSrc(index);
  });

  blocksContainer.addEventListener('focusout', (event) => {
    if (!event.target.classList.contains('block-src')) return;

    if (event.relatedTarget && event.relatedTarget.closest && event.relatedTarget.closest('.autocomplete-menu')) {
      return;
    }

    options.closeAutocomplete();
  });

  blocksContainer.addEventListener('keydown', (event) => {
    if (!event.target.classList.contains('block-src') && !event.target.classList.contains('block-edit-header')) return;
    const textarea = event.target;

    if ((event.metaKey || event.ctrlKey) && (event.key.toLowerCase() === 'z' || event.key.toLowerCase() === 'y')) {
      event.preventDefault();
      event.stopPropagation();
      options.closeInlineCssSurface();

      const wrap = textarea.closest('.block-wrap');
      if (wrap) {
        const index = Number.parseInt(wrap.dataset.blockIndex, 10);
        if (!Number.isNaN(index)) {
          options.commitBlockSourceForHistory(index);
        }
      }

      if (options.isRedoShortcut(event)) {
        if (!options.performGlobalRedo()) {
          options.setStatus('Nothing to redo');
        }
      } else if (!options.performGlobalUndo()) {
        options.setStatus('Nothing to undo');
      }
      return;
    }

    if (textarea.classList.contains('block-edit-header')) {
      const srcWrap = textarea.closest('.block-src-wrapper');
      const bodyEditor = srcWrap ? srcWrap.querySelector('.block-src') : null;

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (wrap) {
          const index = Number.parseInt(wrap.dataset.blockIndex, 10);
          if (!Number.isNaN(index)) {
            options.commitBlockSrc(index);
          }
        }
        options.saveDoc();
      }

      if (event.key === 'ArrowDown' && bodyEditor) {
        const value = String(textarea.value || '');
        const start = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : value.length;
        const end = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : value.length;
        if (start === end && end === value.length) {
          event.preventDefault();
          bodyEditor.focus();
          bodyEditor.setSelectionRange(0, 0);
          return;
        }
      }

      if (event.key === 'ArrowRight' && bodyEditor) {
        const value = String(textarea.value || '');
        const start = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : value.length;
        const end = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : value.length;
        if (start === end && end === value.length) {
          event.preventDefault();
          bodyEditor.focus();
          bodyEditor.setSelectionRange(0, 0);
          return;
        }
      }

      if (event.key === 'Delete' && bodyEditor) {
        const headerValue = String(textarea.value || '');
        const start = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : headerValue.length;
        const end = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : headerValue.length;

        if (start === end && end === headerValue.length) {
          const bodyValue = String(bodyEditor.value || '');
          if (bodyValue.length > 0) {
            event.preventDefault();
            event.stopPropagation();
            options.ensureDocumentUndoSeed(bodyEditor, 'header-delete-join');
            bodyEditor.value = bodyValue.slice(1);
            options.autosizeBlockSrc(bodyEditor);
            options.updateInlineCssAffordance(textarea);
            options.debouncedAutosave();
            return;
          }
        }
      }

      if (event.key === 'Enter' && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        event.stopPropagation();

        if (!bodyEditor) {
          return;
        }

        const headerText = options.getRawSourceFromEditor(textarea.value || '');
        const trimmedHeaderText = headerText.trim();

        if (trimmedHeaderText.startsWith('::')) {
          bodyEditor.dataset.headerSource = headerText;
        } else if (trimmedHeaderText.length > 0) {
          bodyEditor.value = bodyEditor.value.length > 0
            ? `${headerText}\n${bodyEditor.value}`
            : headerText;
          bodyEditor.dataset.headerSource = '';
          textarea.value = '';
          options.autosizeBlockSrc(bodyEditor);
        } else {
          bodyEditor.dataset.headerSource = '';
        }

        options.renderEditableHeader(bodyEditor);
        bodyEditor.focus();
        bodyEditor.setSelectionRange(0, 0);
        return;
      }

      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (!wrap) return;
        const index = Number.parseInt(wrap.dataset.blockIndex, 10);
        if (Number.isNaN(index)) return;
        options.closeBlockSrc(index, true);
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (!wrap) return;
        const index = Number.parseInt(wrap.dataset.blockIndex, 10);
        if (Number.isNaN(index)) return;
        options.closeBlockSrc(index, true);
        return;
      }

      return;
    }

    if ((event.ctrlKey || event.metaKey) && event.code === 'Space') {
      event.preventDefault();
      options.renderAutocomplete(textarea, true);
      return;
    }

    if (event.key === 'ArrowUp' && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      const start = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : 0;
      const end = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : 0;
      if (start === 0 && end === 0) {
        const headerEditor = options.getBlockHeaderEditor(textarea);
        if (headerEditor && headerEditor.style.display !== 'none') {
          event.preventDefault();
          const caret = String(headerEditor.value || '').length;
          headerEditor.focus();
          headerEditor.setSelectionRange(caret, caret);
          return;
        }
      }
    }

    if (event.key === 'ArrowLeft' && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      const start = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : 0;
      const end = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : 0;
      if (start === 0 && end === 0) {
        const headerEditor = options.getBlockHeaderEditor(textarea);
        if (headerEditor && headerEditor.style.display !== 'none') {
          event.preventDefault();
          const caret = String(headerEditor.value || '').length;
          headerEditor.focus();
          headerEditor.setSelectionRange(caret, caret);
          return;
        }
      }
    }

    if (event.key === 'ArrowDown' && options.autocompleteState.textarea === textarea) {
      if (options.autocompleteState.suggestions.length > 0) {
        event.preventDefault();
        options.updateAutocompleteSelection(1);
        return;
      }
    }

    if (event.key === 'ArrowUp' && options.autocompleteState.textarea === textarea) {
      if (options.autocompleteState.suggestions.length > 0) {
        event.preventDefault();
        options.updateAutocompleteSelection(-1);
        return;
      }
    }

    if (event.key === 'Enter' && options.autocompleteState.textarea === textarea && options.autocompleteState.suggestions.length > 0) {
      event.preventDefault();
      options.acceptAutocomplete(textarea);
      return;
    }

    if (event.key === 'Tab') {
      if (options.autocompleteState.textarea === textarea && options.autocompleteState.suggestions.length > 0) {
        event.preventDefault();
        options.acceptAutocomplete(textarea);
        return;
      }
    }

    if (event.key === 'Escape' && options.autocompleteState.textarea === textarea && options.autocompleteState.suggestions.length > 0) {
      event.preventDefault();
      event.stopPropagation();
      options.closeAutocomplete();
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      const wrap = textarea.closest('.block-wrap');
      if (!wrap) return;
      const index = Number.parseInt(wrap.dataset.blockIndex, 10);
      if (Number.isNaN(index)) return;
      options.closeBlockSrc(index, true);
      return;
    }

    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      const wrap = textarea.closest('.block-wrap');
      if (!wrap) return;
      const index = Number.parseInt(wrap.dataset.blockIndex, 10);
      if (Number.isNaN(index)) return;
      options.closeBlockSrc(index, true);
      return;
    }

    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      event.stopPropagation();
      const wrap = textarea.closest('.block-wrap');
      if (wrap) {
        const index = Number.parseInt(wrap.dataset.blockIndex, 10);
        if (!Number.isNaN(index)) {
          options.commitBlockSrc(index);
        }
      }
      options.saveDoc();
    }
  });

  blocksContainer.addEventListener('input', (event) => {
    if (event.target.classList.contains('block-edit-header')) {
      const header = event.target;
      const srcWrap = header.closest('.block-src-wrapper');
      const source = srcWrap ? srcWrap.querySelector('.block-src') : null;
      options.ensureDocumentUndoSeed(source, 'header-live-edit');
      if (source) {
        source.dataset.headerSource = options.getRawSourceFromEditor(header.value);
      }
      header.style.height = '0px';
      header.style.height = header.scrollHeight + 'px';
      options.updateInlineCssAffordance(header);
      options.debouncedAutosave();
      return;
    }

    if (!event.target.classList.contains('block-src')) return;

    const source = event.target;
    options.ensureDocumentUndoSeed(source, 'block-live-edit');
    const headerEditor = options.getBlockHeaderEditor(source);
    const hasHeader = Boolean(String(source.dataset.headerSource || '').trim());
    const bodyText = String(source.value || '');
    if (!hasHeader && headerEditor && bodyText.startsWith(':') && !bodyText.includes('\n')) {
      source.dataset.headerSource = bodyText;
      source.value = '';
      options.renderEditableHeader(source);
      options.autosizeBlockSrc(source);
      headerEditor.focus();
      const caret = headerEditor.value.length;
      headerEditor.setSelectionRange(caret, caret);
      options.updateInlineCssAffordance(headerEditor);
      options.debouncedAutosave();
      return;
    }

    options.autosizeBlockSrc(event.target);
    options.renderAutocomplete(event.target, false);
    options.updateInlineCssAffordance(event.target);
    options.debouncedAutosave();
  });

  blocksContainer.addEventListener('keyup', (event) => {
    if (event.target.classList.contains('block-edit-header')) {
      options.updateInlineCssAffordance(event.target);
      return;
    }

    if (!event.target.classList.contains('block-src')) return;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') {
      options.renderAutocomplete(event.target, false);
    }
    options.updateInlineCssAffordance(event.target);
  });

  blocksContainer.addEventListener('click', (event) => {
    const target = getEventElementTarget(event);
    if (!target || !target.classList.contains('block-edit-header')) return;
    options.updateInlineCssAffordance(target);
  });

  blocksContainer.addEventListener('click', (event) => {
    const target = getEventElementTarget(event);
    if (!target || !target.classList.contains('block-src')) return;
    options.renderAutocomplete(target, false);
    options.updateInlineCssAffordance(target);
  });

  blocksContainer.addEventListener('mouseup', (event) => {
    const target = getEventElementTarget(event);
    if (!target || !target.classList.contains('block-edit-header')) return;
    const textarea = target;

    const wantsScopedStyleEdit = Boolean(event.altKey || event.metaKey || event.ctrlKey);
    if (!wantsScopedStyleEdit) {
      options.updateInlineCssAffordance(textarea);
      return;
    }

    const selectionStart = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : -1;
    const selectionEnd = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : -1;
    if (selectionStart !== selectionEnd) {
      options.updateInlineCssAffordance(textarea);
      return;
    }

    requestAnimationFrame(() => {
      const styleTarget = options.findAttributeTargetAtCursor(textarea);
      const srcWrap = textarea.closest('.block-src-wrapper');
      const source = srcWrap ? srcWrap.querySelector('.block-src') : null;

      if (styleTarget && styleTarget.selector && source) {
        options.openInlineCssSurface(source, styleTarget.selector);
        return;
      }
      options.updateInlineCssAffordance(textarea);
    });
  });

  blocksContainer.addEventListener('mouseup', (event) => {
    const target = getEventElementTarget(event);
    if (!target || !target.classList.contains('block-src')) return;
    const textarea = target;

    const wantsScopedStyleEdit = Boolean(event.altKey || event.metaKey || event.ctrlKey);
    if (!wantsScopedStyleEdit) {
      options.updateInlineCssAffordance(textarea);
      return;
    }

    const selectionStart = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : -1;
    const selectionEnd = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : -1;
    if (selectionStart !== selectionEnd) {
      options.updateInlineCssAffordance(textarea);
      return;
    }

    requestAnimationFrame(() => {
      const targetAtCursor = options.findAttributeTargetAtCursor(textarea);
      if (targetAtCursor && targetAtCursor.selector) {
        options.openInlineCssSurface(textarea, targetAtCursor.selector);
        return;
      }
      options.updateInlineCssAffordance(textarea);
    });
  });

  blocksContainer.addEventListener('wheel', (event) => {
    const target = getEventElementTarget(event);
    if (!target || !target.classList.contains('block-src')) return;
    const textarea = target;

    if (event.deltaY >= 0 || textarea.scrollTop > 0) {
      return;
    }

    const headerEditor = options.getBlockHeaderEditor(textarea);
    if (!headerEditor) return;
    if (headerEditor.style.display === 'none') return;

    event.preventDefault();
    const caret = String(headerEditor.value || '').length;
    headerEditor.focus();
    headerEditor.setSelectionRange(caret, caret);
  }, { passive: false });

  blocksContainer.addEventListener('mousedown', (event) => {
    const target = getEventElementTarget(event);
    if (!target) return;

    const item = target.closest('.autocomplete-item');
    if (!item) return;

    event.preventDefault();
    const active = options.autocompleteState.textarea;
    const index = Number.parseInt(item.dataset.index, 10);
    if (active && !Number.isNaN(index)) {
      options.acceptAutocomplete(active, index);
      active.focus();
    }
  });

  blocksContainer.addEventListener('scroll', (event) => {
    if (!event.target.classList.contains('block-src')) return;
    const { mirrorEl } = options.getAutocompleteEls(event.target);
    if (mirrorEl) mirrorEl.scrollTop = event.target.scrollTop;
  }, true);

  blocksContainer.addEventListener('mousemove', (event) => {
    const target = getEventElementTarget(event);
    if (!target || !target.classList.contains('block-src')) return;
    options.updateInlineCssAffordance(target);
  });

  if (pageEl) {
    pageEl.addEventListener('mousedown', (event) => {
      const target = getEventElementTarget(event);
      if (!target) {
        blankPagePointerDownHadOpenEditors = false;
        return;
      }

      if (!options.isBlankPageClickTarget(target, pageEl, blocksContainer)) {
        blankPagePointerDownHadOpenEditors = false;
        return;
      }

      blankPagePointerDownHadOpenEditors = options.hasOpenBlockSources() || options.inlineCssHasOpenSurface();
    });

    pageEl.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target) return;

      if (documentRef.activeElement && documentRef.activeElement.classList.contains('block-src')) {
        return;
      }

      const isBlankArea = options.isBlankPageClickTarget(target, pageEl, blocksContainer);
      if (!isBlankArea) return;

      const hadOpenSources = options.hasOpenBlockSources();
      const hadOpenInlineCss = options.inlineCssHasOpenSurface();
      const shouldOnlyCloseEditors = blankPagePointerDownHadOpenEditors || hadOpenSources || hadOpenInlineCss;

      options.closeInlineCssSurface();
      options.commitOpenSources();
      blankPagePointerDownHadOpenEditors = false;

      if (shouldOnlyCloseEditors) {
        return;
      }

      if (!options.isEditModeEnabled()) {
        return;
      }

      const index = options.getInsertIndexForY(blocksContainer, event.clientY);
      options.insertParagraphBlock(index);
    });
  }
}
