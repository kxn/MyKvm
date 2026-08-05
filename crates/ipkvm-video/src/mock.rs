use std::sync::{Arc, RwLock};

use tokio::sync::watch;

use crate::{
    FrameReceiver, FrameSource, SharedVideoFrame, SourceStats, SourceStatsSnapshot,
    VideoSourceInfo, VideoSourceKind,
};

#[derive(Debug)]
pub struct MockFrameSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    stats: Arc<SourceStats>,
}

impl MockFrameSource {
    pub fn new() -> Self {
        let (sender, _receiver) = watch::channel(None);
        Self {
            latest: Arc::new(RwLock::new(None)),
            sender,
            stats: SourceStats::new(),
        }
    }

    pub fn publish_frame(&self, frame: SharedVideoFrame) {
        self.stats.record_publish(frame.seq, frame.timestamp.nanos);
        *self.latest.write().expect("mock frame lock poisoned") = Some(Arc::clone(&frame));
        self.sender.send_replace(Some(frame));
    }
}

impl Default for MockFrameSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSource for MockFrameSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.latest
            .read()
            .expect("mock frame lock poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    fn subscribe(&self) -> FrameReceiver {
        self.sender.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::Generated,
            device_name: "mock".into(),
            is_loop: false,
        }
    }

    fn source_stats(&self) -> Option<SourceStatsSnapshot> {
        Some(self.stats.snapshot())
    }
}
