# Worklist Item 0: Store Distribution — Status Report

## Summary

**Status**: ✓ Partially Complete — Archives Built & Verified, Awaiting Manual Store Submissions

Core infrastructure for browser extension distribution is complete. Archives are built, verified, and ready for human submission to stores. Remaining work requires external account access and manual interactions with store submission portals.

## Completed ✓

### 1. Build Infrastructure
- ✓ Rust release binary built successfully (`dx`)
- ✓ Browser extension directories generated via `dx browser --from editor/github`
- ✓ Archives created with proper ZIP format (forward-slash paths, no backslashes)

### 2. Archives Created & Verified
- ✓ `packaging/build/dx-chrome.zip` (242 KB) 
  - Contains chromium extension for Chrome, Edge, Brave, Vivaldi, Opera, Arc
  - Version: 1.0.0
  - Verified: No __MACOSX/ entries, valid manifest, version advancement
- ✓ `packaging/build/dx-firefox.xpi` (241 KB)
  - Contains firefox extension
  - Version: 1.0.0
  - Verified: No __MACOSX/ entries, valid manifest, version advancement

### 3. Verification Infrastructure
- ✓ Created `packaging/verify-archives.py` (Python-based fallback)
  - Original `packaging/verify-archives.sh` requires `jq` (not available on Windows)
  - Python version provides same validation without external dependencies
  - Both archives pass all preflight checks

### 4. Documentation
- ✓ Created `packaging/SUBMISSION.md` — comprehensive submission guide including:
  - Firefox addons.mozilla.org submission steps
  - Chrome Web Store submission steps ($5 registration)
  - Safari/Apple submission steps (Developer ID required)
  - Post-approval code update procedures
  - Store URLs and form field values
  - Screenshot requirements
  - Permissions justification for reviewers

## Blocked (Cannot Proceed) ✗

### 1. Mozilla Submission & Signature
- **Requirement**: Account at addons.mozilla.org
- **Action**: Upload `packaging/build/dx-firefox.xpi` for "On your own site" unlisted signing
- **Blocker**: No Mozilla account access
- **Result Needed**: Signed XPI file → `packaging/signed/dx-firefox.xpi`

### 2. Chrome Web Store Publication
- **Requirement**: Google account + $5 registration, access to Chrome Web Store devconsole
- **Action**: Upload `packaging/build/dx-chrome.zip`, provide screenshots, wait for approval
- **Blocker**: No Google/Chrome Web Store account access
- **Result Needed**: Published listing URL (e.g., `https://chrome.google.com/webstore/detail/...`)

### 3. Update CHROME_WEB_STORE Constant
- **Requirement**: Chrome Web Store listing URL from previous step
- **Action**: Update `rust/doc-cli/src/extension.rs` line ~297 with the URL
- **Blocker**: Waiting for Chrome Web Store approval and URL
- **Precondition**: Step 2 must complete first

### 4. Rebuild Application
- **Requirement**: Updated CHROME_WEB_STORE constant
- **Action**: Run `cargo build --release -p doc-cli && ./packaging/build-app.sh --safari`
- **Blocker**: Waiting for CHROME_WEB_STORE to be set (Step 3)
- **Precondition**: Step 3 must complete first

### 5. Safari/Apple Submission (Optional Tier)
- **Requirement**: Apple Developer account ($99/year) + full Xcode + Developer ID certificate
- **Action**: Run `./packaging/build-app.sh --safari`, then notarize and submit
- **Blocker**: No Apple Developer account or certificate access
- **Precondition**: Step 4 must complete first

## Architecture Decisions

### 1. ZIP Archive Format (Windows Compatibility)
**Problem**: PowerShell's Compress-Archive on Windows creates ZIPs with backslash path separators, which is invalid for cross-platform ZIP files (ZIP spec requires forward slashes).

**Solution**: Use Python's zipfile module to create archives with proper forward-slash paths.

**Why**: This ensures the archives work correctly on all platforms and pass unzip validation on Unix-like systems.

### 2. Verification Script (jq Not Available)
**Problem**: Original `packaging/verify-archives.sh` uses `jq` for JSON parsing, which is not available in the Windows Bash environment.

**Solution**: Created `packaging/verify-archives.py` using Python's built-in JSON parser.

**Why**: Ensures verification works consistently across platforms without external tool dependencies. Python is more universally available than jq.

## What Still Needs Human Action

1. **Create accounts** (if needed):
   - Mozilla account (free)
   - Google/Chrome Web Store account (one-time $5 fee)
   - Apple Developer account (required for Safari; $99/year or existing membership)

2. **Submit archives**:
   - Upload `packaging/build/dx-chrome.zip` to Chrome Web Store
   - Upload `packaging/build/dx-firefox.xpi` to addons.mozilla.org (unlisted route)

3. **Handle approvals**:
   - Wait for store reviews (24 hours to 3 days typical)
   - Provide any clarification stores may request

4. **Receive and integrate results**:
   - Download signed Firefox XPI → `packaging/signed/dx-firefox.xpi`
   - Note Chrome Web Store listing URL
   - Update `CHROME_WEB_STORE` constant in code
   - Rebuild application
   - For Safari: Handle Apple notarization and submission

5. **Verify installation**:
   - Test `dx setup` on each platform installs extension with one click
   - Verify extension works on github.com with dx documents

## Files Changed

- ✓ `packaging/verify-archives.py` — new, Python verification fallback
- ✓ `packaging/SUBMISSION.md` — new, complete store submission guide

## Files Ready for Submission

- `packaging/build/dx-chrome.zip` — ready ✓
- `packaging/build/dx-firefox.xpi` — ready ✓
- Both verified and ready for upload to stores

## How to Proceed

1. Read `packaging/SUBMISSION.md` for detailed steps
2. Create accounts if needed (Mozilla free, Google $5, Apple $99/year)
3. Follow submission steps for each store
4. After approvals, update code with Chrome Web Store URL
5. Rebuild with `./packaging/build-app.sh --safari`
6. Test installation with `dx setup`

## Next Steps (If Accounts Available)

Run these commands in order once accounts are ready:

```bash
# Verify archives before uploading
cd packaging && python verify-archives.py

# Upload to stores (requires manual web form interaction)
# - Chrome: https://chrome.google.com/webstore/devconsole
# - Firefox: https://addons.mozilla.org/developers/addon/submit/distribution

# After approvals, update code with Chrome Web Store URL
# Edit rust/doc-cli/src/extension.rs:297
# Set: pub const CHROME_WEB_STORE: Option<&str> = Some("https://...");

# Rebuild
cargo build --release -p doc-cli
./packaging/build-app.sh --safari

# For Safari (requires Apple Developer account)
export DX_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
./packaging/build-app.sh --safari
xcrun notarytool submit packaging/build/DX.app --keychain-profile dx --wait
xcrun stapler staple packaging/build/DX.app
```

## Conclusion

The technical work to build, verify, and prepare browser extension archives for store distribution is **complete**. The archives are properly formatted, pass all preflight checks, and are ready for human submission to the respective app stores.

The remaining work requires external account access and human interaction with store submission forms, which cannot be automated or completed in this isolated environment.

**Item Status**: Cannot be checked off until archives are actually submitted to stores, approved, and signed/published. All prerequisites for that submission are now in place.
