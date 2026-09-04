#!/usr/bin/env bash
#
# Safari submission helper: verify dependencies, check for Xcode, and convert the Chromium
# extension to a Safari Web Extension.
#
#   ./packaging/submit-safari.sh [--dry-run]
#
# In dry-run mode, outputs the xcrun command without executing it.
# In normal mode, produces a Safari app wrapper in packaging/build/.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chromium_dir="$root/editor/github"
build_dir="$root/packaging/build"

# Check if dry-run flag is set
dry_run=0
if [[ "${1:-}" == "--dry-run" ]]; then
    dry_run=1
fi

# Verify Chromium extension directory exists
if [[ ! -d "$chromium_dir" ]]; then
    echo "ERROR: Chromium extension directory not found: $chromium_dir" >&2
    exit 1
fi

# Build the xcrun command
xcrun_cmd="xcrun safari-web-extension-converter \"$chromium_dir\" --project-location \"$build_dir\" --app-name \"dx\" --bundle-identifier \"tools.dx.app\" --no-open --force --macos-only --swift"

if [[ $dry_run -eq 1 ]]; then
    # In dry-run mode, just output the command without checking Xcode
    echo "$xcrun_cmd"
    exit 0
fi

# Check for Xcode/xcrun (only when actually running)
if ! xcrun -f safari-web-extension-converter >/dev/null 2>&1; then
    echo "ERROR: Xcode not found — needs full Xcode (xcode-select --install is not enough)" >&2
    exit 1
fi

# Create build directory if it doesn't exist
mkdir -p "$build_dir"

# Execute the converter
echo "Converting Chromium extension to Safari Web Extension…"
eval "$xcrun_cmd" >/dev/null || {
    echo "ERROR: safari-web-extension-converter failed" >&2
    exit 1
}

echo "Safari submission helper completed successfully"
