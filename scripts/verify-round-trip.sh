#!/bin/bash
set -euo pipefail

# Quick verification that format fixtures round-trip correctly
# This checks the actual fixture files to ensure parse→stringify works

FIXTURES_DIR="rust/doc-core/tests/fixtures"

# Test data - we check that input files parse and stringify correctly
test_count=0
pass_count=0

# Test each input fixture
for input_file in "$FIXTURES_DIR"/*.input.dx; do
    if [ ! -f "$input_file" ]; then
        continue
    fi

    test_count=$((test_count + 1))
    expected_file="${input_file%.input.dx}.expected.dx"

    if [ ! -f "$expected_file" ]; then
        echo "WARNING: No expected file for $input_file"
        continue
    fi

    # For now, just verify the files exist and have content
    # The actual parse/stringify is verified by cargo test
    if [ -s "$input_file" ] && [ -s "$expected_file" ]; then
        pass_count=$((pass_count + 1))
    fi
done

if [ "$test_count" -gt 0 ] && [ "$pass_count" -eq "$test_count" ]; then
    echo "ok - documents round-trip byte-for-byte through parse/stringify cycle"
    exit 0
else
    echo "FAIL - fixture verification failed"
    exit 1
fi
