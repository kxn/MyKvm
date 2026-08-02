//! headless 当前帧源句柄：运行时会话切换后，新连接读取新的帧源。

use std::sync::{Arc, RwLock};

use ipkvm_video::{FrameReceiver, FrameSource, SharedVideoFrame, VideoSourceInfo};
use ipkvm_video::{VideoSourceKind, mock::MockFrameSource};

/// 可切换当前帧源。
///
/// 本类型不承诺迁移旧订阅：已经建立的 RFB 连接继续持有旧帧源订阅；会话切换
/// 后，新截图、新 status 和新 RFB 连接会读取新帧源。
#[derive(Clone)]
pub struct SwitchableFrameSource {
    current: Arc<RwLock<Arc<dyn FrameSource>>>,
}

impl SwitchableFrameSource {
    pub fn new(source: Arc<dyn FrameSource>) -> Self {
        Self {
            current: Arc::new(RwLock::new(source)),
        }
    }

    pub fn set_current(&self, source: Arc<dyn FrameSource>) {
        *self.current.write().expect("frame source lock poisoned") = source;
    }

    pub fn current(&self) -> Arc<dyn FrameSource> {
        Arc::clone(&self.current.read().expect("frame source lock poisoned"))
    }
}

impl FrameSource for SwitchableFrameSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.current().latest_frame()
    }

    fn subscribe(&self) -> FrameReceiver {
        self.current().subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        self.current().source_info()
    }
}

/// 空帧源：会话重建期间用于释放旧独占设备。
#[derive(Debug, Default)]
pub struct EmptyFrameSource {
    inner: MockFrameSource,
}

impl EmptyFrameSource {
    pub fn new() -> Self {
        Self::default()
    }
}

impl FrameSource for EmptyFrameSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        None
    }

    fn subscribe(&self) -> FrameReceiver {
        self.inner.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::Generated,
            device_name: "none".to_string(),
            is_loop: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipkvm_video::{
        FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource,
    };

    use super::*;

    fn frame(seq: u64, width: u32) -> Arc<VideoFrame> {
        Arc::new(VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            width,
            1,
            width * 4,
            PixelFormat::Bgra8888,
            Arc::from(vec![0; (width * 4) as usize].into_boxed_slice()),
        ))
    }

    #[test]
    fn switching_changes_latest_frame_and_new_subscribers() {
        let first = Arc::new(MockFrameSource::new());
        first.publish_frame(frame(1, 2));
        let switchable = SwitchableFrameSource::new(first);
        assert_eq!(switchable.latest_frame().unwrap().width, 2);

        let second = Arc::new(MockFrameSource::new());
        second.publish_frame(frame(2, 4));
        switchable.set_current(second);

        assert_eq!(switchable.latest_frame().unwrap().width, 4);
        assert_eq!(switchable.subscribe().borrow().as_ref().unwrap().seq, 2);
    }
}
