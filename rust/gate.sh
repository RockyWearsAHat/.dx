#!/bin/bash
# gate.sh <crate> [cargo test args...]: one crate's tests as a dx gate (tests.dx).
# - rustup's cargo, found the way dev.dx's gates find it (the sandbox redirects $HOME).
# - The warm target is this checkout's own rust/target. A worktree clones it from the main
#   checkout once (`cp -cR`, an APFS clone: ~3 s, no disk), so a gate is never a cold build.
# - stdin is /dev/null: a test that reads stdin must never hang a gate (dx report-763be282).
# - Prints one verdict line: "ok - <crate>: N passed in S s", or the failing tests and exit 1.
set -o pipefail
crate=$1; shift
cargo_bin="$(type -ap cargo 2>/dev/null | grep '/\.cargo/bin/cargo' | head -1)"
[ -n "$cargo_bin" ] || cargo_bin="$(ls /Users/*/.cargo/bin/cargo 2>/dev/null | head -1)"
[ -n "$cargo_bin" ] || { echo "rustup cargo not found"; exit 1; }
export CARGO_HOME="${cargo_bin%/bin/cargo}"
export RUSTUP_HOME="${CARGO_HOME%/.cargo}/.rustup"
export PATH="${cargo_bin%/cargo}:$PATH"
here="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR="$here/target" CARGO_TERM_COLOR=never
log="$(mktemp "${DX_SANDBOX:-${TMPDIR:-/tmp}}/gate-$crate.XXXXXX")"
t0=$(date +%s)
cd "$here" || exit 1
if cargo test --offline --locked -p "$crate" "$@" </dev/null >"$log" 2>&1; then
  grep '^test result' "$log" | awk -v c="$crate" -v t=$(( $(date +%s) - t0 )) \
    '{p+=$4} END {print "ok - " c ": " p " passed in " t " s"}'
  rm -f "$log"
else
  echo "FAILED - $crate after $(( $(date +%s) - t0 )) s"
  grep -E '^test .* FAILED$|panicked at|^error(\[|:)' "$log" | head -20
  tail -5 "$log"
  rm -f "$log"
  exit 1
fi
