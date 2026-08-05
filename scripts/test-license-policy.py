#!/usr/bin/env python3
"""依赖许可证策略自测（跨平台 Python 单份实现）。

对应原 test-license-policy.ps1 / test-license-policy.sh（#9 阶段 B2）。
用临时正负向夹具验证 deny.toml 策略：允许的宽松许可证通过、
被拒许可证（GPL-3.0-only）与未指定 rev 的 git 来源被 cargo-deny 拒绝。
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from license_policy_tools import (
    REQUIRED_CARGO_DENY_VERSION,
    LicensePolicyError,
    assert_cargo_deny_version,
    get_cargo_deny_executable,
)

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent


def run_command(argv: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess:
    """运行命令并合并 stdout/stderr（与旧脚本 2>&1 行为一致）。"""
    return subprocess.run(
        argv,
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
        errors="replace",
    )


def assert_succeeded(
    result: subprocess.CompletedProcess, name: str
) -> None:
    if result.returncode != 0:
        raise AssertionError(
            f"{name} failed with exit code {result.returncode}: "
            f"{result.stdout}{result.stderr}"
        )


def assert_rejected(
    result: subprocess.CompletedProcess,
    expected_exit: int,
    patterns: list[str],
    name: str,
) -> None:
    output = f"{result.stdout}{result.stderr}"
    if result.returncode != expected_exit:
        raise AssertionError(
            f"{name} returned {result.returncode}, expected {expected_exit}: "
            f"{output}"
        )
    for pattern in patterns:
        if not re.search(pattern, output):
            raise AssertionError(
                f"{name} output did not match '{pattern}': {output}"
            )


def write_utf8(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8", newline="\n")


def new_path_dependency_fixture(root: Path, dependency_license: str) -> None:
    write_utf8(
        root / "Cargo.toml",
        """[package]
name = "policy-fixture-app"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[dependencies]
policy-fixture-dependency = { path = "dependency" }

[workspace]
members = ["dependency"]
""",
    )
    write_utf8(
        root / "dependency" / "Cargo.toml",
        f"""[package]
name = "policy-fixture-dependency"
version = "0.1.0"
edition = "2024"
license = "{dependency_license}"
publish = false
""",
    )
    write_utf8(root / "src" / "lib.rs", "pub fn app() {}\n")
    write_utf8(root / "dependency" / "src" / "lib.rs", "pub fn dependency() {}\n")


def new_git_dependency_fixture(root: Path) -> Path:
    """创建本地 git 依赖夹具，返回消费方 crate 根目录。"""
    dependency_root = root / "git-dependency"
    consumer_root = root / "git-consumer"

    write_utf8(
        dependency_root / "Cargo.toml",
        """[package]
name = "policy-git-dependency"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false
""",
    )
    write_utf8(dependency_root / "src" / "lib.rs", "pub fn dependency() {}\n")

    for argv in (
        ["git", "-C", str(dependency_root), "init"],
        [
            "git", "-C", str(dependency_root), "config", "user.name",
            "my_ipkvm policy test",
        ],
        [
            "git", "-C", str(dependency_root), "config", "user.email",
            "policy-test@invalid.local",
        ],
        ["git", "-C", str(dependency_root), "add", "."],
        ["git", "-C", str(dependency_root), "commit", "-m", "fixture"],
    ):
        assert_succeeded(run_command(argv), "Initialize fixture Git repository")

    dependency_uri = dependency_root.resolve().as_uri()
    write_utf8(
        consumer_root / "Cargo.toml",
        f"""[package]
name = "policy-git-consumer"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[dependencies]
policy-git-dependency = {{ git = "{dependency_uri}" }}

[workspace]
""",
    )
    write_utf8(consumer_root / "src" / "lib.rs", "pub fn consumer() {}\n")
    return consumer_root


def main() -> int:
    if REQUIRED_CARGO_DENY_VERSION != "0.20.2":
        raise AssertionError("Required version is not 0.20.2")

    if assert_cargo_deny_version("cargo-deny 0.20.2") != "0.20.2":
        raise AssertionError("Expected version was rejected")

    try:
        assert_cargo_deny_version("cargo-deny 0.20.1")
    except LicensePolicyError as exc:
        if not re.search(r"预期 0\.20\.2.*实际 0\.20\.1", str(exc)):
            raise AssertionError(
                f"Version mismatch message did not match: {exc}"
            )
    else:
        raise AssertionError("Expected version mismatch was accepted")

    try:
        assert_cargo_deny_version("not a version")
    except LicensePolicyError as exc:
        if "无法解析 cargo-deny 版本" not in str(exc):
            raise AssertionError(f"Unparseable version message did not match: {exc}")
    else:
        raise AssertionError("Unparseable version was accepted")

    deny_config = REPOSITORY_ROOT / "deny.toml"
    if not deny_config.is_file():
        raise AssertionError("deny.toml was not found at repository root")

    cargo_deny = get_cargo_deny_executable()
    fixture_root = Path(
        tempfile.mkdtemp(prefix="my-ipkvm-license-policy-")
    )
    try:
        path_fixture = fixture_root / "path-dependency"
        new_path_dependency_fixture(path_fixture, "BSD-3-Clause")
        lock_result = run_command(
            [
                "cargo", "generate-lockfile",
                "--manifest-path", str(path_fixture / "Cargo.toml"),
                "--offline",
            ]
        )
        assert_succeeded(lock_result, "Generate allowed fixture lock file")

        allowed = run_command(
            [
                cargo_deny,
                "--config", str(deny_config),
                "--manifest-path", str(path_fixture / "Cargo.toml"),
                "--locked",
                "check", "licenses", "sources",
            ]
        )
        assert_succeeded(allowed, "Allowed license fixture")

        new_path_dependency_fixture(path_fixture, "GPL-3.0-only")
        rejected_license = run_command(
            [
                cargo_deny,
                "--config", str(deny_config),
                "--manifest-path", str(path_fixture / "Cargo.toml"),
                "--locked",
                "check", "licenses", "sources",
            ]
        )
        assert_rejected(
            rejected_license,
            4,
            ["rejected", r"GPL-3\.0-only"],
            "Rejected license fixture",
        )

        git_consumer = new_git_dependency_fixture(fixture_root)
        git_lock = run_command(
            [
                "cargo", "generate-lockfile",
                "--manifest-path", str(git_consumer / "Cargo.toml"),
            ]
        )
        assert_succeeded(git_lock, "Generate Git fixture lock file")

        rejected_git = run_command(
            [
                cargo_deny,
                "--config", str(deny_config),
                "--manifest-path", str(git_consumer / "Cargo.toml"),
                "--locked",
                "check", "sources",
            ]
        )
        assert_rejected(
            rejected_git,
            8,
            ["source-not-allowed", "git-source-underspecified", "file://"],
            "Rejected Git source fixture",
        )
    finally:
        shutil.rmtree(fixture_root, ignore_errors=True)

    print("Dependency license policy tests passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, LicensePolicyError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        raise SystemExit(1)
