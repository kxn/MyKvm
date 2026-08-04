//! 连接页状态机与预览驱动（M2）。

pub use ipkvm_desktop::probe::{ProductionProbeBackend, resolve_connect_baud};
pub use ipkvm_desktop_core::config::{
    ConnectionSettings, DeviceRef, ManualSnapshot, Profile, ProfileStore,
};
pub use ipkvm_desktop_core::probe::{ProbeBackend, ProbeError, refresh_detection};
pub use ipkvm_desktop_core::state::{
    ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState, PreviewInfo,
    VideoProbeStatus,
};

use std::sync::Arc;
use std::time::{Duration, Instant};

use ipkvm_video::FrameSource;

/// 控制设备探测超时。
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
/// 预览出帧后停帧视为无信号的超时。
pub const NO_SIGNAL_TIMEOUT: Duration = Duration::from_secs(3);

/// 预览源工厂：生产用真实相机，测试注入 mock。
pub trait PreviewSourceFactory {
    fn open(&self, device_id: &str, fps: u64) -> Result<Arc<dyn FrameSource>, String>;
}

/// 生产预览源：ipkvm-video 相机。
#[derive(Default)]
pub struct CameraPreviewFactory;

impl PreviewSourceFactory for CameraPreviewFactory {
    fn open(&self, device_id: &str, fps: u64) -> Result<Arc<dyn FrameSource>, String> {
        ipkvm_video::camera::CameraSource::open(device_id, fps)
            .map(|source| Arc::new(source) as Arc<dyn FrameSource>)
            .map_err(|error| error.to_string())
    }
}

/// 刷新枚举后视频预览的处理决策。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewRefreshAction {
    Skip,
    Reopen,
    KeepDisconnected,
}

pub fn preview_refresh_action(
    status: &VideoProbeStatus,
    device_present: bool,
) -> PreviewRefreshAction {
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

/// 超时判定。
pub fn elapsed_since(since: Option<Instant>, timeout: Duration, now: Instant) -> bool {
    since.is_some_and(|at| now.duration_since(at) >= timeout)
}

/// 预览运行时：持有预览帧源与时间戳，按 tick 推进 video_status。
#[derive(Default)]
pub struct PreviewRuntime {
    source: Option<Arc<dyn FrameSource>>,
    device_id: Option<String>,
    opened_at: Option<Instant>,
    last_frame_at: Option<Instant>,
}

impl PreviewRuntime {
    pub fn reset(&mut self) {
        self.source = None;
        self.device_id = None;
        self.opened_at = None;
        self.last_frame_at = None;
    }

    /// 当前预览源（app 用它取最新帧构建 Handle）。
    pub fn source(&self) -> Option<&Arc<dyn FrameSource>> {
        self.source.as_ref()
    }

    /// 推进一帧：打开/换源并进行超时判定。
    /// 返回 true 表示本 tick 收到了新帧（调用方应刷新预览 Handle）。
    pub fn tick(
        &mut self,
        selection: &mut DeviceSelectionState,
        factory: &dyn PreviewSourceFactory,
        fps: u64,
        now: Instant,
    ) -> bool {
        if selection.video_status == VideoProbeStatus::Disconnected {
            return false;
        }
        let video_id = selection.selected_video_id.clone();
        if self.device_id.as_deref() != video_id.as_deref() {
            self.reset();
            self.device_id = video_id.clone();
            match video_id {
                Some(id) => match factory.open(&id, fps) {
                    Ok(source) => {
                        self.source = Some(source);
                        self.opened_at = Some(now);
                        selection.video_status = VideoProbeStatus::Checking;
                    }
                    Err(error) => {
                        selection.video_status = VideoProbeStatus::OpenFailed(error);
                        return false;
                    }
                },
                None => {
                    selection.video_status = VideoProbeStatus::NotSelected;
                    return false;
                }
            }
        }
        let Some(source) = &self.source else {
            return false;
        };
        let Some(frame) = source.latest_frame() else {
            let stalled = match selection.video_status {
                VideoProbeStatus::Checking => elapsed_since(self.opened_at, PROBE_TIMEOUT, now),
                VideoProbeStatus::Ready(_) => {
                    elapsed_since(self.last_frame_at, NO_SIGNAL_TIMEOUT, now)
                }
                _ => false,
            };
            if stalled {
                selection.video_status = VideoProbeStatus::NoSignal;
            }
            return false;
        };
        self.last_frame_at = Some(now);
        if !matches!(selection.video_status, VideoProbeStatus::Ready(_)) {
            selection.video_status = VideoProbeStatus::Ready(PreviewInfo {
                width: frame.width,
                height: frame.height,
                label: source.source_info().device_name,
            });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_video::mock::MockFrameSource;
    use ipkvm_video::{
        FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame, VideoSourceInfo,
        VideoSourceKind,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn option(id: &str, label: &str) -> DeviceOption {
        DeviceOption {
            id: id.into(),
            label: label.into(),
        }
    }

    fn ready_state() -> DeviceSelectionState {
        DeviceSelectionState {
            video_devices: vec![option("cam0", "Camera 0")],
            control_devices: vec![option("COM9", "COM9")],
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            video_status: VideoProbeStatus::Ready(PreviewInfo {
                width: 640,
                height: 480,
                label: "Camera 0".into(),
            }),
            control_status: ControlProbeStatus::Ready(ControlInfo {
                version: 0x31,
                usb_enumerated: true,
                baud: 115200,
            }),
        }
    }

    fn make_frame(seq: u64, w: u32, h: u32) -> Arc<VideoFrame> {
        let mut data = vec![0u8; (w * h * 4) as usize];
        data[0] = 10;
        data[1] = 20;
        data[2] = 30;
        data[3] = 255;
        Arc::new(VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            w,
            h,
            w * 4,
            PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        ))
    }

    #[derive(Default)]
    struct MockPreviewFactory;

    impl PreviewSourceFactory for MockPreviewFactory {
        fn open(&self, device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            assert_eq!(device_id, "cam0");
            let mock = Arc::new(MockFrameSource::new());
            mock.publish_frame(make_frame(1, 64, 48));
            Ok(mock as Arc<dyn FrameSource>)
        }
    }

    #[derive(Default)]
    struct FailingPreviewFactory;

    impl PreviewSourceFactory for FailingPreviewFactory {
        fn open(&self, _device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            Err("boom".into())
        }
    }

    #[derive(Default)]
    struct EmptyPreviewFactory;

    impl PreviewSourceFactory for EmptyPreviewFactory {
        fn open(&self, device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            assert_eq!(device_id, "cam0");
            Ok(Arc::new(MockFrameSource::new()) as Arc<dyn FrameSource>)
        }
    }

    /// 只出一帧的帧源：出帧后 latest_frame 返回 None，可模拟停帧。
    struct OneShotSource {
        frame: std::sync::Mutex<Option<ipkvm_video::SharedVideoFrame>>,
    }

    impl FrameSource for OneShotSource {
        fn latest_frame(&self) -> Option<ipkvm_video::SharedVideoFrame> {
            self.frame.lock().unwrap().take()
        }

        fn subscribe(&self) -> FrameReceiver {
            tokio::sync::watch::channel(None).1
        }

        fn source_info(&self) -> VideoSourceInfo {
            VideoSourceInfo {
                kind: VideoSourceKind::Generated,
                device_name: "one-shot".into(),
                is_loop: false,
            }
        }
    }

    #[derive(Default)]
    struct OneShotPreviewFactory;

    impl PreviewSourceFactory for OneShotPreviewFactory {
        fn open(&self, device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            assert_eq!(device_id, "cam0");
            Ok(Arc::new(OneShotSource {
                frame: std::sync::Mutex::new(Some(make_frame(1, 64, 48))),
            }) as Arc<dyn FrameSource>)
        }
    }

    #[test]
    fn connect_requires_video_ready_and_control_ready() {
        let mut state = DeviceSelectionState {
            video_status: VideoProbeStatus::Ready(PreviewInfo {
                width: 1920,
                height: 1080,
                label: "capture".into(),
            }),
            control_status: ControlProbeStatus::NoResponse,
            ..DeviceSelectionState::default()
        };
        assert!(!state.can_connect());
        state.control_status = ControlProbeStatus::Ready(ControlInfo {
            version: 0x31,
            usb_enumerated: true,
            baud: 115200,
        });
        assert!(state.can_connect());
    }

    #[test]
    fn refresh_marks_missing_selected_devices_disconnected() {
        let mut state = ready_state();
        state.refresh_devices(Vec::new(), Vec::new());
        assert_eq!(state.video_status, VideoProbeStatus::Disconnected);
        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
        assert!(!state.can_connect());
    }

    #[test]
    fn mark_control_offline_sets_disconnected_status() {
        let mut state = ready_state();
        state.mark_control_offline();
        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
    }

    #[test]
    fn preview_refresh_skips_when_ready_or_checking_or_not_selected() {
        let cases = [
            (
                VideoProbeStatus::Ready(PreviewInfo {
                    width: 1,
                    height: 1,
                    label: "x".into(),
                }),
                true,
            ),
            (VideoProbeStatus::Checking, false),
            (VideoProbeStatus::NotSelected, true),
        ];
        for (status, present) in cases {
            assert_eq!(
                preview_refresh_action(&status, present),
                PreviewRefreshAction::Skip
            );
        }
    }

    #[test]
    fn preview_refresh_reopens_on_failure_or_no_signal() {
        for status in [
            VideoProbeStatus::OpenFailed("x".into()),
            VideoProbeStatus::NoSignal,
        ] {
            assert_eq!(
                preview_refresh_action(&status, true),
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
        let t0 = Instant::now();
        assert!(!elapsed_since(
            None,
            Duration::from_secs(1),
            t0 + Duration::from_secs(2)
        ));
        assert!(elapsed_since(
            Some(t0),
            Duration::from_secs(1),
            t0 + Duration::from_secs(2)
        ));
    }

    #[test]
    fn preview_tick_reaches_ready_with_frame() {
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            ..DeviceSelectionState::default()
        };
        let mut preview = PreviewRuntime::default();
        let got = preview.tick(&mut state, &MockPreviewFactory, 30, Instant::now());
        assert!(got);
        assert!(
            matches!(state.video_status, VideoProbeStatus::Ready(info) if info.width == 64 && info.height == 48)
        );
        assert!(preview.source().is_some());
    }

    #[test]
    fn preview_tick_open_failure_sets_open_failed() {
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            ..DeviceSelectionState::default()
        };
        let mut preview = PreviewRuntime::default();
        assert!(!preview.tick(&mut state, &FailingPreviewFactory, 30, Instant::now()));
        assert_eq!(
            state.video_status,
            VideoProbeStatus::OpenFailed("boom".into())
        );
    }

    #[test]
    fn preview_tick_no_frame_times_out_to_no_signal() {
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            ..DeviceSelectionState::default()
        };
        let mut preview = PreviewRuntime::default();
        let t0 = Instant::now();
        assert!(!preview.tick(&mut state, &EmptyPreviewFactory, 30, t0));
        assert_eq!(state.video_status, VideoProbeStatus::Checking);
        assert!(!preview.tick(&mut state, &EmptyPreviewFactory, 30, t0 + PROBE_TIMEOUT));
        assert_eq!(state.video_status, VideoProbeStatus::NoSignal);
    }

    #[test]
    fn preview_tick_stall_after_ready_moves_to_no_signal() {
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            ..DeviceSelectionState::default()
        };
        let mut preview = PreviewRuntime::default();
        let t0 = Instant::now();
        assert!(preview.tick(&mut state, &OneShotPreviewFactory, 30, t0));
        assert!(matches!(state.video_status, VideoProbeStatus::Ready(_)));
        assert!(!preview.tick(
            &mut state,
            &OneShotPreviewFactory,
            30,
            t0 + NO_SIGNAL_TIMEOUT
        ));
        assert_eq!(state.video_status, VideoProbeStatus::NoSignal);
    }

    #[test]
    fn preview_tick_disconnected_never_reopens() {
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            video_status: VideoProbeStatus::Disconnected,
            ..DeviceSelectionState::default()
        };
        let mut preview = PreviewRuntime::default();
        assert!(!preview.tick(&mut state, &MockPreviewFactory, 30, Instant::now()));
        assert_eq!(state.video_status, VideoProbeStatus::Disconnected);
        assert!(preview.source().is_none());
    }
}
