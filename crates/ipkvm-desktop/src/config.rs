//! 连接 profile、最近使用与“上次手动连接”的本地持久化。
//!
//! 存储布局（所有写入先写临时文件再重命名，避免半截文件）：
//! - `<配置目录>/profiles/<名字>.toml`：每个 profile 一个文件，文件名即显示名；
//! - `<配置目录>/config.toml`：最近使用 profile 列表 + 上次手动连接快照。
//!
//! 配置目录：Windows `%APPDATA%\my_ipkvm`；Linux
//! `$XDG_CONFIG_HOME/my_ipkvm`（缺省 `~/.config/my_ipkvm`）；
//! macOS `~/Library/Application Support/my_ipkvm`。

use std::fs;
use std::path::{Path, PathBuf};

use ipkvm_core::MouseMode;
use serde::{Deserialize, Serialize};

/// 最近使用列表上限（菜单 3 条直显 + “更多”二级菜单内共 10 条）。
pub const RECENT_LIMIT: usize = 10;
/// 文件对话框默认目录使用的 profile 文件扩展名。
pub const PROFILE_EXTENSION: &str = "toml";

/// 设备引用：id 用于匹配当前枚举，label 用于显示/兜底。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceRef {
    pub id: String,
    pub label: String,
}

/// 连接相关参数：菜单“设置…”里保存的是默认值，主页“连接设置”是连接级
/// 副本，可被 profile 覆盖；本地视图偏好（缩放/语言）不属于这里。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionSettings {
    pub baud_rate: u32,
    pub auto_baud: bool,
    pub preview_fps: u64,
    #[serde(with = "mouse_mode_serde")]
    pub mouse_mode: MouseMode,
    pub relative_sensitivity: f32,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            baud_rate: ipkvm_core::DEFAULT_BAUD_RATE,
            // 默认绝对模式（进系统体验更好）；BIOS/启动菜单若绝对 HID 映射不对，
            // 用 Ctrl+Alt+M 切相对模式。
            mouse_mode: MouseMode::Absolute,
            preview_fps: 30,
            relative_sensitivity: 1.0,
            auto_baud: true,
        }
    }
}

/// 单个连接 profile 文件的内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_device: Option<DeviceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_device: Option<DeviceRef>,
    pub connection: ConnectionSettings,
}

/// 上次手动连接快照：启动连接界面时的默认预填值（profile 连接不覆盖它）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManualSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_device: Option<DeviceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_device: Option<DeviceRef>,
    pub connection: ConnectionSettings,
}

/// 全局配置（config.toml）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_manual: Option<ManualSnapshot>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileStoreError {
    #[error("failed to create {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Profile 存储：封装配置目录、profile 目录与 config.toml。
#[derive(Clone, Debug)]
pub struct ProfileStore {
    base_dir: PathBuf,
}

impl ProfileStore {
    /// 生产路径：按平台配置目录（找不到时退回当前目录，保证不 panic）。
    pub fn production() -> Self {
        Self::new(config_base_dir().unwrap_or_else(|| PathBuf::from(".")))
    }

    /// 测试/自定义路径：直接指定配置根目录。
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.base_dir.join("profiles")
    }

    fn config_path(&self) -> PathBuf {
        self.base_dir.join("config.toml")
    }

    fn ensure_dirs(&self) -> Result<(), ProfileStoreError> {
        fs::create_dir_all(self.profiles_dir()).map_err(|source| ProfileStoreError::CreateDir {
            path: self.profiles_dir(),
            source,
        })?;
        fs::create_dir_all(&self.base_dir).map_err(|source| ProfileStoreError::CreateDir {
            path: self.base_dir.clone(),
            source,
        })
    }

    fn profile_path(&self, name: &str) -> PathBuf {
        let mut file_name = name.to_string();
        if !file_name.ends_with(&format!(".{PROFILE_EXTENSION}")) {
            file_name.push('.');
            file_name.push_str(PROFILE_EXTENSION);
        }
        self.profiles_dir().join(file_name)
    }

    /// 列出全部 profile 名（*.toml 文件名去扩展名，按名字排序）。
    pub fn list_profiles(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.profiles_dir()) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == PROFILE_EXTENSION)
            })
            .filter_map(|entry| {
                entry
                    .path()
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        names
    }

    pub fn profile_exists(&self, name: &str) -> bool {
        self.profile_path(name).is_file()
    }

    /// 按名字加载 profile。
    pub fn load_profile(&self, name: &str) -> Result<Profile, ProfileStoreError> {
        let path = self.profile_path(name);
        let bytes = fs::read(&path).map_err(|source| ProfileStoreError::Read {
            path: path.clone(),
            source,
        })?;
        let mut profile: Profile =
            toml::from_str(&String::from_utf8_lossy(&bytes)).map_err(|error| {
                ProfileStoreError::Parse {
                    path: path.clone(),
                    reason: error.to_string(),
                }
            })?;
        if profile.name.is_empty() {
            profile.name = name.to_string();
        }
        Ok(profile)
    }

    /// 从任意文件加载 profile（文件对话框路径；显示名取文件名）。
    pub fn load_profile_file(&self, path: &Path) -> Result<Profile, ProfileStoreError> {
        let bytes = fs::read(path).map_err(|source| ProfileStoreError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut profile: Profile =
            toml::from_str(&String::from_utf8_lossy(&bytes)).map_err(|error| {
                ProfileStoreError::Parse {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                }
            })?;
        if profile.name.is_empty() {
            profile.name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "profile".to_string());
        }
        Ok(profile)
    }

    /// 保存 profile（原子写：临时文件 + 重命名）。
    pub fn save_profile(&self, profile: &Profile) -> Result<(), ProfileStoreError> {
        self.ensure_dirs()?;
        let path = self.profile_path(&profile.name);
        atomic_write(
            &path,
            &toml::to_string(profile).expect("profile serialization"),
        )
    }

    /// 最近使用 profile 名列表（最多 RECENT_LIMIT 条）。
    pub fn recent_profiles(&self) -> Vec<String> {
        self.read_config().recent_profiles
    }

    /// 记录一次 profile 连接：去重后置顶，上限 RECENT_LIMIT。
    pub fn add_recent_profile(&self, name: &str) -> Result<(), ProfileStoreError> {
        let mut config = self.read_config();
        config.recent_profiles.retain(|existing| existing != name);
        config.recent_profiles.insert(0, name.to_string());
        config.recent_profiles.truncate(RECENT_LIMIT);
        self.write_config(&config)
    }

    /// 上次手动连接快照（打开连接界面时的默认预填）。
    pub fn last_manual(&self) -> Option<ManualSnapshot> {
        self.read_config().last_manual
    }

    pub fn set_last_manual(&self, snapshot: &ManualSnapshot) -> Result<(), ProfileStoreError> {
        let mut config = self.read_config();
        config.last_manual = Some(snapshot.clone());
        self.write_config(&config)
    }

    fn read_config(&self) -> AppConfig {
        let bytes = match fs::read(self.config_path()) {
            Ok(bytes) => bytes,
            Err(_) => return AppConfig::default(),
        };
        // 损坏/不兼容的配置按空配置处理，不让启动或连接失败。
        toml::from_str(&String::from_utf8_lossy(&bytes)).unwrap_or_default()
    }

    fn write_config(&self, config: &AppConfig) -> Result<(), ProfileStoreError> {
        self.ensure_dirs()?;
        let body = toml::to_string(config).expect("config serialization");
        atomic_write(&self.config_path(), &body)
    }
}

fn atomic_write(path: &Path, body: &str) -> Result<(), ProfileStoreError> {
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, body).map_err(|source| ProfileStoreError::Write {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, path).map_err(|source| ProfileStoreError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// 平台配置根目录（不含 my_ipkvm 子目录）。
fn config_base_dir() -> Option<PathBuf> {
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

mod mouse_mode_serde {
    use ipkvm_core::MouseMode;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(mode: &MouseMode, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match mode {
            MouseMode::Absolute => "Absolute",
            MouseMode::Relative => "Relative",
        })
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<MouseMode, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "Absolute" => Ok(MouseMode::Absolute),
            "Relative" => Ok(MouseMode::Relative),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["Absolute", "Relative"],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试独立临时目录（并行测试互不干扰）。
    fn test_store(name: &str) -> (ProfileStore, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("my-ipkvm-config-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        (ProfileStore::new(dir.clone()), dir)
    }

    #[test]
    fn connection_settings_round_trip_via_toml() {
        let settings = ConnectionSettings {
            baud_rate: 115200,
            auto_baud: false,
            preview_fps: 15,
            mouse_mode: MouseMode::Absolute,
            relative_sensitivity: 1.5,
        };
        let text = toml::to_string(&settings).unwrap();
        let parsed: ConnectionSettings = toml::from_str(&text).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn default_mouse_mode_is_absolute() {
        assert_eq!(
            ConnectionSettings::default().mouse_mode,
            MouseMode::Absolute
        );
    }

    #[test]
    fn save_and_load_profile_preserves_all_fields() {
        let (store, dir) = test_store("roundtrip");
        let profile = Profile {
            name: "办公室".into(),
            video_device: Some(DeviceRef {
                id: "cam0".into(),
                label: "USB Video".into(),
            }),
            control_device: Some(DeviceRef {
                id: "COM9".into(),
                label: "CH9329 (COM9)".into(),
            }),
            connection: ConnectionSettings::default(),
        };

        store.save_profile(&profile).unwrap();
        assert!(store.profile_exists("办公室"));
        assert_eq!(store.list_profiles(), vec!["办公室"]);
        assert_eq!(store.load_profile("办公室").unwrap(), profile);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_name_without_extension_resolves_same_file() {
        let (store, dir) = test_store("extension");
        let profile = Profile {
            name: "test".into(),
            video_device: None,
            control_device: None,
            connection: ConnectionSettings::default(),
        };
        store.save_profile(&profile).unwrap();
        assert!(store.profile_path("test.toml").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_profiles_dedupe_and_cap_at_limit() {
        let (store, dir) = test_store("recent");

        for i in 0..(RECENT_LIMIT + 3) {
            store.add_recent_profile(&format!("p{i}")).unwrap();
        }
        // 重复项去重置顶
        store.add_recent_profile("p12").unwrap();

        let recent = store.recent_profiles();
        assert_eq!(recent.len(), RECENT_LIMIT);
        assert_eq!(recent[0], "p12");
        assert_eq!(recent.iter().filter(|name| *name == "p12").count(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_manual_round_trip() {
        let (store, dir) = test_store("last_manual");
        assert!(store.last_manual().is_none());

        let snapshot = ManualSnapshot {
            video_device: Some(DeviceRef {
                id: "cam0".into(),
                label: "USB Video".into(),
            }),
            control_device: None,
            connection: ConnectionSettings::default(),
        };
        store.set_last_manual(&snapshot).unwrap();
        assert_eq!(store.last_manual().unwrap(), snapshot);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_config_falls_back_to_defaults() {
        let (store, dir) = test_store("corrupt_config");
        fs::create_dir_all(&dir).unwrap();
        fs::write(store.config_path(), b"not: [valid toml").unwrap();

        assert!(store.recent_profiles().is_empty());
        assert!(store.last_manual().is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_profile_file_reports_parse_error() {
        let (store, dir) = test_store("corrupt_profile");
        store
            .save_profile(&Profile {
                name: "bad".into(),
                video_device: None,
                control_device: None,
                connection: ConnectionSettings::default(),
            })
            .unwrap();
        fs::write(store.profile_path("bad"), b"{{{").unwrap();

        assert!(store.load_profile("bad").is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
