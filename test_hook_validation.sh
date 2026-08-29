#!/bin/bash
set -e

echo "=== Testing current pre-commit hook ==="

# Test 1: Hook should exist and be executable
if [ ! -x .git/hooks/pre-commit ]; then
  echo "FAIL: hook does not exist or is not executable"
  exit 1
fi
echo "PASS: hook exists and is executable"

# Test 2: Hook should succeed on clean working directory
git diff --cached --quiet
if .git/hooks/pre-commit; then
  echo "PASS: hook succeeds on no staged changes"
else
  echo "FAIL: hook fails on no staged changes"
  exit 1
fi

# Test 3: Create a badly formatted Rust file and test if hook validates it
test_file="test_bad_format.rs"
cat > "$test_file" << 'RUST_EOF'
fn   main( ) {
    println!("hello");
}
RUST_EOF

git add "$test_file"

# Current hook just exits 0, so it should succeed (this is the problem!)
if .git/hooks/pre-commit; then
  echo "FAIL: hook should validate formatting but currently doesn't"
  echo "Current hook output: $(.git/hooks/pre-commit 2>&1)"
  git reset "$test_file"
  rm "$test_file"
  exit 1
fi

git reset "$test_file"
rm "$test_file"
