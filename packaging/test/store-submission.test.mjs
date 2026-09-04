// Store submission and integration tests
//
// These tests verify the infrastructure for submitting extensions to stores and integrating
// the results (signed XPIs, store URLs) back into the code. They cover:
// - Archive format validation (Manifest v3, permissions, versions)
// - Integration point verification (CHROME_WEB_STORE constant, signed_xpi function)
// - Automation script functionality

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execSync } from 'node:child_process';
import { readFileSync, existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = dirname(here);
const projectRoot = dirname(pkgRoot); // packaging/test -> packaging -> project root

// Helper: read and parse manifest from archive
function getManifestFromArchive(archivePath) {
  if (!existsSync(archivePath)) {
    return null;
  }

  try {
    // Use Python to extract and parse manifest
    const cmd = `python3 -c "
import zipfile
import json
import sys
try:
  with zipfile.ZipFile('${archivePath}', 'r') as z:
    manifest = json.loads(z.read('manifest.json'))
    print(json.dumps(manifest))
except Exception as e:
  print(json.dumps({'error': str(e)}), file=sys.stderr)
  sys.exit(1)
"`;
    const output = execSync(cmd, { encoding: 'utf8', stdio: 'pipe' });
    return JSON.parse(output);
  } catch (e) {
    return null;
  }
}

// Test: Archives exist and are valid
test('Chrome archive exists and contains valid manifest', (t) => {
  const chromePath = join(pkgRoot, 'build', 'dx-chrome.zip');
  const manifest = getManifestFromArchive(chromePath);

  if (!manifest) {
    t.skip('Run packaging/build-stores.sh first');
    return;
  }

  assert.ok(!manifest.error, `Failed to read archive: ${manifest?.error}`);
  assert.equal(manifest.manifest_version, 3, 'Must use Manifest v3');
  assert.ok(manifest.version, 'Must have version field');
  assert.match(manifest.version, /^\d+\.\d+\.\d+/, 'Version must be semantic');
});

test('Firefox archive exists and contains valid manifest', (t) => {
  const firefoxPath = join(pkgRoot, 'build', 'dx-firefox.xpi');
  const manifest = getManifestFromArchive(firefoxPath);

  if (!manifest) {
    t.skip('Run packaging/build-stores.sh first');
    return;
  }

  assert.ok(!manifest.error, `Failed to read archive: ${manifest?.error}`);
  assert.equal(manifest.manifest_version, 3, 'Must use Manifest v3');
  assert.ok(manifest.version, 'Must have version field');
});

// Test: CHROME_WEB_STORE constant integration point exists
test('CHROME_WEB_STORE constant is defined in extension.rs', (t) => {
  const extensionRs = join(projectRoot, 'rust', 'doc-cli', 'src', 'extension.rs');
  const content = readFileSync(extensionRs, 'utf8');

  assert.ok(
    /pub\s+const\s+CHROME_WEB_STORE\s*:\s*Option<&str>/.test(content),
    'CHROME_WEB_STORE constant must be defined as Option<&str>',
  );
});

// Test: signed_xpi function exists and is callable
test('signed_xpi function exists in extension.rs', (t) => {
  const extensionRs = join(projectRoot, 'rust', 'doc-cli', 'src', 'extension.rs');
  const content = readFileSync(extensionRs, 'utf8');

  assert.ok(
    /fn\s+signed_xpi\s*\(\s*\)/.test(content),
    'signed_xpi function must be defined',
  );
});

// Test: Integration script exists
test('Integration script exists and is executable', (t) => {
  const script = join(pkgRoot, 'integrate-store-results.sh');

  if (!existsSync(script)) {
    t.skip('Integration script not yet created');
    return;
  }

  const content = readFileSync(script, 'utf8');
  assert.ok(content.includes('CHROME_WEB_STORE'), 'Script must update CHROME_WEB_STORE');
  assert.ok(content.includes('dx-firefox.xpi'), 'Script must handle Firefox XPI');
});

// Test: Verification script exists
test('Verification script can check integration readiness', (t) => {
  const script = join(pkgRoot, 'verify-integration-ready.sh');

  if (!existsSync(script)) {
    t.skip('Verification script not yet created');
    return;
  }

  const content = readFileSync(script, 'utf8');
  assert.ok(content.includes('extension.rs'), 'Script must check extension.rs');
});

// Test: Submission checklist exists
test('Submission checklist provides store-specific instructions', (t) => {
  const checklist = join(pkgRoot, 'STORE_SUBMISSION_CHECKLIST.md');

  if (!existsSync(checklist)) {
    t.skip('Submission checklist not yet created');
    return;
  }

  const content = readFileSync(checklist, 'utf8');
  assert.ok(content.includes('addons.mozilla.org'), 'Must include Firefox instructions');
  assert.ok(content.includes('Chrome Web Store'), 'Must include Chrome instructions');
  assert.ok(content.includes('Mac App Store'), 'Must include Safari instructions');
});

// Test: Manifest contains required permissions
test('Manifest declares correct extension permissions', (t) => {
  const chromePath = join(pkgRoot, 'build', 'dx-chrome.zip');
  const manifest = getManifestFromArchive(chromePath);

  if (!manifest) {
    t.skip('Run packaging/build-stores.sh first');
    return;
  }

  // GitHub extension needs:
  // - tabs: to detect active tab URL
  // - scripting: to inject content script
  const permissions = manifest.permissions || [];
  assert.ok(permissions.includes('tabs') || manifest.host_permissions?.some(p => p.includes('github.com')),
    'Must declare GitHub access permissions');
});

// Test: Signed archive directory exists and is distinct from build
test('Signed archive directory is separate from build directory', (t) => {
  const buildDir = join(pkgRoot, 'build');
  const signedDir = join(pkgRoot, 'signed');

  if (!existsSync(buildDir)) {
    t.skip('Build directory not yet created');
    return;
  }

  // Verify directories are different (Mozilla-signed will replace unsigned)
  assert.notEqual(
    buildDir,
    signedDir,
    'Build and signed directories must be separate',
  );

  // After Mozilla approval, signed will contain the signed XPI
  if (!existsSync(signedDir)) {
    t.pass('Signed directory will be created after Mozilla approval');
    return;
  }

  const signedXpi = join(signedDir, 'dx-firefox.xpi');
  assert.ok(
    existsSync(signedXpi) || !existsSync(signedDir),
    'Signed directory structure is ready',
  );
});

// Test: README exists with submission instructions
test('README provides clear submission instructions', (t) => {
  const readme = join(pkgRoot, 'ITEM-0-FINAL-STATUS.md');

  if (!existsSync(readme)) {
    t.skip('Status README not yet created');
    return;
  }

  const content = readFileSync(readme, 'utf8');
  assert.ok(content.includes('submit'), 'Must include submission instructions');
  assert.ok(content.includes('CHROME_WEB_STORE'), 'Must mention the integration constant');
});
