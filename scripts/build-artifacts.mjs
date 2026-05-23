import { mkdir, readdir, unlink, readFile, writeFile, cp, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import * as esbuild from 'esbuild';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');
const buildDir = path.join(repoRoot, 'build');
const extensionSourceDir = path.join(repoRoot, 'vscode-extension');
const bundleOutput = path.join(buildDir, 'docdb-webview.bundle.min.js');
const stagingExtensionDir = path.join(buildDir, '.vsix-staging', 'vscode-extension');

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

async function clearPreviousArtifacts() {
  await mkdir(buildDir, { recursive: true });

  const entries = await readdir(buildDir, { withFileTypes: true });
  const artifactNames = entries
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .filter((name) => name.endsWith('.vsix') || name.endsWith('.bundle.min.js'));

  if (artifactNames.length === 0) {
    return 0;
  }

  for (const name of artifactNames) {
    await unlink(path.join(buildDir, name));
  }

  return artifactNames.length;
}

async function readExtensionVersion() {
  const packageJsonPath = path.join(extensionSourceDir, 'package.json');
  const raw = await readFile(packageJsonPath, 'utf8');
  const parsed = JSON.parse(raw);
  return String(parsed.version || '0.0.0');
}

async function stageExtensionForPackaging() {
  await rm(path.join(buildDir, '.vsix-staging'), { recursive: true, force: true });
  await mkdir(path.dirname(stagingExtensionDir), { recursive: true });

  await cp(extensionSourceDir, stagingExtensionDir, { recursive: true, force: true });
  await cp(path.join(buildDir, 'runtime'), path.join(stagingExtensionDir, 'build', 'runtime'), { recursive: true, force: true });
  await cp(path.join(buildDir, 'Release'), path.join(stagingExtensionDir, 'build', 'Release'), { recursive: true, force: true });

  const stagedPackageJsonPath = path.join(stagingExtensionDir, 'package.json');
  const raw = await readFile(stagedPackageJsonPath, 'utf8');
  const parsed = JSON.parse(raw);
  parsed.main = 'build/runtime/vscode-extension/extension.cjs';
  await writeFile(stagedPackageJsonPath, `${JSON.stringify(parsed, null, 2)}\n`, 'utf8');

  return stagingExtensionDir;
}

async function emitMinifiedBundle() {
  await esbuild.build({
    entryPoints: [path.join(repoRoot, 'build', 'runtime', 'vscode-extension', 'media', 'webview-main.js')],
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

async function emitVsix(vsixOutputPath, packageCwd) {
  await run('npx', ['@vscode/vsce', 'package', '--out', vsixOutputPath], {
    cwd: packageCwd,
  });
}

async function main() {
  console.log('[build:artifacts] Starting artifact build...');
  const removedCount = await clearPreviousArtifacts();
  console.log(`[build:artifacts] Removed ${removedCount} previous bundle/vsix artifact(s).`);

  await run('npm', ['run', 'build:native']);

  await runQuiet('npm', ['run', 'build:ts']);

  await emitMinifiedBundle();

  const packageCwd = await stageExtensionForPackaging();
  const extensionVersion = await readExtensionVersion();
  const vsixOutputPath = path.join(buildDir, `docdb-virtual-files-${extensionVersion}.vsix`);
  try {
    await emitVsix(vsixOutputPath, packageCwd);
  } finally {
    await rm(path.join(buildDir, '.vsix-staging'), { recursive: true, force: true });
  }

  console.log(`[build:artifacts] Bundle: ${bundleOutput}`);
  console.log(`[build:artifacts] VSIX:   ${vsixOutputPath}`);
  console.log('[build:artifacts] Done.');
}

main().catch((error) => {
  console.error(`[build:artifacts] Failed: ${error.message}`);
  process.exitCode = 1;
});
