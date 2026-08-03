//! 按界面语言选择字体：zh 用系统微软雅黑，en 用内置 Poppins（SIL OFL 1.1）。
//! Poppins 不含中文字形，中文字符由 cosmic-text 自动回退到系统字体。

use iced::Font;

/// 当前语言对应的 UI 字体（读取进程级 locale）。
pub fn ui_font() -> Font {
    if rust_i18n::locale().starts_with("zh") {
        Font::with_name("Microsoft YaHei UI")
    } else {
        Font::with_name("Poppins")
    }
}

/// 内置 Poppins 字体字节（启动时 iced::font::load）。
pub const POPPINS_REGULAR: &[u8] = include_bytes!("../assets/fonts/Poppins-Regular.ttf");
pub const POPPINS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Poppins-Medium.ttf");
pub const POPPINS_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Poppins-SemiBold.ttf");
pub const POPPINS_BOLD: &[u8] = include_bytes!("../assets/fonts/Poppins-Bold.ttf");

/// 启动加载全部 Poppins 字重，结果映射为 `Message::FontsLoaded`（失败由调用方吞掉）。
pub fn load_tasks() -> Vec<iced::Task<crate::app::Message>> {
    [
        POPPINS_REGULAR,
        POPPINS_MEDIUM,
        POPPINS_SEMIBOLD,
        POPPINS_BOLD,
    ]
    .into_iter()
    .map(|bytes| iced::font::load(bytes).map(crate::app::Message::FontsLoaded))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_font_is_yahei_for_chinese_and_poppins_for_english() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rust_i18n::set_locale("zh-CN");
        assert_eq!(ui_font(), Font::with_name("Microsoft YaHei UI"));
        rust_i18n::set_locale("en");
        assert_eq!(ui_font(), Font::with_name("Poppins"));
    }

    #[test]
    fn poppins_constants_are_nonempty_ttf_files() {
        for (name, bytes) in [
            ("regular", POPPINS_REGULAR),
            ("medium", POPPINS_MEDIUM),
            ("semibold", POPPINS_SEMIBOLD),
            ("bold", POPPINS_BOLD),
        ] {
            assert!(!bytes.is_empty(), "Poppins-{name} 字体字节不得为空");
            assert_eq!(
                &bytes[..4],
                b"\x00\x01\x00\x00",
                "Poppins-{name} 必须是 TTF（魔数 \\x00\\x01\\x00\\x00）"
            );
        }
    }
}
