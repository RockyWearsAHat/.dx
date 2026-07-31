#!/usr/bin/env bash
# Build the engine every editor surface runs.
#
#   ./editor/build.sh
#
# `doc-wasm` is one crate and one engine, but `wasm-bindgen` cannot emit one artifact that
# both hosts load: a manifest content script cannot be an ES module, so the browser extension
# needs the `no-modules` target (a plain script defining a `wasm_bindgen` global), and the VS
# Code extension runs under node and needs `nodejs` (CommonJS `require`). Two compilations of
# the same source.
#
# They are built *here, together*, for one reason: a surface built at a different moment is a
# surface running a different renderer, and the whole point of `doc-core` is that no two
# surfaces can disagree. Building one by hand is how that happened before.
# `editor/github/test/engine.test.mjs` holds both builds to the same bytes `dx render` wrote.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

# wasm-pack needs the rustup toolchain ahead of any Homebrew rustc.
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v wasm-pack >/dev/null; then
  echo "wasm-pack is not installed: cargo install wasm-pack" >&2
  exit 1
fi

cd "$root/rust/doc-wasm"

# wasm-pack writes a package.json and a .gitignore for publishing to npm. Neither belongs in
# an extension directory, and the .gitignore would hide the built engine from the repository.
build() {
  local target="$1" out="$2"
  wasm-pack build --release --target "$target" --out-dir "$out"
  rm -f "$out/package.json" "$out/.gitignore" "$out/README.md"
  printf "  %-10s %8d bytes  %s\n" "$target" \
    "$(wc -c <"$out/doc_wasm_bg.wasm" | tr -d ' ')" "$out"
}

echo "building the engine"
build no-modules "$here/github/wasm"
build nodejs "$here/vscode/wasm"

# The editing surface, into the extension that packs it. `editor/surface` is the one source;
# `packaging/build-app.sh` copies the same two files into DX.app. github/ gets nothing —
# github.com shows other people's repositories, which are read there and edited nowhere.
echo "copying the editing surface"
mkdir -p "$here/vscode/surface"
cp "$here/surface/edit.js" "$here/surface/edit.css" "$here/vscode/surface/"
printf "  %-10s %8d bytes  %s\n" "surface" \
  "$(cat "$here/surface/edit.js" "$here/surface/edit.css" | wc -c | tr -d ' ')" \
  "$here/vscode/surface"

cat <<NEXT

verify   node --test "$here/github/test/*.test.mjs"   (after github/test/fixture.sh)

github   chrome://extensions → Developer mode → Load unpacked → $here/github
         about:debugging → This Firefox → Load Temporary Add-on → $here/github/manifest.json
vscode   cd $here/vscode && npm install && npm run package
NEXT
