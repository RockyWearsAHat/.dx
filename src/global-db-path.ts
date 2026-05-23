import os from 'node:os';
import path from 'node:path';

export function resolveDocDbPath() {
  const configured = String(process.env.DOCDB_PATH || '').trim();

  if (configured) {
    return path.resolve(configured);
  }

  return path.join(os.homedir(), '.docdb', 'doc-index.sqlite');
}

export function resolveDocDbDir() {
  return path.dirname(resolveDocDbPath());
}