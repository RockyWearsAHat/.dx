#!/usr/bin/env bash
#
# Build DX.app — the one thing a person installs on a Mac, and what opens a `.dx`.
#
# The bundle is two things at once, and deliberately one application. It is the *viewer*: a
# double-clicked `.dx` opens in a window here, rendered by the `dx` inside this same bundle,
# which is the only way the window and the engine cannot drift apart. And it is the delivery
# vehicle for everything that cannot be a lone binary: Safari's extension has to live inside an
# application, Firefox's add-on has to be a file on disk for the policy to name, and a person
# who has never opened a terminal has to be able to double-click something.
#
# So the app carries the `dx` binary, the extension in every shape, and one Swift executable
# (packaging/app) that reads a document when macOS hands it one and runs `dx setup` when it is
# launched with nothing.
#
#   ./packaging/build-app.sh              build with whatever is available
#   ./packaging/build-app.sh --safari     also build the Safari extension (needs Xcode)
#
# Signing: the bundle is ad-hoc signed, which is enough for it to run on the machine that
# built it. Distributing it needs a Developer ID and notarization — see packaging/packaging.dx,
# which has the two commands and nothing else to work out.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/packaging/build"
app="$out/DX.app"
contents="$app/Contents"
# The version lives in the workspace manifest; every crate inherits it, so reading a crate's
# own Cargo.toml finds `version.workspace = true` and nothing else.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/rust/Cargo.toml" | head -1)"
with_safari=0
[[ "${1:-}" == "--safari" ]] && with_safari=1

echo "DX.app $version"

# ---------------------------------------------------------------------------------------
# The binary. Built release, because this is the copy a person actually runs.
# ---------------------------------------------------------------------------------------
echo "  building dx…"
(cd "$root/rust" && cargo build --release -p doc-cli >/dev/null)

rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Resources"
cp "$root/rust/target/release/dx" "$contents/MacOS/dx"

# ---------------------------------------------------------------------------------------
# The extension, in every shape, built out of `editor/github` by the binary that will read
# it — so what ships can never disagree with what the engine renders.
#
# The application carries it because a browser needs the files on disk at a path that does not
# move; the *binary* does not, which is why `dx browser` needs `--from` here and why a `dx`
# installed on its own is ~0.4 MB smaller. `extension::installed_dir` finds this directory when
# `dx` is running from inside the bundle.
# ---------------------------------------------------------------------------------------
echo "  building the extension…"
"$contents/MacOS/dx" browser --from "$root/editor/github" \
  --dir "$contents/Resources/extension" >/dev/null

# The Mozilla-signed add-on, when there is one. `dx` names this file in the Firefox policy
# and installs with no clicks; without it Firefox falls back to loading by hand, which the
# binary works out for itself (see extension::channel).
if [[ -f "$root/packaging/signed/dx-firefox.xpi" ]]; then
  cp "$root/packaging/signed/dx-firefox.xpi" "$contents/Resources/dx-firefox.xpi"
  echo "  bundled the signed Firefox add-on"
else
  echo "  no signed Firefox add-on — see packaging/packaging.dx (free, one step)"
fi

# ---------------------------------------------------------------------------------------
# The editing surface: the same `editor/surface` the VS Code webview loads.
#
# Copied, never rewritten. What clicking a paragraph does is one behavior, and a Mac and an
# editor disagreeing about it would be two products wearing one name. `Editor.swift` reads
# these files out of the bundle at launch.
#
# `doc_wasm.js` is the geometry engine's glue (`editor/build.sh` copies it into
# `editor/surface` from the `no-modules` build); `doc_wasm_bg.wasm` is its bytes, read fresh
# per page load rather than embedded in a script — the same reason the VS Code extension
# ships them as a separate file instead of inlining them.
# ---------------------------------------------------------------------------------------
echo "  bundling the editing surface…"
mkdir -p "$contents/Resources/surface"
cp "$root/editor/surface/edit.js" "$root/editor/surface/edit.css" \
  "$root/editor/surface/doc_wasm.js" "$root/editor/github/wasm/doc_wasm_bg.wasm" \
  "$contents/Resources/surface/"

# ---------------------------------------------------------------------------------------
# The icon, rasterized from the one source every surface's icon comes from. Finder puts it on
# the application *and* on every .dx file on the disk, so it is what a document looks like.
# ---------------------------------------------------------------------------------------
echo "  drawing the icon…"
(cd "$root/rust" && cargo build --release -p doc-shot --bin build-icons >/dev/null)
"$root/rust/target/release/build-icons" --icns "$contents/Resources/dx.icns" >/dev/null

# ---------------------------------------------------------------------------------------
# The application itself: AppKit + WKWebView, in packaging/app.
#
# It is one executable with two roles, chosen by what macOS hands the launch — a document to
# read, or nothing, which is the `dx setup` this app has always run. See packaging/app and
# packaging/packaging.dx for why that is one application rather than two.
#
# It is named `dx-app` and not `DX`, which cost a build to learn: a Mac's filesystem is
# case-insensitive, so `MacOS/DX` and `MacOS/dx` are one file, and writing the launcher
# silently replaced the binary it was supposed to run. The app looked complete and was 812K
# of extension wrapped around a shell script.
# ---------------------------------------------------------------------------------------
if ! command -v swiftc >/dev/null 2>&1; then
  echo "  swiftc is missing — install Xcode or the Command Line Tools" >&2
  echo "  (xcode-select --install). The viewer is the application; there is no build without it." >&2
  exit 1
fi

echo "  building the viewer…"
swiftc -O -target "$(uname -m)-apple-macos11.0" \
  -o "$contents/MacOS/dx-app" "$root"/packaging/app/*.swift

# ---------------------------------------------------------------------------------------
# Info.plist.
#
# `CFBundleDocumentTypes` plus `UTExportedTypeDeclarations` are what make a double-click in
# Finder arrive here: the first says this application opens that type, the second is dx
# declaring the type in the first place, since nobody else on the machine has heard of it. A
# .dx *is* plain text — a pointer line or the document itself — so the type conforms to
# `public.plain-text`, and every text tool on the Mac keeps working on one.
#
# LSUIElement is 0: this app has windows and should appear while it runs. It is not a
# background agent — that is what the launch agent `dx setup` installs is for, and conflating
# the two would put a permanent icon in the Dock for a daemon.
# ---------------------------------------------------------------------------------------
cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>dx</string>
  <key>CFBundleDisplayName</key><string>dx</string>
  <key>CFBundleIdentifier</key><string>tools.dx.app</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>dx-app</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>dx</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHumanReadableCopyright</key><string>dx — a notepad for code</string>

  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>dx document</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Owner</string>
      <key>CFBundleTypeIconFile</key><string>dx</string>
      <key>LSItemContentTypes</key>
      <array><string>tools.dx.document</string></array>
    </dict>
  </array>

  <key>UTExportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key><string>tools.dx.document</string>
      <key>UTTypeDescription</key><string>dx document</string>
      <key>UTTypeIconFile</key><string>dx</string>
      <key>UTTypeConformsTo</key>
      <array><string>public.plain-text</string></array>
      <key>UTTypeTagSpecification</key>
      <dict>
        <key>public.filename-extension</key><array><string>dx</string></array>
        <key>public.mime-type</key><array><string>text/vnd.dx</string></array>
      </dict>
    </dict>
  </array>
</dict>
</plist>
PLIST

# ---------------------------------------------------------------------------------------
# Safari's extension, which is the only route Safari has and needs Xcode to produce.
# ---------------------------------------------------------------------------------------
if (( with_safari )); then
  "$root/packaging/build-safari.sh" "$app"
fi

# ---------------------------------------------------------------------------------------
# Sign. Ad-hoc keeps it runnable here; a real identity is one variable away.
# ---------------------------------------------------------------------------------------
#
# Nested code is signed before the bundle that contains it — a bundle's signature covers a
# seal over its files, so signing the outside first leaves the inside unsigned and the whole
# thing invalid. `--deep` would do this too, but it is deprecated and it hides which part
# failed; doing it in order means an error names the file.
identity="${DX_SIGNING_IDENTITY:--}"
codesign --force --options runtime --sign "$identity" "$contents/MacOS/dx"
codesign --force --sign "$identity" "$app"
codesign --verify --strict "$app"
echo "  signed with: $identity"

echo
echo "built $app"
du -sh "$app" | awk '{print "  size  "$1}'
printf "  dx    %s bytes\n" "$(stat -f%z "$contents/MacOS/dx")"
printf "  app   %s bytes\n" "$(stat -f%z "$contents/MacOS/dx-app")"
echo "  install it: open $app — or run \"$contents/MacOS/dx\" setup"
if [[ "$identity" == "-" ]]; then
  echo "  ad-hoc signed: it runs here. Distributing it needs a Developer ID — packaging/packaging.dx"
fi
