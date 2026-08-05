//! 视频文件伪设备：把视频文件包装成 `FrameSource`，内部自动循环播放。

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::watch;

use crate::{
    FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, SharedVideoFrame, SourceStats,
    SourceStatsSnapshot, VideoFrame, VideoSourceInfo, VideoSourceKind, now_ns, y4m::Y4mAsset,
};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum FileSourceError {
    #[error("file video source requires at least one asset")]
    EmptyAssets,
    #[error("frames per second must be non-zero")]
    ZeroFramesPerSecond,
}

#[derive(Debug)]
pub struct FileVideoSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    stats: Arc<SourceStats>,
}

impl FileVideoSource {
    pub fn new(assets: Vec<Y4mAsset>, frames_per_second: u64) -> Result<Self, FileSourceError> {
        if assets.is_empty() {
            return Err(FileSourceError::EmptyAssets);
        }
        if frames_per_second == 0 {
            return Err(FileSourceError::ZeroFramesPerSecond);
        }

        let (sender, _receiver) = watch::channel(None);
        let latest = Arc::new(RwLock::new(None));
        let stats = SourceStats::new();
        let task_latest = Arc::clone(&latest);
        let task_sender = sender.clone();
        let task_stats = Arc::clone(&stats);

        tokio::spawn(async move {
            let interval = Duration::from_nanos((1_000_000_000 / frames_per_second).max(1));
            let mut seq = 0_u64;
            loop {
                for asset in &assets {
                    for index in 0..asset.frame_count() {
                        let convert_start = Instant::now();
                        let Some(pixels) = asset.frame_bgra(index) else {
                            continue;
                        };
                        task_stats.record_convert(convert_start.elapsed());
                        let capture_ns = now_ns();
                        seq = seq.saturating_add(1);
                        let frame = VideoFrame::new(
                            seq,
                            MonotonicTimestamp::from_nanos(capture_ns),
                            asset.width(),
                            asset.height(),
                            asset.width() * 4,
                            PixelFormat::Bgra8888,
                            Arc::from(pixels.into_boxed_slice()),
                        );
                        let shared = Arc::new(frame);
                        task_stats.record_publish(seq, capture_ns);
                        *task_latest.write().expect("file source lock poisoned") =
                            Some(Arc::clone(&shared));
                        task_sender.send_replace(Some(shared));
                        tokio::time::sleep(interval).await;
                    }
                }
            }
        });

        Ok(Self {
            latest,
            sender,
            stats,
        })
    }
}

impl FrameSource for FileVideoSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.latest
            .read()
            .expect("file source lock poisoned")
            .as_ref()
            .map(Arc::clone)
    }
    fn subscribe(&self) -> FrameReceiver {
        self.sender.subscribe()
    }
    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::VideoFile,
            device_name: "video file".into(),
            is_loop: true,
        }
    }
    fn source_stats(&self) -> Option<SourceStatsSnapshot> {
        Some(self.stats.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FrameSource;
    use crate::y4m::Y4mAsset;
    use std::time::Duration;
    use tokio::time::timeout;

    fn asset(width: u32, height: u32, luminance: u8, frame_count: usize) -> Y4mAsset {
        let y_len = (width * height) as usize;
        let uv_len = (width.div_ceil(2) * height.div_ceil(2)) as usize;
        let mut bytes = format!("YUV4MPEG2 W{width} H{height} F10:1 Ip A1:1 C420\n").into_bytes();
        for _ in 0..frame_count {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend(std::iter::repeat_n(luminance, y_len));
            bytes.extend(std::iter::repeat_n(128, 2 * uv_len));
        }
        Y4mAsset::parse(&bytes).unwrap()
    }

    #[tokio::test]
    async fn file_source_reports_video_file_kind_and_is_loop() {
        let source = FileVideoSource::new(vec![asset(4, 2, 0, 2)], 1_000).unwrap();
        let info = source.source_info();
        assert_eq!(info.kind, crate::VideoSourceKind::VideoFile);
        assert!(info.is_loop);
        assert_eq!(info.device_name, "video file");
    }

    #[tokio::test]
    async fn file_source_loops_and_publishes_bgra_frames() {
        let source = FileVideoSource::new(vec![asset(4, 2, 0, 2)], 1_000).unwrap();
        let mut receiver = source.subscribe();
        let mut seen = 0;
        while seen < 5 {
            if timeout(Duration::from_secs(5), receiver.changed())
                .await
                .unwrap()
                .is_err()
            {
                break;
            }
            let frame = receiver.borrow().clone().unwrap();
            assert_eq!(frame.pixel_format, crate::PixelFormat::Bgra8888);
            assert_eq!(frame.stride, frame.width * 4);
            assert_eq!(frame.data.len(), (frame.width * frame.height * 4) as usize);
            seen += 1;
        }
        assert!(seen >= 5, "file source should loop, saw {seen} frames");
    }

    #[test]
    fn rejects_empty_assets_and_zero_fps() {
        assert!(matches!(
            FileVideoSource::new(Vec::new(), 10),
            Err(FileSourceError::EmptyAssets)
        ));
        assert!(matches!(
            FileVideoSource::new(vec![asset(2, 2, 0, 1)], 0),
            Err(FileSourceError::ZeroFramesPerSecond)
        ));
    }
}
