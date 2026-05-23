import { DOC_SAVE_STATES, DOC_SAVE_TRANSITIONS, DOC_VIEW_STATES, DOC_VIEW_TRANSITIONS } from './webview-fsm.mjs';
import {
  extractCssFromDocumentModel as extractCssFromModel,
  extractStylesheetLinksFromDocumentModel as extractStylesheetLinksFromModel,
  findCssBlockIndex as findCssBlockIndexInModel,
  KNOWN_BLOCK_TYPES,
  listItemText,
  parseAttributes,
  parseBlock,
  parseDoc,
  stringifyBlock,
  stringifyDoc,
} from './webview-doc-model.js';
import { BlockSourceController, InlineCssSurfaceController } from './webview-edit-controllers.js';
import {
  CallbackUndoRedoController,
  TableDrivenStateMachine,
  TransitionHistory,
} from './webview-state-core.js';
import { createUiStateController } from './webview-ui-state.js';

const vscode = typeof acquireVsCodeApi === 'function' ? acquireVsCodeApi() : { postMessage: () => {} };

let docModel = null;
let currentTheme = 'auto';
let loadNoteEl = null;
let currentDocPath = 'unknown.dx';
let workspaceBaseUri = '';
const appearanceStorageKey = 'docdb.appearance.v1';
const editModeStorageKey = 'docdb.edit-mode.v1';
let currentAppearance = {
  paper: 'white',
  density: 'comfortable',
  scale: 100,
};
let chromeRevealTimer = null;
let loadingGuardTimer = null;
let customCssSheet = null;

let blankPagePointerDownHadOpenEditors = false;

let docViewState = DOC_VIEW_STATES.BOOTSTRAPPING;
let docSaveState = DOC_SAVE_STATES.IDLE;
let lastSavedDoc = '';
let lastSaveErrorMessage = '';
let inFlightSaveRequestId = 0;
let nextSaveRequestId = 1;
let pendingExternalSource = null;
const FSM_HISTORY_LIMIT = 32;
const DOC_HISTORY_LIMIT = 120;
const DOC_EDITOR_UNDO_ACTIONS = new Set([
  'live-edit',
  'block-live-edit',
  'header-live-edit',
  'header-delete-join',
]);
let isApplyingDocHistory = false;
let hasDirtyWorkingCopySignal = false;

const docViewMachine = new TableDrivenStateMachine(
  'doc-view',
  DOC_VIEW_STATES.BOOTSTRAPPING,
  DOC_VIEW_TRANSITIONS,
);
const docSaveMachine = new TableDrivenStateMachine(
  'doc-save',
  DOC_SAVE_STATES.IDLE,
  DOC_SAVE_TRANSITIONS,
);
const fsmTransitionHistory = new TransitionHistory(FSM_HISTORY_LIMIT);
const documentHistory = new CallbackUndoRedoController({
  limit: DOC_HISTORY_LIMIT,
  captureSnapshot: () => cloneDocModel(docModel),
  restoreSnapshot: (snapshot) => applyDocumentSnapshot(snapshot, null),
});
let inlineCssController = null;
let blockSourceController = null;
let uiStateController = null;

function captureFsmSnapshot() {
  return {
    docViewState,
    docSaveState,
    pendingExternalSource,
    inFlightSaveRequestId,
    lastSavedDoc,
    lastSaveErrorMessage,
  };
}

function restoreFsmSnapshot(snapshot) {
  if (!snapshot || typeof snapshot !== 'object') {
    return false;
  }

  docViewState = snapshot.docViewState || DOC_VIEW_STATES.BOOTSTRAPPING;
  docSaveState = snapshot.docSaveState || DOC_SAVE_STATES.IDLE;
  docViewMachine.state = docViewState;
  docSaveMachine.state = docSaveState;
  pendingExternalSource = typeof snapshot.pendingExternalSource === 'string' ? snapshot.pendingExternalSource : null;
  inFlightSaveRequestId = Number(snapshot.inFlightSaveRequestId || 0);
  lastSavedDoc = String(snapshot.lastSavedDoc || '');
  lastSaveErrorMessage = String(snapshot.lastSaveErrorMessage || '');
  updateRuntimeStateAttributes();
  syncVisibleStateFromMachine();

  return true;
}

function undoLastFsmTransition() {
  return fsmTransitionHistory.undo(restoreFsmSnapshot);
}

function redoLastFsmTransition() {
  return fsmTransitionHistory.redo(restoreFsmSnapshot);
}

function performGlobalUndo() {
  if (undoDocumentChange()) {
    return 'document';
  }

  if (undoLastFsmTransition()) {
    setStatus('Undid state change');
    return 'fsm';
  }

  return null;
}

function performGlobalRedo() {
  if (redoDocumentChange()) {
    return 'document';
  }

  if (redoLastFsmTransition()) {
    setStatus('Redid state change');
    return 'fsm';
  }

  return null;
}

function isRedoShortcut(event) {
  if (!event || !(event.metaKey || event.ctrlKey)) {
    return false;
  }

  const key = String(event.key || '').toLowerCase();
  return key === 'y' || (key === 'z' && event.shiftKey);
}

function syncEditModeIndicators(editEnabled) {
  const toggle = document.getElementById('ui-chrome-edit-toggle');
  const modePill = document.getElementById('mode-pill');

  if (toggle) {
    toggle.setAttribute('aria-pressed', editEnabled ? 'true' : 'false');
  }

  if (modePill) {
    modePill.dataset.mode = editEnabled ? 'edit' : 'read';
    modePill.textContent = editEnabled ? 'Editing' : 'Read only';
  }
}

function syncVisibleStateFromMachine() {
  const editEnabled = isEditModeState(docViewState);

  syncEditModeIndicators(editEnabled);

  if (docSaveState === DOC_SAVE_STATES.IDLE) {
    clearStatusPersistent();
  } else if (docSaveState === DOC_SAVE_STATES.DIRTY) {
    setStatusPersistent('Unsaved changes', 'dirty');
  } else if (docSaveState === DOC_SAVE_STATES.SAVING) {
    setStatusPersistent('Saving…', 'saving');
  } else if (docSaveState === DOC_SAVE_STATES.SAVED) {
    setStatusPersistent('Saved', 'saved');
  } else if (docSaveState === DOC_SAVE_STATES.ERROR) {
    setStatusPersistent(lastSaveErrorMessage || 'Save failed', 'error');
  }
}

function cloneDocModel(model) {
  return JSON.parse(JSON.stringify(model || { blocks: [] }));
}

function applyDocumentSnapshot(snapshot, mode = null) {
  if (!snapshot) {
    return false;
  }

  isApplyingDocHistory = true;
  try {
    docModel = cloneDocModel(snapshot);
    renderDocument();
    syncDocumentSaveStateFromModel();
    if (mode === 'redo') {
      setStatus('Redid change');
    } else if (mode === 'undo') {
      setStatus('Undid change');
    }
  } finally {
    isApplyingDocHistory = false;
  }

  return true;
}

function resetDocumentHistory() {
  documentHistory.clear();
}

function markDocumentDirty() {
  reconcileSaveStateFromModel(true);
}

function syncDocumentSaveStateFromModel() {
  reconcileSaveStateFromModel(true);
}

function reconcileSaveStateFromModel(emitDirtySync = true) {
  const latestSource = currentDocSourceText();
  const isDirty = latestSource !== lastSavedDoc;
  const hadDirtyWorkingCopySignal = hasDirtyWorkingCopySignal;

  if (isDirty) {
    transitionDocSaveState('MARK_DIRTY');
    setStatusPersistent('Unsaved changes', 'dirty');
    hasDirtyWorkingCopySignal = true;

    if (emitDirtySync) {
      vscode.postMessage({
        type: 'mark-dirty',
        text: latestSource,
      });
    }
    return;
  }

  transitionDocSaveState('SYNC_CLEAN');
  clearStatusPersistent();
  hasDirtyWorkingCopySignal = false;

  // Flush the clean source back to the backing working copy when needed so
  // SCM status returns to clean after reverting edits.
  if (emitDirtySync && hadDirtyWorkingCopySignal) {
    vscode.postMessage({
      type: 'mark-dirty',
      text: latestSource,
    });
  }
}

function recordDocumentUndo(action = 'edit') {
  if (!docModel || isApplyingDocHistory) {
    return;
  }

  documentHistory.push(String(action || 'edit'));
}

function ensureDocumentUndoSeed(source, action = 'live-edit') {
  if (!source) {
    return;
  }

  if (source.dataset.undoSeeded === '1') {
    return;
  }

  recordDocumentUndo(action);
  source.dataset.undoSeeded = '1';
  source.dataset.undoSeedAction = String(action || 'live-edit');
  source.dataset.undoSeedDepth = String(documentHistory.undoDepth);
}

function clearDocumentUndoSeed(source, options = {}) {
  if (!source) {
    return;
  }

  const discardIfNoop = Boolean(options.discardIfNoop);
  const hasSeededUndo = source.dataset.undoSeeded === '1';
  const seedAction = String(source.dataset.undoSeedAction || '');
  const seedDepth = Number.parseInt(source.dataset.undoSeedDepth || '', 10);

  if (
    discardIfNoop
    && hasSeededUndo
    && Number.isInteger(seedDepth)
    && seedDepth === documentHistory.undoDepth
    && documentHistory.undoDepth > 0
    && DOC_EDITOR_UNDO_ACTIONS.has(seedAction)
  ) {
    const lastEntry = documentHistory.peekUndo();
    if (lastEntry && String(lastEntry.action || '') === seedAction) {
      documentHistory.popUndo();
    }
  }

  source.dataset.undoSeeded = '0';
  source.dataset.undoSeedAction = '';
  source.dataset.undoSeedDepth = '';
}

function applyDocumentHistorySnapshot(entry, mode) {
  if (!entry || !entry.snapshot) {
    return false;
  }

  return applyDocumentSnapshot(entry.snapshot, mode);
}

function undoDocumentChange() {
  if (!docModel) {
    return false;
  }

  const previous = documentHistory.undo();
  if (previous) {
    setStatus('Undid change');
  }
  return Boolean(previous);
}

function redoDocumentChange() {
  if (!docModel) {
    return false;
  }

  const next = documentHistory.redo();
  if (next) {
    setStatus('Redid change');
  }
  return Boolean(next);
}

function isEditModeState(state) {
  return state === DOC_VIEW_STATES.LOADING_EDIT || state === DOC_VIEW_STATES.READY_EDIT;
}

function isReadyViewState(state) {
  return state === DOC_VIEW_STATES.READY_EDIT || state === DOC_VIEW_STATES.READY_READ;
}

function updateRuntimeStateAttributes() {
  const page = document.querySelector('.page');
  if (!page) {
    return;
  }

  const editEnabled = isEditModeState(docViewState);
  const isReady = isReadyViewState(docViewState);

  page.dataset.editMode = editEnabled ? 'true' : 'false';
  page.dataset.ready = isReady ? 'true' : 'false';
  page.dataset.fsmDocState = docViewState;
  page.dataset.fsmSaveState = docSaveState;
  page.setAttribute('aria-busy', isReady ? 'false' : 'true');
}

function transitionDocViewState(event) {
  if (!docViewMachine.transition(event)) {
    return false;
  }

  const before = captureFsmSnapshot();
  docViewState = docViewMachine.state;
  updateRuntimeStateAttributes();
  fsmTransitionHistory.record({
    machine: 'doc-view',
    event,
    before,
    after: captureFsmSnapshot(),
  });
  return true;
}

function transitionDocSaveState(event) {
  if (!docSaveMachine.transition(event)) {
    return false;
  }

  const before = captureFsmSnapshot();
  docSaveState = docSaveMachine.state;
  updateRuntimeStateAttributes();
  fsmTransitionHistory.record({
    machine: 'doc-save',
    event,
    before,
    after: captureFsmSnapshot(),
  });
  return true;
}

function currentDocSourceText() {
  if (!docModel) {
    return '';
  }

  return stringifyDoc(docModel);
}

function canApplyExternalSourceNow() {
  return !hasActiveEditingSurface()
    && docSaveState !== DOC_SAVE_STATES.SAVING
    && docSaveState !== DOC_SAVE_STATES.DIRTY;
}

function applyIncomingSourceText(sourceText) {
  const incomingText = String(sourceText || '');
  const previousSource = currentDocSourceText();

  if (docModel && previousSource !== incomingText && !isApplyingDocHistory) {
    recordDocumentUndo('external-sync');
  }

  docModel = parseDoc(incomingText);
  documentHistory.clearRedo();
  lastSavedDoc = stringifyDoc(docModel);
  pendingExternalSource = null;
  transitionDocSaveState('SYNC_CLEAN');
  renderDocument();
  clearStatusPersistent();
  hasDirtyWorkingCopySignal = false;
  setStatus('Refreshed');
}

function tryApplyPendingExternalSource() {
  if (pendingExternalSource === null) {
    return;
  }

  if (!canApplyExternalSourceNow()) {
    return;
  }

  applyIncomingSourceText(pendingExternalSource);
}

function startSaveRequest(sourceText) {
  const requestId = nextSaveRequestId;
  nextSaveRequestId += 1;
  inFlightSaveRequestId = requestId;

  transitionDocSaveState('START_SAVE');
  setStatusPersistent('Saving…', 'saving');
  vscode.postMessage({ type: 'save', text: sourceText, requestId });
}

function debouncedAutosave() {
  markDocumentDirty();
}

function hasActiveEditingSurface() {
  ensureControllers();
  return hasOpenBlockSources() || inlineCssController.hasOpenSurface();
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

function getActiveScopedCss() {
  ensureControllers();
  return inlineCssController.getActiveScopedCss();
}

function extractCssFromDocumentModel() {
  return extractCssFromModel(docModel);
}

function extractStylesheetLinksFromDocumentModel() {
  return extractStylesheetLinksFromModel(docModel);
}

function resolveStylesheetHref(href) {
  const raw = String(href || '').trim();

  if (!raw) {
    return '';
  }

  if (/^(https?:|data:|vscode-)/i.test(raw)) {
    return raw;
  }

  if (!workspaceBaseUri) {
    return '';
  }

  const normalized = raw.replace(/^\.\//, '').replace(/^\/+/, '');
  return workspaceBaseUri + '/' + normalized;
}

function applyExternalStylesheetLinks(entries) {
  const head = document.head;

  if (!head) {
    return;
  }

  const activeKeys = new Set();

  for (const entry of entries || []) {
    const resolvedHref = resolveStylesheetHref(entry.href);

    if (!resolvedHref) {
      continue;
    }

    const media = String(entry.media || '').trim();
    const key = media ? resolvedHref + '|' + media : resolvedHref;
    activeKeys.add(key);

    let linkEl = head.querySelector(`link[data-doc-stylesheet-key="${CSS.escape(key)}"]`);

    if (!linkEl) {
      linkEl = document.createElement('link');
      linkEl.rel = 'stylesheet';
      linkEl.dataset.docStylesheetKey = key;
      head.appendChild(linkEl);
    }

    linkEl.href = resolvedHref;

    if (media) {
      linkEl.media = media;
    } else {
      linkEl.removeAttribute('media');
    }
  }

  head.querySelectorAll('link[data-doc-stylesheet-key]').forEach((node) => {
    const key = String(node.dataset.docStylesheetKey || '');
    if (!activeKeys.has(key)) {
      node.remove();
    }
  });
}

function refreshDocumentCss() {
  const effectiveCss = extractCssFromDocumentModel();
  applyExternalStylesheetLinks(extractStylesheetLinksFromDocumentModel());
  applyCustomCss(effectiveCss);
  publishViewState(effectiveCss);
}

function publishViewState(effectiveCss) {
  const resolvedTheme = String(document.body && document.body.dataset ? document.body.dataset.resolvedTheme || 'dark' : 'dark');
  const sourceText = docModel ? stringifyDoc(docModel) : '';
  const viewportWidth = Number.isFinite(window.innerWidth) ? Math.max(1, Math.round(window.innerWidth)) : null;
  const viewportHeight = Number.isFinite(window.innerHeight) ? Math.max(1, Math.round(window.innerHeight)) : null;
  const pixelRatio = Number.isFinite(window.devicePixelRatio) ? Number(window.devicePixelRatio) : null;

  vscode.postMessage({
    type: 'view-state',
    payload: {
      docPath: currentDocPath,
      theme: currentTheme,
      resolvedTheme,
      fsm: {
        documentState: docViewState,
        saveState: docSaveState,
        historyLength: fsmTransitionHistory.length,
        lastTransition: fsmTransitionHistory.lastTransition,
      },
      documentHistory: {
        undoDepth: documentHistory.undoDepth,
        redoDepth: documentHistory.redoDepth,
      },
      sourceText,
      appearance: {
        paper: currentAppearance.paper,
        density: currentAppearance.density,
        scale: currentAppearance.scale,
      },
      viewport: {
        width: viewportWidth,
        height: viewportHeight,
        pixelRatio,
      },
      effectiveCss: String(effectiveCss || ''),
    },
  });
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

function isBulletedListType(type) {
  return type === 'list' || type === 'bulleted-list';
}

function isNumberedListType(type) {
  return type === 'numbered-list';
}

// ---------------------------------------------------------------------------
// Inline link support — [label](relative/path.dx) syntax in paragraph/quote/list text
// ---------------------------------------------------------------------------

function parseInlineLinks(text) {
  const tokens = [];
  const pattern = /\[([^\]]+)\]\(([^)]+)\)/g;
  let lastIndex = 0;
  let match = pattern.exec(text);
  while (match !== null) {
    if (match.index > lastIndex) {
      tokens.push({ type: 'text', value: text.slice(lastIndex, match.index) });
    }
    tokens.push({ type: 'link', label: match[1], href: match[2] });
    lastIndex = match.index + match[0].length;
    match = pattern.exec(text);
  }
  if (lastIndex < text.length) {
    tokens.push({ type: 'text', value: text.slice(lastIndex) });
  }
  return tokens;
}

function setInlineContent(el, text) {
  const tokens = parseInlineLinks(String(text || ''));
  if (tokens.length === 1 && tokens[0].type === 'text') {
    el.textContent = tokens[0].value;
    return;
  }
  el.textContent = '';
  for (const token of tokens) {
    if (token.type === 'text') {
      el.appendChild(document.createTextNode(token.value));
    } else {
      const a = document.createElement('a');
      a.className = 'doc-link';
      a.textContent = token.label;
      a.dataset.docHref = token.href;
      a.href = '#';
      el.appendChild(a);
    }
  }
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
    setInlineContent(paragraph, block.text || '');
    return applyBlockDecorations(paragraph);
  }

  if (block.type === 'image') {
    const figure = document.createElement('figure');
    figure.className = 'image-wrap';

    const image = document.createElement('img');
    const rawSrc = String(block.src || '');
    if (rawSrc.startsWith('data:') || rawSrc.startsWith('http://') || rawSrc.startsWith('https://') || rawSrc.startsWith('vscode-')) {
      image.src = rawSrc;
    } else if (rawSrc && workspaceBaseUri) {
      image.src = workspaceBaseUri + '/' + rawSrc.replace(/^\//, '');
    } else {
      image.src = rawSrc;
    }
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

  if (block.type === 'style' || block.type === 'stylesheet') {
    const hidden = document.createElement('span');
    hidden.hidden = true;
    hidden.setAttribute('aria-hidden', 'true');
    return applyBlockDecorations(hidden);
  }

  if (block.type === 'rule') {
    return applyBlockDecorations(document.createElement('hr'));
  }

  if (block.type === 'quote') {
    const quote = document.createElement('blockquote');
    const paragraph = document.createElement('p');
    setInlineContent(paragraph, block.text || '');
    quote.appendChild(paragraph);
    return applyBlockDecorations(quote);
  }

  if (isBulletedListType(block.type)) {
    const ul = document.createElement('ul');
    (block.items || []).forEach((item) => {
      const li = document.createElement('li');
      setInlineContent(li, listItemText(item));
      ul.appendChild(li);
    });
    return applyBlockDecorations(ul);
  }

  if (isNumberedListType(block.type)) {
    const ol = document.createElement('ol');
    (block.items || []).forEach((item) => {
      const li = document.createElement('li');
      setInlineContent(li, listItemText(item));
      ol.appendChild(li);
    });
    return applyBlockDecorations(ol);
  }

  if (block.type === 'checklist') {
    const ul = document.createElement('ul');
    ul.className = 'checklist-wrap';
    (block.items || []).forEach((item, itemIndex) => {
      const li = document.createElement('li');
      const checkbox = document.createElement('input');
      checkbox.type = 'checkbox';
      checkbox.checked = Boolean(item.checked);
      checkbox.dataset.itemIndex = String(itemIndex);
      const span = document.createElement('span');
      setInlineContent(span, item.text);
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

  for (const token of splitClassNames(block && block.className)) {
    wrap.classList.add(token);
  }

  if (block && block.id) {
    wrap.dataset.blockId = String(block.id);
  }

  const view = document.createElement('div');
  view.className = 'block-view';
  view.setAttribute('role', 'article');
  view.setAttribute('tabindex', '0');
  view.setAttribute('aria-label', `Block ${index + 1}: ${block.type}`);
  view.appendChild(buildRenderedContent(block));

  const srcWrap = document.createElement('div');
  srcWrap.className = 'block-src-wrapper';
  srcWrap.style.display = 'none';

  const header = document.createElement('textarea');
  header.className = 'block-edit-header';
  header.setAttribute('aria-label', 'Edit block header');
  header.spellcheck = false;
  header.wrap = 'off';
  header.style.display = 'none';

  const bodyWrap = document.createElement('div');
  bodyWrap.className = 'block-src-body-wrap';

  const mirror = document.createElement('div');
  mirror.className = 'block-src-mirror';
  mirror.setAttribute('aria-hidden', 'true');

  const source = document.createElement('textarea');
  source.className = 'block-src';
  source.setAttribute('aria-label', 'Edit block content');
  source.spellcheck = false;
  source.wrap = 'off';

  const menu = document.createElement('div');
  menu.className = 'autocomplete-menu';
  menu.setAttribute('role', 'listbox');
  menu.setAttribute('aria-label', 'Autocomplete suggestions');
  menu.style.display = 'none';

  bodyWrap.appendChild(mirror);
  bodyWrap.appendChild(source);
  bodyWrap.appendChild(menu);

  srcWrap.appendChild(header);
  srcWrap.appendChild(bodyWrap);

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
  return String(rawSource || '')
    .replace(/\r\n/g, '\n')
    .replace(/\n::end\s*$/, '');  // Strip implicit ::end tag from editor view
}

function getRawSourceFromEditor(editorValue) {
  return String(editorValue || '').replace(/\r\n/g, '\n');
}

function splitBlockSourceForEditor(rawSource, blockType) {
  const normalized = String(rawSource || '').replace(/\r\n/g, '\n');
  const trimmed = normalized.trim();

  if (!trimmed.startsWith('::')) {
    return {
      headerSource: '',
      bodySource: normalized,
      footerSource: '',
    };
  }

  const lines = normalized.split('\n');
  const headerSource = String(lines[0] || '').trimEnd();
  let endIdx = lines.length;

  for (let index = 1; index < lines.length; index += 1) {
    if (lines[index].trim() === '::end') {
      endIdx = index;
      break;
    }
  }

  return {
    headerSource,
    bodySource: lines.slice(1, endIdx).join('\n'),
    footerSource: blockType === 'rule' ? '' : (endIdx < lines.length ? '::end' : ''),
  };
}

function buildRawSourceFromEditorParts(headerSource, bodySource, footerSource) {
  const header = String(headerSource || '').trimEnd();
  const body = String(bodySource || '').replace(/\r\n/g, '\n');
  const footer = String(footerSource || '').trim();

  if (!header) {
    return body;
  }

  const parts = [header];
  if (body.length > 0) {
    parts.push(body);
  }
  if (footer) {
    parts.push(footer);
  }

  return parts.join('\n');
}

function getBlockHeaderEditor(textarea) {
  if (!textarea) return null;
  if (textarea.classList && textarea.classList.contains('block-edit-header')) {
    return textarea;
  }
  const srcWrap = textarea.closest('.block-src-wrapper');
  return srcWrap ? srcWrap.querySelector('.block-edit-header') : null;
}

function getHeaderSourceFromEditor(textarea) {
  const headerEditor = getBlockHeaderEditor(textarea);
  if (!headerEditor) {
    return String(textarea?.dataset?.headerSource || '');
  }
  return getRawSourceFromEditor(headerEditor.value || '');
}

function renderEditableHeader(textarea) {
  if (!textarea) return;

  const headerEl = getBlockHeaderEditor(textarea);
  const headerSource = String(textarea.dataset.headerSource || '');

  if (!headerEl) {
    return;
  }

  if (!headerSource.trim()) {
    headerEl.value = '';
    headerEl.style.display = 'none';
    headerEl.classList.remove('tag-mode', 'paragraph-mode');
    headerEl.removeAttribute('title');
    headerEl.style.height = '0px';
    return;
  }

  headerEl.value = headerSource;
  headerEl.style.display = 'block';
  headerEl.setAttribute('title', 'Type block tag (for example ::heading level=1) or plain text');
  headerEl.style.height = '0px';
  headerEl.style.height = headerEl.scrollHeight + 'px';

  const inTagMode = String(headerEl.value || '').startsWith(':');
  headerEl.classList.toggle('tag-mode', inTagMode);
  headerEl.classList.toggle('paragraph-mode', !inTagMode);
}

function applyEditableBodyPresentation(wrap, block) {
  if (!wrap || !block) return;

  const view = wrap.querySelector('.block-view');
  const source = wrap.querySelector('.block-src');
  if (!view || !source) return;

  const rendered = view.firstElementChild;
  if (rendered && rendered.id) {
    rendered.dataset.originalEditingId = rendered.id;
    rendered.removeAttribute('id');
  }

  const previousClasses = String(source.dataset.editPresentationClasses || '').split(' ').filter(Boolean);
  if (previousClasses.length > 0) {
    source.classList.remove(...previousClasses);
  }

  const nextClasses = [`block-src-type-${String(block.type || 'paragraph').toLowerCase()}`];
  if (block.type === 'heading') {
    nextClasses.push(`block-src-heading-${Math.min(6, Math.max(1, Number(block.level || 1)))}`);
  }

  const customClasses = splitClassNames(block.className);
  nextClasses.push(...customClasses);
  source.classList.add(...nextClasses);
  source.dataset.editPresentationClasses = nextClasses.join(' ');

  if (block.id) {
    source.id = block.id;
  } else {
    source.removeAttribute('id');
  }

  renderEditableHeader(source);
}

function clearEditableBodyPresentation(wrap) {
  if (!wrap) return;

  const view = wrap.querySelector('.block-view');
  const source = wrap.querySelector('.block-src');
  const header = wrap.querySelector('.block-edit-header');
  if (!source) return;

  const previousClasses = String(source.dataset.editPresentationClasses || '').split(' ').filter(Boolean);
  if (previousClasses.length > 0) {
    source.classList.remove(...previousClasses);
  }

  delete source.dataset.editPresentationClasses;
  source.removeAttribute('id');

  if (header) {
    header.textContent = '';
    header.style.display = 'none';
  }

  const rendered = view ? view.firstElementChild : null;
  if (rendered && rendered.dataset.originalEditingId) {
    rendered.id = rendered.dataset.originalEditingId;
    delete rendered.dataset.originalEditingId;
  }
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
  return findCssBlockIndexInModel(docModel);
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

function ensureControllers() {
  if (!inlineCssController) {
    inlineCssController = new InlineCssSurfaceController({
      applyCustomCss,
      extractCssFromDocumentModel,
      findAttributeTargetAtCursor,
      getScopedCssDeclarations,
      getScopedCssForSelector,
      markDocumentDirty,
      mergeScopedCssForSelector,
      publishViewState,
      recordDocumentUndo,
      renderEditableHeader,
      upsertCssBlock,
    });
  }

  if (!blockSourceController) {
    blockSourceController = new BlockSourceController({
      applyEditableBodyPresentation,
      autosizeBlockSrc,
      buildRawSourceFromEditorParts,
      buildRenderedContent,
      clearDocumentUndoSeed,
      clearEditableBodyPresentation,
      closeInlineCssSurface,
      debouncedAutosave,
      getDocModel: () => docModel,
      getHeaderSourceFromEditor,
      getRawSourceForEditor,
      getRawSourceFromEditor,
      knownBlockTypes: KNOWN_BLOCK_TYPES,
      parseBlock,
      recordDocumentUndo,
      refreshDocumentCss,
      renderDocument,
      setStatus,
      splitBlockSourceForEditor,
      stringifyBlock,
      tryApplyPendingExternalSource,
      updateInlineCssAffordance: (textarea) => inlineCssController.updateInlineCssAffordance(textarea),
    });
  }
}

function closeInlineCssSurface(restoreFocus) {
  ensureControllers();
  inlineCssController.closeInlineCssSurface(Boolean(restoreFocus));
}

function openInlineCssSurface(source, selector) {
  ensureControllers();
  inlineCssController.openInlineCssSurface(source, selector);
}

function closeBlockSrc(index, commitChanges) {
  ensureControllers();
  blockSourceController.closeBlockSrc(index, commitChanges);
}

function hasPendingBlockSourceChanges(source) {
  ensureControllers();
  return blockSourceController.hasPendingBlockSourceChanges(source);
}

function commitBlockSourceForHistory(index) {
  ensureControllers();
  blockSourceController.commitBlockSourceForHistory(index);
}

function commitOpenSources(exceptIndex) {
  ensureControllers();
  blockSourceController.commitOpenSources(exceptIndex);
}

function commitOpenSourcesForHistory(exceptIndex) {
  ensureControllers();
  blockSourceController.commitOpenSourcesForHistory(exceptIndex);
}

function hasOpenBlockSources() {
  ensureControllers();
  return blockSourceController.hasOpenBlockSources();
}

function isBlankPageClickTarget(target, pageEl, blocksContainer) {
  if (!target || !pageEl || !blocksContainer) return false;
  if (target.closest('.ui-chrome')) return false;
  if (target.closest('.meta-rendered') || target.closest('.meta-input')) return false;
  if (target.closest('.block-wrap')) return false;  // Don't close on any click within blocks
  if (target.closest('.block-view') || target.closest('.block-src-wrapper')) return false;
  if (target.closest('.autocomplete-menu')) return false;  // Don't close on autocomplete interactions
  if (target.closest('button, select, input, textarea, a, label')) return false;
  // Extra safety: never close editors if focus is in a textarea
  if (document.activeElement && document.activeElement.classList.contains('block-src')) return false;
  return target === pageEl || target === blocksContainer;
}

function updateInlineCssAffordance(textarea) {
  ensureControllers();
  inlineCssController.updateInlineCssAffordance(textarea);
}

function openBlockSrc(index) {
  ensureControllers();
  blockSourceController.openBlockSrc(index);
}

function commitBlockSrc(index) {
  ensureControllers();
  blockSourceController.commitBlockSrc(index);
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

  const currentDoc = currentDocSourceText();
  if (currentDoc === lastSavedDoc) {
    transitionDocSaveState('SYNC_CLEAN');
    clearStatusPersistent();
    return;
  }

  startSaveRequest(currentDoc);
}

function saveDocAuto() {
  // Autosave intentionally disabled. Persistence happens only on explicit Cmd/Ctrl+S.
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

  recordDocumentUndo('insert-paragraph');

  const next = {
    type: 'paragraph',
    text: '',
    rawSource: '',
  };

  const safeIndex = Math.max(0, Math.min(index, docModel.blocks.length));
  docModel.blocks.splice(safeIndex, 0, next);
  renderDocument();
  markDocumentDirty();
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

function syncUiStateMirrorFromController() {
  if (!uiStateController) {
    return;
  }

  currentTheme = uiStateController.getCurrentTheme();
  currentAppearance = uiStateController.getCurrentAppearance();
}

function ensureUiStateController() {
  if (uiStateController) {
    return uiStateController;
  }

  uiStateController = createUiStateController({
    appearanceStorageKey,
    editModeStorageKey,
    DOC_VIEW_STATES,
    transitionDocViewState,
    getDocViewState: () => docViewState,
    isEditModeState,
    syncEditModeIndicators,
    postMessage: (message) => vscode.postMessage(message),
    publishViewState,
    getActiveScopedCss,
    closeInlineCssSurface,
    commitOpenSources,
    closeAutocomplete,
    setStatus,
    tryApplyPendingExternalSource,
  });

  uiStateController.setAppearance(currentAppearance);
  uiStateController.applyTheme(currentTheme, false);
  syncUiStateMirrorFromController();
  return uiStateController;
}

function loadAppearance() {
  ensureUiStateController().loadAppearance();
  syncUiStateMirrorFromController();
}

function persistAppearance() {
  const controller = ensureUiStateController();
  controller.setAppearance(currentAppearance);
  controller.applyAppearance(true);
  syncUiStateMirrorFromController();
}

function applyAppearance(persist = false) {
  const controller = ensureUiStateController();
  controller.setAppearance(currentAppearance);
  controller.applyAppearance(Boolean(persist));
  syncUiStateMirrorFromController();
}

function applyTheme(theme, persist = false) {
  ensureUiStateController().applyTheme(theme, Boolean(persist));
  syncUiStateMirrorFromController();
}

function setControlsOpen(isOpen) {
  ensureUiStateController().setControlsOpen(Boolean(isOpen));
}

function setHelpOpen(isOpen) {
  ensureUiStateController().setHelpOpen(Boolean(isOpen));
}

function revealChromeBriefly(durationMs = 1400) {
  ensureUiStateController().revealChromeBriefly(durationMs);
}

function toggleControls(forceOpen) {
  ensureUiStateController().toggleControls(forceOpen);
}

function toggleHelp() {
  ensureUiStateController().toggleHelp();
}

function isEditModeEnabled() {
  return ensureUiStateController().isEditModeEnabled();
}

function loadEditModePreference() {
  return ensureUiStateController().loadEditModePreference();
}

function persistEditModePreference(enabled) {
  try {
    window.localStorage.setItem(editModeStorageKey, enabled ? 'true' : 'false');
  } catch {
  }
}

function setEditMode(enabled) {
  ensureUiStateController().setEditMode(Boolean(enabled));
  syncUiStateMirrorFromController();
}

function getFocusedBlockIndex() {
  const active = document.activeElement;
  if (!active || !(active instanceof Element)) {
    return null;
  }

  const wrap = active.closest('.block-wrap');
  if (!wrap) {
    return null;
  }

  const index = Number.parseInt(wrap.dataset.blockIndex, 10);
  return Number.isFinite(index) ? index : null;
}

function summarizeBlockTypeCounts() {
  const counts = {
    total: 0,
    headings: 0,
    paragraphs: 0,
    lists: 0,
    codeBlocks: 0,
    images: 0,
    quotes: 0,
    rules: 0,
  };

  if (!docModel || !Array.isArray(docModel.blocks)) {
    return counts;
  }

  for (const block of docModel.blocks) {
    const type = String(block?.type || 'paragraph');
    counts.total += 1;

    if (type === 'heading') counts.headings += 1;
    else if (type === 'paragraph') counts.paragraphs += 1;
    else if (type === 'bulleted-list' || type === 'numbered-list' || type === 'checklist') counts.lists += 1;
    else if (type === 'code') counts.codeBlocks += 1;
    else if (type === 'image') counts.images += 1;
    else if (type === 'quote') counts.quotes += 1;
    else if (type === 'rule') counts.rules += 1;
  }

  return counts;
}

function captureSurfaceSnapshot(options = {}) {
  const includeText = options.includeText !== false;
  const includeStyles = Boolean(options.includeStyles);
  const page = document.querySelector('.page');
  const pageRect = page ? page.getBoundingClientRect() : null;
  const wraps = Array.from(document.querySelectorAll('.block-wrap'));

  const blocks = wraps.map((wrap) => {
    const index = Number.parseInt(wrap.dataset.blockIndex, 10);
    const view = wrap.querySelector('.block-view');
    const srcWrap = wrap.querySelector('.block-src-wrapper');
    const rect = wrap.getBoundingClientRect();
    const block = Number.isFinite(index) && docModel && Array.isArray(docModel.blocks)
      ? docModel.blocks[index]
      : null;
    const computed = view && includeStyles ? window.getComputedStyle(view) : null;

    return {
      index: Number.isFinite(index) ? index : -1,
      id: block && block.id ? block.id : '',
      type: block && block.type ? block.type : 'unknown',
      rect: {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      },
      inViewport: rect.bottom > 0 && rect.top < window.innerHeight,
      sourceOpen: Boolean(srcWrap && srcWrap.style.display === 'block'),
      text: includeText
        ? (block
            ? (Array.isArray(block.items)
                ? block.items.map((item) => (typeof item === 'object' && item ? item.text : item)).join(' | ')
                : String(block.text || block.alt || block.src || ''))
            : String(view ? view.textContent || '' : ''))
        : undefined,
      style: computed
        ? {
            fontSize: computed.fontSize,
            lineHeight: computed.lineHeight,
            color: computed.color,
            backgroundColor: computed.backgroundColor,
          }
        : undefined,
    };
  });

  return {
    documentPath: currentDocPath,
    capturedAt: new Date().toISOString(),
    theme: currentTheme,
    resolvedTheme: document.body.dataset.resolvedTheme || 'light',
    editMode: isEditModeEnabled(),
    fsm: {
      documentState: docViewState,
      saveState: docSaveState,
      historyLength: fsmTransitionHistory.length,
      lastTransition: fsmTransitionHistory.lastTransition,
    },
    documentHistory: {
      undoDepth: documentHistory.undoDepth,
      redoDepth: documentHistory.redoDepth,
    },
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      scrollX: window.scrollX,
      scrollY: window.scrollY,
    },
    page: pageRect
      ? {
          x: Math.round(pageRect.x),
          y: Math.round(pageRect.y),
          width: Math.round(pageRect.width),
          height: Math.round(pageRect.height),
        }
      : null,
    focusedBlockIndex: getFocusedBlockIndex(),
    blockCounts: summarizeBlockTypeCounts(),
    blocks,
  };
}

function findBlockWrap(index) {
  return document.querySelector(`.block-wrap[data-block-index="${index}"]`);
}

function focusBlock(index) {
  const wrap = findBlockWrap(index);
  if (!wrap) return false;
  const view = wrap.querySelector('.block-view');
  if (!view) return false;
  view.focus();
  return true;
}

function scrollToBlock(index, behavior = 'instant') {
  const wrap = findBlockWrap(index);
  if (!wrap) return false;
  wrap.scrollIntoView({ block: 'center', behavior: behavior === 'smooth' ? 'smooth' : 'auto' });
  return true;
}

function withAnimationFrame() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

async function runSurfaceAction(payload = {}) {
  const action = String(payload.action || '').trim();

  if (!action) {
    throw new Error('Surface action is required.');
  }

  if (action === 'setEditMode') {
    setEditMode(Boolean(payload.enabled));
  } else if (action === 'scrollBy') {
    const deltaY = Number(payload.deltaY || 0);
    window.scrollBy({ top: Number.isFinite(deltaY) ? deltaY : 0, behavior: 'auto' });
  } else if (action === 'scrollTo') {
    const top = Number(payload.top || 0);
    window.scrollTo({ top: Number.isFinite(top) ? top : 0, behavior: 'auto' });
  } else if (action === 'scrollToBlock') {
    const index = Number.parseInt(String(payload.blockIndex || '-1'), 10);
    if (!Number.isFinite(index) || !scrollToBlock(index, String(payload.behavior || 'instant'))) {
      throw new Error('Unable to scroll to target block.');
    }
  } else if (action === 'focusBlock') {
    const index = Number.parseInt(String(payload.blockIndex || '-1'), 10);
    if (!Number.isFinite(index) || !focusBlock(index)) {
      throw new Error('Unable to focus target block.');
    }
  } else if (action === 'openBlockSource') {
    const index = Number.parseInt(String(payload.blockIndex || '-1'), 10);
    if (!Number.isFinite(index)) {
      throw new Error('A valid block index is required.');
    }
    openBlockSrc(index);
  } else if (action === 'closeBlockSource') {
    const index = Number.parseInt(String(payload.blockIndex || '-1'), 10);
    if (!Number.isFinite(index)) {
      throw new Error('A valid block index is required.');
    }
    closeBlockSrc(index, payload.commit !== false);
  } else if (action === 'undoState') {
    if (!undoLastFsmTransition()) {
      throw new Error('No FSM transition available to undo.');
    }
  } else if (action === 'undoDocument') {
    commitOpenSourcesForHistory();
    if (!performGlobalUndo()) {
      throw new Error('No document edit available to undo.');
    }
  } else if (action === 'redoDocument') {
    commitOpenSourcesForHistory();
    if (!performGlobalRedo()) {
      throw new Error('No document edit available to redo.');
    }
  } else {
    throw new Error(`Unknown surface action: ${action}`);
  }

  await withAnimationFrame();
  return captureSurfaceSnapshot(payload);
}

function toggleEditMode() {
  const isEnabled = isEditModeEnabled();
  const nextEnabled = !isEnabled;

  if (!nextEnabled) {
    closeInlineCssSurface();
    commitOpenSources();
    closeAutocomplete();
  }

  setEditMode(nextEnabled);
  setStatus(nextEnabled ? 'Edit mode on' : 'Edit mode off');
  tryApplyPendingExternalSource();
}

function hideLoadingScreen() {
  const loadingScreen = document.getElementById('loading-screen');
  const page = document.querySelector('.page');

  if (loadingGuardTimer) {
    clearTimeout(loadingGuardTimer);
    loadingGuardTimer = null;
  }

  // If setEditMode never ran (exception thrown before it, leaving FSM in BOOTSTRAPPING),
  // force through to LOADING_EDIT so MARK_READY can succeed.
  if (docViewState === DOC_VIEW_STATES.BOOTSTRAPPING) {
    transitionDocViewState('START_EDIT');
  }
  transitionDocViewState('MARK_READY');

  // Belt-and-suspenders: directly mark the page ready in case the FSM transition
  // still failed (e.g. an unexpected state). Pointer-events would otherwise stay
  // disabled indefinitely.
  if (page && !isReadyViewState(docViewState)) {
    page.dataset.ready = 'true';
    page.dataset.editMode = 'true';
    page.setAttribute('aria-busy', 'false');
  }

  if (loadingScreen) {
    loadingScreen.dataset.hidden = 'true';
    const removeLoading = () => {
      loadingScreen.style.display = 'none';
    };

    loadingScreen.addEventListener('transitionend', removeLoading, { once: true });
    setTimeout(removeLoading, 220);
  }
}

function armLoadingGuard(preferredEditMode = true) {
  if (loadingGuardTimer) {
    clearTimeout(loadingGuardTimer);
    loadingGuardTimer = null;
  }

  loadingGuardTimer = setTimeout(() => {
    if (isReadyViewState(docViewState)) {
      loadingGuardTimer = null;
      return;
    }

    console.warn('[doc-webview] Loading guard forcing ready state', { state: docViewState });

    if (docViewState === DOC_VIEW_STATES.BOOTSTRAPPING || docViewState === DOC_VIEW_STATES.LOAD_ERROR) {
      transitionDocViewState(preferredEditMode ? 'START_EDIT' : 'START_READ');
    }

    transitionDocViewState('MARK_READY');
    hideLoadingScreen();
  }, 2500);
}

function insertImageBlock(imagePath, altText, atIndex) {
  if (!docModel) return;

  recordDocumentUndo('insert-image');

  const block = {
    type: 'image',
    src: String(imagePath || '').trim(),
    alt: String(altText || '').trim(),
  };

  block.rawSource = stringifyBlock(block);

  const insertAt =
    typeof atIndex === 'number' && Number.isFinite(atIndex) && atIndex >= 0
      ? Math.min(atIndex, docModel.blocks.length)
      : docModel.blocks.length;

  docModel.blocks.splice(insertAt, 0, block);
  renderDocument();
  markDocumentDirty();
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
  let pendingDropIndex = -1;
  let ghostEl = null;

  function ensureGhost() {
    if (!ghostEl) {
      ghostEl = document.createElement('div');
      ghostEl.className = 'drop-ghost';
      ghostEl.setAttribute('aria-hidden', 'true');
      const label = document.createElement('span');
      label.textContent = 'Drop image here';
      ghostEl.appendChild(label);
    }
    return ghostEl;
  }

  function removeGhost() {
    if (ghostEl && ghostEl.parentNode) {
      ghostEl.parentNode.removeChild(ghostEl);
    }
    pendingDropIndex = -1;
  }

  function isDraggedImages(event) {
    const dt = event.dataTransfer;
    if (!dt) return false;
    if (dt.items && dt.items.length > 0) {
      for (let i = 0; i < dt.items.length; i += 1) {
        if (String(dt.items[i].type || '').startsWith('image/')) return true;
      }
    }
    return Array.from(dt.types || []).includes('Files');
  }

  function positionGhost(event) {
    const container = document.getElementById('blocks');
    if (!container) return;
    const insertIdx = getInsertIndexForY(container, event.clientY);
    if (insertIdx !== pendingDropIndex) {
      pendingDropIndex = insertIdx;
      const ghost = ensureGhost();
      const wraps = container.querySelectorAll(':scope > .block-wrap');
      if (insertIdx >= wraps.length) {
        container.appendChild(ghost);
      } else {
        container.insertBefore(ghost, wraps[insertIdx]);
      }
    }
  }

  function clearDropState() {
    document.body.classList.remove('drag-active');
    removeGhost();
  }

  window.addEventListener('dragenter', (event) => {
    event.preventDefault();
    document.body.classList.add('drag-active');
  });

  window.addEventListener('dragover', (event) => {
    event.preventDefault();
    document.body.classList.add('drag-active');
    if (isDraggedImages(event)) {
      positionGhost(event);
    } else {
      removeGhost();
    }
  });

  window.addEventListener('dragleave', (event) => {
    if (event.relatedTarget) return;
    clearDropState();
  });

  window.addEventListener('drop', async (event) => {
    event.preventDefault();
    const capturedInsertAt = pendingDropIndex >= 0 ? pendingDropIndex : (docModel ? docModel.blocks.length : 0);
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
          insertAt: capturedInsertAt,
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
  const initialPaper = initEl && initEl.dataset && initEl.dataset.initialPaper ? initEl.dataset.initialPaper : 'white';
  const initialDensity = initEl && initEl.dataset && initEl.dataset.initialDensity ? initEl.dataset.initialDensity : 'comfortable';
  const initialScale = initEl && initEl.dataset && initEl.dataset.initialScale ? clampScale(initEl.dataset.initialScale) : 100;
  const preferredEditMode = loadEditModePreference();
  currentDocPath = docPath;
  workspaceBaseUri = (initEl && initEl.dataset && initEl.dataset.workspaceUri) ? String(initEl.dataset.workspaceUri).replace(/\/$/, '') : '';

  armLoadingGuard(preferredEditMode);

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

      // Never handle clicks if an editor textarea is currently focused
      if (document.activeElement && document.activeElement.classList.contains('block-src')) {
        return;
      }

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
      if (target.closest('a, button, input, select, textarea, label, .autocomplete-menu')) {
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
      if (!event.target.classList.contains('block-src') && !event.target.classList.contains('block-edit-header')) return;
      const textarea = event.target;

      if ((event.metaKey || event.ctrlKey) && (event.key.toLowerCase() === 'z' || event.key.toLowerCase() === 'y')) {
        event.preventDefault();
        event.stopPropagation();
        closeInlineCssSurface();

        const wrap = textarea.closest('.block-wrap');
        if (wrap) {
          const index = Number.parseInt(wrap.dataset.blockIndex, 10);
          if (!Number.isNaN(index)) {
            commitBlockSourceForHistory(index);
          }
        }

        if (isRedoShortcut(event)) {
          if (!performGlobalRedo()) {
            setStatus('Nothing to redo');
          }
        } else if (!performGlobalUndo()) {
          setStatus('Nothing to undo');
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
              commitBlockSrc(index);
            }
          }
          saveDoc();
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
              ensureDocumentUndoSeed(bodyEditor, 'header-delete-join');
              bodyEditor.value = bodyValue.slice(1);
              autosizeBlockSrc(bodyEditor);
              updateInlineCssAffordance(textarea);
              debouncedAutosave();
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

          const headerText = getRawSourceFromEditor(textarea.value || '');
          const trimmedHeaderText = headerText.trim();

          if (trimmedHeaderText.startsWith('::')) {
            bodyEditor.dataset.headerSource = headerText;
          } else if (trimmedHeaderText.length > 0) {
            // Not a block tag: treat typed text as body content.
            bodyEditor.value = bodyEditor.value.length > 0
              ? `${headerText}\n${bodyEditor.value}`
              : headerText;
            bodyEditor.dataset.headerSource = '';
            textarea.value = '';
            autosizeBlockSrc(bodyEditor);
          } else {
            bodyEditor.dataset.headerSource = '';
          }

          renderEditableHeader(bodyEditor);
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

        return;
      }

      if ((event.ctrlKey || event.metaKey) && event.code === 'Space') {
        event.preventDefault();
        renderAutocomplete(textarea, true);
        return;
      }

      if (event.key === 'ArrowUp' && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        const start = typeof textarea.selectionStart === 'number' ? textarea.selectionStart : 0;
        const end = typeof textarea.selectionEnd === 'number' ? textarea.selectionEnd : 0;
        if (start === 0 && end === 0) {
          const headerEditor = getBlockHeaderEditor(textarea);
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
          const headerEditor = getBlockHeaderEditor(textarea);
          if (headerEditor && headerEditor.style.display !== 'none') {
            event.preventDefault();
            const caret = String(headerEditor.value || '').length;
            headerEditor.focus();
            headerEditor.setSelectionRange(caret, caret);
            return;
          }
        }
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
        const wrap = textarea.closest('.block-wrap');
        if (wrap) {
          const index = Number.parseInt(wrap.dataset.blockIndex, 10);
          if (!Number.isNaN(index)) {
            commitBlockSrc(index);
          }
        }
        saveDoc();
      }
    });

    blocksContainer.addEventListener('input', (event) => {
      if (event.target.classList.contains('block-edit-header')) {
        const header = event.target;
        const srcWrap = header.closest('.block-src-wrapper');
        const source = srcWrap ? srcWrap.querySelector('.block-src') : null;
        ensureDocumentUndoSeed(source, 'header-live-edit');
        if (source) {
          source.dataset.headerSource = getRawSourceFromEditor(header.value);
        }
        header.style.height = '0px';
        header.style.height = header.scrollHeight + 'px';
        updateInlineCssAffordance(header);
        debouncedAutosave();
        return;
      }

      if (!event.target.classList.contains('block-src')) return;

      // Seamless "one-part" editing: if a blank block starts with ':', transition that line into header mode.
      const source = event.target;
      ensureDocumentUndoSeed(source, 'block-live-edit');
      const headerEditor = getBlockHeaderEditor(source);
      const hasHeader = Boolean(String(source.dataset.headerSource || '').trim());
      const bodyText = String(source.value || '');
      if (!hasHeader && headerEditor && bodyText.startsWith(':') && !bodyText.includes('\n')) {
        source.dataset.headerSource = bodyText;
        source.value = '';
        renderEditableHeader(source);
        autosizeBlockSrc(source);
        headerEditor.focus();
        const caret = headerEditor.value.length;
        headerEditor.setSelectionRange(caret, caret);
        updateInlineCssAffordance(headerEditor);
        debouncedAutosave();
        return;
      }

      autosizeBlockSrc(event.target);
      renderAutocomplete(event.target, false);
      updateInlineCssAffordance(event.target);
      debouncedAutosave();
    });

    blocksContainer.addEventListener('keyup', (event) => {
      if (event.target.classList.contains('block-edit-header')) {
        updateInlineCssAffordance(event.target);
        return;
      }

      if (!event.target.classList.contains('block-src')) return;
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') {
        renderAutocomplete(event.target, false);
      }
      updateInlineCssAffordance(event.target);
    });

    blocksContainer.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-edit-header')) return;
      updateInlineCssAffordance(target);
    });

    blocksContainer.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-src')) return;
      renderAutocomplete(target, false);
      updateInlineCssAffordance(target);
    });

    blocksContainer.addEventListener('mouseup', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-edit-header')) return;
      const textarea = target;

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
        const styleTarget = findAttributeTargetAtCursor(textarea);
        const srcWrap = textarea.closest('.block-src-wrapper');
        const source = srcWrap ? srcWrap.querySelector('.block-src') : null;

        if (styleTarget && styleTarget.selector && source) {
          openInlineCssSurface(source, styleTarget.selector);
          return;
        }
        updateInlineCssAffordance(textarea);
      });
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

    blocksContainer.addEventListener('wheel', (event) => {
      const target = getEventElementTarget(event);
      if (!target || !target.classList.contains('block-src')) return;
      const textarea = target;

      if (event.deltaY >= 0 || textarea.scrollTop > 0) {
        return;
      }

      const headerEditor = getBlockHeaderEditor(textarea);
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

      ensureControllers();
      blankPagePointerDownHadOpenEditors = hasOpenBlockSources() || inlineCssController.hasOpenSurface();
    });

    pageEl.addEventListener('click', (event) => {
      const target = getEventElementTarget(event);
      if (!target) return;

      // Never handle clicks if an editor textarea is currently focused
      if (document.activeElement && document.activeElement.classList.contains('block-src')) {
        return;
      }

      const isBlankArea = isBlankPageClickTarget(target, pageEl, blocksContainer);
      if (!isBlankArea) return;

      const hadOpenSources = hasOpenBlockSources();
      ensureControllers();
      const hadOpenInlineCss = inlineCssController.hasOpenSurface();
      const shouldOnlyCloseEditors = blankPagePointerDownHadOpenEditors || hadOpenSources || hadOpenInlineCss;

      closeInlineCssSurface();
      commitOpenSources();
      blankPagePointerDownHadOpenEditors = false;

      // Blank-space clicks: first close editors, second click inserts block
      if (shouldOnlyCloseEditors) {
        return;
      }

      if (!isEditModeEnabled()) {
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

    const docLink = target.closest('.doc-link');
    if (docLink) {
      event.preventDefault();
      const href = String(docLink.dataset.docHref || '').trim();
      if (href && !href.includes('..') && /\.dx$/.test(href)) {
        vscode.postMessage({ type: 'open-doc', path: href });
      }
      return;
    }

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

  try {
    applyTheme(initialTheme, false);
    currentAppearance = {
      paper: ['white', 'cream', 'slate'].includes(String(initialPaper || 'white')) ? String(initialPaper || 'white') : 'white',
      density: ['comfortable', 'compact'].includes(String(initialDensity || 'comfortable')) ? String(initialDensity || 'comfortable') : 'comfortable',
      scale: initialScale,
    };
    applyAppearance(false);
    refreshDocumentCss();
    setControlsOpen(false);
    setHelpOpen(false);
    setEditMode(preferredEditMode);
    wireDragAndDrop();
    vscode.postMessage({ type: 'get-config' });
  } finally {
    hideLoadingScreen();
  }

  document.addEventListener('change', (event) => {
    const checkbox = event.target;
    if (!checkbox || checkbox.type !== 'checkbox') return;

    const li = checkbox.parentElement;
    const ul = li && li.parentElement;
    if (!ul || !ul.classList.contains('checklist-wrap')) return;

    const blockView = ul.closest('.block-view');
    const blockWrap = blockView && blockView.closest('.block-wrap');
    if (!blockWrap) return;

    const blockIndex = Number.parseInt(blockWrap.dataset.blockIndex, 10);
    const itemIndex = Number.parseInt(checkbox.dataset.itemIndex, 10);
    if (!Number.isFinite(blockIndex) || !Number.isFinite(itemIndex)) return;

    const block = docModel && docModel.blocks && docModel.blocks[blockIndex];
    if (!block || block.type !== 'checklist' || !Array.isArray(block.items)) return;

    const item = block.items[itemIndex];
    if (!item) return;

    item.checked = checkbox.checked;

    const span = li.querySelector('span');
    if (span) {
      span.classList.toggle('check-done', checkbox.checked);
    }

    block.rawSource = null;
    block.rawSource = stringifyBlock(block);
    recordDocumentUndo('toggle-checklist');
    markDocumentDirty();
    debouncedAutosave();
  });

  document.addEventListener('keydown', (event) => {
    if (event.defaultPrevented) {
      return;
    }

    const key = String(event.key || '').toLowerCase();
    const isPrimaryModifier = event.metaKey || event.ctrlKey;

    if (isPrimaryModifier && key === 'z' && !isTypingTarget(document.activeElement)) {
      event.preventDefault();
      event.stopPropagation();
      if (typeof event.stopImmediatePropagation === 'function') {
        event.stopImmediatePropagation();
      }

      commitOpenSourcesForHistory();

      if (event.shiftKey) {
        if (!performGlobalRedo()) {
          setStatus('Nothing to redo');
        }
        return;
      }

      if (!performGlobalUndo()) {
        setStatus('Nothing to undo');
      }
      return;
    }

    if (isPrimaryModifier && key === 's') {
      event.preventDefault();
      event.stopPropagation();
      if (typeof event.stopImmediatePropagation === 'function') {
        event.stopImmediatePropagation();
      }

      commitOpenSources();
      saveDoc();
    }
  }, true);

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
      commitOpenSources();
      saveDoc();
    }

    if ((event.metaKey || event.ctrlKey)
      && (event.key.toLowerCase() === 'z' || event.key.toLowerCase() === 'y')
      && !isTypingTarget(document.activeElement)) {
      event.preventDefault();
      commitOpenSourcesForHistory();

      if (isRedoShortcut(event)) {
        if (!performGlobalRedo()) {
          setStatus('Nothing to redo');
        }
        return;
      }

      if (!performGlobalUndo()) {
        setStatus('Nothing to undo');
      }
      return;
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

    if (msg.type === 'surface-capture-request') {
      try {
        const snapshot = captureSurfaceSnapshot(msg.payload || {});
        vscode.postMessage({
          type: 'surface-capture-response',
          requestId: msg.requestId,
          payload: snapshot,
        });
      } catch (error) {
        vscode.postMessage({
          type: 'surface-capture-response',
          requestId: msg.requestId,
          error: error instanceof Error ? error.message : 'Surface capture failed.',
        });
      }
      return;
    }

    if (msg.type === 'surface-action-request') {
      runSurfaceAction(msg.payload || {})
        .then((snapshot) => {
          vscode.postMessage({
            type: 'surface-action-response',
            requestId: msg.requestId,
            payload: {
              action: msg?.payload?.action || '',
              snapshot,
            },
          });
        })
        .catch((error) => {
          vscode.postMessage({
            type: 'surface-action-response',
            requestId: msg.requestId,
            error: error instanceof Error ? error.message : 'Surface action failed.',
          });
        });
      return;
    }

    if (msg.type === 'status') {
      setStatus(msg.text);
      return;
    }

    if (msg.type === 'set-source') {
      const incomingSource = String(msg.text || '');
      const activeSource = currentDocSourceText();

      if (incomingSource === activeSource) {
        lastSavedDoc = activeSource;
        transitionDocSaveState('SYNC_CLEAN');
        clearStatusPersistent();
        hasDirtyWorkingCopySignal = false;
        tryApplyPendingExternalSource();
        return;
      }

      if (!canApplyExternalSourceNow()) {
        pendingExternalSource = incomingSource;
        setStatusPersistent('External update pending… finish edits to sync', 'dirty');
        return;
      }

      applyIncomingSourceText(incomingSource);
      return;
    }

    if (msg.type === 'config') {
      applyTheme(String(msg.theme || 'auto'), false);
      return;
    }

    if (msg.type === 'image-uploaded') {
      const insertAt = typeof msg.insertAt === 'number' && msg.insertAt >= 0 ? msg.insertAt : undefined;
      insertImageBlock(String(msg.path || ''), String(msg.alt || ''), insertAt);
    }

    if (msg.type === 'save-complete') {
      const requestId = Number(msg.requestId || 0);
      if (requestId > 0 && requestId !== inFlightSaveRequestId) {
        return;
      }

      inFlightSaveRequestId = 0;

      if (docSaveState === DOC_SAVE_STATES.DIRTY) {
        setStatusPersistent('Unsaved changes', 'dirty');
        return;
      }

      transitionDocSaveState('SAVE_COMPLETE');
      lastSaveErrorMessage = '';
      lastSavedDoc = stringifyDoc(docModel || { metadata: {}, blocks: [] });
      hasDirtyWorkingCopySignal = false;
      setStatusPersistent('Saved', 'saved');
      setTimeout(() => {
        if (docSaveState === DOC_SAVE_STATES.SAVED) {
          transitionDocSaveState('CLEAR_SAVED');
          clearStatusPersistent();
          tryApplyPendingExternalSource();
        }
      }, 1600);
      return;
    }

    if (msg.type === 'save-error') {
      const requestId = Number(msg.requestId || 0);
      if (requestId > 0 && requestId !== inFlightSaveRequestId) {
        return;
      }

      inFlightSaveRequestId = 0;
      lastSaveErrorMessage = String(msg.error || 'unknown save error');
      transitionDocSaveState('SAVE_FAILED');
      setStatusPersistent('Save failed: ' + lastSaveErrorMessage, 'error');
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
