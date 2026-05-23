"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.createDocumentLifecycle = createDocumentLifecycle;
function createDocumentLifecycle(options = {}) {
    const getDocModel = typeof options.getDocModel === 'function' ? options.getDocModel : () => null;
    const setDocModel = typeof options.setDocModel === 'function' ? options.setDocModel : () => { };
    const stringifyDoc = typeof options.stringifyDoc === 'function' ? options.stringifyDoc : () => '';
    const parseDoc = typeof options.parseDoc === 'function' ? options.parseDoc : () => ({ blocks: [] });
    const getLastSavedDoc = typeof options.getLastSavedDoc === 'function' ? options.getLastSavedDoc : () => '';
    const setLastSavedDoc = typeof options.setLastSavedDoc === 'function' ? options.setLastSavedDoc : () => { };
    const getPendingExternalSource = typeof options.getPendingExternalSource === 'function'
        ? options.getPendingExternalSource
        : () => null;
    const setPendingExternalSource = typeof options.setPendingExternalSource === 'function'
        ? options.setPendingExternalSource
        : () => { };
    const getIsApplyingDocHistory = typeof options.getIsApplyingDocHistory === 'function'
        ? options.getIsApplyingDocHistory
        : () => false;
    const clearDocumentRedoHistory = typeof options.clearDocumentRedoHistory === 'function'
        ? options.clearDocumentRedoHistory
        : () => { };
    const recordDocumentUndo = typeof options.recordDocumentUndo === 'function' ? options.recordDocumentUndo : () => { };
    const transitionDocSaveState = typeof options.transitionDocSaveState === 'function' ? options.transitionDocSaveState : () => { };
    const getDocSaveState = typeof options.getDocSaveState === 'function' ? options.getDocSaveState : () => '';
    const DOC_SAVE_STATES = options.DOC_SAVE_STATES || {};
    const hasActiveEditingSurface = typeof options.hasActiveEditingSurface === 'function'
        ? options.hasActiveEditingSurface
        : () => false;
    const clearStatusPersistent = typeof options.clearStatusPersistent === 'function' ? options.clearStatusPersistent : () => { };
    const setStatusPersistent = typeof options.setStatusPersistent === 'function' ? options.setStatusPersistent : () => { };
    const setStatus = typeof options.setStatus === 'function' ? options.setStatus : () => { };
    const markDocumentDirty = typeof options.markDocumentDirty === 'function' ? options.markDocumentDirty : () => { };
    const setHasDirtyWorkingCopySignal = typeof options.setHasDirtyWorkingCopySignal === 'function' ? options.setHasDirtyWorkingCopySignal : () => { };
    const postMessage = typeof options.postMessage === 'function' ? options.postMessage : () => { };
    const getNextSaveRequestId = typeof options.getNextSaveRequestId === 'function' ? options.getNextSaveRequestId : () => 1;
    const setNextSaveRequestId = typeof options.setNextSaveRequestId === 'function' ? options.setNextSaveRequestId : () => { };
    const setInFlightSaveRequestId = typeof options.setInFlightSaveRequestId === 'function'
        ? options.setInFlightSaveRequestId
        : () => { };
    const commitOpenSources = typeof options.commitOpenSources === 'function' ? options.commitOpenSources : () => { };
    const getBlocksElement = typeof options.getBlocksElement === 'function'
        ? options.getBlocksElement
        : () => document.getElementById('blocks');
    const buildBlockWrap = typeof options.buildBlockWrap === 'function' ? options.buildBlockWrap : () => null;
    const refreshDocumentCss = typeof options.refreshDocumentCss === 'function' ? options.refreshDocumentCss : () => { };
    const currentDocSourceText = typeof options.currentDocSourceText === 'function'
        ? options.currentDocSourceText
        : () => {
            const model = getDocModel();
            return model ? stringifyDoc(model) : '';
        };
    function renderDocument() {
        const model = getDocModel();
        if (!model)
            return;
        const blocksEl = getBlocksElement();
        if (!blocksEl)
            return;
        blocksEl.textContent = '';
        model.blocks.forEach((block, index) => {
            const wrap = buildBlockWrap(block, index);
            if (wrap) {
                blocksEl.appendChild(wrap);
            }
        });
        refreshDocumentCss();
    }
    function canApplyExternalSourceNow() {
        return !hasActiveEditingSurface()
            && getDocSaveState() !== DOC_SAVE_STATES.SAVING
            && getDocSaveState() !== DOC_SAVE_STATES.DIRTY;
    }
    function applyIncomingSourceText(sourceText) {
        const incomingText = String(sourceText || '');
        const previousSource = currentDocSourceText();
        if (getDocModel() && previousSource !== incomingText && !getIsApplyingDocHistory()) {
            recordDocumentUndo('external-sync');
        }
        setDocModel(parseDoc(incomingText));
        clearDocumentRedoHistory();
        setLastSavedDoc(stringifyDoc(getDocModel()));
        setPendingExternalSource(null);
        transitionDocSaveState('SYNC_CLEAN');
        renderDocument();
        clearStatusPersistent();
        setHasDirtyWorkingCopySignal(false);
        setStatus('Refreshed');
    }
    function tryApplyPendingExternalSource() {
        const pendingExternalSource = getPendingExternalSource();
        if (pendingExternalSource === null) {
            return;
        }
        if (!canApplyExternalSourceNow()) {
            return;
        }
        applyIncomingSourceText(pendingExternalSource);
    }
    function startSaveRequest(sourceText) {
        const requestId = getNextSaveRequestId();
        setNextSaveRequestId(requestId + 1);
        setInFlightSaveRequestId(requestId);
        transitionDocSaveState('START_SAVE');
        setStatusPersistent('Saving…', 'saving');
        postMessage({ type: 'save', text: sourceText, requestId });
    }
    function saveDoc() {
        if (!getDocModel())
            return;
        commitOpenSources();
        const currentDoc = currentDocSourceText();
        if (currentDoc === getLastSavedDoc()) {
            transitionDocSaveState('SYNC_CLEAN');
            clearStatusPersistent();
            return;
        }
        startSaveRequest(currentDoc);
    }
    function saveDocAuto() {
        // Autosave intentionally disabled. Persistence happens only on explicit Cmd/Ctrl+S.
    }
    function debouncedAutosave() {
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
