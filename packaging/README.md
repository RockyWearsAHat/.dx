# Shipping dx

One app, installed once, and this device understands `.dx` everywhere it can — including
Finder, where double-clicking one opens the page. This directory builds that app and the store
uploads that go with it.

```bash
./packaging/build-icons.py            # only when packaging/icon.svg changes
./packaging/build-app.sh --safari     # → packaging/build/DX.app
./packaging/build-stores.sh           # → dx-chrome.zip, dx-firefox.xpi
```

Nothing here is inside the shipped binary. Packaging is a release concern, and a document
renderer has no business carrying an archive writer or a rasterizer around on a user's machine.

## The application is the viewer, and the installer

`DX.app` has two roles and is deliberately **one application**, because a viewer shipped apart
from the engine it renders through is a second thing to install and a second thing to go stale.
Which role a launch is, is not a mode anyone chooses — macOS hands an application the documents
it was asked to open:

| The launch carries | What happens |
|---|---|
| a `.dx` document | a window opens showing the rendered page |
| nothing | `dx setup` runs, and one dialog says what happened |

The viewer is `packaging/app` — seven Swift files, AppKit and `WKWebView`, built into
`Contents/MacOS/dx-app`. It renders nothing itself: it runs `Contents/MacOS/dx render` — the
binary in its *own* bundle, never one found on `PATH` — and shows the page that comes back. The
window is a title bar and the page, because the page already carries the whole look. The page is
also the editing surface (`editor/surface`, copied in at build time): clicking a block opens its
field, every save lands on the bundled `dx`'s own `edit` operations — the same calls every other
surface makes — and the run control beside a code block executes that one block through
`dx run --only … --approve`, the click being the review. A plain read stays a read: rendering
executes nothing and writes nothing, the document has no script (`render::escape` allows none), the
page is loaded under `script-src 'none'` so the *document's* scripts could never run anyway,
and the editing surface runs in its own `WKContentWorld` the page cannot reach (`Editor.swift`
explains the arrangement). Opening a pointer creates no index.

Finder routes a double-click here because the bundle declares `CFBundleDocumentTypes` and
exports the type `tools.dx.document` (`UTExportedTypeDeclarations`), conforming to
`public.plain-text` — a `.dx` *is* text, and every text tool on the Mac should keep working on
one. A declaration alone is not enough: LaunchServices has to be told the application exists,
which is `dx setup`'s job below.

## What one install actually does

Double-clicking `DX.app` runs `dx setup`, which:

| | |
|---|---|
| puts `dx` on `PATH` | `~/.local/bin/dx` |
| registers the MCP server | every assistant found — Claude, Codex, Cursor, VS Code, Windsurf, Gemini |
| starts the rendering service at login | a `launchd` agent, per-user, no administrator |
| installs the application and registers the type | `/Applications/DX.app`, then `lsregister -f` |
| reports where the extension is | the app bundle's copy, or what `dx browser --from` wrote |
| configures Firefox | writes its `policies.json`; every other browser is told the one step it reserves — see below |

The application is **copied into `/Applications` before it is registered** (`~/Applications` on
an account that cannot write there), for the same reason the binary is copied onto `PATH`:
registering a bundle where it happens to be sitting binds every double-click on the machine to
a download folder or a build directory, and the next build deletes it.

`dx setup --uninstall` reverses the parts that reach outside `dx`'s own directory: the launch
agent and any policy file written into another application's bundle. It leaves `DX.app` alone
and says so — dragging an application to the Trash is a thing every Mac user already knows how
to do, and it is what un-registers the type.

## The routes, and why they differ

This is the part that cannot be designed around, so it is worth stating plainly. It was
measured, not assumed.

| Browser | Route | Clicks | Why |
|---|---|---|---|
| **Firefox** family | `policies.json` force-install | **none** | Firefox lets a program on the machine install an add-on. Needs a Mozilla signature (free). |
| **Safari** | inside `DX.app` | one | A Safari Web Extension *is* an app extension. Enabling it is a setting only the reader can change. |
| **Chromium** family | Chrome Web Store | one, per browser | Google closed every local route. See below. |

### Chromium cannot be done silently, and that is Google's decision

On macOS a Chromium browser force-installs a non-Web-Store extension **only** when the machine
is MDM-managed, MCX domain-joined, or enrolled in Chrome Enterprise Core. Installing from a
local `.crx` through the external-preferences file has been refused on macOS since Chrome 44.

Both were tested here rather than taken from documentation. A root-owned
`/Library/Managed Preferences/com.google.Chrome.plist` naming an `ExtensionInstallForcelist`
entry with a loopback update manifest **is** read by Chrome — the browser shows "managed by
your organization" — and Chrome then never requests the update manifest at all. Zero requests,
measured over four minutes against a logging server on the port the policy named. Chrome tests
enrollment, not the appearance of management.

So: one click per Chromium browser, from a store listing. That is the floor for everyone, not
just for us.

## Publishing

### Chrome Web Store — $5 once, covers six browsers

Chrome, Edge, Brave, Vivaldi, Opera and Arc all install from this one listing.

1. Register at <https://chrome.google.com/webstore/devconsole>.
2. Upload `packaging/build/dx-chrome.zip`.
3. When it is published, set `CHROME_WEB_STORE` in `rust/doc-cli/src/extension.rs` to the
   listing URL.

That last step is one line, and it is the whole switch: `extension::channel` moves every
Chromium browser from developer mode to a one-click install, `dx browser` and `dx doctor`
change what they say, and a test asserts both halves so it cannot be half-done.

### addons.mozilla.org — free, and the only way Firefox is zero-click

Release and Beta Firefox refuse an unsigned add-on. No policy overrides this; the
`xpinstall.signatures.required` preference is ignored outside ESR, Developer Edition and
Nightly.

1. Submit `packaging/build/dx-firefox.xpi` at
   <https://addons.mozilla.org/developers/addon/submit/distribution>, choosing **"On your own
   site"** — that is an *unlisted* submission, which is signed for self-distribution and never
   appears in the public directory.
2. Put the signed file at `packaging/signed/dx-firefox.xpi`.
3. Rebuild the app.

`dx` checks for that file rather than assuming it: without it, Firefox falls back to loading
by hand and says so, instead of writing a policy that silently does nothing.

### Apple — $99/year, and only for distribution

The app is ad-hoc signed and runs on the machine that built it. Handing it to anyone else
needs a Developer ID, because macOS refuses to open an unsigned app from another machine and
Safari will not load an ad-hoc app extension for an ordinary reader.

```bash
export DX_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
./packaging/build-app.sh --safari
xcrun notarytool submit packaging/build/DX.app --keychain-profile dx --wait
xcrun stapler staple packaging/build/DX.app
```

Without it, a reader can still install: right-click → Open, once. That is the standard
unsigned-app path and it is honest to offer it — but it is a worse first impression than the
$99 avoids.

## The icon

`packaging/icon.svg` is the source; `build-icons.py` rasterizes it to the four sizes and
commits them under `editor/github/icons/`, because `editor/github` is the source every shipped
extension is built out of and a build cannot ship what the tree does not have. The same script
with `--icns <file>` writes the macOS icon the application and every `.dx` file on the disk
wear; that one is *not* committed — `build-app.sh` draws it on the way past, so a document's
icon cannot drift from the extension's.

Two things about that script are deliberate. It refuses any shape that is not a filled
`<rect>`, so the icon cannot quietly lose a piece the source clearly has. And it asserts the
result has ink in it — headless Chrome on this machine writes a **fully transparent** PNG for
any page, including a plain red one, and every check based on file size or dimensions waves
that through. The icon was blank once and looked fine by every measure except looking at it.
