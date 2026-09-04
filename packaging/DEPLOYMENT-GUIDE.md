# Extension Deployment & Store Distribution Guide

This document describes the complete workflow for submitting the dx browser extension to app stores and integrating the results back into the codebase.

## Overview

The extension is distributed through three channels:
1. **Firefox** (addons.mozilla.org) — Free, unlimited distribution
2. **Chrome** (Chrome Web Store) — Requires $5 registration
3. **Safari** (Mac App Store) — Requires $99 Apple Developer account

Each store has its own review and signing process. This guide documents what's been prepared and what you need to do next.

## Prerequisites

### Account Setup
- [ ] **Mozilla Developer** (free): https://addons.mozilla.org/developers/
- [ ] **Google Chrome Web Store** ($5): https://chrome.google.com/webstore/developer/dashboard
- [ ] **Apple Developer** ($99/year, if Safari): https://developer.apple.com/

### Local Requirements
- [ ] Rust toolchain installed (for building archives)
- [ ] `packaging/build-stores.sh` executable
- [ ] Python 3 (for archive verification)

## What's Ready

The following infrastructure is complete:

### ✓ Build Infrastructure
- `packaging/build-stores.sh` — Builds all extension archives
- Archives output to: `packaging/build/dx-chrome.zip`, `packaging/build/dx-firefox.xpi`
- Build verification with `packaging/verify-archives.py`

### ✓ Integration Points in Code
- **CHROME_WEB_STORE** constant in `rust/doc-cli/src/extension.rs:297`
  - Type: `pub const CHROME_WEB_STORE: Option<&str> = None;`
  - Updated by: `packaging/integrate-store-results.sh`
  - Used in: `channel()` function to route to store URL
  
- **signed_xpi()** function in `rust/doc-cli/src/extension.rs:445-447`
  - Checks for signed Firefox XPI at `packaging/signed/dx-firefox.xpi`
  - Used by: Channel routing logic to prefer signed versions

- **channel()** function in `rust/doc-cli/src/extension.rs:344-372`
  - Automatically routes to store URLs when available
  - Prefers signed Firefox XPI from Mozilla
  - Falls back to unsigned for development

### ✓ Testing Infrastructure
- `packaging/test/store-submission.test.mjs` — Validates:
  - Archive format and manifest validity
  - Code integration points exist
  - Store submission infrastructure is in place

### ✓ Documentation
- `packaging/STORE_SUBMISSION_CHECKLIST.md` — Detailed, store-by-store instructions
- `packaging/integrate-store-results.sh` — Automation script for result integration

## Step-by-Step Submission Workflow

### Step 1: Build Archives
```bash
cd D:\SARA\Desktop\DOC
./packaging/build-stores.sh
```

This creates:
- `packaging/build/dx-chrome.zip` (for Chrome Web Store)
- `packaging/build/dx-firefox.xpi` (for Firefox)

Verify with:
```bash
python3 packaging/verify-archives.py
```

### Step 2: Firefox Submission (addons.mozilla.org)

1. Navigate to https://addons.mozilla.org/developers/
2. Sign in with your Mozilla Developer account
3. Click "Submit a New Add-on" → "Firefox" → "Upload"
4. Upload `packaging/build/dx-firefox.xpi`
5. Fill out form (see STORE_SUBMISSION_CHECKLIST.md for field values)
6. Submit for review

**Timeline**: 24-72 hours for review

**On Approval**:
- Download the signed XPI file
- Save to: `packaging/signed/dx-firefox.xpi`
- Verify with: `ls -la packaging/signed/dx-firefox.xpi`

### Step 3: Chrome Submission (Chrome Web Store)

1. Navigate to https://chrome.google.com/webstore/developer/dashboard
2. Click "New Item"
3. Upload `packaging/build/dx-chrome.zip`
4. Fill out form (see STORE_SUBMISSION_CHECKLIST.md for field values)
5. Submit for review

**Timeline**: 1-3 hours for review (usually faster than Firefox)

**On Approval**:
- Note the published listing URL (e.g., `https://chrome.google.com/webstore/detail/dx/[extension-id]`)
- Save this URL for integration

### Step 4: Safari Submission (Mac App Store)

This requires:
- Mac with Xcode installed
- Apple Developer account with Developer ID
- Bundled Mac application (.app)

See `packaging/STORE_SUBMISSION_CHECKLIST.md` for full Safari workflow.

## Integrating Store Results

Once store submissions are approved and you have the results:

### Firefox Integration
```bash
# Copy signed XPI from Mozilla
cp ~/Downloads/dx-firefox.xpi packaging/signed/
```

### Chrome Integration
```bash
./packaging/integrate-store-results.sh \
  --chrome-url "https://chrome.google.com/webstore/detail/dx/[YOUR_EXTENSION_ID]"
```

This script:
1. Updates `CHROME_WEB_STORE` constant with the store URL
2. Validates the change compiles
3. Runs tests to verify integration
4. Shows git diff for review

### Verification
```bash
# Run integration tests
node --test packaging/test/store-submission.test.mjs

# Verify code changes
git diff rust/doc-cli/src/extension.rs

# Run full test suite
cd rust && cargo test
```

## Creating Submission PR

After successful integration:

```bash
# Create a feature branch
git checkout -b feat/store-distribution

# Add the changes
git add rust/doc-cli/src/extension.rs

# Commit with clear message
git commit -m "
Add store distribution URLs (Firefox signed XPI and Chrome Web Store)

Firefox: signed XPI now available at packaging/signed/dx-firefox.xpi
Chrome: extension published at https://chrome.google.com/webstore/detail/dx/[ID]

Both stores reviewed and approved the extension. This commit:
- Updates CHROME_WEB_STORE constant with store URL
- Enables signed Firefox XPI detection
- Routes extension downloads through official stores

Tested: all store integration tests passing
"

# Push and create PR
git push origin feat/store-distribution
```

## Verification Checklist

After integration, verify everything works:

- [ ] `cargo test` passes in `rust/doc-cli/`
- [ ] `node --test packaging/test/*.test.mjs` passes
- [ ] `CHROME_WEB_STORE` points to published Chrome store listing
- [ ] `packaging/signed/dx-firefox.xpi` exists (if Firefox approved)
- [ ] `./dx browser --channel` shows correct URLs
- [ ] Extension works in each browser with store URLs

## Rollback Procedure

If a store submission has issues:

```bash
# Revert the constant update
git checkout HEAD~ -- rust/doc-cli/src/extension.rs

# The extension will continue using default channels
# (GitHub releases for Chrome, GitHub for Firefox)

# After fixing, resubmit to stores
```

## Timeline Estimates

| Task | Estimated Time | Wall Time |
|------|-----------------|-----------|
| Build archives | 5 minutes | Immediate |
| Firefox review | 30 min active | 24-72 hours |
| Chrome review | 15 min active | 1-3 hours |
| Safari review | 1 hour active | 1-3 days |
| Integration | 15 minutes | Immediate |
| Testing | 10 minutes | Immediate |

**Total Active Time**: ~2 hours  
**Total Wall Time**: 3-7 days (runs in parallel)

## CI/CD Integration

A GitHub Actions workflow is available at `.github/workflows/submit-to-stores.yml`.

To use:
1. Add store credentials to GitHub Secrets:
   - `CHROME_WEBSTORE_TOKEN` (if automating Chrome)
   - `APPLE_DEVELOPER_ID` (if automating Safari)

2. Trigger manually:
   ```
   GitHub UI → Actions → "Submit Extension to Stores" → Run workflow
   ```

## Troubleshooting

### Archive Verification Fails
```bash
# Check Python version
python3 --version  # Must be 3.8+

# Run detailed verification
python3 packaging/verify-archives.py --verbose
```

### Store URLs Not Routing Correctly
```bash
# Test channel resolution
./dx browser --channel
./dx browser --channel firefox
./dx browser --channel chromium
```

### Integration Script Issues
```bash
# Check if signed files exist
ls -la packaging/signed/

# Verify path format
./packaging/integrate-store-results.sh --help
```

## References

- Mozilla Add-ons: https://addons.mozilla.org/developers/
- Chrome Web Store: https://chrome.google.com/webstore/developer/dashboard
- Apple Developer: https://developer.apple.com/
- Store submission checklist: `packaging/STORE_SUBMISSION_CHECKLIST.md`
- Integration script: `packaging/integrate-store-results.sh`
- Submission tests: `packaging/test/store-submission.test.mjs`
