#!/bin/bash
set -euo pipefail

# cap-vscode: verify VS Code extension builds successfully
cd editor/vscode
npm run build >/dev/null 2>&1
