//! 运行时设置持久化（#141a）：配置目录 helper + `headless-settings.toml` 原子读写。
//!
//! 分层：CLI > `--config` 文件 > 运行时设置 > 默认值。`auto_baud` 本单仅存储
//! 与校验，headless 输入泵暂不消费（自动波特率探测后续接入）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ipkvm_core::{MouseMode, MouseProfile};
use serde::{Deserialize, Serialize};

/// 运行时设置文件名（放在配置目录下）。
pub const SETTINGS_FILE: &str = "headless-settings.toml";

/// 视频缩放模式（前端 noVNC 渲染用）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    #[default]
    FitWindow,
    ActualSize,
    ResizeToVideo,
}

/// 运行时设置（冻结契约，供前端 #140 并行开发）。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WebSettings {
    pub baud_rate: u32,
    pub auto_baud: bool,
    pub preview_fps: u64,
    #[serde(with = "mouse_profile_serde")]
    pub mouse_profile: MouseProfile,
    #[serde(with = "mouse_mode_serde")]
    pub mouse_mode: MouseMode,
    pub relative_sensitivity: f32,
    pub scale_mode: ScaleMode,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            baud_rate: 9_600,
            auto_baud: true,
            preview_fps: 30,
            mouse_profile: MouseProfile::RawAbsolute,
            mouse_mode: MouseMode::Absolute,
            relative_sensitivity: 1.0,
            scale_mode: ScaleMode::FitWindow,
        }
    }
}

#[derive(Deserialize)]
struct WebSettingsFile {
    baud_rate: u32,
    auto_baud: bool,
    preview_fps: u64,
    #[serde(default)]
    mouse_profile: Option<String>,
    #[serde(default)]
    mouse_mode: Option<String>,
    relative_sensitivity: f32,
    scale_mode: ScaleMode,
}

impl<'de> Deserialize<'de> for WebSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as DeError;
        let raw = WebSettingsFile::deserialize(deserializer)?;
        let legacy_mode = raw
            .mouse_mode
            .as_deref()
            .map(|value| match value {
                "absolute" | "Absolute" => Ok(MouseMode::Absolute),
                "relative" | "Relative" => Ok(MouseMode::Relative),
                other => Err(DeError::custom(format!(
                    "mouse_mode: unknown value {other:?}"
                ))),
            })
            .transpose()?;
        let profile = raw
            .mouse_profile
            .as_deref()
            .map(MouseProfile::parse)
            .transpose()
            .map_err(|error| DeError::custom(format!("mouse_profile: {error}")))?
            .unwrap_or_else(|| match legacy_mode.unwrap_or(MouseMode::Absolute) {
                MouseMode::Absolute => MouseProfile::RawAbsolute,
                MouseMode::Relative => MouseProfile::RawRelative,
            });
        Ok(Self {
            baud_rate: raw.baud_rate,
            auto_baud: raw.auto_baud,
            preview_fps: raw.preview_fps,
            mouse_profile: profile,
            mouse_mode: profile.resolve_mode(),
            relative_sensitivity: raw.relative_sensitivity,
            scale_mode: raw.scale_mode,
        })
    }
}

/// `mouse_mode` 的 JSON/TOML 表示使用小写（与冻结契约一致）。
mod mouse_mode_serde {
    use ipkvm_core::MouseMode;
    use serde::Serializer;

    pub fn serialize<S>(mode: &MouseMode, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match mode {
            MouseMode::Absolute => "absolute",
            MouseMode::Relative => "relative",
        })
    }
}

mod mouse_profile_serde {
    use ipkvm_core::MouseProfile;
    use serde::Serializer;

    pub fn serialize<S>(profile: &MouseProfile, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(profile.as_str())
    }
}

/// 校验运行时设置：波特率/帧率/灵敏度范围与契约一致。
///
/// 范围与桌面端设置弹层一致：baud 1200..=115200、fps 1..=60、
/// sensitivity 0.1..=5.0；枚举取值由 serde 保证。
pub fn validate(settings: &WebSettings) -> Result<(), String> {
    if !(1200..=115_200).contains(&settings.baud_rate) {
        return Err(format!(
            "baud_rate 必须在 1200..=115200 之间，得到 {}",
            settings.baud_rate
        ));
    }
    if !(1..=60).contains(&settings.preview_fps) {
        return Err(format!(
            "preview_fps 必须在 1..=60 之间，得到 {}",
            settings.preview_fps
        ));
    }
    if !(0.1..=5.0).contains(&settings.relative_sensitivity) {
        return Err(format!(
            "relative_sensitivity 必须在 0.1..=5.0 之间，得到 {}",
            settings.relative_sensitivity
        ));
    }
    Ok(())
}

/// 平台配置目录（Windows `%APPDATA%\my_ipkvm`、Linux `$XDG_CONFIG_HOME/my_ipkvm`
/// 缺省 `~/.config/my_ipkvm`、macOS `~/Library/Application Support/my_ipkvm`）。
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|path| PathBuf::from(path).join("my_ipkvm"))
    }
    #[cfg(target_os = "linux")]
    {
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"));
        xdg.or(home).map(|path| path.join("my_ipkvm"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/my_ipkvm"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// 设置存储：内存缓存 + `headless-settings.toml` 原子写（临时文件 + rename）。
pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<WebSettings>,
    write_lock: tokio::sync::Mutex<()>,
}

impl SettingsStore {
    /// 从默认配置目录加载；文件缺失 → 默认；损坏/字段非法 → 默认 + 警告。
    pub fn load() -> (Self, Option<String>) {
        Self::load_from(config_dir().unwrap_or_else(|| PathBuf::from(".")))
    }

    /// 从指定目录加载（测试注入用）。
    ///
    /// 文件缺失 → 默认且无警告；TOML 解析失败/字段非法 → 默认 + 警告；
    /// 其他读取错误（权限等）同样回退默认 + 警告。
    pub fn load_from(dir: PathBuf) -> (Self, Option<String>) {
        let path = dir.join(SETTINGS_FILE);
        let (settings, warning) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<WebSettings>(&text) {
                Ok(parsed) => match validate(&parsed) {
                    Ok(()) => (parsed, None),
                    Err(error) => (
                        WebSettings::default(),
                        Some(format!(
                            "设置文件 {} 字段非法，已回退默认：{error}",
                            path.display()
                        )),
                    ),
                },
                Err(error) => (
                    WebSettings::default(),
                    Some(format!(
                        "设置文件 {} 解析失败，已回退默认：{error}",
                        path.display()
                    )),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (WebSettings::default(), None)
            }
            Err(error) => (
                WebSettings::default(),
                Some(format!(
                    "读取设置文件 {} 失败，已回退默认：{error}",
                    path.display()
                )),
            ),
        };
        let store = Self {
            path,
            current: Mutex::new(settings),
            write_lock: tokio::sync::Mutex::new(()),
        };
        (store, warning)
    }

    /// 设置文件路径（测试断言原子写结果用）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 当前内存中的设置快照。
    pub fn get(&self) -> WebSettings {
        self.current.lock().unwrap().clone()
    }

    /// 校验 + 原子写 + 更新内存缓存；并发写由内部 `tokio::sync::Mutex` 串行化。
    ///
    /// 先写同目录临时文件再 `rename` 覆盖目标，保证任意时刻目标文件要么是
    /// 旧完整内容、要么是新完整内容（损坏窗口最小化）。
    pub async fn save(&self, settings: &WebSettings) -> Result<(), String> {
        validate(settings)?;
        let _guard = self.write_lock.lock().await;
        let body = toml::to_string(settings).map_err(|error| format!("序列化设置失败：{error}"))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建设置目录 {}：{error}", parent.display()))?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, &body)
            .map_err(|error| format!("写入临时设置文件 {} 失败：{error}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            format!("替换设置文件 {} 失败：{error}", self.path.display())
        })?;
        *self.current.lock().unwrap() = settings.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ipkvm-headless-settings-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn default_matches_frozen_contract() {
        let settings = WebSettings::default();
        assert_eq!(settings.baud_rate, 9_600);
        assert!(settings.auto_baud);
        assert_eq!(settings.preview_fps, 30);
        assert_eq!(settings.mouse_profile, MouseProfile::RawAbsolute);
        assert_eq!(settings.mouse_mode, MouseMode::Absolute);
        assert_eq!(settings.relative_sensitivity, 1.0);
        assert_eq!(settings.scale_mode, ScaleMode::FitWindow);

        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["baud_rate"], 9_600);
        assert_eq!(json["auto_baud"], true);
        assert_eq!(json["preview_fps"], 30);
        assert_eq!(json["mouse_profile"], "raw_absolute");
        assert_eq!(json["mouse_mode"], "absolute");
        assert_eq!(json["relative_sensitivity"], 1.0);
        assert_eq!(json["scale_mode"], "fit_window");
    }

    #[test]
    fn settings_roundtrip_through_toml_and_json() {
        let settings = WebSettings {
            baud_rate: 57_600,
            auto_baud: false,
            preview_fps: 15,
            mouse_profile: MouseProfile::Linux,
            mouse_mode: MouseMode::Relative,
            relative_sensitivity: 2.5,
            scale_mode: ScaleMode::ResizeToVideo,
        };

        let toml_text = toml::to_string(&settings).unwrap();
        let from_toml: WebSettings = toml::from_str(&toml_text).unwrap();
        assert_eq!(from_toml, settings);
        assert!(toml_text.contains("mouse_mode = \"relative\""));

        let json_text = serde_json::to_string(&settings).unwrap();
        let from_json: WebSettings = serde_json::from_str(&json_text).unwrap();
        assert_eq!(from_json, settings);
        assert!(json_text.contains("\"scale_mode\":\"resize_to_video\""));
    }

    #[test]
    fn validation_accepts_boundaries_and_rejects_out_of_range() {
        let mut settings = WebSettings::default();
        for (baud, fps, sensitivity) in [(1200, 1, 0.1), (115_200, 60, 5.0), (9_600, 30, 1.0)] {
            settings.baud_rate = baud;
            settings.preview_fps = fps;
            settings.relative_sensitivity = sensitivity;
            assert!(
                validate(&settings).is_ok(),
                "边界值 {baud}/{fps}/{sensitivity} 应合法"
            );
        }

        for (baud, fps, sensitivity) in [
            (1199, 30, 1.0),
            (115_201, 30, 1.0),
            (9_600, 0, 1.0),
            (9_600, 61, 1.0),
            (9_600, 30, 0.0),
            (9_600, 30, 5.1),
        ] {
            settings.baud_rate = baud;
            settings.preview_fps = fps;
            settings.relative_sensitivity = sensitivity;
            let error = validate(&settings).unwrap_err();
            assert!(
                error.contains("baud_rate")
                    || error.contains("preview_fps")
                    || error.contains("relative_sensitivity"),
                "越界值 {baud}/{fps}/{sensitivity} 报错应指明字段：{error}"
            );
        }
    }

    #[test]
    fn unknown_enum_values_are_rejected_by_deserialization() {
        let invalid_mouse = r#"{"baud_rate":115200,"auto_baud":true,"preview_fps":30,
            "mouse_mode":"banana","relative_sensitivity":1.0,"scale_mode":"fit_window"}"#;
        assert!(serde_json::from_str::<WebSettings>(invalid_mouse).is_err());

        let invalid_scale = r#"{"baud_rate":115200,"auto_baud":true,"preview_fps":30,
            "mouse_mode":"absolute","relative_sensitivity":1.0,"scale_mode":"bogus"}"#;
        assert!(serde_json::from_str::<WebSettings>(invalid_scale).is_err());
    }

    #[test]
    fn corrupt_or_invalid_file_falls_back_to_default_with_warning() {
        let dir = TempDir::new();
        std::fs::write(dir.path().join(SETTINGS_FILE), "not = valid toml [").unwrap();
        let (store, warning) = SettingsStore::load_from(dir.path().clone());
        assert_eq!(store.get(), WebSettings::default());
        let warning = warning.expect("损坏文件必须返回警告");
        assert!(
            warning.contains("解析失败"),
            "警告应说明解析失败：{warning}"
        );

        let dir = TempDir::new();
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            "baud_rate = 1\nauto_baud = true\npreview_fps = 30\nmouse_mode = \"absolute\"\nrelative_sensitivity = 1.0\nscale_mode = \"fit_window\"\n",
        )
        .unwrap();
        let (store, warning) = SettingsStore::load_from(dir.path().clone());
        assert_eq!(store.get(), WebSettings::default(), "字段非法应回退默认");
        let warning = warning.expect("字段非法必须返回警告");
        assert!(warning.contains("非法"), "警告应说明字段非法：{warning}");
    }

    #[test]
    fn missing_file_loads_defaults_without_warning() {
        let dir = TempDir::new();
        let (store, warning) = SettingsStore::load_from(dir.path().clone());
        assert_eq!(store.get(), WebSettings::default());
        assert!(warning.is_none(), "文件缺失不是异常：{warning:?}");
    }

    #[tokio::test]
    async fn save_writes_atomically_and_reloads() {
        let dir = TempDir::new();
        let (store, warning) = SettingsStore::load_from(dir.path().clone());
        assert!(warning.is_none());

        let changed = WebSettings {
            baud_rate: 57_600,
            mouse_mode: MouseMode::Relative,
            mouse_profile: MouseProfile::RawRelative,
            scale_mode: ScaleMode::ActualSize,
            ..WebSettings::default()
        };
        store.save(&changed).await.unwrap();

        let path = dir.path().join(SETTINGS_FILE);
        assert!(path.exists(), "设置文件应已写入");
        assert!(
            !path.with_extension("toml.tmp").exists(),
            "原子写后临时文件必须已改名"
        );
        assert_eq!(store.get(), changed, "内存缓存应同步更新");

        let (reloaded, warning) = SettingsStore::load_from(dir.path().clone());
        assert!(warning.is_none());
        assert_eq!(reloaded.get(), changed, "重新加载应读到已保存设置");

        let mut again = changed.clone();
        again.baud_rate = 115_200;
        store.save(&again).await.unwrap();
        let (reloaded, _) = SettingsStore::load_from(dir.path().clone());
        assert_eq!(reloaded.get(), again, "再次保存应覆盖旧文件");
    }

    #[tokio::test]
    async fn save_rejects_invalid_settings_without_writing() {
        let dir = TempDir::new();
        let (store, _) = SettingsStore::load_from(dir.path().clone());
        let invalid = WebSettings {
            preview_fps: 0,
            ..WebSettings::default()
        };

        assert!(store.save(&invalid).await.is_err());
        assert!(!dir.path().join(SETTINGS_FILE).exists(), "非法设置不得落盘");
    }

    #[test]
    fn config_dir_contains_my_ipkvm_when_available() {
        if let Some(dir) = config_dir() {
            assert_eq!(
                dir.file_name().and_then(|name| name.to_str()),
                Some("my_ipkvm")
            );
        }
    }
}
