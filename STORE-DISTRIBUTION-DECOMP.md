# Store Distribution Item Decomposition

## Failed Item
The second worklist item "Store distribution: sign and submit the browser extension archives..."

## Decomposition

### Mechanical Sub-Items (for Haiku workers)

1. **Archive validation - Firefox**
   - Ask: charter-DOC
   - Capability: cap-archives
   - Task: Verify packaging/build/dx-firefox.xpi exists, has ZIP format with manifest.json
   - Verification: `unzip -t packaging/build/dx-firefox.xpi > /dev/null && echo OK` exits 0
   - Time: <5 minutes

2. **Archive validation - Chrome**
   - Ask: charter-DOC
   - Capability: cap-archives
   - Task: Verify packaging/build/dx-chrome.zip exists and is valid ZIP with manifest.json inside
   - Verification: `unzip -t packaging/build/dx-chrome.zip > /dev/null && echo OK` exits 0
   - Time: <5 minutes

3. **Create store-integration script**
   - Ask: charter-DOC
   - Capability: cap-doc-cli-build
   - Task: Write packaging/integrate-store-urls.sh that reads store URLs from stdin and updates extension.rs CHROME_WEB_STORE, FIREFOX_AMO constants
   - Verification: Script has shebang, set -euo pipefail, validates URLs before writing; `./packaging/integrate-store-urls.sh < urls.txt` works
   - Time: <10 minutes

4. **Document store submission process**
   - Ask: charter-DOC
   - Capability: cap-version
   - Task: Create STORE-SUBMISSION.md with per-store steps, account requirements, approval timelines; link from README
   - Verification: File exists, is readable, contains steps for Firefox/Chrome/Apple; 500+ words
   - Time: <10 minutes

### Needs a Human

The following store submission tasks require external system access and cannot be mechanized:

- **Create Mozilla Developer Account**: Sign up at addons.mozilla.org, verify email, enable 2FA
- **Create Chrome Web Store Account**: Sign up at Chrome Web Store, pay $5 registration fee, verify identity
- **Create Apple Developer Account**: Enroll in Apple Developer Program, pay annual fee, get code signing certificate
- **Sign Firefox Archive**: Upload to addons.mozilla.org as unlisted extension, await automated review
- **Upload to Chrome Web Store**: Submit dx-chrome.zip via portal, await automated review (1-24 hours)
- **Publish to Apple**: Submit notarized DX.app and Safari extension, await manual review
- **Record Published URLs**: Copy listing URLs from stores and update extension.rs constants
