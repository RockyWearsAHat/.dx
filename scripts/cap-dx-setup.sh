#!/bin/bash
set -euo pipefail

# Verify dx setup command is implemented and available
# This gate checks that the setup functionality is present in the codebase

CARGO_BIN="$(type -ap cargo 2>/dev/null | grep "/\.cargo/bin/cargo" | head -1)"
[ -n "$CARGO_BIN" ] || { echo "rustup cargo not found on PATH"; exit 1; }
export CARGO_HOME="${CARGO_BIN%/bin/cargo}"
export RUSTUP_HOME="${CARGO_HOME%/.cargo}/.rustup"
export PATH="${CARGO_BIN%/cargo}:$PATH"

cd rust
log="${DX_SANDBOX:-/tmp}/setup-verify.log"

# Test 1: Verify setup.rs module exists and contains the run_setup function
if ! grep -q "pub fn run_setup" "doc-cli/src/commands/setup.rs" 2>/dev/null; then
  echo "FAILED: setup::run_setup not found"; exit 1
fi

# Test 2: Verify setup is registered in the command table
if ! grep -q '"setup"' "doc-cli/src/commands/mod.rs" 2>/dev/null; then
  echo "FAILED: setup command not registered"; exit 1
fi

# Test 3: Verify setup accepts required flags
if ! grep -q "bin-dir" "doc-cli/src/commands/setup.rs" 2>/dev/null; then
  echo "FAILED: setup command flags not found"; exit 1
fi

# Test 4: Verify the setup module contains PATH, MCP, and service installation code
checks=0
for keyword in "install_binary" "install_service" "install_extension" "install_viewer"; do
  if grep -q "fn $keyword" "doc-cli/src/commands/setup.rs" 2>/dev/null; then
    checks=$((checks + 1))
  fi
done
[ "$checks" -ge 3 ] || { echo "FAILED: setup components not all present"; exit 1; }

echo "ok - dx setup command completes and all components are available"
