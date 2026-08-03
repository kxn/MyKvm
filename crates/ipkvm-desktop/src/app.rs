use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use ipkvm_core::MouseMode;
use ipkvm_session::rfb_input::RfbInputNotice;
use ipkvm_video::FrameSource;
use rust_i18n::t;

use crate::clipboard::{ClipboardService, save_jpeg};
use crate::config::{ConnectionSettings, DeviceRef, ManualSnapshot, Profile, ProfileStore};
use crate::frame::bgra_to_rgba;
use crate::input::{
    KeyAction, SpecialKey, egui_key_to_keysym, modifier_diff, pointer_active, pointer_button_mask,
    special_key_sequence,
};
use crate::locale::AppLanguage;
use crate::probe::{ProbeBackend, ProductionProbeBackend, refresh_detection};
use crate::render::{FrameSize, VideoViewport};
use crate::session::{ConnectRequest, DesktopSessionError, ProductionDesktopSessionController};
use crate::state::{
    ControlProbeStatus, DeviceSelectionState, PreviewInfo, VideoProbeStatus, VideoScaleMode,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const NO_SIGNAL_TIMEOUT: Duration = Duration::from_secs(2);
/// 波特率扫描单档超时：GetInfo 应答很快，逐档白等会显著拖慢连接。
const BAUD_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
/// 指针最小发送间隔（约 30Hz 限频），按键状态变化不受此限制。
const POINTER_MIN_INTERVAL: Duration = Duration::from_millis(33);
/// 菜单栏 + 状态栏高度估算兜底（首帧实测之前用）。
const DEFAULT_CHROME_FALLBACK: f32 = 48.0;
/// 视频画面外留白（信箱/黑边区域）的填充色：与黑色视频内容可区分，
/// 便于判断真实屏幕边界。
const LETTERBOX_COLOR: egui::Color32 = egui::Color32::from_rgb(24, 32, 48);
/// 视频画面描边色：进一步标出真实屏幕边界。
const VIDEO_BORDER_COLOR: egui::Color32 = egui::Color32::from_gray(110);
/// 窗口图标：由 scripts/generate_icon.py 生成的 32×32 原始 RGBA。
const WINDOW_ICON: &[u8] = include_bytes!("../assets/icon-32.rgba");
/// 项目主页（实际仓库为内网 Gitea）。
const PROJECT_URL: &str = "http://10.10.10.5:3000/kxn/my_ipkvm";

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title(format!("my_ipkvm {}", env!("GIT_COMMIT")))
            .with_icon(Arc::new(egui::IconData {
                rgba: WINDOW_ICON.to_vec(),
                width: 32,
                height: 32,
            })),
        ..Default::default()
    };
    eframe::run_native(
        "my_ipkvm",
        options,
        Box::new(|cc| {
            crate::fonts::install(&cc.egui_ctx);
            Ok(Box::new(DesktopApp::new()))
        }),
    )
}

struct DesktopApp {
    selection: DeviceSelectionState,
    probe: ProductionProbeBackend,
    session: ProductionDesktopSessionController,
    store: ProfileStore,
    /// 菜单“设置…”里的连接参数默认值（新会话/恢复默认时使用）。
    default_connection: ConnectionSettings,
    /// 主页“连接设置”的连接级副本：启动时取默认/上次手动值，可被 profile 覆盖。
    connection: ConnectionSettings,
    /// 本地视图偏好（不进 profile）。
    scale_mode: VideoScaleMode,
    /// 当前加载的 profile 名（用于连接成功时记录最近使用；手动改动后清空）。
    active_profile: Option<String>,
    texture: Option<egui::TextureHandle>,
    latest_frame: Option<crate::frame::RgbaFrame>,
    frame_size: Option<FrameSize>,
    last_follow_resize: Option<FrameSize>,
    pointer_mask: u8,
    last_pointer: Option<(u16, u16)>,
    last_modifiers: egui::Modifiers,
    cursor_grabbed: bool,
    relative_remainder: (f32, f32),
    relative_wheel: i8,
    pending_pointer: Option<(u8, u16, u16)>,
    last_pointer_sent: Option<(u8, u16, u16)>,
    last_pointer_sent_at: Option<Instant>,
    frame_repaint_task: Option<tokio::task::JoinHandle<()>>,
    menu_chrome: f32,
    status_chrome: f32,
    video_focused: bool,
    paste_busy: bool,
    status_message: Option<String>,
    language: AppLanguage,
    showing_device_dialog: bool,
    show_settings: bool,
    show_connection_settings: bool,
    show_save_profile: bool,
    save_profile_name: String,
    save_profile_confirm_overwrite: bool,
    show_about: bool,
    preview_source: Option<Arc<dyn FrameSource>>,
    preview_texture: Option<egui::TextureHandle>,
    preview_frame_size: Option<FrameSize>,
    preview_device_id: Option<String>,
    /// 预览源打开时刻：用于“打开成功但迟迟没有帧 → 无信号”的超时判定。
    preview_opened_at: Option<Instant>,
    /// 最近一帧到达时刻：用于“正常出帧后停帧 → 断流/无信号”的判定。
    preview_last_frame_at: Option<Instant>,
    last_frame_seq: Option<u64>,
    last_frame_at: Option<Instant>,
}

impl DesktopApp {
    fn new() -> Self {
        let mut app = Self::empty();
        // 跟随系统语言（检测失败回退项目默认中文）；显式语言选择在编辑菜单里覆盖。
        AppLanguage::System.apply();
        let mut selection = app.selection.clone();
        if let Err(error) = refresh_detection(
            &mut selection,
            &mut app.probe,
            app.connection.baud_rate,
            PROBE_TIMEOUT,
        ) {
            eprintln!("warning: 初始设备枚举失败：{error}");
        }
        app.selection = selection;
        // 打开连接界面时预填上次手动连接（profile 连接不覆盖它）。
        if let Some(snapshot) = app.store.last_manual() {
            app.apply_snapshot(snapshot);
        }
        app
    }

    /// 空构造器：不枚举设备，由 `new()` 追加启动刷新；测试直接使用。
    fn empty() -> Self {
        Self {
            selection: DeviceSelectionState::default(),
            probe: ProductionProbeBackend,
            session: ProductionDesktopSessionController::production(),
            store: ProfileStore::production(),
            default_connection: ConnectionSettings::default(),
            connection: ConnectionSettings::default(),
            scale_mode: VideoScaleMode::default(),
            active_profile: None,
            texture: None,
            latest_frame: None,
            frame_size: None,
            last_follow_resize: None,
            pointer_mask: 0,
            last_pointer: None,
            last_modifiers: egui::Modifiers::NONE,
            cursor_grabbed: false,
            relative_remainder: (0.0, 0.0),
            relative_wheel: 0,
            pending_pointer: None,
            last_pointer_sent: None,
            last_pointer_sent_at: None,
            frame_repaint_task: None,
            menu_chrome: 0.0,
            status_chrome: 0.0,
            video_focused: false,
            paste_busy: false,
            status_message: None,
            language: AppLanguage::System,
            showing_device_dialog: true,
            show_settings: false,
            show_connection_settings: false,
            show_save_profile: false,
            save_profile_name: String::new(),
            save_profile_confirm_overwrite: false,
            show_about: false,
            preview_source: None,
            preview_texture: None,
            preview_frame_size: None,
            preview_device_id: None,
            preview_opened_at: None,
            preview_last_frame_at: None,
            last_frame_seq: None,
            last_frame_at: None,
        }
    }

    fn update_impl(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_notices();
        self.refresh_video(ctx);
        self.sync_control_state();

        let menu = egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu_bar(ui));
        self.menu_chrome = menu.response.rect.height();
        if self.showing_device_dialog {
            self.device_dialog(ctx);
        } else {
            self.console_ui(ctx);
        }
        let status = egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        self.status_chrome = status.response.rect.height();

        if self.show_settings {
            egui::Modal::new(egui::Id::new("settings_modal")).show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.heading(t!("settings.title"));
                ui.add_space(8.0);
                self.settings_ui(ui);
                ui.add_space(8.0);
                if ui.button(t!("common.close")).clicked() {
                    self.show_settings = false;
                }
            });
        }
        if self.show_connection_settings {
            egui::Modal::new(egui::Id::new("connection_settings_modal")).show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.heading(t!("connection_settings.title"));
                ui.add_space(8.0);
                self.connection_settings_ui(ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("profile.restore_defaults")).clicked() {
                        self.connection = self.default_connection.clone();
                        self.active_profile = None;
                    }
                    if ui.button(t!("common.close")).clicked() {
                        self.show_connection_settings = false;
                    }
                });
            });
        }
        if self.show_save_profile {
            egui::Modal::new(egui::Id::new("save_profile_modal")).show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.heading(t!("profile.save_title"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(t!("profile.name_label"));
                    ui.text_edit_singleline(&mut self.save_profile_name);
                });
                ui.add_space(8.0);
                if self.save_profile_confirm_overwrite {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        t!(
                            "profile.overwrite_body",
                            name = self.save_profile_name.trim()
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(t!("profile.overwrite_confirm")).clicked() {
                            self.do_save_profile();
                        }
                        if ui.button(t!("common.cancel")).clicked() {
                            self.save_profile_confirm_overwrite = false;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        let name_ok = !self.save_profile_name.trim().is_empty();
                        if ui
                            .add_enabled(name_ok, egui::Button::new(t!("profile.save_button")))
                            .clicked()
                        {
                            let name = self.save_profile_name.trim().to_string();
                            if self.store.profile_exists(&name) {
                                self.save_profile_confirm_overwrite = true;
                            } else {
                                self.do_save_profile();
                            }
                        }
                        if ui.button(t!("common.close")).clicked() {
                            self.close_save_profile_dialog();
                        }
                    });
                }
            });
        }
        if self.show_about {
            egui::Modal::new(egui::Id::new("about_modal")).show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.heading(t!("about.title"));
                ui.add_space(8.0);
                ui.label(t!("about.version", commit = env!("GIT_COMMIT")));
                ui.label(t!("about.license"));
                ui.label(t!("about.project_url", url = PROJECT_URL));
                ui.add_space(8.0);
                if ui.button(t!("common.close")).clicked() {
                    self.show_about = false;
                }
            });
        }
    }

    /// 控制设备离线时的状态同步：标记离线并复位所有输入/粘贴 UI 状态，
    /// 保证停止、掉线后不会残留"聚焦可输入/粘贴中/按键按下"等过期状态。
    fn sync_control_state(&mut self) {
        if !self.showing_device_dialog && !self.session.is_control_online() {
            self.selection.mark_control_offline();
            self.paste_busy = false;
            self.video_focused = false;
            self.pointer_mask = 0;
            self.last_pointer = None;
            self.last_modifiers = egui::Modifiers::NONE;
            self.cursor_grabbed = false;
            self.relative_remainder = (0.0, 0.0);
            self.pending_pointer = None;
            self.relative_wheel = 0;
            self.last_pointer_sent = None;
            self.last_pointer_sent_at = None;
            let message = match self.session.input_offline_reason() {
                Some(reason) => t!("message.offline_with_reason", reason = reason).to_string(),
                None => t!("message.offline_reconnect").to_string(),
            };
            self.status_message = Some(message);
        }
    }

    fn refresh_video(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.session.latest_frame() else {
            return;
        };
        let now = Instant::now();
        if self.last_frame_seq != Some(frame.seq) {
            self.last_frame_seq = Some(frame.seq);
            self.last_frame_at = Some(now);
            // eframe 事件驱动重绘：新帧到达必须请求重绘，否则空闲时画面停滞。
            ctx.request_repaint();
        }
        if let Ok(rgba) = bgra_to_rgba(&frame) {
            let size = FrameSize {
                width: frame.width,
                height: frame.height,
            };
            if self.frame_size != Some(size) {
                self.frame_size = Some(size);
            }
            self.latest_frame = Some(rgba);
        }
    }

    fn drain_notices(&mut self) {
        for notice in self.session.drain_notices() {
            match notice {
                RfbInputNotice::TextTyped { .. } | RfbInputNotice::TextInputFailed { .. } => {
                    self.paste_busy = false;
                }
                RfbInputNotice::KeyboardRejected { .. } => {
                    self.status_message = Some(t!("message.input_rejected").to_string());
                }
                RfbInputNotice::ControllerReleased { .. }
                | RfbInputNotice::TextDispatched { .. } => {}
                _ => {}
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t!("menu.file"), |ui| self.file_menu(ui));
            ui.menu_button(t!("menu.edit"), |ui| self.edit_menu(ui));
            ui.menu_button(t!("menu.send"), |ui| self.send_menu(ui));
            ui.menu_button(t!("menu.about"), |ui| self.about_menu(ui));
        });
    }

    /// 文件：连接生命周期 + profile 加载/最近使用 + 退出。
    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(t!("menu.reselect_device")).clicked() {
            self.show_device_dialog();
            ui.close();
        }
        let online = self.session.is_control_online();
        if ui
            .add_enabled(online, egui::Button::new(t!("menu.stop_connection")))
            .clicked()
        {
            self.stop_session();
            ui.close();
        }
        ui.separator();
        #[cfg(windows)]
        if ui.button(t!("file.load_profile")).clicked() {
            self.load_profile_from_dialog();
            ui.close();
        }
        #[cfg(not(windows))]
        {
            ui.add_enabled(
                false,
                egui::Button::new(t!("file.load_profile_unsupported")),
            );
        }
        ui.menu_button(t!("file.recent"), |ui| self.recent_menu(ui));
        ui.separator();
        if ui.button(t!("file.exit")).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            ui.close();
        }
    }

    /// 最近使用：最近 3 条直显，超过 3 条收进“更多”二级菜单（共 10 条）。
    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let recent = self.store.recent_profiles();
        if recent.is_empty() {
            ui.add_enabled(false, egui::Button::new(t!("profile.no_recent")));
            return;
        }
        for name in recent.iter().take(3) {
            if ui.button(name).clicked() {
                self.load_profile_by_name(name);
                ui.close();
            }
        }
        if recent.len() > 3 {
            ui.menu_button(t!("file.recent_more"), |ui| {
                for name in recent.iter().skip(3) {
                    if ui.button(name).clicked() {
                        self.load_profile_by_name(name);
                        ui.close();
                    }
                }
            });
        }
    }

    /// 编辑：截图 + 语言 + 设置。
    fn edit_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(t!("edit.copy_screenshot")).clicked() {
            self.screenshot_copy();
            ui.close();
        }
        #[cfg(windows)]
        if ui.button(t!("edit.save_screenshot")).clicked() {
            self.screenshot_save();
            ui.close();
        }
        #[cfg(not(windows))]
        {
            ui.add_enabled(
                false,
                egui::Button::new(t!("edit.save_screenshot_unsupported")),
            );
        }
        ui.separator();
        // 语言菜单项固定显示英文 Language，任何界面语言下都能找到。
        ui.menu_button(t!("edit.language"), |ui| self.language_menu(ui));
        if ui.button(t!("edit.settings")).clicked() {
            self.show_settings = true;
            ui.close();
        }
    }

    fn language_menu(&mut self, ui: &mut egui::Ui) {
        for option in AppLanguage::ALL {
            let selected = self.language == option;
            let label = format!("{} {}", if selected { "✓" } else { " " }, option.label());
            if ui.selectable_label(selected, label).clicked() {
                self.language = option;
                option.apply();
            }
        }
    }

    /// 发送：粘贴文本 + 释放按键 + 特殊键（二级菜单）。
    fn send_menu(&mut self, ui: &mut egui::Ui) {
        if self.paste_busy {
            ui.add_enabled(false, egui::Button::new(t!("send.paste_text")));
        } else if ui.button(t!("send.paste_text")).clicked() {
            self.paste();
            ui.close();
        }
        if ui.button(t!("send.release_all")).clicked() {
            let _ = self.session.release_all();
            ui.close();
        }
        ui.separator();
        ui.menu_button(t!("send.special_keys"), |ui| {
            self.special_keys_menu(ui);
        });
    }

    /// 特殊键：本地 OS 会拦截、无法键盘直发的键（Esc 等普通键直接按即可）。
    fn special_keys_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(t!("special_keys.ctrl_alt_del")).clicked() {
            self.send_special(SpecialKey::CtrlAltDel);
            ui.close();
        }
        if ui.button(t!("special_keys.win")).clicked() {
            self.send_special(SpecialKey::Win);
            ui.close();
        }
        if ui.button(t!("special_keys.print_screen")).clicked() {
            self.send_special(SpecialKey::PrintScreen);
            ui.close();
        }
        if ui.button(t!("special_keys.alt_tab")).clicked() {
            self.send_special(SpecialKey::AltTab);
            ui.close();
        }
    }

    /// 关于：关于对话框 + 项目主页。
    fn about_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(t!("about.about")).clicked() {
            self.show_about = true;
            ui.close();
        }
        if ui.button(t!("about.project_home")).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(PROJECT_URL));
            ui.close();
        }
    }

    fn device_dialog(&mut self, ctx: &egui::Context) {
        self.update_preview(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(t!("device.title"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(t!("profile.select"));
                egui::ComboBox::from_id_salt("profile_selector")
                    .width(240.0)
                    .selected_text(
                        self.active_profile
                            .clone()
                            .unwrap_or_else(|| t!("common.not_selected").to_string()),
                    )
                    .show_ui(ui, |ui| {
                        let profiles = self.store.list_profiles();
                        if profiles.is_empty() {
                            ui.add_enabled(false, egui::Button::new(t!("profile.no_recent")));
                        }
                        for name in profiles {
                            let selected = self.active_profile.as_deref() == Some(name.as_str());
                            if ui.selectable_label(selected, &name).clicked() {
                                self.load_profile_by_name(&name);
                            }
                        }
                    });
                if ui.button(t!("profile.save")).clicked() {
                    self.show_save_profile = true;
                    self.save_profile_name.clear();
                    self.save_profile_confirm_overwrite = false;
                }
                if ui.button(t!("profile.connection_settings")).clicked() {
                    self.show_connection_settings = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    ui.label(t!("device.video"));
                    self.video_device_combo(ui);
                    self.video_status_label(ui);
                    ui.add_space(10.0);

                    ui.label(t!("device.control"));
                    self.control_device_combo(ui);
                    self.control_status_label(ui);
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button(t!("device.refresh")).clicked() {
                            let mut selection = self.selection.clone();
                            match refresh_detection(
                                &mut selection,
                                &mut self.probe,
                                self.connection.baud_rate,
                                PROBE_TIMEOUT,
                            ) {
                                Ok(()) => {
                                    self.selection = selection;
                                    self.status_message = None;
                                    self.refresh_video_preview_after_reprobe();
                                }
                                Err(error) => {
                                    self.status_message = Some(
                                        t!("message.enumeration_failed", error = error.to_string())
                                            .to_string(),
                                    );
                                }
                            }
                        }
                        let can_connect = self.selection.can_connect();
                        if ui
                            .add_enabled(
                                can_connect,
                                egui::Button::new(t!("device.connect"))
                                    .min_size(egui::vec2(140.0, 36.0)),
                            )
                            .clicked()
                            && let Err(error) = self.connect(ctx)
                        {
                            self.status_message = Some(
                                t!("message.connect_failed", error = error.to_string()).to_string(),
                            );
                        }
                    });

                    if let Some(message) = &self.status_message {
                        ui.add_space(6.0);
                        ui.colored_label(egui::Color32::LIGHT_RED, message);
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(t!("device.preview"));
                    let (preview_response, preview_painter) =
                        ui.allocate_painter(egui::vec2(320.0, 180.0), egui::Sense::hover());
                    preview_painter.rect_filled(
                        preview_response.rect,
                        0.0,
                        egui::Color32::from_gray(20),
                    );
                    if let Some(texture) = &self.preview_texture {
                        let frame = self.preview_frame_size.unwrap_or(FrameSize {
                            width: 320,
                            height: 180,
                        });
                        let rect = VideoViewport::frame_rect(
                            preview_response.rect,
                            frame,
                            VideoScaleMode::FitWindow,
                        );
                        preview_painter.image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        let placeholder = match &self.selection.video_status {
                            VideoProbeStatus::NoSignal => t!("preview.no_signal"),
                            VideoProbeStatus::OpenFailed(_) => t!("preview.open_failed"),
                            _ => t!("device.no_preview"),
                        };
                        preview_painter.text(
                            preview_response.rect.center(),
                            egui::Align2::CENTER_CENTER,
                            placeholder,
                            egui::FontId::proportional(16.0),
                            egui::Color32::GRAY,
                        );
                    }
                });
            });
        });
    }

    /// 打开/刷新选中视频设备的实时预览（设备切换时重建源）。
    ///
    /// 预览源是视频设备的**唯一**打开者（Windows 媒体设备独占，探测/刷新
    /// 不得再开第二个句柄）。video_status 由这里驱动：
    /// - 打开失败 → OpenFailed；
    /// - 首帧到达 → Ready(宽高)；
    /// - 打开后超时无帧 → NoSignal（帧恢复后自动回到 Ready）；
    /// - 正常出帧后停帧超过无信号超时 → NoSignal；
    /// - 设备从枚举中消失 → Disconnected（由刷新标记，此处不重开）。
    fn update_preview(&mut self, ctx: &egui::Context) {
        // 设备已从枚举消失：不尝试打开（也没有可开的设备），状态保持断开，
        // 恢复由“刷新后设备回来”或“用户重新选择”驱动。
        if self.selection.video_status == VideoProbeStatus::Disconnected {
            return;
        }
        let video_id = self.selection.selected_video_id.clone();
        if self.preview_device_id.as_deref() != video_id.as_deref() {
            self.preview_source = None;
            self.preview_texture = None;
            self.preview_frame_size = None;
            self.preview_opened_at = None;
            self.preview_last_frame_at = None;
            self.preview_device_id = video_id.clone();
            if let Some(id) = &video_id {
                match ipkvm_video::camera::CameraSource::open(id, self.connection.preview_fps) {
                    Ok(source) => {
                        self.preview_source = Some(Arc::new(source));
                        self.preview_opened_at = Some(Instant::now());
                        self.selection.video_status = VideoProbeStatus::Checking;
                    }
                    Err(error) => {
                        self.selection.video_status =
                            VideoProbeStatus::OpenFailed(error.to_string());
                    }
                }
            } else {
                self.selection.video_status = VideoProbeStatus::NotSelected;
            }
        }
        let Some(source) = &self.preview_source else {
            return;
        };
        let Some(frame) = source.latest_frame() else {
            // 打开成功但没帧，或正常出帧后停帧：按状态对应的参考时刻判定无信号，
            // 迁移时清掉残留旧画面，避免显示过期的预览。
            let now = Instant::now();
            let stalled = match self.selection.video_status {
                VideoProbeStatus::Checking => {
                    elapsed_since(self.preview_opened_at, PROBE_TIMEOUT, now)
                }
                VideoProbeStatus::Ready(_) => {
                    elapsed_since(self.preview_last_frame_at, NO_SIGNAL_TIMEOUT, now)
                }
                _ => false,
            };
            if stalled {
                self.selection.video_status = VideoProbeStatus::NoSignal;
                self.preview_texture = None;
                self.preview_frame_size = None;
            }
            return;
        };
        self.preview_last_frame_at = Some(Instant::now());
        let Ok(rgba) = bgra_to_rgba(&frame) else {
            return;
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width as usize, rgba.height as usize],
            &rgba.pixels,
        );
        match &mut self.preview_texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.preview_texture =
                    Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
            }
        }
        self.preview_frame_size = Some(FrameSize {
            width: rgba.width,
            height: rgba.height,
        });
        if !matches!(self.selection.video_status, VideoProbeStatus::Ready(_)) {
            self.selection.video_status = VideoProbeStatus::Ready(PreviewInfo {
                width: rgba.width,
                height: rgba.height,
                label: source.source_info().device_name,
            });
        }
    }

    /// 关闭当前预览源并清空预览状态（重新打开前必须先关旧句柄，Windows
    /// 相机/串口都独占）。设备仍处于选中状态时，下一帧 `update_preview`
    /// 会按当前状态决定是否重建。
    fn reset_preview(&mut self) {
        self.preview_source = None;
        self.preview_texture = None;
        self.preview_frame_size = None;
        self.preview_device_id = None;
        self.preview_opened_at = None;
        self.preview_last_frame_at = None;
    }

    /// 刷新检测完成后，根据当前视频状态决定是否重建预览：
    /// 已打开且正常/正在打开 → 跳过；未打开或出错 → 先关旧源再重开；
    /// 设备已消失 → 关闭旧源保持断开。
    fn refresh_video_preview_after_reprobe(&mut self) {
        let selected_present = self
            .selection
            .selected_video_id
            .as_deref()
            .is_some_and(|id| {
                self.selection
                    .video_devices
                    .iter()
                    .any(|device| device.id == id)
            });
        match preview_refresh_action(&self.selection.video_status, selected_present) {
            PreviewRefreshAction::Skip => {}
            PreviewRefreshAction::Reopen => {
                // 重新探测前必须先关掉旧句柄（Windows 相机独占，不能二次打开）。
                self.reset_preview();
                self.selection.video_status = VideoProbeStatus::Checking;
            }
            PreviewRefreshAction::KeepDisconnected => {
                self.reset_preview();
            }
        }
    }

    fn video_device_combo(&mut self, ui: &mut egui::Ui) {
        let selected_text = self
            .selection
            .video_devices
            .iter()
            .find(|device| self.selection.selected_video_id.as_deref() == Some(device.id.as_str()))
            .map(|device| device.label.clone())
            .unwrap_or_else(|| t!("common.not_selected").to_string());
        egui::ComboBox::from_id_salt("video_devices")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for device in self.selection.video_devices.clone() {
                    let selected =
                        self.selection.selected_video_id.as_deref() == Some(device.id.as_str());
                    if ui.selectable_label(selected, &device.label).clicked() {
                        self.selection.selected_video_id = Some(device.id);
                        self.selection.video_status = VideoProbeStatus::Checking;
                        // 强制重建预览源（同一设备重选也重建），由预览驱动状态。
                        self.reset_preview();
                        // 手动改动后不再属于任何 profile。
                        self.active_profile = None;
                    }
                }
            });
    }

    fn control_device_combo(&mut self, ui: &mut egui::Ui) {
        let selected_text = self
            .selection
            .control_devices
            .iter()
            .find(|device| {
                self.selection.selected_control_id.as_deref() == Some(device.id.as_str())
            })
            .map(|device| device.label.clone())
            .unwrap_or_else(|| t!("common.not_selected").to_string());
        egui::ComboBox::from_id_salt("control_devices")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for device in self.selection.control_devices.clone() {
                    let selected =
                        self.selection.selected_control_id.as_deref() == Some(device.id.as_str());
                    if ui.selectable_label(selected, &device.label).clicked() {
                        self.selection.selected_control_id = Some(device.id);
                        self.selection.control_status = ControlProbeStatus::Checking;
                        if let Some(device_id) = self.selection.selected_control_id.clone() {
                            self.selection.control_status = self.probe.probe_control(
                                &device_id,
                                self.connection.baud_rate,
                                PROBE_TIMEOUT,
                            );
                        }
                        // 手动改动后不再属于任何 profile。
                        self.active_profile = None;
                    }
                }
            });
    }

    fn connect(&mut self, ctx: &egui::Context) -> Result<(), DesktopSessionError> {
        // 预览源占用相机/串口，连接前必须先释放。
        self.reset_preview();
        if self.connection.auto_baud
            && let Some(control_id) = self.selection.selected_control_id.clone()
            && let Some(baud) = crate::probe::detect_baud_rate(&control_id, BAUD_PROBE_TIMEOUT)
        {
            self.connection.baud_rate = baud;
            self.status_message = Some(t!("message.baud_selected", baud = baud).to_string());
        }
        let Some(request) = self.connect_request() else {
            return Ok(());
        };
        match self.session.connect(request) {
            Ok(()) => {
                self.showing_device_dialog = false;
                self.status_message = None;
                self.pointer_mask = 0;
                self.last_pointer = None;
                self.relative_remainder = (0.0, 0.0);
                self.pending_pointer = None;
                self.relative_wheel = 0;
                self.last_pointer_sent = None;
                self.last_pointer_sent_at = None;
                if let Some(task) = self.frame_repaint_task.take() {
                    task.abort();
                }
                self.frame_repaint_task = self.session.spawn_frame_repainter(ctx.clone());
                // 连接成功后的归属记录：profile 连接进“最近使用”，手动连接
                // 只更新“上次手动连接”作为下次打开连接界面的默认值。
                if let Some(name) = self.active_profile.clone() {
                    if let Err(error) = self.store.add_recent_profile(&name) {
                        self.status_message =
                            Some(t!("profile.save_failed", error = error.to_string()).to_string());
                    }
                } else {
                    let snapshot = ManualSnapshot {
                        video_device: self.selected_video_ref(),
                        control_device: self.selected_control_ref(),
                        connection: self.connection.clone(),
                    };
                    if let Err(error) = self.store.set_last_manual(&snapshot) {
                        self.status_message =
                            Some(t!("profile.save_failed", error = error.to_string()).to_string());
                    }
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn connect_request(&self) -> Option<ConnectRequest> {
        Some(ConnectRequest {
            video_device_id: self.selection.selected_video_id.clone()?,
            control_device_id: self.selection.selected_control_id.clone()?,
            baud_rate: self.connection.baud_rate,
            mouse_mode: self.connection.mouse_mode,
            preview_fps: self.connection.preview_fps,
        })
    }

    /// 按名字加载 profile 并应用到当前选择（不自动连接）。
    fn load_profile_by_name(&mut self, name: &str) {
        match self.store.load_profile(name) {
            Ok(profile) => self.apply_profile(profile),
            Err(error) => {
                self.status_message =
                    Some(t!("profile.load_failed", error = error.to_string()).to_string());
            }
        }
    }

    /// 应用 profile：匹配得到的设备选中，匹配不到的留空并提示；连接参数
    /// 用 profile 覆盖；缩放/语言等本地偏好不受影响。
    fn apply_profile(&mut self, profile: Profile) {
        let mut missing = Vec::new();
        if self.apply_device_ref(profile.video_device.clone(), true) {
            missing.push(t!("profile.device_missing").to_string());
        }
        if self.apply_device_ref(profile.control_device.clone(), false) {
            missing.push(t!("profile.control_missing").to_string());
        }
        self.connection = profile.connection;
        self.active_profile = Some(profile.name);
        self.status_message = if missing.is_empty() {
            None
        } else {
            Some(missing.join("；"))
        };
    }

    /// 应用“上次手动连接”快照（启动预填，不提示缺失、不进入 profile 语境）。
    fn apply_snapshot(&mut self, snapshot: ManualSnapshot) {
        self.apply_device_ref(snapshot.video_device, true);
        self.apply_device_ref(snapshot.control_device, false);
        self.connection = snapshot.connection;
    }

    /// 按 id 匹配当前枚举并选中设备；返回 true 表示“指定了设备但找不到”。
    /// 视频选中后由预览源驱动状态；控制选中后立即同步探测（与手动选择一致）。
    fn apply_device_ref(&mut self, device: Option<DeviceRef>, is_video: bool) -> bool {
        let Some(device) = device else {
            self.clear_device_selection(is_video);
            return false;
        };
        let matched = if is_video {
            self.selection
                .video_devices
                .iter()
                .find(|candidate| candidate.id == device.id)
                .map(|candidate| candidate.id.clone())
        } else {
            self.selection
                .control_devices
                .iter()
                .find(|candidate| candidate.id == device.id)
                .map(|candidate| candidate.id.clone())
        };
        match matched {
            Some(id) => {
                if is_video {
                    self.selection.selected_video_id = Some(id.clone());
                    self.selection.video_status = VideoProbeStatus::Checking;
                    self.reset_preview();
                } else {
                    self.selection.selected_control_id = Some(id.clone());
                    self.selection.control_status =
                        self.probe
                            .probe_control(&id, self.connection.baud_rate, PROBE_TIMEOUT);
                }
                false
            }
            None => {
                self.clear_device_selection(is_video);
                true
            }
        }
    }

    fn clear_device_selection(&mut self, is_video: bool) {
        if is_video {
            self.selection.selected_video_id = None;
            self.selection.video_status = VideoProbeStatus::NotSelected;
            self.reset_preview();
        } else {
            self.selection.selected_control_id = None;
            self.selection.control_status = ControlProbeStatus::NotSelected;
        }
    }

    /// 文件对话框加载 profile：默认打开 profile 目录。
    #[cfg(windows)]
    fn load_profile_from_dialog(&mut self) {
        let profiles_dir = self.store.profiles_dir();
        let _ = std::fs::create_dir_all(&profiles_dir);
        let Some(path) = rfd::FileDialog::new()
            .set_directory(&profiles_dir)
            .add_filter("profile", &["toml"])
            .pick_file()
        else {
            return;
        };
        match self.store.load_profile_file(&path) {
            Ok(profile) => self.apply_profile(profile),
            Err(error) => {
                self.status_message =
                    Some(t!("profile.load_failed", error = error.to_string()).to_string());
            }
        }
    }

    /// 保存当前选择与连接参数为 profile（设备可未选全，保存当前状态）。
    fn do_save_profile(&mut self) {
        let name = self.save_profile_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let profile = Profile {
            name: name.clone(),
            video_device: self.selected_video_ref(),
            control_device: self.selected_control_ref(),
            connection: self.connection.clone(),
        };
        match self.store.save_profile(&profile) {
            Ok(()) => {
                self.status_message = Some(t!("profile.saved", name = name).to_string());
                self.close_save_profile_dialog();
            }
            Err(error) => {
                self.status_message =
                    Some(t!("profile.save_failed", error = error.to_string()).to_string());
            }
        }
    }

    fn close_save_profile_dialog(&mut self) {
        self.show_save_profile = false;
        self.save_profile_name.clear();
        self.save_profile_confirm_overwrite = false;
    }

    fn selected_video_ref(&self) -> Option<DeviceRef> {
        selected_device_ref(
            &self.selection.video_devices,
            self.selection.selected_video_id.as_deref(),
        )
    }

    fn selected_control_ref(&self) -> Option<DeviceRef> {
        selected_device_ref(
            &self.selection.control_devices,
            self.selection.selected_control_id.as_deref(),
        )
    }

    fn toggle_mouse_mode(&mut self) {
        let next = match self.connection.mouse_mode {
            MouseMode::Absolute => MouseMode::Relative,
            MouseMode::Relative => MouseMode::Absolute,
        };
        // 在线切换：经输入泵原子更新 sink 模式，避免“UI 模式与会话 sink 分叉”
        // （此前靠重连切换，串口被旧会话占用时重连失败导致绝对/相对错位）。
        match self.session.set_mouse_mode(next) {
            Ok(()) => {
                self.connection.mouse_mode = next;
                // 热键切换视为手动改动，脱离当前 profile。
                self.active_profile = None;
                self.relative_remainder = (0.0, 0.0);
                self.relative_wheel = 0;
                self.last_pointer_sent = None;
                self.status_message = Some(
                    t!("message.mouse_mode_switched", mode = mouse_mode_label(next)).to_string(),
                );
            }
            Err(error) => {
                self.status_message = Some(
                    t!(
                        "message.mouse_mode_switch_failed",
                        error = error.to_string()
                    )
                    .to_string(),
                );
            }
        }
    }

    fn console_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.update_texture(ctx);
            let available = ui.available_size();
            let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
            painter.rect_filled(response.rect, 0.0, LETTERBOX_COLOR);
            if response.clicked() {
                response.request_focus();
            }

            let frame = self.frame_size.unwrap_or(FrameSize {
                width: 1920,
                height: 1080,
            });
            if self.scale_mode == VideoScaleMode::ResizeWindowToVideo
                && let Some(actual) = self.frame_size
                && self.last_follow_resize != Some(actual)
            {
                let chrome = if self.menu_chrome > 0.0 && self.status_chrome > 0.0 {
                    self.menu_chrome + self.status_chrome
                } else {
                    DEFAULT_CHROME_FALLBACK
                };
                let size = desired_window_inner_size(actual, chrome, ctx.pixels_per_point());
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                self.last_follow_resize = Some(actual);
            }
            let video_rect = VideoViewport::frame_rect(response.rect, frame, self.scale_mode);
            if let Some(texture) = &self.texture {
                painter.image(
                    texture.id(),
                    video_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            painter.rect_stroke(
                video_rect,
                0.0,
                egui::Stroke::new(1.0, VIDEO_BORDER_COLOR),
                egui::StrokeKind::Outside,
            );
            if self.latest_frame.is_none() || self.no_signal_elapsed() {
                painter.text(
                    video_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    t!("status.video_no_signal"),
                    egui::FontId::proportional(28.0),
                    egui::Color32::GRAY,
                );
            }

            self.handle_input(&response, video_rect, frame);
        });
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let Some(frame) = &self.latest_frame else {
            return;
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.pixels,
        );
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture = Some(ctx.load_texture("video", image, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn no_signal_elapsed(&self) -> bool {
        self.last_frame_at
            .is_some_and(|at| at.elapsed() >= NO_SIGNAL_TIMEOUT)
    }

    fn handle_input(
        &mut self,
        response: &egui::Response,
        video_rect: egui::Rect,
        frame: FrameSize,
    ) {
        if !self.session.is_control_online() {
            if self.cursor_grabbed {
                response
                    .ctx
                    .send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
                response
                    .ctx
                    .send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
                self.cursor_grabbed = false;
            }
            self.pointer_mask = 0;
            self.last_pointer = None;
            return;
        }
        // 远程输入模式是粘性状态：点击视频区进入；窗口失焦、点击本地 UI、
        // 焦点被移走或 Ctrl+Alt+K 才退出，避免逐帧聚焦判定抖动导致
        // “点一下立即失焦/永远失焦”。
        // 按下即进入（不等松开）：点击的按下帧也会作为远程按键发送，
        // 避免“先按下一帧走绝对分支、再松开才进远程”的错位。
        let pressed_video = response.is_pointer_button_down_on();
        let clicked_video = response.clicked();
        if pressed_video || clicked_video {
            response.request_focus();
        }
        let window_lost = response.ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        });
        let any_click = response.ctx.input(|input| input.pointer.any_click());

        if !self.video_focused && (pressed_video || clicked_video) {
            // 刚进入：以当前修饰键为基线，避免把历史按住状态当新按下。
            self.last_modifiers = response.ctx.input(|input| input.modifiers);
            self.video_focused = true;
        }
        let exit_remote = self.video_focused
            && (window_lost
                || (any_click && !clicked_video)
                || (!clicked_video && !response.has_focus()));
        if exit_remote {
            // 退出远程模式（点击本地 UI / 窗口失焦）：释放所有按键、交还
            // egui 焦点并复位本地状态；再次点击视频区才能重新进入。
            let _ = self.session.release_all();
            response
                .ctx
                .memory_mut(|memory| memory.surrender_focus(response.id));
            self.video_focused = false;
            self.pointer_mask = 0;
            self.last_pointer = None;
            self.relative_remainder = (0.0, 0.0);
            self.pending_pointer = None;
            self.relative_wheel = 0;
            self.last_pointer_sent = None;
            self.last_pointer_sent_at = None;
        }

        // Ctrl+Alt+K：本地退出热键。先于指针/键盘转发处理，拦截后不发送远端。
        if self.video_focused {
            let exit_requested = response
                .ctx
                .input(|input| input.events.iter().any(crate::input::is_remote_exit_combo));
            if exit_requested {
                let _ = self.session.release_all();
                response
                    .ctx
                    .memory_mut(|memory| memory.surrender_focus(response.id));
                self.video_focused = false;
                self.pointer_mask = 0;
                self.last_pointer = None;
                self.last_modifiers = response.ctx.input(|input| input.modifiers);
                return;
            }
        }

        // Ctrl+Alt+M：本地切换绝对/相对鼠标模式（重连应用）。
        if self.video_focused {
            let toggle_requested = response
                .ctx
                .input(|input| input.events.iter().any(crate::input::is_mode_toggle_combo));
            if toggle_requested {
                self.toggle_mouse_mode();
                return;
            }
        }

        if self.video_focused {
            // 锁住焦点导航：Tab/方向键/Esc 都转发远端，不让 egui 拿去移动焦点。
            response.ctx.memory_mut(|memory| {
                memory.set_focus_lock_filter(response.id, crate::input::remote_focus_filter());
            });
        }

        // 相对模式锁定并隐藏本地光标，绝对模式恢复光标。
        let relative_mode = self.connection.mouse_mode == MouseMode::Relative;
        if self.video_focused && relative_mode {
            if !self.cursor_grabbed {
                response
                    .ctx
                    .send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked));
                response
                    .ctx
                    .send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                self.cursor_grabbed = true;
            }
        } else if self.cursor_grabbed {
            response
                .ctx
                .send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
            response
                .ctx
                .send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            self.cursor_grabbed = false;
        }

        // 指针：相对模式用增量（本地光标已锁定），绝对模式用窗口坐标。
        let mask = pointer_button_mask(response, self.pointer_mask);
        if self.video_focused && relative_mode {
            // 光标锁定后位置不变，位置增量恒为 0；必须用原始鼠标运动事件
            // （eframe 从 DeviceEvent::MouseMotion 转发，物理像素）。
            let (raw_dx, raw_dy) = response.ctx.input(|input| {
                input.events.iter().fold((0.0f32, 0.0f32), |acc, event| {
                    if let egui::Event::MouseMoved(delta) = event {
                        (acc.0 + delta.x, acc.1 + delta.y)
                    } else {
                        acc
                    }
                })
            });
            let sensitivity = self.connection.relative_sensitivity;
            let pixels_per_point = response.ctx.pixels_per_point();
            let dx_points = raw_dx / pixels_per_point * sensitivity;
            let dy_points = raw_dy / pixels_per_point * sensitivity;
            let dx = dx_points * (frame.width as f32 / video_rect.width());
            let dy = dy_points * (frame.height as f32 / video_rect.height());
            // 固定间隔采样：位移持续累积到余数，每 33ms（或按键变化时）取一次
            // 整量发送，位移大小不决定发送次数。
            self.relative_remainder.0 += dx;
            self.relative_remainder.1 += dy;
            self.relative_wheel = self
                .relative_wheel
                .saturating_add(self.wheel_steps_from_events(response));
            let now = Instant::now();
            let mask_changed = mask != self.pointer_mask;
            if mask_changed
                || crate::input::throttle_elapsed(
                    now,
                    self.last_pointer_sent_at,
                    POINTER_MIN_INTERVAL,
                )
            {
                let (dx, dy) = crate::input::sample_delta(&mut self.relative_remainder);
                let wheel = self.relative_wheel;
                if dx != 0 || dy != 0 || wheel != 0 || mask_changed {
                    if let Err(error) = self.session.send_pointer_relative(mask, dx, dy, wheel) {
                        self.status_message = Some(
                            t!("message.pointer_send_failed", error = error.to_string())
                                .to_string(),
                        );
                    }
                    self.last_pointer_sent = Some((mask, u16::MAX, u16::MAX));
                    self.last_pointer_sent_at = Some(now);
                }
                self.relative_wheel = 0;
            }
            self.pointer_mask = mask;
        } else if !relative_mode
            && pointer_active(self.video_focused, mask, self.pointer_mask)
            && let Some(position) = response.ctx.input(|input| input.pointer.latest_pos())
            && let Some((x, y)) = VideoViewport::map_pointer(position, video_rect, frame)
        {
            self.pending_pointer = Some((mask, x, y));
            let now = Instant::now();
            let mask_changed = self
                .last_pointer_sent
                .is_some_and(|(last_mask, _, _)| last_mask != mask);
            if mask_changed
                || crate::input::throttle_elapsed(
                    now,
                    self.last_pointer_sent_at,
                    POINTER_MIN_INTERVAL,
                )
            {
                if let Some((send_mask, send_x, send_y)) = self.pending_pointer
                    && crate::input::pointer_changed(
                        (send_mask, send_x, send_y),
                        self.last_pointer_sent,
                    )
                {
                    if let Err(error) = self.session.send_pointer(send_mask, send_x, send_y, frame)
                    {
                        self.status_message = Some(
                            t!("message.pointer_send_failed", error = error.to_string())
                                .to_string(),
                        );
                    }
                    self.last_pointer_sent = Some((send_mask, send_x, send_y));
                    self.last_pointer_sent_at = Some(now);
                }
                self.pending_pointer = None;
            }
            let wheel = self.wheel_steps_from_events(response);
            if wheel != 0
                && let Err(error) = self.session.send_pointer_relative(mask, 0, 0, wheel)
            {
                self.status_message =
                    Some(t!("message.pointer_send_failed", error = error.to_string()).to_string());
            }
            self.last_pointer = Some((x, y));
            self.pointer_mask = mask;
        }
        if mask == 0 {
            self.pointer_mask = 0;
        }

        // 键盘：仅聚焦时发送。
        if self.video_focused {
            let modifiers = response.ctx.input(|input| input.modifiers);
            for action in modifier_diff(self.last_modifiers, modifiers) {
                self.send_key_action(action);
            }
            self.last_modifiers = modifiers;

            let events = response.ctx.input(|input| input.events.clone());
            for event in events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = event
                {
                    if repeat {
                        continue;
                    }
                    match egui_key_to_keysym(key, modifiers) {
                        Some(keysym) => {
                            if let Err(error) = self.session.send_key(pressed, keysym) {
                                self.status_message = Some(
                                    t!("message.keyboard_send_failed", error = error.to_string())
                                        .to_string(),
                                );
                            }
                        }
                        None => {
                            self.status_message = Some(t!("message.unsupported_key").to_string())
                        }
                    }
                }
            }
        }
    }

    /// 汇总本帧滚轮事件为滚轮步数（egui Line/Page 直接取整，Point 按 50 点一步）。
    fn wheel_steps_from_events(&self, response: &egui::Response) -> i8 {
        response.ctx.input(|input| {
            let total: i32 = input
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::MouseWheel { unit, delta, .. } = event {
                        Some(i32::from(crate::input::wheel_steps(*unit, delta.y)))
                    } else {
                        None
                    }
                })
                .sum();
            total.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8
        })
    }

    fn send_key_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::Down(keysym) => {
                let _ = self.session.send_key(true, keysym);
            }
            KeyAction::Up(keysym) => {
                let _ = self.session.send_key(false, keysym);
            }
        }
    }

    fn send_special(&mut self, key: SpecialKey) {
        for action in special_key_sequence(key) {
            self.send_key_action(action);
        }
    }

    fn paste(&mut self) {
        match ClipboardService::read_text() {
            Ok(text) if !text.is_empty() => {
                if self.session.paste_text(text).is_ok() {
                    self.paste_busy = true;
                }
            }
            Ok(_) => self.status_message = Some(t!("message.clipboard_empty").to_string()),
            Err(error) => {
                self.status_message = Some(
                    t!("message.clipboard_read_failed", error = error.to_string()).to_string(),
                );
            }
        }
    }

    fn screenshot_copy(&mut self) {
        let Some(frame) = self.latest_frame.clone() else {
            self.status_message = Some(t!("message.no_frame_screenshot").to_string());
            return;
        };
        match ClipboardService::copy_image(&frame) {
            Ok(()) => {
                self.status_message = Some(t!("message.screenshot_copied").to_string());
            }
            Err(error) => {
                self.status_message = Some(
                    t!("message.screenshot_copy_failed", error = error.to_string()).to_string(),
                );
            }
        }
    }

    #[cfg(windows)]
    fn screenshot_save(&mut self) {
        let Some(frame) = self.latest_frame.clone() else {
            self.status_message = Some(t!("message.no_frame_screenshot").to_string());
            return;
        };
        let Some(path) = choose_screenshot_path() else {
            return;
        };
        match save_jpeg(&path, &frame) {
            Ok(()) => {
                self.status_message = Some(
                    t!(
                        "message.screenshot_saved",
                        path = path.display().to_string()
                    )
                    .to_string(),
                );
            }
            Err(error) => {
                self.status_message = Some(
                    t!("message.screenshot_save_failed", error = error.to_string()).to_string(),
                );
            }
        }
    }

    fn stop_session(&mut self) {
        let _ = self.session.stop();
        self.latest_frame = None;
        self.frame_size = None;
        self.last_follow_resize = None;
        self.texture = None;
        self.paste_busy = false;
        self.video_focused = false;
        self.pointer_mask = 0;
        self.last_pointer = None;
        self.last_modifiers = egui::Modifiers::NONE;
        self.cursor_grabbed = false;
        self.relative_remainder = (0.0, 0.0);
        self.pending_pointer = None;
        self.relative_wheel = 0;
        self.last_pointer_sent = None;
        self.last_pointer_sent_at = None;
        if let Some(task) = self.frame_repaint_task.take() {
            task.abort();
        }
        self.last_frame_seq = None;
        self.last_frame_at = None;
    }

    fn show_device_dialog(&mut self) {
        self.stop_session();
        self.showing_device_dialog = true;
    }

    /// 菜单“设置…”：连接参数默认值 + 本地视图偏好（语言在编辑菜单里）。
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        connection_fields_ui(ui, &mut self.default_connection, "default_mouse_mode");
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(t!("settings.scale_mode"));
            let current = self.scale_mode;
            egui::ComboBox::from_id_salt("scale_mode")
                .selected_text(scale_mode_label(current))
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (VideoScaleMode::FitWindow, t!("scale_mode.fit_window")),
                        (VideoScaleMode::ActualSize, t!("scale_mode.actual_size")),
                        (
                            VideoScaleMode::ResizeWindowToVideo,
                            t!("scale_mode.resize_to_video"),
                        ),
                    ] {
                        if ui.selectable_label(current == mode, label).clicked() {
                            self.scale_mode = mode;
                        }
                    }
                });
        });
    }

    /// 主页“连接设置”：连接级副本，加载 profile 时被覆盖；改动即视为手动连接。
    fn connection_settings_ui(&mut self, ui: &mut egui::Ui) {
        let changed = connection_fields_ui(ui, &mut self.connection, "active_mouse_mode");
        if changed {
            self.active_profile = None;
        }
    }

    fn video_status_label(&self, ui: &mut egui::Ui) {
        match &self.selection.video_status {
            VideoProbeStatus::NotSelected => {
                ui.label(t!("video_status.not_selected"));
            }
            VideoProbeStatus::Checking => {
                ui.label(t!("video_status.checking"));
            }
            VideoProbeStatus::Ready(info) => {
                ui.label(t!(
                    "video_status.ready",
                    width = info.width,
                    height = info.height,
                    label = info.label
                ));
            }
            VideoProbeStatus::NoSignal => {
                ui.label(t!("video_status.no_signal"));
            }
            VideoProbeStatus::OpenFailed(error) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    t!("video_status.open_failed", error = error),
                );
            }
            VideoProbeStatus::Disconnected => {
                ui.label(t!("video_status.disconnected"));
            }
        }
    }

    fn control_status_label(&self, ui: &mut egui::Ui) {
        match &self.selection.control_status {
            ControlProbeStatus::NotSelected => {
                ui.label(t!("control_status_label.not_selected"));
            }
            ControlProbeStatus::Checking => {
                ui.label(t!("control_status_label.checking"));
            }
            ControlProbeStatus::Ready(_) => {
                ui.label(t!(
                    "control_status_label.ready",
                    port = self.selection.selected_control_id.as_deref().unwrap_or("?")
                ));
            }
            ControlProbeStatus::NotCh9329(reason) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    t!("control_status_label.not_ch9329", reason = reason),
                );
            }
            ControlProbeStatus::NoResponse => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    t!("control_status_label.no_response"),
                );
            }
            ControlProbeStatus::OpenFailed(error) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    t!("control_status_label.open_failed", error = error),
                );
            }
            ControlProbeStatus::Disconnected => {
                ui.label(t!("control_status_label.disconnected"));
            }
        }
    }

    fn status_bar_texts(&self) -> StatusBarTexts {
        let control = if !self.showing_device_dialog && !self.session.is_control_online() {
            t!("status.offline").to_string()
        } else {
            control_status_text(
                &self.selection.control_status,
                self.selection.selected_control_id.as_deref(),
            )
        };
        let keyboard = if self.paste_busy {
            t!("status.pasting").to_string()
        } else if self.video_focused {
            t!("status.remote_input").to_string()
        } else {
            t!("status.keyboard_lost").to_string()
        };
        let pointer = if self.connection.mouse_mode == MouseMode::Relative && self.video_focused {
            t!("status.relative_mode").to_string()
        } else {
            self.last_pointer
                .map(|(x, y)| format!("({x}, {y})"))
                .unwrap_or_else(|| t!("status.pointer_outside").to_string())
        };
        StatusBarTexts {
            control,
            keyboard,
            pointer,
            video: self.video_status_text(),
            message: self.status_message.clone(),
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let texts = self.status_bar_texts();
        ui.horizontal(|ui| {
            ui.label(t!("status.control_device", value = texts.control));
            ui.separator();
            ui.label(t!("status.keyboard", value = texts.keyboard));
            ui.separator();
            ui.label(t!("status.pointer", value = texts.pointer));
            ui.separator();
            ui.label(t!("status.video", value = texts.video));
            if let Some(message) = &texts.message {
                ui.separator();
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    t!("status.message", message = message),
                );
            }
        });
    }

    fn video_status_text(&self) -> String {
        match &self.latest_frame {
            Some(frame) if !self.no_signal_elapsed() => {
                format!("{}×{}", frame.width, frame.height)
            }
            Some(_) => t!("status.video_stalled").to_string(),
            None => t!("status.video_no_signal").to_string(),
        }
    }
}

struct StatusBarTexts {
    control: String,
    keyboard: String,
    pointer: String,
    video: String,
    message: Option<String>,
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.update_impl(ctx, frame);
    }

    fn on_exit(&mut self) {
        let _ = self.session.release_all();
        let _ = self.session.stop();
    }
}

/// 刷新枚举后视频预览的处理决策：以当前状态为唯一依据。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewRefreshAction {
    /// 已打开且正常 / 正在打开 / 未选择：跳过，不打断。
    Skip,
    /// 未打开或出错：先关旧源再重建预览（重新探测）。
    Reopen,
    /// 设备已从枚举消失：关闭旧源，保持断开。
    KeepDisconnected,
}

fn preview_refresh_action(status: &VideoProbeStatus, device_present: bool) -> PreviewRefreshAction {
    match status {
        VideoProbeStatus::Ready(_) | VideoProbeStatus::Checking | VideoProbeStatus::NotSelected => {
            PreviewRefreshAction::Skip
        }
        VideoProbeStatus::OpenFailed(_) | VideoProbeStatus::NoSignal => {
            PreviewRefreshAction::Reopen
        }
        VideoProbeStatus::Disconnected if device_present => PreviewRefreshAction::Reopen,
        VideoProbeStatus::Disconnected => PreviewRefreshAction::KeepDisconnected,
    }
}

/// 预览源打开后迟迟无帧的超时判定：从打开时刻起超过 `timeout` 仍无帧 → 无信号。
/// 超时判定：从参考时刻起超过 `timeout` 视为过期（用于“打开后无帧”和
/// “出帧后停帧”两种无信号场景；无参考时刻视为未过期）。
fn elapsed_since(since: Option<Instant>, timeout: Duration, now: Instant) -> bool {
    since.is_some_and(|at| now.duration_since(at) >= timeout)
}

/// 把当前选中设备固化为 DeviceRef（label 兜底用 id）。
fn selected_device_ref(
    devices: &[crate::state::DeviceOption],
    selected_id: Option<&str>,
) -> Option<DeviceRef> {
    let id = selected_id?.to_string();
    let label = devices
        .iter()
        .find(|device| device.id == id)
        .map(|device| device.label.clone())
        .unwrap_or_else(|| id.clone());
    Some(DeviceRef { id, label })
}

/// 连接参数表单（“设置”默认值对话框与“连接设置”对话框共用）。
/// 返回是否有任何字段被修改。
fn connection_fields_ui(
    ui: &mut egui::Ui,
    settings: &mut ConnectionSettings,
    mouse_mode_id: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(t!("settings.baud_rate"));
        changed |= ui
            .add(egui::DragValue::new(&mut settings.baud_rate).range(1200..=115200))
            .changed();
    });
    changed |= ui
        .checkbox(&mut settings.auto_baud, t!("settings.auto_baud"))
        .changed();
    ui.horizontal(|ui| {
        ui.label(t!("settings.preview_fps"));
        changed |= ui
            .add(egui::DragValue::new(&mut settings.preview_fps).range(1..=60))
            .changed();
    });
    ui.horizontal(|ui| {
        ui.label(t!("settings.mouse_mode"));
        let current = settings.mouse_mode;
        egui::ComboBox::from_id_salt(mouse_mode_id)
            .selected_text(mouse_mode_label(current))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current == MouseMode::Absolute, t!("mouse_mode.absolute"))
                    .clicked()
                {
                    settings.mouse_mode = MouseMode::Absolute;
                    changed = true;
                }
                if ui
                    .selectable_label(current == MouseMode::Relative, t!("mouse_mode.relative"))
                    .clicked()
                {
                    settings.mouse_mode = MouseMode::Relative;
                    changed = true;
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label(t!("settings.relative_sensitivity"));
        changed |= ui
            .add(
                egui::DragValue::new(&mut settings.relative_sensitivity)
                    .range(0.1..=5.0)
                    .speed(0.05),
            )
            .changed();
    });
    changed
}

fn control_status_text(status: &ControlProbeStatus, port: Option<&str>) -> String {
    match status {
        ControlProbeStatus::NotSelected => t!("control_status.not_selected").to_string(),
        ControlProbeStatus::Checking => t!("control_status.checking").to_string(),
        ControlProbeStatus::Ready(_) => {
            t!("control_status.ready", port = port.unwrap_or("?")).to_string()
        }
        ControlProbeStatus::NotCh9329(reason) => {
            t!("control_status.not_ch9329", reason = reason).to_string()
        }
        ControlProbeStatus::NoResponse => t!("control_status.no_response").to_string(),
        ControlProbeStatus::OpenFailed(error) => {
            t!("control_status.open_failed", error = error).to_string()
        }
        ControlProbeStatus::Disconnected => t!("control_status.offline").to_string(),
    }
}

/// ResizeWindowToVideo 模式下窗口内容区目标尺寸（点）：视频物理像素按
/// pixels_per_point 换算 + 菜单/状态栏高度。
fn desired_window_inner_size(frame: FrameSize, chrome: f32, pixels_per_point: f32) -> egui::Vec2 {
    let ppp = pixels_per_point.max(1.0);
    egui::vec2(frame.width as f32 / ppp, frame.height as f32 / ppp + chrome)
}

fn mouse_mode_label(mode: MouseMode) -> String {
    match mode {
        MouseMode::Absolute => t!("mouse_mode.absolute").to_string(),
        MouseMode::Relative => t!("mouse_mode.relative").to_string(),
    }
}

fn scale_mode_label(mode: VideoScaleMode) -> String {
    match mode {
        VideoScaleMode::FitWindow => t!("scale_mode.fit_window").to_string(),
        VideoScaleMode::ActualSize => t!("scale_mode.actual_size").to_string(),
        VideoScaleMode::ResizeWindowToVideo => t!("scale_mode.resize_to_video").to_string(),
    }
}

#[cfg(windows)]
fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter(t!("dialog.jpeg_filter"), &["jpg", "jpeg"])
        .set_file_name("my_ipkvm-screenshot.jpg")
        .save_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ControlProbeStatus;

    /// 状态文案依赖进程级 i18n locale：取全局锁并固定为中文，避免并行测试互踩。
    fn lock_zh_locale() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rust_i18n::set_locale("zh-CN");
        guard
    }

    #[test]
    fn status_texts_include_message_and_offline_state() {
        let _guard = lock_zh_locale();
        let mut app = DesktopApp::empty();
        app.showing_device_dialog = false;
        app.paste_busy = true;
        app.video_focused = true;
        app.status_message = Some("粘贴失败".into());
        app.selection.mark_control_offline();

        let texts = app.status_bar_texts();

        assert_eq!(texts.control, "离线");
        assert_eq!(texts.keyboard, "粘贴中");
        assert_eq!(texts.message, Some("粘贴失败".into()));
    }

    #[test]
    fn status_texts_default_to_idle_states() {
        let _guard = lock_zh_locale();
        let app = DesktopApp::empty();

        let texts = app.status_bar_texts();

        assert_eq!(texts.control, "未选择");
        assert_eq!(texts.keyboard, "失焦");
        assert_eq!(texts.pointer, "窗口外");
        assert_eq!(texts.message, None);
    }

    #[test]
    fn offline_sync_resets_paste_and_focus_state() {
        let mut app = DesktopApp::empty();
        app.showing_device_dialog = false;
        app.paste_busy = true;
        app.video_focused = true;
        app.pointer_mask = 1;
        app.last_pointer = Some((1, 2));
        app.last_modifiers = egui::Modifiers {
            shift: true,
            ..Default::default()
        };

        app.sync_control_state();

        assert!(!app.paste_busy);
        assert!(!app.video_focused);
        assert_eq!(app.pointer_mask, 0);
        assert!(app.last_pointer.is_none());
        assert_eq!(app.last_modifiers, egui::Modifiers::NONE);
        assert_eq!(
            app.selection.control_status,
            ControlProbeStatus::Disconnected
        );
    }

    #[test]
    fn stop_session_resets_input_state() {
        let mut app = DesktopApp::empty();
        app.paste_busy = true;
        app.video_focused = true;
        app.pointer_mask = 1;

        app.stop_session();

        assert!(!app.paste_busy);
        assert!(!app.video_focused);
        assert_eq!(app.pointer_mask, 0);
    }

    #[test]
    fn desired_window_inner_size_adds_chrome() {
        let size = desired_window_inner_size(
            FrameSize {
                width: 1280,
                height: 720,
            },
            48.0,
            1.0,
        );

        assert_eq!(size, egui::vec2(1280.0, 768.0));
    }

    #[test]
    fn status_texts_show_remote_input_hint_when_video_focused() {
        let _guard = lock_zh_locale();
        let mut app = DesktopApp::empty();
        app.showing_device_dialog = false;
        app.video_focused = true;

        let texts = app.status_bar_texts();

        assert_eq!(texts.keyboard, "远程输入中 · Ctrl+Alt+K 退出");
    }

    #[test]
    fn status_texts_show_relative_mode_hint_when_video_focused() {
        let _guard = lock_zh_locale();
        let mut app = DesktopApp::empty();
        app.showing_device_dialog = false;
        app.video_focused = true;
        app.connection.mouse_mode = MouseMode::Relative;

        let texts = app.status_bar_texts();

        assert_eq!(texts.pointer, "相对模式");
    }

    #[test]
    fn preview_refresh_skips_when_already_running_or_checking() {
        for status in [
            VideoProbeStatus::Ready(PreviewInfo {
                width: 1920,
                height: 1080,
                label: "cam".into(),
            }),
            VideoProbeStatus::Checking,
            VideoProbeStatus::NotSelected,
        ] {
            assert_eq!(
                preview_refresh_action(&status, true),
                PreviewRefreshAction::Skip,
                "已打开/正在打开/未选择都不应重开相机"
            );
        }
    }

    #[test]
    fn preview_refresh_reopens_on_failure_or_no_signal() {
        for status in [
            VideoProbeStatus::OpenFailed("boom".into()),
            VideoProbeStatus::NoSignal,
        ] {
            assert_eq!(
                preview_refresh_action(&status, true),
                PreviewRefreshAction::Reopen
            );
            assert_eq!(
                preview_refresh_action(&status, false),
                PreviewRefreshAction::Reopen
            );
        }
    }

    #[test]
    fn preview_refresh_keeps_disconnected_when_device_gone_and_reopens_when_back() {
        assert_eq!(
            preview_refresh_action(&VideoProbeStatus::Disconnected, false),
            PreviewRefreshAction::KeepDisconnected
        );
        assert_eq!(
            preview_refresh_action(&VideoProbeStatus::Disconnected, true),
            PreviewRefreshAction::Reopen
        );
    }

    #[test]
    fn preview_timeout_only_moves_checking_to_no_signal() {
        let now = Instant::now();
        let opened = now - PROBE_TIMEOUT - Duration::from_millis(1);
        assert!(elapsed_since(Some(opened), PROBE_TIMEOUT, now));
        assert!(!elapsed_since(Some(now), PROBE_TIMEOUT, now));
        assert!(!elapsed_since(None, PROBE_TIMEOUT, now));
    }

    /// 全部用户可见文案键：两侧语言都必须存在且返回非键原文。
    /// 该表同时是“新增文案必须走 t!()”的回归检查清单。
    const ALL_UI_KEYS: &[&str] = &[
        "common.not_selected",
        "common.close",
        "common.cancel",
        "menu.file",
        "menu.edit",
        "menu.send",
        "menu.about",
        "menu.reselect_device",
        "menu.stop_connection",
        "file.load_profile",
        "file.load_profile_unsupported",
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
        "about.about",
        "about.project_home",
        "about.title",
        "about.version",
        "about.license",
        "about.project_url",
        "device.title",
        "device.video",
        "device.control",
        "device.refresh",
        "device.connect",
        "device.preview",
        "device.no_preview",
        "preview.no_signal",
        "preview.open_failed",
        "profile.select",
        "profile.save",
        "profile.connection_settings",
        "profile.save_title",
        "profile.name_label",
        "profile.save_button",
        "profile.saved",
        "profile.save_failed",
        "profile.load_failed",
        "profile.overwrite_body",
        "profile.overwrite_confirm",
        "profile.device_missing",
        "profile.control_missing",
        "profile.no_recent",
        "profile.restore_defaults",
        "connection_settings.title",
        "settings.title",
        "settings.baud_rate",
        "settings.auto_baud",
        "settings.preview_fps",
        "settings.mouse_mode",
        "settings.relative_sensitivity",
        "settings.scale_mode",
        "language.system",
        "language.chinese",
        "language.english",
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
        "status.keyboard_lost",
        "status.relative_mode",
        "status.pointer_outside",
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
        "message.offline_with_reason",
        "message.offline_reconnect",
        "message.input_rejected",
        "message.enumeration_failed",
        "message.connect_failed",
        "message.baud_selected",
        "message.mouse_mode_switched",
        "message.mouse_mode_switch_failed",
        "message.pointer_send_failed",
        "message.keyboard_send_failed",
        "message.unsupported_key",
        "message.clipboard_empty",
        "message.clipboard_read_failed",
        "message.no_frame_screenshot",
        "message.screenshot_copied",
        "message.screenshot_copy_failed",
        "message.screenshot_saved",
        "message.screenshot_save_failed",
        "dialog.jpeg_filter",
    ];

    #[test]
    fn all_ui_keys_exist_in_both_locales() {
        for locale in ["zh-CN", "en"] {
            for &key in ALL_UI_KEYS {
                // 直接查精确 locale（不走 fallback），缺键时 t! 会回退成 en/键名，
                // 那种兜底不能算“该语言已翻译”。
                let text = crate::_rust_i18n_backend()
                    .translate(locale, key)
                    .unwrap_or_else(|| panic!("键 {key} 在 {locale} 缺失"));
                assert!(!text.is_empty(), "键 {key} 在 {locale} 为空");
            }
        }
    }

    #[test]
    fn arg_keys_substitute_placeholders_in_both_locales() {
        for locale in ["zh-CN", "en"] {
            let substituted = t!("status.control_device", locale = locale, value = "XYZ");
            assert!(substituted.contains("XYZ"));
            assert!(!substituted.contains("%{value}"));
        }
    }

    #[test]
    fn desired_window_inner_size_scales_physical_pixels_to_points() {
        let size = desired_window_inner_size(
            FrameSize {
                width: 1920,
                height: 1080,
            },
            48.0,
            2.5,
        );
        assert!((size.x - 768.0).abs() < 0.01);
        assert!((size.y - 480.0).abs() < 0.01);
    }

    #[test]
    fn window_icon_is_32x32_rgba() {
        assert_eq!(WINDOW_ICON.len(), 32 * 32 * 4);
    }
}
