import { readdir } from 'node:fs/promises';
import path from 'node:path';

import { listGitEligibleDxFiles } from './git-doc-state.js';

const IGNORED_DIRECTORIES = new Set([
  '.git',
  '.github',
  '.vscode',
  'data',
  'node_modules',
]);

export async function findDocFiles(rootDir: string): Promise<string[]> {
  const results: string[] = [];
  const gitEligible = listGitEligibleDxFiles(rootDir);

  async function walk(currentDir: string): Promise<void> {
    const entries = await readdir(currentDir, { withFileTypes: true });

    for (const entry of entries) {
      if (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name)) {
        continue;
      }

      const absolutePath = path.join(currentDir, entry.name);

      if (entry.isDirectory()) {
        await walk(absolutePath);
        continue;
      }

      if (entry.isFile() && entry.name.endsWith('.dx')) {
        if (gitEligible && !gitEligible.has(absolutePath)) {
          continue;
        }
        results.push(absolutePath);
      }
    }
  }

  await walk(rootDir);
  return results.sort();
}