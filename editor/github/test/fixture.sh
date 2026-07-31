#!/usr/bin/env bash
# Build the test fixture: a real pack and pointer, written by the real `dx`.
#
# The extension's job is to read what a repository actually commits, so the tests read what
# `dx sync` actually wrote rather than a pack hand-assembled in JavaScript — a fixture built
# by the code under test would hide exactly the disagreements worth catching.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
dx="${DX:-$root/rust/target/release/dx}"

if [ ! -x "$dx" ]; then
  echo "no dx binary at $dx — run: cd rust && cargo build --release -p doc-cli" >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

git init -q "$scratch"
cp "$root/examples/welcome.dx" "$scratch/"
cp "$root/examples/showcase.dx" "$scratch/"
"$dx" sync "$scratch" >/dev/null

mkdir -p "$here/fixture"
cp "$scratch/.doc/repo.dxcp" "$here/fixture/repo.dxcp"
cp "$scratch/welcome.dx" "$here/fixture/welcome.dx"

# What the CLI renders, so the shipped wasm engine can be held to it. The extension is only
# worth anything if the browser shows the same document the binary does, and the way that
# stops being true is a `doc-core` change with no `build.sh` — a committed engine one version
# behind, which nothing else would notice.
cp "$root/examples/showcase.dx" "$here/fixture/render-input.dx"
"$dx" render "$root/examples/showcase.dx" --theme light --fragment > "$here/fixture/render.html"

echo "wrote $here/fixture/repo.dxcp, welcome.dx, render-input.dx, and render.html"
