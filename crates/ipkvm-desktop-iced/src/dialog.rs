//! 原生文件对话框收口（rfd，Windows only；与 egui 端 rfd 0.15.4 配置一致）。
//! rfd 是阻塞调用，调用方必须放在 iced Task::perform 里（异步线程），
//! 避免阻塞 UI 事件循环。非 Windows 返回 None（菜单项按不支持处理）。

use rust_i18n::t;

/// 弹出“保存截图”对话框，返回用户选择的路径（取消返回 None）。
#[cfg(windows)]
pub async fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter(t!("dialog.jpeg_filter"), &["jpg", "jpeg"])
        .set_file_name("my_ipkvm-screenshot.jpg")
        .save_file()
}

/// 非 Windows 平台不支持原生保存对话框。
#[cfg(not(windows))]
pub async fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    None
}
