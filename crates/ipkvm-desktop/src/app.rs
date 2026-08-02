use std::time::{Duration, Instant};

use eframe::egui;
use ipkvm_core::MouseMode;
use ipkvm_session::rfb_input::RfbInputNotice;

use crate::clipboard::{ClipboardService, save_jpeg};
use crate::frame::bgra_to_rgba;
use crate::input::{
    KeyAction, SpecialKey, egui_key_to_keysym, modifier_diff, pointer_active, pointer_button_mask,
    special_key_sequence,
};
use crate::probe::{ProbeBackend, ProductionProbeBackend, refresh_detection};
use crate::render::{FrameSize, VideoViewport};
use crate::session::{ConnectRequest, ProductionDesktopSessionController};
use crate::state::{ControlProbeStatus, DeviceSelectionState, VideoProbeStatus, VideoScaleMode};

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const NO_SIGNAL_TIMEOUT: Duration = Duration::from_secs(2);

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 600.0])
            .with_title("my_ipkvm"),
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
    pointer_mask: u8,
    last_pointer: Option<(u16, u16)>,
    last_modifiers: egui::Modifiers,
    video_focused: bool,
    paste_busy: bool,
    status_message: Option<String>,
    showing_device_dialog: bool,
    last_frame_seq: Option<u64>,
    last_frame_at: Option<Instant>,
}

impl DesktopApp {
    fn new() -> Self {
        let mut app = Self::empty();
        let mut selection = app.selection.clone();
        refresh_detection(&mut selection, &mut app.probe, PROBE_TIMEOUT);
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
            pointer_mask: 0,
            last_pointer: None,
            last_modifiers: egui::Modifiers::NONE,
            video_focused: false,
            paste_busy: false,
            status_message: None,
            showing_device_dialog: true,
            last_frame_seq: None,
            last_frame_at: None,
        }
    }

    fn update_impl(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_notices();
        self.refresh_video();
        if !self.showing_device_dialog && !self.session.is_control_online() {
            self.selection.mark_control_offline();
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu_bar(ui));
        if self.showing_device_dialog {
            self.device_dialog(ctx);
        } else {
            self.console_ui(ctx);
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
    }

    fn refresh_video(&mut self) {
        let Some(frame) = self.session.latest_frame() else {
            return;
        };
        let now = Instant::now();
        if self.last_frame_seq != Some(frame.seq) {
            self.last_frame_seq = Some(frame.seq);
            self.last_frame_at = Some(now);
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
                    self.status_message = Some("输入被拒绝".into());
                }
                RfbInputNotice::ControllerReleased { .. }
                | RfbInputNotice::TextDispatched { .. } => {}
                _ => {}
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("控制", |ui| self.control_menu(ui));
            ui.menu_button("设备", |ui| {
                if ui.button("重新选择设备").clicked() {
                    self.show_device_dialog();
                    ui.close();
                }
                if ui.button("停止连接").clicked() {
                    self.stop_session();
                    ui.close();
                }
            });
            ui.menu_button("高级设置", |ui| {
                self.advanced_ui(ui);
            });
        });
    }

    fn control_menu(&mut self, ui: &mut egui::Ui) {
        if self.paste_busy {
            ui.add_enabled(false, egui::Button::new("粘贴文本"));
        } else if ui.button("粘贴文本").clicked() {
            self.paste();
            ui.close();
        }
        if ui.button("释放所有按键").clicked() {
            let _ = self.session.release_all();
            ui.close();
        }
        if ui.button("截图复制到剪贴板").clicked() {
            self.screenshot_copy();
            ui.close();
        }
        #[cfg(windows)]
        if ui.button("截图保存为 JPEG…").clicked() {
            self.screenshot_save();
            ui.close();
        }
        #[cfg(not(windows))]
        {
            ui.add_enabled(
                false,
                egui::Button::new("截图保存（当前平台暂不支持保存对话框）"),
            );
        }
        ui.menu_button("发送特殊键", |ui| {
            for (label, key) in [
                ("Ctrl+Alt+Del", SpecialKey::CtrlAltDel),
                ("Esc", SpecialKey::Escape),
                ("Insert", SpecialKey::Insert),
                ("Delete", SpecialKey::Delete),
                ("Home", SpecialKey::Home),
                ("End", SpecialKey::End),
                ("PageUp", SpecialKey::PageUp),
                ("PageDown", SpecialKey::PageDown),
                ("←", SpecialKey::ArrowLeft),
                ("↑", SpecialKey::ArrowUp),
                ("→", SpecialKey::ArrowRight),
                ("↓", SpecialKey::ArrowDown),
            ] {
                if ui.button(label).clicked() {
                    self.send_special(key);
                    ui.close();
                }
            }
            ui.separator();
            for n in 1..=12 {
                if ui.button(format!("F{n}")).clicked() {
                    self.send_special(SpecialKey::F(n));
                    ui.close();
                }
            }
        });
    }

    fn device_dialog(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("选择设备");
            ui.add_space(4.0);

            ui.label("视频设备");
            self.video_device_combo(ui);
            self.video_status_label(ui);
            ui.add_space(8.0);

            ui.label("控制设备（CH9329）");
            self.control_device_combo(ui);
            self.control_status_label(ui);
            ui.add_space(8.0);

            egui::CollapsingHeader::new("高级").show(ui, |ui| self.advanced_ui(ui));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("刷新检测").clicked() {
                    let mut selection = self.selection.clone();
                    refresh_detection(&mut selection, &mut self.probe, PROBE_TIMEOUT);
                    self.selection = selection;
                }
                let can_connect = self.selection.can_connect();
                if ui
                    .add_enabled(can_connect, egui::Button::new("连接"))
                    .clicked()
                {
                    self.connect();
                }
            });

            if let Some(message) = &self.status_message {
                ui.colored_label(egui::Color32::LIGHT_RED, message);
            }
        });
    }

    fn video_device_combo(&mut self, ui: &mut egui::Ui) {
        let selected_text = self
            .selection
            .video_devices
            .iter()
            .find(|device| self.selection.selected_video_id.as_deref() == Some(device.id.as_str()))
            .map(|device| device.label.clone())
            .unwrap_or_else(|| "未选择".into());
        egui::ComboBox::from_id_salt("video_devices")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for device in self.selection.video_devices.clone() {
                    let selected =
                        self.selection.selected_video_id.as_deref() == Some(device.id.as_str());
                    if ui.selectable_label(selected, &device.label).clicked() {
                        self.selection.selected_video_id = Some(device.id);
                        self.selection.video_status = VideoProbeStatus::Checking;
                        self.reprobe_selected();
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
            .unwrap_or_else(|| "未选择".into());
        egui::ComboBox::from_id_salt("control_devices")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for device in self.selection.control_devices.clone() {
                    let selected =
                        self.selection.selected_control_id.as_deref() == Some(device.id.as_str());
                    if ui.selectable_label(selected, &device.label).clicked() {
                        self.selection.selected_control_id = Some(device.id);
                        self.selection.control_status = ControlProbeStatus::Checking;
                        self.reprobe_selected();
                    }
                }
            });
    }

    fn reprobe_selected(&mut self) {
        if let Some(device_id) = self.selection.selected_video_id.clone() {
            self.selection.video_status = self.probe.preview_video(
                &device_id,
                self.selection.advanced.preview_fps,
                PROBE_TIMEOUT,
            );
        }
        if let Some(device_id) = self.selection.selected_control_id.clone() {
            self.selection.control_status = self.probe.probe_control(
                &device_id,
                self.selection.advanced.baud_rate,
                PROBE_TIMEOUT,
            );
        }
    }

    fn connect(&mut self) {
        let Some(video_device_id) = self.selection.selected_video_id.clone() else {
            return;
        };
        let Some(control_device_id) = self.selection.selected_control_id.clone() else {
            return;
        };
        let advanced = self.selection.advanced.clone();
        let request = ConnectRequest {
            video_device_id,
            control_device_id,
            baud_rate: advanced.baud_rate,
            mouse_mode: advanced.mouse_mode,
            preview_fps: advanced.preview_fps,
        };
        match self.session.connect(request) {
            Ok(()) => {
                self.showing_device_dialog = false;
                self.status_message = None;
                self.pointer_mask = 0;
                self.last_pointer = None;
            }
            Err(error) => {
                self.status_message = Some(format!("连接失败：{error}"));
            }
        }
    }

    fn console_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.update_texture(ctx);
            let available = ui.available_size();
            let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
            painter.rect_filled(response.rect, 0.0, egui::Color32::from_gray(20));
            if response.clicked() {
                response.request_focus();
            }

            let frame = self.frame_size.unwrap_or(FrameSize {
                width: 1920,
                height: 1080,
            });
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
            if self.latest_frame.is_none() || self.no_signal_elapsed() {
                painter.text(
                    video_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "无信号",
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
            self.pointer_mask = 0;
            self.last_pointer = None;
            return;
        }
        let focused = response.has_focus();
        if focused && !self.video_focused {
            // 刚获得焦点：以当前修饰键为基线，避免把历史按住状态当新按下。
            self.last_modifiers = response.ctx.input(|input| input.modifiers);
        }
        if !focused && self.video_focused {
            // 失焦：释放所有按键并复位本地状态。
            let _ = self.session.release_all();
            self.pointer_mask = 0;
            self.last_pointer = None;
        }
        self.video_focused = focused;

        // 指针：悬停或拖动中发送坐标；点击视频区会先获得焦点。
        let mask = pointer_button_mask(response, self.pointer_mask);
        if pointer_active(focused, mask, self.pointer_mask)
            && let Some(position) = response.ctx.input(|input| input.pointer.latest_pos())
            && let Some((x, y)) = VideoViewport::map_pointer(position, video_rect, frame)
        {
            let _ = self.session.send_pointer(mask, x, y, frame);
            self.last_pointer = Some((x, y));
            self.pointer_mask = mask;
        }
        if mask == 0 {
            self.pointer_mask = 0;
        }

        // 键盘：仅聚焦时发送。
        if focused {
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
                            let _ = self.session.send_key(pressed, keysym);
                        }
                        None => self.status_message = Some("不支持的按键".into()),
                    }
                }
            }
        }
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
            Ok(_) => self.status_message = Some("剪贴板为空".into()),
            Err(error) => self.status_message = Some(format!("读取剪贴板失败：{error}")),
        }
    }

    fn screenshot_copy(&mut self) {
        let Some(frame) = self.latest_frame.clone() else {
            self.status_message = Some("当前无画面可截图".into());
            return;
        };
        match ClipboardService::copy_image(&frame) {
            Ok(()) => self.status_message = Some("截图已复制到剪贴板".into()),
            Err(error) => self.status_message = Some(format!("截图复制失败：{error}")),
        }
    }

    #[cfg(windows)]
    fn screenshot_save(&mut self) {
        let Some(frame) = self.latest_frame.clone() else {
            self.status_message = Some("当前无画面可截图".into());
            return;
        };
        let Some(path) = choose_screenshot_path() else {
            return;
        };
        match save_jpeg(&path, &frame) {
            Ok(()) => self.status_message = Some(format!("截图已保存到 {}", path.display())),
            Err(error) => self.status_message = Some(format!("截图保存失败：{error}")),
        }
    }

    fn stop_session(&mut self) {
        let _ = self.session.stop();
        self.latest_frame = None;
        self.frame_size = None;
        self.texture = None;
        self.pointer_mask = 0;
        self.last_pointer = None;
        self.last_frame_seq = None;
        self.last_frame_at = None;
    }

    fn show_device_dialog(&mut self) {
        self.stop_session();
        self.showing_device_dialog = true;
    }

    fn advanced_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("波特率");
            ui.add(
                egui::DragValue::new(&mut self.selection.advanced.baud_rate).range(1200..=115200),
            );
        });
        ui.horizontal(|ui| {
            ui.label("预览帧率");
            ui.add(egui::DragValue::new(&mut self.selection.advanced.preview_fps).range(1..=60));
        });
        ui.horizontal(|ui| {
            ui.label("鼠标模式");
            let current = self.selection.advanced.mouse_mode;
            egui::ComboBox::from_id_salt("mouse_mode")
                .selected_text(mouse_mode_label(current))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current == MouseMode::Absolute, "绝对坐标")
                        .clicked()
                    {
                        self.selection.advanced.mouse_mode = MouseMode::Absolute;
                    }
                    if ui
                        .selectable_label(current == MouseMode::Relative, "相对坐标")
                        .clicked()
                    {
                        self.selection.advanced.mouse_mode = MouseMode::Relative;
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("缩放");
            let current = self.selection.advanced.scale_mode;
            egui::ComboBox::from_id_salt("scale_mode")
                .selected_text(scale_mode_label(current))
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (VideoScaleMode::FitWindow, "适配窗口"),
                        (VideoScaleMode::ActualSize, "原始大小"),
                        (VideoScaleMode::ResizeWindowToVideo, "窗口跟随视频"),
                    ] {
                        if ui.selectable_label(current == mode, label).clicked() {
                            self.selection.advanced.scale_mode = mode;
                        }
                    }
                });
        });
    }

    fn video_status_label(&self, ui: &mut egui::Ui) {
        match &self.selection.video_status {
            VideoProbeStatus::NotSelected => {
                ui.label("视频：未选择");
            }
            VideoProbeStatus::Checking => {
                ui.label("视频：检测中…");
            }
            VideoProbeStatus::Ready(info) => {
                ui.label(format!(
                    "视频：预览可用 {}×{}（{}）",
                    info.width, info.height, info.label
                ));
            }
            VideoProbeStatus::NoSignal => {
                ui.label("视频：无信号");
            }
            VideoProbeStatus::OpenFailed(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("视频：打开失败 {error}"));
            }
            VideoProbeStatus::Disconnected => {
                ui.label("视频：设备已断开");
            }
        }
    }

    fn control_status_label(&self, ui: &mut egui::Ui) {
        match &self.selection.control_status {
            ControlProbeStatus::NotSelected => {
                ui.label("控制：未选择");
            }
            ControlProbeStatus::Checking => {
                ui.label("控制：探测中…");
            }
            ControlProbeStatus::Ready(info) => {
                ui.label(format!("控制：合法 CH9329（版本 {:#04x}）", info.version));
            }
            ControlProbeStatus::NotCh9329(reason) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("控制：不是合法 CH9329（{reason}）"),
                );
            }
            ControlProbeStatus::NoResponse => {
                ui.colored_label(egui::Color32::LIGHT_RED, "控制：无应答");
            }
            ControlProbeStatus::OpenFailed(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("控制：打开失败 {error}"));
            }
            ControlProbeStatus::Disconnected => {
                ui.label("控制：设备已断开");
            }
        }
    }

    fn status_bar_texts(&self) -> StatusBarTexts {
        let control = if !self.showing_device_dialog && !self.session.is_control_online() {
            "离线".to_owned()
        } else {
            control_status_text(&self.selection.control_status)
        };
        let keyboard = if self.paste_busy {
            "粘贴中".to_owned()
        } else if self.video_focused {
            "聚焦可输入".to_owned()
        } else {
            "失焦".to_owned()
        };
        let pointer = self
            .last_pointer
            .map(|(x, y)| format!("({x}, {y})"))
            .unwrap_or_else(|| "窗口外".into());
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
            ui.label(format!("控制设备：{}", texts.control));
            ui.separator();
            ui.label(format!("键盘：{}", texts.keyboard));
            ui.separator();
            ui.label(format!("鼠标：{}", texts.pointer));
            ui.separator();
            ui.label(format!("视频：{}", texts.video));
            if let Some(message) = &texts.message {
                ui.separator();
                ui.colored_label(egui::Color32::LIGHT_RED, format!("状态：{message}"));
            }
        });
    }

    fn video_status_text(&self) -> String {
        match &self.latest_frame {
            Some(frame) if !self.no_signal_elapsed() => {
                format!("{}×{}", frame.width, frame.height)
            }
            Some(_) => "断流/无信号".into(),
            None => "无信号".into(),
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

fn control_status_text(status: &ControlProbeStatus) -> String {
    match status {
        ControlProbeStatus::NotSelected => "未选择".into(),
        ControlProbeStatus::Checking => "重新探测中".into(),
        ControlProbeStatus::Ready(info) => format!("合法 CH9329（版本 {:#04x}）", info.version),
        ControlProbeStatus::NotCh9329(reason) => format!("非 CH9329（{reason}）"),
        ControlProbeStatus::NoResponse => "无应答".into(),
        ControlProbeStatus::OpenFailed(error) => format!("打开失败（{error}）"),
        ControlProbeStatus::Disconnected => "离线".into(),
    }
}

fn mouse_mode_label(mode: MouseMode) -> &'static str {
    match mode {
        MouseMode::Absolute => "绝对坐标",
        MouseMode::Relative => "相对坐标",
    }
}

fn scale_mode_label(mode: VideoScaleMode) -> &'static str {
    match mode {
        VideoScaleMode::FitWindow => "适配窗口",
        VideoScaleMode::ActualSize => "原始大小",
        VideoScaleMode::ResizeWindowToVideo => "窗口跟随视频",
    }
}

#[cfg(windows)]
fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("JPEG image", &["jpg", "jpeg"])
        .set_file_name("my_ipkvm-screenshot.jpg")
        .save_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_texts_include_message_and_offline_state() {
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
        let app = DesktopApp::empty();

        let texts = app.status_bar_texts();

        assert_eq!(texts.control, "未选择");
        assert_eq!(texts.keyboard, "失焦");
        assert_eq!(texts.pointer, "窗口外");
        assert_eq!(texts.message, None);
    }
}
