// What a `.dx` pointer on github.com stands for.
//
// GitHub shows a repository's files as they are committed, and a committed `.dx` file is one
// line: `~ dx1 <sha256>`. The content is in `.doc/repo.dxcp`, committed beside it. This
// module turns the first into the second — pointer to document — and is deliberately free of
// the DOM, `chrome.*`, and anything else only a browser has, so the part that has to be
// right can be tested under node against a pack that `dx` actually wrote.
//
// The rule it exists to keep is the same one the CLI keeps: a reader never sees a pointer
// where a document belongs, and never sees nothing where content exists. When resolution
// fails, this module says which of those happened and what to do about it — a message, never
// an empty page.
//
// # Why it exports through a global instead of `export`
// A manifest content script cannot be an ES module, and a file containing `export` is a
// syntax error when loaded as a classic script. A file containing *neither* `import` nor
// `export` is valid as both — so the API is attached to `globalThis` at the end, and the node
// tests read it from there after importing this file for its side effect. One file, no
// bundler, and the tested code is byte-for-byte the code the browser runs.

/// The marker that opens a pointer line, followed by the format tag and a 64-hex digest.
///
/// The one copy of the grammar outside the engine, and it is here because this file runs in
/// the page, where the engine is a message away and the answer is needed before any message
/// can be sent. `resolve.test.mjs` holds it to `doc_core::pointer` — the recognizer of
/// record — case for case, so a file is a pointer to every surface or to none.
const POINTER = /^~ dx1 ([0-9a-f]{64})\s*$/i;

/// Where a repository keeps its committed documents, relative to the repository root.
const REPO_PACK = '.doc/repo.dxcp';

/// The digest a pointer records, or `null` when `text` is not a pointer.
///
/// Recognition is strict on purpose: anything that is not exactly the marker plus 64 hex
/// digits is real content that must be shown as it is, never mistaken for a pointer and
/// replaced. A digest comes back lower case however it was written, which is the form the
/// engine records and the form a pack is keyed by.
function digestIn(text) {
  if (typeof text !== 'string') return null;
  const first = text.split('\n', 1)[0].trimEnd();
  const match = POINTER.exec(first);
  return match ? match[1].toLowerCase() : null;
}

/// Whether `path` names a `.dx` document.
function isDocumentPath(path) {
  return typeof path === 'string' && path.toLowerCase().endsWith('.dx');
}

/// Pick a repository location out of a github.com URL.
///
/// Handles the shapes a document can be looked at through: `blob`, `blame`, `raw`, `tree`,
/// commit pages, and pull-request pages. Returns `null` for anything else, so the extension
/// stays inert on the rest of the site.
///
/// # A branch name with a slash
/// `/owner/repo/blob/feature/x/notes.dx` is genuinely ambiguous — `feature/x` could be a
/// branch holding `notes.dx`, or the branch `feature` holding `x/notes.dx`. A URL cannot say
/// which, so this takes the first segment as the ref and the caller prefers the ref and path
/// the *page* states when it has them ([`refAndPathFrom`] in the content script). Guessing
/// here would resolve the wrong file and look confident about it.
function locate(url) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const parts = parsed.pathname.split('/').filter(Boolean);
  if (parts.length < 2) return null;

  const [owner, repo, kind, ...rest] = parts;
  const location = { owner, repo, kind: kind ?? 'root', ref: null, path: null };

  if ((kind === 'blob' || kind === 'blame' || kind === 'raw' || kind === 'tree') && rest.length) {
    // `<owner>/<repo>/blob/<ref>/<path…>` — a ref can itself contain slashes, so the path is
    // whatever follows the first segment that a `.dx` suffix or a directory boundary ends.
    location.ref = rest[0];
    location.path = rest.slice(1).join('/') || null;
    return location;
  }
  if (kind === 'commit' && rest.length) {
    location.ref = rest[0];
    return location;
  }
  if (kind === 'compare' && rest.length) {
    // `<owner>/<repo>/compare/<base>...<head>`, with two dots accepted as well as three.
    // The sides are usually branch names rather than commit ids, which the raw route serves
    // just as well. A side written `owner:branch` names *another* fork: this repository
    // cannot serve it, and resolving it against this one would show a different repository's
    // file as if it were this commit's, so it is left to fail into a notice.
    const range = rest.join('/');
    const [base, head] = range.split(range.includes('...') ? '...' : '..');
    location.base = base || null;
    location.head = head || null;
    return location;
  }
  if (kind === 'pull') {
    location.pull = rest[0] ?? null;
    return location;
  }
  return location;
}

/// The same-origin URL of a file at a ref.
///
/// github.com's own raw route is used rather than `raw.githubusercontent.com` for one
/// reason that matters: the request carries the reader's session, so a **private**
/// repository resolves exactly like a public one, with no token to configure and no extra
/// host permission to grant.
///
/// The route answers `302 → raw.githubusercontent.com`, and that response carries
/// `access-control-allow-origin: *` — so a content script may follow the redirect and read
/// the bytes with no host permission for the second origin either. That is why the
/// extension declares no permissions at all.
function rawUrl({ owner, repo }, ref, path) {
  // The ref is passed through unescaped: it comes from the page or the URL already in the
  // form github.com expects, and percent-encoding a branch name's slashes would break it.
  return `https://github.com/${owner}/${repo}/raw/${ref}/${path}`;
}

/// One resolution outcome.
///
/// `state` is `'document'` when content was recovered, and otherwise names what went wrong:
/// `'not-a-pointer'`, `'no-pack'`, `'not-in-pack'`, or `'stale'`. Every failing state
/// carries a `message` written for the person reading the page.
function resolved(state, fields = {}) {
  return { state, ...fields };
}

/// Resolve a pointer to its document.
///
/// `engine` is the wasm module (`pack_document`, `pack_paths`, `sha256_hex`), `fetchPack` is
/// an async function from a URL to `Uint8Array | null`, and `pointerText` is the file as
/// github.com serves it.
///
/// Every engine call is awaited, so the engine may be either the module itself — as under
/// node — or a proxy to one living in another context. On github.com it is the latter: the
/// page's content security policy forbids compiling WebAssembly in a content script, so the
/// extension runs the engine in its service worker and passes a proxy in here.
///
/// A recovered document is verified against the digest in the pointer before it is returned.
/// A mismatch is reported as `'stale'` rather than displayed, because showing content that
/// is not what the commit points at is worse than showing nothing: it would be wrong and
/// look right.
async function resolveDocument({ engine, fetchPack, location, ref, path, pointerText }) {
  const digest = digestIn(pointerText);
  if (!digest) {
    return resolved('not-a-pointer', {
      message: 'This file is not a dx pointer, so GitHub is already showing its content.',
    });
  }

  const packBytes = await fetchPack(rawUrl(location, ref, REPO_PACK));
  if (!packBytes || packBytes.length === 0) {
    return resolved('no-pack', {
      digest,
      message:
        `This document's content lives in ${REPO_PACK}, which is not committed at this ` +
        'revision. Run `dx sync .` and commit that file so the document travels with the ' +
        'repository.',
    });
  }

  let source;
  try {
    source = await engine.pack_document(packBytes, path);
  } catch (error) {
    let known = '';
    try {
      known = JSON.parse(await engine.pack_paths(packBytes)).join(', ');
    } catch {
      known = '';
    }
    return resolved('not-in-pack', {
      digest,
      message:
        `${REPO_PACK} does not carry ${path} (${error}). ` +
        (known ? `It holds: ${known}. ` : '') +
        'Run `dx sync .` to adopt the document and commit the pack again.',
    });
  }

  const actual = await engine.sha256_hex(new TextEncoder().encode(source));
  if (actual !== digest) {
    return resolved('stale', {
      digest,
      actual,
      source,
      message:
        `The pack at this revision holds a different version of ${path} than its pointer ` +
        `names (pointer ${digest.slice(0, 12)}…, pack ${actual.slice(0, 12)}…). Run ` +
        '`dx sync .` and commit the result.',
    });
  }

  return resolved('document', { digest, source, path });
}

/// A line-oriented diff of two texts, as rows a caller can render.
///
/// Each row is `{ kind, before, after, text }` where `kind` is `'same'`, `'added'`, or
/// `'removed'`. This exists because GitHub's own diff of a `.dx` file is a one-line digest
/// change — true, and useless. Diffing the resolved documents instead shows what a reviewer
/// came to see: which block changed and how.
///
/// The algorithm is a standard longest-common-subsequence walk, `O(n·m)` in the two line
/// counts. Documents are small enough that this is the right trade for exactness; a cap
/// keeps a pathological input from hanging a page.
function diffLines(before, after, limit = 4000) {
  const a = before === '' ? [] : before.replace(/\n$/, '').split('\n');
  const b = after === '' ? [] : after.replace(/\n$/, '').split('\n');

  if (a.length * b.length > limit * limit) {
    return [
      { kind: 'removed', before: 1, after: null, text: `${a.length} lines` },
      { kind: 'added', before: null, after: 1, text: `${b.length} lines` },
    ];
  }

  // lcs[i][j] = length of the longest common subsequence of a[i..] and b[j..].
  const lcs = Array.from({ length: a.length + 1 }, () => new Uint32Array(b.length + 1));
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      lcs[i][j] =
        a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const rows = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      rows.push({ kind: 'same', before: i + 1, after: j + 1, text: a[i] });
      i += 1;
      j += 1;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      rows.push({ kind: 'removed', before: i + 1, after: null, text: a[i] });
      i += 1;
    } else {
      rows.push({ kind: 'added', before: null, after: j + 1, text: b[j] });
      j += 1;
    }
  }
  while (i < a.length) {
    rows.push({ kind: 'removed', before: i + 1, after: null, text: a[i] });
    i += 1;
  }
  while (j < b.length) {
    rows.push({ kind: 'added', before: null, after: j + 1, text: b[j] });
    j += 1;
  }
  return rows;
}

/// Collapse runs of unchanged lines, keeping `context` of them either side of a change.
///
/// A diff of a long document is mostly lines that did not change; showing all of them buries
/// the ones that did. A collapsed run becomes a single `{ kind: 'skip', count }` row.
function collapse(rows, context = 3) {
  const keep = new Array(rows.length).fill(false);
  rows.forEach((row, index) => {
    if (row.kind === 'same') return;
    for (let at = index - context; at <= index + context; at += 1) {
      if (at >= 0 && at < rows.length) keep[at] = true;
    }
  });

  const out = [];
  let skipped = 0;
  rows.forEach((row, index) => {
    if (keep[index]) {
      if (skipped > 0) {
        out.push({ kind: 'skip', count: skipped });
        skipped = 0;
      }
      out.push(row);
    } else {
      skipped += 1;
    }
  });
  if (skipped > 0) out.push({ kind: 'skip', count: skipped });
  return out;
}

// The public surface, published on `globalThis` — the one global every host agrees on. A
// content script's `window` is the *page's* window in Firefox and the sandbox's own in
// Chrome, so a reader that says `window` works in one browser and finds nothing in the
// other; `content.js` reads it from here. See the note at the top of this file.
globalThis.dxResolve = {
  REPO_PACK,
  collapse,
  diffLines,
  digestIn,
  isDocumentPath,
  locate,
  rawUrl,
  resolveDocument,
  resolved,
};
