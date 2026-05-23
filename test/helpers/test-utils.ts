import os from 'node:os';
import path from 'node:path';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';

export async function createTempWorkspace(prefix = 'doc-tests-') {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), prefix));
  const dbPath = path.join(rootDir, '.tmp', 'doc-index.sqlite');
  await mkdir(path.dirname(dbPath), { recursive: true });
  process.env.DOCDB_PATH = dbPath;
  return { rootDir, dbPath };
}

export async function cleanupTempWorkspace(rootDir) {
  if (!rootDir) return;
  await rm(rootDir, { recursive: true, force: true });
}

export async function writeDxFile(rootDir, relativePath, source) {
  const absolute = path.join(rootDir, relativePath);
  await mkdir(path.dirname(absolute), { recursive: true });
  await writeFile(absolute, source, 'utf8');
  return absolute;
}
