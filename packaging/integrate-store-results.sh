#!/usr/bin/env bash
#
# Integrate store submission results back into the codebase
#
# After Chrome Web Store publishes the extension, use this script to:
# 1. Update CHROME_WEB_STORE constant with the listing URL
# 2. Verify the update is correct
# 3. Rebuild the project with the new constant
#
# Usage:
#   ./packaging/integrate-store-results.sh \
#     --chrome-url "https://chrome.google.com/webstore/detail/dx-documents/EXTENSION_ID"
#

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$script_dir/.." && pwd)"

chrome_url=""
firefox_xpi=""

print_usage() {
  cat >&2 <<'EOF'
Usage: ./packaging/integrate-store-results.sh [OPTIONS]

Options:
  --chrome-url URL    Chrome Web Store listing URL
                      Format: https://chrome.google.com/webstore/detail/[NAME]/[ID]
  --firefox-xpi PATH  Path to Mozilla-signed Firefox XPI (optional)
  --help              Show this help message

Examples:
  # Update Chrome constant only
  ./packaging/integrate-store-results.sh \
    --chrome-url "https://chrome.google.com/webstore/detail/dx-documents/abc123def456"

  # Update both Chrome and Firefox
  ./packaging/integrate-store-results.sh \
    --chrome-url "https://chrome.google.com/webstore/detail/dx-documents/abc123def456" \
    --firefox-xpi /path/to/signed/dx-firefox.xpi
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --chrome-url)
      chrome_url="$2"
      shift 2
      ;;
    --firefox-xpi)
      firefox_xpi="$2"
      shift 2
      ;;
    --help)
      print_usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      print_usage
      exit 1
      ;;
  esac
done

# Validate inputs
if [ -z "$chrome_url" ] && [ -z "$firefox_xpi" ]; then
  echo "Error: At least one of --chrome-url or --firefox-xpi must be provided" >&2
  print_usage
  exit 1
fi

if [ -n "$chrome_url" ]; then
  # Validate Chrome URL format
  if ! [[ "$chrome_url" =~ ^https://chrome\.google\.com/webstore/detail/[^/]+/[^/]+$ ]]; then
    echo "Error: Invalid Chrome URL format" >&2
    echo "Expected: https://chrome.google.com/webstore/detail/[NAME]/[EXTENSION_ID]" >&2
    exit 1
  fi

  echo "Updating CHROME_WEB_STORE constant..."

  extension_rs="$root/rust/doc-cli/src/extension.rs"

  # Extract extension ID from URL
  extension_id=$(basename "$chrome_url")

  # Verify the file exists
  if [ ! -f "$extension_rs" ]; then
    echo "Error: File not found: $extension_rs" >&2
    exit 1
  fi

  # Check current value
  current=$(grep -o 'pub const CHROME_WEB_STORE: Option<&str> = [^;]*;' "$extension_rs" || true)
  echo "  Current: $current"

  # Update the constant
  # Use sed to replace the line - this handles macOS and Linux sed differently
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s|pub const CHROME_WEB_STORE: Option<&str> = [^;]*;|pub const CHROME_WEB_STORE: Option<&str> = Some(\"$chrome_url\");|" "$extension_rs"
  else
    sed -i "s|pub const CHROME_WEB_STORE: Option<&str> = [^;]*;|pub const CHROME_WEB_STORE: Option<&str> = Some(\"$chrome_url\");|" "$extension_rs"
  fi

  # Verify the update
  updated=$(grep -o 'pub const CHROME_WEB_STORE: Option<&str> = [^;]*;' "$extension_rs" || true)
  echo "  Updated: $updated"

  if [[ ! "$updated" =~ "$chrome_url" ]]; then
    echo "Error: Update verification failed" >&2
    exit 1
  fi

  echo "✓ CHROME_WEB_STORE constant updated"
fi

if [ -n "$firefox_xpi" ]; then
  echo "Verifying Firefox XPI..."

  if [ ! -f "$firefox_xpi" ]; then
    echo "Error: Firefox XPI not found: $firefox_xpi" >&2
    exit 1
  fi

  # Copy signed XPI to the expected location
  target_xpi="$root/packaging/signed/dx-firefox.xpi"
  mkdir -p "$(dirname "$target_xpi")"
  cp "$firefox_xpi" "$target_xpi"

  echo "✓ Firefox XPI installed at $target_xpi"
fi

echo ""
echo "Running tests to verify integration..."
cd "$root"

if [ -n "$chrome_url" ]; then
  # Run Rust build to verify syntax
  echo "Building Rust project..."
  cd "$root/rust"
  cargo build --release -p doc-cli 2>&1 | tail -5

  if [ $? -eq 0 ]; then
    echo "✓ Rust build succeeded"
  else
    echo "Error: Rust build failed" >&2
    exit 1
  fi
fi

echo ""
echo "Integration complete!"
echo ""
echo "Next steps:"
echo "1. Review the changes: git diff"
echo "2. Create a new commit: git commit -m 'Update store listing URLs'"
echo "3. Push to your branch: git push origin"
echo "4. Create a pull request with these changes"
