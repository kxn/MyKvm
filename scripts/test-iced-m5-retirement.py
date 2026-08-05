#!/usr/bin/env python3
"""iced M5 桌面退役门禁（跨平台 Python 单份实现）。

对应原 test-iced-m5-retirement.ps1 / test-iced-m5-retirement.sh（#9 阶段 B3）。
断言旧 egui/eframe 桌面端（ipkvm-desktop、ipkvm-desktop-iced-spike）
已从 workspace 与源码树退役，正式 iced 桌面端具备 Windows GUI 子系统
与图标资源。
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent

_EGUI_TREE_PATTERN = re.compile(r"(?m)(^|[├└]── )(eframe|egui) v")


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
    package_names = [p["name"] for p in metadata["packages"]]
    if "ipkvm-desktop-iced-spike" in package_names:
        raise AssertionError("workspace still contains ipkvm-desktop-iced-spike")

    tree_result = run_command(["cargo", "tree", "--workspace", "--all-features"])
    if tree_result.returncode != 0:
        raise AssertionError(f"cargo tree failed\n{tree_result.stderr}")
    if _EGUI_TREE_PATTERN.search(tree_result.stdout):
        raise AssertionError("workspace dependency tree still contains egui")

    desktop_manifest = (
        REPOSITORY_ROOT / "crates" / "ipkvm-desktop" / "Cargo.toml"
    ).read_text(encoding="utf-8")
    if re.search(r"(?m)^eframe\s*=", desktop_manifest):
        raise AssertionError("ipkvm-desktop still declares eframe")
    if re.search(r"(?m)^wgpu\s*=", desktop_manifest):
        raise AssertionError("ipkvm-desktop still declares wgpu")

    desktop_src = REPOSITORY_ROOT / "crates" / "ipkvm-desktop" / "src"
    if (desktop_src / "main.rs").exists():
        raise AssertionError("egui desktop binary entry still exists")
    if (desktop_src / "app.rs").exists():
        raise AssertionError("egui desktop UI source still exists")
    if (REPOSITORY_ROOT / "crates" / "ipkvm-desktop-iced-spike").exists():
        raise AssertionError("iced spike directory still exists")

    iced_main = (
        REPOSITORY_ROOT / "crates" / "ipkvm-desktop-iced" / "src" / "main.rs"
    ).read_text(encoding="utf-8")
    if '#![cfg_attr(windows, windows_subsystem = "windows")]' not in iced_main:
        raise AssertionError("iced entry lacks Windows GUI subsystem")

    iced_assets = REPOSITORY_ROOT / "crates" / "ipkvm-desktop-iced" / "assets"
    if not (iced_assets / "icon.ico").is_file():
        raise AssertionError("iced Windows icon is missing")
    if not (iced_assets / "icon.rc").is_file():
        raise AssertionError("iced Windows resource script is missing")

    print("M5 desktop retirement gate passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AssertionError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        raise SystemExit(1)
