#!/usr/bin/env python3
"""crate 依赖边界检查（跨平台 Python 单份实现）。

对应原 test-crate-boundaries.ps1 / test-crate-boundaries.sh（#9 阶段 B3）。
断言必需 workspace package 与二进制 target 存在、已退役的 iced spike
不在 workspace，且 headless / browser-fixture / desktop-core 三个库
不泄漏硬件后端或 UI 依赖（检查 cargo tree 输出）。
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent

REQUIRED_PACKAGES = [
    "ipkvm-device",
    "ipkvm-headless",
    "ipkvm-headless-app",
    "ipkvm-headless-demo",
    "ipkvm-browser-fixture",
    "ipkvm-desktop-core",
]

REQUIRED_BINARIES = {
    "ipkvm-headless-app": "ipkvm-headless",
    "ipkvm-headless-demo": "ipkvm-demo",
    "ipkvm-browser-fixture": "ipkvm-browser-fixture",
}

# 各库的泄漏模式（与旧 ps1 版一致；ps1 为超集，覆盖 sh 版未查的场景）
BOUNDARY_PATTERNS = {
    "ipkvm-headless": [
        "serialport",
        "nokhwa",
        "windows v",
        r"ipkvm-video.*camera",
    ],
    "ipkvm-browser-fixture": [
        "serialport",
        "nokhwa",
        r"ipkvm-device.*platform",
        "windows v",
    ],
    "ipkvm-desktop-core": [
        "serialport",
        "nokhwa",
        "iced v",
        "eframe",
        "egui",
        "windows v",
    ],
}


def run_command(argv: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        argv, capture_output=True, text=True, errors="replace"
    )


def main() -> int:
    metadata_result = run_command(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"]
    )
    if metadata_result.returncode != 0:
        raise AssertionError("cargo metadata failed")
    metadata = json.loads(metadata_result.stdout)
    packages = {p["name"]: p for p in metadata["packages"]}

    for name in REQUIRED_PACKAGES:
        if name not in packages:
            raise AssertionError(f"Missing workspace package: {name}")

    for package_name, target_name in REQUIRED_BINARIES.items():
        target_count = sum(
            1 for t in packages[package_name]["targets"]
            if t["name"] == target_name
        )
        if target_count != 1:
            raise AssertionError(
                f"Expected one {target_name} binary in {package_name}, "
                f"got {target_count}"
            )

    if "ipkvm-desktop-iced-spike" in packages:
        raise AssertionError("Retired iced spike remains in workspace")

    for package_name, patterns in BOUNDARY_PATTERNS.items():
        tree_result = run_command(
            ["cargo", "tree", "-p", package_name, "--edges", "normal"]
        )
        if tree_result.returncode != 0:
            raise AssertionError(f"cargo tree failed for {package_name}")
        tree_output = tree_result.stdout
        for pattern in patterns:
            if re.search(pattern, tree_output):
                label = {
                    "ipkvm-headless": "Headless library leaks a hardware backend",
                    "ipkvm-browser-fixture": (
                        "Browser fixture leaks a hardware provider"
                    ),
                    "ipkvm-desktop-core": (
                        "Desktop core leaks UI or hardware dependencies"
                    ),
                }[package_name]
                raise AssertionError(f"{label}: matched '{pattern}'")

    print("Crate boundary checks passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AssertionError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        raise SystemExit(1)
