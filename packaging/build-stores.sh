#!/usr/bin/env bash
#
# Build the archives the extension stores take.
#
#   ./packaging/build-stores.sh
#     → packaging/build/dx-chrome.zip     upload to the Chrome Web Store
#     → packaging/build/dx-firefox.xpi    upload to addons.mozilla.org for signing
#
# Both are the extension directory in an envelope, and both are written by `dx browser --from`
# out of `editor/github` — the one source every artifact is built from, using the same code
# path a developer's `dx browser --from` takes. Assembling an archive here by hand would let a
# store listing drift from the engine the CLI ships, which is the one disagreement this project
# cannot tolerate.
#
# Nothing here is in the shipped binary: an archive writer is a release tool, and a document
# renderer has no business carrying one.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/packaging/build"
staging="$out/stores"

echo "store archives"
(cd "$root/rust" && cargo build --release -p doc-cli >/dev/null)
dx="$root/rust/target/release/dx"

rm -rf "$staging"
mkdir -p "$staging"
"$dx" browser --from "$root/editor/github" --dir "$staging" >/dev/null

# `zip -X` drops the extra file attributes; both stores reject archives carrying resource
# forks, and a macOS `zip` adds them unasked.
package() {
  local dir="$1" archive="$2"
  rm -f "$archive"
  (cd "$dir" && zip -q -r -X "$archive" . -x '.*' -x '__MACOSX/*')
  printf "  %-22s %8d bytes\n" "$(basename "$archive")" "$(stat -f%z "$archive" 2>/dev/null || stat -c%s "$archive")"
}

package "$staging/chromium" "$out/dx-chrome.zip"
package "$staging/firefox" "$out/dx-firefox.xpi"
rm -rf "$staging"

cat <<'NEXT'

next
  Chrome Web Store   https://chrome.google.com/webstore/devconsole
                     One-time $5 registration. Covers Chrome, Edge, Brave, Vivaldi,
                     Opera and Arc — all of them install from this one listing.
                     After it is published, set CHROME_WEB_STORE in
                     rust/doc-cli/src/extension.rs to the listing URL. That single
                     line switches every Chromium browser from developer mode to a
                     one-click install; there is nothing else to change.

  addons.mozilla.org https://addons.mozilla.org/developers/addon/submit/distribution
                     Free. Choose "On your own site" to get an *unlisted* signature.
                     Put the signed file at packaging/signed/dx-firefox.xpi and
                     rebuild the app; `dx setup` then installs it into Firefox with
                     no clicks at all. Unsigned, release Firefox refuses it — which
                     is why dx checks for the file rather than assuming it.
NEXT
