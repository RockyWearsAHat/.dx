import type { PipelineBlock } from './doc-pipeline.js';

type CssCursorTarget = { selector?: string } | null;

type InlineCssSurfaceOptions = {
  getScopedCssForSelector: (cssText: string, selector: string) => string;
  mergeScopedCssForSelector: (baseCssText: string, selector: string, declarations: string) => string;
  applyCustomCss: (cssText: string) => boolean;
  recordDocumentUndo: (action: string) => void;
  upsertCssBlock: (cssText: string) => void;
  markDocumentDirty: () => void;
  publishViewState: (effectiveCss: string) => void;
  extractCssFromDocumentModel: () => string;
  getScopedCssDeclarations: (cssText: string, selector: string) => string;
  findAttributeTargetAtCursor: (textarea: HTMLTextAreaElement) => CssCursorTarget;
  renderEditableHeader: (textarea: HTMLTextAreaElement) => void;
};

type InlineCssSurfaceState = {
  source: HTMLTextAreaElement | null;
  selector: string;
  baseCssText: string;
  selectionStart: number;
  selectionEnd: number;
  undoSeeded: boolean;
};

type BlockSourceControllerOptions = {
  buildRawSourceFromEditorParts: (headerSource: string, bodySource: string, footerSource: string) => string;
  getHeaderSourceFromEditor: (source: HTMLTextAreaElement) => string;
  getRawSourceFromEditor: (text: string) => string;
  closeInlineCssSurface: (restoreFocus?: boolean) => void;
  clearDocumentUndoSeed: (source: HTMLTextAreaElement, options?: { discardIfNoop?: boolean }) => void;
  clearEditableBodyPresentation: (wrap: HTMLElement) => void;
  tryApplyPendingExternalSource: () => void;
  getDocModel: () => { blocks?: PipelineBlock[] } | null;
  stringifyBlock: (block: PipelineBlock | null | undefined) => string;
  splitBlockSourceForEditor: (rawSource: string, blockType: string) => { headerSource: string; bodySource: string; footerSource: string };
  getRawSourceForEditor: (bodySource: string) => string;
  applyEditableBodyPresentation: (wrap: HTMLElement, block: PipelineBlock | null | undefined) => void;
  autosizeBlockSrc: (source: HTMLTextAreaElement) => void;
  updateInlineCssAffordance: (source: HTMLTextAreaElement) => void;
  normalizeBlockSourceInput?: (source: string) => string;
  parseBlock: (source: string) => PipelineBlock;
  knownBlockTypes: Set<string>;
  recordDocumentUndo: (action: string) => void;
  renderDocument: () => void;
  setStatus: (message: string) => void;
  debouncedAutosave: () => void;
  buildRenderedContent: (block: PipelineBlock | null | undefined) => Node;
  refreshDocumentCss: () => void;
  applyBlockViewPresentation?: (view: HTMLElement, block: PipelineBlock | null | undefined) => void;
};

export class InlineCssSurfaceController {
  options: InlineCssSurfaceOptions;
  state: InlineCssSurfaceState;

  constructor(options: InlineCssSurfaceOptions) {
    this.options = options;
    this.state = {
      source: null,
      selector: '',
      baseCssText: '',
      selectionStart: 0,
      selectionEnd: 0,
      undoSeeded: false,
    };
  }

  hasOpenSurface() {
    return Boolean(this.state.source);
  }

  getActiveScopedCss() {
    if (!this.state.selector || !this.state.baseCssText) {
      return '';
    }

    return this.options.getScopedCssForSelector(this.state.baseCssText, this.state.selector);
  }

  autosizeInlineCssEditor(editor: HTMLTextAreaElement | null) {
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

  ensureInlineCssSurface(source: HTMLTextAreaElement | null): HTMLElement | null {
    const srcWrap = source ? source.closest('.block-src-wrapper') : null;
    if (!srcWrap) {
      return null;
    }

    const sourceArea = srcWrap.querySelector<HTMLTextAreaElement>('.block-src');
    const bodyWrap = sourceArea ? sourceArea.closest('.block-src-body-wrap') : null;
    if (!sourceArea) {
      return null;
    }

    let surface = srcWrap.querySelector<HTMLElement>('.inline-css-surface');

    if (surface) {
      if (bodyWrap && surface.nextElementSibling !== bodyWrap) {
        srcWrap.insertBefore(surface, bodyWrap);
      }
      return surface;
    }

    surface = document.createElement('div');
    surface.className = 'inline-css-surface';
    surface.style.display = 'none';
    surface.innerHTML = '<div class="inline-css-meta inline-css-head" aria-hidden="true"></div><textarea class="inline-css-src" spellcheck="false" aria-label="Inline CSS declarations"></textarea><div class="inline-css-meta inline-css-tail" aria-hidden="true">}</div>';

    const editor = surface.querySelector<HTMLTextAreaElement>('.inline-css-src');
    if (editor) {
      editor.addEventListener('input', () => {
        const cssText = this.options.mergeScopedCssForSelector(
          this.state.baseCssText,
          this.state.selector,
          editor.value,
        );
        this.autosizeInlineCssEditor(editor);
        const scopedCss = this.options.getScopedCssForSelector(cssText, this.state.selector);
        if (this.options.applyCustomCss(scopedCss)) {
          if (!this.state.undoSeeded) {
            this.options.recordDocumentUndo('inline-css-edit');
            this.state.undoSeeded = true;
          }
          this.options.upsertCssBlock(cssText);
          this.state.baseCssText = cssText;
          this.options.markDocumentDirty();
          this.options.publishViewState(scopedCss);
        }
      });

      editor.addEventListener('keydown', (event: KeyboardEvent) => {
        if (event.key !== 'Escape') return;
        event.preventDefault();
        event.stopPropagation();
        this.closeInlineCssSurface(true);
      });
    }

    if (bodyWrap) {
      srcWrap.insertBefore(surface, bodyWrap);
    } else {
      srcWrap.appendChild(surface);
    }

    return surface;
  }

  closeInlineCssSurface(restoreFocus = false) {
    const activeSource = this.state.source;
    const selectionStart = this.state.selectionStart;
    const selectionEnd = this.state.selectionEnd;
    const srcWrap = activeSource ? activeSource.closest('.block-src-wrapper') : null;
    const surface = srcWrap
      ? srcWrap.querySelector<HTMLElement>('.inline-css-surface')
      : document.querySelector<HTMLElement>('.block-src-wrapper .inline-css-surface');

    if (surface) {
      surface.style.display = 'none';
    }

    this.state = {
      source: null,
      selector: '',
      baseCssText: '',
      selectionStart: 0,
      selectionEnd: 0,
      undoSeeded: false,
    };

    this.options.applyCustomCss('');
    this.options.publishViewState('');

    if (restoreFocus && activeSource) {
      const start = Number.isFinite(selectionStart) ? selectionStart : activeSource.value.length;
      const end = Number.isFinite(selectionEnd) ? selectionEnd : start;
      activeSource.focus();
      if (typeof activeSource.setSelectionRange === 'function') {
        activeSource.setSelectionRange(start, end);
      }
    }
  }

  openInlineCssSurface(source: HTMLTextAreaElement | null, selector: string) {
    if (!source) return;

    const surface = this.ensureInlineCssSurface(source);
    if (!surface) return;

    const editor = surface.querySelector<HTMLTextAreaElement>('.inline-css-src');
    if (!editor) return;

    const requestedSelector = String(selector || '').trim();
    const alreadyOpen = this.state.source === source
      && this.state.selector === requestedSelector
      && surface.style.display !== 'none';

    if (alreadyOpen) {
      editor.focus();
      return;
    }

    const meta = surface.querySelector<HTMLElement>('.inline-css-meta');
    if (meta) {
      meta.textContent = requestedSelector ? `${requestedSelector} {` : '{';
    }

    const cssText = this.options.extractCssFromDocumentModel();
    const declarations = this.options.getScopedCssDeclarations(cssText, requestedSelector);
    const scopedCss = this.options.getScopedCssForSelector(cssText, requestedSelector);

    editor.value = declarations || '';
    surface.style.display = 'block';
    this.state = {
      source,
      selector: requestedSelector,
      baseCssText: cssText,
      selectionStart: typeof source.selectionStart === 'number' ? source.selectionStart : source.value.length,
      selectionEnd: typeof source.selectionEnd === 'number' ? source.selectionEnd : source.value.length,
      undoSeeded: false,
    };

    this.options.applyCustomCss(scopedCss);
    this.options.publishViewState(scopedCss);

    this.autosizeInlineCssEditor(editor);

    const caret = editor.value.length;
    editor.focus();
    editor.setSelectionRange(caret, caret);
    editor.scrollTop = editor.scrollHeight;
  }

  updateInlineCssAffordance(textarea: HTMLTextAreaElement | null) {
    if (!textarea) return;
    const target = this.options.findAttributeTargetAtCursor(textarea);
    if (target && target.selector) {
      textarea.setAttribute('title', 'Option/Alt + click to edit scoped styles');
    } else {
      textarea.removeAttribute('title');
    }

    if (textarea.classList.contains('block-src')) {
      this.options.renderEditableHeader(textarea);
    }
  }
}

export class BlockSourceController {
  options: BlockSourceControllerOptions;

  constructor(options: BlockSourceControllerOptions) {
    this.options = options;
  }

  hasPendingBlockSourceChanges(source: HTMLTextAreaElement | null) {
    if (!source) {
      return false;
    }

    const originalSource = source.dataset.originalSource || '';
    const nextSource = this.options.buildRawSourceFromEditorParts(
      this.options.getHeaderSourceFromEditor(source),
      this.options.getRawSourceFromEditor(source.value),
      source.dataset.footerSource || '',
    );

    return nextSource !== originalSource;
  }

  closeBlockSrc(index: number, commitChanges: boolean) {
    const wrap = document.querySelector<HTMLElement>('.block-wrap[data-block-index="' + index + '"]');
    if (!wrap) return;

    const view = wrap.querySelector<HTMLElement>('.block-view');
    const srcWrap = wrap.querySelector<HTMLElement>('.block-src-wrapper');
    const source = wrap.querySelector<HTMLTextAreaElement>('.block-src');

    if (!view || !srcWrap || !source || srcWrap.style.display !== 'block') {
      return;
    }

    if (commitChanges) {
      this.commitBlockSrc(index);
      return;
    }

    this.options.closeInlineCssSurface();
    this.options.clearDocumentUndoSeed(source, { discardIfNoop: true });
    this.options.clearEditableBodyPresentation(wrap);
    srcWrap.style.display = 'none';
    view.style.display = '';
  }

  commitBlockSourceForHistory(index: number) {
    const wrap = document.querySelector<HTMLElement>('.block-wrap[data-block-index="' + index + '"]');
    if (!wrap) {
      return;
    }

    const source = wrap.querySelector<HTMLTextAreaElement>('.block-src');
    if (this.hasPendingBlockSourceChanges(source)) {
      this.commitBlockSrc(index);
      return;
    }

    this.closeBlockSrc(index, false);
  }

  getOpenSourceIndices(exceptIndex?: number) {
    const openWraps = Array.from(document.querySelectorAll<HTMLElement>('.block-wrap .block-src-wrapper'))
      .filter((node) => node.style.display === 'block')
      .map((node) => node.closest('.block-wrap'))
      .filter(Boolean) as HTMLElement[];

    return openWraps
        .map((wrap) => Number.parseInt(wrap.dataset.blockIndex || '', 10))
      .filter((value) => !Number.isNaN(value) && value !== exceptIndex)
      .sort((a, b) => b - a);
  }

      commitOpenSources(exceptIndex?: number) {
    const indices = this.getOpenSourceIndices(exceptIndex);

    for (const index of indices) {
      this.commitBlockSrc(index);
    }

    this.options.tryApplyPendingExternalSource();
  }

  commitOpenSourcesForHistory(exceptIndex?: number) {
    const indices = this.getOpenSourceIndices(exceptIndex);

    for (const index of indices) {
      this.commitBlockSourceForHistory(index);
    }

    this.options.tryApplyPendingExternalSource();
  }

  hasOpenBlockSources() {
    return Boolean(Array.from(document.querySelectorAll<HTMLElement>('.block-wrap .block-src-wrapper'))
      .find((node) => node.style.display === 'block'));
  }

  openBlockSrc(index: number) {
    const wrap = document.querySelector<HTMLElement>('.block-wrap[data-block-index="' + index + '"]');
    if (!wrap) return;

    const view = wrap.querySelector<HTMLElement>('.block-view');
    const srcWrap = wrap.querySelector<HTMLElement>('.block-src-wrapper');
    const source = wrap.querySelector<HTMLTextAreaElement>('.block-src');

    if (!view || !source || !srcWrap || srcWrap.style.display === 'block') {
      return;
    }

    const docModel = this.options.getDocModel();
    const block = docModel && Array.isArray(docModel.blocks) ? docModel.blocks[index] : null;
    if (!block) return;

    const rawSource = typeof block.rawSource === 'string' ? block.rawSource : this.options.stringifyBlock(block);
    const editorParts = this.options.splitBlockSourceForEditor(rawSource, String(block.type || 'paragraph'));
    source.value = this.options.getRawSourceForEditor(editorParts.bodySource);
    source.dataset.originalSource = rawSource;
    source.dataset.headerSource = editorParts.headerSource;
    source.dataset.footerSource = editorParts.footerSource;
    source.dataset.undoSeeded = '0';
    source.dataset.undoSeedAction = '';
    source.dataset.undoSeedDepth = '';
    this.options.applyEditableBodyPresentation(wrap, block);
    view.style.display = 'none';
    srcWrap.style.display = 'block';
    this.options.autosizeBlockSrc(source);
    this.options.updateInlineCssAffordance(source);
    source.focus();
  }

  commitBlockSrc(index: number) {
    const wrap = document.querySelector<HTMLElement>('.block-wrap[data-block-index="' + index + '"]');
    if (!wrap) return;

    const view = wrap.querySelector<HTMLElement>('.block-view');
    const srcWrap = wrap.querySelector<HTMLElement>('.block-src-wrapper');
    const source = wrap.querySelector<HTMLTextAreaElement>('.block-src');

    if (!view || !source || !srcWrap) {
      return;
    }

    const docModel = this.options.getDocModel();
    if (!docModel || !Array.isArray(docModel.blocks)) {
      return;
    }

    const originalSource = source.dataset.originalSource || '';
    const nextSource = this.options.buildRawSourceFromEditorParts(
      this.options.getHeaderSourceFromEditor(source),
      this.options.getRawSourceFromEditor(source.value),
      source.dataset.footerSource || '',
    );
    const normalizedSource = typeof this.options.normalizeBlockSourceInput === 'function'
      ? this.options.normalizeBlockSourceInput(nextSource)
      : nextSource;
    const parsed = this.options.parseBlock(normalizedSource);
    parsed.rawSource = normalizedSource;
    const previousBlock = docModel.blocks[index] || null;
    const previousCanonical = previousBlock ? this.options.stringifyBlock(previousBlock) : '';
    const nextCanonical = this.options.stringifyBlock(parsed);
    const sourceChanged = nextCanonical !== previousCanonical;
    const hasSeededUndo = source.dataset.undoSeeded === '1';

    if (parsed.type === 'paragraph' && !String(parsed.text || '').trim()) {
      if (sourceChanged && !hasSeededUndo) {
        this.options.recordDocumentUndo('commit-block-source');
      }
      docModel.blocks.splice(index, 1);
      this.options.clearDocumentUndoSeed(source, { discardIfNoop: !sourceChanged });
      this.options.renderDocument();
      this.options.setStatus('Block removed');
      this.options.debouncedAutosave();
      return;
    }

    if (!this.options.knownBlockTypes.has(parsed.type)) {
      if (!originalSource) {
        docModel.blocks.splice(index, 1);
        this.options.clearDocumentUndoSeed(source, { discardIfNoop: !sourceChanged });
        this.options.renderDocument();
        return;
      }
      const reverted = this.options.parseBlock(originalSource);
      reverted.rawSource = originalSource;
      docModel.blocks[index] = reverted;
      view.textContent = '';
      view.appendChild(this.options.buildRenderedContent(reverted));
      this.options.clearEditableBodyPresentation(wrap);
      srcWrap.style.display = 'none';
      view.style.display = '';
      this.options.clearDocumentUndoSeed(source, { discardIfNoop: true });
      this.options.setStatus('Reverted - string | number | boolean | null | undefined | object block type');
      return;
    }

    if (sourceChanged && !hasSeededUndo) {
      this.options.recordDocumentUndo('commit-block-source');
    }

    docModel.blocks[index] = parsed;
    if (typeof this.options.applyBlockViewPresentation === 'function') {
      this.options.applyBlockViewPresentation(view, parsed);
    }
    view.textContent = '';
    view.appendChild(this.options.buildRenderedContent(parsed));
    this.options.clearEditableBodyPresentation(wrap);
    srcWrap.style.display = 'none';
    view.style.display = '';
    this.options.refreshDocumentCss();
    this.options.clearDocumentUndoSeed(source, { discardIfNoop: !sourceChanged });
    if (sourceChanged) {
      this.options.setStatus('Unsaved changes');
      this.options.debouncedAutosave();
    }
  }
}
