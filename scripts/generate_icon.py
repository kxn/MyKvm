#!/usr/bin/env python3
"""生成 my_ipkvm 应用图标（多尺寸 PNG + 多尺寸 ICO + 窗口图标 RGBA）。

几何风格：深蓝灰圆角渐变底 + 显示器（屏幕内青色播放三角）+ 键盘。
全部由代码绘制，无第三方素材，不引入额外许可证义务。
依赖 Pillow：python scripts/generate_icon.py

输出（写入 crates/ipkvm-desktop/assets/）：
  icon-16.png / icon-24.png / icon-32.png / icon-48.png /
  icon-64.png / icon-128.png / icon-256.png
  icon.ico         多尺寸 ICO（16/24/32/48/64/128/256，供 exe 资源嵌入）
  icon-32.rgba     32x32 原始 RGBA（供 egui 窗口图标，include_bytes! 直接使用）
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates" / "ipkvm-desktop" / "assets"

# 以 256 逻辑坐标设计，渲染时 8 倍超采样再降采样，保证小尺寸边缘平滑。
LOGICAL = 256
SUPERSAMPLE = 8
TARGETS = [16, 24, 32, 48, 64, 128, 256]

BG_TOP = (34, 51, 74)
BG_BOTTOM = (18, 26, 38)
BORDER = (56, 78, 104)
FRAME = (62, 90, 120)
SCREEN = (11, 16, 24)
ACCENT = (79, 195, 247)
KEYBOARD = (44, 63, 88)
KEYS = (74, 100, 130)


def draw_master() -> Image.Image:
    size = LOGICAL * SUPERSAMPLE
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    def s(v: float) -> int:
        return int(round(v * SUPERSAMPLE))

    # 圆角渐变底
    gradient = Image.new("RGBA", (size, size))
    gdraw = ImageDraw.Draw(gradient)
    for y in range(size):
        t = y / max(size - 1, 1)
        color = tuple(
            round(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3)
        ) + (255,)
        gdraw.line([(0, y), (size, y)], fill=color)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [s(3), s(3), s(252), s(252)], radius=s(58), fill=255
    )
    img.paste(gradient, (0, 0), mask)

    # 描边
    draw.rounded_rectangle(
        [s(3.5), s(3.5), s(252.5), s(252.5)],
        radius=s(58),
        outline=BORDER + (255,),
        width=max(1, s(1.8)),
    )

    # 显示器外框
    draw.rounded_rectangle(
        [s(62), s(48), s(194), s(156)], radius=s(14), fill=FRAME + (255,)
    )
    # 屏幕
    draw.rounded_rectangle(
        [s(70), s(56), s(186), s(148)], radius=s(8), fill=SCREEN + (255,)
    )
    # 播放三角
    draw.polygon(
        [(s(88), s(80)), (s(88), s(124)), (s(132), s(102))], fill=ACCENT + (255,)
    )
    # 支架与底座
    draw.rectangle([s(120), s(156), s(136), s(171)], fill=FRAME + (255,))
    draw.rounded_rectangle(
        [s(106), s(168), s(150), s(178)], radius=s(5), fill=FRAME + (255,)
    )

    # 键盘
    draw.rounded_rectangle(
        [s(58), s(186), s(198), s(218)], radius=s(12), fill=KEYBOARD + (255,)
    )
    for cx in (88, 128, 168):
        draw.ellipse(
            [s(cx - 5), s(196), s(cx + 5), s(206)], fill=KEYS + (255,)
        )
    draw.rounded_rectangle(
        [s(108), s(208), s(148), s(214)], radius=s(3), fill=KEYS + (255,)
    )

    return img


def main() -> None:
    master = draw_master()
    ASSETS.mkdir(parents=True, exist_ok=True)

    images = {}
    for target in TARGETS:
        img = master.resize((target, target), Image.LANCZOS)
        img.save(ASSETS / f"icon-{target}.png")
        images[target] = img

    images[256].save(
        ASSETS / "icon.ico",
        format="ICO",
        sizes=[(size, size) for size in TARGETS],
    )

    rgba = images[32].convert("RGBA").tobytes()
    (ASSETS / "icon-32.rgba").write_bytes(rgba)

    print(f"generated icons in {ASSETS}")
    print(f"icon-32.rgba: {len(rgba)} bytes (expect {32 * 32 * 4})")


if __name__ == "__main__":
    main()
