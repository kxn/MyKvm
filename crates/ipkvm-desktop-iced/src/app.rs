//! 应用状态/消息/视图/订阅（M1）：连接 mock 帧源，消费帧订阅并渲染。

use std::sync::Arc;

use iced::widget::image::Handle;
use iced::{Color, Element, Length, Size, Subscription, Task};
use ipkvm_core::{InputError, InputSink, KeyEvent, MouseMode, PointerEvent};
use ipkvm_desktop::{ConnectRequest, DesktopSessionController, DesktopSessionError, SessionParts};
use ipkvm_session::rfb_connection::RfbConnectionGate;
use ipkvm_video::mock::MockFrameSource;
use ipkvm_video::{FrameSource, VideoFrame};

use crate::frames::{FrameUpdate, frame_subscription};
use crate::scale::{FrameSize, ScaleMode};
use crate::status::{ConnectionStatus, derive_status};
use crate::video::handle_from_frame;
use crate::{WINDOW_SIZE, WINDOW_TITLE};

/// 记录型 sink：测试与 mock 连接用，观察键/指针批次。
#[derive(Clone, Debug, Default)]
pub struct RecordingSink {
    pub key_batches: Arc<std::sync::Mutex<usize>>,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> Result<(), InputError> {
        Ok(())
    }

    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> Result<(), InputError> {
        *self.key_batches.lock().unwrap() += 1;
        Ok(())
    }

    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> Result<(), InputError> {
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        Ok(())
    }
}

type MockFactory =
    Box<dyn FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>>;
type MockController = DesktopSessionController<RecordingSink, MockFactory>;

/// 应用消息。
#[derive(Clone, Debug)]
pub enum Message {
    /// 新帧到达（来自 iced Subscription）。
    FrameReady(VideoFrame),
    /// 帧源关闭。
    FrameClosed,
    /// 切换缩放模式。
    SetScaleMode(ScaleMode),
    /// 设置黑边颜色。
    SetLetterboxColor(Color),
    /// 切换界面语言。
    ToggleLocale,
    /// 主窗口已打开（iced 0.14 无 Id::MAIN，需运行时捕获窗口 Id）。
    WindowOpened(iced::window::Id),
}

/// 应用状态：controller + 当前帧 Handle + 缩放/黑边/状态栏。
pub struct App {
    pub(crate) controller: MockController,
    frame_source: Arc<MockFrameSource>,
    handle: Option<Handle>,
    frame_size: Option<FrameSize>,
    scale_mode: ScaleMode,
    letterbox_color: Color,
    status: ConnectionStatus,
    subscribed: bool,
    zh: bool,
    window_id: Option<iced::window::Id>,
    pending_resize: Option<Size>,
}

impl App {
    /// 构造并连接 mock 会话（注入 MockFrameSource + RecordingSink）。
    pub fn new_mock() -> (Self, Task<Message>) {
        let frame_source = Arc::new(MockFrameSource::new());
        let fs = Arc::clone(&frame_source);
        let factory: MockFactory = Box::new(move |_req| {
            let src: Arc<dyn FrameSource> = fs.clone();
            Ok((src, RecordingSink::default(), RfbConnectionGate::new()))
        });
        let mut controller = DesktopSessionController::with_factory(factory);
        controller.connect(connect_request()).expect("mock connect");
        let status = derive_status(
            controller.is_control_online(),
            controller.input_offline_reason(),
        );
        (
            Self {
                controller,
                frame_source,
                handle: None,
                frame_size: None,
                scale_mode: ScaleMode::FitWindow,
                letterbox_color: Color::from_rgb(0.0, 0.0, 0.0),
                status,
                subscribed: true,
                zh: true,
                window_id: None,
                pending_resize: None,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FrameReady(frame) => {
                self.handle = Some(handle_from_frame(&frame));
                self.frame_size = Some(FrameSize {
                    width: frame.width,
                    height: frame.height,
                });
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
                self.zh = !self.zh;
                Task::none()
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                if let Some(size) = self.pending_resize.take() {
                    return iced::window::resize(id, size);
                }
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let window_events = iced::window::open_events().map(Message::WindowOpened);
        if !self.subscribed {
            return window_events;
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
        Subscription::batch([frames, window_events])
    }

    pub fn view(&self) -> Element<'_, Message> {
        use iced::widget::{column, container, image, text};
        let video: Element<'_, Message> = match self.handle.as_ref() {
            Some(handle) => image::Image::<Handle>::new(handle.clone())
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => text("等待帧…").into(),
        };
        let video_area = container(video)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(self.letterbox_color.into()),
                ..Default::default()
            });
        let status_line = container(text(self.status.label(self.zh)))
            .width(Length::Fill)
            .padding(6);
        column![video_area, status_line].into()
    }

    pub fn sync_status(&mut self) {
        self.status = derive_status(
            self.controller.is_control_online(),
            self.controller.input_offline_reason(),
        );
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

    pub fn frame_source(&self) -> &Arc<MockFrameSource> {
        &self.frame_source
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

/// 启动 iced 应用（bin 入口调用；测试不启动真实窗口）。
pub fn run() -> iced::Result {
    iced::application(App::new_mock, App::update, App::view)
        .subscription(App::subscription)
        .title(WINDOW_TITLE)
        .window_size(WINDOW_SIZE)
        .run()
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
        let (mut app, _) = App::new_mock();
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
        let (mut app, _) = App::new_mock();
        assert!(app.subscribed());
        let _ = app.update(Message::FrameClosed);
        assert!(!app.subscribed(), "FrameClosed 后订阅必须停");
    }

    #[test]
    fn scale_mode_and_letterbox_transitions() {
        let (mut app, _) = App::new_mock();
        let _ = app.update(Message::SetScaleMode(ScaleMode::ActualSize));
        assert_eq!(app.scale_mode(), ScaleMode::ActualSize);
        let color = Color::from_rgb(0.1, 0.2, 0.3);
        let _ = app.update(Message::SetLetterboxColor(color));
        assert_eq!(app.letterbox_color(), color);
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
        let (mut app, _) = App::new_mock();
        app.controller.stop().unwrap();
        app.sync_status();
        assert_eq!(app.status(), &ConnectionStatus::Disconnected);
    }
}
