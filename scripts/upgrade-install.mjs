import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const rootDir = path.resolve(path.dirname(__filename), '..');
const extensionDir = path.join(rootDir, 'vscode-extension');

function parseArgs() {
  const result = {
    level: 'patch',
    skipSmoke: false,
  };

  for (const arg of process.argv.slice(2)) {
    if (arg === '--skip-smoke') {
      result.skipSmoke = true;
      continue;
    }

    if (arg.startsWith('--level=')) {
      const value = arg.slice('--level='.length).trim();

      if (value === 'patch' || value === 'minor' || value === 'major') {
        result.level = value;
      }
    }
  }

  return result;
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd || rootDir,
      stdio: options.stdio || 'inherit',
      shell: false,
      env: process.env,
    });

    child.on('error', reject);
    child.on('close', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`${command} ${args.join(' ')} exited with ${code}`));
      }
    });
  });
}

function runCapture(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd || rootDir,
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: false,
      env: process.env,
    });

    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk.toString();
    });

    child.on('error', reject);
    child.on('close', (code) => {
      if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(new Error(`${command} ${args.join(' ')} exited with ${code}\n${stderr}`));
      }
    });
  });
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function runSmokeTest() {
  // Verify bundle + dxlite engine is reachable after install.
  const { execFileSync } = await import('node:child_process');

  try {
    console.log('📦 Verifying bundle engine access...');
    execFileSync(process.execPath, ['--input-type=module', '--eval', "import { listOrSearchDocuments } from './build/runtime/src/doc-service.js'; const docs = await listOrSearchDocuments(process.cwd(), null, ''); console.log('✓ Bundle engine reachable; docs=' + docs.length);"], {
      cwd: rootDir,
      stdio: 'inherit',
    });

    console.log('✅ Smoke test passed: bundle engine working');
    return true;
  } catch (err) {
    throw new Error(`Smoke test failed: ${err.message}`);
  }
}

async function main() {
  const args = parseArgs();

  console.log(`[upgrade-install] bumping extension version: ${args.level}`);
  await run('npm', ['version', args.level, '--no-git-tag-version'], { cwd: extensionDir });

  console.log('[upgrade-install] installing root dependencies');
  await run('npm', ['install'], { cwd: rootDir });

  console.log('[upgrade-install] installing vscode-extension dependencies');
  await run('npm', ['install'], { cwd: extensionDir });

  console.log('[upgrade-install] reindexing documents');
  await run('npm', ['run', 'ingest'], { cwd: rootDir });

  if (!args.skipSmoke) {
    console.log('[upgrade-install] running smoke test');
    await runSmokeTest();
  }

  const versionInfo = await runCapture('node', ['-e', "const fs=require('fs');const pkg=JSON.parse(fs.readFileSync('vscode-extension/package.json','utf8'));process.stdout.write(pkg.version);"]);
  const version = versionInfo.stdout.trim();

  // build:artifacts compiles TypeScript, emits the minified webview bundle,
  // stages the extension (rewriting package.json main to the in-VSIX path),
  // and packages the VSIX — all from the correct locations.
  console.log('[upgrade-install] building and packaging VSIX via build:artifacts');
  await run('npm', ['run', 'build:artifacts']);

  const vsixPath = path.join(rootDir, 'build', `docdb-virtual-files-${version}.vsix`);
  console.log('[upgrade-install] installing extension');
  const codeCmd = process.platform === 'darwin' ? 'code-insiders' : 'code';
  await run(codeCmd, ['--install-extension', vsixPath, '--force']);

  console.log(`[upgrade-install] done, vscode-extension version=${version}`);
}

main().catch((error) => {
  console.error(`[upgrade-install] ${error.message}`);
  process.exit(1);
});
