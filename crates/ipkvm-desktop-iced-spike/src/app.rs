//! 最小 iced 应用骨架（spike 1）。
//!
//! 复用 [`ipkvm_desktop::DesktopSessionController`]，仅把帧唤醒从 egui 的
//! `request_repaint` 换成 iced 的 `Subscription`（见 [`crate::frames`]）。
//! `image::Handle` 存 state，view 只 clone（#3160 经验）。

use std::sync::Arc;
use std::time::Instant;

use iced::widget::image::Handle;
use iced::widget::{Image, column, text};
use iced::{Center, Element, Fill, Subscription, Task};
use ipkvm_core::{InputError, InputSink, KeyEvent, MouseMode, PointerEvent};
use ipkvm_desktop::{ConnectRequest, DesktopSessionController, DesktopSessionError, SessionParts};
use ipkvm_session::rfb_connection::RfbConnectionGate;
use ipkvm_video::mock::MockFrameSource;
use ipkvm_video::{FrameSource, VideoFrame};

use crate::frames::{FrameUpdate, frame_subscription};

/// 记录型 sink：spike 用，复刻自 desktop session 测试。观察键/指针批次。
#[derive(Clone, Debug, Default)]
pub struct RecordingSink {
    pub key_batches: Arc<std::sync::Mutex<usize>>,
    pub pointer_batches: Arc<std::sync::Mutex<usize>>,
    /// 按到达顺序记录的键事件（down, HID usage）。
    pub key_events: Arc<std::sync::Mutex<Vec<(bool, u8)>>>,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> Result<(), InputError> {
        Ok(())
    }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> Result<(), InputError> {
        *self.key_batches.lock().unwrap() += 1;
        let mut recorded = self.key_events.lock().unwrap();
        for event in events {
            match event {
                KeyEvent::Down { usage } => recorded.push((true, usage.get())),
                KeyEvent::Up { usage } => recorded.push((false, usage.get())),
            }
        }
        Ok(())
    }

    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> Result<(), InputError> {
        *self.pointer_batches.lock().unwrap() += 1;
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        Ok(())
    }
}

type SpikeController = DesktopSessionController<RecordingSink, SpikeFactory>;
type SpikeFactory =
    Box<dyn FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>>;

/// 帧渲染统计（跨线程共享，perf 脚本退出时读取打印 JSON）。
#[derive(Debug, Default)]
pub struct FrameStats {
    inner: std::sync::Mutex<FrameStatsInner>,
}

#[derive(Debug, Default)]
struct FrameStatsInner {
    /// 每帧渲染到达的时间戳（首帧后）。
    timestamps: Vec<Instant>,
}

impl FrameStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 记录一帧到达。
    fn record(&self) {
        self.inner.lock().unwrap().timestamps.push(Instant::now());
    }

    /// 计算总结统计（帧数、平均/p95 帧间隔，毫秒）。
    pub fn summary(&self) -> (u64, f64, f64) {
        let inner = self.inner.lock().unwrap();
        let n = inner.timestamps.len() as u64;
        let mut intervals: Vec<f64> = Vec::new();
        for w in inner.timestamps.windows(2) {
            intervals.push(w[1].duration_since(w[0]).as_secs_f64() * 1000.0);
        }
        let avg = if intervals.is_empty() {
            0.0
        } else {
            intervals.iter().sum::<f64>() / intervals.len() as f64
        };
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95 = if intervals.is_empty() {
            0.0
        } else {
            intervals[((intervals.len() as f64 - 1.0) * 0.95).round() as usize]
        };
        (n, avg, p95)
    }
}

/// spike 应用状态：持 controller + 帧源 + 当前 Handle + 帧计数。
pub struct SpikeApp {
    controller: SpikeController,
    frame_source: Arc<MockFrameSource>,
    /// 当前帧的 image Handle（存 state；view 只 clone）。
    handle: Option<Handle>,
    /// 已渲染的帧数（性能/丢帧计数用）。
    rendered_frames: u64,
    /// 帧订阅是否仍活跃（Closed 后停）。
    subscribed: bool,
    /// 帧渲染统计（跨线程共享，退出时读）。
    stats: Arc<FrameStats>,
}

/// spike 消息。
#[derive(Clone, Debug)]
pub enum Message {
    /// 新帧到达（来自 iced Subscription）。
    FrameReady(VideoFrame),
    /// 帧源关闭。
    FrameClosed,
}

impl SpikeApp {
    /// 构造并连接：注入 MockFrameSource + RecordingSink。
    pub fn new(
        frame_source: Arc<MockFrameSource>,
        stats: Arc<FrameStats>,
    ) -> (Self, Task<Message>) {
        let fs_for_factory = Arc::clone(&frame_source);
        let factory: SpikeFactory = Box::new(move |_request| {
            let frame_source: Arc<dyn FrameSource> = fs_for_factory.clone();
            Ok((
                frame_source,
                RecordingSink::default(),
                RfbConnectionGate::new(),
            ))
        });
        let mut controller = DesktopSessionController::with_factory(factory);
        controller
            .connect(connect_request())
            .expect("spike controller connect");

        (
            Self {
                controller,
                frame_source,
                handle: None,
                rendered_frames: 0,
                subscribed: true,
                stats,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FrameReady(frame) => {
                // Handle 存 state：每次新帧重建 Handle，view 只 clone。
                self.handle = Some(handle_from_frame(&frame));
                self.rendered_frames += 1;
                self.stats.record();
                Task::none()
            }
            Message::FrameClosed => {
                self.subscribed = false;
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content = if let Some(handle) = self.handle.as_ref() {
            column![
                Image::new(handle.clone()),
                text(format!("已渲染帧数: {}", self.rendered_frames)),
            ]
        } else {
            column![text("等待帧…")]
        };
        content.align_x(Center).width(Fill).height(Fill).into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if !self.subscribed {
            return Subscription::none();
        }
        // controller 暴露平台中立帧订阅；每次重新 subscribe 一份 receiver。
        self.controller
            .subscribe_frames()
            .map(|receiver| {
                frame_subscription(0, receiver).map(|update| match update {
                    FrameUpdate::Frame(frame) => Message::FrameReady((*frame).clone()),
                    FrameUpdate::Closed => Message::FrameClosed,
                })
            })
            .unwrap_or_else(Subscription::none)
    }

    /// 已渲染帧数（性能采集用）。
    pub fn rendered_frames(&self) -> u64 {
        self.rendered_frames
    }

    /// 帧源句柄（example 推帧用）。
    pub fn frame_source(&self) -> &Arc<MockFrameSource> {
        &self.frame_source
    }
}

/// 把 BGRA VideoFrame 转 RGBA image::Handle（复刻 desktop frame.rs 的转换）。
pub fn handle_from_frame(frame: &VideoFrame) -> Handle {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = &frame.data[y * stride..y * stride + width * 4];
        let dst = &mut rgba[y * width * 4..(y + 1) * width * 4];
        for (o, bgra) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            o.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    Handle::from_rgba(frame.width, frame.height, rgba)
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

    fn make_bgra_frame(seq: u64) -> VideoFrame {
        VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            2,
            2,
            8,
            PixelFormat::Bgra8888,
            Arc::from(
                vec![
                    10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
                ]
                .into_boxed_slice(),
            ),
        )
    }

    #[test]
    fn handle_from_frame_converts_without_panic() {
        // 2×2 BGRA 帧 → RGBA Handle。Handle 无公开像素访问器，这里验证转换不 panic。
        // （#3160 模式：Handle 存 state 由 view clone；转换正确性由 BGRA→RGBA 顺序保证。）
        let frame = make_bgra_frame(1);
        let _handle = handle_from_frame(&frame);
    }

    #[test]
    fn update_increments_rendered_frames_and_stores_handle() {
        // 构造完整 SpikeApp（含 controller 连接），验证 update 逻辑。
        let frame_source = Arc::new(MockFrameSource::new());
        let stats = FrameStats::new();
        let (mut app, _) = SpikeApp::new(frame_source, stats);

        assert_eq!(app.rendered_frames(), 0);
        assert!(app.handle.is_none());

        let _ = app.update(Message::FrameReady(make_bgra_frame(1)));
        assert_eq!(app.rendered_frames(), 1);
        assert!(app.handle.is_some(), "FrameReady 后 Handle 必须存入 state");

        let _ = app.update(Message::FrameReady(make_bgra_frame(2)));
        assert_eq!(app.rendered_frames(), 2);

        let _ = app.controller.stop();
    }

    #[test]
    fn frame_closed_stops_subscription_flag() {
        let frame_source = Arc::new(MockFrameSource::new());
        let stats = FrameStats::new();
        let (mut app, _) = SpikeApp::new(frame_source, stats);

        assert!(app.subscribed);
        let _ = app.update(Message::FrameClosed);
        assert!(
            !app.subscribed,
            "FrameClosed 后订阅标志必须为 false（subscription 据此返回 none）"
        );

        let _ = app.controller.stop();
    }
}
