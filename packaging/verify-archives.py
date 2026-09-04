#!/usr/bin/env python3

import zipfile
import json
import sys
import os
from pathlib import Path

def verify_archive(archive_path, last_version):
    """Verify that an archive passes preflight checks."""
    archive_name = os.path.basename(archive_path)
    
    if not os.path.exists(archive_path):
        print(f"ERROR: archive not found: {archive_path}", file=sys.stderr)
        return False
    
    # Check for __MACOSX entries
    try:
        with zipfile.ZipFile(archive_path, 'r') as zf:
            for name in zf.namelist():
                if '__MACOSX/' in name:
                    print(f"ERROR: {archive_name} contains __MACOSX/ entries", file=sys.stderr)
                    return False
            
            # Extract and validate manifest.json
            if 'manifest.json' not in zf.namelist():
                print(f"ERROR: manifest.json not found in {archive_name}", file=sys.stderr)
                return False
            
            manifest_data = json.loads(zf.read('manifest.json').decode('utf-8'))
            manifest_version = manifest_data.get('version')
            
            if not manifest_version:
                print(f"ERROR: version field missing in manifest.json in {archive_name}", file=sys.stderr)
                return False
            
            # Check version advancement
            if not version_gt(manifest_version, last_version):
                print(f"ERROR: {archive_name} version {manifest_version} does not advance past {last_version}", file=sys.stderr)
                return False
            
            print(f"  {archive_name} OK (version {manifest_version})")
            return True
            
    except Exception as e:
        print(f"ERROR: failed to process {archive_name}: {e}", file=sys.stderr)
        return False

def version_gt(v1, v2):
    """Simple semantic version comparison."""
    parts1 = [int(x) for x in v1.split('.')]
    parts2 = [int(x) for x in v2.split('.')]
    return parts1 > parts2

# Main
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
build_dir = os.path.join(root, 'packaging', 'build')
version_file = os.path.join(build_dir, 'last-published-version.txt')

if not os.path.exists(version_file):
    with open(version_file, 'w') as f:
        f.write('0.0.0\n')

with open(version_file, 'r') as f:
    last_version = f.read().strip()

print("Archive verification")
print(f"  Last published version: {last_version}")

success = True
success &= verify_archive(os.path.join(build_dir, 'dx-chrome.zip'), last_version)
success &= verify_archive(os.path.join(build_dir, 'dx-firefox.xpi'), last_version)

sys.exit(0 if success else 1)
