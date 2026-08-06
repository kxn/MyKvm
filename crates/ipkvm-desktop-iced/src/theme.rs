//! my_ipkvm 自定义主题：亮/暗 Palette 与派生色纯函数。

use iced::Color;
use iced::theme::{Palette, Theme};
use iced::widget::{button, pick_list, text_input};
use iced::{Background, Border, Shadow, Vector};

/// 模态卡片固定宽度（不再横向填充，居中显示）。
pub const PANEL_WIDTH: f32 = 440.0;
pub const PANEL_RADIUS: f32 = 12.0;
pub const CONTROL_HEIGHT: f32 = 34.0;
pub const CONTROL_RADIUS: f32 = 6.0;

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

pub fn button_style(theme: &Theme, status: button::Status, primary: bool) -> button::Style {
    let palette = theme.palette();
    let background = if primary {
        palette.primary
    } else {
        surface(palette)
    };
    let background = match status {
        button::Status::Hovered => hover(palette),
        button::Status::Pressed => Color::from_rgba(
            palette.primary.r,
            palette.primary.g,
            palette.primary.b,
            if primary { 0.82 } else { 0.26 },
        ),
        button::Status::Disabled => {
            Color::from_rgba(background.r, background.g, background.b, 0.45)
        }
        button::Status::Active => background,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if primary { Color::WHITE } else { palette.text },
        border: Border::default()
            .rounded(CONTROL_RADIUS)
            .width(1.0)
            .color(border_color(palette)),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.10),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 2.0,
        },
        snap: false,
    }
}

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    button_style(theme, status, true)
}

pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    button_style(theme, status, false)
}

/// 危险操作按钮（覆盖保存确认）：danger 底 + 白字。
pub fn danger_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.palette();
    let background = match status {
        button::Status::Hovered => {
            Color::from_rgba(palette.danger.r, palette.danger.g, palette.danger.b, 0.88)
        }
        button::Status::Pressed => {
            Color::from_rgba(palette.danger.r, palette.danger.g, palette.danger.b, 0.72)
        }
        button::Status::Disabled => {
            Color::from_rgba(palette.danger.r, palette.danger.g, palette.danger.b, 0.45)
        }
        button::Status::Active => palette.danger,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border::default()
            .rounded(CONTROL_RADIUS)
            .width(1.0)
            .color(palette.danger),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.10),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 2.0,
        },
        snap: false,
    }
}

/// 模态标题栏关闭按钮：无底透明，悬停文本变 danger 红。
pub fn close_button_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.palette();
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => palette.danger,
        button::Status::Disabled => {
            Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.35)
        }
        button::Status::Active => palette.text,
    };
    let background = match status {
        button::Status::Hovered => {
            Color::from_rgba(palette.danger.r, palette.danger.g, palette.danger.b, 0.10)
        }
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().rounded(CONTROL_RADIUS),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// 文本输入框：内凹表面、圆角边框、聚焦时主色高亮。
pub fn text_input_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.palette();
    let focused = matches!(status, text_input::Status::Focused { .. });
    let border_color = if focused {
        palette.primary
    } else {
        match status {
            text_input::Status::Hovered => {
                Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.35)
            }
            _ => border_color(palette),
        }
    };
    text_input::Style {
        background: Background::Color(palette.background),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            width: if focused { 1.5 } else { 1.0 },
            color: border_color,
        },
        icon: Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.5),
        placeholder: Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.40),
        value: palette.text,
        selection: Color::from_rgba(
            palette.primary.r,
            palette.primary.g,
            palette.primary.b,
            0.30,
        ),
    }
}

/// 下拉选择框：与文本输入框同款表面/边框/聚焦高亮。
pub fn pick_list_style(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let palette = theme.palette();
    let focused = matches!(status, pick_list::Status::Opened { .. });
    let border_color = if focused {
        palette.primary
    } else {
        match status {
            pick_list::Status::Hovered => {
                Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.35)
            }
            _ => border_color(palette),
        }
    };
    pick_list::Style {
        text_color: palette.text,
        placeholder_color: Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.40),
        handle_color: Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.55),
        background: Background::Color(palette.background),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            width: if focused { 1.5 } else { 1.0 },
            color: border_color,
        },
    }
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

    #[test]
    fn visual_tokens_keep_panels_and_controls_compact() {
        let panel_width = PANEL_WIDTH;
        let control_height = CONTROL_HEIGHT;
        let panel_radius = PANEL_RADIUS;
        let control_radius = CONTROL_RADIUS;
        assert!((400.0..=480.0).contains(&panel_width));
        assert!((30.0..=38.0).contains(&control_height));
        assert!(panel_radius <= 14.0 && control_radius <= 8.0);
    }
}
