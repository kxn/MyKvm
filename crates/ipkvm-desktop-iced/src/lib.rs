//! iced 桌面端（M1）：视频链路（scale/frames/video/status/app）。
//!
//! 迁移设计文档：docs/superpowers/specs/2026-08-03-iced-migration-design.md。
//! M1–M3 已收编视频、菜单和输入链路，迁移期间的验证实现已归入正式模块。
//!
//! 可测试性要求（每阶段强制）：lib/bin 拆分，UI 逻辑可 headless 测试
//! （iced_test Simulator），窗口元数据走常量/函数并可断言。

use iced::Size;
use rust_i18n::t;

pub mod app;
pub mod clipboard;
pub mod connect;
pub mod diag;
pub mod dialog;
pub mod fonts;
pub mod frames;
pub mod input;
pub mod keymap;
pub mod locale;
pub mod menu;
pub mod modal;
pub mod perf;
pub mod platform;
pub mod preloaded;
pub mod profile;
pub mod relative;
pub mod scale;
pub mod status;
pub mod theme;
pub mod video;
pub mod video_area;

/// 稳定窗口标题；构建 short hash 仅在 About/诊断信息中展示。
pub const WINDOW_TITLE: &str = "my_ipkvm iced";

/// 视频画布宽高比：初始窗口的视频区域固定 16:9（之后窗口可自由拖动 resize）。
pub const VIDEO_ASPECT: f32 = 16.0 / 9.0;

/// 视频区之外的窗口 chrome（菜单栏 + 状态栏）兜底高度（逻辑 px）。
///
/// **不再用于精确窗口尺寸**：启动窗口尺寸由 [`measure_chrome_height`] 离屏实测
/// 真实 chrome 高度后传入 [`initial_window_size`] 计算，本常量仅作为离屏测量失败时
/// 的兜底（不应发生），以及 [`app::desired_window_size`]（ResizeWindowToVideo 模式）
/// 的输入。保留它是为了不破坏那些不常见路径，但精确窗口尺寸的事实来源是实测。
pub const CHROME_H: f32 = 48.0;

/// 适中基准视频区：1280×720（720p，16:9）。
///
/// 1080p 屏上约占 2/3 高度，4K 屏不夸张；小屏（1366×768 等）自动等比缩小。
const BASE_VIDEO_SIZE: Size = Size::new(1280.0, 720.0);

/// 把 `target` 等比缩小（不放大）到能放进 `max`，保持宽高比。
fn fit_keep_aspect(target: Size, max: Size) -> Size {
    let scale = (max.width / target.width)
        .min(max.height / target.height)
        .min(1.0);
    Size::new(target.width * scale, target.height * scale)
}

/// 初始窗口尺寸：视频区域严格 16:9，窗口 = 视频区 + chrome。
///
/// `chrome_h` 是菜单栏 + 状态栏的**实测**逻辑高度（由 [`measure_chrome_height`] 离屏
/// 渲染得到），不再用估算常量 [`CHROME_H`]。目标视频区 1280×720，等比缩放到能放进
/// 工作区（宽 90%，高 90% 再扣 chrome_h），不放大；下限 640×360（可用空间够大时生效，
/// 仍保持 16:9 且不超可用空间）。之后窗口可自由拖动 resize，本函数只决定启动初始尺寸。
pub fn initial_window_size(work_area: Size, chrome_h: f32) -> Size {
    let max_video = Size::new(
        work_area.width * 0.9,
        (work_area.height * 0.9 - chrome_h).max(1.0),
    );
    let mut video = fit_keep_aspect(BASE_VIDEO_SIZE, max_video);
    // 下限 640×360：fit_keep_aspect 保证 min_video 不超可用空间且保持 16:9。
    let min_video = fit_keep_aspect(Size::new(640.0, 360.0), max_video);
    if video.width < min_video.width {
        video = min_video;
    }
    Size::new(video.width, video.height + chrome_h)
}

/// 启动时离屏实测菜单栏 + 状态栏的 chrome 逻辑高度（用于 [`initial_window_size`]）。
///
/// 用 `iced_tiny_skia::Renderer` 渲染一次复刻 `App::view` 顶/底结构的布局
/// （`menu_bar` + Fill 占位 + 等价 `status_line` 骨架），通过 [`video_area::BoundsRecorder`]
/// 读出 Fill 占位的真实 bounds，`chrome = 画布高 - 占位高`。这是确定性的、与系统 DPI
/// 无关的逻辑点测量：只要菜单/状态栏的 widget 结构不变，返回值稳定；字体或控件结构
/// 变化时会自动反映（无需手动同步常量）。
///
/// `work_area_width` 为画布宽度（取桌面工作区宽度，保证菜单/状态栏单行不换行）。
/// `font` 必须与运行时 UI 字体一致（菜单/状态栏文字行高依赖字体）。
pub fn measure_chrome_height(work_area_width: f32, font: iced::Font) -> f32 {
    use iced::advanced::clipboard::Null;
    use iced::advanced::graphics::Viewport;
    use iced::advanced::mouse;
    use iced::widget::{PickList, column, container, row, text};
    use iced::{Length, Point, Rectangle};
    use iced_runtime::user_interface::{self, UserInterface};
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::video_area::BoundsRecorder;

    // 画布：宽度取工作区宽度（菜单/状态栏单行），高度给足让 Fill 占位能撑开。
    // 宽度有下限，避免极窄工作区导致 PickList/text 换行扭曲测量。
    let canvas_w = work_area_width.max(640.0);
    let canvas_h = 600.0;
    let cell: Rc<RefCell<Option<Rectangle>>> = Rc::new(RefCell::new(None));

    // 菜单栏：直接复用真实 menu_bar（零漂移）。语言用系统默认即可，不影响高度。
    // 显式指定 Renderer 为 iced_tiny_skia::Renderer，与占位/状态栏类型一致。
    let menu: iced::Element<'_, crate::menu::MenuAction, iced::Theme, iced_tiny_skia::Renderer> =
        crate::menu::menu_bar(&[], false, crate::locale::AppLanguage::System, true, true);

    // Fill 占位（包 BoundsRecorder）：模拟视频区，撑满菜单与状态栏之间。
    let placeholder: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    let placeholder: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        BoundsRecorder::new(cell.clone(), placeholder).into();

    // 状态栏骨架：复刻 App::status_line 的 widget 结构（高度由结构与字体决定，
    // 与运行时值无关）。row 含 5 个 ui_font 文本 + 一个 width=150 的 PickList，
    // 外层 container padding(6)，无 align_y（与真实 status_line 一致）。
    // PickList options 内容不影响高度，用单元素占位即可。
    let options = vec!["profile".to_string()];
    let status_row = row![
        text("control").font(font),
        text("keyboard").font(font),
        text("pointer").font(font),
        PickList::new(options.clone(), Some(options[0].clone()), |_| ())
            .width(Length::Fixed(150.0)),
        text("video").font(font),
    ]
    .spacing(16);
    let status: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        container(status_row).width(Length::Fill).padding(6).into();

    let view: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        column![menu.map(|_| ()), placeholder, status].into();

    let mut renderer = iced_tiny_skia::Renderer::new(font, 16.0.into());
    let mut ui = UserInterface::build(
        view,
        iced::Size::new(canvas_w, canvas_h),
        user_interface::Cache::new(),
        &mut renderer,
    );

    let mut messages = Vec::new();
    let mut clipboard = Null;
    let _ = ui.update(
        &[],
        mouse::Cursor::Available(Point::ORIGIN),
        &mut renderer,
        &mut clipboard,
        &mut messages,
    );

    let theme = iced::Theme::Light;
    let style = iced::advanced::renderer::Style {
        text_color: theme.palette().text,
    };
    ui.draw(
        &mut renderer,
        &theme,
        &style,
        mouse::Cursor::Available(Point::ORIGIN),
    );
    // draw 后 BoundsRecorder 已把占位 bounds 写入 cell；无需真正光栅化像素。
    // 但 tiny_skia 的 draw 接口需要 pixmap，这里用一个小画布满足签名即可
    // （BoundsRecorder 在 ui.draw 阶段已完成记录）。
    let mut pixmap = tiny_skia::Pixmap::new(2, 2).expect("2x2 pixmap");
    let mut mask = tiny_skia::Mask::new(2, 2).expect("2x2 mask");
    let _ = renderer.draw(
        &mut pixmap.as_mut(),
        &mut mask,
        &Viewport::with_physical_size(iced::Size::new(2, 2), 1.0),
        &[Rectangle::with_size(iced::Size::new(canvas_w, canvas_h))],
        iced::Color::WHITE,
    );

    match *cell.borrow() {
        Some(rect) => (canvas_h - rect.height).max(0.0),
        None => CHROME_H, // 离屏测量失败的兜底（不应发生）。
    }
}

/// 桌面工作区（逻辑点）。Windows 读 SPI_GETWORKAREA 并按系统 DPI 换算；
/// 失败/兜底返回足够大的值（1920x1080），使初始窗口 = 720p 适中窗口。
#[cfg(windows)]
pub fn desktop_work_area() -> Size {
    unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::HiDpi::GetDpiForSystem;
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        let mut rect = RECT::default();
        let ok = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        if ok {
            let dpi = GetDpiForSystem().max(96) as f32 / 96.0;
            Size::new(
                (rect.right - rect.left) as f32 / dpi,
                (rect.bottom - rect.top) as f32 / dpi,
            )
        } else {
            Size::new(1920.0, 1080.0)
        }
    }
}

#[cfg(target_os = "macos")]
pub fn desktop_work_area() -> Size {
    // TODO(macos): 用 NSScreen.main.visibleFrame 换算逻辑尺寸。
    // 当前兜底 1920×1080 → 初始 720p 适中窗口；回头补独立实现即可。
    Size::new(1920.0, 1080.0)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn desktop_work_area() -> Size {
    // Linux 桌面使用概率低，兜底 1920×1080 → 初始 720p 适中窗口。
    Size::new(1920.0, 1080.0)
}

pub use app::run;
pub use app::{App, MockApp};
pub use perf::FrameStats;

rust_i18n::i18n!("locales", fallback = "en");

/// i18n 全局 locale 是进程级状态，涉及它的测试串行执行。
pub static I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 运行时翻译（集成测试无法直接调 t! 宏，收口成函数）。
pub fn translate_key(key: &str) -> String {
    match key {
        "menu.file" => t!("menu.file").to_string(),
        "menu.edit" => t!("menu.edit").to_string(),
        "menu.send" => t!("menu.send").to_string(),
        "menu.about" => t!("menu.about").to_string(),
        "menu.disconnect" => t!("menu.disconnect").to_string(),
        "file.load_profile" => t!("file.load_profile").to_string(),
        "file.recent" => t!("file.recent").to_string(),
        "file.recent_more" => t!("file.recent_more").to_string(),
        "file.exit" => t!("file.exit").to_string(),
        "edit.copy_screenshot" => t!("edit.copy_screenshot").to_string(),
        "edit.save_screenshot" => t!("edit.save_screenshot").to_string(),
        "edit.save_screenshot_unsupported" => t!("edit.save_screenshot_unsupported").to_string(),
        "edit.language" => t!("edit.language").to_string(),
        "edit.settings" => t!("edit.settings").to_string(),
        "send.paste_text" => t!("send.paste_text").to_string(),
        "send.release_all" => t!("send.release_all").to_string(),
        "send.special_keys" => t!("send.special_keys").to_string(),
        "special_keys.ctrl_alt_del" => t!("special_keys.ctrl_alt_del").to_string(),
        "special_keys.win" => t!("special_keys.win").to_string(),
        "special_keys.print_screen" => t!("special_keys.print_screen").to_string(),
        "special_keys.alt_tab" => t!("special_keys.alt_tab").to_string(),
        "language.system" => t!("language.system").to_string(),
        "language.chinese" => t!("language.chinese").to_string(),
        "language.english" => t!("language.english").to_string(),
        "about.project_home" => t!("about.project_home").to_string(),
        "about.title" => t!("about.title").to_string(),
        "about.version" => t!("about.version", commit = "x").to_string(),
        "about.license" => t!("about.license").to_string(),
        "about.project_url" => t!("about.project_url", url = "x").to_string(),
        "dialog.jpeg_filter" => t!("dialog.jpeg_filter").to_string(),
        "modal.settings_title" => t!("modal.settings_title").to_string(),
        "modal.connection_title" => t!("modal.connection_title").to_string(),
        "modal.close" => t!("modal.close").to_string(),
        "modal.about_title" => t!("modal.about_title").to_string(),
        "modal.save_title" => t!("modal.save_title").to_string(),
        "modal.name_label" => t!("modal.name_label").to_string(),
        "modal.save" => t!("modal.save").to_string(),
        "device.title" => t!("device.title").to_string(),
        "device.video" => t!("device.video").to_string(),
        "device.control" => t!("device.control").to_string(),
        "device.refresh" => t!("device.refresh").to_string(),
        "device.connect" => t!("device.connect").to_string(),
        "device.preview" => t!("device.preview").to_string(),
        "device.no_preview" => t!("device.no_preview").to_string(),
        "preview.no_signal" => t!("preview.no_signal").to_string(),
        "preview.open_failed" => t!("preview.open_failed").to_string(),
        "profile.save" => t!("profile.save").to_string(),
        "profile.saved" => t!("profile.saved", name = "x").to_string(),
        "profile.save_failed" => t!("profile.save_failed", error = "x").to_string(),
        "profile.load_failed" => t!("profile.load_failed", error = "x").to_string(),
        "profile.device_missing" => t!("profile.device_missing").to_string(),
        "profile.control_missing" => t!("profile.control_missing").to_string(),
        "profile.no_recent" => t!("profile.no_recent").to_string(),
        "profile.overwrite_body" => t!("profile.overwrite_body", name = "x").to_string(),
        "profile.overwrite_confirm" => t!("profile.overwrite_confirm").to_string(),
        "profile.restore_defaults" => t!("profile.restore_defaults").to_string(),
        "connection_settings.title" => t!("connection_settings.title").to_string(),
        "settings.title" => t!("settings.title").to_string(),
        "settings.baud_rate" => t!("settings.baud_rate").to_string(),
        "settings.auto_baud" => t!("settings.auto_baud").to_string(),
        "settings.preview_fps" => t!("settings.preview_fps").to_string(),
        "settings.mouse_mode" => t!("settings.mouse_mode").to_string(),
        "settings.relative_sensitivity" => t!("settings.relative_sensitivity").to_string(),
        "settings.scale_mode" => t!("settings.scale_mode").to_string(),
        "mouse_mode.absolute" => t!("mouse_mode.absolute").to_string(),
        "mouse_mode.relative" => t!("mouse_mode.relative").to_string(),
        "scale_mode.fit_window" => t!("scale_mode.fit_window").to_string(),
        "scale_mode.actual_size" => t!("scale_mode.actual_size").to_string(),
        "scale_mode.resize_to_video" => t!("scale_mode.resize_to_video").to_string(),
        "status.control_device" => t!("status.control_device", value = "x").to_string(),
        "status.keyboard" => t!("status.keyboard", value = "x").to_string(),
        "status.pointer" => t!("status.pointer", value = "x").to_string(),
        "status.video" => t!("status.video", value = "x").to_string(),
        "status.message" => t!("status.message", message = "x").to_string(),
        "status.offline" => t!("status.offline").to_string(),
        "status.pasting" => t!("status.pasting").to_string(),
        "status.remote_input" => t!("status.remote_input").to_string(),
        "status.ready" => t!("status.ready").to_string(),
        "status.relative_mode" => t!("status.relative_mode").to_string(),
        "status.video_no_signal" => t!("status.video_no_signal").to_string(),
        "status.video_stalled" => t!("status.video_stalled").to_string(),
        "control_status.not_selected" => t!("control_status.not_selected").to_string(),
        "control_status.checking" => t!("control_status.checking").to_string(),
        "control_status.ready" => t!("control_status.ready", port = "x").to_string(),
        "control_status.not_ch9329" => t!("control_status.not_ch9329", reason = "x").to_string(),
        "control_status.no_response" => t!("control_status.no_response").to_string(),
        "control_status.open_failed" => t!("control_status.open_failed", error = "x").to_string(),
        "control_status.offline" => t!("control_status.offline").to_string(),
        "video_status.not_selected" => t!("video_status.not_selected").to_string(),
        "video_status.checking" => t!("video_status.checking").to_string(),
        "video_status.ready" => {
            t!("video_status.ready", width = 1, height = 1, label = "x").to_string()
        }
        "video_status.no_signal" => t!("video_status.no_signal").to_string(),
        "video_status.open_failed" => t!("video_status.open_failed", error = "x").to_string(),
        "video_status.disconnected" => t!("video_status.disconnected").to_string(),
        "control_status_label.not_selected" => t!("control_status_label.not_selected").to_string(),
        "control_status_label.checking" => t!("control_status_label.checking").to_string(),
        "control_status_label.ready" => t!("control_status_label.ready", port = "x").to_string(),
        "control_status_label.not_ch9329" => {
            t!("control_status_label.not_ch9329", reason = "x").to_string()
        }
        "control_status_label.no_response" => t!("control_status_label.no_response").to_string(),
        "control_status_label.open_failed" => {
            t!("control_status_label.open_failed", error = "x").to_string()
        }
        "control_status_label.disconnected" => t!("control_status_label.disconnected").to_string(),
        "message.enumeration_failed" => t!("message.enumeration_failed", error = "x").to_string(),
        "message.connect_failed" => t!("message.connect_failed", error = "x").to_string(),
        "message.baud_selected" => t!("message.baud_selected", baud = 9600).to_string(),
        "message.offline_with_reason" => {
            t!("message.offline_with_reason", reason = "x").to_string()
        }
        "message.offline_reconnect" => t!("message.offline_reconnect").to_string(),
        "message.input_rejected" => t!("message.input_rejected").to_string(),
        "message.unsupported_key" => t!("message.unsupported_key").to_string(),
        "message.clipboard_empty" => t!("message.clipboard_empty").to_string(),
        "message.clipboard_read_failed" => {
            t!("message.clipboard_read_failed", error = "x").to_string()
        }
        "message.no_frame_screenshot" => t!("message.no_frame_screenshot").to_string(),
        "message.screenshot_copied" => t!("message.screenshot_copied").to_string(),
        "message.screenshot_copy_failed" => {
            t!("message.screenshot_copy_failed", error = "x").to_string()
        }
        "message.screenshot_saved" => t!("message.screenshot_saved", path = "x").to_string(),
        "message.screenshot_save_failed" => {
            t!("message.screenshot_save_failed", error = "x").to_string()
        }
        "message.keyboard_send_failed" => {
            t!("message.keyboard_send_failed", error = "x").to_string()
        }
        "message.pointer_send_failed" => t!("message.pointer_send_failed", error = "x").to_string(),
        "common.cancel" => t!("common.cancel").to_string(),
        "common.not_selected" => t!("common.not_selected").to_string(),
        _ => key.to_string(),
    }
}

/// M2 全部 i18n key（labels 测试遍历）。
pub const I18N_KEYS: &[&str] = &[
    "menu.file",
    "menu.edit",
    "menu.send",
    "menu.about",
    "menu.disconnect",
    "file.load_profile",
    "file.recent",
    "file.recent_more",
    "file.exit",
    "edit.copy_screenshot",
    "edit.save_screenshot",
    "edit.save_screenshot_unsupported",
    "edit.language",
    "edit.settings",
    "send.paste_text",
    "send.release_all",
    "send.special_keys",
    "special_keys.ctrl_alt_del",
    "special_keys.win",
    "special_keys.print_screen",
    "special_keys.alt_tab",
    "language.system",
    "language.chinese",
    "language.english",
    "about.title",
    "about.version",
    "about.license",
    "about.project_url",
    "dialog.jpeg_filter",
    "modal.settings_title",
    "modal.connection_title",
    "modal.close",
    "modal.about_title",
    "modal.save_title",
    "modal.name_label",
    "modal.save",
    "device.title",
    "device.video",
    "device.control",
    "device.refresh",
    "device.connect",
    "device.preview",
    "device.no_preview",
    "preview.no_signal",
    "preview.open_failed",
    "profile.save",
    "profile.saved",
    "profile.save_failed",
    "profile.load_failed",
    "profile.device_missing",
    "profile.control_missing",
    "profile.no_recent",
    "profile.overwrite_body",
    "profile.overwrite_confirm",
    "profile.restore_defaults",
    "connection_settings.title",
    "settings.title",
    "settings.baud_rate",
    "settings.auto_baud",
    "settings.preview_fps",
    "settings.mouse_mode",
    "settings.relative_sensitivity",
    "settings.scale_mode",
    "mouse_mode.absolute",
    "mouse_mode.relative",
    "scale_mode.fit_window",
    "scale_mode.actual_size",
    "scale_mode.resize_to_video",
    "status.control_device",
    "status.keyboard",
    "status.pointer",
    "status.video",
    "status.message",
    "status.offline",
    "status.pasting",
    "status.remote_input",
    "status.ready",
    "status.relative_mode",
    "status.video_no_signal",
    "status.video_stalled",
    "control_status.not_selected",
    "control_status.checking",
    "control_status.ready",
    "control_status.not_ch9329",
    "control_status.no_response",
    "control_status.open_failed",
    "control_status.offline",
    "video_status.not_selected",
    "video_status.checking",
    "video_status.ready",
    "video_status.no_signal",
    "video_status.open_failed",
    "video_status.disconnected",
    "control_status_label.not_selected",
    "control_status_label.checking",
    "control_status_label.ready",
    "control_status_label.not_ch9329",
    "control_status_label.no_response",
    "control_status_label.open_failed",
    "control_status_label.disconnected",
    "message.enumeration_failed",
    "message.connect_failed",
    "message.baud_selected",
    "message.offline_with_reason",
    "message.offline_reconnect",
    "message.input_rejected",
    "message.unsupported_key",
    "message.clipboard_empty",
    "message.clipboard_read_failed",
    "message.no_frame_screenshot",
    "message.screenshot_copied",
    "message.screenshot_copy_failed",
    "message.screenshot_saved",
    "message.screenshot_save_failed",
    "message.keyboard_send_failed",
    "message.pointer_send_failed",
    "common.cancel",
    "common.not_selected",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_and_size_are_stable() {
        // 窗口元数据走常量，标题变化时此处会强制更新。
        assert_eq!(WINDOW_TITLE, "my_ipkvm iced");
        assert_eq!(VIDEO_ASPECT, 16.0 / 9.0);
        assert_eq!(CHROME_H, 48.0);
    }

    #[test]
    fn initial_window_size_keeps_16_9_video_area() {
        // 用一组典型 chrome 值验证：给定实测 chrome，视频区严格 16:9。
        let chrome = 72.0;
        // 1080p 工作区：初始视频区恰好 1280×720（720p），窗口 = 视频区 + chrome。
        let win = initial_window_size(Size::new(1920.0, 1080.0), chrome);
        assert_eq!(win, Size::new(1280.0, 720.0 + chrome));
        assert!(((win.height - chrome) / win.width - 9.0 / 16.0).abs() < 1e-3);

        // 1366×768 笔记本：等比缩小，视频区仍严格 16:9。
        let win = initial_window_size(Size::new(1366.0, 768.0), chrome);
        let video_h = win.height - chrome;
        assert!((win.width / video_h - 16.0 / 9.0).abs() < 1e-3);
        assert!(win.width <= 1366.0 * 0.9 + 1.0);
        assert!(win.height <= 768.0 * 0.9 + 1.0);

        // 极窄工作区：不超可用空间，仍保持 16:9。
        let win = initial_window_size(Size::new(640.0, 480.0), chrome);
        let video_h = win.height - chrome;
        assert!((win.width / video_h - 16.0 / 9.0).abs() < 1e-3);
        assert!(win.width <= 640.0 * 0.9 + 1.0);
        assert!(win.height <= 480.0 * 0.9 + 1.0);
    }

    #[test]
    fn initial_window_size_scales_with_measured_chrome() {
        // 关键回归：chrome 越大窗口越高，但视频区始终严格 16:9（不被 chrome 压扁）。
        // 这是原 bug 的核心——旧实现把 CHROME_H 写死，无论真实 chrome 多大都用 48，
        // 导致视频区比例失准。新签名把 chrome 作为参数，调用方传实测值。
        for chrome in [48.0_f32, 72.0, 100.0] {
            let win = initial_window_size(Size::new(1920.0, 1080.0), chrome);
            let video_h = win.height - chrome;
            assert!(
                (win.width / video_h - 16.0 / 9.0).abs() < 1e-3,
                "chrome={chrome} 时视频区必须严格 16:9，实际 {:.4}",
                win.width / video_h
            );
        }
    }

    #[test]
    fn initial_window_size_has_usable_floor() {
        // 可用空间够大时至少 640×360 视频区（保持 16:9）。
        let chrome = 72.0;
        let win = initial_window_size(Size::new(1920.0, 1080.0), chrome);
        assert!(win.width >= 640.0);
        assert!(win.height - chrome >= 360.0);

        // 超大屏不放大：仍是 720p 适中基准。
        let win = initial_window_size(Size::new(3840.0, 2160.0), chrome);
        assert_eq!(win, Size::new(1280.0, 720.0 + chrome));
    }

    #[test]
    fn measure_chrome_height_is_reasonable_and_deterministic() {
        // 离屏实测菜单栏 + 状态栏高度：应在合理区间，且两次调用一致（确定性）。
        let font = crate::fonts::ui_font();
        let h1 = measure_chrome_height(1920.0, font);
        let h2 = measure_chrome_height(1920.0, font);
        assert!(
            (20.0..200.0).contains(&h1),
            "实测 chrome 高度应落在合理区间 20..200，实际 {h1}"
        );
        assert_eq!(h1, h2, "离屏测量必须确定（两次调用应完全一致）");
    }

    #[test]
    fn initial_window_size_with_measured_chrome_yields_16_9_video() {
        // 端到端：实测 chrome → 算初始窗口 → 视频区严格 16:9。
        // 这是修复的核心验收点：无论真实 chrome 是多少，启动窗口视频区必为 16:9。
        let font = crate::fonts::ui_font();
        let chrome = measure_chrome_height(1920.0, font);
        let win = initial_window_size(Size::new(1920.0, 1080.0), chrome);
        let video_h = win.height - chrome;
        assert!(
            (win.width / video_h - 16.0 / 9.0).abs() < 1e-3,
            "实测 chrome 后启动窗口视频区必须严格 16:9，实际 {:.4}",
            win.width / video_h
        );
    }

    #[test]
    fn labels_are_single_line_nonempty_and_not_keys() {
        let _guard = I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        for locale in ["en", "zh-CN"] {
            rust_i18n::set_locale(locale);
            for key in I18N_KEYS {
                let label = translate_key(key);
                assert!(!label.contains('\n'), "[{locale}] {key} 译文不得含换行");
                assert!(
                    !label.is_empty() && label != *key,
                    "[{locale}] {key} 译文不得为空或等于 key"
                );
            }
        }
    }

    #[test]
    fn fixed_width_en_labels_stay_within_28_chars() {
        let _guard = I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        // 菜单 240 / 按钮 140 / 下拉 240 固定宽度下不换行越界的英文关键标签。
        for key in [
            "file.load_profile",
            "send.release_all",
            "device.control",
            "device.refresh",
            "edit.save_screenshot",
            "settings.auto_baud",
            "connection_settings.title",
            "profile.save",
            "modal.connection_title",
        ] {
            let label = translate_key(key);
            assert!(
                label.chars().count() <= 28,
                "[en] {key} 英文文案 {label:?} 超过 28 字符，固定宽度控件会换行/越界"
            );
        }
    }

    #[test]
    fn gui_entry_suppresses_console_subsystem_on_windows() {
        let source = include_str!("main.rs");
        assert!(
            source.contains("#![cfg_attr(windows, windows_subsystem = \"windows\")]"),
            "main.rs 缺少 windows_subsystem 属性，Windows release 启动会带黑窗"
        );
    }
}
