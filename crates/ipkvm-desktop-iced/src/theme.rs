//! my_ipkvm 自定义主题：亮/暗 Palette 与派生色纯函数。

use iced::Color;
use iced::theme::{Palette, Theme};

/// 亮色主题（默认，沿用迁移前桌面端跟随系统的浅色观感，#95 确认）。
pub const LIGHT: Palette = Palette {
    background: Color::from_rgb(0.96, 0.96, 0.97),
    text: Color::from_rgb(0.12, 0.12, 0.14),
    primary: Color::from_rgb(0.20, 0.42, 0.82),
    success: Color::from_rgb(0.12, 0.55, 0.35),
    warning: Color::from_rgb(0.78, 0.55, 0.12),
    danger: Color::from_rgb(0.76, 0.20, 0.20),
};

/// 暗色主题（设置模态中切换：KVM 监控类工具深色观感）。
pub const DARK: Palette = Palette {
    background: Color::from_rgb(0.13, 0.14, 0.17),
    text: Color::from_rgb(0.90, 0.90, 0.92),
    primary: Color::from_rgb(0.34, 0.56, 0.95),
    success: Color::from_rgb(0.30, 0.72, 0.52),
    warning: Color::from_rgb(0.92, 0.66, 0.28),
    danger: Color::from_rgb(0.90, 0.34, 0.32),
};

/// 按模式返回应用主题（iced application builder 使用）。
pub fn app_theme(dark: bool) -> Theme {
    Theme::custom("my_ipkvm", if dark { DARK } else { LIGHT })
}

/// 弹出面板/卡片表面色：背景与文本按 6% 混合，亮色模式更亮、暗色模式更暗。
pub fn surface(palette: Palette) -> Color {
    mix(palette.background, palette.text, 0.06)
}

/// 选中/悬停高亮：主题主色半透明。
pub fn hover(palette: Palette) -> Color {
    Color::from_rgba(
        palette.primary.r,
        palette.primary.g,
        palette.primary.b,
        0.18,
    )
}

/// 面板边框：文本色低透明度。
pub fn border_color(palette: Palette) -> Color {
    Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.16)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color) -> f32 {
        fn lin(v: f32) -> f32 {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn light_and_dark_palettes_differ() {
        assert_ne!(LIGHT, DARK);
        assert_ne!(LIGHT.background, DARK.background);
        assert_ne!(LIGHT.text, DARK.text);
    }

    #[test]
    fn text_background_contrast_is_readable_in_both_modes() {
        for palette in [LIGHT, DARK] {
            assert!(
                contrast(palette.text, palette.background) >= 4.5,
                "文本/背景对比度必须 >= 4.5"
            );
        }
    }

    #[test]
    fn app_theme_returns_custom_palette_for_mode() {
        assert_eq!(app_theme(true).palette(), DARK);
        assert_eq!(app_theme(false).palette(), LIGHT);
    }

    #[test]
    fn derived_colors_are_distinct_and_opaque() {
        for palette in [LIGHT, DARK] {
            let s = surface(palette);
            let h = hover(palette);
            let b = border_color(palette);
            assert_ne!(s, palette.background, "surface 不得等于背景");
            assert!(h.a > 0.0 && h.a < 1.0, "hover 必须是半透明高亮");
            assert!(b.a > 0.0, "边框色必须可见");
        }
    }
}
