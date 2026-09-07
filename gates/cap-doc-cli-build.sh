#!/bin/bash
set -euo pipefail

# cap-doc-cli-build: verify doc-cli cargo build succeeds
cd rust
cargo build -p doc-cli >/dev/null 2>&1
