#!/usr/bin/env bash
#
# Shared helpers for packaging scripts.

# Extract and validate manifest.json from an archive.
# Returns version on success (stdout), exits non-zero on failure.
#
#   unzip_and_inspect_manifest "path/to/archive.zip"
#   → prints version string (e.g., "1.0.0")
#
unzip_and_inspect_manifest() {
    local archive="$1"

    if [[ ! -f "$archive" ]]; then
        echo "ERROR: archive not found: $archive" >&2
        return 1
    fi

    local tmpdir=$(mktemp -d)
    trap "rm -rf '$tmpdir'" RETURN

    # Unzip archive to temp directory
    if ! unzip -q "$archive" -d "$tmpdir"; then
        echo "ERROR: failed to extract $archive" >&2
        return 1
    fi

    # Check for manifest.json
    if [[ ! -f "$tmpdir/manifest.json" ]]; then
        echo "ERROR: manifest.json not found in $archive" >&2
        return 1
    fi

    # Try to parse manifest.json and extract version
    local manifest_version
    if ! manifest_version=$(jq -r '.version' "$tmpdir/manifest.json" 2>/dev/null); then
        echo "ERROR: failed to parse manifest.json in $archive" >&2
        return 1
    fi

    if [[ -z "$manifest_version" || "$manifest_version" == "null" ]]; then
        echo "ERROR: version field missing in manifest.json in $archive" >&2
        return 1
    fi

    echo "$manifest_version"
    return 0
}

# Check if archive contains __MACOSX entries.
# Returns 0 if clean, 1 if contaminated.
#
#   archive_has_macosx "path/to/archive.zip"
#
archive_has_macosx() {
    local archive="$1"
    unzip -l "$archive" | grep -q '__MACOSX/'
}

# Simple semantic version comparison (v1 > v2).
# Returns 0 if v1 > v2, 1 otherwise.
#
#   version_gt "1.1.0" "1.0.0" && echo "v1 is greater"
#
version_gt() {
    local v1="$1" v2="$2"
    [[ "$v1" != "$v2" ]] && [[ "$(printf '%s\n' "$v2" "$v1" | sort -V | head -n1)" == "$v2" ]]
}
