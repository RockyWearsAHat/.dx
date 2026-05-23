import type { parseDoc as parseDocFn } from './webview-doc-model.js';
import type { PipelineBlock } from './doc-pipeline.js';

type DocumentModel = ReturnType<typeof parseDocFn>;

interface SaveStates {
  SAVING?: string;
  DIRTY?: string;
}

interface SaveMessage {
  type: 'save';
  text: string;
  requestId: number;
}

interface DocumentLifecycleOptions {
  getDocModel?: () => DocumentModel | null;
  setDocModel?: (model: DocumentModel) => void;
  stringifyDoc?: (model: DocumentModel) => string;
  parseDoc?: (sourceText: string) => DocumentModel;
  getLastSavedDoc?: () => string;
  setLastSavedDoc?: (sourceText: string) => void;
  getPendingExternalSource?: () => string | null;
  setPendingExternalSource?: (sourceText: string | null) => void;
  getIsApplyingDocHistory?: () => boolean;
  clearDocumentRedoHistory?: () => void;
  recordDocumentUndo?: (reason: string) => void;
  transitionDocSaveState?: (eventName: string) => void;
  getDocSaveState?: () => string;
  DOC_SAVE_STATES?: SaveStates;
  hasActiveEditingSurface?: () => boolean;
  clearStatusPersistent?: () => void;
  setStatusPersistent?: (message: string, kind: string) => void;
  setStatus?: (message: string) => void;
  markDocumentDirty?: () => void;
  setHasDirtyWorkingCopySignal?: (value: boolean) => void;
  postMessage?: (message: SaveMessage) => void;
  getNextSaveRequestId?: () => number;
  setNextSaveRequestId?: (requestId: number) => void;
  setInFlightSaveRequestId?: (requestId: number) => void;
  commitOpenSources?: (exceptIndex?: number) => void;
  getBlocksElement?: () => HTMLElement | null;
  buildBlockWrap?: (block: PipelineBlock, index: number) => Node | null;
  refreshDocumentCss?: () => void;
  currentDocSourceText?: () => string;
}

interface DocumentLifecycleController {
  applyIncomingSourceText: (sourceText: string) => void;
  canApplyExternalSourceNow: () => boolean;
  debouncedAutosave: () => void;
  renderDocument: () => void;
  saveDoc: () => void;
  saveDocAuto: () => void;
  startSaveRequest: (sourceText: string) => void;
  tryApplyPendingExternalSource: () => void;
}

export function createDocumentLifecycle(options: DocumentLifecycleOptions = {}): DocumentLifecycleController {
  const getDocModel = typeof options.getDocModel === 'function' ? options.getDocModel : (): DocumentModel | null => null;
  const setDocModel = typeof options.setDocModel === 'function' ? options.setDocModel : (_model: DocumentModel): void => {};
  const stringifyDoc = typeof options.stringifyDoc === 'function' ? options.stringifyDoc : (_model: DocumentModel): string => '';
  const parseDoc = typeof options.parseDoc === 'function' ? options.parseDoc : (_sourceText: string): DocumentModel => ({ blocks: [] });
  const getLastSavedDoc = typeof options.getLastSavedDoc === 'function' ? options.getLastSavedDoc : (): string => '';
  const setLastSavedDoc = typeof options.setLastSavedDoc === 'function' ? options.setLastSavedDoc : (_sourceText: string): void => {};
  const getPendingExternalSource = typeof options.getPendingExternalSource === 'function'
    ? options.getPendingExternalSource
    : (): string | null => null;
  const setPendingExternalSource = typeof options.setPendingExternalSource === 'function'
    ? options.setPendingExternalSource
    : (_sourceText: string | null): void => {};
  const getIsApplyingDocHistory = typeof options.getIsApplyingDocHistory === 'function'
    ? options.getIsApplyingDocHistory
    : (): boolean => false;
  const clearDocumentRedoHistory = typeof options.clearDocumentRedoHistory === 'function'
    ? options.clearDocumentRedoHistory
    : (): void => {};
  const recordDocumentUndo = typeof options.recordDocumentUndo === 'function' ? options.recordDocumentUndo : (_reason: string): void => {};
  const transitionDocSaveState = typeof options.transitionDocSaveState === 'function' ? options.transitionDocSaveState : (_eventName: string): void => {};
  const getDocSaveState = typeof options.getDocSaveState === 'function' ? options.getDocSaveState : (): string => '';
  const DOC_SAVE_STATES: SaveStates = options.DOC_SAVE_STATES || {};
  const hasActiveEditingSurface = typeof options.hasActiveEditingSurface === 'function'
    ? options.hasActiveEditingSurface
    : (): boolean => false;
  const clearStatusPersistent = typeof options.clearStatusPersistent === 'function' ? options.clearStatusPersistent : (): void => {};
  const setStatusPersistent = typeof options.setStatusPersistent === 'function' ? options.setStatusPersistent : (_message: string, _kind: string): void => {};
  const setStatus = typeof options.setStatus === 'function' ? options.setStatus : (_message: string): void => {};
  const markDocumentDirty = typeof options.markDocumentDirty === 'function' ? options.markDocumentDirty : (): void => {};
  const setHasDirtyWorkingCopySignal = typeof options.setHasDirtyWorkingCopySignal === 'function' ? options.setHasDirtyWorkingCopySignal : (_value: boolean): void => {};
  const postMessage = typeof options.postMessage === 'function' ? options.postMessage : (_message: SaveMessage): void => {};
  const getNextSaveRequestId = typeof options.getNextSaveRequestId === 'function' ? options.getNextSaveRequestId : (): number => 1;
  const setNextSaveRequestId = typeof options.setNextSaveRequestId === 'function' ? options.setNextSaveRequestId : (_requestId: number): void => {};
  const setInFlightSaveRequestId = typeof options.setInFlightSaveRequestId === 'function'
    ? options.setInFlightSaveRequestId
    : (_requestId: number): void => {};
  const commitOpenSources = typeof options.commitOpenSources === 'function' ? options.commitOpenSources : (_exceptIndex?: number): void => {};
  const getBlocksElement = typeof options.getBlocksElement === 'function'
    ? options.getBlocksElement
    : (): HTMLElement | null => document.getElementById('blocks');
  const buildBlockWrap = typeof options.buildBlockWrap === 'function' ? options.buildBlockWrap : (_block: PipelineBlock, _index: number): Node | null => null;
  const refreshDocumentCss = typeof options.refreshDocumentCss === 'function' ? options.refreshDocumentCss : (): void => {};
  const currentDocSourceText = typeof options.currentDocSourceText === 'function'
    ? options.currentDocSourceText
    : (): string => {
        const model = getDocModel();
        return model ? stringifyDoc(model) : '';
      };

  function renderDocument(): void {
    const model = getDocModel();
    if (!model) return;

    const blocksEl = getBlocksElement();
    if (!blocksEl) return;

    blocksEl.textContent = '';

    model.blocks.forEach((block: PipelineBlock, index: number) => {
      const wrap = buildBlockWrap(block, index);
      if (wrap) {
        blocksEl.appendChild(wrap);
      }
    });

    refreshDocumentCss();
  }

  function canApplyExternalSourceNow(): boolean {
    return !hasActiveEditingSurface()
      && getDocSaveState() !== DOC_SAVE_STATES.SAVING
      && getDocSaveState() !== DOC_SAVE_STATES.DIRTY;
  }

  function applyIncomingSourceText(sourceText: string): void {
    const incomingText = String(sourceText || '');
    const previousSource = currentDocSourceText();

    if (getDocModel() && previousSource !== incomingText && !getIsApplyingDocHistory()) {
      recordDocumentUndo('external-sync');
    }

    const nextModel = parseDoc(incomingText);
    setDocModel(nextModel);
    clearDocumentRedoHistory();
    setLastSavedDoc(stringifyDoc(nextModel));
    setPendingExternalSource(null);
    transitionDocSaveState('SYNC_CLEAN');
    renderDocument();
    clearStatusPersistent();
    setHasDirtyWorkingCopySignal(false);
    setStatus('Refreshed');
  }

  function tryApplyPendingExternalSource(): void {
    const pendingExternalSource = getPendingExternalSource();
    if (pendingExternalSource === null) {
      return;
    }

    if (!canApplyExternalSourceNow()) {
      return;
    }

    applyIncomingSourceText(pendingExternalSource);
  }

  function startSaveRequest(sourceText: string): void {
    const requestId = getNextSaveRequestId();
    setNextSaveRequestId(requestId + 1);
    setInFlightSaveRequestId(requestId);

    transitionDocSaveState('START_SAVE');
    setStatusPersistent('Saving…', 'saving');
    postMessage({ type: 'save', text: sourceText, requestId });
  }

  function saveDoc(): void {
    if (!getDocModel()) return;
    commitOpenSources();

    const currentDoc = currentDocSourceText();
    if (currentDoc === getLastSavedDoc()) {
      transitionDocSaveState('SYNC_CLEAN');
      clearStatusPersistent();
      return;
    }

    startSaveRequest(currentDoc);
  }

  function saveDocAuto(): void {
    // Autosave intentionally disabled. Persistence happens only on explicit Cmd/Ctrl+S.
  }

  function debouncedAutosave(): void {
    markDocumentDirty();
  }

  return {
    applyIncomingSourceText,
    canApplyExternalSourceNow,
    debouncedAutosave,
    renderDocument,
    saveDoc,
    saveDocAuto,
    startSaveRequest,
    tryApplyPendingExternalSource,
  };
}
