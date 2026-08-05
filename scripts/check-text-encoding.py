#!/usr/bin/env python3
"""检查 git 跟踪的文本文件是否均为无 BOM 的合法 UTF-8。

由 cargo-make 的 text-encoding 任务调用（#9 阶段 A）。
替代原 verify.ps1 / verify.sh 内嵌的编码检查逻辑，跨平台单份实现。

与旧实现的行为差异：旧版 fail-fast（首个错误即退出），本版收集全部
违规文件后一次性列出并返回非零退出码，便于一次修复所有问题。
"""

from __future__ import annotations

import pathlib
import subprocess
import sys

# 与既有 verify.ps1 / verify.sh 的受检类型保持一致。
TRACKED_PATTERNS = [
    "*.css",
    "*.html",
    "*.js",
    "*.json",
    "*.md",
    "*.mjs",
    "*.ps1",
    "*.psm1",
    "*.py",
    "*.rs",
    "*.sha256",
    "*.sh",
    "*.toml",
    "*.yaml",
    "*.yml",
    "AGENTS.md",
    "Cargo.lock",
]

UTF8_BOM = b"\xef\xbb\xbf"


def main() -> int:
    listed = subprocess.run(
        ["git", "ls-files", "--", *TRACKED_PATTERNS],
        check=True,
        capture_output=True,
        text=True,
    )
    failures: list[str] = []
    for relative_path in listed.stdout.splitlines():
        path = pathlib.Path(relative_path)
        if not path.is_file():
            continue

        data = path.read_bytes()
        if data.startswith(UTF8_BOM):
            failures.append(f"{relative_path} contains a UTF-8 BOM")
            continue
        try:
            data.decode("utf-8")
        except UnicodeDecodeError as exc:
            failures.append(f"{relative_path} is not valid UTF-8: {exc}")

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
