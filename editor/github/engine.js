// The engine the page talks to: the local dx daemon when it is running, the bundled wasm
// when it is not.
//
// # Why the engine is not in the page
// A content script shares the page's WebAssembly policy. github.com serves
// `script-src github.githubassets.com` with no `'wasm-unsafe-eval'`, so **every**
// `WebAssembly.compile` in a content script on github.com fails with a bare `CompileError`,
// however the bytes were obtained. This file runs in the extension's own background context,
// governed by this extension's `content_security_policy.extension_pages`, where
// `'wasm-unsafe-eval'` is granted.
//
// # Why there are two engines and not one
// They are the same engine — `doc-core` — reached two ways, and the difference that matters
// is not speed but *memory*.
//
// This background context is a service worker: the browser shuts it down after seconds of
// idleness and starts it again on the next page. Nothing it decodes survives, so a reader
// moving through a repository paid to download and decompress the same `.doc/repo.dxcp` on
// every single page, and a pull request touching six documents paid for it twelve times —
// once per file per side. `dx serve` is a process that stays alive, so it decodes a pack once
// and answers from memory for as long as the reader is reading.
//
// The wasm stays as the fallback, because a reader who has not started the daemon should see
// documents rather than an explanation. Prefer the daemon; never require it.
//
// # The protocol
// One message shape, so there is exactly one thing to keep in step with the content script:
//
//   → { kind: 'dx-engine', call: 'pack_document', args: [{ packRef: '<url>' }, 'notes.dx'] }
//   ← { value: '::heading level=1 …' }
//   ← { needPack: '<url>' }        the engine does not hold that pack; send it and retry
//   ← { error: 'no such path in pack' }
//
//   → { kind: 'dx-pack', url: '<url>', bytes: '<base64>' }
//   ← { stored: true }
//
// A pack is named, never inlined. That is the whole point: the bytes cross this boundary once
// per repository revision instead of once per call, and both engines hold them afterwards.

/* global wasm_bindgen */

// # Two browsers, two kinds of background
// Chrome runs this file as a **service worker**, where `importScripts` is how a worker pulls
// in another script. Firefox has no MV3 service worker: it runs background code as an
// **event page**, where `importScripts` does not exist and scripts are listed in the manifest
// instead — so the Firefox manifest names the glue ahead of this file. Both arrive here with
// `wasm_bindgen` defined; the manifests are written per browser by `dx browser`.
if (typeof importScripts === 'function') importScripts('wasm/doc_wasm.js');

/// The ports `dx serve` binds, in the order it tries them.
///
/// Kept in step with `daemon::PORTS` in `rust/doc-cli/src/daemon/mod.rs` by
/// `the_extension_probes_exactly_the_ports_the_daemon_binds` — an extension cannot read a
/// file to discover the port, so both sides hold the list and a test compares them.
const PORTS = [7333, 7334, 7335, 7336];

/// How long to wait for the daemon before deciding it is not there.
///
/// It is a loopback service, so anything slower than this is not a daemon that is running.
const PROBE_MS = 400;

/// The calls the content script is allowed to make.
///
/// An allowlist rather than a lookup on the module: a message from a page must never be able
/// to name an arbitrary export, and this also states the whole surface the page side depends
/// on in one place. It is the same set the daemon answers (`daemon::engine::CALLS`).
const ALLOWED = new Set([
  'pack_document',
  'pack_paths',
  'sha256_hex',
  'render_html',
  'references',
  'file_references',
  'stylesheet',
]);

/// Decode base64 to bytes.
function unbase64(text) {
  return Uint8Array.from(atob(text), (character) => character.charCodeAt(0));
}

// ---------------------------------------------------------------------------------------
// The daemon

/// The in-flight or settled search for the daemon, or `null` when it has not been looked for.
///
/// The **promise** is memoized, not its result. Memoizing the result is a race: the check
/// happens synchronously, before the first `await`, so every call that starts while the first
/// probe is still in flight finds nothing cached and walks all four ports itself. Two
/// concurrent calls — which is exactly what rendering one document does, `stylesheet()` and
/// `render_html()` together — probed twice; three with no daemon running probed twelve times,
/// each with its own timeout.
let daemonSearch = null;

/// Find the daemon among [`PORTS`], or `false` if none of them answers.
///
/// One search per worker lifetime, shared by every caller that asks while it is running. A
/// worker that starts before the daemon does will use the wasm until the browser next
/// restarts it, which is a matter of seconds of idleness.
function findDaemon() {
  if (!daemonSearch) {
    daemonSearch = (async () => {
      for (const port of PORTS) {
        const base = `http://127.0.0.1:${port}`;
        try {
          const answer = await fetch(`${base}/health`, { signal: AbortSignal.timeout(PROBE_MS) });
          if (answer.ok && (await answer.json())?.name === 'dx') return base;
        } catch {
          // Not this port; try the next.
        }
      }
      return false;
    })();
  }
  return daemonSearch;
}

/// Forget where the daemon was, so the next call looks again.
///
/// Used when a daemon that answered the probe fails a call. Deciding it is gone *for the
/// worker's lifetime* on one bad answer would strand a reader on the fallback long after the
/// daemon came back; re-probing costs one round trip and settles it.
function forgetDaemon() {
  daemonSearch = null;
}

/// Run one call against the daemon at `base`.
///
/// Returns `{ value }`, `{ needPack }`, or `{ error }` — the same three shapes the content
/// script sees, so neither this file's caller nor `resolve.js` knows which engine answered.
async function daemonCall(base, message) {
  const answer = await fetch(`${base}/engine`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    // `{ packRef }` on the wire becomes `{ pack }`, which is what the daemon's argument
    // grammar calls it. Everything else is passed through untouched.
    body: JSON.stringify({
      call: message.call,
      args: (message.args ?? []).map((argument) =>
        argument && typeof argument === 'object' && argument.packRef
          ? { pack: argument.packRef }
          : argument,
      ),
    }),
  });
  const body = await answer.json();
  if (answer.status === 409) return { needPack: body.needPack };
  if (!answer.ok) return { error: body.error ?? `the dx daemon answered ${answer.status}` };
  return { value: body.value };
}

/// Hand a pack's bytes to the daemon, once.
async function daemonStore(base, url, bytes) {
  const answer = await fetch(`${base}/pack`, {
    method: 'POST',
    headers: { 'x-dx-pack': url },
    body: bytes,
  });
  if (!answer.ok) throw new Error((await answer.json())?.error ?? 'the dx daemon refused the pack');
}

// ---------------------------------------------------------------------------------------
// The bundled wasm

/// The wasm engine, started once per worker lifetime and shared by every call.
let enginePromise = null;

/// The packs this worker is holding, by the URL they were fetched from.
///
/// The fallback for when the daemon is not running, and the reason a daemon that stops
/// mid-read does not cost the reader a re-fetch. Capped like the daemon's own cache: a `Map`
/// that only grows would accumulate every pack of a long browsing session in a context that
/// cannot be asked to release it.
const heldPacks = new Map();

/// How many packs the fallback holds before dropping the least recently used, matching the
/// daemon's own capacity.
const HELD_LIMIT = 12;

/// Hold `bytes` under `url`, dropping the oldest entry once [`HELD_LIMIT`] is exceeded.
function hold(url, bytes) {
  heldPacks.delete(url); // Re-inserting moves it to the end, which is what makes this LRU.
  heldPacks.set(url, bytes);
  while (heldPacks.size > HELD_LIMIT) {
    heldPacks.delete(heldPacks.keys().next().value);
  }
}

/// The pack held under `url`, marking it most recently used.
function heldPack(url) {
  const bytes = heldPacks.get(url);
  if (bytes) hold(url, bytes);
  return bytes;
}

/// Load the wasm engine, resolving to the module namespace.
function wasmEngine() {
  if (!enginePromise) {
    enginePromise = wasm_bindgen({
      module_or_path: chrome.runtime.getURL('wasm/doc_wasm_bg.wasm'),
    }).then(() => wasm_bindgen);
  }
  return enginePromise;
}

/// Run one call against the bundled wasm, resolving pack references out of [`heldPacks`].
async function wasmCall(message) {
  const args = [];
  for (const argument of message.args ?? []) {
    if (argument && typeof argument === 'object' && argument.packRef) {
      const held = heldPack(argument.packRef);
      if (!held) return { needPack: argument.packRef };
      args.push(held);
    } else if (argument && typeof argument === 'object' && typeof argument.text === 'string') {
      // The daemon's byte encoding, so one call shape serves both engines.
      args.push(new TextEncoder().encode(argument.text));
    } else {
      args.push(argument);
    }
  }
  const module = await wasmEngine();
  return { value: module[message.call](...args) };
}

// ---------------------------------------------------------------------------------------
// The one door the content script knocks on

/// Run one call on whichever engine is reachable.
///
/// A daemon that stops between the probe and the call falls back rather than failing: the
/// reader gets the document either way, which is the rule the whole system keeps.
async function perform(message) {
  if (!ALLOWED.has(message.call)) throw new Error(`unknown engine call: ${message.call}`);
  const base = await findDaemon();
  if (base) {
    try {
      return await daemonCall(base, message);
    } catch {
      forgetDaemon(); // It was there at the probe and is not now; look again next time.
    }
  }
  return wasmCall(message);
}

/// Take a pack's bytes and give them to whichever engine will be answering about them.
///
/// Held in this worker as well as sent to the daemon, so a daemon that stops mid-read leaves
/// the fallback able to answer without asking the page to fetch the pack again.
async function store(message) {
  const bytes = unbase64(message.bytes);
  hold(message.url, bytes);
  const base = await findDaemon();
  if (base) {
    try {
      // Storing under a key that is already held replaces it, which is what makes this the
      // way a stale pack is refreshed as well as the way a new one arrives.
      await daemonStore(base, message.url, bytes);
    } catch {
      forgetDaemon();
    }
  }
  return { stored: true };
}

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  const handler =
    message?.kind === 'dx-engine' ? perform : message?.kind === 'dx-pack' ? store : null;
  if (!handler) return false;
  handler(message).then(
    (reply) => respond(reply),
    // The message text is what the reader is eventually shown, so it is kept, not swallowed.
    (error) => respond({ error: String(error?.message ?? error) }),
  );
  return true; // The response is asynchronous.
});
