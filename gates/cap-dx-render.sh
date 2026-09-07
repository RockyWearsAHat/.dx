#!/bin/bash
set -euo pipefail

# cap-dx-render: verify document rendering tests pass
cd rust
cargo test --lib render >/dev/null 2>&1
