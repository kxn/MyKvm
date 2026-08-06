#!/usr/bin/env python3
"""验收检查：README.md 的「Linux 桌面端说明」小节与失效 issue 引用（issue #37 收尾）。

断言：
1. 「Linux 桌面端说明」小节（自该标题行起至下一个 `## ` 标题止）内不再出现
   `--list-cameras` 或 `--assets`：这两个是 ipkvm-headless（headless-app）的 CLI
   参数，桌面 iced 应用（main.rs 只调用 ipkvm_desktop_iced::run()，不解析 CLI
   参数）不支持，选设备走 UI。
2. README 全文不再出现 `自动恢复见 issue #37`：这是 Gitea 时代遗留的失效引用，
   指向旧追踪器上"CH9329 掉线自动恢复"的 issue，与 GitHub 上本次 Linux 桌面
   支持任务 #37 无关。
3. 正向断言：小节仍保留 Linux V4L2 / 应用内设备列表选择的事实描述，防止整节
   被删导致检查空转。

用法：在仓库根目录运行 `python3 scripts/check-readme-linux-desktop.py`。
"""

import sys
from pathlib import Path

README = Path("README.md")
SECTION_HEADER = "Linux 桌面端说明："


def main() -> int:
    if not README.is_file():
        print(f"FAIL: 未找到 {README}", file=sys.stderr)
        return 1

    text = README.read_text(encoding="utf-8")
    lines = text.splitlines()

    # 定位「Linux 桌面端说明」小节范围：标题行起，至下一个 `## ` 标题止。
    header_idx = next(
        (i for i, ln in enumerate(lines) if ln.strip() == SECTION_HEADER), None
    )
    if header_idx is None:
        print(f"FAIL: 未找到小节标题「{SECTION_HEADER}」", file=sys.stderr)
        return 1

    end_idx = next(
        (i for i in range(header_idx + 1, len(lines)) if lines[i].startswith("## ")),
        len(lines),
    )
    section = "\n".join(lines[header_idx:end_idx])

    errors = []

    for flag in ("--list-cameras", "--assets"):
        if flag in section:
            errors.append(
                f"「Linux 桌面端说明」小节内仍出现 {flag}"
                "（该参数属 headless-app，桌面端不支持）"
            )

    stale = "自动恢复见 issue #37"
    if stale in text:
        errors.append(f"README 全文仍出现失效引用「{stale}」")

    # 正向断言：小节仍保留关键事实描述。
    for required in ("Linux V4L2", "设备列表"):
        if required not in section:
            errors.append(
                f"「Linux 桌面端说明」小节缺少关键描述「{required}」，疑似整节被删"
            )

    if errors:
        print("FAIL: README.md 未通过 Linux 桌面端说明检查：", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("PASS: README.md Linux 桌面端说明检查通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
