"use strict";
/**
 * Reusable state/history utilities for the DOC webview.
 * These are intentionally UI-agnostic so behavior can be tested in isolation.
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.TransitionHistory = exports.CallbackUndoRedoController = exports.AbstractUndoRedoController = exports.TableDrivenStateMachine = exports.AbstractStateMachine = exports.BoundedHistory = void 0;
/**
 * @template TItem
 */
class BoundedHistory {
    /**
     * @param {number} limit
     */
    constructor(limit = 100) {
        this.limit = Number.isFinite(limit) && limit > 0 ? Math.floor(limit) : 100;
        /** @type {TItem[]} */
        this.items = [];
    }
    /**
     * @param {TItem} item
     */
    push(item) {
        this.items.push(item);
        if (this.items.length > this.limit) {
            this.items = this.items.slice(-this.limit);
        }
    }
    /**
     * @returns {TItem | undefined}
     */
    pop() {
        return this.items.pop();
    }
    /**
     * @returns {TItem | undefined}
     */
    peek() {
        return this.items.length > 0 ? this.items[this.items.length - 1] : undefined;
    }
    clear() {
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
    toArray() {
        return [...this.items];
    }
}
exports.BoundedHistory = BoundedHistory;
/**
 * @template TState
 * @template TEvent
 * @abstract
 */
class AbstractStateMachine {
    /**
     * @param {string} name
     * @param {TState} initialState
     */
    constructor(name, initialState) {
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
    resolveNextState(currentState, event) {
        throw new Error('resolveNextState must be implemented by subclasses.');
    }
    /**
     * @param {TEvent} event
     * @returns {boolean}
     */
    transition(event) {
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
exports.AbstractStateMachine = AbstractStateMachine;
/**
 * @template TState
 * @template TEvent
 * @extends {AbstractStateMachine<TState, TEvent>}
 */
class TableDrivenStateMachine extends AbstractStateMachine {
    /**
     * @param {string} name
     * @param {TState} initialState
     * @param {Record<string, Record<string, TState>>} transitionTable
     */
    constructor(name, initialState, transitionTable) {
        super(name, initialState);
        this.transitionTable = transitionTable || {};
    }
    /**
     * @protected
     * @param {TState} currentState
     * @param {TEvent} event
     * @returns {TState | null}
     */
    resolveNextState(currentState, event) {
        const stateTransitions = this.transitionTable[String(currentState)] || null;
        if (!stateTransitions) {
            return null;
        }
        return stateTransitions[String(event)] || null;
    }
}
exports.TableDrivenStateMachine = TableDrivenStateMachine;
/**
 * @template TSnapshot
 * @typedef {{ action: string, at: string, snapshot: TSnapshot }} SnapshotHistoryEntry
 */
/**
 * @template TSnapshot
 * @abstract
 */
class AbstractUndoRedoController {
    /**
     * @param {number} limit
     */
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
    captureSnapshot() {
        throw new Error('captureSnapshot must be implemented by subclasses.');
    }
    /**
     * @protected
     * @abstract
     * @param {TSnapshot} snapshot
     * @returns {boolean}
     */
    restoreSnapshot(snapshot) {
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
    undo() {
        if (this.past.length === 0) {
            return null;
        }
        const currentSnapshot = this.captureSnapshot();
        this.future.push({
            action: 'redo-anchor',
            at: new Date().toISOString(),
            snapshot: currentSnapshot,
        });
        const previous = this.past.pop();
        if (!previous) {
            return null;
        }
        const restored = this.restoreSnapshot(previous.snapshot);
        return restored ? previous : null;
    }
    /**
     * @returns {SnapshotHistoryEntry<TSnapshot> | null}
     */
    redo() {
        if (this.future.length === 0) {
            return null;
        }
        const currentSnapshot = this.captureSnapshot();
        this.past.push({
            action: 'undo-anchor',
            at: new Date().toISOString(),
            snapshot: currentSnapshot,
        });
        const next = this.future.pop();
        if (!next) {
            return null;
        }
        const restored = this.restoreSnapshot(next.snapshot);
        return restored ? next : null;
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
    peekUndo() {
        return this.past.peek() || null;
    }
    /**
     * @returns {SnapshotHistoryEntry<TSnapshot> | null}
     */
    popUndo() {
        return this.past.pop() || null;
    }
}
exports.AbstractUndoRedoController = AbstractUndoRedoController;
/**
 * @template TSnapshot
 * @extends {AbstractUndoRedoController<TSnapshot>}
 */
class CallbackUndoRedoController extends AbstractUndoRedoController {
    /**
     * @param {{
     *   limit?: number,
     *   captureSnapshot: () => TSnapshot,
     *   restoreSnapshot: (snapshot: TSnapshot) => boolean,
     * }} options
     */
    constructor(options) {
        super(options?.limit || 100);
        this.captureSnapshotFn = options.captureSnapshot;
        this.restoreSnapshotFn = options.restoreSnapshot;
    }
    /**
     * @protected
     * @returns {TSnapshot}
     */
    captureSnapshot() {
        return this.captureSnapshotFn();
    }
    /**
     * @protected
     * @param {TSnapshot} snapshot
     * @returns {boolean}
     */
    restoreSnapshot(snapshot) {
        return this.restoreSnapshotFn(snapshot);
    }
}
exports.CallbackUndoRedoController = CallbackUndoRedoController;
/**
 * @template TSnapshot
 * @typedef {{
 *   ts: string,
 *   machine: string,
 *   event: string,
 *   before: TSnapshot,
 *   after: TSnapshot,
 * }} TransitionHistoryEntry
 */
/**
 * @template TSnapshot
 */
class TransitionHistory {
    /**
     * @param {number} limit
     */
    constructor(limit = 32) {
        this.past = new BoundedHistory(limit);
        this.future = new BoundedHistory(limit);
    }
    /**
     * @param {{ machine: string, event: string, before: TSnapshot, after: TSnapshot }} entry
     */
    record(entry) {
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
    undo(restore) {
        const lastEntry = this.past.pop();
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
    redo(restore) {
        const entry = this.future.pop();
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
    get lastTransition() {
        return this.past.peek() || null;
    }
}
exports.TransitionHistory = TransitionHistory;
