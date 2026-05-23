import { mkdir, readdir, rename, copyFile, unlink, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import * as esbuild from 'esbuild';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');
const buildDir = path.join(repoRoot, 'build');
const oldBuildsDir = path.join(buildDir, 'old_builds');
const extensionDir = path.join(repoRoot, 'vscode-extension');
const bundleOutput = path.join(buildDir, 'docdb-webview.bundle.min.js');

function timestampLabel() {
  const now = new Date();
  const parts = [
    now.getUTCFullYear(),
    String(now.getUTCMonth() + 1).padStart(2, '0'),
    String(now.getUTCDate()).padStart(2, '0'),
    '-',
    String(now.getUTCHours()).padStart(2, '0'),
    String(now.getUTCMinutes()).padStart(2, '0'),
    String(now.getUTCSeconds()).padStart(2, '0'),
  ];
  return parts.join('');
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      stdio: 'inherit',
      shell: process.platform === 'win32',
      ...options,
    });

    child.on('exit', (code) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(new Error(`${command} ${args.join(' ')} failed with exit code ${code}`));
    });
  });
}

function runQuiet(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: process.platform === 'win32',
      ...options,
    });

    let stdout = '';
    let stderr = '';

    child.stdout.on('data', (chunk) => {
      stdout += String(chunk);
    });

    child.stderr.on('data', (chunk) => {
      stderr += String(chunk);
    });

    child.on('exit', (code) => {
      if (code === 0) {
        resolve();
        return;
      }

      const details = stderr || stdout;
      reject(new Error(`${command} ${args.join(' ')} failed with exit code ${code}${details ? `\n${details.slice(0, 1200)}` : ''}`));
    });
  });
}

async function moveFileSafe(source, target) {
  try {
    await rename(source, target);
  } catch (error) {
    if (error && error.code === 'EXDEV') {
      await copyFile(source, target);
      await unlink(source);
      return;
    }

    throw error;
  }
}

async function archiveOldArtifacts() {
  await mkdir(buildDir, { recursive: true });
  await mkdir(oldBuildsDir, { recursive: true });

  const entries = await readdir(buildDir, { withFileTypes: true });
  const artifactNames = entries
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .filter((name) => name.endsWith('.vsix') || name.endsWith('.bundle.min.js'));

  if (artifactNames.length === 0) {
    return null;
  }

  const archiveDir = path.join(oldBuildsDir, timestampLabel());
  await mkdir(archiveDir, { recursive: true });

  for (const name of artifactNames) {
    const source = path.join(buildDir, name);
    const target = path.join(archiveDir, name);
    await moveFileSafe(source, target);
  }

  return archiveDir;
}

async function readExtensionVersion() {
  const packageJsonPath = path.join(extensionDir, 'package.json');
  const raw = await readFile(packageJsonPath, 'utf8');
  const parsed = JSON.parse(raw);
  return String(parsed.version || '0.0.0');
}

async function emitMinifiedBundle() {
  await esbuild.build({
    entryPoints: [path.join(repoRoot, 'vscode-extension', 'media', 'webview-main.js')],
    outfile: bundleOutput,
    bundle: true,
    minify: true,
    platform: 'browser',
    format: 'iife',
    target: 'es2020',
    legalComments: 'none',
    logLevel: 'info',
  });
}

async function emitVsix(vsixOutputPath) {
  await run('npx', ['@vscode/vsce', 'package', '--out', vsixOutputPath], {
    cwd: extensionDir,
  });
}

async function main() {
  console.log('[build:artifacts] Starting artifact build...');
  const archiveDir = await archiveOldArtifacts();

  if (archiveDir) {
    console.log(`[build:artifacts] Archived previous artifacts to ${archiveDir}`);
  } else {
    console.log('[build:artifacts] No existing bundle/vsix artifacts to archive.');
  }

  try {
    await runQuiet('npm', ['run', 'build:ts']);
  } catch (error) {
    console.warn(`[build:artifacts] TypeScript build reported errors, continuing artifact packaging: ${error.message}`);
  }

  await emitMinifiedBundle();

  const extensionVersion = await readExtensionVersion();
  const vsixOutputPath = path.join(buildDir, `docdb-virtual-files-${extensionVersion}.vsix`);
  await emitVsix(vsixOutputPath);

  console.log(`[build:artifacts] Bundle: ${bundleOutput}`);
  console.log(`[build:artifacts] VSIX:   ${vsixOutputPath}`);
  console.log('[build:artifacts] Done.');
}

main().catch((error) => {
  console.error(`[build:artifacts] Failed: ${error.message}`);
  process.exitCode = 1;
});
