//! 原生文件对话框收口（rfd，Windows/macOS）。
//! rfd 是阻塞调用，调用方必须放在 iced Task::perform 里（异步线程），
//! 避免阻塞 UI 事件循环。其余平台返回 None（菜单项按不支持处理）。

#[cfg(any(windows, target_os = "macos"))]
use rust_i18n::t;

/// 弹出“保存截图”对话框，返回用户选择的路径（取消返回 None）。
#[cfg(any(windows, target_os = "macos"))]
pub async fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter(t!("dialog.jpeg_filter"), &["jpg", "jpeg"])
        .set_file_name("my_ipkvm-screenshot.jpg")
        .save_file()
}

/// 其余平台不支持原生保存对话框。
#[cfg(not(any(windows, target_os = "macos")))]
pub async fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    None
}

/// 弹出“加载 profile”对话框，默认打开 profiles 目录。
#[cfg(any(windows, target_os = "macos"))]
pub async fn choose_profile_path(profiles_dir: std::path::PathBuf) -> Option<std::path::PathBuf> {
    let _ = std::fs::create_dir_all(&profiles_dir);
    rfd::FileDialog::new()
        .set_directory(&profiles_dir)
        .add_filter("profile", &["toml"])
        .pick_file()
}

/// 其余平台不支持原生打开对话框。
#[cfg(not(any(windows, target_os = "macos")))]
pub async fn choose_profile_path(_profiles_dir: std::path::PathBuf) -> Option<std::path::PathBuf> {
    None
}
