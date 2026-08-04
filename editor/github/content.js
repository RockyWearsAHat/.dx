// The page adapter: find a `.dx` pointer on github.com, replace it with the document.
//
// Everything that decides *what* the reader sees lives in `resolve.js` and is tested under
// node. This file is the thin layer that only a browser can run: locate the pointer in
// GitHub's markup, ask the engine what it stands for, and put the rendering in its place.
//
// # Why this exists at all
// github.com's file view is not extensible. `github/markup` picks from a fixed, first-party
// list of renderers, and a GitHub App can add checks and comments but cannot change what a
// blob page shows. So a committed `.dx` file — one line, `~ dx1 <sha256>` — renders as that
// line for every reader. Resolving it in the page is the only way the actual document
// appears on github.com, and it works because the content is *committed*: `.doc/repo.dxcp`
// is fetched from the same origin, so private repositories resolve on the reader's own
// session with no token and no server in the middle.
//
// # What it never does
// It never invents content. If the pack is missing, does not hold the path, or disagrees
// with the pointer's digest, the page says so and says what to run — the same rule the CLI
// keeps: a reader must never see a pointer where a document belongs, or nothing where
// content exists.
//
// # Why the resolver is read off `globalThis`
// `resolve.js` publishes its API on `globalThis`, and this file reads it from there — not
// from `window`. In Chrome those are the same object inside a content script's isolated
// world, so either spelling works; in **Firefox** they are not. A content script's `window`
// is a wrapper around the *page's* window, while `globalThis` is the sandbox the extension's
// own scripts share, so `window.dxResolve` is `undefined` in Firefox and every page renders
// nothing at all. Reading the global the resolver actually wrote to is correct in both.
//
// # Why the engine is not here
// A content script inherits the page's WebAssembly policy, and github.com's `script-src`
// does not grant `'wasm-unsafe-eval'` — so compiling the engine in this script fails on
// every github.com page. The engine is reached through `engine.js`, which answers from the
// local `dx serve` daemon when it is running and from bundled wasm when it is not; this
// script cannot tell which, and does not need to. The *fetching* stays here, because only a
// page-context request carries the reader's session, which is what makes a private
// repository resolve with no token and no server in the middle.

/* global chrome */

(() => {
  'use strict';

  /// Marker attribute so a rendered container is never processed twice.
  const DONE = 'data-dx-rendered';

  /// Base64 for a byte array, in chunks small enough to pass in one message.
  ///
  /// Runtime messages are JSON, so a `Uint8Array` has to be encoded to survive the trip to
  /// the worker; a pack sent as a raw typed array would arrive as an object of numeric keys.
  /// This runs once per repository revision — see [`call`].
  function base64(bytes) {
    const CHUNK = 0x8000;
    let text = '';
    for (let at = 0; at < bytes.length; at += CHUNK) {
      text += String.fromCharCode.apply(null, bytes.subarray(at, at + CHUNK));
    }
    return btoa(text);
  }

  /// Send one message to the background engine and resolve with its reply.
  function send(message) {
    return new Promise((resolve, reject) => {
      chrome.runtime.sendMessage(message, (reply) => {
        const disconnected = chrome.runtime.lastError;
        if (disconnected) reject(new Error(disconnected.message));
        else if (!reply) reject(new Error('the dx engine did not answer'));
        else if (reply.error) reject(new Error(reply.error));
        else resolve(reply);
      });
    });
  }

  /// Make one engine call and return its result.
  ///
  /// # Why a pack is named rather than sent
  /// The engine is asked about `{ packRef: url }`, not about bytes. A pull request touching
  /// six documents makes twelve of these calls; sending the repository's pack with each one
  /// meant base64-encoding several megabytes twelve times and decompressing it twelve times
  /// on the other side, which is what made a diff page crawl. Now the engine says which pack
  /// it lacks, gets it once, and answers every later call from memory — and if that engine is
  /// the daemon, from memory that outlives the page.
  ///
  /// Rejects with the engine's own message, so a failure reaches the reader as the sentence
  /// the engine wrote rather than as a blank page.
  async function call(name, ...args) {
    const message = { kind: 'dx-engine', call: name, args };
    const reply = await send(message);
    if (!reply.needPack) return reply.value;

    const bytes = await packBytes(reply.needPack);
    if (!bytes) throw new Error(`the pack at ${reply.needPack} could not be read`);
    await send({ kind: 'dx-pack', url: reply.needPack, bytes: base64(bytes) });

    const answered = await send(message);
    if (answered.needPack) {
      throw new Error('the dx engine did not keep the pack it asked for');
    }
    return answered.value;
  }

  /// The engine, as the object `resolve.js` and this script call into.
  ///
  /// Every method returns a promise, which is why callers await them: the work happens in
  /// another context. `resolve.js` awaits its engine calls for exactly this reason and so
  /// stays usable with a synchronous engine under node.
  const engine = {
    pack_document: (pack, path) => call('pack_document', pack, path),
    pack_paths: (pack) => call('pack_paths', pack),
    // The only bytes ever hashed are a document's source, which came from a string and is
    // therefore valid UTF-8, so it crosses as text and the engine encodes it back. That
    // keeps one argument grammar for both engines rather than a second base64 path.
    sha256_hex: (bytes) => call('sha256_hex', { text: new TextDecoder().decode(bytes) }),
    render_html: (...args) => call('render_html', ...args),
    references: (source) => call('references', source),
    stylesheet: () => call('stylesheet'),
  };

  /// Packs this page has fetched, by URL, as promises so concurrent callers share one request.
  const packs = new Map();

  /// The bytes of the pack at `url`, fetched at most once per page.
  ///
  /// `credentials: 'same-origin'` is what makes a private repository work: the request
  /// carries the session the reader is already using to view the page. This is why the
  /// fetching stays in the page and does not move to the daemon, which has no session and
  /// wants no token.
  function packBytes(url) {
    if (!packs.has(url)) {
      packs.set(
        url,
        (async () => {
          try {
            const response = await fetch(url, { credentials: 'same-origin' });
            if (!response.ok) return null;
            return new Uint8Array(await response.arrayBuffer());
          } catch {
            return null;
          }
        })(),
      );
    }
    return packs.get(url);
  }

  /// A reference to the pack at `url`, or `null` when there is no pack there.
  ///
  /// The reference is what every engine call carries. Fetching here rather than only on
  /// demand is deliberate: a missing pack is the difference between "this repository did not
  /// commit its documents" and "this path is not in it", and the reader is told which.
  async function packHandle(url) {
    return (await packBytes(url)) ? { packRef: url } : null;
  }

  /// Fetch the pack at `url` again and hand it to the engine, replacing what it held.
  ///
  /// # Why a pack ever goes stale
  /// A pack is keyed by the URL it came from, and that URL names a *ref* — which is a commit
  /// id on a permalink but often a branch name (`…/raw/main/.doc/repo.dxcp`). A branch moves.
  /// While the engine was a service worker the browser shut down after seconds this could
  /// hardly be observed; the daemon holds what it decodes for as long as it runs, so the
  /// window is now the daemon's uptime. When a resolved document does not match its pointer,
  /// the first suspect is this cache rather than the repository.
  async function refreshPack(url) {
    packs.delete(url);
    const bytes = await packBytes(url);
    if (!bytes) return null;
    await send({ kind: 'dx-pack', url, bytes: base64(bytes) });
    return { packRef: url };
  }

  /// Resolve `path` at `ref`, retrying once against a freshly fetched pack if the first
  /// answer disagrees with the pointer.
  ///
  /// Everything the reader is shown comes through here, so there is one verification path and
  /// not two. A document is only ever displayed when its SHA-256 equals the digest its
  /// pointer names.
  async function resolveVerified(location, ref, path, pointerText) {
    const ask = () =>
      globalThis.dxResolve.resolveDocument({
        engine,
        fetchPack: packHandle,
        location,
        ref,
        path,
        pointerText,
      });

    const outcome = await ask();
    if (outcome.state !== 'stale') return outcome;

    // Could be a moved branch cached by the daemon, or a repository whose pack really was
    // committed without its documents. Refetching tells them apart.
    const refreshed = await refreshPack(
      globalThis.dxResolve.rawUrl(location, ref, globalThis.dxResolve.REPO_PACK),
    );
    return refreshed ? await ask() : outcome;
  }

  /// The ref and path the *page* states, preferred over parsing them out of the URL.
  ///
  /// A branch name containing a slash makes `/blob/feature/x/notes.dx` ambiguous, and GitHub
  /// already resolved that ambiguity server-side: the permalink it renders names the commit
  /// and the full path. Reading it avoids guessing.
  function refAndPathFrom(document_) {
    const permalink = document_.querySelector('[data-hotkey="y"]')?.getAttribute('href');
    if (permalink) {
      const parts = permalink.split('/').filter(Boolean);
      const at = parts.indexOf('blob');
      if (at >= 0 && parts.length > at + 2) {
        return { ref: parts[at + 1], path: parts.slice(at + 2).join('/') };
      }
    }
    return null;
  }

  /// Gather what `source` references into the resources JSON `render_html` takes, or
  /// `undefined` when it references nothing or the page's context is unknown.
  ///
  /// The engine says *what* is referenced (`references`) and what becomes of the bytes;
  /// this gathers them the only way a page on github.com can: sibling documents from the
  /// repository's committed pack, sibling files from the repository's own raw route —
  /// the same session-carrying fetch the pack itself uses, so a private repository's
  /// files resolve exactly like its documents. A path that cannot be gathered is left
  /// out, and the engine renders its honest sentence in that block's place.
  async function resourcesFor(source, context) {
    if (!context || !context.path) return undefined;
    let refs;
    try {
      refs = JSON.parse(await engine.references(source));
    } catch {
      return undefined;
    }
    if (!Array.isArray(refs) || refs.length === 0) return undefined;

    const slash = context.path.lastIndexOf('/');
    const folder = slash < 0 ? '' : context.path.slice(0, slash);
    const inRepo = (relative) => (folder ? `${folder}/${relative}` : relative);
    const files = {};
    const documents = {};
    for (const ref of refs) {
      if (typeof ref.path !== 'string') continue;
      if (ref.kind === 'document') {
        const handle = await packHandle(
          globalThis.dxResolve.rawUrl(context.location, context.ref, globalThis.dxResolve.REPO_PACK),
        );
        if (!handle) continue;
        try {
          documents[ref.path] = await engine.pack_document(handle, inRepo(ref.path));
        } catch {
          // Not in the pack: the engine's sentence on the page says so.
        }
      } else if (ref.kind === 'file') {
        try {
          const response = await fetch(
            globalThis.dxResolve.rawUrl(context.location, context.ref, inRepo(ref.path)),
            { credentials: 'same-origin' },
          );
          if (response.ok) files[ref.path] = await response.text();
        } catch {
          // Unreachable: the engine's sentence on the page says so.
        }
      }
    }
    return JSON.stringify({ files, documents });
  }

  /// Build the element that shows a resolved document: the rendered page, on a blank sheet.
  ///
  /// `context` is `{ location, ref, path }` when the page knows where the document sits,
  /// which is what lets its references resolve; without it they render as sentences.
  async function documentElement(source, context) {
    const host = document.createElement('div');
    host.className = 'dx-github-doc';
    host.setAttribute(DONE, 'true');

    // Both come from the engine in the service worker, so both are awaited.
    const [css, html] = await Promise.all([
      engine.stylesheet(),
      resourcesFor(source, context).then((resources) =>
        engine.render_html(source, theme(), true, resources),
      ),
    ]);

    // A shadow root keeps the document's stylesheet from touching GitHub's page and vice
    // versa, so the rendering looks the same here as in the editor and the CLI.
    const shadow = host.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = css;
    const body = document.createElement('div');
    // Wide content — a full-bleed table or diagram — scrolls here rather than on the host, so
    // that the host never becomes a scroll container. See the note in `content.css`.
    body.style.overflowX = 'auto';
    body.innerHTML = html;
    shadow.append(style, body);
    return host;
  }

  /// The palette to render with, taken from GitHub's own setting so the document matches the
  /// page it sits in.
  function theme() {
    const mode = document.documentElement.getAttribute('data-color-mode');
    if (mode === 'light' || mode === 'dark') return mode;
    return 'auto';
  }

  /// Build the element that explains why a document could not be shown.
  function noticeElement(message) {
    const host = document.createElement('div');
    host.className = 'dx-github-notice';
    host.setAttribute(DONE, 'true');
    host.textContent = `dx: ${message}`;
    return host;
  }

  /// Render a resolved outcome into `container`, replacing what GitHub put there.
  ///
  /// The replacement is built before the container is emptied, so a failure inside the engine
  /// leaves GitHub's own rendering in place rather than blanking the file.
  async function show(container, outcome, context) {
    if (outcome.state === 'not-a-pointer') return;
    const replacement =
      outcome.state === 'document'
        ? await documentElement(outcome.source, context)
        : noticeElement(outcome.message);
    container.setAttribute(DONE, 'true');
    container.textContent = '';
    container.append(replacement);
  }

  /// The nearest element that contains both nodes.
  function commonAncestor(one, other) {
    const above = new Set();
    for (let at = one; at; at = at.parentElement) above.add(at);
    for (let at = other; at; at = at.parentElement) {
      if (above.has(at)) return at;
    }
    return one;
  }

  /// The element GitHub renders this file's content into, across the layouts it has shipped.
  ///
  /// The current layout paints the file *twice*: `.react-code-lines` holds the lines the
  /// reader sees, and `#read-only-cursor-text-area` is an overlaid textarea carrying the
  /// selection and cursor. Replacing either one alone leaves the other still showing the
  /// pointer line, so the target is the nearest element that holds both.
  ///
  /// Only unhashed hooks are named. GitHub's CSS-module class names carry a build hash
  /// (`CodeBlob-module__codeBlobInner__tfjuQ`) that changes without notice, so the container
  /// is found by structure instead of by matching one.
  function blobContainer() {
    // The file view and the blame view both render the file into one region, and it is the
    // right level to replace for a reason that is easy to get wrong: the regions *inside* it
    // are height-pinned by GitHub's virtual scroller and clipped by `overflow-y: hidden`, so a
    // document placed in one of those has the correct measured height and is still shown as a
    // single line. Only the prefix of the CSS-module name is matched — the hash after it
    // changes between builds, the name itself comes from the stylesheet and does not.
    const region = document.querySelector('[class*="CodeBlob-module__codeBlobInner"]');
    if (region) return region;

    // Older layouts, which state the container directly.
    const lines =
      document.querySelector('[data-testid="blob-viewer-file-content"]') ??
      document.querySelector('.js-file-line-container') ??
      document.querySelector('.blob-wrapper') ??
      document.querySelector('.react-code-lines');
    if (!lines) return null;

    // Where a cursor overlay exists, the file is painted twice and both copies have to go, or
    // the pointer stays on screen beside the document.
    const cursor = document.querySelector('#read-only-cursor-text-area');
    return cursor ? commonAncestor(lines, cursor) : lines;
  }

  /// Handle a file view: one pointer, one document.
  async function renderBlob(location) {
    const stated = refAndPathFrom(document);
    const ref = stated?.ref ?? location.ref;
    const path = stated?.path ?? location.path;
    if (!ref || !path || !globalThis.dxResolve.isDocumentPath(path)) return;

    const container = blobContainer();
    if (!container || container.getAttribute(DONE)) return;

    const pointerText = await fetchText(
      globalThis.dxResolve.rawUrl(location, ref, path),
      container,
    );
    if (pointerText === null) return;

    const outcome = await resolveVerified(location, ref, path, pointerText);
    await show(container, outcome, { location, ref, path });
  }

  /// Fetch a file as text, falling back to the text GitHub already rendered.
  ///
  /// Reading the served bytes is more reliable than scraping the DOM, but if the fetch is
  /// blocked the pointer is right there on the page, so the container's own text is used.
  async function fetchText(url, container) {
    try {
      const response = await fetch(url, { credentials: 'same-origin' });
      if (response.ok) return await response.text();
    } catch {
      // Fall through to the page's own text.
    }
    const shown = container.textContent?.trim();
    return shown ? `${shown}\n` : null;
  }

  /// Handle a diff view: every changed `.dx` file becomes a document diff.
  ///
  /// GitHub's diff of a pointer is a one-line digest change — accurate and useless. This
  /// resolves both revisions and diffs the documents instead, which is what a reviewer came
  /// to read.
  async function renderDiffs(location) {
    const files = changedFiles();
    if (!files.length) return;

    const refs = diffRefs(location);
    if (!refs) return;

    for (const { path, body } of files) {
      if (!globalThis.dxResolve.isDocumentPath(path)) continue;
      if (!body || body.getAttribute(DONE)) continue;
      body.setAttribute(DONE, 'true');

      const [base, head] = await Promise.all([
        sideSource(location, refs.base, path),
        sideSource(location, refs.head, path),
      ]);
      if (base === null && head === null) {
        body.textContent = '';
        body.append(
          noticeElement(
            `could not resolve ${path} at either revision — is .doc/repo.dxcp committed?`,
          ),
        );
        continue;
      }

      // A side that resolved to something other than a verified document is said out loud.
      // Diffing around it would present the reviewer with a difference that is not real.
      const unresolved = [base, head].find(
        (side) => side !== null && side.state !== 'document',
      );
      if (unresolved) {
        body.textContent = '';
        body.append(noticeElement(unresolved.message));
        continue;
      }

      const rows = globalThis.dxResolve.collapse(
        globalThis.dxResolve.diffLines(base?.source ?? '', head?.source ?? ''),
      );
      body.textContent = '';
      body.append(diffElement(rows));
    }
  }

  /// What GitHub prefixes the accessible name of a diff table with.
  const DIFF_FOR = 'Diff for: ';

  /// Every changed file on a diff page, as `{ path, body }` pairs to replace.
  ///
  /// GitHub is midway through a rewrite and the two layouts are both live: a pull request's
  /// **Files changed** tab still states each path in `data-tagsearch-path`, while a commit
  /// page states it in the diff table's accessible name (`aria-label="Diff for: notes.dx"`)
  /// and has none of the older markup at all. Both are read, so one page migrating does not
  /// take the other down with it.
  ///
  /// Only unhashed hooks are named, for the reason given on [`blobContainer`].
  function changedFiles() {
    const found = [];

    for (const table of document.querySelectorAll(`table[aria-label^="${DIFF_FOR}"]`)) {
      found.push({
        path: table.getAttribute('aria-label').slice(DIFF_FOR.length).trim(),
        body: table,
      });
    }

    for (const file of document.querySelectorAll('[data-tagsearch-path], .file[data-path]')) {
      const path = file.getAttribute('data-tagsearch-path') ?? file.getAttribute('data-path');
      const body = file.querySelector('.js-file-content, .diff-table')?.parentElement;
      if (path && body) found.push({ path, body });
    }

    return found;
  }

  /// A 40-character git object id, as it appears inside a URL or an attribute.
  const SHA = '([0-9a-f]{40})';

  /// Every attribute value on the page that could name a revision.
  ///
  /// GitHub states the two sides of a diff in the URLs of its own routes rather than in one
  /// documented attribute, and which route is present depends on the page and on how large
  /// the diff is. Collecting the candidates once and matching patterns against them keeps
  /// this to a single pass and one place to add the next shape to.
  function revisionCarriers() {
    const carriers = [];
    for (const element of document.querySelectorAll('[data-url], [src], [href], [data-commit]')) {
      for (const name of ['data-url', 'src', 'href', 'data-commit']) {
        const value = element.getAttribute(name);
        if (value) carriers.push(value);
      }
    }
    return carriers;
  }

  /// The first capture of `pattern` across `carriers`, or `null`.
  function firstMatch(carriers, pattern) {
    for (const carrier of carriers) {
      const found = pattern.exec(carrier);
      if (found) return found.slice(1);
    }
    return null;
  }

  /// The two revisions a diff page compares, or `null` when they cannot be determined.
  ///
  /// Tried in order of how directly each source states the answer: the explicit attributes of
  /// the older layout, then GitHub's own diff route which names both sides at once, then the
  /// base and head separately from the routes that carry only one.
  function diffRefs(location) {
    // A compare view says what it compares in its own URL, which is more direct than anything
    // the page carries — and often the only statement of it, since the two sides are branch
    // names and no attribute on the page holds their commit ids.
    if (location?.base && location?.head) return { base: location.base, head: location.head };

    const base = document.querySelector('[data-base-sha]')?.getAttribute('data-base-sha');
    const head = document.querySelector('[data-head-sha]')?.getAttribute('data-head-sha');
    if (base && head) return { base, head };

    const carriers = revisionCarriers();

    // A commit page compares the commit with its parent, and states both without ambiguity:
    // the commit is in the URL, and the parent is the one other commit it links to.
    if (location?.kind === 'commit' && new RegExp(`^${SHA}$`).test(location.ref ?? '')) {
      const parent = firstMatch(
        carriers.filter((carrier) => !carrier.includes(location.ref)),
        new RegExp(`/commit/${SHA}`),
      );
      if (parent) return { base: parent[0], head: location.ref };
    }

    // `/<owner>/<repo>/diffs/<base>..<head>` — both sides, unambiguously.
    const pair = firstMatch(carriers, new RegExp(`/diffs/${SHA}\\.\\.${SHA}`));
    if (pair) return { base: pair[0], head: pair[1] };

    // Otherwise each side from whichever route names it: the comparison endpoints carry the
    // base, and a link to a file at this revision carries the head.
    const fromBase =
      firstMatch(carriers, new RegExp(`base_commit_oid=${SHA}`)) ??
      firstMatch(carriers, new RegExp(`base_sha=${SHA}`));
    const fromHead =
      firstMatch(carriers, new RegExp(`end_commit_oid=${SHA}`)) ??
      firstMatch(carriers, new RegExp(`/blob/${SHA}/`)) ??
      firstMatch(carriers, new RegExp(`^${SHA}$`));
    if (fromBase && fromHead) return { base: fromBase[0], head: fromHead[0] };

    // A commit page compares the commit with its first parent.
    const commit = document.querySelector('.sha.user-select-contain')?.textContent?.trim();
    const parent = document
      .querySelector('.sha-block a.sha, .commit-meta a.sha')
      ?.textContent?.trim();
    if (commit && parent) return { base: parent, head: commit };

    // A commit with no parent — the first commit in a repository — has nothing to compare
    // against, and GitHub itself shows every line of every file as added. `null` here says
    // "this side is empty", which is different from failing to find it: the head is still
    // resolved and shown. Without this, the very first commit of a repository was the one
    // page that still displayed a raw pointer.
    if (location?.kind === 'commit' && new RegExp(`^${SHA}$`).test(location.ref ?? '')) {
      return { base: null, head: location.ref };
    }
    return null;
  }

  /// One side of a diff: what `path` resolves to at `ref`.
  ///
  /// Returns `null` for a side that does not exist — the base of a root commit, or a file
  /// added on one side — and otherwise the same verified outcome a blob page shows.
  ///
  /// # Why this verifies
  /// It used to call the engine directly and return whatever came back, with a bare `catch`
  /// that turned every failure into "this side is empty". So a pack that disagreed with its
  /// pointer produced a confident, wrong diff, and a genuine engine error was indistinguishable
  /// from a newly added file. A reviewer reads a diff to decide whether to merge; showing them
  /// content the commit does not point at is the one thing this system must never do.
  async function sideSource(location, ref, path) {
    if (!ref) return null;
    const pointerText = await fetchPointer(globalThis.dxResolve.rawUrl(location, ref, path));
    if (pointerText === null) return null; // Absent at this revision.
    return resolveVerified(location, ref, path, pointerText);
  }

  /// Fetch a pointer file as text, or `null` when it is not there at this revision.
  async function fetchPointer(url) {
    try {
      const response = await fetch(url, { credentials: 'same-origin' });
      return response.ok ? await response.text() : null;
    } catch {
      return null;
    }
  }

  /// Render diff rows as a plain table: line numbers, a marker, the line.
  function diffElement(rows) {
    const host = document.createElement('div');
    host.className = 'dx-github-diff';
    host.setAttribute(DONE, 'true');

    const table = document.createElement('table');
    for (const row of rows) {
      const tr = document.createElement('tr');
      tr.className = `dx-${row.kind}`;
      if (row.kind === 'skip') {
        const cell = document.createElement('td');
        cell.colSpan = 3;
        cell.className = 'dx-skip-cell';
        cell.textContent = `${row.count} unchanged line${row.count === 1 ? '' : 's'}`;
        tr.append(cell);
      } else {
        const before = document.createElement('td');
        before.className = 'dx-line-number';
        before.textContent = row.before ?? '';
        const after = document.createElement('td');
        after.className = 'dx-line-number';
        after.textContent = row.after ?? '';
        const text = document.createElement('td');
        text.className = 'dx-line';
        const marker = { added: '+', removed: '-', same: ' ' }[row.kind] ?? ' ';
        text.textContent = `${marker} ${row.text}`;
        tr.append(before, after, text);
      }
      table.append(tr);
    }
    host.append(table);
    return host;
  }

  /// Look at the current page and render whatever it holds.
  async function run() {
    const location = globalThis.dxResolve.locate(window.location.href);
    if (!location) return;
    if (location.kind === 'blob' || location.kind === 'blame') {
      await renderBlob(location);
      return;
    }
    if (location.kind === 'pull' || location.kind === 'commit' || location.kind === 'compare') {
      await renderDiffs(location);
    }
  }

  // GitHub navigates without reloading, so the view changes with no load event to hang this
  // on. Turbo fires `turbo:load` on the pages it still drives, but the React router that now
  // serves file and commit views fires neither event — so the DOM is watched directly, and
  // the whole subtree, because a navigation replaces content deep inside the page rather than
  // a direct child of `body`. Watching only direct children meant clicking a `.dx` file from
  // the repository listing left the reader looking at the raw pointer.
  const SETTLE = 150;
  const AT_LATEST = 1000;

  let pending = null;
  let deadline = 0;

  /// Run once the DOM has been quiet for `SETTLE`, and in any case within `AT_LATEST`.
  ///
  /// The deadline matters: a page that mutates continuously — a spinner, a lazily loaded
  /// pane — would reset a plain debounce forever and the document would never appear.
  function schedule() {
    const now = performance.now();
    if (!deadline) deadline = now + AT_LATEST;
    clearTimeout(pending);
    pending = setTimeout(
      () => {
        deadline = 0;
        run().catch((error) => console.error('[dx]', error));
      },
      Math.max(0, Math.min(SETTLE, deadline - now)),
    );
  }

  schedule();
  document.addEventListener('turbo:load', schedule);
  document.addEventListener('pjax:end', schedule);
  window.addEventListener('popstate', schedule);
  new MutationObserver(schedule).observe(document.body, { childList: true, subtree: true });
})();
