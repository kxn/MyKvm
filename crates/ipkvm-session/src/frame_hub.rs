use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU64, Ordering},
};

use ipkvm_video::{
    FrameReceiver, FrameSource, SharedVideoFrame, SourceStatsSnapshot, VideoFrame, VideoSourceInfo,
    VideoSourceKind,
};
use tokio::sync::watch;

/// 稳定的视频帧入口。
///
/// 上层长期订阅 `FrameHub` 自己的 watch channel；底层可以替换真实
/// [`FrameSource`]，旧订阅不会因为源替换而关闭。
#[derive(Clone)]
pub struct FrameHub {
    sender: watch::Sender<Option<SharedVideoFrame>>,
    current: Arc<RwLock<Option<Arc<dyn FrameSource>>>>,
    generation: Arc<AtomicU64>,
    generation_tx: watch::Sender<u64>,
    next_frame_seq: Arc<AtomicU64>,
}

/// 单个真实帧源到 [`FrameHub`] 的转发任务。
pub struct FrameHubForwarder {
    hub: FrameHub,
    source: Arc<dyn FrameSource>,
    generation: u64,
    generation_rx: watch::Receiver<u64>,
    last_source_seq: Option<u64>,
}

impl FrameHub {
    pub fn new_empty() -> Self {
        let (sender, _receiver) = watch::channel(None);
        let (generation_tx, _generation_rx) = watch::channel(0);
        Self {
            sender,
            current: Arc::new(RwLock::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            generation_tx,
            next_frame_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn set_source<S>(&self, source: Arc<S>) -> FrameHubForwarder
    where
        S: FrameSource + 'static,
    {
        let source: Arc<dyn FrameSource> = source;
        self.set_dyn_source(source)
    }

    pub fn set_dyn_source(&self, source: Arc<dyn FrameSource>) -> FrameHubForwarder {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self
            .current
            .write()
            .expect("frame hub source lock poisoned") = Some(Arc::clone(&source));
        self.generation_tx.send_replace(generation);
        let last_source_seq = source.latest_frame().map(|frame| {
            let seq = frame.seq;
            self.publish_source_frame(frame);
            seq
        });
        FrameHubForwarder {
            hub: self.clone(),
            source,
            generation,
            generation_rx: self.generation_tx.subscribe(),
            last_source_seq,
        }
    }

    pub fn clear(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let had_source = self
            .current
            .write()
            .expect("frame hub source lock poisoned")
            .take()
            .is_some();
        self.generation_tx.send_replace(generation);
        if had_source || self.sender.borrow().is_some() {
            self.sender.send_replace(None);
        }
    }

    fn is_current_generation(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    fn publish_source_frame(&self, frame: SharedVideoFrame) {
        let seq = self.next_frame_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut hub_frame: VideoFrame = (*frame).clone();
        hub_frame.seq = seq;
        self.sender.send_replace(Some(Arc::new(hub_frame)));
    }
}

impl FrameHubForwarder {
    pub async fn run(mut self) {
        let mut source_rx = self.source.subscribe();
        if let Some(frame) = source_rx.borrow().clone()
            && self.last_source_seq != Some(frame.seq)
        {
            self.last_source_seq = Some(frame.seq);
            self.hub.publish_source_frame(frame);
        }
        loop {
            tokio::select! {
                changed = source_rx.changed() => {
                    if changed.is_err() || !self.hub.is_current_generation(self.generation) {
                        return;
                    }
                    match source_rx.borrow().clone() {
                        Some(frame) if self.last_source_seq != Some(frame.seq) => {
                            self.last_source_seq = Some(frame.seq);
                            self.hub.publish_source_frame(frame);
                        }
                        Some(_) => {}
                        None => {
                            self.last_source_seq = None;
                            self.hub.sender.send_replace(None);
                        }
                    }
                }
                changed = self.generation_rx.changed() => {
                    if changed.is_err() || *self.generation_rx.borrow() != self.generation {
                        return;
                    }
                }
            }
        }
    }
}

impl FrameSource for FrameHub {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.sender.borrow().clone()
    }

    fn subscribe(&self) -> FrameReceiver {
        self.sender.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        self.current
            .read()
            .expect("frame hub source lock poisoned")
            .as_ref()
            .map(|source| source.source_info())
            .unwrap_or_else(|| VideoSourceInfo {
                kind: VideoSourceKind::None,
                device_name: "none".to_string(),
                is_loop: false,
            })
    }

    fn source_stats(&self) -> Option<SourceStatsSnapshot> {
        self.current
            .read()
            .expect("frame hub source lock poisoned")
            .as_ref()
            .and_then(|source| source.source_stats())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipkvm_video::{
        FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource,
    };

    use super::FrameHub;

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

    #[tokio::test]
    async fn subscribers_survive_source_replacement() {
        let hub = FrameHub::new_empty();
        let mut rx = hub.subscribe();

        let first = Arc::new(MockFrameSource::new());
        let forwarder = hub.set_source(first.clone());
        let task = tokio::spawn(forwarder.run());
        first.publish_frame(frame(1, 2));
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().as_ref().unwrap().width, 2);
        task.abort();

        let second = Arc::new(MockFrameSource::new());
        let forwarder = hub.set_source(second.clone());
        let task = tokio::spawn(forwarder.run());
        second.publish_frame(frame(2, 4));
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().as_ref().unwrap().width, 4);
        task.abort();
    }

    #[tokio::test]
    async fn hub_frame_sequence_stays_monotonic_when_sources_reset() {
        let hub = FrameHub::new_empty();
        let mut rx = hub.subscribe();

        let first = Arc::new(MockFrameSource::new());
        let forwarder = hub.set_source(first.clone());
        let first_task = tokio::spawn(forwarder.run());
        first.publish_frame(frame(10, 2));
        rx.changed().await.unwrap();
        let first_seq = rx.borrow().as_ref().unwrap().seq;

        let second = Arc::new(MockFrameSource::new());
        let forwarder = hub.set_source(second.clone());
        let second_task = tokio::spawn(forwarder.run());
        second.publish_frame(frame(1, 4));
        rx.changed().await.unwrap();
        let second_seq = rx.borrow().as_ref().unwrap().seq;

        assert!(
            second_seq > first_seq,
            "source-local seq reset must not regress the stable hub stream"
        );
        first_task.abort();
        second_task.abort();
    }

    #[test]
    fn clear_publishes_none_without_closing_subscription() {
        let hub = FrameHub::new_empty();
        let rx = hub.subscribe();

        hub.clear();

        assert!(rx.borrow().is_none());
        assert!(rx.has_changed().is_ok());
    }
}
