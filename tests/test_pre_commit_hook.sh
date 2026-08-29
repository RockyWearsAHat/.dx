#!/usr/bin/env bash
# Test suite for pre-commit hook

set -e

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_PATH="$REPO_ROOT/.git/hooks/pre-commit"

echo "=== Pre-commit Hook Test Suite ==="

# Test 1: Hook must exist and be executable
test_hook_exists() {
    if [[ ! -x "$HOOK_PATH" ]]; then
        echo "FAIL: Hook does not exist or is not executable at $HOOK_PATH"
        return 1
    fi
    echo "PASS: Hook exists and is executable"
    return 0
}

# Test 2: Hook must pass on no-op commit (no staged Rust files)
test_hook_passes_on_empty() {
    cd "$REPO_ROOT"
    if git diff --cached --quiet; then
        if git commit --allow-empty -m "test: no-op commit" 2>/dev/null; then
            git reset --soft HEAD~1
            echo "PASS: Hook allows no-op commits"
            return 0
        else
            echo "FAIL: Hook rejected no-op commit"
            return 1
        fi
    fi
}

# Test 3: Hook must reject commits with unformatted Rust code
test_hook_rejects_unformatted() {
    local temp_dir=$(mktemp -d)
    trap "rm -rf $temp_dir" RETURN

    cd "$temp_dir"

    # Initialize a temp git repo
    git init >/dev/null 2>&1
    git config user.email "test@example.com"
    git config user.name "Test User"

    # Copy the hook
    mkdir -p .git/hooks
    cp "$HOOK_PATH" .git/hooks/pre-commit

    # Create a badly formatted Rust file
    cat > test.rs << 'EOF'
fn main(  ) {
  let x = 5;
}
EOF

    git add test.rs

    # Try to commit - should fail
    if ! git commit -m "test: badly formatted" 2>/dev/null; then
        echo "PASS: Hook rejects unformatted Rust code"
        return 0
    else
        echo "FAIL: Hook did not reject unformatted code"
        return 1
    fi
}

# Test 4: Hook must pass on well-formatted Rust code
test_hook_accepts_formatted() {
    local temp_dir=$(mktemp -d)
    trap "rm -rf $temp_dir" RETURN

    cd "$temp_dir"

    # Initialize a temp git repo
    git init >/dev/null 2>&1
    git config user.email "test@example.com"
    git config user.name "Test User"

    # Copy the hook
    mkdir -p .git/hooks
    cp "$HOOK_PATH" .git/hooks/pre-commit

    # Create a properly formatted Rust file
    cat > test.rs << 'EOF'
fn main() {
    let x = 5;
}
EOF

    git add test.rs

    # Try to commit - should succeed
    if git commit -m "test: well formatted" 2>/dev/null; then
        echo "PASS: Hook accepts well-formatted Rust code"
        return 0
    else
        echo "FAIL: Hook rejected well-formatted code"
        return 1
    fi
}

# Run all tests
cd "$REPO_ROOT"
all_pass=true

test_hook_exists || all_pass=false
test_hook_passes_on_empty || all_pass=false
test_hook_rejects_unformatted || all_pass=false
test_hook_accepts_formatted || all_pass=false

echo ""
if [[ "$all_pass" == "true" ]]; then
    echo "=== ALL TESTS PASSED ==="
    exit 0
else
    echo "=== SOME TESTS FAILED ==="
    exit 1
fi
