#!/bin/bash
set -euo pipefail

# cap-archives: verify browser extension archives are valid and intact
unzip -t packaging/build/dx-firefox.xpi >/dev/null 2>&1
unzip -t packaging/build/dx-chrome.zip >/dev/null 2>&1
