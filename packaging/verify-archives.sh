#!/usr/bin/env bash
#
# Verify that extension store archives pass preflight checks.
#
#   ./packaging/verify-archives.sh
#     Checks dx-chrome.zip and dx-firefox.xpi for:
#     - No __MACOSX/ entries (resource fork contamination)
#     - Parsable manifest.json
#     - Version advancement past the last published version

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$root/packaging/build"
version_file="$build_dir/last-published-version.txt"

# Source shared helpers
source "$root/packaging/lib.sh"

# Initialize last-published-version.txt if it doesn't exist
if [[ ! -f "$version_file" ]]; then
    echo "0.0.0" > "$version_file"
fi

last_version=$(cat "$version_file")
echo "Archive verification"
echo "  Last published version: $last_version"

verify_archive() {
    local archive="$1"
    local name=$(basename "$archive")

    # Check for __MACOSX entries
    if archive_has_macosx "$archive"; then
        echo "ERROR: $name contains __MACOSX/ entries" >&2
        return 1
    fi

    # Extract and validate manifest.json, get version
    local manifest_version
    if ! manifest_version=$(unzip_and_inspect_manifest "$archive"); then
        return 1
    fi

    # Check version advancement
    if ! version_gt "$manifest_version" "$last_version"; then
        echo "ERROR: $name version $manifest_version does not advance past $last_version" >&2
        return 1
    fi

    echo "  $name OK (version $manifest_version)"
    return 0
}

# Verify both archives
local_exit=0
verify_archive "$build_dir/dx-chrome.zip" || local_exit=$?
verify_archive "$build_dir/dx-firefox.xpi" || local_exit=$?

exit "$local_exit"
