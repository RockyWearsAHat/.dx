# Documents on GitHub

A committed `.dx` file is one line:

```text
~ dx1 c939d5becfb64b14193566ffed7ccf8217c90bf5c90e6ba2a5ce8bf87903c823
```

The document is in `.doc/repo.dxcp`, committed beside it. Locally that is invisible — every
read goes through the resolver, and `dx git-setup` teaches `git diff`, `git show`, and
`git log -p` to render documents. On github.com, it is the pointer that shows.

This is how to fix that, and why it takes what it takes.

## github.com cannot be extended server-side

Worth stating plainly, because it is the first thing anyone asks:

- **`github/markup` picks from a fixed, first-party list.** Markdown, AsciiDoc, reStructuredText,
  notebooks, CSV, PDF, SVG, GeoJSON, STL. A repository cannot add to it.
- **A GitHub App cannot change a blob page.** Apps get webhooks, checks, statuses, comments, and
  annotations — not the file viewer.
- **"Custom file viewer" is an open, unimplemented request.**
  [community/discussions/15132](https://github.com/orgs/community/discussions/15132). Bitbucket
  has file-viewer add-ons; GitHub does not.
- **`.gitattributes` does not help.** Linguist overrides affect syntax highlighting and whether a
  diff is collapsed. `diff.dx.textconv` is a *local* git driver, and github.com does not run it.

So nothing installed on the repository can make the page show the document. What *can* is code
running in the reader's browser — which is what `editor/github` is.

## The extension

`editor/github` is a browser extension for Chrome and Firefox. On a github.com page it finds a
`.dx` pointer, fetches the repository's committed pack, resolves the pointer with the same Rust
engine the CLI and the editor use, and puts the real document in its place.

The extension is a shim. The engine it calls is `dx serve` — the local rendering service, which
holds what it decodes — with the bundled wasm as a fallback for a reader who has not started it.
See [The engine is a service](#the-engine-is-a-service) below.

| Page | What you see |
|------|--------------|
| A `.dx` file | The rendered document — headings, lists, tables, diagrams, captured output |
| A blame view | The rendered document. A pointer's blame is one line naming the commit that last changed the digest, so the document is the useful thing to show |
| A pull request, commit, or compare view | A **document diff**: which blocks changed, line by line, instead of a one-line digest change |
| Anything else | Nothing. The extension is inert outside `.dx`. |

### The engine runs in the service worker, not in the page

This is the one piece of the design that is not obvious, and it is forced:

**A content script shares the host page's WebAssembly policy.** github.com serves `script-src
github.githubassets.com` with no `'wasm-unsafe-eval'`, so every `WebAssembly.compile` inside a
content script on github.com fails with a bare `CompileError` — however the bytes were obtained,
and even though the same module compiles fine in the same extension on a page with a permissive
policy. So the engine cannot live in the page.

It lives in the extension's service worker (`engine.js`), which is governed by this extension's
own `content_security_policy.extension_pages` — where `'wasm-unsafe-eval'` is granted.

**The fetching stays in the content script**, because only a page-context request carries the
reader's github.com session, which is what makes private repositories work with no permissions.

So the two halves split like this, and talk over one message:

```text
content.js   locate the pointer, fetch the pointer text and the pack  (session, no engine)
     │   { kind: 'dx-engine', call: 'pack_document', args: [{ packRef: '…' }, 'notes.dx'] }
     ▼
engine.js    resolve, verify, render                                  (engine, no fetching)
```

`engine.js` answers only an allowlist of calls, so a page can never name an arbitrary export.

### The engine is a service

A call names a pack; it never carries one. The engine answers `{ needPack: url }` if it is not
holding that pack, the content script sends the bytes once, and every call after it is answered
from memory.

That indirection is the whole performance story. Before it, the pack was base64'd into *every
call*: a pull request touching six documents downloaded, transferred, and decompressed the
entire repository twelve times — once per file per side — and kept none of it, because the only
place to keep it was a service worker the browser shuts down after seconds of idleness.

So the engine moved to a process that stays alive:

| | Where it runs | What it remembers |
|---|---|---|
| **`dx serve`** — preferred | A daemon on this machine, loopback only | Every pack it has been given, for as long as it runs |
| **Bundled wasm** — fallback | The extension's background context | Only until the browser shuts that context down |

`engine.js` probes `127.0.0.1` on the ports `dx serve` binds (`daemon::PORTS`, compared against
the extension's list by a test in `extension.rs` — an extension cannot read a file to be told
which port was chosen, so both sides hold the list). It finds the daemon once per background
context, not once per call, and a daemon that stops mid-read falls back to the wasm rather than
failing: **prefer the daemon, never require it.** The reader sees the document either way.

The daemon reads no files, writes none, and runs nothing — it is a pure function from bytes the
caller handed it to a rendering of them, and `dx run` is not reachable from it.

It does, however, *hold* repository content the browser gave it, which can be private. So it
does not answer everyone who finds the port:

| Check | What it stops |
|---|---|
| `Host` must be loopback | **DNS rebinding** — a site whose name re-resolves to `127.0.0.1` is same-origin as far as the browser is concerned, so no cross-origin rule applies at all. It cannot forge `Host`, which still names the attacker. This is the load-bearing check |
| `Origin`, when present, must be the extension | An ordinary cross-origin `fetch`. A `POST` of `text/plain` is a *simple* request and arrives with **no preflight**, so this is refused in the handler, not by a CORS header |
| No `Origin` at all | Nothing — that is a native client (`curl`, an editor, the phone app). No page can make a browser omit the header |

github.com itself is not on the list: the extension reaches the daemon from its own background
context, so a request claiming to be the page is not the extension.
`rust/doc-cli/src/daemon/` has the protocol.

`resolve.js` is unchanged by any of this: it takes the engine as a collaborator and awaits every
call, so it works the same whether the engine is the module itself (as under node, where it is
tested) or a proxy to another context (as on github.com).

### How a reader gets it

`dx setup` gives the extension to every browser on the machine, and the route differs per
family because the browsers differ — not because dx does.

| Family | Route | Clicks | Why not fewer |
|---|---|---|---|
| **Firefox** | `dx` writes `distribution/policies.json`; Firefox installs the add-on at its next start | none | — |
| **Safari** | inside `DX.app`, as an app extension | one | Enabling an extension is a setting only the reader can change |
| **Chromium** | the Chrome Web Store listing | one, per browser | Google closed every local route; see below |

Two of those need a signature that only the vendor can issue, so `dx` checks rather than
assumes. Release and Beta Firefox refuse an unsigned add-on whatever the policy says, so the
policy is only written when a Mozilla-signed XPI is actually present in the application;
otherwise Firefox falls back to loading by hand and the report says so. The same holds for
Safari: the step "tick dx in Settings" is only offered when the app really carries the
extension. `extension::channel` is the single place that decides, and both cases are tests.

**Chromium cannot be installed into silently, and that is measured.** On macOS a Chromium
browser force-installs a non-Web-Store extension only when the machine is MDM-managed, MCX
domain-joined, or enrolled in Chrome Enterprise Core; installing from a local `.crx` has been
refused on macOS since Chrome 44. A root-owned
`/Library/Managed Preferences/com.google.Chrome.plist` naming an `ExtensionInstallForcelist`
entry *is* read — Chrome shows "managed by your organization" — and Chrome then never requests
the update manifest at all: zero requests over four minutes, against a logging server on the
port the policy named. Chrome tests enrollment, not the appearance of it. So one click per
Chromium browser is the floor, for anyone.

Publishing is one line here: set `CHROME_WEB_STORE` in `rust/doc-cli/src/extension.rs` to the
listing URL and every Chromium browser switches from developer mode to a store install.
`packaging/README.md` is the runbook.

### Build it from source

```bash
./editor/build.sh
```

That compiles `doc-wasm` for both editor surfaces at once — wasm-bindgen's `no-modules` target
into `editor/github/wasm/` for this extension, and `nodejs` into `editor/vscode/wasm/` for the
editor — because two surfaces built at different moments are two renderers. Then:

- **Chrome** — `chrome://extensions` → Developer mode → Load unpacked → `editor/github`
- **Firefox** — `about:debugging` → This Firefox → Load Temporary Add-on → `editor/github/manifest.json`

Requires `wasm-pack`, with the rustup toolchain ahead of any Homebrew `rustc` on `PATH`
(`PATH="$HOME/.cargo/bin:$PATH"`) and the `wasm32-unknown-unknown` target.

The icon is rendered from `packaging/icon.svg` by `packaging/build-icons.py` and committed
under `editor/github/icons/`; both stores reject a submission without a 128px one.

### It asks for nothing over any site you visit

`manifest.json` declares `"permissions": []`, and one host permission: `http://127.0.0.1/*`,
which is where `dx serve` is. It holds no permission over github.com or any other site. The
pack is fetched from
`github.com/<owner>/<repo>/raw/<ref>/.doc/repo.dxcp` — the page's own origin — which has two
consequences worth understanding:

- **Private repositories work.** The request carries the session the reader is already using.
  There is no token to configure and no server in the middle.
- **No second host permission is needed.** That route answers `302 →
  raw.githubusercontent.com`, and that response sends `access-control-allow-origin: *`, so
  following the redirect and reading the bytes is allowed.

### It never invents content

The rule the whole system rests on is that a reader never sees a pointer where a document
belongs, and never sees nothing where content exists. The extension keeps it by verifying before
it displays: the recovered source is hashed and compared with the digest in the pointer.

| What happened | What the page says |
|---------------|--------------------|
| Resolved and verified | The document |
| `.doc/repo.dxcp` is not committed at this revision | Which file is missing, and to run `dx sync .` and commit it |
| The pack does not hold this path | That, plus the paths it *does* hold |
| The pack and the pointer disagree | Both digests, and to run `dx sync .` — it will not show content the commit does not point at |
| The file is not a pointer | Nothing: GitHub is already showing the content |

A rendered document sits in a shadow root styled by `doc-core`'s own stylesheet, so it looks the
same here as in the editor and in `dx png`, and cannot be restyled by the page around it. Light
and dark follow GitHub's own `data-color-mode`.

### How it is tested, and what a terminal cannot reach

The resolution logic, the pointer grammar, the URL handling, the document diff, and the manifest
are covered by tests that run without a browser:

```bash
./editor/github/test/fixture.sh          # a real pack, written by a real `dx sync`
node --test "editor/github/test/*.test.mjs"
```

The fixture is deliberately produced by the `dx` binary rather than assembled in JavaScript: a
fixture built by the code under test would hide exactly the disagreements worth catching.

The DOM wiring — which container GitHub renders a blob into, which attributes carry the base and
head SHAs — cannot be exercised from a terminal, so it is checked by loading the extension in a
real Chrome against a real pushed repository. Doing that for the first time found ten defects that
every terminal test passed straight through, including two that made the extension render nothing
at all, on any page: `resolve.js` was missing from the manifest's injection list, and the engine
was being compiled in the page. **The manifest, the CSP, and the message protocol are now asserted
by tests**, because those were pure declaration bugs a browser was not needed to catch. The
container and revision selectors are not, and remain the part to check by hand after a GitHub
redesign.

When checking by hand, measure — do not just look for the absence of a pointer. One of the ten was
a blame view that had replaced the pointer with a document clipped to the height of a single line:
nothing was wrong with the resolution, and a check that asked "is there a document element, and is
the pointer gone?" passed it. Ask instead how tall the rendering is, and whether any ancestor's
`overflow` is clipping it.

`content.js` avoids depending on GitHub's build hashes. Its CSS-module class names look like
`CodeBlob-module__codeBlobInner__tfjuQ`: the name comes from the stylesheet and is stable, the hash
after it is not, so only the prefix is ever matched
(`[class*="CodeBlob-module__codeBlobInner"]`). That region is deliberately the level replaced —
everything inside it is height-pinned by GitHub's virtual scroller and clipped, and a document
placed further in is painted as one line however tall it measures. Older layouts, which state their
container outright (`.blob-wrapper`, `.js-file-line-container`), are still read as fallbacks, and
where a cursor overlay exists the file is painted twice, so both copies have to go or the pointer
stays on screen beside the document.

GitHub is also midway through a rewrite, with the two layouts live on different pages at once: a
pull request still states each changed path in `data-tagsearch-path`, while a commit page states it
in `aria-label="Diff for: …"` and carries none of the older markup. Both are read, and the two
revisions being compared are recovered from whichever of GitHub's own diff routes the page
happens to expose. If a page stops rendering after a redesign, that adapter is where to look;
nothing else needs to change.

To check it yourself: the extension needs to be loaded unpacked (`chrome://extensions` →
Developer mode → Load unpacked), then open a `.dx` file, its blame view, and a pull request that
touches it. What to look for is that no page anywhere shows a raw `~ dx1 …` line.

## The one thing to keep committed

Everything above depends on `.doc/repo.dxcp` being in the commit. `dx git-setup` writes the
`.gitignore` lines that keep the rebuildable index out and the pack in, and `dx sync` reports
anything it cannot resolve rather than discarding it. A repository whose pointers are committed
without their pack has documents nobody — no reader, no agent, no extension — can recover.

If you want that guarded in CI, a workflow that runs `dx sync .` and fails when the pack changes
is the check to write: a dirty pack after a sync means a pointer was committed without its
content.
