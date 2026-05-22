import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync, isNativeSQLiteBridge } from '../src/sqlite-native.js';

test('sqlite native bridge is active and usable', () => {
  assert.equal(isNativeSQLiteBridge, true, 'Expected native C++ SQLite bridge to be active for backend DB calls.');

  const db = new DatabaseSync(':memory:');
  db.exec('CREATE TABLE t(id INTEGER PRIMARY KEY, value TEXT NOT NULL);');
  db.prepare('INSERT INTO t(value) VALUES (?)').run('ok');
  const row = db.prepare('SELECT value FROM t WHERE id = 1').get();

  assert.equal(row.value, 'ok');
  db.close();
});
