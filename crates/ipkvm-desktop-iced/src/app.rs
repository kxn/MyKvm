//! 应用状态/消息/视图/订阅（M2）：菜单/模态/连接页/profile + M1 视频链路。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::border::Border;
use iced::widget::image::Handle;
use iced::{Color, Element, Length, Size, Subscription, Task};
use ipkvm_core::{
    Ch9329InputSink, InputError, InputSink, KeyEvent, MouseMode, PointerButton, PointerEvent,
    SerialCommandQueue,
};
use ipkvm_desktop::{
    ConnectRequest, DesktopSessionController, DesktopSessionError,
    ProductionDesktopSessionController, ProductionSessionFactory, SessionParts,
};
use ipkvm_session::rfb_connection::RfbConnectionGate;
use ipkvm_session::rfb_input::RfbInputNotice;
use ipkvm_video::mock::MockFrameSource;
use ipkvm_video::{FrameSource, VideoFrame};
use rust_i18n::t;

use crate::clipboard::{ClipboardReader, SystemClipboard};
use crate::connect::{
    CameraPreviewFactory, ConnectionSettings, ControlProbeStatus, DeviceSelectionState,
    PROBE_TIMEOUT, PreviewRefreshAction, PreviewRuntime, PreviewSourceFactory,
    ProductionProbeBackend, VideoProbeStatus, preview_refresh_action, refresh_detection,
    resolve_connect_baud,
};
use crate::diag;
use crate::frames::{FrameUpdate, frame_subscription};
use crate::input::{
    KeyAction, SpecialKey, is_mode_toggle_combo, is_remote_exit_combo, modifier_diff,
    special_key_from_menu, special_key_sequence, wheel_steps,
};
use crate::keymap::physical_code_to_keysym;
use crate::locale::AppLanguage;
use crate::menu::{MenuAction, menu_bar};
use crate::modal::{ModalAction, ModalKind, ModalState};
use crate::perf::FrameStats;
use crate::preloaded::PreloadedImage;
use crate::profile::{apply_profile_to_selection, build_profile};
use crate::relative::{
    ChannelRelativeFactory, DeltaReceiver, DeltaSampler, RelativePointerSource,
    RelativeSourceFactory,
};
use crate::scale::{FrameSize, ScaleMode};
use crate::status::{ConnectionStatus, derive_status};
use crate::video::handle_from_frame;
use crate::{DEFAULT_WINDOW_SIZE, WINDOW_TITLE};

/// mock 实例序号：每个 new_mock 用独立临时目录，避免测试并行互踩。
static MOCK_STORE_SEQ: AtomicU64 = AtomicU64::new(0);

/// UiTick 节流间隔（#89）：flush_pending/drain_notices 足够及时，避免 60Hz
/// 无条件重绘放大闪烁与 CPU 占用。
const UI_TICK_INTERVAL: Duration = Duration::from_millis(33);

/// 指针最小发送间隔（对齐 egui app.rs：#102 限频，避免高频移动刷爆串口）。
const POINTER_MIN_INTERVAL: Duration = Duration::from_millis(33);

/// 项目主页（实际仓库为内网 Gitea）。
pub const PROJECT_URL: &str = "http://10.10.10.5:3000/kxn/my_ipkvm";

/// 记录型 sink：测试与 mock 连接用。
#[derive(Clone, Debug, Default)]
pub struct RecordingSink {
    pub key_batches: Arc<std::sync::Mutex<usize>>,
    pub pointer_batches: Arc<std::sync::Mutex<usize>>,
    /// 按到达顺序记录的键事件（down, HID usage）。
    pub key_events: Arc<std::sync::Mutex<Vec<(bool, u8)>>>,
    /// 按到达顺序记录的绝对指针坐标（x, y）。
    pub absolute_moves: Arc<std::sync::Mutex<Vec<(u16, u16)>>>,
    /// 按到达顺序记录的相对指针增量（dx, dy）。
    pub relative_deltas: Arc<std::sync::Mutex<Vec<(i16, i16)>>>,
    /// 按到达顺序记录的相对指针发送（mask, dx, dy, wheel）。
    pub relative_events: Arc<std::sync::Mutex<Vec<(u8, i16, i16, i8)>>>,
    /// 重建按钮掩码：sink 只收到事件流，跨批次跟踪当前掩码。
    relative_mask: Arc<std::sync::Mutex<u8>>,
    /// release_all 调用次数。
    pub releases: Arc<std::sync::Mutex<usize>>,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> Result<(), InputError> {
        Ok(())
    }

    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> Result<(), InputError> {
        *self.key_batches.lock().unwrap() += 1;
        let mut recorded = self.key_events.lock().unwrap();
        for event in _events {
            match event {
                KeyEvent::Down { usage } => recorded.push((true, usage.get())),
                KeyEvent::Up { usage } => recorded.push((false, usage.get())),
            }
        }
        Ok(())
    }

    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> Result<(), InputError> {
        *self.pointer_batches.lock().unwrap() += 1;
        let mut absolute_moves = self.absolute_moves.lock().unwrap();
        let mut relative_deltas = self.relative_deltas.lock().unwrap();
        let mut relative_events = self.relative_events.lock().unwrap();
        let mut mask = self.relative_mask.lock().unwrap();
        let mut has_absolute = false;
        let mut has_relative = false;
        let mut dx = 0i16;
        let mut dy = 0i16;
        let mut wheel = 0i8;
        for event in _events {
            match event {
                PointerEvent::AbsoluteMove { x, y, .. } => {
                    has_absolute = true;
                    absolute_moves.push((*x as u16, *y as u16));
                }
                PointerEvent::RelativeMove { dx: rx, dy: ry } => {
                    has_relative = true;
                    dx = dx.saturating_add(*rx);
                    dy = dy.saturating_add(*ry);
                    relative_deltas.push((*rx, *ry));
                }
                PointerEvent::Button { button, down } => {
                    has_relative = true;
                    let bit = match button {
                        PointerButton::Left => 0b001,
                        PointerButton::Right => 0b010,
                        PointerButton::Middle => 0b100,
                    };
                    if *down {
                        *mask |= bit;
                    } else {
                        *mask &= !bit;
                    }
                }
                PointerEvent::Wheel { delta } => {
                    has_relative = true;
                    wheel = wheel.saturating_add(*delta as i8);
                }
            }
        }
        if has_relative && !has_absolute {
            relative_events.push((*mask, dx, dy, wheel));
        }
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        *self.releases.lock().unwrap() += 1;
        Ok(())
    }
}

pub type MockFactory =
    Box<dyn FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>>;

/// 测试/示例用 App 类型。
pub type MockApp = App<RecordingSink, MockFactory>;
/// 生产 App 类型（真实相机 + CH9329 串口）。
pub type ProductionApp = App<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory>;

/// 测试用探测后端：控制设备永远 Ready。
#[derive(Default)]
struct FakeProbeBackend;

impl ipkvm_desktop::probe::ProbeBackend for FakeProbeBackend {
    fn list_video_devices(
        &mut self,
    ) -> Result<Vec<crate::connect::DeviceOption>, ipkvm_desktop::probe::ProbeError> {
        Ok(vec![crate::connect::DeviceOption {
            id: "cam0".into(),
            label: "Camera 0".into(),
        }])
    }

    fn list_control_devices(
        &mut self,
    ) -> Result<Vec<crate::connect::DeviceOption>, ipkvm_desktop::probe::ProbeError> {
        Ok(vec![crate::connect::DeviceOption {
            id: "COM9".into(),
            label: "CH9329 (COM9)".into(),
        }])
    }

    fn probe_control(
        &mut self,
        _device_id: &str,
        _baud_rate: u32,
        _timeout: Duration,
    ) -> ControlProbeStatus {
        ControlProbeStatus::Ready(crate::connect::ControlInfo {
            version: 0x31,
            usb_enumerated: true,
            baud: 115200,
        })
    }
}

/// 应用消息。
#[derive(Clone, Debug)]
pub enum Message {
    FrameReady(VideoFrame),
    FrameClosed,
    SetScaleMode(ScaleMode),
    SetLetterboxColor(Color),
    ToggleLocale,
    WindowOpened(iced::window::Id),
    Menu(MenuAction),
    Modal(ModalAction),
    OpenModal(ModalKind),
    SelectVideo(String),
    SelectControl(String),
    RefreshDevices,
    Connect,
    Disconnect,
    PreviewTick,
    ScreenshotPath(Option<std::path::PathBuf>),
    SetBaudRate(u32),
    SetAutoBaud(bool),
    SetPreviewFps(u64),
    SetMouseMode(MouseMode),
    LoadProfile(String),
    ProfilePath(Option<std::path::PathBuf>),
    Keyboard(iced::keyboard::Event),
    IcedEvent(iced::Event),
    UiTick,
    FontsLoaded(Result<(), iced::font::Error>),
}

/// 应用状态：controller + 连接页 + 菜单/模态 + 视频。
pub struct App<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: FnMut(&ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError>,
{
    pub(crate) controller: DesktopSessionController<S, F>,
    frame_source: Option<Arc<MockFrameSource>>,
    handle: Option<Handle>,
    frame_size: Option<FrameSize>,
    latest_frame: Option<ipkvm_video::VideoFrame>,
    language: crate::locale::AppLanguage,
    scale_mode: ScaleMode,
    letterbox_color: Color,
    status: ConnectionStatus,
    subscribed: bool,
    zh: bool,
    window_id: Option<iced::window::Id>,
    pending_resize: Option<Size>,
    stats: Option<Arc<FrameStats>>,
    modal: ModalState,
    selection: DeviceSelectionState,
    connection: ConnectionSettings,
    default_connection: ConnectionSettings,
    probe: Box<dyn ipkvm_desktop::probe::ProbeBackend>,
    preview: PreviewRuntime,
    preview_factory: Arc<dyn PreviewSourceFactory>,
    preview_handle: Option<Handle>,
    /// 预览最近一次显示的帧 seq（同 seq 不重建 Handle，避免重复上传闪烁）。
    last_preview_seq: Option<u64>,
    /// 诊断用：上一次记录的在线状态（检测整页切换抖动）。
    last_diag_online: Option<bool>,
    store: ipkvm_desktop::config::ProfileStore,
    active_profile: Option<String>,
    status_message: Option<String>,
    remote_input: bool,
    last_modifiers: iced::keyboard::Modifiers,
    relative_source: Option<Box<dyn RelativePointerSource>>,
    relative_rx: Option<DeltaReceiver>,
    relative_sampler: DeltaSampler,
    relative_wheel: i8,
    pointer_mask: u8,
    last_relative_mask: u8,
    video_bounds: std::rc::Rc<std::cell::RefCell<Option<iced::Rectangle>>>,
    last_cursor: Option<iced::Point>,
    scale_factor: f32,
    last_pointer_sent: Option<(u8, u16, u16)>,
    last_pointer_sent_at: Option<std::time::Instant>,
    last_pointer: Option<(u16, u16)>,
    paste_busy: bool,
    recording: Option<RecordingSink>,
    clipboard: Arc<dyn ClipboardReader>,
    relative_factory: Arc<dyn RelativeSourceFactory>,
    cursor: Arc<dyn crate::platform::cursor::CursorController>,
    /// 测试记录句柄：与 cursor 指向同一控制器（仅 test 构建存在）。
    #[cfg(test)]
    cursor_records: Arc<RecordingCursorController>,
    dark: bool,
}

impl App<RecordingSink, MockFactory> {
    /// 构造并连接 mock 会话（测试/示例用）。
    pub fn new_mock() -> (Self, Task<Message>) {
        let frame_source = Arc::new(MockFrameSource::new());
        let fs = Arc::clone(&frame_source);
        let recording = RecordingSink::default();
        let recording_for_factory = recording.clone();
        let factory: MockFactory = Box::new(move |_req| {
            let src: Arc<dyn FrameSource> = fs.clone();
            Ok((src, recording_for_factory.clone(), RfbConnectionGate::new()))
        });
        let mut controller = DesktopSessionController::with_factory(factory);
        controller.connect(connect_request()).expect("mock connect");
        let status = derive_status(
            controller.is_control_online(),
            controller.input_offline_reason(),
        );
        let seq = MOCK_STORE_SEQ.fetch_add(1, Ordering::Relaxed);
        #[cfg(test)]
        let cursor_records = Arc::new(RecordingCursorController::default());
        let mut app = Self {
            controller,
            frame_source: Some(frame_source),
            handle: None,
            frame_size: None,
            latest_frame: None,
            language: crate::locale::AppLanguage::System,
            scale_mode: ScaleMode::FitWindow,
            letterbox_color: Color::from_rgb(0.91, 0.91, 0.91),
            status,
            subscribed: true,
            zh: true,
            window_id: None,
            pending_resize: None,
            stats: None,
            modal: ModalState::default(),
            selection: DeviceSelectionState::default(),
            connection: ConnectionSettings::default(),
            default_connection: ConnectionSettings::default(),
            probe: Box::new(FakeProbeBackend),
            preview: PreviewRuntime::default(),
            preview_factory: Arc::new(MockPreviewFactory),
            preview_handle: None,
            last_preview_seq: None,
            last_diag_online: None,
            store: ipkvm_desktop::config::ProfileStore::new(
                std::env::temp_dir()
                    .join(format!("my-ipkvm-iced-mock-{}-{seq}", std::process::id())),
            ),
            active_profile: None,
            status_message: None,
            remote_input: false,
            last_modifiers: iced::keyboard::Modifiers::empty(),
            relative_source: None,
            relative_rx: None,
            relative_sampler: DeltaSampler::new(Duration::from_millis(33)),
            relative_wheel: 0,
            pointer_mask: 0,
            last_relative_mask: 0,
            video_bounds: std::rc::Rc::new(std::cell::RefCell::new(None)),
            last_cursor: None,
            scale_factor: 1.0,
            last_pointer_sent: None,
            last_pointer_sent_at: None,
            last_pointer: None,
            paste_busy: false,
            recording: Some(recording),
            clipboard: Arc::new(SystemClipboard),
            relative_factory: Arc::new(ChannelRelativeFactory::new()),
            #[cfg(test)]
            cursor: cursor_records.clone(),
            #[cfg(not(test))]
            cursor: Arc::new(crate::platform::cursor::ProductionCursorController::default()),
            #[cfg(test)]
            cursor_records,
            dark: false,
        };
        // 对齐 egui 启动行为：预填上次手动连接（mock 的临时 store 通常为空）。
        app.prefill_last_manual();
        (app, Task::done(Self::startup_message()))
    }
}

impl App<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory> {
    /// 构造生产应用（真实 controller + 设备探测 + 相机预览 + 磁盘 profile）。
    pub fn production() -> (Self, Task<Message>) {
        let zh = crate::locale::apply_system_locale();
        let controller = ProductionDesktopSessionController::production();
        let store = ipkvm_desktop::config::ProfileStore::production();
        let mut app = Self {
            controller,
            frame_source: None,
            handle: None,
            frame_size: None,
            latest_frame: None,
            language: crate::locale::AppLanguage::System,
            scale_mode: ScaleMode::FitWindow,
            letterbox_color: Color::from_rgb(0.91, 0.91, 0.91),
            status: ConnectionStatus::Disconnected,
            subscribed: true,
            zh,
            window_id: None,
            pending_resize: None,
            stats: None,
            modal: ModalState::default(),
            selection: DeviceSelectionState::default(),
            connection: ConnectionSettings::default(),
            default_connection: ConnectionSettings::default(),
            probe: Box::new(ProductionProbeBackend),
            preview: PreviewRuntime::default(),
            preview_factory: Arc::new(CameraPreviewFactory),
            preview_handle: None,
            last_preview_seq: None,
            last_diag_online: None,
            store,
            active_profile: None,
            status_message: None,
            remote_input: false,
            last_modifiers: iced::keyboard::Modifiers::empty(),
            relative_source: None,
            relative_rx: None,
            relative_sampler: DeltaSampler::new(Duration::from_millis(33)),
            relative_wheel: 0,
            pointer_mask: 0,
            last_relative_mask: 0,
            video_bounds: std::rc::Rc::new(std::cell::RefCell::new(None)),
            last_cursor: None,
            scale_factor: 1.0,
            last_pointer_sent: None,
            last_pointer_sent_at: None,
            last_pointer: None,
            paste_busy: false,
            recording: None,
            clipboard: Arc::new(SystemClipboard),
            relative_factory: Arc::new(crate::platform::PlatformRelativeSourceFactory),
            cursor: Arc::new(crate::platform::cursor::ProductionCursorController::default()),
            // 仅测试构建存在：production() 在 test 构建也需初始化该字段，运行期不会被使用。
            #[cfg(test)]
            cursor_records: Arc::new(RecordingCursorController::default()),
            dark: false,
        };
        // 预填上次手动连接（对齐 egui new()），随后由启动 Task 自动枚举。
        app.prefill_last_manual();
        let mut startup_tasks = vec![Task::done(Self::startup_message())];
        startup_tasks.extend(crate::fonts::load_tasks());
        (app, Task::batch(startup_tasks))
    }
}

impl<S, F> App<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: FnMut(&ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError>,
{
    pub fn with_stats(mut self, stats: Arc<FrameStats>) -> Self {
        self.stats = Some(stats);
        self
    }

    /// 启动后立即执行的动作：自动枚举设备（对齐 egui `DesktopApp::new()` 的启动刷新）。
    /// `Task::done` 的 units 为 1（`Task::none` 为 0），测试可据此断言接线。
    pub(crate) fn startup_message() -> Message {
        Message::RefreshDevices
    }

    /// 预填上次手动连接快照（对齐 egui `new()`：连接参数 + 设备选择）。
    ///
    /// 构造时设备列表尚未枚举，选中 id 直接预填；`RefreshDevices` 完成后由
    /// `refresh_detection` 复核（设备缺失会置为 `Disconnected`，不阻塞启动）。
    fn prefill_last_manual(&mut self) {
        let Some(snapshot) = self.store.last_manual() else {
            return;
        };
        self.connection = snapshot.connection;
        if let Some(device) = snapshot.video_device {
            self.selection.selected_video_id = Some(device.id);
            self.selection.video_status = VideoProbeStatus::Checking;
        }
        if let Some(device) = snapshot.control_device {
            self.selection.selected_control_id = Some(device.id);
            self.selection.control_status = ControlProbeStatus::Checking;
        }
    }

    /// 清空预览 Handle 与帧 seq 记录（换源/断开时调用）。
    fn reset_preview_handle(&mut self) {
        self.preview_handle = None;
        self.last_preview_seq = None;
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FrameReady(frame) => {
                diag::log(format!("FrameReady seq={}", frame.seq));
                self.handle = Some(handle_from_frame(&frame));
                if let Some(stats) = &self.stats {
                    stats.record_at(Instant::now());
                }
                self.frame_size = Some(FrameSize {
                    width: frame.width,
                    height: frame.height,
                });
                self.latest_frame = Some(frame);
                self.sync_status();
                if self.scale_mode == ScaleMode::ResizeWindowToVideo
                    && let Some(size) = desired_window_size(self.frame_size, self.scale_mode)
                {
                    if let Some(id) = self.window_id {
                        return iced::window::resize(id, size);
                    }
                    self.pending_resize = Some(size);
                }
                Task::none()
            }
            Message::FrameClosed => {
                self.subscribed = false;
                Task::none()
            }
            Message::SetScaleMode(mode) => {
                self.scale_mode = mode;
                Task::none()
            }
            Message::SetLetterboxColor(color) => {
                self.letterbox_color = color;
                Task::none()
            }
            Message::ToggleLocale => {
                if self.zh {
                    AppLanguage::English.apply();
                } else {
                    AppLanguage::Chinese.apply();
                }
                self.zh = rust_i18n::locale().starts_with("zh");
                self.language = if self.zh {
                    AppLanguage::Chinese
                } else {
                    AppLanguage::English
                };
                Task::none()
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                if let Some(size) = self.pending_resize.take() {
                    return iced::window::resize(id, size);
                }
                Task::none()
            }
            Message::Menu(action) => self.handle_menu_action(action),
            Message::ScreenshotPath(path) => {
                if let Some(path) = path {
                    self.save_screenshot(path);
                }
                Task::none()
            }
            Message::Modal(action) => {
                self.handle_modal_action(action);
                Task::none()
            }
            Message::OpenModal(kind) => {
                if kind == ModalKind::Settings {
                    self.modal.baud_rate = self.default_connection.baud_rate;
                    self.modal.preview_fps = self.default_connection.preview_fps;
                    self.modal.auto_baud = self.default_connection.auto_baud;
                    self.modal.mouse_mode = self.default_connection.mouse_mode;
                    self.modal.relative_sensitivity = self.default_connection.relative_sensitivity;
                    self.modal.baud_text = self.default_connection.baud_rate.to_string();
                    self.modal.fps_text = self.default_connection.preview_fps.to_string();
                    self.modal.sensitivity_text =
                        self.default_connection.relative_sensitivity.to_string();
                    self.modal.scale_mode = self.scale_mode;
                }
                if kind == ModalKind::Connection {
                    self.modal.baud_rate = self.connection.baud_rate;
                    self.modal.preview_fps = self.connection.preview_fps;
                    self.modal.auto_baud = self.connection.auto_baud;
                    self.modal.mouse_mode = self.connection.mouse_mode;
                    self.modal.relative_sensitivity = self.connection.relative_sensitivity;
                    self.modal.baud_text = self.connection.baud_rate.to_string();
                    self.modal.fps_text = self.connection.preview_fps.to_string();
                    self.modal.sensitivity_text = self.connection.relative_sensitivity.to_string();
                }
                if kind == ModalKind::SaveProfile {
                    self.modal.confirm_overwrite = false;
                }
                self.modal.open(kind);
                Task::none()
            }
            Message::SelectVideo(label) => {
                if let Some(device) = self
                    .selection
                    .video_devices
                    .iter()
                    .find(|device| device.label == label)
                {
                    self.selection.selected_video_id = Some(device.id.clone());
                    self.selection.video_status = VideoProbeStatus::Checking;
                    self.preview.reset();
                    self.reset_preview_handle();
                    self.active_profile = None;
                }
                Task::none()
            }
            Message::SelectControl(label) => {
                if let Some(device) = self
                    .selection
                    .control_devices
                    .iter()
                    .find(|device| device.label == label)
                {
                    self.selection.selected_control_id = Some(device.id.clone());
                    let status = self.probe.probe_control(
                        &device.id,
                        self.connection.baud_rate,
                        PROBE_TIMEOUT,
                    );
                    self.selection
                        .record_control_probe(self.connection.baud_rate, status);
                    self.active_profile = None;
                }
                Task::none()
            }
            Message::RefreshDevices => {
                diag::log("RefreshDevices");
                let mut selection = self.selection.clone();
                match refresh_detection(
                    &mut selection,
                    self.probe.as_mut(),
                    self.connection.baud_rate,
                    PROBE_TIMEOUT,
                ) {
                    Ok(()) => {
                        self.selection = selection;
                        self.status_message = None;
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
                        match preview_refresh_action(&self.selection.video_status, selected_present)
                        {
                            PreviewRefreshAction::Skip => {}
                            PreviewRefreshAction::Reopen => {
                                self.preview.reset();
                                self.reset_preview_handle();
                                self.selection.video_status = VideoProbeStatus::Checking;
                            }
                            PreviewRefreshAction::KeepDisconnected => {
                                self.preview.reset();
                                self.reset_preview_handle();
                            }
                        }
                    }
                    Err(error) => {
                        self.status_message = Some(
                            t!("message.enumeration_failed", error = error.to_string()).to_string(),
                        );
                    }
                }
                Task::none()
            }
            Message::Connect => {
                self.preview.reset();
                self.reset_preview_handle();
                diag::log("connect begin");
                // 波特率解析收敛到共享层（#97）：当前波特率已被选中/刷新探测验证时
                // 直接使用、不重测；未验证且 auto_baud 开启时才用专用短超时兜底检测。
                if let Some(control_id) = self.selection.selected_control_id.clone() {
                    let previous = self.connection.baud_rate;
                    self.connection.baud_rate = resolve_connect_baud(
                        self.connection.auto_baud,
                        previous,
                        &self.selection.control_status,
                        &control_id,
                    );
                    if self.connection.baud_rate != previous {
                        self.status_message = Some(
                            t!("message.baud_selected", baud = self.connection.baud_rate)
                                .to_string(),
                        );
                    }
                }
                let Some(request) = self.connect_request() else {
                    return Task::none();
                };
                match self.controller.connect(request) {
                    Ok(()) => {
                        diag::log("connect ok");
                        self.status_message = None;
                        self.sync_status();
                        if let Some(name) = self.active_profile.clone() {
                            if let Err(error) = self.store.add_recent_profile(&name) {
                                self.status_message = Some(
                                    t!("profile.save_failed", error = error.to_string())
                                        .to_string(),
                                );
                            }
                        } else {
                            let snapshot = ipkvm_desktop::config::ManualSnapshot {
                                video_device: crate::profile::selected_device_ref(
                                    &self.selection.video_devices,
                                    self.selection.selected_video_id.as_deref(),
                                ),
                                control_device: crate::profile::selected_device_ref(
                                    &self.selection.control_devices,
                                    self.selection.selected_control_id.as_deref(),
                                ),
                                connection: self.connection.clone(),
                            };
                            if let Err(error) = self.store.set_last_manual(&snapshot) {
                                self.status_message = Some(
                                    t!("profile.save_failed", error = error.to_string())
                                        .to_string(),
                                );
                            }
                        }
                    }
                    Err(error) => {
                        diag::log(format!("connect failed: {error}"));
                        self.status_message = Some(
                            t!("message.connect_failed", error = error.to_string()).to_string(),
                        );
                    }
                }
                Task::none()
            }
            Message::Disconnect => {
                self.disconnect();
                Task::none()
            }
            Message::PreviewTick => {
                diag::log("PreviewTick");
                if self.controller.is_control_online() {
                    return Task::none();
                }
                if self.preview.tick(
                    &mut self.selection,
                    self.preview_factory.as_ref(),
                    self.connection.preview_fps,
                    Instant::now(),
                ) && let Some(source) = self.preview.source()
                    && let Some(frame) = source.latest_frame()
                {
                    // 同 seq 帧不重建 Handle：避免每 100ms 重复上传同一帧纹理。
                    if self.last_preview_seq != Some(frame.seq) {
                        self.preview_handle = Some(handle_from_frame(&frame));
                        self.last_preview_seq = Some(frame.seq);
                    }
                }
                Task::none()
            }
            Message::SetBaudRate(baud) => {
                self.connection.baud_rate = baud;
                self.active_profile = None;
                Task::none()
            }
            Message::SetAutoBaud(enabled) => {
                self.connection.auto_baud = enabled;
                self.active_profile = None;
                Task::none()
            }
            Message::SetPreviewFps(fps) => {
                self.connection.preview_fps = fps;
                self.active_profile = None;
                Task::none()
            }
            Message::SetMouseMode(mode) => {
                // 当前路径仅设置页/测试可达，不会在远程输入态生效；
                // 若未来远程输入态可达，需在此接 sync_cursor()。
                self.connection.mouse_mode = mode;
                self.active_profile = None;
                Task::none()
            }
            Message::LoadProfile(name) => {
                self.load_profile(&name);
                Task::none()
            }
            Message::ProfilePath(path) => {
                if let Some(path) = path {
                    self.load_profile_file(&path);
                }
                Task::none()
            }
            Message::Keyboard(event) => {
                self.handle_keyboard_event(event);
                Task::none()
            }
            Message::IcedEvent(event) => {
                self.handle_iced_event(event);
                Task::none()
            }
            Message::UiTick => {
                diag::ui_tick();
                let _ = self.controller.flush_pending();
                self.drain_notices();
                self.poll_relative();
                if self.remote_input
                    && self.connection.mouse_mode == MouseMode::Absolute
                    && let Some(cursor) = self.last_cursor
                {
                    self.send_absolute(cursor);
                }
                Task::none()
            }
            Message::FontsLoaded(result) => {
                if let Err(error) = result {
                    diag::log(format!("font load failed: {error:?}"));
                }
                Task::none()
            }
        }
    }

    fn handle_menu_action(&mut self, action: MenuAction) -> Task<Message> {
        match action {
            MenuAction::OpenModal(kind) => {
                if kind == ModalKind::Settings {
                    self.modal.baud_rate = self.default_connection.baud_rate;
                    self.modal.preview_fps = self.default_connection.preview_fps;
                    self.modal.auto_baud = self.default_connection.auto_baud;
                    self.modal.mouse_mode = self.default_connection.mouse_mode;
                    self.modal.relative_sensitivity = self.default_connection.relative_sensitivity;
                    self.modal.baud_text = self.default_connection.baud_rate.to_string();
                    self.modal.fps_text = self.default_connection.preview_fps.to_string();
                    self.modal.sensitivity_text =
                        self.default_connection.relative_sensitivity.to_string();
                    self.modal.scale_mode = self.scale_mode;
                }
                if kind == ModalKind::Connection {
                    self.modal.baud_rate = self.connection.baud_rate;
                    self.modal.preview_fps = self.connection.preview_fps;
                    self.modal.auto_baud = self.connection.auto_baud;
                    self.modal.mouse_mode = self.connection.mouse_mode;
                    self.modal.relative_sensitivity = self.connection.relative_sensitivity;
                    self.modal.baud_text = self.connection.baud_rate.to_string();
                    self.modal.fps_text = self.connection.preview_fps.to_string();
                    self.modal.sensitivity_text = self.connection.relative_sensitivity.to_string();
                }
                if kind == ModalKind::SaveProfile {
                    self.modal.confirm_overwrite = false;
                }
                self.modal.open(kind);
                Task::none()
            }
            MenuAction::SetLanguage(choice) => {
                let language = match choice {
                    crate::menu::LanguageChoice::System => AppLanguage::System,
                    crate::menu::LanguageChoice::Chinese => AppLanguage::Chinese,
                    crate::menu::LanguageChoice::English => AppLanguage::English,
                };
                language.apply();
                self.zh = rust_i18n::locale().starts_with("zh");
                self.language = choice.into();
                Task::none()
            }
            MenuAction::LoadRecent(name) => {
                self.load_profile(&name);
                Task::none()
            }
            MenuAction::SpecialKey(name) => {
                if let Some(key) = special_key_from_menu(&name) {
                    self.send_special(key);
                }
                Task::none()
            }
            MenuAction::Disconnect => {
                self.disconnect();
                Task::none()
            }
            MenuAction::Simple("paste") => {
                self.paste();
                Task::none()
            }
            MenuAction::Simple("copy_screenshot") => {
                self.copy_screenshot();
                Task::none()
            }
            MenuAction::Simple("save_screenshot") => {
                if self.latest_frame.is_none() {
                    self.status_message = Some(t!("message.no_frame_screenshot").to_string());
                    Task::none()
                } else {
                    Task::perform(
                        crate::dialog::choose_screenshot_path(),
                        Message::ScreenshotPath,
                    )
                }
            }
            MenuAction::Simple("load_profile") => Task::perform(
                crate::dialog::choose_profile_path(self.store.profiles_dir()),
                Message::ProfilePath,
            ),
            MenuAction::Simple("exit") => {
                if let Some(id) = self.window_id {
                    iced::window::close(id)
                } else {
                    Task::none()
                }
            }
            MenuAction::Simple("project_home") => {
                crate::platform::open_url(PROJECT_URL);
                Task::none()
            }
            MenuAction::Simple("release_all") => {
                let _ = self.controller.release_all();
                Task::none()
            }
            MenuAction::Simple(_) => Task::none(),
        }
    }

    fn handle_modal_action(&mut self, action: ModalAction) {
        match action {
            ModalAction::Close => self.modal.close(),
            ModalAction::SaveNameChanged(name) => self.modal.save_name = name,
            ModalAction::Save => {
                let name = self.modal.save_name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                if self.store.profile_exists(&name) && !self.modal.confirm_overwrite {
                    self.modal.confirm_overwrite = true;
                } else {
                    self.save_profile();
                }
            }
            ModalAction::CancelOverwrite => self.modal.confirm_overwrite = false,
            ModalAction::RestoreDefaults => {
                self.connection = self.default_connection.clone();
                self.active_profile = None;
                self.modal.baud_text = self.connection.baud_rate.to_string();
                self.modal.fps_text = self.connection.preview_fps.to_string();
                self.modal.sensitivity_text = self.connection.relative_sensitivity.to_string();
            }
            ModalAction::SetBaudRate(baud) => {
                self.apply_modal_connection_field(|c| c.baud_rate = baud);
                self.modal.baud_text = baud.to_string();
            }
            ModalAction::SetPreviewFps(fps) => {
                self.apply_modal_connection_field(|c| c.preview_fps = fps);
                self.modal.fps_text = fps.to_string();
            }
            ModalAction::SetAutoBaud(enabled) => {
                self.apply_modal_connection_field(|c| c.auto_baud = enabled);
            }
            ModalAction::SetMouseMode(mode) => {
                self.apply_modal_connection_field(|c| c.mouse_mode = mode);
            }
            ModalAction::SetRelativeSensitivity(value) => {
                self.apply_modal_connection_field(|c| c.relative_sensitivity = value);
                self.modal.sensitivity_text = value.to_string();
            }
            ModalAction::BaudRateTextChanged(text) => {
                self.modal.baud_text = text.clone();
                if let Ok(baud) = text.trim().parse::<u32>() {
                    self.apply_modal_connection_field(|c| {
                        c.baud_rate = baud.clamp(1200, 115_200);
                    });
                }
            }
            ModalAction::PreviewFpsTextChanged(text) => {
                self.modal.fps_text = text.clone();
                if let Ok(fps) = text.trim().parse::<u64>() {
                    self.apply_modal_connection_field(|c| {
                        c.preview_fps = fps.clamp(1, 60);
                    });
                }
            }
            ModalAction::RelativeSensitivityTextChanged(text) => {
                self.modal.sensitivity_text = text.clone();
                if let Ok(value) = text.trim().parse::<f32>() {
                    self.apply_modal_connection_field(|c| {
                        c.relative_sensitivity = value.clamp(0.1, 5.0);
                    });
                }
            }
            ModalAction::SetScaleMode(mode) => self.scale_mode = mode,
            ModalAction::Noop => {}
        }
    }

    /// 连接参数编辑路由：设置模态写默认值，连接设置模态写连接副本并标记手动连接。
    fn apply_modal_connection_field(&mut self, f: impl FnOnce(&mut ConnectionSettings)) {
        if self.modal.open == Some(ModalKind::Settings) {
            f(&mut self.default_connection);
        } else {
            f(&mut self.connection);
            self.active_profile = None;
        }
    }

    fn load_profile(&mut self, name: &str) {
        match self.store.load_profile(name) {
            Ok(profile) => self.apply_profile(profile),
            Err(error) => {
                self.status_message =
                    Some(t!("profile.load_failed", error = error.to_string()).to_string());
            }
        }
    }

    fn load_profile_file(&mut self, path: &std::path::Path) {
        match self.store.load_profile_file(path) {
            Ok(profile) => self.apply_profile(profile),
            Err(error) => {
                self.status_message =
                    Some(t!("profile.load_failed", error = error.to_string()).to_string());
            }
        }
    }

    fn apply_profile(&mut self, profile: ipkvm_desktop::config::Profile) {
        let missing = apply_profile_to_selection(&mut self.selection, &profile);
        self.connection = profile.connection;
        self.active_profile = Some(profile.name);
        self.preview.reset();
        self.reset_preview_handle();
        let mut notes = Vec::new();
        if missing.video {
            notes.push(t!("profile.device_missing").to_string());
        }
        if missing.control {
            notes.push(t!("profile.control_missing").to_string());
        }
        self.status_message = if notes.is_empty() {
            None
        } else {
            Some(notes.join("；"))
        };
    }

    fn save_profile(&mut self) {
        let name = self.modal.save_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let profile = build_profile(name.clone(), &self.selection, &self.connection);
        match self.store.save_profile(&profile) {
            Ok(()) => {
                self.status_message = Some(t!("profile.saved", name = name).to_string());
                self.modal.confirm_overwrite = false;
                self.modal.close();
            }
            Err(error) => {
                self.status_message =
                    Some(t!("profile.save_failed", error = error.to_string()).to_string());
            }
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

    fn handle_keyboard_event(&mut self, event: iced::keyboard::Event) {
        if !self.remote_input {
            return;
        }
        match event {
            iced::keyboard::Event::KeyPressed {
                physical_key,
                modifiers,
                repeat,
                ..
            } => {
                let iced::keyboard::key::Physical::Code(code) = physical_key else {
                    return;
                };
                if is_remote_exit_combo(code, modifiers, repeat) {
                    self.remote_input = false;
                    self.last_modifiers = iced::keyboard::Modifiers::empty();
                    self.sync_cursor();
                    return;
                }
                if is_mode_toggle_combo(code, modifiers, repeat) {
                    self.toggle_mouse_mode();
                    return;
                }
                if repeat {
                    return;
                }
                let Some(keysym) = physical_code_to_keysym(code) else {
                    self.status_message = Some(t!("message.unsupported_key").to_string());
                    return;
                };
                if let Err(error) = self.controller.send_key(true, keysym) {
                    self.status_message = Some(
                        t!("message.keyboard_send_failed", error = error.to_string()).to_string(),
                    );
                }
            }
            iced::keyboard::Event::KeyReleased { physical_key, .. } => {
                let iced::keyboard::key::Physical::Code(code) = physical_key else {
                    return;
                };
                if let Some(keysym) = physical_code_to_keysym(code)
                    && let Err(error) = self.controller.send_key(false, keysym)
                {
                    self.status_message = Some(
                        t!("message.keyboard_send_failed", error = error.to_string()).to_string(),
                    );
                }
            }
            iced::keyboard::Event::ModifiersChanged(modifiers) => {
                for action in modifier_diff(self.last_modifiers, modifiers) {
                    self.send_key_action(action);
                }
                self.last_modifiers = modifiers;
            }
        }
    }

    fn handle_iced_event(&mut self, event: iced::Event) {
        // 窗口事件不受在线状态影响：DPI 缩放即使离线也要记录。
        if let iced::Event::Window(iced::window::Event::Rescaled(factor)) = &event {
            self.scale_factor = (*factor).max(0.1);
            return;
        }
        if !self.controller.is_control_online() {
            return;
        }
        match event {
            iced::Event::Window(iced::window::Event::Unfocused) => {
                self.exit_remote_input();
            }
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                self.last_cursor = Some(position);
                if self.remote_input && self.connection.mouse_mode == MouseMode::Absolute {
                    self.send_absolute(position);
                }
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button)) => {
                self.pointer_mask |= mouse_button_bit(button);
                if self.pointer_inside_video() {
                    if !self.remote_input {
                        self.enter_remote_input();
                    }
                } else if self.remote_input {
                    self.exit_remote_input();
                }
                if self.remote_input
                    && self.connection.mouse_mode == MouseMode::Absolute
                    && let Some(cursor) = self.last_cursor
                {
                    self.send_absolute(cursor);
                }
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(button)) => {
                self.pointer_mask &= !mouse_button_bit(button);
                if self.remote_input
                    && self.connection.mouse_mode == MouseMode::Absolute
                    && let Some(cursor) = self.last_cursor
                {
                    self.send_absolute(cursor);
                }
            }
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) => {
                if self.remote_input {
                    self.relative_wheel = self.relative_wheel.saturating_add(wheel_steps(delta));
                    if self.connection.mouse_mode == MouseMode::Absolute
                        && let Some(cursor) = self.last_cursor
                    {
                        self.send_absolute(cursor);
                    }
                }
            }
            _ => {}
        }
    }

    fn pointer_inside_video(&self) -> bool {
        match (*self.video_bounds.borrow(), self.last_cursor) {
            // bounds 尚未记录（首帧前）或无光标位置：兼容进入，避免既有测试/首帧行为变化。
            (None, _) => true,
            (Some(_), None) => true,
            (Some(rect), Some(p)) => {
                p.x >= rect.x
                    && p.x <= rect.x + rect.width
                    && p.y >= rect.y
                    && p.y <= rect.y + rect.height
            }
        }
    }

    fn enter_remote_input(&mut self) {
        self.remote_input = true;
        self.sync_cursor();
    }

    fn exit_remote_input(&mut self) {
        if !self.remote_input {
            return;
        }
        self.remote_input = false;
        self.sync_cursor();
        self.pointer_mask = 0;
        self.last_pointer = None;
        self.last_relative_mask = 0;
        // 对齐 egui 退出语义：复位去重/限频状态，避免重进同位置点击被节流吞掉。
        self.last_pointer_sent = None;
        self.last_pointer_sent_at = None;
        self.relative_sampler.reset();
        let _ = self.controller.release_all();
    }

    /// 按当前模式/远程输入状态同步光标（相对+远程=隐藏裁剪；否则恢复）。
    fn sync_cursor(&mut self) {
        let grab = self.remote_input && self.connection.mouse_mode == MouseMode::Relative;
        self.cursor.set_visible(!grab);
        self.cursor.set_clipped(grab);
    }

    fn send_absolute(&mut self, cursor: iced::Point) {
        let Some(rect) = *self.video_bounds.borrow() else {
            return;
        };
        let Some(frame) = self.frame_size else {
            return;
        };
        let video_rect = crate::scale::frame_rect(
            crate::scale::Rect::from_min_size(rect.x, rect.y, rect.width, rect.height),
            frame,
            self.scale_mode,
        );
        let Some((x, y)) = crate::scale::map_pointer((cursor.x, cursor.y), video_rect, frame)
        else {
            return;
        };
        self.last_pointer = Some((x, y));
        let now = Instant::now();
        let mask_changed = self
            .last_pointer_sent
            .is_some_and(|(last_mask, _, _)| last_mask != self.pointer_mask);
        if (mask_changed
            || crate::input::throttle_elapsed(now, self.last_pointer_sent_at, POINTER_MIN_INTERVAL))
            && crate::input::pointer_changed((self.pointer_mask, x, y), self.last_pointer_sent)
        {
            let session_frame = ipkvm_desktop::FrameSize {
                width: frame.width,
                height: frame.height,
            };
            if let Err(error) = self
                .controller
                .send_pointer(self.pointer_mask, x, y, session_frame)
            {
                self.status_message =
                    Some(t!("message.pointer_send_failed", error = error.to_string()).to_string());
            }
            self.last_pointer_sent = Some((self.pointer_mask, x, y));
            self.last_pointer_sent_at = Some(now);
        }
        let wheel = self.relative_wheel;
        if wheel != 0 {
            if let Err(error) =
                self.controller
                    .send_pointer_relative(self.pointer_mask, 0, 0, wheel)
            {
                self.status_message =
                    Some(t!("message.pointer_send_failed", error = error.to_string()).to_string());
            }
            self.relative_wheel = 0;
        }
    }

    /// 视频比：目标帧像素 / 视频区逻辑点；bounds 或帧未知时回退 (1,1)。
    fn video_ratio(&self) -> (f32, f32) {
        let Some(rect) = *self.video_bounds.borrow() else {
            return (1.0, 1.0);
        };
        let Some(frame) = self.frame_size else {
            return (1.0, 1.0);
        };
        let rendered = crate::scale::frame_rect(
            crate::scale::Rect::from_min_size(rect.x, rect.y, rect.width, rect.height),
            frame,
            self.scale_mode,
        );
        if rendered.w <= 0.0 || rendered.h <= 0.0 {
            return (1.0, 1.0);
        }
        (
            frame.width as f32 / rendered.w,
            frame.height as f32 / rendered.h,
        )
    }

    fn send_key_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::Down(keysym) => {
                let _ = self.controller.send_key(true, keysym);
            }
            KeyAction::Up(keysym) => {
                let _ = self.controller.send_key(false, keysym);
            }
        }
    }

    fn send_special(&mut self, key: SpecialKey) {
        for action in special_key_sequence(key) {
            self.send_key_action(action);
        }
    }

    fn drain_notices(&mut self) {
        for notice in self.controller.drain_notices() {
            match notice {
                RfbInputNotice::TextTyped { .. } | RfbInputNotice::TextInputFailed { .. } => {
                    self.paste_busy = false;
                }
                _ => {}
            }
        }
    }

    fn poll_relative(&mut self) {
        if !self.remote_input
            || !self.controller.is_control_online()
            || self.connection.mouse_mode != MouseMode::Relative
        {
            return;
        }
        if self.relative_rx.is_none() {
            match self.relative_factory.create() {
                Ok(mut source) => match source.receiver() {
                    Ok(rx) => {
                        self.relative_source = Some(source);
                        self.relative_rx = Some(rx);
                    }
                    Err(error) => {
                        self.status_message = Some(format!("relative capture: {error}"));
                    }
                },
                Err(error) => {
                    self.status_message = Some(format!("relative capture: {error}"));
                }
            }
        }
        let Some(rx) = &self.relative_rx else {
            return;
        };
        let mut acc = (0.0f32, 0.0f32);
        while let Ok((dx, dy)) = rx.try_recv() {
            acc.0 += f32::from(dx);
            acc.1 += f32::from(dy);
        }
        let now = Instant::now();
        let sensitivity = self.connection.relative_sensitivity;
        let ratio = self.video_ratio();
        let (sx, sy) =
            crate::input::scale_relative_delta(acc.0, acc.1, self.scale_factor, sensitivity, ratio);
        let mask_changed = self.pointer_mask != self.last_relative_mask;
        let sampled = self.relative_sampler.feed(sx, sy, now);
        let (dx, dy) = sampled.unwrap_or((0, 0));
        let wheel = self.relative_wheel;
        if dx != 0 || dy != 0 || wheel != 0 || mask_changed {
            if let Err(error) =
                self.controller
                    .send_pointer_relative(self.pointer_mask, dx, dy, wheel)
            {
                self.status_message =
                    Some(t!("message.pointer_send_failed", error = error.to_string()).to_string());
            }
            self.relative_wheel = 0;
            self.last_relative_mask = self.pointer_mask;
        }
    }

    fn toggle_mouse_mode(&mut self) {
        let next = match self.connection.mouse_mode {
            MouseMode::Absolute => MouseMode::Relative,
            MouseMode::Relative => MouseMode::Absolute,
        };
        if let Ok(()) = self.controller.set_mouse_mode(next) {
            self.connection.mouse_mode = next;
            if next != MouseMode::Relative {
                self.stop_relative_source();
            }
            self.sync_cursor();
        }
    }

    fn stop_relative_source(&mut self) {
        if let Some(mut source) = self.relative_source.take() {
            source.stop();
        }
        self.relative_rx = None;
    }

    fn disconnect(&mut self) {
        diag::log("disconnect");
        let _ = self.controller.stop();
        self.sync_status();
        // 保留已探测状态（对齐 egui stop_session）：Connect 立即恢复可点，
        // 预览源保持常驻，由 PreviewTick 继续出帧。
        self.latest_frame = None;
        self.frame_size = None;
        self.remote_input = false;
        self.last_pointer = None;
        self.last_relative_mask = 0;
        self.sync_cursor();
        self.stop_relative_source();
    }

    fn paste(&mut self) {
        match crate::clipboard::read_clipboard_text(self.clipboard.as_ref()) {
            Ok(text) if !text.is_empty() => {
                if self.controller.paste_text(text).is_ok() {
                    self.paste_busy = true;
                }
            }
            Ok(_) => self.status_message = Some(t!("message.clipboard_empty").to_string()),
            Err(error) => {
                self.status_message =
                    Some(t!("message.clipboard_read_failed", error = error).to_string());
            }
        }
    }

    fn copy_screenshot(&mut self) {
        let Some(frame) = self.latest_frame.clone() else {
            self.status_message = Some(t!("message.no_frame_screenshot").to_string());
            return;
        };
        match ipkvm_desktop::frame::bgra_to_rgba(&frame) {
            Ok(rgba) => match ipkvm_desktop::clipboard::ClipboardService::copy_image(&rgba) {
                Ok(()) => self.status_message = Some(t!("message.screenshot_copied").to_string()),
                Err(error) => {
                    self.status_message = Some(
                        t!("message.screenshot_copy_failed", error = error.to_string()).to_string(),
                    )
                }
            },
            Err(error) => {
                self.status_message =
                    Some(t!("message.screenshot_copy_failed", error = error).to_string())
            }
        }
    }

    fn save_screenshot(&mut self, path: std::path::PathBuf) {
        let Some(frame) = self.latest_frame.clone() else {
            self.status_message = Some(t!("message.no_frame_screenshot").to_string());
            return;
        };
        match ipkvm_desktop::frame::bgra_to_rgba(&frame) {
            Ok(rgba) => match ipkvm_desktop::clipboard::save_jpeg(&path, &rgba) {
                Ok(()) => {
                    self.status_message = Some(
                        t!(
                            "message.screenshot_saved",
                            path = path.display().to_string()
                        )
                        .to_string(),
                    )
                }
                Err(error) => {
                    self.status_message = Some(
                        t!("message.screenshot_save_failed", error = error.to_string()).to_string(),
                    )
                }
            },
            Err(error) => {
                self.status_message =
                    Some(t!("message.screenshot_save_failed", error = error).to_string())
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let window_events = iced::window::open_events().map(Message::WindowOpened);
        let preview_timer =
            iced::time::every(Duration::from_millis(100)).map(|_| Message::PreviewTick);
        let keyboard = iced::keyboard::listen().map(Message::Keyboard);
        let events = iced::event::listen().map(Message::IcedEvent);
        // #89：UiTick 只承担 flush_pending/drain_notices，16ms 无条件重绘会
        // 放大视频闪烁与 CPU 占用；节流到 33ms（约 30Hz）语义不变。
        let ui_tick = iced::time::every(UI_TICK_INTERVAL).map(|_| Message::UiTick);
        if !self.subscribed {
            return Subscription::batch([window_events, preview_timer, keyboard, events, ui_tick]);
        }
        let frames = self
            .controller
            .subscribe_frames()
            .map(|receiver| {
                frame_subscription(0, receiver).map(|update| match update {
                    FrameUpdate::Frame(frame) => Message::FrameReady((*frame).clone()),
                    FrameUpdate::Closed => Message::FrameClosed,
                })
            })
            .unwrap_or_else(Subscription::none);
        Subscription::batch([
            frames,
            window_events,
            preview_timer,
            keyboard,
            events,
            ui_tick,
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        use iced::widget::{column, stack};
        let page = column![self.menu_view(), self.main_view(), self.status_line()];
        let page: Element<'_, Message> = page.into();
        match self.modal.view() {
            Some(modal) => stack![page, crate::modal::overlay(modal).map(Message::Modal)].into(),
            None => page,
        }
    }

    fn menu_view(&self) -> Element<'_, Message> {
        let recent: Vec<String> = self.store.recent_profiles();
        let recent_refs: Vec<&str> = recent.iter().map(String::as_str).collect();
        menu_bar(
            &recent_refs,
            self.paste_busy,
            self.language,
            self.controller.is_control_online(),
            self.latest_frame.is_some(),
        )
        .map(Message::Menu)
    }

    fn main_view(&self) -> Element<'_, Message> {
        if self.controller.is_control_online() {
            self.video_view()
        } else {
            self.connection_view()
        }
    }

    fn fit_image(
        handle: iced::widget::image::Handle,
    ) -> PreloadedImage<iced::widget::image::Handle> {
        PreloadedImage::new(handle)
            .content_fit(iced::ContentFit::Contain)
            .width(Length::Fill)
            .height(Length::Fill)
    }

    fn video_view(&self) -> Element<'_, Message> {
        use iced::widget::{column, container, text};
        let video: Element<'_, Message> = match self.handle.as_ref() {
            Some(handle) => Self::fit_image(handle.clone()).into(),
            None => text(t!("preview.no_signal")).size(28).into(),
        };
        let video_area = container(video)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(self.letterbox_color.into()),
                border: Border::default()
                    .width(1.0)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.20)),
                ..Default::default()
            });
        let recorded =
            crate::video_area::BoundsRecorder::new(self.video_bounds.clone(), video_area.into());
        column![recorded].into()
    }

    fn connection_view(&self) -> Element<'_, Message> {
        use iced::widget::{PickList, button, column, container, row, space, text};
        let video_pick = PickList::new(
            self.video_labels(),
            self.selected_video_label(),
            Message::SelectVideo,
        )
        .placeholder(t!("common.not_selected"));
        let control_pick = PickList::new(
            self.control_labels(),
            self.selected_control_label(),
            Message::SelectControl,
        )
        .placeholder(t!("common.not_selected"));
        let profile_pick = PickList::new(
            self.store.list_profiles(),
            self.active_profile.clone(),
            Message::LoadProfile,
        )
        .width(Length::Fixed(240.0))
        .placeholder(t!("profile.no_recent"));
        let connect = button(text(t!("device.connect")))
            .on_press_maybe(self.selection.can_connect().then_some(Message::Connect))
            .style(iced::widget::button::primary)
            .width(Length::Fixed(140.0))
            .height(Length::Fixed(36.0));
        let refresh = button(text(t!("device.refresh")))
            .on_press(Message::RefreshDevices)
            .width(Length::Fixed(140.0))
            .height(Length::Fixed(36.0));
        let save_profile =
            button(text(t!("profile.save"))).on_press(Message::OpenModal(ModalKind::SaveProfile));
        let connection_settings = button(text(t!("modal.connection_title")))
            .on_press(Message::OpenModal(ModalKind::Connection));
        let preview: Element<'_, Message> = match &self.preview_handle {
            Some(handle) => Self::fit_image(handle.clone()).into(),
            None => text(self.preview_placeholder()).into(),
        };
        let preview_area = container(preview)
            .width(Length::Fixed(320.0))
            .height(Length::Fixed(180.0))
            .style(|_theme| container::Style {
                background: Some(Color::from_rgb(0.91, 0.91, 0.91).into()),
                ..Default::default()
            });
        let left_pane = column![
            text(t!("device.video")),
            video_pick,
            self.video_status_text(),
            text(t!("device.control")),
            control_pick,
            self.control_status_text(),
            row![refresh, connect].spacing(8),
            self.status_message_view(),
        ]
        .spacing(8);
        let left = container(left_pane).width(Length::Fixed(380.0));
        let divider = container(space())
            .width(Length::Fixed(1.0))
            .height(Length::Fill)
            .style(|theme: &iced::Theme| container::Style {
                background: Some(crate::theme::border_color(theme.palette()).into()),
                ..Default::default()
            });
        let profile_row = row![
            text(t!("device.title")).size(18),
            profile_pick,
            save_profile,
            connection_settings,
        ]
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center);
        let content = column![profile_row, row![left, divider, preview_area].spacing(12),]
            .spacing(8)
            .padding(12);
        container(content)
            .width(Length::Fill)
            .padding(16)
            .style(|theme: &iced::Theme| container::Style {
                background: Some(crate::theme::surface(theme.palette()).into()),
                border: Border::default()
                    .rounded(10)
                    .width(1.0)
                    .color(crate::theme::border_color(theme.palette())),
                ..Default::default()
            })
            .into()
    }

    fn status_line(&self) -> Element<'_, Message> {
        use iced::widget::{container, text};
        let control = if self.controller.is_control_online() {
            self.control_status_value()
        } else {
            t!("status.offline").to_string()
        };
        let keyboard = if self.paste_busy {
            t!("status.pasting").to_string()
        } else if self.remote_input {
            t!("status.remote_input").to_string()
        } else {
            t!("status.ready").to_string()
        };
        let pointer = if self.connection.mouse_mode == MouseMode::Relative && self.remote_input {
            t!("status.relative_mode").to_string()
        } else if let Some((x, y)) = self.last_pointer {
            format!("({x}, {y})")
        } else {
            t!("status.ready").to_string()
        };
        let video = match self.frame_size {
            Some(size) => format!("{}×{}", size.width, size.height),
            None => t!("status.video_no_signal").to_string(),
        };
        let mut fields = iced::widget::Row::new()
            .spacing(16)
            .push(text(t!("status.control_device", value = control)).font(crate::fonts::ui_font()))
            .push(text(t!("status.keyboard", value = keyboard)).font(crate::fonts::ui_font()))
            .push(text(t!("status.pointer", value = pointer)).font(crate::fonts::ui_font()))
            .push(text(t!("status.video", value = video)).font(crate::fonts::ui_font()));
        if let Some(message) = &self.status_message {
            fields = fields
                .push(text(t!("status.message", message = message)).font(crate::fonts::ui_font()));
        }
        container(fields)
            .width(Length::Fill)
            .padding(6)
            .style(|theme: &iced::Theme| container::Style {
                background: Some(crate::theme::surface(theme.palette()).into()),
                ..Default::default()
            })
            .into()
    }

    fn control_status_value(&self) -> String {
        match &self.selection.control_status {
            ControlProbeStatus::NotSelected => t!("control_status.not_selected").to_string(),
            ControlProbeStatus::Checking => t!("control_status.checking").to_string(),
            ControlProbeStatus::Ready(_) => t!(
                "control_status.ready",
                port = self.selection.selected_control_id.as_deref().unwrap_or("?")
            )
            .to_string(),
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

    fn status_message_view(&self) -> Element<'_, Message> {
        use iced::widget::{container, text};
        match &self.status_message {
            Some(message) => container(text(message.clone())).padding(4).into(),
            None => container(text("")).into(),
        }
    }

    fn preview_placeholder(&self) -> String {
        match &self.selection.video_status {
            VideoProbeStatus::NoSignal => t!("preview.no_signal").to_string(),
            VideoProbeStatus::OpenFailed(_) => t!("preview.open_failed").to_string(),
            _ => t!("device.no_preview").to_string(),
        }
    }

    fn video_status_text(&self) -> Element<'_, Message> {
        use iced::widget::text;
        let label = match &self.selection.video_status {
            VideoProbeStatus::NotSelected => t!("video_status.not_selected"),
            VideoProbeStatus::Checking => t!("video_status.checking"),
            VideoProbeStatus::Ready(info) => t!(
                "video_status.ready",
                width = info.width,
                height = info.height,
                label = info.label
            ),
            VideoProbeStatus::NoSignal => t!("video_status.no_signal"),
            VideoProbeStatus::OpenFailed(error) => {
                t!("video_status.open_failed", error = error)
            }
            VideoProbeStatus::Disconnected => t!("video_status.disconnected"),
        };
        text(label).font(crate::fonts::ui_font()).into()
    }

    fn control_status_text(&self) -> Element<'_, Message> {
        use iced::widget::text;
        let label = match &self.selection.control_status {
            ControlProbeStatus::NotSelected => t!("control_status_label.not_selected"),
            ControlProbeStatus::Checking => t!("control_status_label.checking"),
            ControlProbeStatus::Ready(_) => t!(
                "control_status_label.ready",
                port = self.selection.selected_control_id.as_deref().unwrap_or("?")
            ),
            ControlProbeStatus::NotCh9329(reason) => {
                t!("control_status_label.not_ch9329", reason = reason)
            }
            ControlProbeStatus::NoResponse => t!("control_status_label.no_response"),
            ControlProbeStatus::OpenFailed(error) => {
                t!("control_status_label.open_failed", error = error)
            }
            ControlProbeStatus::Disconnected => t!("control_status_label.disconnected"),
        };
        text(label).font(crate::fonts::ui_font()).into()
    }

    fn video_labels(&self) -> Vec<String> {
        self.selection
            .video_devices
            .iter()
            .map(|device| device.label.clone())
            .collect()
    }

    fn control_labels(&self) -> Vec<String> {
        self.selection
            .control_devices
            .iter()
            .map(|device| device.label.clone())
            .collect()
    }

    fn selected_video_label(&self) -> Option<String> {
        let id = self.selection.selected_video_id.as_deref()?;
        Some(
            self.selection
                .video_devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.label.clone())
                .unwrap_or_default(),
        )
    }

    fn selected_control_label(&self) -> Option<String> {
        let id = self.selection.selected_control_id.as_deref()?;
        Some(
            self.selection
                .control_devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.label.clone())
                .unwrap_or_default(),
        )
    }

    pub fn sync_status(&mut self) {
        let online = self.controller.is_control_online();
        // 诊断：整页切换抖动取证（main_view 按在线状态切视频页/连接页）。
        if self.last_diag_online != Some(online) {
            diag::log(format!("online_flip online={online}"));
            self.last_diag_online = Some(online);
        }
        self.status = derive_status(online, self.controller.input_offline_reason());
    }

    pub fn subscribed(&self) -> bool {
        self.subscribed
    }

    pub fn status(&self) -> &ConnectionStatus {
        &self.status
    }

    pub fn handle(&self) -> Option<&Handle> {
        self.handle.as_ref()
    }

    pub fn frame_size(&self) -> Option<FrameSize> {
        self.frame_size
    }

    pub fn scale_mode(&self) -> ScaleMode {
        self.scale_mode
    }

    pub fn letterbox_color(&self) -> Color {
        self.letterbox_color
    }

    pub fn frame_source(&self) -> Option<&Arc<MockFrameSource>> {
        self.frame_source.as_ref()
    }

    pub fn recording_sink(&self) -> Option<&RecordingSink> {
        self.recording.as_ref()
    }

    pub fn remote_input(&self) -> bool {
        self.remote_input
    }

    pub fn paste_busy(&self) -> bool {
        self.paste_busy
    }

    /// 应用主题（iced builder 的 theme 回调）。
    pub fn theme(&self) -> iced::Theme {
        crate::theme::app_theme(self.dark)
    }
}

/// mock 预览源：open 即返回已有一帧 64×48 的 MockFrameSource。
#[derive(Default)]
struct MockPreviewFactory;

impl PreviewSourceFactory for MockPreviewFactory {
    fn open(&self, _device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
        let mock = Arc::new(MockFrameSource::new());
        let mut data = vec![0u8; 64 * 48 * 4];
        data[0] = 10;
        data[1] = 20;
        data[2] = 30;
        data[3] = 255;
        let frame = VideoFrame::new(
            1,
            ipkvm_video::MonotonicTimestamp::from_nanos(1),
            64,
            48,
            256,
            ipkvm_video::PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        );
        mock.publish_frame(Arc::new(frame));
        Ok(mock as Arc<dyn FrameSource>)
    }
}

/// ResizeWindowToVideo 模式的期望窗口尺寸；其余模式返回 None。
pub fn desired_window_size(frame: Option<FrameSize>, mode: ScaleMode) -> Option<Size> {
    match (mode, frame) {
        (ScaleMode::ResizeWindowToVideo, Some(f)) => {
            Some(Size::new(f.width as f32, f.height as f32))
        }
        _ => None,
    }
}

/// 鼠标按钮 → RFB button mask（Primary=1、Secondary=2、Middle=4）。
fn mouse_button_bit(button: iced::mouse::Button) -> u8 {
    match button {
        iced::mouse::Button::Left => 0b001,
        iced::mouse::Button::Right => 0b010,
        iced::mouse::Button::Middle => 0b100,
        _ => 0,
    }
}

fn connect_request() -> ConnectRequest {
    ConnectRequest {
        video_device_id: "mock".into(),
        control_device_id: "mock".into(),
        baud_rate: 9_600,
        mouse_mode: MouseMode::Absolute,
        preview_fps: 30,
    }
}

/// 启动生产 iced 应用（bin 入口调用；测试不启动真实窗口）。
pub fn run() -> iced::Result {
    let _ = crate::locale::apply_system_locale();
    diag::init();
    diag::log(format!(
        "startup args: {:?}",
        std::env::args().collect::<Vec<_>>()
    ));
    iced::application(App::production, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .title(WINDOW_TITLE)
        .window_size(crate::fit_initial_size(
            DEFAULT_WINDOW_SIZE,
            crate::desktop_work_area(),
        ))
        .default_font(crate::fonts::ui_font())
        .run()
}

/// 记录型光标控制器：断言 set_visible/set_clipped 调用序列（Task 7b 测试用）。
/// 定义在 app.rs 模块级（而非 tests 模块内），因为 App 结构体的测试字段
/// `cursor_records` 需要引用该类型。
#[cfg(test)]
#[derive(Default)]
struct RecordingCursorController {
    visible: std::sync::Mutex<Vec<bool>>,
    clipped: std::sync::Mutex<Vec<bool>>,
}

#[cfg(test)]
impl crate::platform::cursor::CursorController for RecordingCursorController {
    fn set_visible(&self, visible: bool) {
        self.visible.lock().unwrap().push(visible);
    }

    fn set_clipped(&self, clipped: bool) {
        self.clipped.lock().unwrap().push(clipped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_video::{MonotonicTimestamp, PixelFormat};

    fn make_bgra_frame(seq: u64, w: u32, h: u32) -> VideoFrame {
        let mut data = vec![0u8; (w * h * 4) as usize];
        data[0] = 10;
        data[1] = 20;
        data[2] = 30;
        data[3] = 255;
        VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            w,
            h,
            w * 4,
            PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        )
    }

    #[test]
    fn frame_ready_stores_handle_and_frame_size_and_status_connected() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::FrameReady(make_bgra_frame(1, 320, 240)));
        assert!(app.handle().is_some(), "FrameReady 后 Handle 必须存 state");
        assert_eq!(
            app.frame_size(),
            Some(FrameSize {
                width: 320,
                height: 240
            })
        );
        assert_eq!(app.status(), &ConnectionStatus::Connected);
    }

    #[test]
    fn frame_closed_stops_subscription() {
        let (mut app, _) = MockApp::new_mock();
        assert!(app.subscribed());
        let _ = app.update(Message::FrameClosed);
        assert!(!app.subscribed(), "FrameClosed 后订阅必须停");
    }

    /// 枚举失败测试后端：列表接口直接报错（复刻真实设备不可枚举场景）。
    #[derive(Default)]
    struct FailingProbeBackend;

    impl ipkvm_desktop::probe::ProbeBackend for FailingProbeBackend {
        fn list_video_devices(
            &mut self,
        ) -> Result<Vec<crate::connect::DeviceOption>, ipkvm_desktop::probe::ProbeError> {
            Err(ipkvm_desktop::probe::ProbeError::VideoList("boom".into()))
        }

        fn list_control_devices(
            &mut self,
        ) -> Result<Vec<crate::connect::DeviceOption>, ipkvm_desktop::probe::ProbeError> {
            Err(ipkvm_desktop::probe::ProbeError::ControlList("boom".into()))
        }

        fn probe_control(
            &mut self,
            _device_id: &str,
            _baud_rate: u32,
            _timeout: Duration,
        ) -> ControlProbeStatus {
            ControlProbeStatus::NoResponse
        }
    }

    #[test]
    fn startup_task_auto_enumerates_devices() {
        // 启动任务必须是 RefreshDevices；Task::none() 的 units 为 0，done 为 1。
        assert!(matches!(
            MockApp::startup_message(),
            Message::RefreshDevices
        ));
        let (_, task) = MockApp::new_mock();
        assert_eq!(
            task.units(),
            1,
            "启动 Task 必须携带消息，不能是 Task::none()"
        );

        // 消费启动消息后列表必须非空（FakeProbeBackend 返回 cam0/COM9）。
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::RefreshDevices);
        assert_eq!(app.selection.video_devices.len(), 1);
        assert_eq!(app.selection.control_devices.len(), 1);
    }

    #[test]
    fn production_startup_task_triggers_enumeration_and_loads_fonts() {
        let (_, task) = ProductionApp::production();
        assert!(
            task.units() >= 5,
            "生产 App 启动 Task 必须携带枚举消息 + 4 个 Poppins 字体加载（实际 units={}）",
            task.units()
        );
    }

    #[test]
    fn production_zh_follows_system_locale() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let (app, _) = ProductionApp::production();
        assert_eq!(
            app.zh,
            rust_i18n::locale().starts_with("zh"),
            "production() 必须按 apply_system_locale 初始化 zh"
        );
    }

    #[test]
    fn startup_enumeration_failure_reports_and_does_not_block() {
        let (mut app, _) = MockApp::new_mock();
        app.probe = Box::new(FailingProbeBackend);
        let _ = app.update(Message::RefreshDevices);
        assert!(
            app.selection.video_devices.is_empty(),
            "失败时不得替换旧列表"
        );
        assert!(
            app.status_message.is_some(),
            "枚举失败必须把原因显示到状态消息区"
        );
    }

    #[test]
    fn startup_prefills_last_manual_snapshot() {
        let (mut app, _) = MockApp::new_mock();
        let snapshot = ipkvm_desktop::config::ManualSnapshot {
            video_device: Some(ipkvm_desktop::config::DeviceRef {
                id: "cam0".into(),
                label: "Camera 0".into(),
            }),
            control_device: Some(ipkvm_desktop::config::DeviceRef {
                id: "COM9".into(),
                label: "CH9329 (COM9)".into(),
            }),
            connection: ipkvm_desktop::config::ConnectionSettings {
                baud_rate: 115200,
                ..Default::default()
            },
        };
        app.store.set_last_manual(&snapshot).expect("写入快照");
        app.prefill_last_manual();
        assert_eq!(app.selection.selected_video_id.as_deref(), Some("cam0"));
        assert_eq!(app.selection.selected_control_id.as_deref(), Some("COM9"));
        assert_eq!(app.connection.baud_rate, 115200);
        assert_eq!(app.selection.video_status, VideoProbeStatus::Checking);
        assert_eq!(app.selection.control_status, ControlProbeStatus::Checking);
    }

    #[test]
    fn preview_tick_same_seq_does_not_rebuild_handle() {
        let (mut app, _) = MockApp::new_mock();
        // new_mock 构造时已连接（M1 语义），先断开进入连接页流程，
        // 再模拟启动自动枚举，否则设备列表为空无法选择。
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        let first = app.preview_handle.clone().expect("预览必须出帧");
        let _ = app.update(Message::PreviewTick);
        let second = app.preview_handle.clone().expect("预览仍持有帧");
        assert_eq!(
            first.id(),
            second.id(),
            "同 seq 帧不得重建 Handle（否则每次 100ms 重复纹理上传 → 闪烁）"
        );
    }

    #[test]
    fn scale_mode_and_letterbox_transitions() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::SetScaleMode(ScaleMode::ActualSize));
        assert_eq!(app.scale_mode(), ScaleMode::ActualSize);
        let color = Color::from_rgb(0.1, 0.2, 0.3);
        let _ = app.update(Message::SetLetterboxColor(color));
        assert_eq!(app.letterbox_color(), color);
    }

    #[test]
    fn default_letterbox_is_light() {
        let (app, _) = MockApp::new_mock();
        assert_eq!(app.letterbox_color(), Color::from_rgb(0.91, 0.91, 0.91));
    }

    #[test]
    fn relative_mode_button_press_sends_without_motion() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Relative;
        *app.video_bounds.borrow_mut() = Some(iced::Rectangle::new(
            iced::Point::ORIGIN,
            iced::Size::new(320.0, 180.0),
        ));
        app.frame_size = Some(FrameSize {
            width: 320,
            height: 180,
        });
        move_cursor(&mut app, 160.0, 90.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input, "点击视频区必须进入远程输入");
        let _ = app.update(Message::UiTick);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink() {
                let events = sink.relative_events.lock().unwrap();
                if events.iter().any(|&event| event == (1, 0, 0, 0)) {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "按钮按下事件 (1,0,0,0) 未达 sink"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = app.update(Message::IcedEvent(iced::Event::Mouse(
            iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left),
        )));
        let _ = app.update(Message::UiTick);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink() {
                let events = sink.relative_events.lock().unwrap();
                if events.iter().any(|&event| event == (0, 0, 0, 0)) {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "按钮释放事件 (0,0,0,0) 未达 sink"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn desired_window_size_only_for_resize_mode() {
        let frame = Some(FrameSize {
            width: 1920,
            height: 1080,
        });
        assert_eq!(
            desired_window_size(frame, ScaleMode::ResizeWindowToVideo),
            Some(Size::new(1920.0, 1080.0))
        );
        assert_eq!(desired_window_size(frame, ScaleMode::FitWindow), None);
        assert_eq!(
            desired_window_size(None, ScaleMode::ResizeWindowToVideo),
            None
        );
    }

    #[test]
    fn stop_session_derives_disconnected_status() {
        let (mut app, _) = MockApp::new_mock();
        app.controller.stop().unwrap();
        app.sync_status();
        assert_eq!(app.status(), &ConnectionStatus::Disconnected);
    }

    #[test]
    fn select_video_then_preview_tick_reaches_ready() {
        let (mut app, _) = MockApp::new_mock();
        // new_mock 构造时已连接（M1 语义），先断开进入连接页流程。
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        assert!(matches!(
            app.selection.video_status,
            VideoProbeStatus::Ready(info) if info.width == 64 && info.height == 48
        ));
        assert!(
            app.preview_handle.is_some(),
            "预览 tick 后必须有预览 Handle"
        );
    }

    #[test]
    fn select_control_reaches_ready_and_can_connect() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        assert!(matches!(
            app.selection.control_status,
            ControlProbeStatus::Ready(_)
        ));
    }

    #[test]
    fn control_selection_records_verified_baud_and_baud_change_invalidates() {
        // #97：选中探测成功后，Ready 必须携带“已验证的波特率”。
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SetBaudRate(9_600));
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        assert_eq!(
            app.selection.control_status,
            ControlProbeStatus::Ready(crate::connect::ControlInfo {
                version: 0x31,
                usb_enumerated: true,
                baud: 9_600,
            })
        );

        // 改波特率后旧验证失配：连接前 resolve 会走兜底检测（共享层单测覆盖）。
        let _ = app.update(Message::SetBaudRate(115_200));
        assert!(!matches!(
            app.selection.control_status,
            ControlProbeStatus::Ready(info) if info.baud == app.connection.baud_rate
        ));
    }

    #[test]
    fn connect_then_disconnect_transitions() {
        let (mut app, _) = MockApp::new_mock();
        // 断开后从连接页重走 选设备→预览→连接→断开。
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::Connect);
        assert_eq!(app.status(), &ConnectionStatus::Connected);
        let _ = app.update(Message::Disconnect);
        assert_eq!(app.status(), &ConnectionStatus::Disconnected);
        assert!(
            app.selection.can_connect(),
            "断开后应保留探测状态，Connect 可点"
        );
    }

    #[test]
    fn menu_disconnect_returns_to_connection_page() {
        let (mut app, _) = MockApp::new_mock(); // 构造即在线 → 视频页
        assert_eq!(app.status(), &ConnectionStatus::Connected);
        // 建立 Ready 探测状态并准备一帧，再通过菜单断开（保留状态的断言前提）。
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::FrameReady(make_bgra_frame(1, 16, 9)));
        assert!(app.latest_frame.is_some());
        let _ = app.update(Message::Menu(MenuAction::Disconnect));
        assert_eq!(app.status(), &ConnectionStatus::Disconnected);
        assert!(
            app.selection.can_connect(),
            "断开后应保留探测状态，Connect 可点"
        );
        assert!(app.latest_frame.is_none(), "断开后截图应不可用");
        assert!(!app.remote_input, "断开后必须退出远程输入");
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(ui.find("Connect").is_ok(), "断开后必须回到连接页");
    }

    #[test]
    fn disconnect_keeps_probe_state_and_clears_frame() {
        let (mut app, _) = MockApp::new_mock();
        // 先建立 Ready 探测状态（new_mock 默认 NotSelected，无状态可保留）。
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::FrameReady(make_bgra_frame(1, 16, 9)));
        assert!(app.latest_frame.is_some());
        let _ = app.update(Message::Disconnect);
        assert_eq!(app.status(), &ConnectionStatus::Disconnected);
        assert!(app.selection.can_connect(), "探测状态必须保留");
        assert!(app.latest_frame.is_none(), "latest_frame 必须清空");
    }

    #[test]
    fn save_profile_flow_writes_store() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
        let _ = app.update(Message::Modal(ModalAction::SaveNameChanged(
            "办公室".into(),
        )));
        let _ = app.update(Message::Modal(ModalAction::Save));
        assert!(app.store.profile_exists("办公室"));
        assert!(app.modal.open.is_none(), "保存成功后模态必须关闭");
    }

    #[test]
    fn save_profile_overwrite_requires_confirm() {
        let (mut app, _) = MockApp::new_mock();
        // 先保存一个已有名字。
        let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
        let _ = app.update(Message::Modal(ModalAction::SaveNameChanged(
            "办公室".into(),
        )));
        let _ = app.update(Message::Modal(ModalAction::Save));
        assert!(app.store.profile_exists("办公室"));

        // 再次用同名保存：第一次 Save 只进入覆盖确认，不写盘、不关闭。
        let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
        let _ = app.update(Message::Modal(ModalAction::SaveNameChanged(
            "办公室".into(),
        )));
        let _ = app.update(Message::Modal(ModalAction::Save));
        assert!(app.modal.confirm_overwrite, "同名保存必须先进入覆盖确认");
        assert_eq!(
            app.modal.open,
            Some(ModalKind::SaveProfile),
            "确认前不得关闭模态"
        );

        // 第二次 Save 才真正覆盖并关闭，同时复位确认标志。
        let _ = app.update(Message::Modal(ModalAction::Save));
        assert!(
            app.store.profile_exists("办公室"),
            "确认覆盖后必须写入 profile"
        );
        assert!(!app.modal.confirm_overwrite, "覆盖确认后标志必须复位");
        assert!(app.modal.open.is_none(), "确认后模态必须关闭");
    }

    #[test]
    fn load_profile_applies_selection() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
        let _ = app.update(Message::Modal(ModalAction::SaveNameChanged(
            "办公室".into(),
        )));
        let _ = app.update(Message::Modal(ModalAction::Save));
        // 清空选择再加载。
        app.selection.selected_video_id = None;
        app.selection.selected_control_id = None;
        let _ = app.update(Message::LoadProfile("办公室".into()));
        assert_eq!(app.selection.selected_video_id.as_deref(), Some("cam0"));
        assert_eq!(app.selection.selected_control_id.as_deref(), Some("COM9"));
    }

    #[test]
    fn load_profile_file_applies_profile() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
        let _ = app.update(Message::Modal(ModalAction::SaveNameChanged(
            "办公室".into(),
        )));
        let _ = app.update(Message::Modal(ModalAction::Save));
        let path = app.store.profiles_dir().join("办公室.toml");
        assert!(path.exists(), "保存的 profile 文件必须存在");

        // 清空选择再通过文件路径加载。
        app.selection.selected_video_id = None;
        app.selection.selected_control_id = None;
        app.active_profile = None;
        let _ = app.update(Message::ProfilePath(Some(path)));
        assert_eq!(app.selection.selected_video_id.as_deref(), Some("cam0"));
        assert_eq!(app.selection.selected_control_id.as_deref(), Some("COM9"));
        assert_eq!(app.active_profile.as_deref(), Some("办公室"));

        // None（取消对话框）必须无副作用。
        let connection_before = app.connection.clone();
        let _ = app.update(Message::ProfilePath(None));
        assert_eq!(app.connection, connection_before);
        assert_eq!(app.active_profile.as_deref(), Some("办公室"));
    }

    #[test]
    fn menu_action_opens_and_closes_modal() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Menu(MenuAction::OpenModal(ModalKind::Settings)));
        assert_eq!(app.modal.open, Some(ModalKind::Settings));
        let _ = app.update(Message::Modal(ModalAction::Close));
        assert!(app.modal.open.is_none());
    }

    #[test]
    fn copy_screenshot_without_frame_reports_no_frame() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Menu(MenuAction::Simple("copy_screenshot")));
        assert_eq!(
            app.status_message,
            Some(t!("message.no_frame_screenshot").to_string())
        );
    }

    #[test]
    fn save_screenshot_writes_jpeg_from_latest_frame() {
        let (mut app, _) = MockApp::new_mock();
        let path = std::env::temp_dir().join(format!(
            "my-ipkvm-iced-screenshot-{}-{}.jpg",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = app.update(Message::FrameReady(make_bgra_frame(1, 16, 9)));
        let _ = app.update(Message::ScreenshotPath(Some(path.clone())));
        assert!(path.exists(), "截图文件必须写出");
        assert!(
            std::fs::metadata(&path).expect("截图文件 metadata").len() > 0,
            "截图文件不得为空"
        );
        let expected = t!(
            "message.screenshot_saved",
            path = path.display().to_string()
        );
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.contains(expected.as_ref())),
            "状态消息必须包含保存成功文案，实际: {:?}",
            app.status_message
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn exit_returns_window_close_task() {
        let (mut app, _) = MockApp::new_mock();
        app.window_id = Some(iced::window::Id::unique());
        let task = app.update(Message::Menu(MenuAction::Simple("exit")));
        assert_eq!(task.units(), 1, "exit 必须返回有效 window close task");
    }

    #[test]
    fn locale_switch_updates_zh_flag() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Menu(MenuAction::SetLanguage(
            crate::menu::LanguageChoice::Chinese,
        )));
        assert!(app.zh);
        let _ = app.update(Message::Menu(MenuAction::SetLanguage(
            crate::menu::LanguageChoice::English,
        )));
        assert!(!app.zh);
    }

    #[test]
    fn settings_fields_update_connection() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::SetBaudRate(115200));
        let _ = app.update(Message::SetAutoBaud(false));
        let _ = app.update(Message::SetPreviewFps(15));
        let _ = app.update(Message::SetMouseMode(MouseMode::Relative));
        assert_eq!(app.connection.baud_rate, 115200);
        assert!(!app.connection.auto_baud);
        assert_eq!(app.connection.preview_fps, 15);
        assert_eq!(app.connection.mouse_mode, MouseMode::Relative);
    }

    #[test]
    fn settings_modal_updates_default_connection() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::OpenModal(ModalKind::Settings));
        let connection_before = app.connection.clone();
        let _ = app.update(Message::Modal(ModalAction::SetBaudRate(9600)));
        let _ = app.update(Message::Modal(ModalAction::SetPreviewFps(15)));
        let _ = app.update(Message::Modal(ModalAction::SetAutoBaud(false)));
        let _ = app.update(Message::Modal(ModalAction::SetMouseMode(
            MouseMode::Absolute,
        )));
        let _ = app.update(Message::Modal(ModalAction::SetRelativeSensitivity(2.5)));
        assert_eq!(app.default_connection.baud_rate, 9600);
        assert_eq!(app.default_connection.preview_fps, 15);
        assert!(!app.default_connection.auto_baud);
        assert_eq!(app.default_connection.mouse_mode, MouseMode::Absolute);
        assert_eq!(app.default_connection.relative_sensitivity, 2.5);
        assert_eq!(
            app.connection, connection_before,
            "设置模态只能改默认值，不得影响当前连接"
        );
    }

    fn press_key(code: iced::keyboard::key::Code) -> Message {
        Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Unidentified,
            modified_key: iced::keyboard::Key::Unidentified,
            physical_key: iced::keyboard::key::Physical::Code(code),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }

    fn release_key(code: iced::keyboard::key::Code) -> Message {
        Message::Keyboard(iced::keyboard::Event::KeyReleased {
            key: iced::keyboard::Key::Unidentified,
            modified_key: iced::keyboard::Key::Unidentified,
            physical_key: iced::keyboard::key::Physical::Code(code),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
        })
    }

    fn click_video(app: &mut MockApp) {
        let _ = app.update(Message::IcedEvent(iced::Event::Mouse(
            iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left),
        )));
    }

    fn last_visible(app: &MockApp) -> Option<bool> {
        app.cursor_records.visible.lock().unwrap().last().copied()
    }

    fn last_clipped(app: &MockApp) -> Option<bool> {
        app.cursor_records.clipped.lock().unwrap().last().copied()
    }

    /// 发送 Ctrl+Alt+M（切换鼠标模式）。
    fn press_mode_toggle(app: &mut MockApp) {
        let mut modifiers = iced::keyboard::Modifiers::empty();
        modifiers.set(iced::keyboard::Modifiers::CTRL, true);
        modifiers.set(iced::keyboard::Modifiers::ALT, true);
        let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Unidentified,
            modified_key: iced::keyboard::Key::Unidentified,
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyM),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }));
    }

    fn enter_remote(app: &mut MockApp) {
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::Connect);
        click_video(app);
    }

    fn wait_sink(app: &MockApp, count: usize) -> Vec<(bool, u8)> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink() {
                let recorded = sink.key_events.lock().unwrap();
                if recorded.len() >= count {
                    return recorded.clone();
                }
                let len = recorded.len();
                drop(recorded);
                assert!(
                    std::time::Instant::now() < deadline,
                    "sink 事件未达 {count}（实际 {len}）"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sink 事件未达 {count}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// 持续触发 UiTick（模拟真实 16ms 定时器）直到 sink 事件足够。
    fn drain_to(app: &mut MockApp, count: usize) -> Vec<(bool, u8)> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let _ = app.update(Message::UiTick);
            if let Some(sink) = app.recording_sink() {
                let recorded = sink.key_events.lock().unwrap();
                if recorded.len() >= count {
                    return recorded.clone();
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sink 事件未达 {count}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn keyboard_press_release_reaches_sink_in_order() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
        let _ = app.update(release_key(iced::keyboard::key::Code::KeyA));
        let recorded = wait_sink(&app, 2);
        assert_eq!(recorded[0], (true, 0x04));
        assert_eq!(recorded[1], (false, 0x04));
    }

    #[test]
    fn five_hundred_mixed_keys_reach_sink_in_order() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        for i in 0..500 {
            let code = match i % 3 {
                0 => iced::keyboard::key::Code::KeyA,
                1 => iced::keyboard::key::Code::ArrowUp,
                _ => iced::keyboard::key::Code::F1,
            };
            let _ = app.update(press_key(code));
            let _ = app.update(release_key(code));
            let _ = app.update(Message::UiTick);
        }
        let recorded = drain_to(&mut app, 1000);
        for (i, (down, _usage)) in recorded.iter().enumerate() {
            assert_eq!(*down, i % 2 == 0, "第 {i} 个事件 down/up 顺序不符");
        }
    }

    #[test]
    fn first_key_is_not_swallowed() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
        let recorded = wait_sink(&app, 1);
        assert_eq!(recorded[0], (true, 0x04));
    }

    #[test]
    fn flush_tick_drains_burst_without_further_input() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        // 50 组按下/抬起（交替键，避免重复 down 被输入泵去重）。
        for i in 0..50 {
            let code = if i % 2 == 0 {
                iced::keyboard::key::Code::KeyA
            } else {
                iced::keyboard::key::Code::KeyB
            };
            let _ = app.update(press_key(code));
            let _ = app.update(release_key(code));
        }
        // 突发后无后续输入，仅靠持续 tick 补送（真实 16ms 定时器语义）。
        let recorded = drain_to(&mut app, 100);
        for (i, (down, _usage)) in recorded.iter().enumerate() {
            assert_eq!(*down, i % 2 == 0, "第 {i} 个事件 down/up 顺序不符");
        }
    }

    #[test]
    fn ctrl_alt_k_exits_remote_input_without_forwarding() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        let mut modifiers = iced::keyboard::Modifiers::empty();
        modifiers.set(iced::keyboard::Modifiers::CTRL, true);
        modifiers.set(iced::keyboard::Modifiers::ALT, true);
        let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Unidentified,
            modified_key: iced::keyboard::Key::Unidentified,
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyK),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }));
        assert!(!app.remote_input(), "Ctrl+Alt+K 必须退出远程输入");
        assert_eq!(
            last_visible(&app),
            Some(true),
            "Ctrl+Alt+K 退出必须恢复光标可见"
        );
        assert_eq!(
            last_clipped(&app),
            Some(false),
            "Ctrl+Alt+K 退出必须解除裁剪"
        );
        let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Some(sink) = app.recording_sink() {
            assert_eq!(sink.key_events.lock().unwrap().len(), 0, "退出后不得转发");
        }
    }

    #[test]
    fn ctrl_alt_m_toggles_mouse_mode() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        let before = app.connection.mouse_mode;
        press_mode_toggle(&mut app);
        assert_ne!(
            app.connection.mouse_mode, before,
            "Ctrl+Alt+M 必须切换鼠标模式"
        );
    }

    #[test]
    fn relative_mode_enter_hides_and_clips_cursor() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Relative;
        click_video(&mut app);
        assert!(app.remote_input, "相对模式点击视频区必须进入远程输入");
        assert_eq!(last_visible(&app), Some(false), "相对模式必须隐藏光标");
        assert_eq!(last_clipped(&app), Some(true), "相对模式必须裁剪光标");
    }

    #[test]
    fn absolute_mode_enter_keeps_cursor_visible() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Absolute;
        click_video(&mut app);
        assert!(app.remote_input, "绝对模式点击视频区必须进入远程输入");
        assert_eq!(last_visible(&app), Some(true), "绝对模式不得隐藏光标");
        let clipped = app.cursor_records.clipped.lock().unwrap();
        assert!(
            !clipped.iter().any(|clipped| *clipped),
            "绝对模式不得裁剪光标"
        );
    }

    #[test]
    fn exit_restores_cursor() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Relative;
        *app.video_bounds.borrow_mut() = Some(iced::Rectangle::new(
            iced::Point::ORIGIN,
            iced::Size::new(320.0, 180.0),
        ));
        move_cursor(&mut app, 160.0, 90.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input, "点击视频区必须进入远程输入");
        assert_eq!(last_visible(&app), Some(false));
        assert_eq!(last_clipped(&app), Some(true));
        // 点击视频区外：退出并恢复光标。
        move_cursor(&mut app, 500.0, 500.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(!app.remote_input, "点击视频区外必须退出远程输入");
        assert_eq!(last_visible(&app), Some(true), "退出必须恢复光标可见");
        assert_eq!(last_clipped(&app), Some(false), "退出必须解除裁剪");
    }

    #[test]
    fn toggle_to_absolute_restores_cursor() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Relative;
        click_video(&mut app);
        assert!(app.remote_input);
        assert_eq!(last_visible(&app), Some(false));
        assert_eq!(last_clipped(&app), Some(true));
        // 仍在远程输入时切到绝对：必须恢复光标。
        press_mode_toggle(&mut app);
        assert_eq!(app.connection.mouse_mode, MouseMode::Absolute);
        assert!(app.remote_input, "切换模式不得退出远程输入");
        assert_eq!(
            last_visible(&app),
            Some(true),
            "切到绝对模式必须恢复光标可见"
        );
        assert_eq!(last_clipped(&app), Some(false), "切到绝对模式必须解除裁剪");
        // 再切回相对（仍在远程输入）：重新隐藏裁剪。
        press_mode_toggle(&mut app);
        assert_eq!(app.connection.mouse_mode, MouseMode::Relative);
        assert!(app.remote_input);
        assert_eq!(
            last_visible(&app),
            Some(false),
            "切回相对模式必须重新隐藏光标"
        );
        assert_eq!(
            last_clipped(&app),
            Some(true),
            "切回相对模式必须重新裁剪光标"
        );
    }

    #[test]
    fn disconnect_restores_cursor() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Relative;
        click_video(&mut app);
        assert!(app.remote_input);
        assert_eq!(last_visible(&app), Some(false));
        assert_eq!(last_clipped(&app), Some(true));
        let _ = app.update(Message::Disconnect);
        assert!(!app.remote_input, "断开必须退出远程输入");
        assert_eq!(last_visible(&app), Some(true), "断开必须恢复光标可见");
        assert_eq!(last_clipped(&app), Some(false), "断开必须解除裁剪");
    }

    #[test]
    fn special_key_menu_sends_sequence() {
        let (mut app, _) = MockApp::new_mock();
        enter_remote(&mut app);
        let _ = app.update(Message::Menu(MenuAction::SpecialKey("CtrlAltDel".into())));
        let recorded = wait_sink(&app, 6);
        assert_eq!(recorded[0], (true, 0xe0));
        assert_eq!(recorded[1], (true, 0xe2));
        assert_eq!(recorded[2], (true, 0x4c));
        assert_eq!(recorded[3], (false, 0x4c));
        assert_eq!(recorded[4], (false, 0xe2));
        assert_eq!(recorded[5], (false, 0xe0));
    }

    #[test]
    fn paste_uses_clipboard_and_sets_busy() {
        struct EmptyClipboard;
        impl crate::clipboard::ClipboardReader for EmptyClipboard {
            fn read_text(&self) -> Result<String, String> {
                Ok("hello".into())
            }
        }
        let (mut app, _) = MockApp::new_mock();
        app.clipboard = Arc::new(EmptyClipboard);
        enter_remote(&mut app);
        let _ = app.update(Message::Menu(MenuAction::Simple("paste")));
        assert!(app.paste_busy(), "paste_text 成功后必须置 paste_busy");
    }

    #[test]
    fn relative_pointer_delta_reaches_sink() {
        let (mut app, _) = MockApp::new_mock();
        let factory = Arc::new(crate::relative::ChannelRelativeFactory::new());
        app.relative_factory = factory.clone();
        enter_remote(&mut app);
        let _ = app.update(Message::SetMouseMode(MouseMode::Relative));
        let _ = app.update(Message::UiTick); // 启动相对源
        factory.push(5, -3);
        let _ = app.update(Message::UiTick); // 采样并发送
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink()
                && *sink.pointer_batches.lock().unwrap() > 0
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// 构造绝对模式 + 320x180 视频区/帧就绪的 app（点击进入语义由专门用例覆盖）。
    fn absolute_video_app() -> MockApp {
        let (mut app, _) = MockApp::new_mock();
        app.connection.mouse_mode = MouseMode::Absolute;
        *app.video_bounds.borrow_mut() = Some(iced::Rectangle::new(
            iced::Point::ORIGIN,
            iced::Size::new(320.0, 180.0),
        ));
        app.frame_size = Some(FrameSize {
            width: 320,
            height: 180,
        });
        app
    }

    fn move_cursor(app: &mut MockApp, x: f32, y: f32) {
        let _ = app.update(Message::IcedEvent(iced::Event::Mouse(
            iced::mouse::Event::CursorMoved {
                position: iced::Point::new(x, y),
            },
        )));
    }

    fn press_button(app: &mut MockApp, button: iced::mouse::Button) {
        let _ = app.update(Message::IcedEvent(iced::Event::Mouse(
            iced::mouse::Event::ButtonPressed(button),
        )));
    }

    fn wait_releases(app: &MockApp, count: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink() {
                let releases = sink.releases.lock().unwrap();
                if *releases >= count {
                    return;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "release_all 调用未达 {count} 次"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn wait_absolute_move(app: &MockApp, x: u16, y: u16, count: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink() {
                let moves = sink.absolute_moves.lock().unwrap();
                let seen = moves.iter().filter(|&&(mx, my)| mx == x && my == y).count();
                if seen >= count {
                    return;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "绝对指针 ({x},{y}) 第 {count} 次未达 sink"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn absolute_cursor_move_sends_mapped_coordinates() {
        let mut app = absolute_video_app();

        // 绝对模式移动本身不进入远程输入（点击视频区才进入）。
        move_cursor(&mut app, 160.0, 90.0);
        assert!(!app.remote_input, "移动不得进入远程输入");
        assert!(
            app.recording_sink()
                .unwrap()
                .absolute_moves
                .lock()
                .unwrap()
                .is_empty(),
            "未进入远程输入时移动不得发送绝对指针"
        );

        // 远程输入激活后移动：窗口坐标映射到帧内坐标并发送。
        app.remote_input = true;
        move_cursor(&mut app, 80.0, 45.0);
        wait_absolute_move(&app, 80, 45, 1);
        assert!(app.remote_input, "移动不得退出远程输入");
    }

    #[test]
    fn click_inside_video_enters_remote_input() {
        let mut app = absolute_video_app();
        move_cursor(&mut app, 160.0, 90.0);
        assert!(!app.remote_input, "移动不得进入远程输入");
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input, "点击视频区必须进入远程输入");
    }

    #[test]
    fn click_outside_video_exits_remote_input() {
        let mut app = absolute_video_app();
        // 先进入。
        move_cursor(&mut app, 160.0, 90.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input);
        // 点击视频区外：退出并释放全部按键。
        move_cursor(&mut app, 500.0, 500.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(!app.remote_input, "点击视频区外必须退出远程输入");
        wait_releases(&app, 1);
    }

    #[test]
    fn window_unfocused_exits_remote_input() {
        let mut app = absolute_video_app();
        move_cursor(&mut app, 160.0, 90.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input);
        let _ = app.update(Message::IcedEvent(iced::Event::Window(
            iced::window::Event::Unfocused,
        )));
        assert!(!app.remote_input, "窗口失焦必须退出远程输入");
        wait_releases(&app, 1);
    }

    #[test]
    fn window_rescaled_stores_scale_factor() {
        let (mut app, _) = MockApp::new_mock();
        assert_eq!(app.scale_factor, 1.0);
        let _ = app.update(Message::IcedEvent(iced::Event::Window(
            iced::window::Event::Rescaled(2.5),
        )));
        assert_eq!(app.scale_factor, 2.5);
        let _ = app.update(Message::IcedEvent(iced::Event::Window(
            iced::window::Event::Rescaled(0.0),
        )));
        assert_eq!(app.scale_factor, 0.1);
    }

    #[test]
    fn window_rescaled_stores_scale_factor_even_when_control_offline() {
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Disconnect);
        assert!(!app.controller.is_control_online());
        let _ = app.update(Message::IcedEvent(iced::Event::Window(
            iced::window::Event::Rescaled(2.0),
        )));
        assert_eq!(app.scale_factor, 2.0, "DPI 缩放不应被在线门控吞掉");
    }

    #[test]
    fn video_ratio_uses_rendered_rect_for_actual_size() {
        let (mut app, _) = MockApp::new_mock();
        app.scale_mode = ScaleMode::ActualSize;
        *app.video_bounds.borrow_mut() = Some(iced::Rectangle::new(
            iced::Point::ORIGIN,
            iced::Size::new(1000.0, 600.0),
        ));
        app.frame_size = Some(FrameSize {
            width: 320,
            height: 180,
        });
        // ActualSize 无黑边缩放：渲染矩形 == 帧像素，比例恒为 1:1。
        assert_eq!(app.video_ratio(), (1.0, 1.0));
    }

    #[test]
    fn video_ratio_uses_rendered_rect_for_fit_window() {
        let (mut app, _) = MockApp::new_mock();
        app.scale_mode = ScaleMode::FitWindow;
        *app.video_bounds.borrow_mut() = Some(iced::Rectangle::new(
            iced::Point::ORIGIN,
            iced::Size::new(1000.0, 600.0),
        ));
        let frame = FrameSize {
            width: 1920,
            height: 1080,
        };
        app.frame_size = Some(frame);
        let rendered = crate::scale::frame_rect(
            crate::scale::Rect::from_min_size(0.0, 0.0, 1000.0, 600.0),
            frame,
            app.scale_mode,
        );
        let ratio = app.video_ratio();
        assert!(
            (ratio.0 - frame.width as f32 / rendered.w).abs() < 1e-6,
            "ratio.x 必须按渲染矩形宽计算，实际 {ratio:?}"
        );
        assert!(
            (ratio.1 - frame.height as f32 / rendered.h).abs() < 1e-6,
            "ratio.y 必须按渲染矩形高计算，实际 {ratio:?}"
        );
        // 证明不是直接用容器：letterbox 场景下 y 比必须与容器比不同。
        assert!(
            (ratio.1 - frame.height as f32 / 600.0).abs() > 1e-3,
            "ratio.y 不得直接用容器高度，实际 {ratio:?}"
        );
    }

    #[test]
    fn reenter_after_exit_resends_same_absolute_coordinates() {
        let mut app = absolute_video_app();
        // 进入并发送一次 (160,90)。
        move_cursor(&mut app, 160.0, 90.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input);
        wait_absolute_move(&app, 160, 90, 1);
        // 点击视频区外退出（release_all + 去重/限频复位）。
        move_cursor(&mut app, 500.0, 500.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(!app.remote_input);
        wait_releases(&app, 1);
        // 重进同坐标：必须再次发送，验证去重/限频状态已复位。
        move_cursor(&mut app, 160.0, 90.0);
        press_button(&mut app, iced::mouse::Button::Left);
        assert!(app.remote_input);
        wait_absolute_move(&app, 160, 90, 2);
    }

    #[test]
    fn relative_sensitivity_scales_delta() {
        let (mut app, _) = MockApp::new_mock();
        app.connection.relative_sensitivity = 2.0;
        let factory = Arc::new(crate::relative::ChannelRelativeFactory::new());
        app.relative_factory = factory.clone();
        enter_remote(&mut app);
        let _ = app.update(Message::SetMouseMode(MouseMode::Relative));
        let _ = app.update(Message::UiTick); // 启动相对源
        factory.push(1, 0);
        let _ = app.update(Message::UiTick); // 采样并发送
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(sink) = app.recording_sink() {
                let deltas = sink.relative_deltas.lock().unwrap();
                if !deltas.is_empty() {
                    assert_eq!(deltas[0], (2, 0), "灵敏度 2.0 必须把 (1,0) 放大为 (2,0)");
                    break;
                }
            }
            assert!(std::time::Instant::now() < deadline, "相对增量未达 sink");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn connection_page_view_renders_after_theme_wiring() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(ui.find("Select device").is_ok(), "连接页标题必须渲染");
        assert!(ui.find("Refresh").is_ok(), "刷新按钮必须渲染");
        // #90 布局：profile 行 + 左栏 380 + 右栏 320x180 预览 + 连接设置入口。
        assert!(
            ui.find("Save current options…").is_ok(),
            "profile 行必须包含保存按钮"
        );
        assert!(
            ui.find("Connection settings").is_ok(),
            "profile 行必须包含连接设置入口"
        );
        assert!(ui.find("Connect").is_ok(), "左栏必须包含连接按钮");
        assert!(
            ui.find("No preview").is_ok(),
            "右栏预览区无帧时必须显示占位文字"
        );
    }

    #[test]
    fn video_view_shows_no_signal_when_connected_without_frame() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        // new_mock 构造即在线且无帧 → 视频页。
        let (app, _) = MockApp::new_mock();
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(
            ui.find("No signal").is_ok(),
            "视频页无帧时必须显示 28px 无信号文字"
        );
    }

    #[test]
    fn status_line_shows_five_fields() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Disconnect);
        {
            let mut ui = iced_test::simulator::simulator(app.view());
            assert!(
                ui.find("Control device: Offline").is_ok(),
                "状态栏必须显示控制设备字段"
            );
            assert!(
                ui.find("Keyboard: Ready").is_ok(),
                "断开后键盘状态必须为就绪"
            );
            assert!(ui.find("Mouse: Ready").is_ok(), "断开后鼠标状态必须为就绪");
            assert!(
                ui.find("Video: No signal").is_ok(),
                "状态栏必须显示视频字段"
            );
            assert!(
                ui.find("Status: ").is_err(),
                "无消息时状态栏不得渲染 Status:"
            );
        }
        app.status_message = Some("hello".into());
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(ui.find("Status: hello").is_ok(), "状态栏必须显示消息字段");
    }

    #[test]
    fn status_line_control_has_no_redundant_prefix() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::Disconnect);
        let _ = app.update(Message::RefreshDevices);
        let _ = app.update(Message::SelectVideo("Camera 0".into()));
        let _ = app.update(Message::PreviewTick);
        let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
        let _ = app.update(Message::Connect);
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(
            ui.find("Control device: CH9329(COM9)").is_ok(),
            "状态栏控制设备不得再带 Control: 前缀"
        );
        assert!(
            ui.find("Control device: Control:").is_err(),
            "不得出现冗余的 Control: 前缀"
        );
    }

    #[test]
    fn absolute_mode_shows_last_pointer_coordinates() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let mut app = absolute_video_app();
        app.remote_input = true;
        move_cursor(&mut app, 160.0, 90.0);
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(
            ui.find("Mouse: (160, 90)").is_ok(),
            "绝对模式必须显示最近指针坐标"
        );
    }

    #[test]
    fn status_line_hides_empty_message() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let (app, _) = MockApp::new_mock();
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(ui.find("Status: ").is_err(), "无消息时不得渲染 Status: ");
    }

    #[test]
    fn connection_modal_contains_connection_params() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::OpenModal(ModalKind::Connection));
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(ui.find("Baud rate").is_ok(), "连接设置模态必须含波特率");
        assert!(ui.find("Preview FPS").is_ok(), "连接设置模态必须含预览帧率");
        assert!(
            ui.find("Auto-detect baud rate").is_ok(),
            "连接设置模态必须含自动波特率开关"
        );
        assert!(ui.find("Mouse mode").is_ok(), "连接设置模态必须含鼠标模式");
        assert!(
            ui.find("Relative sensitivity").is_ok(),
            "连接设置模态必须含相对灵敏度"
        );
        assert!(
            ui.find("Restore defaults").is_ok(),
            "连接设置模态必须含恢复默认值按钮"
        );
    }

    #[test]
    fn about_modal_shows_version_license_and_url() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        let (mut app, _) = MockApp::new_mock();
        let _ = app.update(Message::OpenModal(ModalKind::About));
        let mut ui = iced_test::simulator::simulator(app.view());
        assert!(ui.find("About my_ipkvm").is_ok(), "关于模态必须含标题");
        assert!(
            ui.find(format!("Version: {}", env!("GIT_COMMIT"))).is_ok(),
            "关于模态必须含构建版本"
        );
        assert!(ui.find("License: MIT").is_ok(), "关于模态必须含许可证");
        assert!(
            ui.find(format!("Project: {}", PROJECT_URL)).is_ok(),
            "关于模态必须含项目地址"
        );
    }

    #[test]
    fn video_image_is_centered_in_free_axis() {
        use iced::advanced::clipboard::Null;
        use iced::advanced::mouse;
        use iced::advanced::renderer;
        use iced::{Color, Point, Rectangle, Size};
        use iced_runtime::user_interface::{self, UserInterface};

        // 画布 2:1（256x64），图像 16:9：富余方向为水平轴。
        // 不能用 128x64：tiny_skia 离屏光栅把 (bounds.x / scale) 截断成源像素
        // 索引，7.11px 的居中偏移会被截成 0（正确修复后像素仍贴左）；加宽到
        // 256x64 后水平边距 71.11px 恰为 10 个源像素，可被像素探测区分。
        const WIDTH: u32 = 256;
        const HEIGHT: u32 = 64;

        let handle =
            iced::widget::image::Handle::from_rgba(16, 9, [255u8, 0, 0, 255].repeat(16 * 9));
        let view: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
            iced::widget::container(MockApp::fit_image(handle))
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into();

        let mut renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), 16.0.into());
        let mut ui = UserInterface::build(
            view,
            Size::new(WIDTH as f32, HEIGHT as f32),
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

        let theme = iced::Theme::Dark;
        let style = renderer::Style {
            text_color: theme.palette().text,
        };
        let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT).expect("pixmap");
        let mut mask = tiny_skia::Mask::new(WIDTH, HEIGHT).expect("mask");
        let viewport =
            iced::advanced::graphics::Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0);
        ui.draw(
            &mut renderer,
            &theme,
            &style,
            mouse::Cursor::Available(Point::ORIGIN),
        );
        renderer.draw(
            &mut pixmap.as_mut(),
            &mut mask,
            &viewport,
            &[Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32))],
            Color::from_rgb(0.1, 0.1, 0.1),
        );

        let background = [26u8, 26, 26];
        let mut min_x = WIDTH as i32;
        let mut min_y = HEIGHT as i32;
        let mut max_x = 0i32;
        let mut max_y = 0i32;
        let mut count = 0usize;
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let px = pixmap.pixel(x, y).expect("pixel");
                let close = |a: u8, b: u8| (a as i16 - b as i16).unsigned_abs() <= 8;
                if !(close(px.red(), background[0])
                    && close(px.green(), background[1])
                    && close(px.blue(), background[2]))
                {
                    count += 1;
                    min_x = min_x.min(x as i32);
                    min_y = min_y.min(y as i32);
                    max_x = max_x.max(x as i32);
                    max_y = max_y.max(y as i32);
                }
            }
        }
        assert!(count > 0, "图像必须渲染出像素（实际 {count} 个）");
        let center_x = (min_x + max_x) as f32 / 2.0;
        let center_y = (min_y + max_y) as f32 / 2.0;
        // 16:9 图像在 2:1 画布上垂直轴完全填满，富余方向只有水平轴；
        // 因此断言水平居中与水平不贴边（min.y 恒为 0，无需断言）。
        assert!(
            (center_x - 128.0).abs() <= 2.0,
            "图像水平中心应≈128，实际 {center_x}"
        );
        assert!(
            (center_y - 32.0).abs() <= 2.0,
            "图像垂直中心应≈32，实际 {center_y}"
        );
        assert!(min_x > 4, "图像不应贴左（bbox.min.x={min_x}）");
    }
}
