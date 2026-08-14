// What the extension has to get right, checked without a browser.
//
// The DOM wiring cannot be tested from a terminal, so everything that decides *what the
// reader sees* lives in `resolve.js` and is exercised here against a pack that the real `dx`
// binary wrote — the same bytes github.com would serve from `.doc/repo.dxcp`.
//
// Run with:
//   node --test editor/github/test/
//
// The fixture is built by `test/fixture.sh`, which needs a `dx` build; the tests that need
// it skip with a message rather than failing when it is absent, so the suite still runs on a
// machine that has not built the binary.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

import { loadShippedEngine } from './engine.mjs';

// Imported for its side effect: `resolve.js` is loadable as a classic content script, so it
// publishes its API on `globalThis` rather than with `export`.
import '../resolve.js';

const { REPO_PACK, collapse, diffLines, digestIn, isDocumentPath, locate, rawUrl, resolveDocument } =
  globalThis.dxResolve;

const here = dirname(fileURLToPath(import.meta.url));

/// The engine the extension ships, loaded once for the whole file.
///
/// This is the wasm in `editor/github/wasm` — the bytes `dx` embeds and a browser runs — not
/// a second build made for node. Testing against anything else would leave the shipped engine
/// unexercised, which is exactly the gap a reader falls into.
const shipped = await loadShippedEngine();

/// The wasm engine, or `null` when it has not been built.
function engine() {
  return shipped;
}

/// The pack and pointer `dx sync` produced for the fixture repository.
function fixture() {
  const pack = join(here, 'fixture/repo.dxcp');
  const pointer = join(here, 'fixture/welcome.dx');
  if (!existsSync(pack) || !existsSync(pointer)) return null;
  return {
    pack: new Uint8Array(readFileSync(pack)),
    pointer: readFileSync(pointer, 'utf8'),
  };
}

test('a pointer line is recognized, and nothing else is', () => {
  assert.equal(
    digestIn('~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823\n'),
    'c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823',
  );
  // A document that merely starts with a tilde is content, not a pointer: treating it as one
  // would replace what the author wrote.
  assert.equal(digestIn('~ dx1 not-a-digest'), null);
  assert.equal(digestIn('~ dx2 c939d5be'), null);
  assert.equal(digestIn('::paragraph id=p\n~ dx1 abc\n::end\n'), null);
  assert.equal(digestIn(''), null);
  assert.equal(digestIn(null), null);
});

/**
 * The page cannot ask the engine what a pointer is — the engine is a message away and the
 * answer is needed before the message can be sent — so `resolve.js` keeps the only copy of
 * the grammar outside `doc_core::pointer`. This is what keeps the copy honest. It had
 * already diverged once: the engine accepts a digest written in upper case and this did not,
 * so such a file rendered everywhere except on github.com, where it stayed a pointer.
 */
test('the page recognizes exactly the pointers the engine does', (t) => {
  const wasm = engine();
  if (!wasm) {
    t.skip('run editor/build.sh first');
    return;
  }
  const digest = 'c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823';
  for (const text of [
    `~ dx1 ${digest}\n`,
    `~ dx1 ${digest.toUpperCase()}  \n`,
    `~ dx1 ${digest}`,
    '~ dx1 not-a-digest',
    '~ dx2 c939d5be',
    '~',
    '::paragraph id=p\n~ dx1 abc\n::end\n',
    '',
  ]) {
    assert.equal(digestIn(text) ?? '', wasm.pointer_digest(text), JSON.stringify(text));
  }
});

test('only .dx files are claimed', () => {
  assert.ok(isDocumentPath('docs/notes.dx'));
  assert.ok(isDocumentPath('NOTES.DX'));
  assert.ok(!isDocumentPath('notes.md'));
  assert.ok(!isDocumentPath('notes.dx.bak'));
});

test('a blob url yields owner, repo, ref, and path', () => {
  const at = locate('https://github.com/rocky/notes/blob/main/docs/guide.dx');
  assert.deepEqual(
    { owner: at.owner, repo: at.repo, kind: at.kind, ref: at.ref, path: at.path },
    { owner: 'rocky', repo: 'notes', kind: 'blob', ref: 'main', path: 'docs/guide.dx' },
  );
  assert.equal(locate('https://github.com/rocky/notes/pull/12/files').kind, 'pull');
  // A repository root names no file, and a page that is not a repository names nothing.
  assert.equal(locate('https://github.com/rocky/notes').path, null);
  assert.equal(locate('https://github.com/rocky'), null);
  assert.equal(locate('not a url'), null);
});

test('a compare url states both sides of the diff', () => {
  // Nothing else on a compare page does. The two sides are branch names, which no attribute
  // in GitHub's markup carries as commit ids, so a compare view rendered a raw pointer until
  // the range was read out of the URL.
  const three = locate('https://github.com/rocky/notes/compare/main...feature');
  assert.equal(three.kind, 'compare');
  assert.equal(three.base, 'main');
  assert.equal(three.head, 'feature');

  // GitHub accepts two dots for the same comparison.
  const two = locate('https://github.com/rocky/notes/compare/v1.0..v2.0');
  assert.deepEqual([two.base, two.head], ['v1.0', 'v2.0']);

  // A branch name may contain slashes on either side.
  const slashes = locate('https://github.com/rocky/notes/compare/release/1...feature/x');
  assert.deepEqual([slashes.base, slashes.head], ['release/1', 'feature/x']);

  // A side naming another fork stays as written: this repository cannot serve it, and
  // resolving it here would show a different repository's file as if it were this one's.
  const fork = locate('https://github.com/rocky/notes/compare/main...someone:main');
  assert.equal(fork.head, 'someone:main');
});

test('a pull request diff tab states its sides through embedded route data', () => {
  // Captured shapes from github.com's current "Files changed" tab (2026-08-14): neither
  // `data-base-sha` nor `data-head-sha` exists on the page any more, and the two commit ids
  // live nested inside a `<script data-target="react-app.embeddedData">` blob instead, at a
  // path that has moved more than once across GitHub's own rollout. `routeShasFrom` is handed
  // the raw script text the way `content.js` collects it — this is the one function GitHub's
  // next reshuffle is likely to break, so it is the one under test.
  const changesRoutePayload = JSON.stringify({
    payload: {
      pullRequestsChangesRoute: {
        comparison: {
          baseOid: 'a'.repeat(40),
          headOid: 'b'.repeat(40),
          fullDiff: { baseOid: 'a'.repeat(40), headOid: 'b'.repeat(40) },
        },
      },
    },
  });
  assert.deepEqual(globalThis.dxResolve.routeShasFrom([changesRoutePayload]), {
    base: 'a'.repeat(40),
    head: 'b'.repeat(40),
  });

  // An older shape seen on the same route: the ids nested one level deeper, under the pull
  // request object rather than directly under the comparison. The search has to find them
  // there too, since which shape is live is not something the extension controls.
  const nestedUnderPullRequest = JSON.stringify({
    payload: {
      pullRequestsChangesRoute: {
        pullRequest: { comparison: { baseOid: 'c'.repeat(40), headOid: 'd'.repeat(40) } },
      },
    },
  });
  assert.deepEqual(globalThis.dxResolve.routeShasFrom([nestedUnderPullRequest]), {
    base: 'c'.repeat(40),
    head: 'd'.repeat(40),
  });
});

test('a commit page has no route shas, so its own strategy still applies', () => {
  // A commit page's embedded data (when it has any at all) carries no key shaped like
  // `baseOid`/`headOid` — the parent-commit lookup is a different code path entirely. This
  // pins that `routeShasFrom` steps aside rather than matching something it should not.
  const commitPagePayload = JSON.stringify({
    payload: { commit: { oid: 'e'.repeat(40), author: { name: 'rocky' } } },
  });
  assert.equal(globalThis.dxResolve.routeShasFrom([commitPagePayload]), null);
});

test('route sha extraction tries every script and skips one that will not parse', () => {
  const unrelated = JSON.stringify({ payload: { something: 'else' } });
  const malformed = '{not json';
  const real = JSON.stringify({
    payload: { pullRequestsChangesRoute: { comparison: { baseOid: 'f'.repeat(40), headOid: '1'.repeat(40) } } },
  });
  assert.deepEqual(globalThis.dxResolve.routeShasFrom([unrelated, malformed, real]), {
    base: 'f'.repeat(40),
    head: '1'.repeat(40),
  });
  assert.equal(globalThis.dxResolve.routeShasFrom([]), null);
  assert.equal(globalThis.dxResolve.routeShasFrom([malformed]), null);
});

test('the pack is fetched from github.com itself, so private repos work', () => {
  // raw.githubusercontent.com would need its own host permission and would not carry the
  // reader's session; github.com/<owner>/<repo>/raw/... does both.
  const url = rawUrl({ owner: 'rocky', repo: 'notes' }, 'main', REPO_PACK);
  assert.equal(url, 'https://github.com/rocky/notes/raw/main/.doc/repo.dxcp');
  assert.ok(url.startsWith('https://github.com/'));
});

test('a pointer resolves to the document the pack holds', async (t) => {
  const wasm = engine();
  const data = fixture();
  if (!wasm || !data) {
    t.skip('run editor/github/test/fixture.sh first (needs a dx build)');
    return;
  }

  const result = await resolveDocument({
    engine: wasm,
    fetchPack: async () => data.pack,
    location: { owner: 'rocky', repo: 'notes' },
    ref: 'main',
    path: 'welcome.dx',
    pointerText: data.pointer,
  });

  assert.equal(result.state, 'document');
  // The real thing, not a summary of it: block syntax intact, byte-for-byte from the pack.
  assert.match(result.source, /^::heading level=1 id=/);
  assert.ok(result.source.includes('::end'));
  assert.ok(result.source.length > 200);
});

test('a document is verified against its pointer before it is shown', async (t) => {
  const wasm = engine();
  const data = fixture();
  if (!wasm || !data) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }

  // A pointer naming a different digest than the pack holds means the commit and the pack
  // disagree. Showing the pack's version would be wrong and look right, so it is refused.
  const result = await resolveDocument({
    engine: wasm,
    fetchPack: async () => data.pack,
    location: { owner: 'rocky', repo: 'notes' },
    ref: 'main',
    path: 'welcome.dx',
    pointerText: `~ dx1 ${'a'.repeat(64)}\n`,
  });
  assert.equal(result.state, 'stale');
  assert.match(result.message, /dx sync/);
});

test('a missing pack says what to run instead of showing nothing', async (t) => {
  const wasm = engine();
  const data = fixture();
  if (!wasm || !data) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }

  const missing = await resolveDocument({
    engine: wasm,
    fetchPack: async () => null,
    location: { owner: 'rocky', repo: 'notes' },
    ref: 'main',
    path: 'welcome.dx',
    pointerText: data.pointer,
  });
  assert.equal(missing.state, 'no-pack');
  assert.match(missing.message, /\.doc\/repo\.dxcp/);
  assert.match(missing.message, /dx sync/);

  const absent = await resolveDocument({
    engine: wasm,
    fetchPack: async () => data.pack,
    location: { owner: 'rocky', repo: 'notes' },
    ref: 'main',
    path: 'nowhere.dx',
    pointerText: data.pointer,
  });
  assert.equal(absent.state, 'not-in-pack');
  // It names what the pack does hold, so the reader is not left guessing.
  assert.match(absent.message, /welcome\.dx/);
});

test('a file that is not a pointer is left to github', async () => {
  const result = await resolveDocument({
    engine: {},
    fetchPack: async () => {
      throw new Error('must not be fetched');
    },
    location: { owner: 'a', repo: 'b' },
    ref: 'main',
    path: 'plain.dx',
    pointerText: '::paragraph id=p\nPlain text document.\n::end\n',
  });
  assert.equal(result.state, 'not-a-pointer');
});

test('the diff shows which lines of the document changed', () => {
  const before = '::paragraph id=intro\nOne.\n::end\n';
  const after = '::paragraph id=intro\nTwo.\n::end\n';
  const rows = diffLines(before, after);
  assert.deepEqual(
    rows.map((row) => [row.kind, row.text]),
    [
      ['same', '::paragraph id=intro'],
      ['removed', 'One.'],
      ['added', 'Two.'],
      ['same', '::end'],
    ],
  );
});

test('an unchanged document diffs to nothing but context', () => {
  const text = '::paragraph id=p\nSame.\n::end\n';
  assert.ok(diffLines(text, text).every((row) => row.kind === 'same'));
});

test('a diff against an empty side is all additions or all removals', () => {
  const text = '::paragraph id=p\nNew.\n::end\n';
  assert.ok(diffLines('', text).every((row) => row.kind === 'added'));
  assert.ok(diffLines(text, '').every((row) => row.kind === 'removed'));
});

test('long runs of unchanged lines collapse around the change', () => {
  const before = Array.from({ length: 40 }, (_, i) => `line ${i}`).join('\n');
  const after = before.replace('line 20', 'line twenty');
  const rows = collapse(diffLines(before, after), 2);

  const skips = rows.filter((row) => row.kind === 'skip');
  assert.equal(skips.length, 2, 'the head and tail runs both collapse');
  assert.ok(rows.some((row) => row.kind === 'added' && row.text === 'line twenty'));
  assert.ok(rows.some((row) => row.kind === 'removed' && row.text === 'line 20'));
  // Nothing is lost: every collapsed line is still counted.
  const shown = rows.filter((row) => row.kind !== 'skip').length;
  const hidden = skips.reduce((sum, row) => sum + row.count, 0);
  assert.equal(shown + hidden, diffLines(before, after).length);
});

test('resolve.js stays loadable as a classic content script', () => {
  // A manifest content script is not a module. One stray `export` or `import` would make
  // the whole extension fail to load, in a way no logic test would notice — the tests here
  // run this file as a module, where both are legal.
  const source = readFileSync(join(here, '../resolve.js'), 'utf8');
  const offenders = source
    .split('\n')
    .map((line, index) => [index + 1, line])
    .filter(([, line]) => /^\s*(export|import)\s/.test(line));
  assert.deepEqual(offenders, [], 'resolve.js must contain no module syntax');

  // The same for the content script, and for the service worker: the worker is declared
  // without `"type": "module"` so that it can `importScripts` the engine, which means module
  // syntax would stop it loading — and a worker that fails to load takes every render with it.
  for (const file of ['../content.js', '../engine.js']) {
    const source = readFileSync(join(here, file), 'utf8');
    assert.deepEqual(
      source
        .split('\n')
        .map((line, index) => [index + 1, line])
        .filter(([, line]) => /^\s*(export|import)\s/.test(line)),
      [],
      `${file} must contain no module syntax`,
    );
  }
});

test('the manifest declares exactly what the extension needs', () => {
  const manifest = JSON.parse(readFileSync(join(here, '../manifest.json'), 'utf8'));
  assert.equal(manifest.manifest_version, 3);
  // No permissions at all: the pack is fetched same-origin from the page the reader is on.
  assert.deepEqual(manifest.permissions, []);
  assert.deepEqual(manifest.content_scripts[0].matches, ['https://github.com/*']);

  // Every file the content script needs, in the order it needs them: `resolve.js` publishes
  // `window.dxResolve`, which `content.js` calls on its first line of work. Injecting
  // `content.js` without it — which is what this manifest did until it was loaded in a real
  // browser — leaves the extension throwing on every github.com page and rendering nothing.
  assert.deepEqual(manifest.content_scripts[0].js, ['resolve.js', 'content.js']);
});

test('the engine is declared where WebAssembly is allowed to run', () => {
  const manifest = JSON.parse(readFileSync(join(here, '../manifest.json'), 'utf8'));

  // A content script shares the host page's WebAssembly policy, and github.com serves
  // `script-src github.githubassets.com` with no `'wasm-unsafe-eval'` — so every
  // `WebAssembly.compile` inside a content script on github.com fails, however the bytes were
  // obtained. The engine therefore runs in the service worker, under this extension's own
  // policy, which has to grant `'wasm-unsafe-eval'` for that to work.
  assert.equal(manifest.background.service_worker, 'engine.js');
  assert.match(manifest.content_security_policy.extension_pages, /'wasm-unsafe-eval'/);

  // The corollary: the engine must not be injected into the page, where it cannot compile.
  assert.ok(
    !manifest.content_scripts[0].js.some((file) => file.startsWith('wasm/')),
    'the wasm engine must not be a content script',
  );
  const content = readFileSync(join(here, '../content.js'), 'utf8');
  assert.ok(
    !/\bwasm_bindgen\b/.test(content),
    'content.js must reach the engine by message, not load it',
  );
});

test('every file the manifest names exists', () => {
  const manifest = JSON.parse(readFileSync(join(here, '../manifest.json'), 'utf8'));
  const named = [
    manifest.background.service_worker,
    ...manifest.content_scripts.flatMap((script) => [...script.js, ...(script.css ?? [])]),
    ...(manifest.web_accessible_resources ?? []).flatMap((entry) => entry.resources),
  ];
  for (const file of named) {
    // The wasm directory is a build product, so its absence means "not built yet" rather than
    // a broken manifest; everything checked in must be there.
    if (file.startsWith('wasm/')) continue;
    assert.ok(existsSync(join(here, '..', file)), `manifest names a missing file: ${file}`);
  }
});

test('the worker answers every engine call the page side makes', () => {
  // The two halves of the message protocol are in different files and different contexts, so
  // nothing but a test keeps them in step: a call added to `content.js` and forgotten in
  // `engine.js`'s allowlist would fail only on a live page, as `unknown engine call`.
  const worker = readFileSync(join(here, '../engine.js'), 'utf8');
  const allowed = /const ALLOWED = new Set\(\[([^\]]*)\]\)/.exec(worker);
  assert.ok(allowed, 'engine.js must state its allowlist');
  const names = new Set(Array.from(allowed[1].matchAll(/'([^']+)'/g), (m) => m[1]));

  const content = readFileSync(join(here, '../content.js'), 'utf8');
  const called = Array.from(content.matchAll(/\bcall\('([a-z0-9_]+)'/g), (m) => m[1]);
  assert.ok(called.length >= 5, 'the engine proxy should forward the calls the page needs');
  for (const name of called) {
    assert.ok(names.has(name), `engine.js does not allow the call content.js makes: ${name}`);
  }
});

test('resolution works when the engine lives in another context', async (t) => {
  const wasm = engine();
  const data = fixture();
  if (!wasm || !data) {
    t.skip('run editor/github/test/fixture.sh first');
    return;
  }

  // On github.com the engine is not an object in the caller's world: it is a proxy that
  // messages the service worker, so every method answers with a promise. This stands in for
  // that, and would fail if `resolveDocument` ever stopped awaiting an engine call — the
  // digest check would compare a `Promise` against a string and report every document stale.
  const remote = {
    pack_document: async (...args) => wasm.pack_document(...args),
    pack_paths: async (...args) => wasm.pack_paths(...args),
    sha256_hex: async (...args) => wasm.sha256_hex(...args),
  };

  const result = await resolveDocument({
    engine: remote,
    fetchPack: async () => data.pack,
    location: { owner: 'rocky', repo: 'notes' },
    ref: 'main',
    path: 'welcome.dx',
    pointerText: data.pointer,
  });
  assert.equal(result.state, 'document');
  assert.match(result.source, /^::heading level=1 id=/);

  // And a failing call still reports the reason, rather than an unhandled rejection.
  const missing = await resolveDocument({
    engine: remote,
    fetchPack: async () => data.pack,
    location: { owner: 'rocky', repo: 'notes' },
    ref: 'main',
    path: 'nowhere.dx',
    pointerText: data.pointer,
  });
  assert.equal(missing.state, 'not-in-pack');
});
