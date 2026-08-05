#!/usr/bin/env python3
"""校验仓库内已固定的 noVNC web 资源与浏览器依赖锁。

对应原 verify-web-assets.ps1 / verify-web-assets.sh（#9 阶段 B1）。
离线核验固定资源、许可证和浏览器锁文件，不访问网络。
"""

from __future__ import annotations

import sys
from pathlib import Path

from web_assets_tools import (
    NOVNC_VERSION,
    WebAssetsError,
    assert_browser_package_lock,
    assert_novnc_package,
    assert_web_asset_tree,
)

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    no_vnc_root = REPOSITORY_ROOT / "third_party" / "novnc"
    try:
        assert_web_asset_tree(
            root=str(no_vnc_root / NOVNC_VERSION),
            manifest_path=str(no_vnc_root / "manifest.sha256"),
        )
        assert_novnc_package(
            package_root=str(no_vnc_root / NOVNC_VERSION),
            metadata_path=str(no_vnc_root / "npm-metadata.json"),
            attestations_path=str(no_vnc_root / "npm-attestations.json"),
        )
        assert_browser_package_lock(
            package_json_path=str(REPOSITORY_ROOT / "browser-tests" / "package.json"),
            package_lock_path=str(
                REPOSITORY_ROOT / "browser-tests" / "package-lock.json"
            ),
        )
    except WebAssetsError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    print("Web assets and browser dependency lock passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
