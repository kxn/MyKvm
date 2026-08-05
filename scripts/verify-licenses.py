#!/usr/bin/env python3
"""检查当前锁定依赖图的许可证和来源（跨平台 Python 单份实现）。

对应原 verify-licenses.ps1 / verify-licenses.sh（#9 阶段 B2）。
在仓库根运行 cargo-deny --locked check licenses sources。
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from license_policy_tools import get_cargo_deny_executable

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    cargo_deny = get_cargo_deny_executable()
    result = subprocess.run(
        [
            cargo_deny,
            "--locked",
            "check", "licenses", "sources",
        ],
        cwd=str(REPOSITORY_ROOT),
    )
    if result.returncode != 0:
        print(
            f"Dependency license or source check failed with exit code "
            f"{result.returncode}",
            file=sys.stderr,
        )
        return result.returncode
    print("Dependency licenses and sources passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
