#!/usr/bin/env python3
"""检查 headless Web UI 样式是否绕过设计 token（#104 UI 实施规范）。

由 cargo-make 的 web-ui-tokens 任务调用。设计 token 的唯一事实来源是
crates/ipkvm-headless/web/app.css 顶部的 token 定义区，规则细节见
docs/superpowers/specs/2026-08-17-headless-web-ui-redesign-design.md 的
"UI 实施规范"一节。

检查规则（均针对 crates/ipkvm-headless/web/ 下的 .css 文件）：

R1 颜色：token 定义块（:root 与 html[data-theme=...] 规则块）之外禁止
   颜色字面量（#hex、rgb()/rgba()/hsl()/hsla()）。组件必须引用
   var(--*) token，transparent / currentColor 等关键字不受限。
R2 层级：z-index 只能取层级表内的值（Z_INDEX_LAYERS），新增层级必须
   先修改设计文档层级表再同步本脚本。
R3 间距：margin / padding / gap / inset / top / right / bottom / left
   属性中的 px 值必须落在允许集合内（SPACING_ALLOWED = 间距 token 值
   ∪ 历史微调值白名单）。历史值禁止新增使用，新代码应使用
   var(--space-*)；确需新值时先加入 token 或白名单并同步设计文档。

收集全部违规后一次性列出，返回非零退出码。
"""

from __future__ import annotations

import pathlib
import re
import sys

WEB_DIR = pathlib.Path(__file__).resolve().parent.parent / "crates" / "ipkvm-headless" / "web"

# token 定义块的选择器：块内允许颜色字面量（token 值本身）。
TOKEN_BLOCK_SELECTORS = re.compile(r"^(:root|html\[data-theme=.*\])$")

COLOR_LITERAL = re.compile(
    r"#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\("
)

# 层级表（与设计文档"UI 实施规范"一节的 z-index 层级表保持一致）。
Z_INDEX_LAYERS = {0, 4, 10, 40, 50, 60, 100}

# 间距 token 值（--space-1..8）∪ 历史微调值白名单（禁止新增使用）。
SPACING_TOKENS = {4, 8, 12, 16, 20, 24, 32}
SPACING_LEGACY = {1, 2, 3, 5, 6, 7, 10, 14, 18, 22, 28, 40, 48, 60, 880}
SPACING_ALLOWED = SPACING_TOKENS | SPACING_LEGACY

SPACING_PROPERTIES = re.compile(
    r"^(margin|padding|gap|inset|top|right|bottom|left)(-[a-z]+)?$"
)
PX_VALUE = re.compile(r"(-?\d+(?:\.\d+)?)px")


def strip_comments(text: str) -> str:
    return re.sub(r"/\*.*?\*/", lambda m: "\n" * m.group(0).count("\n"), text, flags=re.S)


def iter_rule_blocks(text: str):
    """产出 (selector, body, body_start_line)。仅支持一层嵌套，够本仓库用。"""
    depth = 0
    selector_start = 0
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "{":
            if depth == 0:
                selector = text[selector_start:i].strip().split("\n")[-1].strip()
                body_start = i + 1
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                body = text[body_start:i]
                line = text.count("\n", 0, body_start) + 1
                yield selector, body, line
            selector_start = i + 1
        i += 1


def check_file(path: pathlib.Path) -> list[str]:
    text = strip_comments(path.read_text(encoding="utf-8"))
    failures: list[str] = []
    for selector, body, body_line in iter_rule_blocks(text):
        in_token_block = bool(TOKEN_BLOCK_SELECTORS.match(selector))
        # 按分号拆声明而不是按行，避免单行多条声明绕过检查。
        cursor = 0
        for declaration in body.split(";"):
            lineno = body_line + body.count("\n", 0, cursor)
            cursor += len(declaration) + 1
            stripped = declaration.strip()
            if not stripped or ":" not in stripped:
                continue
            prop = stripped.split(":", 1)[0].strip().lower()
            if not in_token_block:
                match = COLOR_LITERAL.search(stripped)
                if match:
                    failures.append(
                        f"{path.name}:{lineno}: R1 color literal {match.group(0)!r} outside "
                        "token definition blocks; use var(--*) tokens"
                    )
            if prop == "z-index":
                value = stripped.split(":", 1)[1].strip()
                if value.isdigit() and int(value) not in Z_INDEX_LAYERS:
                    failures.append(
                        f"{path.name}:{lineno}: R2 z-index {value} not in layer table; "
                        "new layers require updating the design doc and this script"
                    )
            if SPACING_PROPERTIES.match(prop):
                for px in PX_VALUE.finditer(stripped):
                    value = abs(float(px.group(1)))
                    if value == int(value) and int(value) in SPACING_ALLOWED:
                        continue
                    failures.append(
                        f"{path.name}:{lineno}: R3 spacing value {px.group(0)} not in the "
                        "allowed set; use var(--space-*) tokens"
                    )
    return failures


def main() -> int:
    css_files = sorted(WEB_DIR.glob("*.css"))
    if not css_files:
        print(f"web-ui-tokens: no css files under {WEB_DIR}", file=sys.stderr)
        return 1
    failures: list[str] = []
    for path in css_files:
        failures.extend(check_file(path))
    if failures:
        print("web-ui-tokens check failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"web-ui-tokens: {len(css_files)} css file(s) OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
