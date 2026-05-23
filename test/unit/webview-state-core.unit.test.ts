import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BoundedHistory,
  CallbackUndoRedoController,
  computeDirtyReconcileResult,
  TableDrivenStateMachine,
  TransitionHistory,
} from '#runtime-media/webview-state-core.js';

test('BoundedHistory enforces max length and preserves most recent values', () => {
  const history = new BoundedHistory(2);
  history.push('a');
  history.push('b');
  history.push('c');

  assert.deepEqual(history.toArray(), ['b', 'c']);
  assert.equal(history.peek(), 'c');
  assert.equal(history.pop(), 'c');
  assert.deepEqual(history.toArray(), ['b']);
});

test('TableDrivenStateMachine transitions through valid events and rejects invalid ones', () => {
  const machine = new TableDrivenStateMachine('doc-view', 'boot', {
    boot: { LOAD_OK: 'ready' },
    ready: { START_EDIT: 'editing' },
    editing: { STOP_EDIT: 'ready' },
  });

  assert.equal(machine.transition('LOAD_OK'), true);
  assert.equal(machine.state, 'ready');
  assert.equal(machine.transition('START_EDIT'), true);
  assert.equal(machine.state, 'editing');
  assert.equal(machine.transition('MISSING_EVENT'), false);
  assert.equal(machine.state, 'editing');
});

test('CallbackUndoRedoController round-trips snapshots across undo/redo', () => {
  const state = { value: 0 };
  const controller = new CallbackUndoRedoController({
    limit: 10,
    captureSnapshot: () => ({ ...state }),
    restoreSnapshot: (snapshot) => {
      state.value = Number(snapshot.value || 0);
      return true;
    },
  });

  controller.push('seed');
  state.value = 1;
  controller.push('increment');
  state.value = 2;

  const undone = controller.undo();
  assert.ok(undone);
  assert.equal(state.value, 1);
  assert.equal(controller.undoDepth, 1);
  assert.equal(controller.redoDepth, 1);

  const redone = controller.redo();
  assert.ok(redone);
  assert.equal(state.value, 2);
  assert.equal(controller.undoDepth, 2);
  assert.equal(controller.redoDepth, 0);
});

test('TransitionHistory records and replays before/after snapshots', () => {
  const history = new TransitionHistory(8);
  const state = { mode: 'boot' };

  history.record({
    machine: 'doc-view',
    event: 'LOAD_OK',
    before: { mode: 'boot' },
    after: { mode: 'ready' },
  });

  assert.equal(history.length, 1);
  assert.equal(history.lastTransition?.event, 'LOAD_OK');

  const undoWorked = history.undo((snapshot) => {
    state.mode = snapshot.mode;
    return true;
  });
  assert.equal(undoWorked, true);
  assert.equal(state.mode, 'boot');

  const redoWorked = history.redo((snapshot) => {
    state.mode = snapshot.mode;
    return true;
  });
  assert.equal(redoWorked, true);
  assert.equal(state.mode, 'ready');
});

test('computeDirtyReconcileResult marks clean whenever source matches saved snapshot', () => {
  const result = computeDirtyReconcileResult({
    latestSource: 'same text',
    lastSavedSource: 'same text',
    hadDirtyWorkingCopySignal: true,
    emitDirtySync: true,
  });

  assert.equal(result.isDirty, false);
  assert.equal(result.shouldPostMarkDirty, false);
  assert.equal(result.shouldPostMarkClean, true);
  assert.equal(result.nextHasDirtyWorkingCopySignal, false);
});

test('computeDirtyReconcileResult still emits clean when already synchronized at source level', () => {
  const result = computeDirtyReconcileResult({
    latestSource: 'saved text',
    lastSavedSource: 'saved text',
    hadDirtyWorkingCopySignal: false,
    emitDirtySync: true,
  });

  assert.equal(result.isDirty, false);
  assert.equal(result.shouldPostMarkDirty, false);
  assert.equal(result.shouldPostMarkClean, true);
  assert.equal(result.nextHasDirtyWorkingCopySignal, false);
});

test('computeDirtyReconcileResult posts dirty update while source differs from saved snapshot', () => {
  const result = computeDirtyReconcileResult({
    latestSource: 'changed',
    lastSavedSource: 'original',
    hadDirtyWorkingCopySignal: false,
    emitDirtySync: true,
  });

  assert.equal(result.isDirty, true);
  assert.equal(result.shouldPostMarkDirty, true);
  assert.equal(result.shouldPostMarkClean, false);
  assert.equal(result.nextHasDirtyWorkingCopySignal, true);
});
