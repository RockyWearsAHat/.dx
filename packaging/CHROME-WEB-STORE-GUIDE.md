# Chrome Web Store Publication Guide

This guide provides step-by-step instructions for publishing the `dx` extension to the Chrome Web Store.

## Prerequisites

- ✓ Google account (free)
- ✓ `packaging/build/dx-chrome.zip` built and verified
- ✓ Screenshots showing the extension working (recommended: 1280×800 or 640×400 px)
- $5 one-time developer registration fee

## Step 1: Create Developer Account

1. Go to [Chrome Web Store Developer Console](https://chrome.google.com/webstore/devconsole)
2. Sign in with your Google account
3. Pay the $5 developer registration fee (one-time, required to publish anything)
   - This enables you to publish to Chrome, Edge, Brave, Vivaldi, Opera, and Arc all from one listing

## Step 2: Verify Archive is Ready

Run the verification script to ensure everything is in order:

```bash
./packaging/verify-integration-ready.sh
```

All checks should pass with:
- ✓ Chrome archive exists (242 KB)
- ✓ Integration points in place
- ✓ All tests passing

## Step 3: Create New Item in Chrome Web Store

1. In the Developer Console, click **"Create new item"**
2. Upload `packaging/build/dx-chrome.zip`
3. Google will automatically extract and validate the manifest

## Step 4: Fill Store Listing Form

### Basic Information

| Field | Value |
|-------|-------|
| **Name** | `dx` (or `dx documents`) |
| **Short description** | "Renders .dx documents on github.com as pages" |
| **Category** | Developer Tools |
| **Language** | English |

### Detailed Description

See `packaging/SUBMISSION.md` section "Chrome Web Store Copy Text" for the full marketing copy. Example:

```
dx renders .dx documents directly on github.com.

When browsing a repository containing dx files (*.dx files in the git tree), 
the extension detects them and renders them as full pages:
- File views show the document as it appears in the viewer
- Pull request diffs show what changed in the document
- Commit messages can link to sections

The renderer is the same one inside `dx` itself — what you see on github.com 
is exactly what the CLI renders locally.
```

### Screenshots

You'll need at least 1 screenshot showing the extension working on github.com. Guidelines:
- **Size**: 1280×800 px (16:9 aspect ratio)
- **Content**: Show the extension rendering a `.dx` document on github.com
- **Note**: Clear screenshots of real usage work best

Steps to capture a screenshot:
1. Navigate to a GitHub repository with a `.dx` file
2. Open the file in GitHub's viewer
3. The extension should render it as a page (if already installed in dev mode)
4. Screenshot the rendered page
5. Crop to 1280×800 px if needed

### Permissions Justification

Google requires justification for all extension permissions. The `dx` extension needs:

| Permission | Why |
|------------|-----|
| `host_permissions: ["*://github.com/*", "*://gist.github.com/*"]` | To detect and inject rendering on GitHub's content script |
| `activeTab` | To detect which tab is currently active |
| `scripting` | To inject the rendering script into the page |

**Store description** (what to put in the "permissions" section):
> "This extension only operates on github.com and gist.github.com. It detects when you browse a .dx document file, downloads and renders it alongside GitHub's UI, and provides no other functionality."

## Step 5: Upload and Submit

1. Fill all required fields (name, short description, category, language)
2. Upload at least 1 screenshot
3. Click **"Save and continue"**
4. Review the **"Pricing and distribution"** section:
   - Set visibility to **"Public"** (so anyone can find and install it)
   - Accept Chrome Web Store policies
5. Click **"Submit for review"**

## Step 6: Wait for Google Review

- Typical review time: **1-24 hours** (can be longer)
- You can check status in the Developer Console under "Status"
- Google may reject if:
  - Permissions seem excessive (they won't for `dx`)
  - The description doesn't match functionality (it will)
  - Any security issues detected in the manifest (there are none)

## Step 7: Published! Record the Listing URL

Once published (status shows "Published"):

1. In the Developer Console, click on the extension name
2. The URL bar will show: `https://chrome.google.com/webstore/detail/[NAME]/[EXTENSION_ID]`
3. Copy the full URL (the part after `/detail/[NAME]/` is the `EXTENSION_ID`)

Example published URL:
```
https://chrome.google.com/webstore/detail/dx-documents/abcdef1234567890
```

## Step 8: Integrate the URL into the Codebase

Once you have the published listing URL, run the integration script:

```bash
./packaging/integrate-store-results.sh \
  --chrome-url "https://chrome.google.com/webstore/detail/dx-documents/YOUR_EXTENSION_ID"
```

This will:
1. Update `rust/doc-cli/src/extension.rs` with the `CHROME_WEB_STORE` constant
2. Verify the syntax is correct
3. Test that Rust compilation still works

Verify the change:
```bash
git diff rust/doc-cli/src/extension.rs
```

Expected change:
```rust
// Before
pub const CHROME_WEB_STORE: Option<&str> = None;

// After  
pub const CHROME_WEB_STORE: Option<&str> = Some("https://chrome.google.com/webstore/detail/dx-documents/YOUR_ID");
```

## Step 9: Rebuild and Test

```bash
# Rebuild the CLI with the new constant
cd rust
cargo build --release -p doc-cli
```

Test that the listing URL works:
```bash
./target/release/dx browser --channel
# Should now show the Chrome Web Store link for Chromium browsers
```

## Step 10: Commit and Create PR

```bash
git add rust/doc-cli/src/extension.rs
git commit -m "Update CHROME_WEB_STORE listing URL after publication"
git push origin
# Create a pull request
```

## Verification

After publication, anyone can:
1. Search for "dx documents" in the Chrome Web Store
2. Click the extension
3. Click "Add to Chrome"
4. Grant permissions
5. Visit github.com and see `.dx` files rendered as pages

## Troubleshooting

### Google rejected the submission
- Check the rejection reason in the Developer Console
- Common issues:
  - Description doesn't match the extension's functionality
  - Permissions seem mismatched
  - Manifest has errors
- Fix the issue in the code/manifest and resubmit

### Extension doesn't render on GitHub after installation
- Make sure you're viewing a `.dx` file
- Check that the extension is enabled in chrome://extensions
- Check browser console (F12) for any JavaScript errors

### Lost the extension ID
- Go back to Chrome Developer Console
- The extension ID is shown in the URL or under "More details"

## Next Steps After Publication

Once the Chrome Web Store listing is live:
- Users can install with one click instead of developer mode instructions
- All Chromium browsers (Chrome, Edge, Brave, Vivaldi, Opera, Arc) install from the same listing
- Update `CLAUDE.md` and documentation to point users to the store listing
- Consider also publishing to Firefox's Add-ons store (separate process in `SUBMISSION.md`)

## Resources

- [Chrome Web Store Developer Program Policies](https://developer.chrome.com/docs/webstore/program_policies)
- [Manifest v3 Documentation](https://developer.chrome.com/docs/extensions/mv3)
- [Chrome Web Store Publishing Guide](https://developer.chrome.com/docs/webstore/publish)
- [Local testing in developer mode](../../docs/DEVELOPMENT.md)
