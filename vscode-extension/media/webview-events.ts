interface AutocompleteState {
  textarea: HTMLTextAreaElement | null;
  suggestions: string[];
}

interface AttributeTarget {
  selector: string;
}

interface BlockInteractionOptions {
  getEventElementTarget?: (event: Event) => Element | null;
  documentRef?: Document;
  windowRef?: Window;
  blocksContainer?: HTMLElement | null;
  pageEl?: HTMLElement | null;
  autocompleteState?: AutocompleteState;
  acceptAutocomplete?: (textarea: HTMLTextAreaElement, index?: number) => void;
  isEditModeEnabled?: () => boolean;
  commitOpenSources?: (index?: number) => void;
  openBlockSrc?: (index: number) => void;
  closeAutocomplete?: () => void;
  closeInlineCssSurface?: () => void;
  commitBlockSourceForHistory?: (index: number) => void;
  isRedoShortcut?: (event: KeyboardEvent) => boolean;
  performGlobalRedo?: () => boolean;
  performGlobalUndo?: () => boolean;
  setStatus?: (message: string) => void;
  commitBlockSrc?: (index: number) => void;
  saveDoc?: () => void;
  ensureDocumentUndoSeed?: (textarea: HTMLTextAreaElement | null, reason: string) => void;
  autosizeBlockSrc?: (textarea: HTMLTextAreaElement) => void;
  updateInlineCssAffordance?: (textarea: HTMLTextAreaElement) => void;
  debouncedAutosave?: () => void;
  getRawSourceFromEditor?: (text: string) => string;
  renderEditableHeader?: (textarea: HTMLTextAreaElement) => void;
  closeBlockSrc?: (index: number, shouldCommit: boolean) => void;
  renderAutocomplete?: (textarea: HTMLTextAreaElement, forceShow: boolean) => void;
  getBlockHeaderEditor?: (textarea: HTMLTextAreaElement) => HTMLTextAreaElement | null;
  updateAutocompleteSelection?: (delta: number) => void;
  findAttributeTargetAtCursor?: (textarea: HTMLTextAreaElement) => AttributeTarget | null;
  openInlineCssSurface?: (textarea: HTMLTextAreaElement, selector: string) => void;
  getAutocompleteEls?: (textarea: HTMLTextAreaElement) => { mirrorEl?: HTMLElement | null };
  isBlankPageClickTarget?: (target: Element, pageEl: HTMLElement, blocksContainer: HTMLElement) => boolean;
  hasOpenBlockSources?: () => boolean;
  inlineCssHasOpenSurface?: () => boolean;
  getInsertIndexForY?: (blocksContainer: HTMLElement, clientY: number) => number;
  insertParagraphBlock?: (index: number) => void;
}

export function registerBlockInteractionEvents(rawOptions: BlockInteractionOptions = {}): void {
  const fallbackAutocompleteState: AutocompleteState = { textarea: null, suggestions: [] };
  const options: Required<BlockInteractionOptions> = {
    getEventElementTarget: (event: Event): Element | null => (event.target instanceof Element ? event.target : null),
    documentRef: document,
    windowRef: window,
    blocksContainer: null,
    pageEl: null,
    acceptAutocomplete: (_textarea: HTMLTextAreaElement, _index?: number): void => {},
    isEditModeEnabled: (): boolean => false,
    commitOpenSources: (_index?: number): void => {},
    openBlockSrc: (_index: number): void => {},
    closeAutocomplete: (): void => {},
    closeInlineCssSurface: (): void => {},
    commitBlockSourceForHistory: (_index: number): void => {},
    isRedoShortcut: (_event: KeyboardEvent): boolean => false,
    performGlobalRedo: (): boolean => false,
    performGlobalUndo: (): boolean => false,
    setStatus: (_message: string): void => {},
    commitBlockSrc: (_index: number): void => {},
    saveDoc: (): void => {},
    ensureDocumentUndoSeed: (_textarea: HTMLTextAreaElement | null, _reason: string): void => {},
    autosizeBlockSrc: (_textarea: HTMLTextAreaElement): void => {},
    updateInlineCssAffordance: (_textarea: HTMLTextAreaElement): void => {},
    debouncedAutosave: (): void => {},
    getRawSourceFromEditor: (text: string): string => String(text || ''),
    renderEditableHeader: (_textarea: HTMLTextAreaElement): void => {},
    closeBlockSrc: (_index: number, _shouldCommit: boolean): void => {},
    renderAutocomplete: (_textarea: HTMLTextAreaElement, _forceShow: boolean): void => {},
    getBlockHeaderEditor: (_textarea: HTMLTextAreaElement): HTMLTextAreaElement | null => null,
    updateAutocompleteSelection: (_delta: number): void => {},
    findAttributeTargetAtCursor: (_textarea: HTMLTextAreaElement): AttributeTarget | null => null,
    openInlineCssSurface: (_textarea: HTMLTextAreaElement, _selector: string): void => {},
    getAutocompleteEls: (_textarea: HTMLTextAreaElement): { mirrorEl?: HTMLElement | null } => ({ mirrorEl: null }),
    isBlankPageClickTarget: (_target: Element, _pageEl: HTMLElement, _blocksContainer: HTMLElement): boolean => false,
    hasOpenBlockSources: (): boolean => false,
    inlineCssHasOpenSurface: (): boolean => false,
    getInsertIndexForY: (_blocksContainer: HTMLElement, _clientY: number): number => 0,
    insertParagraphBlock: (_index: number): void => {},
    ...rawOptions,
    autocompleteState: rawOptions.autocompleteState || fallbackAutocompleteState,
  };

  const getEventElementTarget = options.getEventElementTarget;
  const documentRef = options.documentRef;
  const windowRef = options.windowRef;
  const blocksContainer = options.blocksContainer || documentRef.getElementById('blocks');
  const pageEl = options.pageEl || documentRef.querySelector<HTMLElement>('.page');
  const toHtmlElement = (value: EventTarget | Element | null): HTMLElement | null => {
    return value instanceof HTMLElement ? value : null;
  };
  const toTextArea = (value: EventTarget | Element | null): HTMLTextAreaElement | null => {
    return value instanceof HTMLTextAreaElement ? value : null;
  };

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

        const completionEl = toHtmlElement(completionItem);
        const index = Number.parseInt(String(completionEl?.dataset.index || ''), 10);
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

      const viewElement = toHtmlElement(view);
      if (!viewElement) {
        return;
      }
      viewElement.focus();

    const wrap = view.closest('.block-wrap');
    if (!wrap) return;

      const wrapElement = toHtmlElement(wrap);
      const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
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

    const wrapElement = toHtmlElement(wrap);
    const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
    if (Number.isNaN(index)) return;

    event.preventDefault();
    options.commitOpenSources(index);
    options.openBlockSrc(index);
  });

  blocksContainer.addEventListener('focusout', (event) => {
    const target = toTextArea(event.target);
    if (!target || !target.classList.contains('block-src')) return;

    const related = toHtmlElement(event.relatedTarget);
    if (related && related.closest('.autocomplete-menu')) {
      return;
    }

    options.closeAutocomplete();
  });

  blocksContainer.addEventListener('keydown', (event) => {
    const textarea = toTextArea(event.target);
    if (!textarea) return;
    if (!textarea.classList.contains('block-src') && !textarea.classList.contains('block-edit-header')) return;

    if ((event.metaKey || event.ctrlKey) && (event.key.toLowerCase() === 'z' || event.key.toLowerCase() === 'y')) {
      event.preventDefault();
      event.stopPropagation();
      options.closeInlineCssSurface();

      const wrap = textarea.closest('.block-wrap');
      if (wrap) {
        const wrapElement = toHtmlElement(wrap);
        const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
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
      const bodyEditor = srcWrap ? toTextArea(srcWrap.querySelector('.block-src')) : null;

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (wrap) {
          const wrapElement = toHtmlElement(wrap);
          const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
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
        const wrapElement = toHtmlElement(wrap);
        const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
        if (Number.isNaN(index)) return;
        options.closeBlockSrc(index, true);
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        event.stopPropagation();
        const wrap = textarea.closest('.block-wrap');
        if (!wrap) return;
        const wrapElement = toHtmlElement(wrap);
        const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
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
      const wrapElement = toHtmlElement(wrap);
      const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
      if (Number.isNaN(index)) return;
      options.closeBlockSrc(index, true);
      return;
    }

    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      const wrap = textarea.closest('.block-wrap');
      if (!wrap) return;
      const wrapElement = toHtmlElement(wrap);
      const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
      if (Number.isNaN(index)) return;
      options.closeBlockSrc(index, true);
      return;
    }

    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      event.stopPropagation();
      const wrap = textarea.closest('.block-wrap');
      if (wrap) {
        const wrapElement = toHtmlElement(wrap);
        const index = Number.parseInt(String(wrapElement?.dataset.blockIndex || ''), 10);
        if (!Number.isNaN(index)) {
          options.commitBlockSrc(index);
        }
      }
      options.saveDoc();
    }
  });

  blocksContainer.addEventListener('input', (event) => {
    const textareaTarget = toTextArea(event.target);
    if (!textareaTarget) {
      return;
    }

    if (textareaTarget.classList.contains('block-edit-header')) {
      const header = textareaTarget;
      const srcWrap = header.closest('.block-src-wrapper');
      const source = srcWrap ? toTextArea(srcWrap.querySelector('.block-src')) : null;
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

    if (!textareaTarget.classList.contains('block-src')) return;

    const source = textareaTarget;
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

    options.autosizeBlockSrc(textareaTarget);
    options.renderAutocomplete(textareaTarget, false);
    options.updateInlineCssAffordance(textareaTarget);
    options.debouncedAutosave();
  });

  blocksContainer.addEventListener('keyup', (event) => {
    const textareaTarget = toTextArea(event.target);
    if (!textareaTarget) return;

    if (textareaTarget.classList.contains('block-edit-header')) {
      options.updateInlineCssAffordance(textareaTarget);
      return;
    }

    if (!textareaTarget.classList.contains('block-src')) return;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') {
      options.renderAutocomplete(textareaTarget, false);
    }
    options.updateInlineCssAffordance(textareaTarget);
  });

  blocksContainer.addEventListener('click', (event) => {
    const target = getEventElementTarget(event);
    const header = toTextArea(target);
    if (!header || !header.classList.contains('block-edit-header')) return;
    options.updateInlineCssAffordance(header);
  });

  blocksContainer.addEventListener('click', (event) => {
    const target = getEventElementTarget(event);
    const source = toTextArea(target);
    if (!source || !source.classList.contains('block-src')) return;
    options.renderAutocomplete(source, false);
    options.updateInlineCssAffordance(source);
  });

  blocksContainer.addEventListener('mouseup', (event) => {
    const target = getEventElementTarget(event);
    const textarea = toTextArea(target);
    if (!textarea || !textarea.classList.contains('block-edit-header')) return;

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
      const source = srcWrap ? toTextArea(srcWrap.querySelector('.block-src')) : null;

      if (styleTarget && styleTarget.selector && source) {
        options.openInlineCssSurface(source, styleTarget.selector);
        return;
      }
      options.updateInlineCssAffordance(textarea);
    });
  });

  blocksContainer.addEventListener('mouseup', (event) => {
    const target = getEventElementTarget(event);
    const textarea = toTextArea(target);
    if (!textarea || !textarea.classList.contains('block-src')) return;

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
    const textarea = toTextArea(target);
    if (!textarea || !textarea.classList.contains('block-src')) return;

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
    const itemElement = toHtmlElement(item);
    const index = Number.parseInt(String(itemElement?.dataset.index || ''), 10);
    if (active && !Number.isNaN(index)) {
      options.acceptAutocomplete(active, index);
      active.focus();
    }
  });

  blocksContainer.addEventListener('scroll', (event) => {
    const textarea = toTextArea(event.target);
    if (!textarea || !textarea.classList.contains('block-src')) return;
    const { mirrorEl } = options.getAutocompleteEls(textarea);
    if (mirrorEl) mirrorEl.scrollTop = textarea.scrollTop;
  }, true);

  blocksContainer.addEventListener('mousemove', (event) => {
    const target = getEventElementTarget(event);
    const source = toTextArea(target);
    if (!source || !source.classList.contains('block-src')) return;
    options.updateInlineCssAffordance(source);
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
