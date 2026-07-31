// The shipped engine, and the worker that owns it.
//
// `resolve.test.mjs` covers what the reader is shown. This file covers the thing underneath
// it: the wasm in `editor/github/wasm` — the bytes `dx` embeds and a browser loads — and
// `engine.js`, the one file that has to work as *both* a Chrome service worker and a Firefox
// event page.
//
// The failure this exists to catch is silent. A `doc-core` change without a rerun of
// `editor/build.sh` leaves a built engine one version behind: every test that exercises only
// the resolver still passes, and github.com quietly renders documents the way `dx` used to.
// So the engine renders a real document here and is held to what the `dx` binary produced for
// the same input — and it is done for **every** build, because `doc-wasm` is compiled twice
// and an unchecked build is exactly where that drift used to hide.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

import { BUILD_NAMES, loadEngine, loadShippedEngine, shippedWasmBytes } from './engine.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const shipped = await loadShippedEngine();

/// The rendering `dx` produced for the fixture document, or `null` before `fixture.sh` runs.
function expectedRendering() {
  const input = join(here, 'fixture/render-input.dx');
  const html = join(here, 'fixture/render.html');
  if (!existsSync(input) || !existsSync(html)) return null;
  return {
    source: readFileSync(input, 'utf8'),
    html: readFileSync(html, 'utf8'),
  };
}

test('the shipped engine is a wasm module of the expected size', (t) => {
  const bytes = shippedWasmBytes();
  if (!bytes) {
    t.skip('run editor/build.sh first');
    return;
  }
  assert.deepEqual([...bytes.subarray(0, 4)], [0x00, 0x61, 0x73, 0x6d], 'not a wasm module');
  assert.ok(bytes.length > 100_000, 'suspiciously small for the whole of doc-core');
});

// One assertion, made of every build there is. `doc-wasm` is compiled twice and the two
// artifacts are not interchangeable — but what they *render* has to be identical, to each
// other and to the binary, or the editor and github.com are two renderers wearing one name.
// Checking only the browser's build is how the editor's went a year without being compared.
for (const build of BUILD_NAMES) {
  test(`the ${build} engine renders what the dx binary renders`, async (t) => {
    const expected = expectedRendering();
    const engine = await loadEngine(build);
    if (!engine || !expected) {
      t.skip('run editor/build.sh and github/test/fixture.sh first');
      return;
    }

    // The same call the content script makes: a fragment, in GitHub's own palette.
    const rendered = engine.render_html(expected.source, 'light', true, false);
    assert.equal(
      rendered,
      expected.html,
      `the ${build} engine disagrees with the dx binary — rerun editor/build.sh`,
    );
  });
}

test('the shipped engine exports every call the worker allows', (t) => {
  if (!shipped) {
    t.skip('run editor/build.sh first');
    return;
  }
  const worker = readFileSync(join(here, '../engine.js'), 'utf8');
  const allowed = /const ALLOWED = new Set\(\[([^\]]*)\]\)/.exec(worker);
  assert.ok(allowed, 'engine.js must state its allowlist');
  for (const [, name] of allowed[1].matchAll(/'([^']+)'/g)) {
    assert.equal(typeof shipped[name], 'function', `the engine has no export named ${name}`);
  }
});

test('the page adapter reads the resolver from the global it is published on', () => {
  // Firefox found this one. `resolve.js` writes its API to `globalThis`; inside a content
  // script Chrome makes `window` and `globalThis` the same object, and Firefox does not —
  // there, `window` is a wrapper around the *page's* window, so `window.dxResolve` is
  // `undefined` and every github.com page renders nothing, silently, with the pointer still
  // on screen. Publisher and reader have to name the same global.
  const resolver = readFileSync(join(here, '../resolve.js'), 'utf8');
  assert.match(resolver, /globalThis\.dxResolve\s*=/, 'resolve.js must publish on globalThis');

  const adapter = readFileSync(join(here, '../content.js'), 'utf8');
  const lines = adapter.split('\n').filter((line) => !line.trim().startsWith('//'));
  assert.equal(
    lines.filter((line) => /window\.dxResolve/.test(line)).length,
    0,
    'content.js must read globalThis.dxResolve, which is where resolve.js puts it',
  );
  assert.ok(/globalThis\.dxResolve\./.test(adapter), 'content.js must use the resolver');
});

/// Load `engine.js` the way a **Firefox event page** does: no `importScripts`, with the glue
/// already evaluated so `wasm_bindgen` is simply in scope.
///
/// Chrome and Firefox differ here and nowhere else in this extension. Chrome runs the file as
/// a service worker, where `importScripts` pulls the glue in; Firefox has no MV3 service
/// worker and lists the glue ahead of it in the manifest instead. A file that assumed either
/// one would be dead in the other browser, with nothing on the page to say why.
/// `fetch` is injected rather than inherited so a machine that happens to be running
/// `dx serve` cannot change what these tests exercise. The default is a machine with no
/// daemon, which is the path the bundled wasm has to cover.
function loadWorker(engineModule, fetchImpl = () => Promise.reject(new Error('no daemon'))) {
  const wasm_bindgen = Object.assign(() => Promise.resolve(), engineModule);
  let listener = null;
  const chrome = {
    runtime: {
      getURL: (path) => `moz-extension://dx/${path}`,
      onMessage: { addListener: (fn) => (listener = fn) },
    },
  };
  const source = readFileSync(join(here, '../engine.js'), 'utf8');
  // `importScripts` is deliberately absent from this scope, which is what makes it the
  // Firefox shape: the file has to notice and carry on.
  new Function('chrome', 'wasm_bindgen', 'atob', 'fetch', source)(
    chrome,
    wasm_bindgen,
    atob,
    fetchImpl,
  );
  assert.ok(listener, 'engine.js registered no message listener');
  return listener;
}

/// A stand-in for `dx serve`: answers `/health`, `/pack`, and `/engine` the way the daemon
/// does, and records every request so a test can say how many times the pack crossed.
///
/// It is a fake rather than the real binary because these tests run without building
/// anything; `rust/doc-cli/src/daemon` holds the real protocol to its own contract, and
/// `the extension probes exactly the ports the daemon binds` keeps the two in step.
function fakeDaemon({ holds = new Map() } = {}) {
  const requests = [];
  const fetchImpl = async (url, options = {}) => {
    requests.push({ url, options });
    const reply = (status, body) => ({
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
    });
    if (url.endsWith('/health')) return reply(200, { name: 'dx', version: 'test', packs: 0 });
    if (url.endsWith('/pack')) {
      holds.set(options.headers['x-dx-pack'], options.body);
      return reply(200, { paths: ['welcome.dx'] });
    }
    if (url.endsWith('/engine')) {
      const { args } = JSON.parse(options.body);
      const named = args.find((argument) => argument && argument.pack)?.pack;
      if (named && !holds.has(named)) return reply(409, { needPack: named });
      return reply(200, { value: 'from the daemon' });
    }
    return reply(404, { error: 'no such route' });
  };
  return { fetchImpl, requests, holds };
}

/// Send one message to a worker listener and await its reply.
function ask(listener, message) {
  return new Promise((resolve, reject) => {
    const handled = listener(message, {}, resolve);
    if (!handled) reject(new Error('the worker refused the message'));
  });
}

test('the worker runs as a Firefox event page, not only a Chrome service worker', async (t) => {
  if (!shipped) {
    t.skip('run editor/build.sh first');
    return;
  }
  const listener = loadWorker(shipped);

  const source = '::heading level=1 id=hi\nHi\n::end\n';
  const reply = await ask(listener, {
    kind: 'dx-engine',
    call: 'render_html',
    args: [source, 'light', true, false],
  });
  assert.equal(reply.error, undefined, reply.error);
  assert.equal(reply.value, shipped.render_html(source, 'light', true, false));
});

test('a call the worker does not allow is refused by name', async (t) => {
  if (!shipped) {
    t.skip('run editor/build.sh first');
    return;
  }
  const listener = loadWorker(shipped);
  // A message from a page must never be able to name an arbitrary export.
  const reply = await ask(listener, { kind: 'dx-engine', call: 'parse', args: [''] });
  assert.match(reply.error, /unknown engine call: parse/);
});

/// The fixture pack `fixture.sh` wrote, or `null` before it has been run.
function fixturePack() {
  const path = join(here, 'fixture/repo.dxcp');
  return existsSync(path) ? readFileSync(path) : null;
}

/// The call a blob page makes, naming a pack rather than carrying it.
const PACK_URL = 'https://github.com/a/b/raw/main/.doc/repo.dxcp';
const READ_WELCOME = {
  kind: 'dx-engine',
  call: 'pack_document',
  args: [{ packRef: PACK_URL }, 'welcome.dx'],
};

test('a pack the engine does not hold is asked for, not failed over', async (t) => {
  const pack = fixturePack();
  if (!shipped || !pack) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }
  const listener = loadWorker(shipped);

  // This is the difference that made diff pages crawl: the pack crosses once, keyed by the
  // URL it came from, instead of being base64'd into every call.
  const asked = await ask(listener, READ_WELCOME);
  assert.equal(asked.needPack, PACK_URL, 'the engine must say which pack it lacks');
  assert.equal(asked.value, undefined, 'asking for a pack is not an answer');

  const stored = await ask(listener, {
    kind: 'dx-pack',
    url: PACK_URL,
    bytes: pack.toString('base64'),
  });
  assert.equal(stored.stored, true);

  const answered = await ask(listener, READ_WELCOME);
  assert.equal(answered.error, undefined, answered.error);
  assert.match(answered.value, /^::heading level=1 id=/);

  // Every later call is answered from what the worker kept — no second upload.
  const again = await ask(listener, READ_WELCOME);
  assert.match(again.value, /^::heading level=1 id=/);
});

test('the daemon answers when it is running, and the pack crosses to it once', async (t) => {
  const pack = fixturePack();
  if (!shipped || !pack) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }
  const daemon = fakeDaemon();
  const listener = loadWorker(shipped, daemon.fetchImpl);

  assert.equal((await ask(listener, READ_WELCOME)).needPack, PACK_URL);
  await ask(listener, { kind: 'dx-pack', url: PACK_URL, bytes: pack.toString('base64') });
  for (let call = 0; call < 5; call += 1) {
    assert.equal((await ask(listener, READ_WELCOME)).value, 'from the daemon');
  }

  const uploads = daemon.requests.filter((request) => request.url.endsWith('/pack'));
  assert.equal(uploads.length, 1, 'the pack must cross to the daemon once, not once per call');
  const probes = daemon.requests.filter((request) => request.url.endsWith('/health'));
  assert.equal(probes.length, 1, 'the daemon is found once per worker, not once per call');
});

test('concurrent calls share one search for the daemon', async (t) => {
  if (!shipped) {
    t.skip('run editor/build.sh first');
    return;
  }
  // Issuing these sequentially hides the bug this exists to catch: the cached answer is read
  // synchronously, before the first `await`, so everything that starts while the first probe
  // is in flight probes again. Rendering one document does exactly this — `content.js` awaits
  // `stylesheet()` and `render_html()` together.
  const daemon = fakeDaemon();
  const listener = loadWorker(shipped, daemon.fetchImpl);
  await Promise.all([
    ask(listener, { kind: 'dx-engine', call: 'stylesheet', args: [] }),
    ask(listener, { kind: 'dx-engine', call: 'render_html', args: ['x', 'auto', true, false] }),
    ask(listener, { kind: 'dx-engine', call: 'stylesheet', args: [] }),
  ]);

  const probes = daemon.requests.filter((request) => request.url.endsWith('/health'));
  assert.equal(probes.length, 1, `three concurrent calls made ${probes.length} probes`);
});

test('the fallback does not accumulate every pack of a browsing session', async (t) => {
  const pack = fixturePack();
  if (!shipped || !pack) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }
  const listener = loadWorker(shipped);
  const url = (n) => `https://github.com/a/b${n}/raw/main/.doc/repo.dxcp`;

  for (let repo = 0; repo < 20; repo += 1) {
    await ask(listener, { kind: 'dx-pack', url: url(repo), bytes: pack.toString('base64') });
  }

  // The most recent is still answered from memory; the oldest is asked for again.
  const recent = { kind: 'dx-engine', call: 'pack_document', args: [{ packRef: url(19) }, 'welcome.dx'] };
  assert.match((await ask(listener, recent)).value, /^::heading level=1 id=/);
  const oldest = { kind: 'dx-engine', call: 'pack_document', args: [{ packRef: url(0) }, 'welcome.dx'] };
  assert.equal((await ask(listener, oldest)).needPack, url(0), 'nothing was ever dropped');
});

test('a daemon that stops mid-read falls back to the wasm rather than failing', async (t) => {
  const pack = fixturePack();
  if (!shipped || !pack) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }
  let alive = true;
  const daemon = fakeDaemon();
  const listener = loadWorker(shipped, (url, options) => {
    if (!alive) return Promise.reject(new Error('connection refused'));
    return daemon.fetchImpl(url, options);
  });

  // Hand the pack over while the daemon is up; the worker keeps a copy for exactly this.
  assert.equal((await ask(listener, READ_WELCOME)).needPack, PACK_URL);
  await ask(listener, { kind: 'dx-pack', url: PACK_URL, bytes: pack.toString('base64') });
  assert.equal((await ask(listener, READ_WELCOME)).value, 'from the daemon');

  alive = false;
  const answered = await ask(listener, READ_WELCOME);
  assert.equal(answered.error, undefined, answered.error);
  assert.match(answered.value, /^::heading level=1 id=/, 'the reader must still see the document');
});

test('the worker probes only loopback, and only the ports dx serve binds', () => {
  const worker = readFileSync(join(here, '../engine.js'), 'utf8');
  const ports = /const PORTS = \[([^\]]*)\]/.exec(worker);
  assert.ok(ports, 'engine.js must state the ports it probes');
  assert.deepEqual(
    ports[1].split(',').map((port) => Number(port.trim())),
    [7333, 7334, 7335, 7336],
  );
  // A daemon address that was not loopback would send the reader's private repository
  // somewhere off this machine.
  for (const [, host] of worker.matchAll(/fetch\(`(https?:\/\/[^/`$]*)/g)) {
    assert.match(host, /^http:\/\/127\.0\.0\.1:\$?\{?/, `the worker fetched ${host}`);
  }
});

test('every surface styles itself from palette names the engine actually publishes', (t) => {
  if (!shipped) {
    t.skip('run editor/build.sh first');
    return;
  }
  // A CSS variable that is never defined is not a soft fallback: the declaration using it is
  // invalid and the browser drops it. The VS Code toolbar named `--dx-surface-2`, `--dx-border`
  // and `--dx-sans` long after the stylesheet stopped defining them, so it rendered with no
  // background, no rule and the document's serif — unstyled, and silently. The palette is
  // `doc-core`'s to declare, so anything drawn beside a document has to be held to it.
  const palette = new Set(
    [...shipped.stylesheet().matchAll(/(--dx-[a-z0-9-]+)\s*:/g)].map(([, name]) => name),
  );
  assert.ok(palette.size > 5, 'the stylesheet must declare a palette');

  const toolbar = readFileSync(join(here, '../../vscode/src/preview.ts'), 'utf8');
  for (const [, name] of toolbar.matchAll(/var\((--dx-[a-z0-9-]+)/g)) {
    assert.ok(palette.has(name), `the VS Code toolbar uses ${name}, which the engine never defines`);
  }
});

test('a message that is not for the engine is left alone', async (t) => {
  if (!shipped) {
    t.skip('run editor/build.sh first');
    return;
  }
  const listener = loadWorker(shipped);
  // Other extensions and GitHub's own code share this channel; answering their messages would
  // break them. `false` means "not mine", and it has to stay that way.
  assert.equal(listener({ kind: 'something-else' }, {}, () => {}), false);
});
