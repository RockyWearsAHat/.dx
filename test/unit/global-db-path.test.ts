import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import os from 'node:os';

import { resolveDocDbDir, resolveDocDbPath } from '#runtime-src/global-db-path.js';

test('resolveDocDbPath uses configured DOCDB_PATH when set', () => {
  const prev = process.env.DOCDB_PATH;
  process.env.DOCDB_PATH = './tmp/custom.sqlite';
  try {
    const resolved = resolveDocDbPath();
    assert.equal(resolved, path.resolve('./tmp/custom.sqlite'));
    assert.equal(resolveDocDbDir(), path.dirname(path.resolve('./tmp/custom.sqlite')));
  } finally {
    if (prev === undefined) delete process.env.DOCDB_PATH;
    else process.env.DOCDB_PATH = prev;
  }
});

test('resolveDocDbPath falls back to homedir default when DOCDB_PATH missing/blank', () => {
  const prev = process.env.DOCDB_PATH;
  process.env.DOCDB_PATH = '   ';
  try {
    const expected = path.join(os.homedir(), '.docdb', 'doc-index.sqlite');
    assert.equal(resolveDocDbPath(), expected);
    assert.equal(resolveDocDbDir(), path.dirname(expected));
  } finally {
    if (prev === undefined) delete process.env.DOCDB_PATH;
    else process.env.DOCDB_PATH = prev;
  }
});

test('resolveDocDbPath falls back to homedir default when DOCDB_PATH is unset', () => {
  const prev = process.env.DOCDB_PATH;
  delete process.env.DOCDB_PATH;
  try {
    const expected = path.join(os.homedir(), '.docdb', 'doc-index.sqlite');
    assert.equal(resolveDocDbPath(), expected);
  } finally {
    if (prev === undefined) delete process.env.DOCDB_PATH;
    else process.env.DOCDB_PATH = prev;
  }
});
