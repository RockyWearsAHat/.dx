import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

const repoRoot = process.cwd();
const runtimeDir = path.join(repoRoot, 'build', 'runtime');
const runtimePackageJson = path.join(runtimeDir, 'package.json');

await mkdir(runtimeDir, { recursive: true });
await writeFile(runtimePackageJson, '{\n  "type": "module"\n}\n', 'utf8');
