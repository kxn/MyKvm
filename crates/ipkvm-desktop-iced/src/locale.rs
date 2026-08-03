//! 界面语言选择：跟随系统，或显式指定中文/英文（移植 egui desktop locale.rs）。

use rust_i18n::t;

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

    pub fn label(self) -> String {
        match self {
            AppLanguage::System => t!("language.system").to_string(),
            AppLanguage::Chinese => t!("language.chinese").to_string(),
            AppLanguage::English => t!("language.english").to_string(),
        }
    }

    pub fn apply(self) {
        rust_i18n::set_locale(match self {
            AppLanguage::System => detect_system_locale(),
            AppLanguage::Chinese => "zh-CN",
            AppLanguage::English => "en",
        });
    }
}

fn detect_system_locale() -> &'static str {
    map_system_locale(sys_locale::get_locale().as_deref())
}

fn map_system_locale(locale: Option<&str>) -> &'static str {
    match locale {
        Some(locale) if locale.starts_with("zh") => "zh-CN",
        Some(_) => "en",
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
        rust_i18n::set_locale("zh-CN");
    }
}
