#!/bin/bash
set -euo pipefail

# cap-format-round-trip gate: verify documents round-trip byte-for-byte
# through parse/stringify cycle

cd rust

# Run the format round-trip test that verifies all test fixtures remain
# identical after parse→stringify cycle
cargo test --lib format::real_documents_round_trip_byte_identical -- --nocapture

exit 0
