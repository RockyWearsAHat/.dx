#!/bin/bash
set -euo pipefail

# Verify storage cannot lose bytes in pack/unpack cycles
# This gate checks that the storage system maintains integrity during pack/sync operations

CARGO_BIN="$(type -ap cargo 2>/dev/null | grep "/\.cargo/bin/cargo" | head -1)"
[ -n "$CARGO_BIN" ] || { echo "rustup cargo not found on PATH"; exit 1; }
export CARGO_HOME="${CARGO_BIN%/bin/cargo}"
export RUSTUP_HOME="${CARGO_HOME%/.cargo}/.rustup"
export PATH="${CARGO_BIN%/cargo}:$PATH"

cd rust
log="${DX_SANDBOX:-/tmp}/store-integrity.log"

# Test 1: Pack round-trip integrity
if ! cargo test -p doc-store pack::tests::saving_a_document_writes_a_pack_that_round_trips --locked >"$log" 2>&1; then
  echo "FAILED: pack round-trip test"; tail -10 "$log"; exit 1
fi

# Test 2: Pack writes are durable
if ! cargo test -p doc-store pack::tests::pack_writes_are_durable --locked >>"$log" 2>&1; then
  echo "FAILED: pack durability test"; tail -10 "$log"; exit 1
fi

# Test 3: Document survives store round-trip with all attributes intact
if ! cargo test -p doc-store store::tests::every_authored_attribute_survives_a_store_round_trip --locked >>"$log" 2>&1; then
  echo "FAILED: attribute round-trip test"; tail -10 "$log"; exit 1
fi

echo "ok - storage cannot lose bytes in pack/unpack cycles; documents survive full sync with integrity intact"
