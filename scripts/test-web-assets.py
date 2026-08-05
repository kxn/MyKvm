#!/usr/bin/env python3
"""noVNC web 资产策略自测（跨平台 Python 单份实现）。

对应原 test-web-assets.ps1 / test-web-assets.sh（#9 阶段 B1）。
使用临时负向夹具验证策略：文件被篡改、缺失、额外增加，固定元数据
或许可证缺失，浏览器锁文件出现未批准包、浮动版本、非 npm registry
来源或缺少 integrity 时都会失败。
"""

from __future__ import annotations

import json
import os
import re
import shutil
import sys
import tempfile
import uuid
from pathlib import Path

from web_assets_tools import (
    WebAssetsError,
    assert_browser_package_lock,
    assert_novnc_package,
    assert_safe_tar_entries,
    assert_safe_temporary_path,
    assert_web_asset_tree,
    write_web_asset_manifest,
)

TEST_ROOT_PREFIX = "my-ipkvm-web-assets-"


def assert_raises(pattern: str, func, *args, **kwargs) -> None:
    """断言 func 抛出 WebAssetsError 且消息匹配正则 pattern。

    与 PowerShell -match 语义一致，大小写不敏感。
    """
    try:
        func(*args, **kwargs)
    except WebAssetsError as exc:
        if re.search(pattern, str(exc), re.IGNORECASE):
            return
        raise AssertionError(
            f"Exception did not match '{pattern}': {exc}"
        ) from exc
    raise AssertionError("Command succeeded but failure was expected")


def set_utf8_file(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8", newline="")


def new_test_root() -> Path:
    root = Path(tempfile.gettempdir()) / (
        TEST_ROOT_PREFIX + str(uuid.uuid4())
    )
    assert_safe_temporary_path(str(root))
    return root


def new_novnc_fixture(root: Path) -> Path:
    package = root / "package"
    set_utf8_file(package / "AUTHORS", "fixture authors")
    set_utf8_file(package / "LICENSE.txt", "fixture license")
    set_utf8_file(package / "docs/LICENSE.MPL-2.0", "fixture MPL")
    set_utf8_file(package / "vendor/pako/LICENSE", "fixture pako license")
    set_utf8_file(package / "core/crypto/des.js", "fixture BSD notice")
    set_utf8_file(package / "core/rfb.js", "export default class RFB {}")
    set_utf8_file(
        package / "package.json",
        json.dumps(
            {
                "name": "@novnc/novnc",
                "version": "1.7.0",
                "license": "MPL-2.0",
                "dependencies": {},
            },
            indent=2,
        )
        + "\n",
    )
    set_utf8_file(
        root / "npm-metadata.json",
        json.dumps(
            {
                "version": "1.7.0",
                "gitHead": "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e",
                "dist": {
                    "tarball": "https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz",
                    "integrity": (
                        "sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/"
                        "yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA=="
                    ),
                    "shasum": "7f832cf07c66475a81a25708b8e5299a5c4efec5",
                },
            },
            indent=2,
        )
        + "\n",
    )
    set_utf8_file(
        root / "npm-attestations.json",
        json.dumps(
            {
                "attestations": [
                    {
                        "predicateType": "https://slsa.dev/provenance/v1",
                        "repository": "https://github.com/novnc/noVNC",
                        "commit": "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e",
                    }
                ]
            },
            indent=2,
        )
        + "\n",
    )
    return package


def new_browser_lock_fixture(root: Path) -> Path:
    browser_root = root / "browser-tests"
    set_utf8_file(
        browser_root / "package.json",
        json.dumps(
            {
                "name": "my-ipkvm-browser-tests",
                "private": True,
                "type": "module",
                "devDependencies": {"playwright-core": "1.62.1"},
            },
            indent=2,
        )
        + "\n",
    )
    set_utf8_file(
        browser_root / "package-lock.json",
        json.dumps(
            {
                "name": "my-ipkvm-browser-tests",
                "lockfileVersion": 3,
                "requires": True,
                "packages": {
                    "": {
                        "name": "my-ipkvm-browser-tests",
                        "devDependencies": {"playwright-core": "1.62.1"},
                    },
                    "node_modules/playwright-core": {
                        "version": "1.62.1",
                        "resolved": (
                            "https://registry.npmjs.org/playwright-core/"
                            "-/playwright-core-1.62.1.tgz"
                        ),
                        "integrity": (
                            "sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS"
                            "7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw=="
                        ),
                        "dev": True,
                        "license": "Apache-2.0",
                        "engines": {"node": ">=20"},
                    },
                },
            },
            indent=2,
        )
        + "\n",
    )
    return browser_root


def main() -> int:
    root = new_test_root()
    root.mkdir()
    try:
        no_vnc_root = root / "novnc"
        package_root = new_novnc_fixture(no_vnc_root)
        manifest_path = no_vnc_root / "manifest.sha256"
        write_web_asset_manifest(str(package_root), str(manifest_path))

        assert_web_asset_tree(str(package_root), str(manifest_path))
        assert_novnc_package(
            str(package_root),
            str(no_vnc_root / "npm-metadata.json"),
            str(no_vnc_root / "npm-attestations.json"),
        )

        # 篡改文件 → hash mismatch
        rfb_path = package_root / "core/rfb.js"
        set_utf8_file(rfb_path, "tampered")
        assert_raises(
            r"hash mismatch.*core/rfb\.js",
            assert_web_asset_tree,
            str(package_root),
            str(manifest_path),
        )
        set_utf8_file(rfb_path, "export default class RFB {}")

        # 删除文件 → missing
        authors_path = package_root / "AUTHORS"
        authors_path.unlink()
        assert_raises(
            r"missing.*AUTHORS",
            assert_web_asset_tree,
            str(package_root),
            str(manifest_path),
        )
        set_utf8_file(authors_path, "fixture authors")

        # 额外文件 → unexpected
        extra_path = package_root / "unexpected.js"
        set_utf8_file(extra_path, "unexpected")
        assert_raises(
            r"unexpected.*unexpected\.js",
            assert_web_asset_tree,
            str(package_root),
            str(manifest_path),
        )
        extra_path.unlink()

        # package.json 版本篡改 → package version
        package_json_path = package_root / "package.json"
        valid_package_json = package_json_path.read_text(encoding="utf-8")
        set_utf8_file(
            package_json_path, valid_package_json.replace('"1.7.0"', '"1.7.1"')
        )
        assert_raises(
            r"package version",
            assert_novnc_package,
            str(package_root),
            str(no_vnc_root / "npm-metadata.json"),
            str(no_vnc_root / "npm-attestations.json"),
        )
        set_utf8_file(package_json_path, valid_package_json)

        # metadata gitHead 篡改 → gitHead
        metadata_path = no_vnc_root / "npm-metadata.json"
        valid_metadata = metadata_path.read_text(encoding="utf-8")
        set_utf8_file(
            metadata_path,
            valid_metadata.replace(
                "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e",
                "0000000000000000000000000000000000000000",
            ),
        )
        assert_raises(
            r"gitHead",
            assert_novnc_package,
            str(package_root),
            str(metadata_path),
            str(no_vnc_root / "npm-attestations.json"),
        )
        set_utf8_file(metadata_path, valid_metadata)

        # 删除 pako 许可证 → required noVNC file
        pako_license = package_root / "vendor/pako/LICENSE"
        pako_license.unlink()
        assert_raises(
            r"required noVNC file.*vendor/pako/LICENSE",
            assert_novnc_package,
            str(package_root),
            str(metadata_path),
            str(no_vnc_root / "npm-attestations.json"),
        )
        set_utf8_file(pako_license, "fixture pako license")

        # 浏览器依赖锁
        browser_root = new_browser_lock_fixture(root)
        browser_package = browser_root / "package.json"
        browser_lock = browser_root / "package-lock.json"
        assert_browser_package_lock(str(browser_package), str(browser_lock))

        # resolved 篡改 → registry source
        valid_lock = browser_lock.read_text(encoding="utf-8")
        set_utf8_file(
            browser_lock,
            valid_lock.replace(
                "https://registry.npmjs.org/playwright-core/",
                "https://example.invalid/playwright-core/",
            ),
        )
        assert_raises(
            r"registry source",
            assert_browser_package_lock,
            str(browser_package),
            str(browser_lock),
        )
        set_utf8_file(browser_lock, valid_lock)

        # 未批准包 → unapproved package
        set_utf8_file(
            browser_lock,
            valid_lock.replace(
                '"node_modules/playwright-core": {',
                '"node_modules/unapproved": {\n'
                '      "version": "1.0.0",\n'
                '      "resolved": "https://registry.npmjs.org/unapproved/-/unapproved-1.0.0.tgz",\n'
                '      "integrity": "sha512-invalid",\n'
                '      "license": "MIT"\n'
                "    },\n"
                '    "node_modules/playwright-core": {',
            ),
        )
        assert_raises(
            r"unapproved package",
            assert_browser_package_lock,
            str(browser_package),
            str(browser_lock),
        )
        set_utf8_file(browser_lock, valid_lock)

        # tar 条目安全
        assert_safe_tar_entries(
            ["package/", "package/core/rfb.js"],
            [
                "drwxr-xr-x  0 0 0 0 Jan 01 00:00 package/",
                "-rw-r--r--  0 0 0 1 Jan 01 00:00 package/core/rfb.js",
            ],
        )
        assert_raises(
            r"unsafe tar path",
            assert_safe_tar_entries,
            ["package/../outside"],
            ["-rw-r--r--  0 0 0 1 Jan 01 00:00 package/../outside"],
        )
        assert_raises(
            r"unsafe tar path",
            assert_safe_tar_entries,
            ["C:/outside"],
            ["-rw-r--r--  0 0 0 1 Jan 01 00:00 C:/outside"],
        )
        assert_raises(
            r"unsafe tar entry type",
            assert_safe_tar_entries,
            ["package/link"],
            ["lrwxr-xr-x  0 0 0 0 Jan 01 00:00 package/link -> outside"],
        )
        assert_raises(
            r"unsafe tar entry type",
            assert_safe_tar_entries,
            ["package/hardlink"],
            ["hrw-r--r--  0 0 0 0 Jan 01 00:00 package/hardlink link to outside"],
        )

        # 临时目录安全断言（PSScriptRoot 不在系统临时目录下）
        assert_raises(
            r"outside the system temporary directory",
            assert_safe_temporary_path,
            str(Path(__file__).resolve().parent),
        )
    finally:
        if root.exists():
            assert_safe_temporary_path(str(root))
            shutil.rmtree(root)

    print("Web asset policy tests passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
