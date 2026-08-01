use std::sync::{Arc, RwLock};

use tokio::sync::watch;

use crate::{FrameReceiver, FrameSource, SharedVideoFrame, VideoSourceInfo, VideoSourceKind};

#[derive(Debug)]
pub struct MockFrameSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
}

impl MockFrameSource {
    pub fn new() -> Self {
        let (sender, _receiver) = watch::channel(None);
        Self {
            latest: Arc::new(RwLock::new(None)),
            sender,
        }
    }

    pub fn publish_frame(&self, frame: SharedVideoFrame) {
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
}
