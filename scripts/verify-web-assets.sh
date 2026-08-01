#!/usr/bin/env bash
# 校验仓库内已固定的 noVNC web 资源与浏览器依赖锁（sh 版本，对应 verify-web-assets.ps1）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# shellcheck source=scripts/web-assets-tools.sh
source "$SCRIPT_DIR/web-assets-tools.sh"

NOVNC_ROOT="$REPOSITORY_ROOT/third_party/novnc"
NOVNC_VERSION_ROOT="$NOVNC_ROOT/$NOVNC_VERSION"

assert_web_asset_tree "$NOVNC_VERSION_ROOT" "$NOVNC_ROOT/manifest.sha256"
assert_novnc_package \
    "$NOVNC_VERSION_ROOT" \
    "$NOVNC_ROOT/npm-metadata.json" \
    "$NOVNC_ROOT/npm-attestations.json"
assert_browser_package_lock \
    "$REPOSITORY_ROOT/browser-tests/package.json" \
    "$REPOSITORY_ROOT/browser-tests/package-lock.json"

echo "Web assets and browser dependency lock passed."
