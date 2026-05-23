import test from 'node:test';
import assert from 'node:assert/strict';
import {
  computeAutocompleteSuggestions,
  computeNextSelectedIndex,
  getLineContextFromValue,
} from '#runtime-media/webview-autocomplete-controller.js';

test('autocomplete suggests block commands by prefix', () => {
  const context = getLineContextFromValue('::he', 4);
  const model = computeAutocompleteSuggestions({
    context,
    blockAutocomplete: ['::paragraph', '::heading level=1', '::quote'],
  });

  assert.equal(model.replaceStart, 0);
  assert.equal(model.replaceEnd, 4);
  assert.equal(model.typed, '::he');
  assert.deepEqual(
    model.suggestions.map((item) => item.label),
    ['::heading level=1'],
  );
});

test('autocomplete id completion extends replace range through right-side token text', () => {
  const value = '::paragraph id=hello';
  const cursor = value.indexOf('hello') + 2;
  const context = getLineContextFromValue(value, cursor);
  const model = computeAutocompleteSuggestions({
    context,
    knownIds: ['hero', 'hex'],
  });

  assert.equal(model.typed, 'he');
  assert.equal(model.replaceStart, value.indexOf('hello'));
  assert.equal(model.replaceEnd, value.length);
  assert.deepEqual(
    model.suggestions.map((item) => item.label),
    ['hero', 'hex'],
  );
});

test('autocomplete class completion skips exact token matches', () => {
  const value = '::paragraph class=pri';
  const cursor = value.indexOf('pri') + 3;
  const context = getLineContextFromValue(value, cursor);
  const model = computeAutocompleteSuggestions({
    context,
    knownClasses: ['primary', 'print', 'pri'],
  });

  assert.equal(model.typed, 'pri');
  assert.deepEqual(
    model.suggestions.map((item) => item.label),
    ['primary', 'print'],
  );
});

test('autocomplete suggests attribute keys from observed schema', () => {
  const value = '::heading le';
  const context = getLineContextFromValue(value, value.length);
  const model = computeAutocompleteSuggestions({
    context,
    knownAttributeKeys: ['level', 'id', 'class'],
  });

  assert.equal(model.typed, 'le');
  assert.deepEqual(
    model.suggestions.map((item) => item.label),
    ['level='],
  );
});

test('autocomplete suggests attribute values from observed schema', () => {
  const value = '::heading level=';
  const context = getLineContextFromValue(value, value.length);
  const model = computeAutocompleteSuggestions({
    context,
    knownAttributeValuesByKey: {
      level: ['1', '2', '3'],
    },
  });

  assert.equal(model.typed, '');
  assert.deepEqual(
    model.suggestions.map((item) => item.label),
    ['1', '2', '3'],
  );
});

test('autocomplete selection index wraps for keyboard cycling', () => {
  assert.equal(computeNextSelectedIndex(0, 1, 3), 1);
  assert.equal(computeNextSelectedIndex(2, 1, 3), 0);
  assert.equal(computeNextSelectedIndex(0, -1, 3), 2);
});
