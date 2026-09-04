#!/bin/bash
# Test for report-933a6178: Step 8 acceptance test fix
# Verifies that the Build section of index.dx contains cargo or npm commands
# by using dx text (markdown) instead of dx render (HTML)

set -euo pipefail

cd "$(dirname "$0")"

# The CORRECT test: uses dx text to get markdown output
if dx text index.dx 2>/dev/null | grep -A 2 '^##.*Build' | grep -E 'cargo|npm' >/dev/null; then
    echo "✓ report-933a6178: Step 8 acceptance test passes with dx text"
    exit 0
else
    echo "✗ report-933a6178: Step 8 acceptance test FAILED"
    exit 1
fi
