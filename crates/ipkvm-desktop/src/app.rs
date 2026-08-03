use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use ipkvm_core::MouseMode;
use ipkvm_session::rfb_input::RfbInputNotice;
use ipkvm_video::FrameSource;
use rust_i18n::t;

use crate::clipboard::{ClipboardService, save_jpeg};
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

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title(format!("my_ipkvm {}", env!("GIT_COMMIT"))),
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
    show_special_keys: bool,
    show_advanced: bool,
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
        // 跟随系统语言（检测失败回退项目默认中文）；显式语言选择在设置里覆盖。
        AppLanguage::System.apply();
        let mut selection = app.selection.clone();
        if let Err(error) = refresh_detection(&mut selection, &mut app.probe, PROBE_TIMEOUT) {
            eprintln!("warning: 初始设备枚举失败：{error}");
        }
        app.selection = selection;
        app
    }

    /// 空构造器：不枚举设备，由 `new()` 追加启动刷新；测试直接使用。
    fn empty() -> Self {
        Self {
            selection: DeviceSelectionState::default(),
            probe: ProductionProbeBackend,
            session: ProductionDesktopSessionController::production(),
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
            show_special_keys: false,
            show_advanced: false,
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

        if self.show_special_keys {
            egui::Modal::new(egui::Id::new("special_keys_modal")).show(ctx, |ui| {
                ui.set_min_width(240.0);
                ui.heading(t!("special_keys.title"));
                ui.add_space(8.0);
                ui.label(t!("special_keys.hint"));
                ui.add_space(8.0);
                if ui.button(t!("special_keys.ctrl_alt_del")).clicked() {
                    self.send_special(SpecialKey::CtrlAltDel);
                }
                if ui.button(t!("special_keys.esc")).clicked() {
                    self.send_special(SpecialKey::Escape);
                }
                ui.add_space(8.0);
                if ui.button(t!("common.close")).clicked() {
                    self.show_special_keys = false;
                }
            });
        }
        if self.show_advanced {
            egui::Modal::new(egui::Id::new("advanced_modal")).show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.heading(t!("advanced.title"));
                ui.add_space(8.0);
                self.advanced_ui(ui);
                ui.add_space(8.0);
                if ui.button(t!("common.close")).clicked() {
                    self.show_advanced = false;
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
            ui.menu_button(t!("menu.control"), |ui| self.control_menu(ui));
            ui.menu_button(t!("menu.device"), |ui| {
                if ui.button(t!("menu.reselect_device")).clicked() {
                    self.show_device_dialog();
                    ui.close();
                }
                if ui.button(t!("menu.stop_connection")).clicked() {
                    self.stop_session();
                    ui.close();
                }
            });
            if ui.button(t!("menu.advanced")).clicked() {
                self.show_advanced = true;
            }
        });
    }

    fn control_menu(&mut self, ui: &mut egui::Ui) {
        if self.paste_busy {
            ui.add_enabled(false, egui::Button::new(t!("menu.paste_text")));
        } else if ui.button(t!("menu.paste_text")).clicked() {
            self.paste();
            ui.close();
        }
        if ui.button(t!("menu.release_keys")).clicked() {
            let _ = self.session.release_all();
            ui.close();
        }
        if ui.button(t!("menu.copy_screenshot")).clicked() {
            self.screenshot_copy();
            ui.close();
        }
        #[cfg(windows)]
        if ui.button(t!("menu.save_screenshot")).clicked() {
            self.screenshot_save();
            ui.close();
        }
        #[cfg(not(windows))]
        {
            ui.add_enabled(
                false,
                egui::Button::new(t!("menu.save_screenshot_unsupported")),
            );
        }
        if ui.button(t!("menu.send_special_keys")).clicked() {
            self.show_special_keys = true;
            ui.close();
        }
    }

    fn device_dialog(&mut self, ctx: &egui::Context) {
        self.update_preview(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(t!("device.title"));
            ui.add_space(6.0);
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

                    egui::CollapsingHeader::new(t!("device.advanced"))
                        .show(ui, |ui| self.advanced_ui(ui));
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button(t!("device.refresh")).clicked() {
                            let mut selection = self.selection.clone();
                            match refresh_detection(&mut selection, &mut self.probe, PROBE_TIMEOUT)
                            {
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
                match ipkvm_video::camera::CameraSource::open(
                    id,
                    self.selection.advanced.preview_fps,
                ) {
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
                                self.selection.advanced.baud_rate,
                                PROBE_TIMEOUT,
                            );
                        }
                    }
                }
            });
    }

    fn connect(&mut self, ctx: &egui::Context) -> Result<(), DesktopSessionError> {
        // 预览源占用相机/串口，连接前必须先释放。
        self.reset_preview();
        if self.selection.advanced.auto_baud
            && let Some(control_id) = self.selection.selected_control_id.clone()
            && let Some(baud) = crate::probe::detect_baud_rate(&control_id, BAUD_PROBE_TIMEOUT)
        {
            self.selection.advanced.baud_rate = baud;
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
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn connect_request(&self) -> Option<ConnectRequest> {
        Some(ConnectRequest {
            video_device_id: self.selection.selected_video_id.clone()?,
            control_device_id: self.selection.selected_control_id.clone()?,
            baud_rate: self.selection.advanced.baud_rate,
            mouse_mode: self.selection.advanced.mouse_mode,
            preview_fps: self.selection.advanced.preview_fps,
        })
    }

    fn toggle_mouse_mode(&mut self) {
        let next = match self.selection.advanced.mouse_mode {
            MouseMode::Absolute => MouseMode::Relative,
            MouseMode::Relative => MouseMode::Absolute,
        };
        // 在线切换：经输入泵原子更新 sink 模式，避免“UI 模式与会话 sink 分叉”
        // （此前靠重连切换，串口被旧会话占用时重连失败导致绝对/相对错位）。
        match self.session.set_mouse_mode(next) {
            Ok(()) => {
                self.selection.advanced.mouse_mode = next;
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
            if self.selection.advanced.scale_mode == VideoScaleMode::ResizeWindowToVideo
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
            let video_rect =
                VideoViewport::frame_rect(response.rect, frame, self.selection.advanced.scale_mode);
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
        let relative_mode = self.selection.advanced.mouse_mode == MouseMode::Relative;
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
            let sensitivity = self.selection.advanced.relative_sensitivity;
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

    fn advanced_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(t!("settings.baud_rate"));
            ui.add(
                egui::DragValue::new(&mut self.selection.advanced.baud_rate).range(1200..=115200),
            );
        });
        ui.checkbox(
            &mut self.selection.advanced.auto_baud,
            t!("settings.auto_baud"),
        );
        ui.horizontal(|ui| {
            ui.label(t!("settings.preview_fps"));
            ui.add(egui::DragValue::new(&mut self.selection.advanced.preview_fps).range(1..=60));
        });
        ui.horizontal(|ui| {
            ui.label(t!("settings.mouse_mode"));
            let current = self.selection.advanced.mouse_mode;
            egui::ComboBox::from_id_salt("mouse_mode")
                .selected_text(mouse_mode_label(current))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current == MouseMode::Absolute, t!("mouse_mode.absolute"))
                        .clicked()
                    {
                        self.selection.advanced.mouse_mode = MouseMode::Absolute;
                    }
                    if ui
                        .selectable_label(current == MouseMode::Relative, t!("mouse_mode.relative"))
                        .clicked()
                    {
                        self.selection.advanced.mouse_mode = MouseMode::Relative;
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(t!("settings.relative_sensitivity"));
            ui.add(
                egui::DragValue::new(&mut self.selection.advanced.relative_sensitivity)
                    .range(0.1..=5.0)
                    .speed(0.05),
            );
        });
        ui.horizontal(|ui| {
            ui.label(t!("settings.scale_mode"));
            let current = self.selection.advanced.scale_mode;
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
                            self.selection.advanced.scale_mode = mode;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(t!("settings.language"));
            let current = self.language;
            egui::ComboBox::from_id_salt("language")
                .selected_text(current.label())
                .show_ui(ui, |ui| {
                    for option in AppLanguage::ALL {
                        if ui
                            .selectable_label(current == option, option.label())
                            .clicked()
                        {
                            self.language = option;
                            option.apply();
                        }
                    }
                });
        });
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
        let pointer =
            if self.selection.advanced.mouse_mode == MouseMode::Relative && self.video_focused {
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
        app.selection.advanced.mouse_mode = MouseMode::Relative;

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
        "menu.control",
        "menu.device",
        "menu.reselect_device",
        "menu.stop_connection",
        "menu.advanced",
        "menu.paste_text",
        "menu.release_keys",
        "menu.copy_screenshot",
        "menu.save_screenshot",
        "menu.save_screenshot_unsupported",
        "menu.send_special_keys",
        "device.title",
        "device.video",
        "device.control",
        "device.advanced",
        "device.refresh",
        "device.connect",
        "device.preview",
        "device.no_preview",
        "preview.no_signal",
        "preview.open_failed",
        "special_keys.title",
        "special_keys.hint",
        "special_keys.ctrl_alt_del",
        "special_keys.esc",
        "advanced.title",
        "settings.baud_rate",
        "settings.auto_baud",
        "settings.preview_fps",
        "settings.mouse_mode",
        "settings.relative_sensitivity",
        "settings.scale_mode",
        "settings.language",
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
}
