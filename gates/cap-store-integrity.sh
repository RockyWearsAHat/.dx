#!/bin/bash
set -euo pipefail

# cap-store-integrity gate: verify storage cannot lose bytes in pack/unpack cycles

cd rust

# Run storage integrity tests that verify documents survive full sync cycles.
# Key test: a_clean_workspace_syncs_to_no_changes verifies that pack/unpack
# cycles are idempotent and documents survive sync without corruption.
cargo test --lib doc_store::store::tests::a_clean_workspace_syncs_to_no_changes -- --nocapture

exit 0
