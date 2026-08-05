#!/usr/bin/env python3
"""noVNC web 资产校验公共工具（跨平台 Python 单份实现）。

对应原 web-assets-tools.psm1 / web-assets-tools.sh（#9 阶段 B1）。
行为语义与原实现一致：noVNC 版本/哈希策略常量、资产树 manifest 校验、
noVNC 包元数据校验、浏览器依赖锁校验、tar 条目安全校验。

同时提供 CLI 子命令，供保留的 Windows 维护脚本 update-novnc.ps1 调用：

    python web_assets_tools.py policy --json
    python web_assets_tools.py write-manifest --root R --path P
    python web_assets_tools.py check-tree --root R --manifest M
    python web_assets_tools.py check-novnc-package --package-root R --metadata M --attestations A
    python web_assets_tools.py check-tar-entries --names N --verbose V
    python web_assets_tools.py check-extracted-tree --root R
    python web_assets_tools.py check-path-under-root --path P --root R --message M
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# noVNC 发布策略（与 npm registry 元数据/产物绑定，升级 noVNC 时同步更新）
# ---------------------------------------------------------------------------
NOVNC_VERSION = "1.7.0"
NOVNC_COMMIT = "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e"
NOVNC_TARBALL = "https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz"
NOVNC_INTEGRITY = (
    "sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/"
    "yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA=="
)
NOVNC_SHASUM = "7f832cf07c66475a81a25708b8e5299a5c4efec5"
NOVNC_ARCHIVE_SIZE = 155185
NOVNC_ARCHIVE_SHA256 = (
    "32689f18d6abe96bc6530828a6bd0b9ae33bda07c083a6575ed255b5a8f2e903"
)
NOVNC_ARCHIVE_SHA512 = (
    "b9c1093b1e13d9abc844295ea1d93b6286d98f93b619ad8078b7d0ebd03fca31"
    "bd1c76dadbe3c38304a437716db6b986fbbd191b1db5b0494d168b8ad77473c8"
)
NOVNC_METADATA_URL = "https://registry.npmjs.org/@novnc%2Fnovnc/1.7.0"
NOVNC_ATTESTATIONS_URL = (
    "https://registry.npmjs.org/-/npm/v1/attestations/@novnc%2Fnovnc@1.7.0"
)

# 浏览器测试依赖策略（browser-tests/package.json 与锁文件绑定）
PLAYWRIGHT_VERSION = "1.62.1"
PLAYWRIGHT_RESOLVED = (
    "https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz"
)
PLAYWRIGHT_INTEGRITY = (
    "sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS"
    "7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw=="
)


class WebAssetsError(Exception):
    """门禁检查失败；消息直接用于展示。"""


# ---------------------------------------------------------------------------
# 路径安全（对应 Get-PathPrefix / Assert-PathUnderRoot 系列）
# ---------------------------------------------------------------------------

def _path_prefix(path: str) -> str:
    separators = "".join(s for s in (os.sep, os.altsep) if s)
    return path.rstrip(separators) + os.sep


def assert_path_under_root(path: str, root: str, message: str) -> str:
    """Path 必须在 Root 之下且不等于 Root，否则抛错。返回绝对路径。"""
    full_path = os.path.abspath(path)
    full_root = os.path.abspath(root)
    try:
        common = os.path.commonpath([full_path, full_root])
    except ValueError:  # 不同盘符（Windows）
        common = ""
    if full_path == full_root or common != full_root:
        raise WebAssetsError(f"{message}: {full_path}")
    return full_path


def assert_safe_temporary_path(path: str) -> None:
    temporary_root = os.path.abspath(tempfile.gettempdir())
    assert_path_under_root(
        path, temporary_root, "Path is outside the system temporary directory"
    )


def assert_safe_repository_target(
    path: str, repository_root: str, allowed_relative_root: str
) -> None:
    allowed_root = os.path.abspath(
        os.path.join(repository_root, allowed_relative_root)
    )
    assert_path_under_root(
        path, allowed_root, "Path is outside the approved repository target"
    )


def get_asset_relative_path(root: str, path: str) -> str:
    """返回 Root 下相对路径，统一用 / 分隔（对应 Get-AssetRelativePath）。"""
    full_root = os.path.abspath(root)
    full_path = assert_path_under_root(path, full_root, "Asset is outside its root")
    return os.path.relpath(full_path, full_root).replace(os.sep, "/")


def assert_safe_manifest_path(path: str) -> None:
    if (
        not path
        or path.isspace()
        or "\\" in path
        or "\x00" in path
        or path.startswith("/")
        or re.match(r"^[A-Za-z]:", path)
    ):
        raise WebAssetsError(f"Unsafe manifest path: {path}")
    segments = path.split("/")
    if not segments:
        raise WebAssetsError(f"Unsafe manifest path: {path}")
    for segment in segments:
        if not segment or segment in (".", ".."):
            raise WebAssetsError(f"Unsafe manifest path: {path}")


# ---------------------------------------------------------------------------
# 资产树 manifest（对应 Write/Read-WebAssetManifest、Assert-WebAssetTree）
# ---------------------------------------------------------------------------

def _sha256_hex(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_web_asset_manifest(root: str, path: str) -> None:
    full_root = os.path.abspath(root)
    lines: list[str] = []
    for dirpath, _dirnames, filenames in os.walk(full_root):
        for filename in filenames:
            file_path = os.path.join(dirpath, filename)
            relative_path = get_asset_relative_path(full_root, file_path)
            assert_safe_manifest_path(relative_path)
            lines.append(f"{_sha256_hex(file_path)}  {relative_path}")
    lines.sort()
    content = "" if not lines else "\n".join(lines) + "\n"
    manifest_path = os.path.abspath(path)
    os.makedirs(os.path.dirname(manifest_path), exist_ok=True)
    with open(manifest_path, "w", encoding="utf-8", newline="") as handle:
        handle.write(content)


def read_web_asset_manifest(path: str) -> dict[str, str]:
    entries: dict[str, str] = {}
    with open(path, "r", encoding="utf-8-sig", newline="") as handle:
        for line_number, line in enumerate(handle, start=1):
            line = line.rstrip("\r\n")
            match = re.match(r"^([0-9a-fA-F]{64})  (.+)$", line)
            if not match:
                raise WebAssetsError(f"Invalid manifest line {line_number}")
            relative_path = match.group(2)
            assert_safe_manifest_path(relative_path)
            if relative_path in entries:
                raise WebAssetsError(f"Duplicate manifest path: {relative_path}")
            entries[relative_path] = match.group(1).lower()
    return entries


def assert_web_asset_tree(root: str, manifest_path: str) -> None:
    full_root = os.path.abspath(root)
    expected = read_web_asset_manifest(manifest_path)
    actual: dict[str, str] = {}
    for dirpath, _dirnames, filenames in os.walk(full_root):
        for filename in filenames:
            file_path = os.path.join(dirpath, filename)
            relative_path = get_asset_relative_path(full_root, file_path)
            if relative_path in actual:
                raise WebAssetsError(f"Duplicate web asset path: {relative_path}")
            actual[relative_path] = _sha256_hex(file_path)

    for relative_path, digest in expected.items():
        if relative_path not in actual:
            raise WebAssetsError(f"Web asset is missing: {relative_path}")
        if actual[relative_path] != digest:
            raise WebAssetsError(f"Web asset hash mismatch: {relative_path}")
    for relative_path in actual:
        if relative_path not in expected:
            raise WebAssetsError(f"Unexpected web asset: {relative_path}")


# ---------------------------------------------------------------------------
# JSON 断言与 noVNC 包校验（对应 Assert-JsonPropertyEquals / Assert-NoVncPackage）
# ---------------------------------------------------------------------------

def _read_json(path: str):
    with open(path, "r", encoding="utf-8-sig") as handle:
        return json.load(handle)


def assert_json_property_equals(actual, expected, name: str) -> None:
    if actual != expected:
        raise WebAssetsError(
            f"Unexpected {name}: expected '{expected}', got '{actual}'"
        )


NOVNC_REQUIRED_FILES = [
    "AUTHORS",
    "LICENSE.txt",
    "docs/LICENSE.MPL-2.0",
    "vendor/pako/LICENSE",
    "core/crypto/des.js",
    "core/rfb.js",
    "package.json",
]


def assert_novnc_package(
    package_root: str, metadata_path: str, attestations_path: str
) -> None:
    for relative_path in NOVNC_REQUIRED_FILES:
        path = os.path.join(package_root, *relative_path.split("/"))
        if not os.path.isfile(path):
            raise WebAssetsError(f"Missing required noVNC file: {relative_path}")

    package = _read_json(os.path.join(package_root, "package.json"))
    assert_json_property_equals(package.get("name"), "@novnc/novnc", "package name")
    assert_json_property_equals(package.get("version"), NOVNC_VERSION, "package version")
    assert_json_property_equals(package.get("license"), "MPL-2.0", "package license")
    if package.get("dependencies"):
        raise WebAssetsError("noVNC package has unexpected runtime dependencies")

    metadata = _read_json(metadata_path)
    assert_json_property_equals(
        metadata.get("version"), NOVNC_VERSION, "npm metadata version"
    )
    assert_json_property_equals(
        metadata.get("gitHead"), NOVNC_COMMIT, "npm metadata gitHead"
    )
    assert_json_property_equals(
        metadata.get("dist", {}).get("tarball"),
        NOVNC_TARBALL,
        "npm metadata tarball",
    )
    assert_json_property_equals(
        metadata.get("dist", {}).get("integrity"),
        NOVNC_INTEGRITY,
        "npm metadata integrity",
    )
    assert_json_property_equals(
        metadata.get("dist", {}).get("shasum"), NOVNC_SHASUM, "npm metadata shasum"
    )

    attestations = _read_json(attestations_path)
    attestation_list = attestations.get("attestations") or []
    if not attestation_list:
        raise WebAssetsError("npm attestation reference is empty")
    if not any(
        item.get("predicateType") == "https://slsa.dev/provenance/v1"
        for item in attestation_list
    ):
        raise WebAssetsError("npm attestation reference lacks SLSA provenance")


# ---------------------------------------------------------------------------
# 浏览器依赖锁校验（对应 Assert-BrowserPackageLock）
# ---------------------------------------------------------------------------

def assert_browser_package_lock(
    package_json_path: str, package_lock_path: str
) -> None:
    package = _read_json(package_json_path)
    dev_dependencies = package.get("devDependencies") or {}
    if len(dev_dependencies) != 1 or dev_dependencies.get("playwright-core") != PLAYWRIGHT_VERSION:
        raise WebAssetsError("Browser package must pin only playwright-core 1.62.1")

    lock = _read_json(package_lock_path)
    assert_json_property_equals(
        lock.get("lockfileVersion"), 3, "npm lockfile version"
    )
    package_entries = lock.get("packages") or {}
    allowed_entries = {"", "node_modules/playwright-core"}
    for entry_name in package_entries:
        if entry_name not in allowed_entries:
            raise WebAssetsError(
                f"Browser lock contains unapproved package: {entry_name}"
            )
    for allowed_entry in allowed_entries:
        if allowed_entry not in package_entries:
            raise WebAssetsError(f"Browser lock is missing package: {allowed_entry}")

    root_entry = package_entries[""]
    assert_json_property_equals(
        (root_entry.get("devDependencies") or {}).get("playwright-core"),
        PLAYWRIGHT_VERSION,
        "root playwright-core version",
    )
    playwright = package_entries["node_modules/playwright-core"]
    assert_json_property_equals(
        playwright.get("version"), PLAYWRIGHT_VERSION, "playwright-core version"
    )
    if playwright.get("resolved") != PLAYWRIGHT_RESOLVED:
        raise WebAssetsError(
            f"Unexpected playwright-core registry source: {playwright.get('resolved')}"
        )
    assert_json_property_equals(
        playwright.get("integrity"), PLAYWRIGHT_INTEGRITY, "playwright-core integrity"
    )
    assert_json_property_equals(
        playwright.get("license"), "Apache-2.0", "playwright-core license"
    )


# ---------------------------------------------------------------------------
# tar 条目与解压树安全（对应 Assert-SafeTarEntries / Assert-SafeExtractedTree）
# ---------------------------------------------------------------------------

def assert_safe_tar_entries(names: list[str], verbose_lines: list[str]) -> None:
    if len(names) != len(verbose_lines):
        raise WebAssetsError("Tar name and verbose entry counts differ")

    for name, verbose_line in zip(names, verbose_lines):
        entry_type = verbose_line[0] if verbose_line else "\x00"
        if entry_type not in ("-", "d"):
            raise WebAssetsError(f"Unsafe tar entry type '{entry_type}': {name}")
        if (
            not name
            or name.isspace()
            or "\\" in name
            or "\x00" in name
            or name.startswith("/")
            or re.match(r"^[A-Za-z]:", name)
            or (name != "package" and not name.startswith("package/"))
        ):
            raise WebAssetsError(f"Unsafe tar path: {name}")

        segments = name.split("/")
        for segment_index, segment in enumerate(segments):
            is_trailing_directory_separator = (
                segment_index == len(segments) - 1
                and not segment
                and entry_type == "d"
            )
            if not is_trailing_directory_separator and (
                not segment or segment in (".", "..")
            ):
                raise WebAssetsError(f"Unsafe tar path: {name}")


def _is_reparse_point(path: str) -> bool:
    if os.name == "nt":
        attributes = os.lstat(path).st_file_attributes
        return bool(attributes & 0x400)  # FILE_ATTRIBUTE_REPARSE_POINT
    return os.path.islink(path)


def assert_safe_extracted_tree(root: str) -> None:
    full_root = os.path.abspath(root)
    assert_safe_temporary_path(full_root)
    for dirpath, dirnames, filenames in os.walk(full_root, followlinks=False):
        for name in dirnames + filenames:
            item_path = os.path.join(dirpath, name)
            if _is_reparse_point(item_path):
                raise WebAssetsError(
                    f"Extracted tree contains a reparse point: {item_path}"
                )
            assert_path_under_root(
                item_path, full_root, "Extracted item is outside its root"
            )


# ---------------------------------------------------------------------------
# 策略输出（对应 Get-NoVncReleasePolicy）
# ---------------------------------------------------------------------------

def get_novnc_release_policy() -> dict:
    return {
        "Version": NOVNC_VERSION,
        "Commit": NOVNC_COMMIT,
        "Tarball": NOVNC_TARBALL,
        "Integrity": NOVNC_INTEGRITY,
        "Shasum": NOVNC_SHASUM,
        "ArchiveSize": NOVNC_ARCHIVE_SIZE,
        "ArchiveSha256": NOVNC_ARCHIVE_SHA256,
        "ArchiveSha512": NOVNC_ARCHIVE_SHA512,
        "MetadataUrl": NOVNC_METADATA_URL,
        "AttestationsUrl": NOVNC_ATTESTATIONS_URL,
    }


# ---------------------------------------------------------------------------
# CLI（供 update-novnc.ps1 等保留的维护脚本调用）
# ---------------------------------------------------------------------------

def _read_lines(path: str) -> list[str]:
    with open(path, "r", encoding="utf-8", newline="") as handle:
        return [line.rstrip("\r\n") for line in handle]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("policy", help="输出 noVNC 发布策略 JSON（stdout）")

    write_parser = subparsers.add_parser("write-manifest", help="写资产树 manifest")
    write_parser.add_argument("--root", required=True)
    write_parser.add_argument("--path", required=True)

    tree_parser = subparsers.add_parser("check-tree", help="校验资产树与 manifest")
    tree_parser.add_argument("--root", required=True)
    tree_parser.add_argument("--manifest", required=True)

    novnc_parser = subparsers.add_parser("check-novnc-package", help="校验 noVNC 包")
    novnc_parser.add_argument("--package-root", required=True)
    novnc_parser.add_argument("--metadata", required=True)
    novnc_parser.add_argument("--attestations", required=True)

    tar_parser = subparsers.add_parser("check-tar-entries", help="校验 tar 条目")
    tar_parser.add_argument("--names", required=True, help="条目名文件（每行一个）")
    tar_parser.add_argument("--verbose", required=True, help="tar -tvf 明细文件")

    extracted_parser = subparsers.add_parser(
        "check-extracted-tree", help="校验解压树安全"
    )
    extracted_parser.add_argument("--root", required=True)

    path_parser = subparsers.add_parser(
        "check-path-under-root", help="校验路径位于指定根之下"
    )
    path_parser.add_argument("--path", required=True)
    path_parser.add_argument("--root", required=True)
    path_parser.add_argument("--message", required=True)

    args = parser.parse_args(argv)

    try:
        if args.command == "policy":
            print(json.dumps(get_novnc_release_policy(), indent=2))
        elif args.command == "write-manifest":
            write_web_asset_manifest(args.root, args.path)
        elif args.command == "check-tree":
            assert_web_asset_tree(args.root, args.manifest)
        elif args.command == "check-novnc-package":
            assert_novnc_package(args.package_root, args.metadata, args.attestations)
        elif args.command == "check-tar-entries":
            assert_safe_tar_entries(
                _read_lines(args.names), _read_lines(args.verbose)
            )
        elif args.command == "check-extracted-tree":
            assert_safe_extracted_tree(args.root)
        elif args.command == "check-path-under-root":
            assert_path_under_root(args.path, args.root, args.message)
        else:  # pragma: no cover - argparse required 已保证
            parser.error(f"unknown command: {args.command}")
    except WebAssetsError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
