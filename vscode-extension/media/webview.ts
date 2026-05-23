import { DOC_SAVE_STATES, DOC_SAVE_TRANSITIONS, DOC_VIEW_STATES, DOC_VIEW_TRANSITIONS } from './webview-fsm.js';
import { createAutocompleteController } from './webview-autocomplete-controller.js';
import { createBlockRenderer } from './webview-block-renderer.js';
import { applyBlockViewPresentation } from './webview-block-presentation.js';
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
import { registerBlockInteractionEvents } from './webview-events.js';
import {
  CallbackUndoRedoController,
  TableDrivenStateMachine,
  TransitionHistory,
} from './webview-state-core.js';
import { createDocumentLifecycle } from './webview-document-lifecycle.js';
import { createUiStateController } from './webview-ui-state.js';
import { createSurfaceController } from './webview-surface-controller.js';
import type { PipelineBlock } from './doc-pipeline.js';

const vscode = typeof acquireVsCodeApi === 'function' ? acquireVsCodeApi() : { postMessage: () => {} };

type JsonLike = string | number | boolean | null | undefined | object;
type DocModel = ReturnType<typeof parseDoc>;
type FsmSnapshot = {
  docViewState?: string;
  docSaveState?: string;
  pendingExternalSource?: string | null;
  inFlightSaveRequestId?: number;
  lastSavedDoc?: string;
  lastSaveErrorMessage?: string;
};

type BlockRendererController = ReturnType<typeof createBlockRenderer>;
type AutocompleteController = ReturnType<typeof createAutocompleteController>;
type DocumentLifecycleController = ReturnType<typeof createDocumentLifecycle>;
type UiStateController = ReturnType<typeof createUiStateController>;
type SurfaceController = ReturnType<typeof createSurfaceController>;

interface VsCodeMessagePayload {
  type?: string;
  text?: string;
  requestId?: number;
  payload?: Record<string, JsonLike>;
  theme?: string;
  path?: string;
  alt?: string;
  insertAt?: number;
  error?: string;
}

let docModel: DocModel | null = null;
let loadNoteEl: HTMLElement | null = null;
let currentDocPath = 'unknown.dx';
let workspaceBaseUri = '';
const appearanceStorageKey = 'docdb.appearance.v1';
const editModeStorageKey = 'docdb.edit-mode.v1';
let loadingGuardTimer: ReturnType<typeof setTimeout> | null = null;
let customCssSheet: CSSStyleSheet | null = null;

let blankPagePointerDownHadOpenEditors = false;

let docViewState: string = DOC_VIEW_STATES.BOOTSTRAPPING;
let docSaveState: string = DOC_SAVE_STATES.IDLE;
let lastSavedDoc = '';
let lastSaveErrorMessage = '';
let inFlightSaveRequestId = 0;
let nextSaveRequestId = 1;
let pendingExternalSource: string | null = null;
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
  restoreSnapshot: (snapshot) => applyDocumentSnapshot(asDocModelSnapshot(snapshot), null),
});
let documentLifecycleController: DocumentLifecycleController | null = null;
let inlineCssController: InlineCssSurfaceController | null = null;
let blockSourceController: BlockSourceController | null = null;
let blockRendererController: BlockRendererController | null = null;
let autocompleteController: AutocompleteController | null = null;
let uiStateController: UiStateController | null = null;
let surfaceController: SurfaceController | null = null;

function collectAutocompleteHeaders(): string[] {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const headers: string[] = [];

  for (const block of docModel.blocks) {
    const raw = String(block && block.rawSource ? block.rawSource : '').trim();
    if (!raw) {
      continue;
    }

    const header = raw.split('\n')[0] || '';
    if (header.trim()) {
      headers.push(header);
    }
  }

  return headers;
}

function ensureBlockRendererController(): BlockRendererController {
  if (blockRendererController) {
    return blockRendererController;
  }

  blockRendererController = createBlockRenderer({
    applyBlockViewPresentation,
    listItemText,
    splitClassNames,
    getWorkspaceBaseUri: () => workspaceBaseUri,
  });

  return blockRendererController;
}

function ensureAutocompleteController(): AutocompleteController {
  if (autocompleteController) {
    return autocompleteController;
  }

  autocompleteController = createAutocompleteController({
    blockAutocomplete: BLOCK_AUTOCOMPLETE,
    collectKnownIds,
    collectKnownClasses,
    collectKnownImageSources,
    collectAutocompleteHeaders,
    escapeHtml,
    storage: window.localStorage,
    storageKey: 'docdb.autocomplete-history.v1',
  });
  autocompleteState = autocompleteController.state;

  return autocompleteController;
}

function ensureDocumentLifecycleController(): DocumentLifecycleController {
  if (documentLifecycleController) {
    return documentLifecycleController;
  }

  documentLifecycleController = createDocumentLifecycle({
    getDocModel: () => docModel,
    setDocModel: (nextDocModel: DocModel) => {
      docModel = nextDocModel;
    },
    stringifyDoc,
    parseDoc,
    getLastSavedDoc: () => lastSavedDoc,
    setLastSavedDoc: (nextLastSavedDoc) => {
      lastSavedDoc = String(nextLastSavedDoc || '');
    },
    getPendingExternalSource: () => pendingExternalSource,
    setPendingExternalSource: (nextPendingExternalSource: string | null) => {
      pendingExternalSource = nextPendingExternalSource;
    },
    getIsApplyingDocHistory: () => isApplyingDocHistory,
    clearDocumentRedoHistory: () => documentHistory.clearRedo(),
    recordDocumentUndo,
    transitionDocSaveState,
    getDocSaveState: () => docSaveState,
    DOC_SAVE_STATES,
    hasActiveEditingSurface,
    clearStatusPersistent,
    setStatusPersistent,
    setStatus,
    markDocumentDirty,
    setHasDirtyWorkingCopySignal: (nextHasDirtyWorkingCopySignal: boolean) => {
      hasDirtyWorkingCopySignal = Boolean(nextHasDirtyWorkingCopySignal);
    },
    postMessage: (message) => vscode.postMessage(message),
    getNextSaveRequestId: () => nextSaveRequestId,
    setNextSaveRequestId: (nextRequestId: number) => {
      nextSaveRequestId = Number(nextRequestId || 0);
    },
    setInFlightSaveRequestId: (nextInFlightSaveRequestId: number) => {
      inFlightSaveRequestId = Number(nextInFlightSaveRequestId || 0);
    },
    commitOpenSources,
    getBlocksElement: () => document.getElementById('blocks'),
    buildBlockWrap,
    refreshDocumentCss,
    currentDocSourceText,
  });

  return documentLifecycleController;
}

function captureFsmSnapshot(): FsmSnapshot {
  return {
    docViewState,
    docSaveState,
    pendingExternalSource,
    inFlightSaveRequestId,
    lastSavedDoc,
    lastSaveErrorMessage,
  };
}

function restoreFsmSnapshot(snapshot: JsonLike): boolean {
  if (!snapshot || typeof snapshot !== 'object') {
    return false;
  }

  const state = snapshot as FsmSnapshot;

  docViewState = state.docViewState || DOC_VIEW_STATES.BOOTSTRAPPING;
  docSaveState = state.docSaveState || DOC_SAVE_STATES.IDLE;
  docViewMachine.state = docViewState;
  docSaveMachine.state = docSaveState;
  pendingExternalSource = typeof state.pendingExternalSource === 'string' ? state.pendingExternalSource : null;
  inFlightSaveRequestId = Number(state.inFlightSaveRequestId || 0);
  lastSavedDoc = String(state.lastSavedDoc || '');
  lastSaveErrorMessage = String(state.lastSaveErrorMessage || '');
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

function performGlobalUndo(): 'document' | 'fsm' | null {
  if (undoDocumentChange()) {
    return 'document';
  }

  if (undoLastFsmTransition()) {
    setStatus('Undid state change');
    return 'fsm';
  }

  return null;
}

function performGlobalRedo(): 'document' | 'fsm' | null {
  if (redoDocumentChange()) {
    return 'document';
  }

  if (redoLastFsmTransition()) {
    setStatus('Redid state change');
    return 'fsm';
  }

  return null;
}

function isRedoShortcut(event: KeyboardEvent): boolean {
  if (!event || !(event.metaKey || event.ctrlKey)) {
    return false;
  }

  const key = String(event.key || '').toLowerCase();
  return key === 'y' || (key === 'z' && event.shiftKey);
}

function syncEditModeIndicators(editEnabled: boolean): void {
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

function syncVisibleStateFromMachine(): void {
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

function cloneDocModel(model: DocModel | null): DocModel {
  return JSON.parse(JSON.stringify(model || { blocks: [] })) as DocModel;
}

function asDocModelSnapshot(snapshot: JsonLike): DocModel | null {
  if (!snapshot || typeof snapshot !== 'object') {
    return null;
  }

  const candidate = snapshot as { blocks?: PipelineBlock[] };
  if (!Array.isArray(candidate.blocks)) {
    return null;
  }

  return { blocks: candidate.blocks };
}

function applyDocumentSnapshot(snapshot: DocModel | null, mode: 'undo' | 'redo' | null = null): boolean {
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

function resetDocumentHistory(): void {
  documentHistory.clear();
}

function markDocumentDirty(): void {
  reconcileSaveStateFromModel(true);
}

function syncDocumentSaveStateFromModel(): void {
  reconcileSaveStateFromModel(true);
}

function reconcileSaveStateFromModel(emitDirtySync = true): void {
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

function recordDocumentUndo(action = 'edit'): void {
  if (!docModel || isApplyingDocHistory) {
    return;
  }

  documentHistory.push(String(action || 'edit'));
}

function ensureDocumentUndoSeed(source: HTMLTextAreaElement | null, action = 'live-edit'): void {
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

function clearDocumentUndoSeed(source: HTMLTextAreaElement | null, options: { discardIfNoop?: boolean } = {}): void {
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

function applyDocumentHistorySnapshot(entry: { snapshot?: JsonLike } | null, mode: 'undo' | 'redo'): boolean {
  if (!entry || !entry.snapshot) {
    return false;
  }

  return applyDocumentSnapshot(asDocModelSnapshot(entry.snapshot), mode);
}

function undoDocumentChange(): boolean {
  if (!docModel) {
    return false;
  }

  const previous = documentHistory.undo();
  if (previous) {
    setStatus('Undid change');
  }
  return Boolean(previous);
}

function redoDocumentChange(): boolean {
  if (!docModel) {
    return false;
  }

  const next = documentHistory.redo();
  if (next) {
    setStatus('Redid change');
  }
  return Boolean(next);
}

function isEditModeState(state: string): boolean {
  return state === DOC_VIEW_STATES.LOADING_EDIT || state === DOC_VIEW_STATES.READY_EDIT;
}

function isReadyViewState(state: string): boolean {
  return state === DOC_VIEW_STATES.READY_EDIT || state === DOC_VIEW_STATES.READY_READ;
}

function updateRuntimeStateAttributes(): void {
  const page = document.querySelector<HTMLElement>('.page');
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

function transitionDocViewState(event: string): boolean {
  if (!docViewMachine.transition(event)) {
    return false;
  }

  const before = captureFsmSnapshot();
  docViewState = String(docViewMachine.state || DOC_VIEW_STATES.BOOTSTRAPPING);
  updateRuntimeStateAttributes();
  fsmTransitionHistory.record({
    machine: 'doc-view',
    event,
    before,
    after: captureFsmSnapshot(),
  });
  return true;
}

function transitionDocSaveState(event: string): boolean {
  if (!docSaveMachine.transition(event)) {
    return false;
  }

  const before = captureFsmSnapshot();
  docSaveState = String(docSaveMachine.state || DOC_SAVE_STATES.IDLE);
  updateRuntimeStateAttributes();
  fsmTransitionHistory.record({
    machine: 'doc-save',
    event,
    before,
    after: captureFsmSnapshot(),
  });
  return true;
}

function currentDocSourceText(): string {
  if (!docModel) {
    return '';
  }

  return stringifyDoc(docModel);
}

function canApplyExternalSourceNow(): boolean {
  return ensureDocumentLifecycleController().canApplyExternalSourceNow();
}

function applyIncomingSourceText(sourceText: string): void {
  ensureDocumentLifecycleController().applyIncomingSourceText(sourceText);
}

function tryApplyPendingExternalSource(): void {
  ensureDocumentLifecycleController().tryApplyPendingExternalSource();
}

function startSaveRequest(sourceText: string): void {
  ensureDocumentLifecycleController().startSaveRequest(sourceText);
}

function debouncedAutosave(): void {
  ensureDocumentLifecycleController().debouncedAutosave();
}

function hasActiveEditingSurface() {
  ensureControllers();
  return hasOpenBlockSources() || Boolean(inlineCssController && inlineCssController.hasOpenSurface());
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

let autocompleteState: AutocompleteController['state'] | null = null;

function setStatus(text: string): void {
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

function setStatusPersistent(text: string, state = 'idle'): void {
  const el = document.getElementById('status');
  if (!el) return;

  el.textContent = text;
  el.style.display = 'block';
  el.dataset.state = state;
}

function clearStatusPersistent(): void {
  const el = document.getElementById('status');
  if (!el) return;
  el.dataset.state = 'transient';
  el.style.display = 'none';
}

function isTypingTarget(element: Element | null): boolean {
  if (!element) return false;
  const tag = String(element.tagName || '').toLowerCase();
  const isContentEditable = element instanceof HTMLElement ? element.isContentEditable : false;
  return tag === 'textarea' || tag === 'input' || tag === 'select' || isContentEditable;
}

function getEventElementTarget(event: Event): Element | null {
  const target = event && event.target;
  if (target instanceof Element) {
    return target;
  }

  if (target instanceof Node && target.parentElement instanceof Element) {
    return target.parentElement;
  }

  return null;
}

function escapeHtml(value: JsonLike): string {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function normalizeClassName(value: JsonLike): string {
  return String(value || '')
    .split(/\s+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .join(' ');
}

function splitClassNames(value: JsonLike): string[] {
  const className = normalizeClassName(value);
  return className ? className.split(/\s+/) : [];
}


function ensureCustomCssSheet(): CSSStyleSheet {
  if (customCssSheet) {
    return customCssSheet;
  }

  customCssSheet = new CSSStyleSheet();
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, customCssSheet];
  return customCssSheet;
}

function applyCustomCss(cssText: JsonLike): boolean {
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
  return inlineCssController ? inlineCssController.getActiveScopedCss() : '';
}

function extractCssFromDocumentModel(): string {
  return extractCssFromModel(docModel);
}

function extractStylesheetLinksFromDocumentModel(): Array<{ href: string; media: string }> {
  return extractStylesheetLinksFromModel(docModel);
}

function resolveStylesheetHref(href: JsonLike): string {
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

function applyExternalStylesheetLinks(entries: Array<{ href: string; media: string }>): void {
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

    let linkEl = head.querySelector<HTMLLinkElement>(`link[data-doc-stylesheet-key="${CSS.escape(key)}"]`);

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

  head.querySelectorAll<HTMLLinkElement>('link[data-doc-stylesheet-key]').forEach((node) => {
    const key = String(node.dataset.docStylesheetKey || '');
    if (!activeKeys.has(key)) {
      node.remove();
    }
  });
}

function refreshDocumentCss(): void {
  const effectiveCss = extractCssFromDocumentModel();
  applyExternalStylesheetLinks(extractStylesheetLinksFromDocumentModel());
  applyCustomCss(effectiveCss);
  publishViewState(effectiveCss);
}

function publishViewState(effectiveCss: JsonLike): void {
  const resolvedTheme = String(document.body && document.body.dataset ? document.body.dataset.resolvedTheme || 'dark' : 'dark');
  const sourceText = docModel ? stringifyDoc(docModel) : '';
  const viewportWidth = Number.isFinite(window.innerWidth) ? Math.max(1, Math.round(window.innerWidth)) : null;
  const viewportHeight = Number.isFinite(window.innerHeight) ? Math.max(1, Math.round(window.innerHeight)) : null;
  const pixelRatio = Number.isFinite(window.devicePixelRatio) ? Number(window.devicePixelRatio) : null;
  const appearance = ensureUiStateController().getCurrentAppearance();

  vscode.postMessage({
    type: 'view-state',
    payload: {
      docPath: currentDocPath,
      theme: ensureUiStateController().getCurrentTheme(),
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
        paper: appearance.paper,
        density: appearance.density,
        scale: appearance.scale,
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

function collectKnownIds(): string[] {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const known = new Set<string>();

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

function collectKnownImageSources(): string[] {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const known = new Set<string>();

  for (const block of docModel.blocks) {
    if (!block || block.type !== 'image') continue;
    const src = String(block.src || '').trim();
    if (src) {
      known.add(src);
    }
  }

  return Array.from(known).sort((a, b) => a.localeCompare(b));
}

function collectKnownClasses(): string[] {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return [];
  }

  const known = new Set<string>();

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

function getLineContext(textarea: HTMLTextAreaElement) {
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

function getAutocompleteSuggestions(textarea: HTMLTextAreaElement | null, forceOpen = false) {
  return ensureAutocompleteController().getAutocompleteSuggestions(textarea, forceOpen);
}

function getAutocompleteEls(textarea: HTMLTextAreaElement | null): { menuEl: HTMLDivElement | null; mirrorEl: HTMLDivElement | null } {
  const srcWrap = textarea ? textarea.closest('.block-src-wrapper') : null;
  if (!srcWrap) return { menuEl: null, mirrorEl: null };
  const menuEl = srcWrap.querySelector<HTMLDivElement>('.autocomplete-menu');
  const mirrorEl = srcWrap.querySelector<HTMLDivElement>('.block-src-mirror');
  return {
    menuEl,
    mirrorEl,
  };
}

function renderGhostText(textarea: HTMLTextAreaElement, mirrorEl: HTMLDivElement | null): void {
  if (!mirrorEl || !autocompleteState || !autocompleteState.suggestions.length) return;
  const { suggestions, selectedIndex, replaceStart, replaceEnd, typed } = autocompleteState;
  const selected = suggestions[selectedIndex];
  if (!selected) return;
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
  if (srcWrap instanceof HTMLElement) srcWrap.classList.add('ghost-active');
  textarea.classList.add('ghost-active');
}

function closeAutocomplete(): void {
  ensureAutocompleteController().closeAutocomplete();
}

function renderAutocomplete(textarea: HTMLTextAreaElement | null, forceOpen = false): boolean {
  return ensureAutocompleteController().renderAutocomplete(textarea, forceOpen);
}

function positionAutocompleteMenu(textarea: HTMLTextAreaElement | null, menuEl: HTMLDivElement | null): void {
  return ensureAutocompleteController().positionAutocompleteMenu(textarea, menuEl);
}

function updateAutocompleteSelection(delta: number): boolean {
  return ensureAutocompleteController().updateAutocompleteSelection(delta);
}

function acceptAutocomplete(textarea: HTMLTextAreaElement | null, explicitIndex?: number): boolean {
  return ensureAutocompleteController().acceptAutocomplete(textarea, explicitIndex);
}

function isBulletedListType(type: string): boolean {
  return type === 'list' || type === 'bulleted-list';
}

function isNumberedListType(type: string): boolean {
  return type === 'numbered-list';
}

// ---------------------------------------------------------------------------
// Inline link support — [label](relative/path.dx) syntax in paragraph/quote/list text
// ---------------------------------------------------------------------------

function parseInlineLinks(text: string): Array<{ type: 'text'; value: string } | { type: 'link'; label: string; href: string }> {
  const tokens: Array<{ type: 'text'; value: string } | { type: 'link'; label: string; href: string }> = [];
  const pattern = /\[([^\]]+)\]\(([^)]+)\)/g;
  let lastIndex = 0;
  let match = pattern.exec(text);
  while (match !== null) {
    if (match.index > lastIndex) {
      tokens.push({ type: 'text', value: text.slice(lastIndex, match.index) });
    }
    tokens.push({ type: 'link', label: match[1] || '', href: match[2] || '' });
    lastIndex = match.index + match[0].length;
    match = pattern.exec(text);
  }
  if (lastIndex < text.length) {
    tokens.push({ type: 'text', value: text.slice(lastIndex) });
  }
  return tokens;
}

function setInlineContent(el: HTMLElement, text: string): void {
  const tokens = parseInlineLinks(String(text || ''));
  const firstToken = tokens[0];
  if (tokens.length === 1 && firstToken && firstToken.type === 'text') {
    el.textContent = firstToken.value;
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

function buildRenderedContent(block: PipelineBlock | null | undefined): Node {
  if (!block) {
    return document.createTextNode('');
  }
  return ensureBlockRendererController().buildRenderedContent(block);
}

function buildBlockWrap(block: PipelineBlock, index: number): HTMLDivElement {
  return ensureBlockRendererController().buildBlockWrap(block, index);
}

function autosizeBlockSrc(textarea: HTMLTextAreaElement): void {
  if (!textarea) {
    return;
  }
  textarea.style.height = '0px';
  textarea.style.height = `${textarea.scrollHeight}px`;
}

function getRawSourceForEditor(rawSource: JsonLike): string {
  return ensureBlockRendererController().getRawSourceForEditor(rawSource);
}

function getRawSourceFromEditor(editorValue: JsonLike): string {
  return ensureBlockRendererController().getRawSourceFromEditor(editorValue);
}

function splitBlockSourceForEditor(rawSource: JsonLike, blockType: string) {
  return ensureBlockRendererController().splitBlockSourceForEditor(rawSource, blockType);
}

function buildRawSourceFromEditorParts(headerSource: JsonLike, bodySource: JsonLike, footerSource: JsonLike): string {
  return ensureBlockRendererController().buildRawSourceFromEditorParts(headerSource, bodySource, footerSource);
}

function getBlockHeaderEditor(textarea: HTMLTextAreaElement | null): HTMLTextAreaElement | null {
  return ensureBlockRendererController().getBlockHeaderEditor(textarea);
}

function getHeaderSourceFromEditor(textarea: HTMLTextAreaElement): string {
  return ensureBlockRendererController().getHeaderSourceFromEditor(textarea);
}

function renderEditableHeader(textarea: HTMLTextAreaElement): void {
  return ensureBlockRendererController().renderEditableHeader(textarea);
}

function applyEditableBodyPresentation(wrap: HTMLElement, block: PipelineBlock | null | undefined): void {
  return ensureBlockRendererController().applyEditableBodyPresentation(wrap, block || null);
}

function clearEditableBodyPresentation(wrap: HTMLElement): void {
  return ensureBlockRendererController().clearEditableBodyPresentation(wrap);
}

function findAttributeTargetAtCursor(textarea: HTMLTextAreaElement): { selector: string; kind: string } | null {
  const ctx = getLineContext(textarea);
  const cursorInLine = ctx.cursor - ctx.lineStart;
  const pattern = /([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g;
  let match = pattern.exec(ctx.lineText);

  while (match) {
    const key = String(match[1] || '').toLowerCase();
    const value = match[2] ?? match[3] ?? match[4] ?? '';
    const quoted = match[2] != null || match[3] != null;
    const valueStart = match.index + String(match[1] || '').length + 1 + (quoted ? 1 : 0);
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

function findCssBlockIndex(): number {
  return findCssBlockIndexInModel(docModel);
}

function splitRuleSelectors(selectorText: JsonLike): string[] {
  return String(selectorText || '')
    .split(',')
    .map((token) => token.trim())
    .filter(Boolean);
}

function isSelectorForTarget(selector: JsonLike, target: JsonLike): boolean {
  const normalizedSelector = String(selector || '').trim();
  const normalizedTarget = String(target || '').trim();

  if (!normalizedSelector || !normalizedTarget) {
    return false;
  }

  return normalizedSelector === normalizedTarget
    || normalizedSelector.startsWith(normalizedTarget + ':')
    || normalizedSelector.startsWith(normalizedTarget + '::');
}

function getScopedCssForSelector(cssText: JsonLike, selector: JsonLike): string {
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

function getScopedCssDeclarations(cssText: JsonLike, selector: JsonLike): string {
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

function buildScopedRule(selector: JsonLike, declarationsText: JsonLike): string {
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

function mergeScopedCssForSelector(baseCssText: JsonLike, selector: JsonLike, declarationsText: JsonLike): string {
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

function upsertCssBlock(cssText: JsonLike): void {
  if (!docModel || !Array.isArray(docModel.blocks)) {
    return;
  }

  const text = String(cssText || '').trim();
  let cssIndex = findCssBlockIndex();

  if (cssIndex < 0) {
    const block: PipelineBlock = {
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
  if (!block) {
    return;
  }
  block.language = 'css';
  block.text = text;
  block.rawSource = stringifyBlock({ ...block, rawSource: '' });
}

function ensureControllers(): void {
  ensureBlockRendererController();
  ensureAutocompleteController();

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
      updateInlineCssAffordance: (textarea) => {
        if (inlineCssController) {
          inlineCssController.updateInlineCssAffordance(textarea);
        }
      },
    });
  }
}

function closeInlineCssSurface(restoreFocus?: boolean): void {
  ensureControllers();
  if (inlineCssController) {
    inlineCssController.closeInlineCssSurface(Boolean(restoreFocus));
  }
}

function openInlineCssSurface(source: HTMLTextAreaElement | null, selector: string): void {
  ensureControllers();
  if (inlineCssController) {
    inlineCssController.openInlineCssSurface(source, selector);
  }
}

function closeBlockSrc(index: number, commitChanges: boolean): void {
  ensureControllers();
  if (blockSourceController) {
    blockSourceController.closeBlockSrc(index, commitChanges);
  }
}

function hasPendingBlockSourceChanges(source: HTMLTextAreaElement | null): boolean {
  ensureControllers();
  return blockSourceController ? blockSourceController.hasPendingBlockSourceChanges(source) : false;
}

function commitBlockSourceForHistory(index: number): void {
  ensureControllers();
  if (blockSourceController) {
    blockSourceController.commitBlockSourceForHistory(index);
  }
}

function commitOpenSources(exceptIndex?: number): void {
  ensureControllers();
  if (blockSourceController) {
    blockSourceController.commitOpenSources(exceptIndex);
  }
}

function commitOpenSourcesForHistory(exceptIndex?: number): void {
  ensureControllers();
  if (blockSourceController) {
    blockSourceController.commitOpenSourcesForHistory(exceptIndex);
  }
}

function hasOpenBlockSources(): boolean {
  ensureControllers();
  return blockSourceController ? blockSourceController.hasOpenBlockSources() : false;
}

function isBlankPageClickTarget(target: Element | null, pageEl: Element | null, blocksContainer: Element | null): boolean {
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

function updateInlineCssAffordance(textarea: HTMLTextAreaElement): void {
  ensureControllers();
  if (inlineCssController) {
    inlineCssController.updateInlineCssAffordance(textarea);
  }
}

function openBlockSrc(index: number): void {
  ensureControllers();
  if (blockSourceController) {
    blockSourceController.openBlockSrc(index);
  }
}

function commitBlockSrc(index: number): void {
  ensureControllers();
  if (blockSourceController) {
    blockSourceController.commitBlockSrc(index);
  }
}

function renderDocument(): void {
  ensureDocumentLifecycleController().renderDocument();
}

function saveDoc(): void {
  ensureDocumentLifecycleController().saveDoc();
}

function saveDocAuto(): void {
  ensureDocumentLifecycleController().saveDocAuto();
}

function getInsertIndexForY(container: HTMLElement, clientY: number): number {
  const wraps = Array.from(container.querySelectorAll('.block-wrap'));

  for (let index = 0; index < wraps.length; index += 1) {
    const wrap = wraps[index];
    if (!(wrap instanceof HTMLElement)) {
      continue;
    }
    const rect = wrap.getBoundingClientRect();

    if (clientY < rect.top + rect.height / 2) {
      return index;
    }
  }

  return wraps.length;
}

function insertParagraphBlock(index: number): void {
  if (!docModel || !Array.isArray(docModel.blocks)) return;

  recordDocumentUndo('insert-paragraph');

  const next: PipelineBlock = {
    type: 'paragraph',
    text: '',
    id: '',
    className: '',
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
    postMessage: (message: Record<string, JsonLike>) => vscode.postMessage(message),
    publishViewState,
    getActiveScopedCss,
    closeInlineCssSurface,
    commitOpenSources,
    closeAutocomplete,
    setStatus,
    tryApplyPendingExternalSource,
  });

  return uiStateController;
}

function applyTheme(theme: string, persist = false): void {
  ensureUiStateController().applyTheme(theme, Boolean(persist));
}

function setControlsOpen(isOpen: boolean): void {
  ensureUiStateController().setControlsOpen(Boolean(isOpen));
}

function setHelpOpen(isOpen: boolean): void {
  ensureUiStateController().setHelpOpen(Boolean(isOpen));
}

function revealChromeBriefly(durationMs = 1400) {
  ensureUiStateController().revealChromeBriefly(durationMs);
}

function toggleControls(forceOpen?: boolean): void {
  ensureUiStateController().toggleControls(forceOpen);
}

function toggleHelp(): void {
  ensureUiStateController().toggleHelp();
}

function isEditModeEnabled(): boolean {
  return ensureUiStateController().isEditModeEnabled();
}

function loadEditModePreference(): boolean {
  return ensureUiStateController().loadEditModePreference();
}

function setEditMode(enabled: boolean): void {
  ensureUiStateController().setEditMode(Boolean(enabled));
}

function getFocusedBlockIndex(): number | null {
  const active = document.activeElement;
  if (!active || !(active instanceof Element)) {
    return null;
  }

  const wrap = active.closest('.block-wrap');
  if (!wrap) {
    return null;
  }

  const index = Number.parseInt((wrap as HTMLElement).dataset.blockIndex || '', 10);
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

function ensureSurfaceController() {
  if (surfaceController) return surfaceController;
  surfaceController = createSurfaceController({
    getDocModel: () => docModel,
    getCurrentDocPath: () => currentDocPath,
    getFsmViewState: () => ({
      documentState: docViewState,
      saveState: docSaveState,
      historyLength: fsmTransitionHistory.length,
      lastTransition: fsmTransitionHistory.lastTransition,
    }),
    getDocumentHistoryDepths: () => ({
      undoDepth: documentHistory.undoDepth,
      redoDepth: documentHistory.redoDepth,
    }),
    getCurrentTheme: () => ensureUiStateController().getCurrentTheme(),
    getResolvedTheme: () => String(document.body.dataset.resolvedTheme || 'light'),
    isEditModeEnabled,
    getFocusedBlockIndex,
    summarizeBlockTypeCounts,
    setEditMode,
    openBlockSource: openBlockSrc,
    closeBlockSource: closeBlockSrc,
    undoLastFsmTransition,
    commitOpenSourcesForHistory,
    performGlobalUndo,
    performGlobalRedo,
  });
  return surfaceController;
}

function toggleEditMode() {
  ensureUiStateController().toggleEditMode();
}

function hideLoadingScreen() {
  const loadingScreen = document.getElementById('loading-screen');
  const page = document.querySelector<HTMLElement>('.page');

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

function insertImageBlock(imagePath: string, altText: string, atIndex?: number) {
  if (!docModel) return;

  recordDocumentUndo('insert-image');

  const block: PipelineBlock = {
    type: 'image',
    src: String(imagePath || '').trim(),
    alt: String(altText || '').trim(),
    id: '',
    className: '',
    rawSource: '',
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

type ImageUploadPayload = {
  name: string;
  mimeType: string;
  base64Data: string;
};

function readFileAsPayload(file: File): Promise<ImageUploadPayload> {
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

function wireDragAndDrop(): void {
  let pendingDropIndex = -1;
  let ghostEl: HTMLDivElement | null = null;

  function ensureGhost(): HTMLDivElement {
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

  function removeGhost(): void {
    if (ghostEl && ghostEl.parentNode) {
      ghostEl.parentNode.removeChild(ghostEl);
    }
    pendingDropIndex = -1;
  }

  function isDraggedImages(event: DragEvent): boolean {
    const dt = event.dataTransfer;
    if (!dt) return false;
    if (dt.items && dt.items.length > 0) {
      for (let i = 0; i < dt.items.length; i += 1) {
        const item = dt.items[i];
        if (item && String(item.type || '').startsWith('image/')) return true;
      }
    }
    return Array.from(dt.types || []).includes('Files');
  }

  function positionGhost(event: DragEvent): void {
    const container = document.getElementById('blocks');
    if (!(container instanceof HTMLElement)) return;
    const insertIdx = getInsertIndexForY(container, event.clientY);
    if (insertIdx !== pendingDropIndex) {
      pendingDropIndex = insertIdx;
      const ghost = ensureGhost();
      const wraps = container.querySelectorAll(':scope > .block-wrap');
      if (insertIdx >= wraps.length) {
        container.appendChild(ghost);
      } else {
        container.insertBefore(ghost, wraps[insertIdx] || null);
      }
    }
  }

  function clearDropState(): void {
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
  const sourceText = (sourceEl instanceof HTMLTextAreaElement || sourceEl instanceof HTMLInputElement)
    ? sourceEl.value
    : '';
  const errorText = initEl && initEl.dataset && initEl.dataset.docError ? initEl.dataset.docError : '';
  const initialTheme = initEl && initEl.dataset && initEl.dataset.initialTheme ? initEl.dataset.initialTheme : 'auto';
  const initialPaper = initEl && initEl.dataset && initEl.dataset.initialPaper ? initEl.dataset.initialPaper : 'white';
  const initialDensity = initEl && initEl.dataset && initEl.dataset.initialDensity ? initEl.dataset.initialDensity : 'comfortable';
  const initialScale = initEl && initEl.dataset && initEl.dataset.initialScale ? Number(initEl.dataset.initialScale) : 100;
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
      docModel = { blocks: [] };
    } else {
      docModel = parseDoc(sourceText);
    }

    renderDocument();
    lastSavedDoc = stringifyDoc(docModel || { blocks: [] });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus('Parse error: ' + message);
  }

  const blocksContainer = document.getElementById('blocks');

  if (blocksContainer) {
    ensureControllers();
    registerBlockInteractionEvents({
      blocksContainer,
      pageEl: document.querySelector('.page'),
      documentRef: document,
      windowRef: window,
      getEventElementTarget,
      autocompleteState: autocompleteState
        ? {
            textarea: autocompleteState.textarea,
            suggestions: autocompleteState.suggestions.map((entry) => String(entry.insertText || '')),
          }
        : undefined,
      acceptAutocomplete,
      closeAutocomplete,
      closeInlineCssSurface,
      commitBlockSourceForHistory,
      commitOpenSources,
      closeBlockSrc,
      openBlockSrc,
      isEditModeEnabled,
      isRedoShortcut,
      performGlobalUndo: () => Boolean(performGlobalUndo()),
      performGlobalRedo: () => Boolean(performGlobalRedo()),
      saveDoc,
      renderAutocomplete,
      updateInlineCssAffordance,
      ensureDocumentUndoSeed,
      autosizeBlockSrc,
      renderEditableHeader,
      getRawSourceFromEditor,
      getBlockHeaderEditor,
      findAttributeTargetAtCursor,
      openInlineCssSurface,
      setStatus,
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
      blankPagePointerDownHadOpenEditors = hasOpenBlockSources() || Boolean(inlineCssController && inlineCssController.hasOpenSurface());
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
      const hadOpenInlineCss = Boolean(inlineCssController && inlineCssController.hasOpenSurface());
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

      const pointerY = event instanceof MouseEvent ? event.clientY : 0;
      const index = getInsertIndexForY(blocksContainer, pointerY);
      insertParagraphBlock(index);
    });
  }

  const themeSelect = document.getElementById('theme-select');
  if (themeSelect) {
    themeSelect.addEventListener('change', (event) => {
      const target = event.target;
      if (target instanceof HTMLSelectElement) {
        applyTheme(target.value, true);
      }
    });
  }

  const paperSelect = document.getElementById('paper-select');
  if (paperSelect) {
    paperSelect.addEventListener('change', (event) => {
      const target = event.target;
      if (!(target instanceof HTMLSelectElement)) {
        return;
      }
      ensureUiStateController().updateAppearance({
        paper: target.value === 'cream' || target.value === 'slate' ? target.value : 'white',
      }, true);
    });
  }

  const densitySelect = document.getElementById('density-select');
  if (densitySelect) {
    densitySelect.addEventListener('change', (event) => {
      const target = event.target;
      if (!(target instanceof HTMLSelectElement)) {
        return;
      }
      ensureUiStateController().updateAppearance({
        density: target.value === 'compact' ? 'compact' : 'comfortable',
      }, true);
    });
  }

  const scaleSlider = document.getElementById('scale-slider');
  if (scaleSlider) {
    scaleSlider.addEventListener('input', (event) => {
      const target = event.target;
      if (!(target instanceof HTMLInputElement)) {
        return;
      }
      ensureUiStateController().updateAppearance({
        scale: Number(target.value || 100),
      }, true);
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
      const href = String((docLink as HTMLElement).dataset.docHref || '').trim();
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
      if (ensureUiStateController().getCurrentTheme() === 'auto') {
        applyTheme('auto', false);
      }
    });
  }

  try {
    applyTheme(initialTheme, false);
    ensureUiStateController().updateAppearance({
      paper: initialPaper === 'cream' || initialPaper === 'slate' ? initialPaper : 'white',
      density: initialDensity === 'compact' ? 'compact' : 'comfortable',
      scale: initialScale,
    }, false);
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
    if (!(checkbox instanceof HTMLInputElement) || checkbox.type !== 'checkbox') return;

    const li = checkbox.parentElement;
    const ul = li && li.parentElement;
    if (!ul || !ul.classList.contains('checklist-wrap')) return;

    const blockView = ul.closest('.block-view');
    const blockWrap = blockView && blockView.closest('.block-wrap');
    if (!blockWrap) return;

    const blockIndex = Number.parseInt((blockWrap as HTMLElement).dataset.blockIndex || '', 10);
    const itemIndex = Number.parseInt(String(checkbox.dataset.itemIndex || ''), 10);
    if (!Number.isFinite(blockIndex) || !Number.isFinite(itemIndex)) return;

    const block = docModel && docModel.blocks && docModel.blocks[blockIndex];
    if (!block || block.type !== 'checklist' || !Array.isArray(block.items)) return;

    const item = block.items[itemIndex];
    if (!item) return;

    if (typeof item === 'object' && item !== null) {
      item.checked = checkbox.checked;
    }

    const span = li.querySelector('span');
    if (span) {
      span.classList.toggle('check-done', checkbox.checked);
    }

    block.rawSource = '';
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

      const blockViews = Array.from(blocksContainer.querySelectorAll<HTMLElement>('.block-view'));
      const focused = blockViews.findIndex((block) => block.contains(document.activeElement) || block === document.activeElement);

      if (focused === -1 && blockViews.length > 0) {
        const first = blockViews[0];
        if (first) {
          first.focus();
        }
      } else if (focused !== -1) {
        const nextFocus = event.shiftKey ? Math.max(0, focused - 1) : Math.min(blockViews.length - 1, focused + 1);
        const next = blockViews[nextFocus];
        if (next) {
          next.focus();
        }
      }
    }
  });

  window.addEventListener('message', (event) => {
    const msg = event.data || {};

    if (msg.type === 'surface-capture-request') {
      try {
        const snapshot = ensureSurfaceController().captureSurfaceSnapshot(msg.payload || {});
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
      ensureSurfaceController().runSurfaceAction(msg.payload || {})
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
      lastSavedDoc = stringifyDoc(docModel || { blocks: [] });
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
