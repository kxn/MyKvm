use rust_i18n::t;

/// 界面语言选择：跟随系统，或显式指定中文/英文。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppLanguage {
    System,
    Chinese,
    English,
}

impl AppLanguage {
    pub const ALL: [AppLanguage; 3] = [
        AppLanguage::System,
        AppLanguage::Chinese,
        AppLanguage::English,
    ];

    /// 语言选项在 UI 里的显示名（跟随当前界面语言）。
    pub fn label(self) -> String {
        match self {
            AppLanguage::System => t!("language.system").to_string(),
            AppLanguage::Chinese => t!("language.chinese").to_string(),
            AppLanguage::English => t!("language.english").to_string(),
        }
    }

    /// 应用该语言选择到全局 i18n locale。
    pub fn apply(self) {
        rust_i18n::set_locale(match self {
            AppLanguage::System => detect_system_locale(),
            AppLanguage::Chinese => "zh-CN",
            AppLanguage::English => "en",
        });
    }
}

/// 跟随系统：检测系统 locale，中文（zh*）→ zh-CN，其余 → en；
/// 检测失败时回退到项目默认语言中文。
fn detect_system_locale() -> &'static str {
    map_system_locale(sys_locale::get_locale().as_deref())
}

/// 把系统 locale 字符串映射到受支持的语言代码（纯函数，便于测试）。
fn map_system_locale(locale: Option<&str>) -> &'static str {
    match locale {
        Some(locale) if locale.starts_with("zh") => "zh-CN",
        Some(_) => "en",
        // 检测失败：回退项目默认语言（与 [package.metadata.i18n] default-locale 一致）。
        None => "zh-CN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_system_locales_map_to_zh_cn() {
        for locale in ["zh-CN", "zh-Hans-CN", "zh-TW", "zh-SG"] {
            assert_eq!(map_system_locale(Some(locale)), "zh-CN");
        }
    }

    #[test]
    fn non_chinese_system_locales_map_to_en() {
        for locale in ["en-US", "en-SG", "ja-JP", "de-DE"] {
            assert_eq!(map_system_locale(Some(locale)), "en");
        }
    }

    #[test]
    fn undetectable_system_locale_falls_back_to_zh_cn() {
        assert_eq!(map_system_locale(None), "zh-CN");
    }

    #[test]
    fn explicit_languages_apply_matching_locales() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rust_i18n::set_locale("en");
        AppLanguage::Chinese.apply();
        assert_eq!(&*rust_i18n::locale(), "zh-CN");
        AppLanguage::English.apply();
        assert_eq!(&*rust_i18n::locale(), "en");
        // 结束回到中文，避免影响其他按中文断言的测试（锁内本身已串行，双保险）。
        rust_i18n::set_locale("zh-CN");
    }
}
