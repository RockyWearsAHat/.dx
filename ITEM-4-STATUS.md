# Worklist Item 4: Sign and Notarize DX.app — Status Report

**Date**: 2026-09-04  
**Status**: ✗ BLOCKED — Cannot Proceed

## Summary

Worklist item #4 requests signing and notarizing DX.app with an Apple Developer ID certificate. This item **cannot be completed at this time** due to three independent blockers that must be resolved before work can proceed.

## Item Details

**Item**: Sign and notarize DX.app with Apple Developer ID  
**Precondition**: Developer ID certificate obtained  
**Verification**: `codesign -v` shows "Developer ID Application" signature and notary ticket is attached

## Blockers (Cannot Proceed)

### 1. Prerequisite Item #3 Not Complete

The worklist dependency chain is:
- Item #1: Firefox submission (not started)
- Item #2: Chrome Web Store upload (not started)
- Item #3: Update CHROME_WEB_STORE constant (blocked by item #2)
- Item #4: Sign and notarize DX.app (blocked by item #3)

**Current Status**:
```rust
// rust/doc-cli/src/extension.rs:297
pub const CHROME_WEB_STORE: Option<&str> = None;
```

Item #4 explicitly depends on item #3 being completed. Item #3 requires the Chrome Web Store listing URL to be published first (item #2), which has not yet happened.

**Resolution**: Must complete items #1, #2, and #3 before item #4 can proceed.

### 2. Precondition Not Met: Developer ID Certificate Not Obtained

**Current Status** (per ITEM-0-STATUS.md):
- "**Blocker**: No Apple Developer account or certificate access"

**Requirements**:
- Apple Developer account ($99/year membership)
- Developer ID certificate issued by Apple
- Certificate stored in macOS Keychain
- Valid developer credentials to submit for notarization

**What's Required**:
```bash
export DX_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
./packaging/build-app.sh --safari
xcrun notarytool submit packaging/build/DX.app --keychain-profile dx --wait
xcrun stapler staple packaging/build/DX.app
```

Without a valid Developer ID certificate stored in Keychain, the `DX_SIGNING_IDENTITY` environment variable cannot be set to a valid developer identity, and the signing commands will fail.

**Resolution**: Enroll in Apple Developer Program ($99/year), obtain Developer ID certificate, and configure it in Keychain on a macOS machine.

### 3. Platform Constraint: Windows System Without macOS Tools

**Current Environment**: Windows 10 Home 10.0.19045

**Required Tools** (all macOS-only):
- `swiftc` — Swift compiler (needed to build the app first)
- `codesign` — Apple code signing utility
- `xcrun notarytool` — Apple notarization tool
- `xcrun stapler` — Apple notary ticket stapler
- Xcode or Command Line Tools

**Why**: The DX.app application bundle is a macOS-specific artifact. Its Swift source code (`packaging/app/*.swift`) can only be compiled on macOS with Xcode or the Command Line Tools. The app must be built, signed with a Developer ID, and notarized on macOS.

**Windows Limitation**: 
- None of these tools can run on Windows
- The `.app` bundle format is macOS-specific
- The notarization API (`xcrun notarytool`) requires macOS
- Even with tools installed, they require macOS kernel APIs

**Resolution**: This task must be executed on a macOS machine with Xcode installed.

## Dependency Tree

```
Item #1: Firefox submission
  ├─ Requires: Mozilla Developer account (free)
  ├─ Status: Not started (manual submission to addons.mozilla.org)
  └─ Blocks: Nothing (independent)

Item #2: Chrome Web Store upload
  ├─ Requires: Google account + $5 developer fee
  ├─ Status: Not started (manual upload to Chrome Web Store)
  └─ Blocks: Item #3

Item #3: Update CHROME_WEB_STORE constant
  ├─ Requires: Chrome Web Store listing URL from Item #2
  ├─ Precondition: Item #2 must be published
  ├─ Status: Not started (CHROME_WEB_STORE still = None)
  └─ Blocks: Item #4

Item #4: Sign and notarize DX.app
  ├─ Requires: 
  │  ├─ Apple Developer account ($99/year)
  │  ├─ Developer ID certificate (obtained from Apple)
  │  ├─ macOS machine with Xcode
  │  └─ Item #3 to be completed first
  ├─ Precondition: Developer ID certificate obtained
  ├─ Status: BLOCKED (cannot start)
  └─ Blocks: Nothing (final step in chain)
```

## Why This Item Is Blocked

| Blocker | Can Fix On Windows? | Can Fix Without External Resources? |
|---------|-------------------|-------------------------------------|
| Item #3 not complete | Yes (can wait for it) | No (requires Chrome Web Store approval) |
| No Developer ID cert | No (Apple-only) | No (requires $99/year Apple account) |
| No macOS tools | No (Windows system) | No (requires macOS machine) |

**Verdict**: All three blockers must be resolved. Even one blocker makes this item impossible to complete.

## What Would Need to Happen

1. **First**: Complete items #1, #2, #3 in order
   - Item #2 requires uploading to Chrome Web Store and waiting for approval
   - This is the only blocker that can be resolved on the current system (eventually)

2. **Then**: Obtain Apple Developer credentials
   - Enroll in Apple Developer Program ($99/year)
   - This is external and requires paying Apple
   - This cannot be done on Windows

3. **Finally**: Execute on macOS
   - Use a macOS machine with Xcode installed
   - Set Developer ID certificate in Keychain
   - Run the signing and notarization commands
   - This must happen on macOS; cannot be done from Windows

## Files Ready for Review

The following documents show the complete workflow for this item:
- `packaging/packaging.dx` — Contains the exact commands and workflow
- `ITEM-0-FINAL-STATUS.md` — Complete status of the store distribution work
- `packaging/SUBMISSION.md` — Step-by-step submission guide

## Conclusion

**Item #4 is blocked by three factors**:

1. **Workflow dependency**: Item #3 must be completed first
2. **External resource**: Apple Developer account not set up
3. **Platform constraint**: This is a Windows system; requires macOS machine

This is **not a task execution failure**. It is a legitimate blocker where:
- The preceding workflow step has not been completed
- Required external resources have not been obtained
- The required tools and platform are not available on this system

**Item Status**: UNCHECKED — Cannot proceed without resolving all three blockers.

**Next Steps to Unblock**:

1. Complete Chrome Web Store upload (item #2) and update code (item #3)
2. Enroll in Apple Developer Program and obtain Developer ID certificate
3. Switch to a macOS machine with Xcode
4. Then execute the signing and notarization workflow

Until these are resolved, this item must remain blocked.
