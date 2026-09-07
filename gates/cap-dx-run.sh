#!/bin/bash
set -euo pipefail

# cap-dx-run: verify code execution and output storage tests pass
cd rust
cargo test --lib run >/dev/null 2>&1
