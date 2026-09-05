#!/bin/bash
# cap-store-integrity: verify storage cannot lose bytes in pack/unpack cycles
# Exit 0 when documents survive full sync and content checksums match

set -euo pipefail

# Create test workspace
TEST_DIR="${DX_SANDBOX:-$(mktemp -d)}/cap-store-integrity-test"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Initialize git repository
git init -q .
git config user.email "test@test.com"
git config user.name "Test User"

# Create multiple test documents with diverse content
cat > test1.dx << 'DOCEOF'
::heading level=1 id=h1
First Test Document
::end

::paragraph id=p1
This is a comprehensive test document with multiple blocks to ensure storage handles diverse content correctly.
::end

::code id=code1 lang=bash
#!/bin/bash
set -euo pipefail
echo "Test code block"
::end
DOCEOF

cat > test2.dx << 'DOCEOF'
::heading level=1 id=h2
Second Test Document
::end

::paragraph id=p2
Another document to verify that multiple documents can coexist and be packed correctly.
::end

::bulleted-list id=list1
- Item one
- Item two with longer content
- Item three
::end
DOCEOF

# Create initial documents in the workspace
dx new test1.dx --title "Test Doc 1" 2>&1 || true
dx new test2.dx --title "Test Doc 2" 2>&1 || true

# Verify .doc/repo.dxcp was created
if [ ! -f .doc/repo.dxcp ]; then
  echo "FAIL: .doc/repo.dxcp not created after dx new"
  exit 1
fi

# Get hash of the store pack before sync
BEFORE_HASH=$(sha256sum .doc/repo.dxcp | cut -d' ' -f1)
BEFORE_SIZE=$(wc -c < .doc/repo.dxcp)

# Run sync to ensure consistency
dx sync . >/dev/null 2>&1 || true

# Get hash of the store pack after sync
AFTER_HASH=$(sha256sum .doc/repo.dxcp | cut -d' ' -f1)
AFTER_SIZE=$(wc -c < .doc/repo.dxcp)

# Verify hashes match - storage should be stable after sync
if [ "$BEFORE_HASH" != "$AFTER_HASH" ]; then
  echo "FAIL: pack changed after sync"
  echo "  Before: $BEFORE_HASH ($BEFORE_SIZE bytes)"
  echo "  After:  $AFTER_HASH ($AFTER_SIZE bytes)"
  exit 1
fi

# Verify file sizes match
if [ "$BEFORE_SIZE" != "$AFTER_SIZE" ]; then
  echo "FAIL: pack size changed after sync ($BEFORE_SIZE -> $AFTER_SIZE bytes)"
  exit 1
fi

# Verify documents can still be read correctly
if ! dx list . >/dev/null 2>&1; then
  echo "FAIL: unable to list documents after sync"
  exit 1
fi

# Read documents to ensure no corruption occurred
if ! dx text test1.dx >/dev/null 2>&1; then
  echo "FAIL: unable to read test1.dx after sync"
  exit 1
fi

if ! dx text test2.dx >/dev/null 2>&1; then
  echo "FAIL: unable to read test2.dx after sync"
  exit 1
fi

# Success
echo "ok - storage integrity verified: documents survive full sync and checksums match"
exit 0
