#!/usr/bin/env python3
"""cargo-deny 工具函数（跨平台 Python 单份实现）。

对应原 license-policy-tools.psm1 / license-policy-tools.sh（#9 阶段 B2）。
提供 cargo-deny 版本校验与可执行文件定位，供 verify-licenses.py 与
test-license-policy.py 使用。
"""

from __future__ import annotations

import re
import shutil
import subprocess

REQUIRED_CARGO_DENY_VERSION = "0.20.2"
CARGO_DENY_INSTALL_COMMAND = (
    "cargo install --locked --version 0.20.2 cargo-deny"
)

_VERSION_PATTERN = re.compile(
    r"^cargo-deny\s+(\d+\.\d+\.\d+)(?:\s.*)?$"
)


class LicensePolicyError(RuntimeError):
    """许可证策略检查失败。"""


def assert_cargo_deny_version(version_output: str) -> str:
    """解析并校验 cargo-deny --version 输出，返回实际版本号。

    版本不符或无法解析时抛出 LicensePolicyError，附带安装指引。
    """
    match = _VERSION_PATTERN.match(version_output.strip())
    if not match:
        raise LicensePolicyError(
            f"无法解析 cargo-deny 版本。请执行：{CARGO_DENY_INSTALL_COMMAND}"
        )

    actual = match.group(1)
    if actual != REQUIRED_CARGO_DENY_VERSION:
        raise LicensePolicyError(
            f"cargo-deny 版本不符：预期 {REQUIRED_CARGO_DENY_VERSION}，"
            f"实际 {actual}。请执行：{CARGO_DENY_INSTALL_COMMAND}"
        )
    return actual


def get_cargo_deny_executable() -> str:
    """定位 cargo-deny 可执行文件并校验版本，返回其路径。"""
    path = shutil.which("cargo-deny")
    if path is None:
        raise LicensePolicyError(
            f"未找到 cargo-deny。请执行：{CARGO_DENY_INSTALL_COMMAND}"
        )

    result = subprocess.run(
        [path, "--version"], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise LicensePolicyError(
            f"cargo-deny --version 执行失败，退出码：{result.returncode}"
        )
    assert_cargo_deny_version(result.stdout)
    return path
