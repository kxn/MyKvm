//! 视频采集抽象。

pub mod dirty_rects;

#[cfg(feature = "camera")]
pub mod camera;
// Linux/macOS nokhwa 后端（Windows 不编译，Windows 用 DirectShow sink filter）。
#[cfg(all(unix, feature = "camera"))]
pub mod camera_nokhwa;
#[cfg(feature = "camera")]
pub mod dshow_sink;
#[cfg(feature = "assets")]
pub mod file_source;
#[cfg(feature = "assets")]
pub mod looping;
#[cfg(feature = "test-support")]
pub mod mock;
#[cfg(feature = "assets")]
pub mod y4m;

use std::sync::{Arc, Mutex};

/// 全工作区唯一的进程单调时钟（纳秒）。
///
/// 零点为首次调用时刻（进程启动后不久），不可跨进程比较。`ipkvm-session` 的
/// `now_ns()` 转发到本函数，确保 `frame.timestamp`（采集时间）与 `/api/status`
/// 的 `observe_ns`（观察时间）同源可比，可正确相减得到端到端延迟。
///
/// 见调研 `docs/superpowers/specs/2026-08-04-video-pipeline-performance-research.md`
/// 阶段 0：`last_frame_ns` 语义修正。
pub fn now_ns() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Yuy2,
    Nv12,
    Rgb888,
    Bgra8888,
    Mjpeg,
    H264,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub pixel_format: PixelFormat,
}

impl VideoFormat {
    pub fn new(width: u32, height: u32, frames_per_second: u32, pixel_format: PixelFormat) -> Self {
        Self {
            width,
            height,
            frames_per_second,
            pixel_format,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoDeviceInfo {
    pub id: String,
    pub display_name: String,
    pub backend: String,
    pub supported_formats: Vec<VideoFormat>,
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub seq: u64,
    pub timestamp: MonotonicTimestamp,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: PixelFormat,
    pub data: Arc<[u8]>,
    /// 变化区域（dirty rects），由帧源的 DirtyRectDetector 填充（可开关，默认 None）。
    /// None 表示无差分信息（发全帧）；Some(空 Vec) 表示无变化（静态画面）。
    pub dirty_rects: Option<Vec<Rect>>,
}

/// 矩形区域（帧内坐标）。对应 RFB 的 RfbRectangle，但定义在 video crate（不依赖 rfb）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl VideoFrame {
    pub fn new(
        seq: u64,
        timestamp: MonotonicTimestamp,
        width: u32,
        height: u32,
        stride: u32,
        pixel_format: PixelFormat,
        data: Arc<[u8]>,
    ) -> Self {
        Self {
            seq,
            timestamp,
            width,
            height,
            stride,
            pixel_format,
            data,
            dirty_rects: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoSourceKind {
    None,
    Camera,
    VideoFile,
    Generated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoSourceInfo {
    pub kind: VideoSourceKind,
    pub device_name: String,
    pub is_loop: bool,
}

pub type SharedVideoFrame = Arc<VideoFrame>;
pub type FrameReceiver = tokio::sync::watch::Receiver<Option<SharedVideoFrame>>;

pub trait FrameSource: Send + Sync {
    fn latest_frame(&self) -> Option<SharedVideoFrame>;
    fn subscribe(&self) -> FrameReceiver;
    fn source_info(&self) -> VideoSourceInfo;
    /// 可选：返回源侧采集/转换统计快照（默认 None，供 `/api/status` 读取）。
    fn source_stats(&self) -> Option<SourceStatsSnapshot> {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MonotonicTimestamp {
    pub nanos: u64,
}

impl MonotonicTimestamp {
    pub fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    /// 取当前时刻（统一进程时钟 [`now_ns`]）。
    pub fn now() -> Self {
        Self { nanos: now_ns() }
    }
}

/// 采集/转换段统计快照（供 `/api/status` 等只读读取）。
///
/// 字段含义见 [`SourceStats`] 各 `record_*` 方法。所有累计值为进程启动以来
/// 的累计（不重置），供实施单 B–G 优化前后对比。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceStatsSnapshot {
    /// 成功 publish 的帧数（不含被 watch coalesce 掉的中间帧）。
    pub published_frames: u64,
    /// publish 侧 seq 跳跃累计丢帧数。
    pub dropped_frames: u64,
    /// 像素格式转换累计耗时（纳秒）。
    pub convert_ns_total: u64,
    /// 像素格式转换调用次数。
    pub convert_count: u64,
    /// 采集等待累计耗时（纳秒）。
    pub capture_wait_ns_total: u64,
    /// 采集等待调用次数。
    pub capture_wait_count: u64,
    /// 最后一次 publish 的采集时间（统一时钟），None 表示从未出帧。
    pub last_capture_ns: Option<u64>,
}

/// 源侧采集/转换统计：源 FPS、丢帧、转换耗时、采集等待耗时。
///
/// 设计目标：为后续性能优化（实施单 B–G）提供回归基线。线程安全，内部
/// `Mutex`；采集线程写，`/api/status` 等读端调 [`snapshot`](Self::snapshot)。
///
/// 注意：`dropped_frames` 计的是 **publish 侧** seq 跳跃（源到 watch 之间）；
/// 消费侧（watch coalesce 后）的丢帧由 `ipkvm-session::SessionStats` 另计。
pub struct SourceStats {
    inner: Mutex<SourceStatsInner>,
}

#[derive(Default)]
struct SourceStatsInner {
    published_frames: u64,
    last_seq: Option<u64>,
    dropped_frames: u64,
    convert_ns_total: u64,
    convert_count: u64,
    capture_wait_ns_total: u64,
    capture_wait_count: u64,
    last_capture_ns: Option<u64>,
}

impl SourceStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 记录一次 publish：累加帧数，检测 seq 跳跃累计丢帧，记采集时间。
    /// 首帧初始化基准（不计数）；seq 回退/重复视为重置（不计数）。
    pub fn record_publish(&self, seq: u64, capture_ns: u64) {
        let mut g = self.inner.lock().expect("SourceStats lock poisoned");
        match g.last_seq {
            Some(last) if seq > last + 1 => {
                g.dropped_frames = g.dropped_frames.saturating_add(seq - last - 1);
            }
            _ => {}
        }
        g.last_seq = Some(seq);
        g.published_frames = g.published_frames.saturating_add(1);
        g.last_capture_ns = Some(capture_ns);
    }

    /// 记录一次像素格式转换耗时。
    pub fn record_convert(&self, duration: std::time::Duration) {
        let mut g = self.inner.lock().expect("SourceStats lock poisoned");
        g.convert_ns_total = g
            .convert_ns_total
            .saturating_add(duration.as_nanos() as u64);
        g.convert_count = g.convert_count.saturating_add(1);
    }

    /// 记录一次采集等待耗时（阻塞拿帧的等待时长）。
    pub fn record_capture_wait(&self, duration: std::time::Duration) {
        let mut g = self.inner.lock().expect("SourceStats lock poisoned");
        g.capture_wait_ns_total = g
            .capture_wait_ns_total
            .saturating_add(duration.as_nanos() as u64);
        g.capture_wait_count = g.capture_wait_count.saturating_add(1);
    }

    /// 取只读快照。
    pub fn snapshot(&self) -> SourceStatsSnapshot {
        let g = self.inner.lock().expect("SourceStats lock poisoned");
        SourceStatsSnapshot {
            published_frames: g.published_frames,
            dropped_frames: g.dropped_frames,
            convert_ns_total: g.convert_ns_total,
            convert_count: g.convert_count,
            capture_wait_ns_total: g.capture_wait_ns_total,
            capture_wait_count: g.capture_wait_count,
            last_capture_ns: g.last_capture_ns,
        }
    }
}

impl Default for SourceStats {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SourceStatsInner::default()),
        }
    }
}

impl std::fmt::Debug for SourceStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceStats")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_format_records_dimensions_and_pixel_format() {
        let format = VideoFormat::new(1920, 1080, 60, PixelFormat::Mjpeg);

        assert_eq!(format.width, 1920);
        assert_eq!(format.height, 1080);
        assert_eq!(format.frames_per_second, 60);
        assert_eq!(format.pixel_format, PixelFormat::Mjpeg);
    }

    #[test]
    fn video_frame_records_explicit_bgra8888_layout() {
        let bytes: Arc<[u8]> = Arc::from(vec![1, 2, 3, 4].into_boxed_slice());

        let frame = VideoFrame::new(
            42,
            MonotonicTimestamp::from_nanos(1_000),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::clone(&bytes),
        );

        assert_eq!(frame.seq, 42);
        assert_eq!(frame.timestamp, MonotonicTimestamp::from_nanos(1_000));
        assert_eq!(frame.pixel_format, PixelFormat::Bgra8888);
        assert_eq!(frame.stride, 4);
        assert!(Arc::ptr_eq(&frame.data, &bytes));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn frame_sources_are_send_and_sync() {
        fn assert_send_sync<T: FrameSource + Send + Sync>() {}

        assert_send_sync::<crate::mock::MockFrameSource>();
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn mock_frame_source_shares_latest_frame_with_subscribers() {
        use crate::mock::MockFrameSource;

        let source = MockFrameSource::new();
        let receiver = source.subscribe();
        let frame = Arc::new(VideoFrame::new(
            7,
            MonotonicTimestamp::from_nanos(700),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from(vec![0, 0, 0, 255].into_boxed_slice()),
        ));

        source.publish_frame(Arc::clone(&frame));

        assert!(Arc::ptr_eq(&source.latest_frame().unwrap(), &frame));
        assert!(Arc::ptr_eq(receiver.borrow().as_ref().unwrap(), &frame));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn mock_frame_source_reports_generated_kind() {
        use crate::mock::MockFrameSource;

        let info = MockFrameSource::new().source_info();

        assert_eq!(info.kind, crate::VideoSourceKind::Generated);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn mock_frame_source_retains_frame_published_before_subscription() {
        use crate::mock::MockFrameSource;

        let source = MockFrameSource::new();
        let frame = Arc::new(VideoFrame::new(
            8,
            MonotonicTimestamp::from_nanos(800),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from(vec![0, 0, 0, 0].into_boxed_slice()),
        ));

        source.publish_frame(Arc::clone(&frame));
        let receiver = source.subscribe();

        assert!(Arc::ptr_eq(receiver.borrow().as_ref().unwrap(), &frame));
    }

    #[test]
    fn now_ns_is_monotonic_nondecreasing() {
        let a = now_ns();
        let b = now_ns();
        assert!(b >= a, "now_ns must be nondecreasing: a={a} b={b}");
    }

    #[test]
    fn monotonic_timestamp_now_matches_clock() {
        // now() 应贴近 now_ns()，且两者同源（差距在微秒级）。
        let ts = MonotonicTimestamp::now();
        let after = now_ns();
        assert!(
            ts.nanos <= after,
            "timestamp must precede later now_ns call"
        );
    }

    #[test]
    fn source_stats_counts_published_frames() {
        let stats = SourceStats::new();
        stats.record_publish(1, 100);
        stats.record_publish(2, 200);
        stats.record_publish(3, 300);

        let snap = stats.snapshot();
        assert_eq!(snap.published_frames, 3);
        assert_eq!(snap.dropped_frames, 0);
        assert_eq!(snap.last_capture_ns, Some(300));
    }

    #[test]
    fn source_stats_counts_seq_jumps_as_dropped() {
        let stats = SourceStats::new();
        stats.record_publish(1, 10);
        // seq 2,3,4 丢失 → 3 帧丢帧
        stats.record_publish(5, 50);

        let snap = stats.snapshot();
        assert_eq!(snap.published_frames, 2);
        assert_eq!(snap.dropped_frames, 3);
    }

    #[test]
    fn source_stats_first_frame_does_not_count_as_dropped() {
        let stats = SourceStats::new();
        // 首帧 seq=10，基准初始化，不计丢帧
        stats.record_publish(10, 1);

        let snap = stats.snapshot();
        assert_eq!(snap.published_frames, 1);
        assert_eq!(snap.dropped_frames, 0);
    }

    #[test]
    fn source_stats_seq_reset_does_not_count_as_dropped() {
        let stats = SourceStats::new();
        stats.record_publish(5, 1);
        // seq 回退视为重置，不计丢帧
        stats.record_publish(1, 2);

        let snap = stats.snapshot();
        assert_eq!(snap.published_frames, 2);
        assert_eq!(snap.dropped_frames, 0);
    }

    #[test]
    fn source_stats_accumulates_convert_duration() {
        let stats = SourceStats::new();
        stats.record_convert(std::time::Duration::from_nanos(1_500));
        stats.record_convert(std::time::Duration::from_nanos(2_500));

        let snap = stats.snapshot();
        assert_eq!(snap.convert_count, 2);
        assert!(
            snap.convert_ns_total >= 4_000,
            "total: {}",
            snap.convert_ns_total
        );
    }

    #[test]
    fn source_stats_accumulates_capture_wait_duration() {
        let stats = SourceStats::new();
        stats.record_capture_wait(std::time::Duration::from_nanos(1_000));
        stats.record_capture_wait(std::time::Duration::from_nanos(3_000));

        let snap = stats.snapshot();
        assert_eq!(snap.capture_wait_count, 2);
        assert!(
            snap.capture_wait_ns_total >= 4_000,
            "total: {}",
            snap.capture_wait_ns_total
        );
    }

    #[test]
    fn source_stats_default_snapshot_is_empty() {
        let stats = SourceStats::new();
        let snap = stats.snapshot();
        assert_eq!(snap, SourceStatsSnapshot::default());
    }
}
