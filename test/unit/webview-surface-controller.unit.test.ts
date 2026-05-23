import test from 'node:test';
import assert from 'node:assert/strict';
import { createSurfaceController } from '#runtime-media/webview-surface-controller.js';

// ---------------------------------------------------------------------------
// DOM stubs — installed once before the test functions run
// ---------------------------------------------------------------------------

const elements = new Map();
let _activeElement = null;
let _scrollByLastCall = null;
let _scrollToLastCall = null;
let _rafCallbacks = [];

const fakeDocument = {
  querySelector(selector) {
    return elements.get(selector) ?? null;
  },
  querySelectorAll(selector) {
    if (selector === '.block-wrap') return Array.from(elements.get('__block-wraps__') ?? []);
    return [];
  },
  get activeElement() {
    return _activeElement;
  },
  body: { dataset: { resolvedTheme: 'dark' } },
};

const fakeWindow = {
  innerWidth: 1280,
  innerHeight: 800,
  scrollX: 0,
  scrollY: 100,
  scrollBy(opts) { _scrollByLastCall = opts; },
  scrollTo(opts) { _scrollToLastCall = opts; },
  getComputedStyle(_el) {
    return { fontSize: '16px', lineHeight: '1.5', color: 'black', backgroundColor: 'white' };
  },
};

function requestAnimationFrame(cb) {
  _rafCallbacks.push(cb);
}

function flushRaf() {
  const cbs = _rafCallbacks.splice(0);
  cbs.forEach((cb) => cb());
}

global.document = fakeDocument;
global.window = fakeWindow;
global.requestAnimationFrame = requestAnimationFrame;

// ---------------------------------------------------------------------------
// Fake dependency builder
// ---------------------------------------------------------------------------

function makeBlockWrap(index) {
  const view = {
    focus() { this._focused = true; },
    textContent: `block ${index} text`,
    getBoundingClientRect() { return { x: 0, y: index * 50, width: 600, height: 40 }; },
  };
  const srcWrapper = { style: { display: 'none' } };
  return {
    dataset: { blockIndex: String(index) },
    querySelector(sel) {
      if (sel === '.block-view') return view;
      if (sel === '.block-src-wrapper') return srcWrapper;
      return null;
    },
    getBoundingClientRect() { return { x: 0, y: index * 50, width: 600, height: 40, top: index * 50, bottom: index * 50 + 40 }; },
    scrollIntoView(opts) { this._lastScrollIntoView = opts; },
    _view: view,
    _srcWrapper: srcWrapper,
  };
}

function makeOptions(overrides = {}) {
  const wrap0 = makeBlockWrap(0);
  const wrap1 = makeBlockWrap(1);
  elements.set('__block-wraps__', [wrap0, wrap1]);
  elements.set('.block-wrap[data-block-index="0"]', wrap0);
  elements.set('.block-wrap[data-block-index="1"]', wrap1);
  elements.set('.page', {
    getBoundingClientRect() { return { x: 0, y: 0, width: 800, height: 2000 }; },
  });

  const docModel = {
    blocks: [
      { id: 'b0', type: 'paragraph', text: 'Hello world' },
      { id: 'b1', type: 'heading', text: 'A heading' },
    ],
  };

  return {
    getDocModel: () => docModel,
    getCurrentDocPath: () => '/workspace/note.dx',
    getFsmViewState: () => ({
      documentState: 'READY',
      saveState: 'IDLE',
      historyLength: 3,
      lastTransition: 'MARK_READY',
    }),
    getDocumentHistoryDepths: () => ({ undoDepth: 2, redoDepth: 0 }),
    getCurrentTheme: () => 'ocean',
    getResolvedTheme: () => 'dark',
    isEditModeEnabled: () => false,
    getFocusedBlockIndex: () => null,
    summarizeBlockTypeCounts: () => ({
      total: 2, headings: 1, paragraphs: 1, lists: 0, codeBlocks: 0, images: 0, quotes: 0, rules: 0,
    }),
    setEditMode: () => {},
    openBlockSource: () => {},
    closeBlockSource: () => {},
    undoLastFsmTransition: () => false,
    commitOpenSourcesForHistory: () => {},
    performGlobalUndo: () => null,
    performGlobalRedo: () => null,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// captureSurfaceSnapshot
// ---------------------------------------------------------------------------

test('captureSurfaceSnapshot returns expected top-level shape', () => {
  const ctrl = createSurfaceController(makeOptions());
  const snap = ctrl.captureSurfaceSnapshot();

  assert.equal(snap.documentPath, '/workspace/note.dx');
  assert.equal(snap.theme, 'ocean');
  assert.equal(snap.resolvedTheme, 'dark');
  assert.equal(snap.editMode, false);
  assert.deepEqual(snap.fsm, { documentState: 'READY', saveState: 'IDLE', historyLength: 3, lastTransition: 'MARK_READY' });
  assert.deepEqual(snap.documentHistory, { undoDepth: 2, redoDepth: 0 });
  assert.equal(snap.viewport.width, 1280);
  assert.equal(snap.viewport.height, 800);
  assert.equal(snap.viewport.scrollY, 100);
  assert.ok(snap.capturedAt);
  assert.ok(new Date(snap.capturedAt).getTime() > 0, 'capturedAt is a valid ISO date');
});

test('captureSurfaceSnapshot includes block text by default', () => {
  const ctrl = createSurfaceController(makeOptions());
  const snap = ctrl.captureSurfaceSnapshot();

  assert.equal(snap.blocks.length, 2);
  assert.equal(snap.blocks[0].id, 'b0');
  assert.equal(snap.blocks[0].type, 'paragraph');
  assert.equal(snap.blocks[0].text, 'Hello world');
  assert.equal(snap.blocks[1].text, 'A heading');
});

test('captureSurfaceSnapshot omits text when includeText is false', () => {
  const ctrl = createSurfaceController(makeOptions());
  const snap = ctrl.captureSurfaceSnapshot({ includeText: false });

  assert.equal(snap.blocks[0].text, undefined);
  assert.equal(snap.blocks[1].text, undefined);
});

test('captureSurfaceSnapshot includes style when includeStyles is true', () => {
  const ctrl = createSurfaceController(makeOptions());
  const snap = ctrl.captureSurfaceSnapshot({ includeStyles: true });

  assert.ok(snap.blocks[0].style, 'style should be present');
  assert.equal(snap.blocks[0].style.fontSize, '16px');
});

test('captureSurfaceSnapshot omits style by default', () => {
  const ctrl = createSurfaceController(makeOptions());
  const snap = ctrl.captureSurfaceSnapshot();

  assert.equal(snap.blocks[0].style, undefined);
});

test('captureSurfaceSnapshot summarises block type counts', () => {
  const ctrl = createSurfaceController(makeOptions());
  const snap = ctrl.captureSurfaceSnapshot();

  assert.equal(snap.blockCounts.total, 2);
  assert.equal(snap.blockCounts.headings, 1);
  assert.equal(snap.blockCounts.paragraphs, 1);
});

// ---------------------------------------------------------------------------
// runSurfaceAction — error cases
// ---------------------------------------------------------------------------

test('runSurfaceAction throws when action is missing', async () => {
  const ctrl = createSurfaceController(makeOptions());
  await assert.rejects(() => ctrl.runSurfaceAction({}), /Surface action is required/);
});

test('runSurfaceAction throws on unknown action', async () => {
  const ctrl = createSurfaceController(makeOptions());
  const p = ctrl.runSurfaceAction({ action: 'doesNotExist' });
  flushRaf();
  await assert.rejects(() => p, /Unknown surface action/);
});

test('runSurfaceAction undoState throws when no transition available', async () => {
  const ctrl = createSurfaceController(makeOptions({ undoLastFsmTransition: () => false }));
  await assert.rejects(() => ctrl.runSurfaceAction({ action: 'undoState' }), /No FSM transition available/);
});

test('runSurfaceAction undoDocument throws when nothing to undo', async () => {
  const ctrl = createSurfaceController(makeOptions({ performGlobalUndo: () => null }));
  await assert.rejects(() => ctrl.runSurfaceAction({ action: 'undoDocument' }), /No document edit available to undo/);
});

test('runSurfaceAction redoDocument throws when nothing to redo', async () => {
  const ctrl = createSurfaceController(makeOptions({ performGlobalRedo: () => null }));
  await assert.rejects(() => ctrl.runSurfaceAction({ action: 'redoDocument' }), /No document edit available to redo/);
});

test('runSurfaceAction scrollToBlock throws when block not found', async () => {
  const ctrl = createSurfaceController(makeOptions());
  await assert.rejects(
    () => ctrl.runSurfaceAction({ action: 'scrollToBlock', blockIndex: 999 }),
    /Unable to scroll to target block/,
  );
});

test('runSurfaceAction focusBlock throws when block not found', async () => {
  const ctrl = createSurfaceController(makeOptions());
  await assert.rejects(
    () => ctrl.runSurfaceAction({ action: 'focusBlock', blockIndex: 999 }),
    /Unable to focus target block/,
  );
});

test('runSurfaceAction openBlockSource throws on non-finite index', async () => {
  const ctrl = createSurfaceController(makeOptions());
  await assert.rejects(
    () => ctrl.runSurfaceAction({ action: 'openBlockSource', blockIndex: 'bad' }),
    /A valid block index is required/,
  );
});

// ---------------------------------------------------------------------------
// runSurfaceAction — success paths
// ---------------------------------------------------------------------------

test('runSurfaceAction setEditMode calls setEditMode and returns snapshot', async () => {
  let called = null;
  const ctrl = createSurfaceController(makeOptions({ setEditMode: (v) => { called = v; } }));
  const p = ctrl.runSurfaceAction({ action: 'setEditMode', enabled: true });
  flushRaf();
  const snap = await p;
  assert.equal(called, true);
  assert.ok(snap.documentPath);
});

test('runSurfaceAction scrollBy calls window.scrollBy', async () => {
  _scrollByLastCall = null;
  const ctrl = createSurfaceController(makeOptions());
  const p = ctrl.runSurfaceAction({ action: 'scrollBy', deltaY: 200 });
  flushRaf();
  await p;
  assert.deepEqual(_scrollByLastCall, { top: 200, behavior: 'auto' });
});

test('runSurfaceAction scrollTo calls window.scrollTo', async () => {
  _scrollToLastCall = null;
  const ctrl = createSurfaceController(makeOptions());
  const p = ctrl.runSurfaceAction({ action: 'scrollTo', top: 500 });
  flushRaf();
  await p;
  assert.deepEqual(_scrollToLastCall, { top: 500, behavior: 'auto' });
});

test('runSurfaceAction scrollToBlock calls scrollIntoView on the wrap', async () => {
  const ctrl = createSurfaceController(makeOptions());
  const wrap = elements.get('.block-wrap[data-block-index="0"]');
  wrap._lastScrollIntoView = null;
  const p = ctrl.runSurfaceAction({ action: 'scrollToBlock', blockIndex: 0 });
  flushRaf();
  await p;
  assert.ok(wrap._lastScrollIntoView, 'scrollIntoView was called');
  assert.equal(wrap._lastScrollIntoView.block, 'center');
});

test('runSurfaceAction undoState succeeds when undoLastFsmTransition returns true', async () => {
  const ctrl = createSurfaceController(makeOptions({ undoLastFsmTransition: () => true }));
  const p = ctrl.runSurfaceAction({ action: 'undoState' });
  flushRaf();
  const snap = await p;
  assert.ok(snap.documentPath);
});

test('runSurfaceAction undoDocument calls commitOpenSourcesForHistory', async () => {
  let committed = false;
  const ctrl = createSurfaceController(makeOptions({
    commitOpenSourcesForHistory: () => { committed = true; },
    performGlobalUndo: () => 'document',
  }));
  const p = ctrl.runSurfaceAction({ action: 'undoDocument' });
  flushRaf();
  await p;
  assert.ok(committed);
});

test('runSurfaceAction result snapshot reflects deps at call time', async () => {
  let editMode = false;
  const ctrl = createSurfaceController(makeOptions({ isEditModeEnabled: () => editMode }));
  editMode = true;
  const p = ctrl.runSurfaceAction({ action: 'setEditMode', enabled: true });
  flushRaf();
  const snap = await p;
  assert.equal(snap.editMode, true);
});
