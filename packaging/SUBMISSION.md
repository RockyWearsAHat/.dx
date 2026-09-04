# Browser Extension Store Submission Guide

**Status**: Archives built and verified ✓

This guide walks through submitting the dx browser extension to Firefox, Chrome, and Safari stores.

## Prerequisites

- `packaging/build/dx-chrome.zip` — verified ✓ 242 KB
- `packaging/build/dx-firefox.xpi` — verified ✓ 241 KB
- Accounts at:
  - Mozilla (addons.mozilla.org) — free, optional account
  - Google (Chrome Web Store) — $5 one-time registration fee
  - Apple Developer — $99/year membership (for Safari distribution)

## 1. Firefox (addons.mozilla.org) — Free, Unlisted

Estimated time: 10-30 minutes for approval (or longer depending on queue)

### Steps

1. Go to **https://addons.mozilla.org/developers/addon/submit/distribution**
2. Log in with Mozilla account (create if needed)
3. Choose **"On your own site"** — this gives you an unlisted signature (not in public directory)
4. Upload `packaging/build/dx-firefox.xpi`
5. Fill in the store listing form:
   - **Name**: dx documents for GitHub
   - **Summary**: Renders .dx documents on github.com as pages
   - **Category**: Developer Tools
   - **Language**: English
   - **Description**: See packaging/packaging.dx section `submission-copy-text`
6. Submit for review
7. Mozilla will review (typically 1-3 days for unlisted)
8. Once approved, **download the signed XPI** from the approval email or developer dashboard
9. Save the signed file to: **`packaging/signed/dx-firefox.xpi`**
10. The `dx setup` command will detect this file and install Firefox with zero clicks

### Verification

After placing the signed XPI:
```bash
ls -la packaging/signed/dx-firefox.xpi  # Should exist and be ~242 KB
dx checks  # Verifies the file exists
```

## 2. Chrome Web Store — $5, Covers 6 Browsers

Estimated time: 24 hours to 3 days for approval

Chrome Web Store listing covers:
- Google Chrome
- Microsoft Edge
- Brave Browser
- Vivaldi
- Opera
- Arc

### Steps

1. Go to **https://chrome.google.com/webstore/devconsole**
2. Pay $5 one-time registration fee (if first time)
3. Click **"Create new item"** or **"New"**
4. Upload `packaging/build/dx-chrome.zip`
5. Fill in the store listing form:
   - **Name**: dx documents for GitHub
   - **Short description**: (max 132 chars) Renders .dx documents on github.com as pages: file views, diffs, and pull requests, resolved from the repository itself.
   - **Detailed description**: See packaging/packaging.dx section `submission-copy-text`
   - **Category**: Developer Tools
   - **Language**: English
6. **Screenshots** (required):
   - Need at least 1 screenshot (1280×800 or 640×400 recommended)
   - Capture the extension in action on github.com showing a .dx file rendering
   - Must show the extension loaded and working (not just a local render)
   - Run `dx browser` to get developer-mode instructions for loading the extension
7. **Permissions justification**:
   - `host_permissions: http://127.0.0.1/*`
   - → "The extension talks to dx serve, the rendering service the user installed on their own machine, over loopback only. It is optional; without it the extension renders with its own bundled WebAssembly engine. Nothing is sent to any host but the reader's own computer."
   - `content_script on https://github.com/*`
   - → "The extension resolves .dx document pointers appearing only on github.com, in file views, diffs, and pull requests. It reads the page to find them and replaces each with the rendered document. The repository content is fetched from github.com in the reader's own session — the same request the browser would make for the file itself — and stays in the tab."
8. Submit for review
9. Google will review (typically 24 hours to a few days for first submission)
10. Once **published**, go to the listing page and **copy the full URL**
    - It will look like: `https://chrome.google.com/webstore/detail/dx-documents/EXTENSIONIDHERE`

### After Publishing: Update the Code

Once the listing is published and has a URL:

1. Open `rust/doc-cli/src/extension.rs`
2. Find line ~297: `pub const CHROME_WEB_STORE: Option<&str> = None;`
3. Change it to: `pub const CHROME_WEB_STORE: Option<&str> = Some("https://chrome.google.com/webstore/detail/...");`
   - Paste the exact URL from step 10 above
4. Save and rebuild:
   ```bash
   cargo build --release -p doc-cli
   ./packaging/build-app.sh --safari
   ```
5. This single line switches every Chromium browser (Chrome, Edge, Brave, Vivaldi, Opera, Arc) from developer-mode installation to one-click store install

### Verification

```bash
grep CHROME_WEB_STORE rust/doc-cli/src/extension.rs  # Should show the URL
cargo test --package doc-cli extension  # Verifies both halves cannot be half-done
```

## 3. Safari — Requires Apple Developer Account ($99/year)

Estimated time: Several days to 2 weeks (Apple review queue)

**Note**: Safari distribution requires building and signing a macOS app, which needs an Apple Developer ID certificate.

### Prerequisites

- Apple Developer account ($99/year membership)
- Full Xcode installed (not just command-line tools; `xcode-select --install` is not enough)
- Developer ID certificate (issued by Apple, not self-signed)

### Steps

1. Open Xcode and create your Developer ID signing certificate (if you don't have one):
   - Xcode → Settings → Accounts → Manage Certificates
   - Click + and create a "Developer ID Application" certificate
2. Set your signing identity:
   ```bash
   export DX_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
   ```
   - Replace "Your Name (TEAMID)" with the exact name from your certificate
3. Build the app:
   ```bash
   ./packaging/build-app.sh --safari
   ```
   - This uses `build-safari.sh` to:
     - Run Xcode's `safari-web-extension-converter` on the Chromium extension
     - Bundle it into a macOS app
     - Code-sign the app with your Developer ID
4. Notarize the app (required by macOS for distribution):
   ```bash
   xcrun notarytool submit packaging/build/DX.app \
     --keychain-profile dx \
     --wait
   xcrun stapler staple packaging/build/DX.app
   ```
5. Submit to **App Store** or distribute via your own site:
   - **App Store**: Use Transporter or App Store Connect
   - **Direct distribution**: Host `packaging/build/DX.app.zip` on your website

### Verification

After signing and notarizing:
```bash
codesign -dv packaging/build/DX.app  # Verify code signature
```

When distributed and installed:
```bash
dx setup  # Installs Safari extension with zero clicks
```

## Summary

### What triggers "one-click install"

| Browser | Route | Requirement |
|---------|-------|------------|
| Firefox | Policy file | Signed XPI at `packaging/signed/dx-firefox.xpi` |
| Chromium (Chrome, Edge, etc.) | Chrome Web Store | `CHROME_WEB_STORE` constant set in code |
| Safari | App bundle | Signed and notarized DX.app |

### Build commands

```bash
# Build the archives (done ✓)
./packaging/build-stores.sh

# Update Chrome URL and rebuild
cargo build --release -p doc-cli
./packaging/build-app.sh --safari

# Sign and notarize macOS app
./packaging/build-app.sh --safari
xcrun notarytool submit packaging/build/DX.app --keychain-profile dx --wait
xcrun stapler staple packaging/build/DX.app
```

### Store submission URLs

- Firefox: https://addons.mozilla.org/developers/addon/submit/distribution
- Chrome: https://chrome.google.com/webstore/devconsole
- Safari: https://appstoreconnect.apple.com

## Notes

- **No manual ZIP assembly**: Both archives are built by `dx browser --from editor/github`, the same code path the CLI ships and every developer uses. Hand-assembled archives could drift from the engine, which is the one thing this project does not tolerate.
- **Screenshot authenticity**: Chrome wants proof the extension works on github.com, not a local render. Load the extension in developer mode and capture a real github.com file view.
- **Unlisted Firefox**: "On your own site" distribution is unlisted — it never appears in the public directory, only to people who already know dx exists and are installing it via `dx setup`. This matches the project's stance: one app, one install, this device understands .dx everywhere it can.
