//! 按顺序循环播放多个 Y4M 素材的演示帧源（mock 功能）。
//!
//! 素材尺寸可以不同：切换素材时发布的 `VideoFrame` 尺寸随之变化，
//! 下游 RFB 连接会据此触发 `DesktopSize` 通知。

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::watch;

use crate::{
    FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, SharedVideoFrame, SourceStats,
    SourceStatsSnapshot, VideoFrame, VideoSourceInfo, VideoSourceKind, now_ns, y4m::Y4mAsset,
};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum LoopingSourceError {
    #[error("looping video source requires at least one asset")]
    EmptyAssets,
    #[error("frames per second must be non-zero")]
    ZeroFramesPerSecond,
}

#[derive(Debug)]
pub struct LoopingVideoSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    stats: Arc<SourceStats>,
}

impl LoopingVideoSource {
    pub fn new(assets: Vec<Y4mAsset>, frames_per_second: u64) -> Result<Self, LoopingSourceError> {
        Self::new_with_dirty_rects(assets, frames_per_second, None)
    }

    /// 创建循环播放帧源，可选开启 dirty rects 检测（FU-4，issue #35）。
    pub fn new_with_dirty_rects(
        assets: Vec<Y4mAsset>,
        frames_per_second: u64,
        dirty_rect_tile_size: Option<u32>,
    ) -> Result<Self, LoopingSourceError> {
        if assets.is_empty() {
            return Err(LoopingSourceError::EmptyAssets);
        }
        if frames_per_second == 0 {
            return Err(LoopingSourceError::ZeroFramesPerSecond);
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
            // BGRA 输出 buffer 复用：消除每帧 Vec 分配（调研阶段 1.2，#19）。
            let mut bgra_buf: Vec<u8> = Vec::new();
            let mut detector = dirty_rect_tile_size.map(crate::dirty_rects::DirtyRectDetector::new);
            loop {
                for asset in &assets {
                    for index in 0..asset.frame_count() {
                        let convert_start = Instant::now();
                        let Some(()) = asset.frame_bgra_into(index, &mut bgra_buf) else {
                            continue;
                        };
                        task_stats.record_convert(convert_start.elapsed());
                        let data: Arc<[u8]> =
                            Arc::from(std::mem::take(&mut bgra_buf).into_boxed_slice());
                        let capture_ns = now_ns();
                        seq = seq.saturating_add(1);
                        let mut frame = VideoFrame::new(
                            seq,
                            MonotonicTimestamp::from_nanos(capture_ns),
                            asset.width(),
                            asset.height(),
                            asset.width() * 4,
                            PixelFormat::Bgra8888,
                            data,
                        );
                        if let Some(d) = &mut detector {
                            frame.dirty_rects = Some(d.detect(&frame));
                        }
                        let shared = Arc::new(frame);
                        task_stats.record_publish(seq, capture_ns);
                        *task_latest
                            .write()
                            .expect("looping video source lock poisoned") =
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

impl FrameSource for LoopingVideoSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.latest
            .read()
            .expect("looping video source lock poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    fn subscribe(&self) -> FrameReceiver {
        self.sender.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::Generated,
            device_name: "looping y4m".into(),
            is_loop: true,
        }
    }

    fn source_stats(&self) -> Option<SourceStatsSnapshot> {
        Some(self.stats.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::LoopingSourceError;
    use crate::FrameSource;
    use crate::looping::LoopingVideoSource;
    use crate::y4m::Y4mAsset;

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

    async fn observed_sizes(source: &LoopingVideoSource, limit: usize) -> Vec<(u32, u32)> {
        let mut receiver = source.subscribe();
        let mut sizes = Vec::new();
        while sizes.len() < limit {
            if timeout(Duration::from_secs(5), receiver.changed())
                .await
                .unwrap()
                .is_err()
            {
                break;
            }
            let frame = receiver.borrow().clone().unwrap();
            let size = (frame.width, frame.height);
            if sizes.last() != Some(&size) {
                sizes.push(size);
            }
        }
        sizes
    }

    #[tokio::test]
    async fn publishes_both_resolutions_and_loops_back() {
        let small = asset(4, 2, 0, 8);
        let large = asset(2, 4, 255, 8);
        let source = LoopingVideoSource::new(vec![small, large], 1_000).unwrap();

        let sizes = observed_sizes(&source, 5).await;

        assert!(sizes.len() >= 3, "应观察到多次切换，实际 {sizes:?}");
        assert!(sizes[0] != sizes[1], "相邻尺寸不应相同：{sizes:?}");
        assert!(sizes[1] != sizes[2], "相邻尺寸不应相同：{sizes:?}");
        assert!(sizes.contains(&(4, 2)), "缺少 4x2：{sizes:?}");
        assert!(sizes.contains(&(2, 4)), "缺少 2x4：{sizes:?}");
    }

    #[tokio::test]
    async fn published_frames_are_bgra_with_monotonic_sequence() {
        let source =
            LoopingVideoSource::new(vec![asset(4, 2, 0, 8), asset(2, 4, 255, 8)], 1_000).unwrap();
        let mut receiver = source.subscribe();
        let mut previous_seq = 0;

        for _ in 0..5 {
            timeout(Duration::from_secs(5), receiver.changed())
                .await
                .unwrap()
                .unwrap();
            let frame = receiver.borrow().clone().unwrap();
            assert!(frame.seq > previous_seq);
            assert_eq!(frame.pixel_format, crate::PixelFormat::Bgra8888);
            assert_eq!(frame.stride, frame.width * 4);
            assert_eq!(frame.data.len(), (frame.width * frame.height * 4) as usize);
            assert_eq!(frame.data[3], 255);
            previous_seq = frame.seq;
        }
    }

    #[test]
    fn rejects_empty_assets_and_zero_fps() {
        assert!(matches!(
            LoopingVideoSource::new(Vec::new(), 10),
            Err(LoopingSourceError::EmptyAssets)
        ));
        assert!(matches!(
            LoopingVideoSource::new(vec![asset(2, 2, 0, 1)], 0),
            Err(LoopingSourceError::ZeroFramesPerSecond)
        ));
    }
}
