//! 视频采集抽象。

#[cfg(feature = "mf")]
pub mod camera;
#[cfg(feature = "mock")]
pub mod file_source;
#[cfg(feature = "mock")]
pub mod looping;
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "mock")]
pub mod y4m;

use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Yuy2,
    Nv12,
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
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoSourceKind {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MonotonicTimestamp {
    pub nanos: u64,
}

impl MonotonicTimestamp {
    pub fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
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

    #[cfg(feature = "mock")]
    #[test]
    fn frame_sources_are_send_and_sync() {
        fn assert_send_sync<T: FrameSource + Send + Sync>() {}

        assert_send_sync::<crate::mock::MockFrameSource>();
    }

    #[cfg(feature = "mock")]
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

    #[cfg(feature = "mock")]
    #[test]
    fn mock_frame_source_reports_generated_kind() {
        use crate::mock::MockFrameSource;

        let info = MockFrameSource::new().source_info();

        assert_eq!(info.kind, crate::VideoSourceKind::Generated);
    }

    #[cfg(feature = "mock")]
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
}
