/**
 * Reusable state/history utilities for the DOC webview.
 * These are intentionally UI-agnostic so behavior can be tested in isolation.
 */

type SnapshotValue = string | number | boolean | null | undefined | object;

export class BoundedHistory<TItem> {
  limit: number;
  items: TItem[];

  constructor(limit = 100) {
    this.limit = Number.isFinite(limit) && limit > 0 ? Math.floor(limit) : 100;
    this.items = [];
  }

  /**
   * @param {TItem} item
   */
  push(item: TItem): void {
    this.items.push(item);
    if (this.items.length > this.limit) {
      this.items = this.items.slice(-this.limit);
    }
  }

  /**
   * @returns {TItem | undefined}
   */
  pop(): TItem | undefined {
    return this.items.pop();
  }

  /**
   * @returns {TItem | undefined}
   */
  peek(): TItem | undefined {
    return this.items.length > 0 ? this.items[this.items.length - 1] : undefined;
  }

  clear(): void {
    this.items = [];
  }

  /**
   * @returns {number}
   */
  get length() {
    return this.items.length;
  }

  /**
   * @returns {TItem[]}
   */
  toArray(): TItem[] {
    return [...this.items];
  }
}

export class AbstractStateMachine {
  name: string;
  state: SnapshotValue;

  constructor(name: string, initialState: SnapshotValue) {
    if (new.target === AbstractStateMachine) {
      throw new Error('AbstractStateMachine cannot be instantiated directly.');
    }

    this.name = String(name || 'state-machine');
    this.state = initialState;
  }

  /**
   * @protected
   * @abstract
   * @param {TState} currentState
   * @param {TEvent} event
   * @returns {TState | null}
   */
  resolveNextState(currentState: SnapshotValue, event: SnapshotValue): SnapshotValue | null {
    throw new Error('resolveNextState must be implemented by subclasses.');
  }

  /**
   * @param {TEvent} event
   * @returns {boolean}
   */
  transition(event: SnapshotValue): boolean {
    const nextState = this.resolveNextState(this.state, event);

    if (!nextState) {
      console.warn('[fsm] invalid transition', {
        machine: this.name,
        currentState: this.state,
        event,
      });
      return false;
    }

    this.state = nextState;
    return true;
  }
}

export class TableDrivenStateMachine extends AbstractStateMachine {
  transitionTable: Record<string, Record<string, SnapshotValue>>;

  constructor(name: string, initialState: SnapshotValue, transitionTable: Record<string, Record<string, SnapshotValue>>) {
    super(name, initialState);
    this.transitionTable = transitionTable || {};
  }

  /**
   * @protected
   * @param {TState} currentState
   * @param {TEvent} event
   * @returns {TState | null}
   */
  resolveNextState(currentState: SnapshotValue, event: SnapshotValue): SnapshotValue | null {
    const stateTransitions = this.transitionTable[String(currentState)] || null;
    if (!stateTransitions) {
      return null;
    }

    return stateTransitions[String(event)] || null;
  }
}

export type SnapshotHistoryEntry = { action: string; at: string; snapshot: SnapshotValue };

export class AbstractUndoRedoController {
  past: BoundedHistory<SnapshotHistoryEntry>;
  future: BoundedHistory<SnapshotHistoryEntry>;

  constructor(limit = 100) {
    if (new.target === AbstractUndoRedoController) {
      throw new Error('AbstractUndoRedoController cannot be instantiated directly.');
    }

    this.past = new BoundedHistory(limit);
    this.future = new BoundedHistory(limit);
  }

  /**
   * @protected
   * @abstract
   * @returns {TSnapshot}
   */
  captureSnapshot(): SnapshotValue {
    throw new Error('captureSnapshot must be implemented by subclasses.');
  }

  /**
   * @protected
   * @abstract
   * @param {TSnapshot} snapshot
   * @returns {boolean}
   */
  restoreSnapshot(snapshot: SnapshotValue): boolean {
    throw new Error('restoreSnapshot must be implemented by subclasses.');
  }

  /**
   * @param {string} action
   */
  push(action = 'edit') {
    this.past.push({
      action: String(action || 'edit'),
      at: new Date().toISOString(),
      snapshot: this.captureSnapshot(),
    });
    this.future.clear();
  }

  clear() {
    this.past.clear();
    this.future.clear();
  }

  clearRedo() {
    this.future.clear();
  }

  /**
   * @returns {SnapshotHistoryEntry<TSnapshot> | null}
   */
  undo(): SnapshotHistoryEntry | null {
    if (this.past.length === 0) {
      return null;
    }

    const currentSnapshot = this.captureSnapshot();
    this.future.push({
      action: 'redo-anchor',
      at: new Date().toISOString(),
      snapshot: currentSnapshot,
    });

    const previous = this.past.pop() as SnapshotHistoryEntry | undefined;
    if (!previous) {
      return null;
    }

    const restored = this.restoreSnapshot(previous.snapshot);
    return restored ? (previous as SnapshotHistoryEntry) : null;
  }

  /**
   * @returns {SnapshotHistoryEntry<TSnapshot> | null}
   */
  redo(): SnapshotHistoryEntry | null {
    if (this.future.length === 0) {
      return null;
    }

    const currentSnapshot = this.captureSnapshot();
    this.past.push({
      action: 'undo-anchor',
      at: new Date().toISOString(),
      snapshot: currentSnapshot,
    });

    const next = this.future.pop() as SnapshotHistoryEntry | undefined;
    if (!next) {
      return null;
    }

    const restored = this.restoreSnapshot(next.snapshot);
    return restored ? (next as SnapshotHistoryEntry) : null;
  }

  /**
   * @returns {number}
   */
  get undoDepth() {
    return this.past.length;
  }

  /**
   * @returns {number}
   */
  get redoDepth() {
    return this.future.length;
  }

  /**
   * @returns {SnapshotHistoryEntry<TSnapshot> | null}
   */
  peekUndo(): SnapshotHistoryEntry | null {
    return (this.past.peek() as SnapshotHistoryEntry) || null;
  }

  /**
   * @returns {SnapshotHistoryEntry<TSnapshot> | null}
   */
  popUndo(): SnapshotHistoryEntry | null {
    return (this.past.pop() as SnapshotHistoryEntry) || null;
  }
}

export class CallbackUndoRedoController extends AbstractUndoRedoController {
  captureSnapshotFn: () => SnapshotValue;
  restoreSnapshotFn: (snapshot: SnapshotValue) => boolean;

  constructor(options: {
    limit?: number;
    captureSnapshot: () => SnapshotValue;
    restoreSnapshot: (snapshot: SnapshotValue) => boolean;
  }) {
    super(options?.limit || 100);
    this.captureSnapshotFn = options.captureSnapshot;
    this.restoreSnapshotFn = options.restoreSnapshot;
  }

  /**
   * @protected
   * @returns {TSnapshot}
   */
  captureSnapshot(): SnapshotValue {
    return this.captureSnapshotFn();
  }

  /**
   * @protected
   * @param {TSnapshot} snapshot
   * @returns {boolean}
   */
  restoreSnapshot(snapshot: SnapshotValue): boolean {
    return this.restoreSnapshotFn(snapshot);
  }
}

export type TransitionHistoryEntry = {
  ts: string;
  machine: string;
  event: string;
  before: SnapshotValue;
  after: SnapshotValue;
};

export class TransitionHistory {
  past: BoundedHistory<TransitionHistoryEntry>;
  future: BoundedHistory<TransitionHistoryEntry>;

  constructor(limit = 32) {
    this.past = new BoundedHistory(limit);
    this.future = new BoundedHistory(limit);
  }

  /**
   * @param {{ machine: string, event: string, before: TSnapshot, after: TSnapshot }} entry
   */
  record(entry: { machine: string; event: string; before: SnapshotValue; after: SnapshotValue }): void {
    this.past.push({
      ts: new Date().toISOString(),
      machine: String(entry.machine || ''),
      event: String(entry.event || ''),
      before: entry.before,
      after: entry.after,
    });
    this.future.clear();
  }

  /**
   * @param {(snapshot: TSnapshot) => boolean} restore
   * @returns {boolean}
   */
  undo(restore: (snapshot: SnapshotValue) => boolean): boolean {
    const lastEntry = this.past.pop() as TransitionHistoryEntry | undefined;
    if (!lastEntry || !lastEntry.before) {
      return false;
    }

    this.future.push(lastEntry);
    return Boolean(restore(lastEntry.before));
  }

  /**
   * @param {(snapshot: TSnapshot) => boolean} restore
   * @returns {boolean}
   */
  redo(restore: (snapshot: SnapshotValue) => boolean): boolean {
    const entry = this.future.pop() as TransitionHistoryEntry | undefined;
    if (!entry || !entry.after) {
      return false;
    }

    this.past.push(entry);
    return Boolean(restore(entry.after));
  }

  /**
   * @returns {number}
   */
  get length() {
    return this.past.length;
  }

  /**
   * @returns {TransitionHistoryEntry<TSnapshot> | null}
   */
  get lastTransition(): TransitionHistoryEntry | null {
    return (this.past.peek() as TransitionHistoryEntry) || null;
  }
}
