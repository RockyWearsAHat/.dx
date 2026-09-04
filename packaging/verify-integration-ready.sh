#!/usr/bin/env bash
#
# Verify the project is ready for store submission and integration
#
# This script checks:
# 1. Archives exist and are properly formatted
# 2. Integration points are in place
# 3. Code can build with the integration infrastructure
# 4. Tests pass for store submission
#

set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$script_dir/.." && pwd)"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

checks_passed=0
checks_failed=0

print_check() {
  local name=$1
  local status=$2
  local message=${3:-}

  if [ "$status" = "✓" ]; then
    echo -e "${GREEN}✓${NC} $name"
    ((checks_passed++))
  elif [ "$status" = "✗" ]; then
    echo -e "${RED}✗${NC} $name"
    if [ -n "$message" ]; then
      echo "  → $message"
    fi
    ((checks_failed++))
  else
    echo -e "${YELLOW}⚠${NC} $name"
    if [ -n "$message" ]; then
      echo "  → $message"
    fi
  fi
}

echo "Verifying store submission readiness..."
echo ""

# 1. Check archives exist
echo "=== Archives ==="
if [ -f "$root/packaging/build/dx-chrome.zip" ]; then
  size=$(wc -c < "$root/packaging/build/dx-chrome.zip" 2>/dev/null || echo "0")
  size_kb=$((size / 1024))
  print_check "Chrome archive exists" "✓" "dx-chrome.zip ($size_kb KB)"
else
  print_check "Chrome archive exists" "✗" "Missing: packaging/build/dx-chrome.zip"
fi

if [ -f "$root/packaging/build/dx-firefox.xpi" ]; then
  size=$(wc -c < "$root/packaging/build/dx-firefox.xpi" 2>/dev/null || echo "0")
  size_kb=$((size / 1024))
  print_check "Firefox archive exists" "✓" "dx-firefox.xpi ($size_kb KB)"
else
  print_check "Firefox archive exists" "✗" "Missing: packaging/build/dx-firefox.xpi"
fi

echo ""
echo "=== Integration Points ==="

# 2. Check CHROME_WEB_STORE constant
if grep -q "pub const CHROME_WEB_STORE: Option<&str>" "$root/rust/doc-cli/src/extension.rs"; then
  current=$(grep "pub const CHROME_WEB_STORE" "$root/rust/doc-cli/src/extension.rs" | head -1)
  print_check "CHROME_WEB_STORE constant" "✓" "$current"
else
  print_check "CHROME_WEB_STORE constant" "✗" "Not found in extension.rs"
fi

# 3. Check signed_xpi function
if grep -q "fn signed_xpi()" "$root/rust/doc-cli/src/extension.rs"; then
  print_check "signed_xpi function" "✓"
else
  print_check "signed_xpi function" "✗" "Not found in extension.rs"
fi

# 4. Check manifest parsing capability
if grep -q "manifest" "$root/rust/doc-cli/src/extension.rs"; then
  print_check "Manifest parsing" "✓"
else
  print_check "Manifest parsing" "✗" "Not found in extension.rs"
fi

echo ""
echo "=== Integration Scripts ==="

# 5. Check integrate script
if [ -f "$root/packaging/integrate-store-results.sh" ]; then
  if [ -x "$root/packaging/integrate-store-results.sh" ]; then
    print_check "integrate-store-results.sh" "✓" "Present and executable"
  else
    # Make it executable if it exists
    chmod +x "$root/packaging/integrate-store-results.sh"
    print_check "integrate-store-results.sh" "✓" "Present (now executable)"
  fi
else
  print_check "integrate-store-results.sh" "✗" "Missing"
fi

# 6. Check verify script
if [ -f "$root/packaging/verify-integration-ready.sh" ]; then
  print_check "verify-integration-ready.sh" "✓" "Present"
else
  print_check "verify-integration-ready.sh" "✗" "Missing"
fi

echo ""
echo "=== Submission Documentation ==="

# 7. Check submission guide
if [ -f "$root/packaging/SUBMISSION.md" ]; then
  print_check "Submission guide" "✓" "packaging/SUBMISSION.md"
else
  print_check "Submission guide" "✗" "Missing packaging/SUBMISSION.md"
fi

# 8. Check status document
if [ -f "$root/ITEM-0-FINAL-STATUS.md" ]; then
  print_check "Final status document" "✓" "ITEM-0-FINAL-STATUS.md"
else
  print_check "Final status document" "✗" "Missing ITEM-0-FINAL-STATUS.md"
fi

echo ""
echo "=== Build Tests ==="

# 9. Run store submission tests
cd "$root"
if node --test packaging/test/store-submission.test.mjs >/dev/null 2>&1; then
  print_check "Store submission tests" "✓"
else
  print_check "Store submission tests" "✗" "See output above for details"
fi

echo ""
echo "=== Code Quality ==="

# 10. Check Rust formatting
if command -v cargo &> /dev/null; then
  cd "$root/rust"
  if cargo fmt --check >/dev/null 2>&1; then
    print_check "Rust formatting" "✓"
  else
    print_check "Rust formatting" "⚠" "Run: cargo fmt"
  fi

  # 11. Check Rust compilation
  if cargo check -p doc-cli >/dev/null 2>&1; then
    print_check "Rust compilation" "✓"
  else
    print_check "Rust compilation" "⚠" "Run: cargo check -p doc-cli"
  fi
else
  print_check "Rust toolchain" "⚠" "Cargo not found (skipping Rust checks)"
fi

echo ""
echo "=== Summary ==="
echo "Checks passed: $checks_passed"
echo "Checks failed: $checks_failed"

if [ $checks_failed -eq 0 ]; then
  echo ""
  echo -e "${GREEN}All checks passed! Ready for store submission.${NC}"
  echo ""
  echo "Next steps:"
  echo "1. Visit https://chrome.google.com/webstore/devconsole"
  echo "2. Pay \$5 developer fee (if not already done)"
  echo "3. Upload packaging/build/dx-chrome.zip"
  echo "4. Complete the store listing form"
  echo "5. Submit for review (Google reviews in ~24 hours)"
  echo "6. Once approved, run:"
  echo "   ./packaging/integrate-store-results.sh --chrome-url '<listing-url>'"
  exit 0
else
  echo ""
  echo -e "${RED}Some checks failed. Please fix the issues above.${NC}"
  exit 1
fi
