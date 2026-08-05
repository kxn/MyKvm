#!/usr/bin/env python3
"""真实浏览器闭环验证（跨平台 Python 单份实现）。

对应原 verify-browser.ps1 / verify-browser.sh（#9 阶段 B4）。
校验 Node 版本、安装锁定依赖、断言 browser-fixture 无 feature 边界、
构建夹具二进制，并以 IPKVM_BROWSER_FIXTURE 环境变量驱动
browser-tests/novnc-browser.mjs 跑真实浏览器闭环。
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
BROWSER_TEST_ROOT = REPOSITORY_ROOT / "browser-tests"
NODE_MIN_MAJOR = 20


def run_command(
    argv: list[str], cwd: Path | None = None, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess:
    return subprocess.run(
        argv,
        cwd=str(cwd) if cwd else None,
        env=env,
        capture_output=True,
        text=True,
        errors="replace",
    )


def run_checked(argv: list[str], name: str, **kwargs) -> subprocess.CompletedProcess:
    print(f"==> {name}", flush=True)
    result = run_command(argv, **kwargs)
    if result.returncode != 0:
        raise RuntimeError(
            f"{name} failed with exit code {result.returncode}: "
            f"{result.stdout}{result.stderr}"
        )
    return result


def get_fixture_executable() -> str:
    """编译 ipkvm-browser-fixture，从 cargo JSON 消息解析唯一可执行文件路径。"""
    result = run_command(
        [
            "cargo", "build",
            "-p", "ipkvm-browser-fixture",
            "--bin", "ipkvm-browser-fixture",
            "--message-format=json-render-diagnostics",
        ],
        cwd=REPOSITORY_ROOT,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"Build browser fixture failed with exit code {result.returncode}: "
            f"{result.stdout}{result.stderr}"
        )

    found: list[str] = []
    for line in (result.stdout + result.stderr).splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            message.get("reason") == "compiler-artifact"
            and message.get("target", {}).get("name") == "ipkvm-browser-fixture"
            and message.get("executable")
        ):
            found.append(os.path.abspath(message["executable"]))

    unique = sorted(set(found))
    if len(unique) != 1:
        raise RuntimeError(
            f"Expected one browser fixture executable, got {len(unique)}"
        )
    if not Path(unique[0]).is_file():
        raise RuntimeError(
            f"Browser fixture executable does not exist: {unique[0]}"
        )
    return unique[0]


def assert_fixture_feature_boundary() -> None:
    """断言 ipkvm-browser-fixture 是独立 package/target 且不要求 feature。"""
    result = run_command(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=REPOSITORY_ROOT,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"Cargo metadata failed with exit code {result.returncode}: "
            f"{result.stderr}"
        )
    metadata = json.loads(result.stdout)
    fixture_packages = [
        p for p in metadata.get("packages", [])
        if p.get("name") == "ipkvm-browser-fixture"
    ]
    if len(fixture_packages) != 1:
        raise RuntimeError(
            "Expected one ipkvm-browser-fixture package in Cargo metadata"
        )
    targets = [
        t for t in fixture_packages[0].get("targets", [])
        if t.get("name") == "ipkvm-browser-fixture"
    ]
    if len(targets) != 1:
        raise RuntimeError("Expected one ipkvm-browser-fixture target")
    required_features = targets[0].get("required-features", [])
    if required_features:
        raise RuntimeError("Browser fixture must not require a package feature")


def main() -> int:
    node_result = run_command(["node", "-p", "process.versions.node"])
    if node_result.returncode != 0:
        raise RuntimeError("Node.js is required for browser verification")
    node_version = node_result.stdout.strip()
    try:
        node_major = int(node_version.split(".")[0])
    except (ValueError, IndexError):
        node_major = 0
    if node_major < NODE_MIN_MAJOR:
        raise RuntimeError(
            f"Node.js {NODE_MIN_MAJOR} or newer is required, got {node_version}"
        )

    run_checked(
        ["npm", "ci", "--ignore-scripts", "--prefix", str(BROWSER_TEST_ROOT)],
        "Install locked browser test dependency",
        cwd=REPOSITORY_ROOT,
    )

    assert_fixture_feature_boundary()

    fixture_path = get_fixture_executable()

    env = os.environ.copy()
    had_fixture_path = "IPKVM_BROWSER_FIXTURE" in env
    previous_fixture_path = env.get("IPKVM_BROWSER_FIXTURE")
    env["IPKVM_BROWSER_FIXTURE"] = fixture_path
    try:
        run_checked(
            ["node", str(BROWSER_TEST_ROOT / "novnc-browser.mjs")],
            "Run real browser verification",
            cwd=REPOSITORY_ROOT,
            env=env,
        )
    finally:
        if had_fixture_path:
            env["IPKVM_BROWSER_FIXTURE"] = previous_fixture_path
        else:
            env.pop("IPKVM_BROWSER_FIXTURE", None)

    print("Real browser verification passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, AssertionError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        raise SystemExit(1)
