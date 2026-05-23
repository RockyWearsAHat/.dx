#!/usr/bin/env node

// Verifies the DOC save-state FSM table used by the webview.
// Keep this in sync with vscode-extension/media/webview-fsm.mjs.

import { DOC_SAVE_STATES, DOC_SAVE_TRANSITIONS } from '../vscode-extension/media/webview-fsm.mjs';

function verifySaveFsm(states, transitions) {
  const errors = [];
  const stateValues = new Set(Object.values(states));

  for (const state of stateValues) {
    if (!Object.prototype.hasOwnProperty.call(transitions, state)) {
      errors.push(`Missing transition map for state '${state}'.`);
      continue;
    }

    const events = transitions[state];
    if (!events || typeof events !== 'object') {
      errors.push(`Transition map for state '${state}' must be an object.`);
      continue;
    }

    for (const [event, target] of Object.entries(events)) {
      if (!event || typeof event !== 'string') {
        errors.push(`State '${state}' has an invalid event key.`);
      }
      if (!stateValues.has(target)) {
        errors.push(`State '${state}' event '${event}' targets unknown state '${String(target)}'.`);
      }
    }
  }

  const required = [
    [DOC_SAVE_STATES.IDLE, 'MARK_DIRTY', DOC_SAVE_STATES.DIRTY],
    [DOC_SAVE_STATES.DIRTY, 'START_SAVE', DOC_SAVE_STATES.SAVING],
    [DOC_SAVE_STATES.SAVING, 'SAVE_COMPLETE', DOC_SAVE_STATES.SAVED],
    [DOC_SAVE_STATES.SAVING, 'SAVE_FAILED', DOC_SAVE_STATES.ERROR],
    [DOC_SAVE_STATES.SAVED, 'CLEAR_SAVED', DOC_SAVE_STATES.IDLE],
    [DOC_SAVE_STATES.ERROR, 'SYNC_CLEAN', DOC_SAVE_STATES.IDLE],
  ];

  for (const [state, event, expectedTarget] of required) {
    const actualTarget = transitions[state] && transitions[state][event];
    if (actualTarget !== expectedTarget) {
      errors.push(
        `Required transition mismatch: ${state} --${event}--> ${String(actualTarget)} (expected ${expectedTarget}).`,
      );
    }
  }

  return errors;
}

const errors = verifySaveFsm(DOC_SAVE_STATES, DOC_SAVE_TRANSITIONS);

if (errors.length > 0) {
  console.error('DOC save FSM verification failed:');
  for (const entry of errors) {
    console.error(`- ${entry}`);
  }
  process.exit(1);
}

console.log('DOC save FSM verification passed.');
