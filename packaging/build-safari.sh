#!/usr/bin/env bash
#
# Build the Safari extension and put it inside DX.app.
#
#   ./packaging/build-safari.sh <path to DX.app>
#
# Safari is the one browser that takes no directory and no store archive: a Safari Web
# Extension is an app extension, it lives inside an application bundle, and it is produced by
# an Xcode project that Apple's converter generates from the Chromium extension. That is one of
# the two things only an application can do — the other is opening a double-clicked `.dx`,
# which is what packaging/app is — and it is why this is a separate script: everything here
# needs Xcode, which most machines building `dx` will not have.
#
# The generated project is deliberately *not* committed, and is not even written into the
# repository. It is a mechanical translation of editor/github, it is several hundred files,
# and Xcode's derived data beside it is another 149 MB — a working tree that size is one a
# person searches, backs up, and greps through by accident. It is built in a temporary
# directory and removed on the way out; regenerating takes seconds.

set -euo pipefail

app="${1:?usage: build-safari.sh <path to DX.app>}"

if ! xcrun -f safari-web-extension-converter >/dev/null 2>&1; then
  echo "  Safari: skipped — needs Xcode (xcode-select --install is not enough)" >&2
  exit 0
fi

echo "  building the Safari extension…"
work="$(mktemp -d "${TMPDIR:-/tmp}/dx-safari.XXXXXX")"
# Every exit below is a deliberate `exit 0` — the app is complete without Safari's extension —
# so the cleanup is a trap rather than a line at the end that most paths never reach.
trap 'rm -rf "$work"' EXIT

# The converter reads the Chromium directory, which `dx browser` has already written into the
# app's resources — so the Safari extension is generated from exactly the files every other
# browser gets.
source_dir="$app/Contents/Resources/extension/chromium"

xcrun safari-web-extension-converter "$source_dir" \
  --project-location "$work" \
  --app-name "dx" \
  --bundle-identifier "tools.dx.app" \
  --no-open \
  --force \
  --macos-only \
  --swift >/dev/null

project="$(find "$work" -name '*.xcodeproj' -maxdepth 2 | head -1)"
if [[ -z "$project" ]]; then
  echo "  Safari: the converter produced no project — skipped" >&2
  exit 0
fi

# Ad-hoc signing is enough to load the extension on this machine with Safari's developer
# setting turned on. Safari will not load an ad-hoc extension for an ordinary reader, so a
# distributed build needs DX_SIGNING_IDENTITY set to a Developer ID — see packaging/README.md.
identity="${DX_SIGNING_IDENTITY:--}"
xcodebuild -project "$project" \
  -scheme "dx" \
  -configuration Release \
  -derivedDataPath "$work/derived" \
  CODE_SIGN_IDENTITY="$identity" \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGNING_ALLOWED=NO \
  build >/dev/null 2>&1 || {
    echo "  Safari: xcodebuild failed — the app is complete without it" >&2
    exit 0
  }

built="$(find "$work/derived/Build/Products/Release" -maxdepth 1 -name '*.app' | head -1)"
extension="$(find "$built/Contents/PlugIns" -maxdepth 1 -name '*.appex' 2>/dev/null | head -1)"
if [[ -z "$extension" ]]; then
  echo "  Safari: no app extension was produced — the app is complete without it" >&2
  exit 0
fi

mkdir -p "$app/Contents/PlugIns"
cp -R "$extension" "$app/Contents/PlugIns/"
echo "  Safari: embedded $(basename "$extension")"
