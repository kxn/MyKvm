use std::sync::Arc;
use std::time::Instant;

use ipkvm_core::InputSink;
use ipkvm_video::{FrameReceiver, FrameSource, SharedVideoFrame};
use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::frame_hub::FrameHub;
use crate::rfb_connection::{RfbConnectionGate, RfbServerEvent};
use crate::rfb_input::RfbInputNotice;
use crate::session_manager::{SessionManager, SessionState};

use super::{
    ControlRuntimeStatus, RecoveryPolicy, RuntimePhase, SessionIntent, SupervisorStatus,
    VideoRuntimeStatus,
};

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("session manager failed: {0}")]
    Session(#[from] crate::console_session::SessionError),
}

pub struct SessionSupervisor<S: InputSink + Clone + Send + 'static> {
    frame_hub: FrameHub,
    manager: SessionManager<S>,
    gate: RfbConnectionGate,
    policy: RecoveryPolicy,
    status_tx: watch::Sender<SupervisorStatus>,
    video_task: Option<tokio::task::JoinHandle<()>>,
    video_started_at: Option<Instant>,
    video_last_frame: Option<(u64, Instant)>,
    video_attempts: u32,
    video_next_retry: Option<Instant>,
    video_last_error: Option<String>,
    control_attempts: u32,
    control_next_retry: Option<Instant>,
    control_last_error: Option<String>,
}

impl<S: InputSink + Clone + Send + 'static> SessionSupervisor<S> {
    pub fn new(gate: RfbConnectionGate, policy: RecoveryPolicy) -> Self {
        let frame_hub = FrameHub::new_empty();
        let manager = SessionManager::empty();
        let (status_tx, _) = watch::channel(SupervisorStatus::default());
        Self {
            frame_hub,
            manager,
            gate,
            policy,
            status_tx,
            video_task: None,
            video_started_at: None,
            video_last_frame: None,
            video_attempts: 0,
            video_next_retry: None,
            video_last_error: None,
            control_attempts: 0,
            control_next_retry: None,
            control_last_error: None,
        }
    }

    pub fn frame_source(&self) -> FrameHub {
        self.frame_hub.clone()
    }

    pub fn subscribe_frames(&self) -> FrameReceiver {
        self.frame_hub.subscribe()
    }

    pub fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.frame_hub.latest_frame()
    }

    pub fn status(&self) -> SupervisorStatus {
        self.status_tx.borrow().clone()
    }

    pub fn status_receiver(&self) -> watch::Receiver<SupervisorStatus> {
        self.status_tx.subscribe()
    }

    pub fn event_publisher(&self) -> watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>> {
        self.manager.event_publisher()
    }

    pub fn set_notice_mirror(
        &mut self,
        notice_mirror: Option<mpsc::UnboundedSender<RfbInputNotice>>,
    ) {
        self.manager.set_notice_mirror(notice_mirror);
    }

    pub fn is_control_ready(&self) -> bool {
        matches!(self.status().control, ControlRuntimeStatus::Ready)
    }

    pub fn input_offline_reason(&self) -> Option<String> {
        self.manager
            .session()
            .and_then(|session| session.stats().input_offline.clone())
            .map(|info| info.reason)
            .or_else(|| self.control_last_error.clone())
    }

    pub fn manager(&self) -> &SessionManager<S> {
        &self.manager
    }

    pub fn manager_mut(&mut self) -> &mut SessionManager<S> {
        &mut self.manager
    }

    pub async fn start(
        &mut self,
        open_video: impl FnMut() -> Result<Arc<dyn FrameSource>, String>,
        open_control: impl FnMut() -> Result<S, String>,
    ) {
        let now = Instant::now();
        self.start_at(open_video, open_control, now).await;
    }

    pub async fn start_at(
        &mut self,
        mut open_video: impl FnMut() -> Result<Arc<dyn FrameSource>, String>,
        mut open_control: impl FnMut() -> Result<S, String>,
        now: Instant,
    ) {
        self.stop_video(true).await;
        let _ = self.manager.stop_and_destroy().await;
        self.video_attempts = 0;
        self.video_next_retry = None;
        self.video_last_error = None;
        self.video_last_frame = None;
        self.control_attempts = 0;
        self.control_next_retry = None;
        self.control_last_error = None;
        self.publish_status(SupervisorStatus::new(
            SessionIntent::Recovering,
            VideoRuntimeStatus::Starting { attempt: 0 },
            ControlRuntimeStatus::Starting { attempt: 0 },
        ));
        self.try_open_video(&mut open_video, now).await;
        self.try_open_control(&mut open_control, now).await;
        self.refresh_status(now);
    }

    pub async fn stop_manual(&mut self) -> Result<(), SupervisorError> {
        self.stop_video(true).await;
        self.manager.stop_and_destroy().await?;
        self.video_attempts = 0;
        self.video_next_retry = None;
        self.video_last_error = None;
        self.video_last_frame = None;
        self.control_attempts = 0;
        self.control_next_retry = None;
        self.control_last_error = None;
        self.publish_status(SupervisorStatus::new(
            SessionIntent::ManualStopped,
            VideoRuntimeStatus::Idle,
            ControlRuntimeStatus::Idle,
        ));
        Ok(())
    }

    pub async fn tick(
        &mut self,
        open_video: impl FnMut() -> Result<Arc<dyn FrameSource>, String>,
        open_control: impl FnMut() -> Result<S, String>,
    ) {
        self.tick_at(open_video, open_control, Instant::now()).await;
    }

    pub async fn tick_at(
        &mut self,
        mut open_video: impl FnMut() -> Result<Arc<dyn FrameSource>, String>,
        mut open_control: impl FnMut() -> Result<S, String>,
        now: Instant,
    ) {
        if matches!(self.status().intent, SessionIntent::ManualStopped) {
            return;
        }
        self.refresh_video_state(now).await;
        self.refresh_control_state(now).await;
        if self.video_retry_due(now) {
            self.try_open_video(&mut open_video, now).await;
        }
        if self.control_retry_due(now) {
            self.try_open_control(&mut open_control, now).await;
        }
        self.refresh_status(now);
    }

    async fn try_open_video(
        &mut self,
        open_video: &mut impl FnMut() -> Result<Arc<dyn FrameSource>, String>,
        now: Instant,
    ) {
        self.stop_video(true).await;
        self.publish_status(SupervisorStatus::new(
            self.status().intent,
            VideoRuntimeStatus::Starting {
                attempt: self.video_attempts,
            },
            self.status().control,
        ));
        match open_video() {
            Ok(source) => {
                let forwarder = self.frame_hub.set_dyn_source(source);
                self.video_task = Some(tokio::spawn(forwarder.run()));
                self.video_started_at = Some(now);
                self.video_next_retry = None;
                self.video_last_error = None;
                if let Some(frame) = self.frame_hub.latest_frame() {
                    self.video_last_frame = Some((frame.seq, now));
                }
            }
            Err(reason) => {
                self.schedule_video_retry(reason, now).await;
            }
        }
    }

    async fn try_open_control(
        &mut self,
        open_control: &mut impl FnMut() -> Result<S, String>,
        now: Instant,
    ) {
        let _ = self.manager.stop_and_destroy().await;
        self.publish_status(SupervisorStatus::new(
            self.status().intent,
            self.status().video,
            ControlRuntimeStatus::Starting {
                attempt: self.control_attempts,
            },
        ));
        match open_control() {
            Ok(sink) => {
                let hub_source: Arc<dyn FrameSource> = Arc::new(self.frame_hub.clone());
                match self
                    .manager
                    .replace_and_start(hub_source, sink, self.gate.clone())
                    .await
                {
                    Ok(()) => {
                        self.control_next_retry = None;
                        self.control_last_error = None;
                    }
                    Err(error) => {
                        self.schedule_control_retry(error.to_string(), now);
                    }
                }
            }
            Err(reason) => {
                self.schedule_control_retry(reason, now);
            }
        }
    }

    async fn stop_video(&mut self, clear_hub: bool) {
        if let Some(task) = self.video_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.video_started_at = None;
        if clear_hub {
            self.frame_hub.clear();
        }
    }

    async fn refresh_video_state(&mut self, now: Instant) {
        if self.video_next_retry.is_some() || self.video_last_error.is_some() {
            return;
        }
        if self
            .video_task
            .as_ref()
            .is_some_and(|task| task.is_finished())
        {
            self.schedule_video_retry("video source stopped".to_string(), now)
                .await;
            return;
        }

        let Some(frame) = self.frame_hub.latest_frame() else {
            if self.video_started_at.is_some_and(|started| {
                now.duration_since(started) >= self.policy.video_start_timeout
            }) {
                self.schedule_video_retry("video source produced no frames".to_string(), now)
                    .await;
            }
            return;
        };
        match self.video_last_frame {
            Some((seq, observed_at)) if frame.seq == seq => {
                if now.duration_since(observed_at) >= self.policy.video_start_timeout {
                    self.schedule_video_retry("video source stalled".to_string(), now)
                        .await;
                }
            }
            _ => {
                self.video_last_frame = Some((frame.seq, now));
                self.video_attempts = 0;
                self.video_next_retry = None;
                self.video_last_error = None;
            }
        }
    }

    async fn refresh_control_state(&mut self, now: Instant) {
        self.manager.refresh_stats();
        if self.manager.state() == SessionState::Stopped {
            let reason = self
                .manager
                .session()
                .and_then(|session| session.stats().input_offline.clone())
                .map(|info| info.reason);
            if let Some(reason) = reason {
                let _ = self.manager.stop_and_destroy().await;
                self.schedule_control_retry(reason, now);
            }
        } else if self.manager.state() == SessionState::Running {
            self.control_attempts = 0;
            self.control_next_retry = None;
            self.control_last_error = None;
        }
    }

    fn video_retry_due(&self, now: Instant) -> bool {
        self.video_next_retry.is_some_and(|retry| now >= retry)
    }

    fn control_retry_due(&self, now: Instant) -> bool {
        self.control_next_retry.is_some_and(|retry| now >= retry)
    }

    async fn schedule_video_retry(&mut self, reason: String, now: Instant) {
        self.stop_video(true).await;
        self.video_last_error = Some(reason);
        self.video_next_retry = self
            .policy
            .next_delay(self.video_attempts)
            .map(|delay| now + delay);
        self.video_attempts = self.video_attempts.saturating_add(1);
    }

    fn schedule_control_retry(&mut self, reason: String, now: Instant) {
        self.control_last_error = Some(reason);
        self.control_next_retry = self
            .policy
            .next_delay(self.control_attempts)
            .map(|delay| now + delay);
        self.control_attempts = self.control_attempts.saturating_add(1);
    }

    fn refresh_status(&mut self, now: Instant) {
        let video = if self.video_next_retry.is_some() {
            VideoRuntimeStatus::Recovering {
                reason: self.video_last_error.clone().unwrap_or_default(),
                attempt: self.video_attempts,
            }
        } else if self.video_last_error.is_some() {
            VideoRuntimeStatus::Failed {
                reason: self.video_last_error.clone().unwrap_or_default(),
                attempts: self.video_attempts,
            }
        } else if let Some((_, observed_at)) = self.video_last_frame {
            if now.duration_since(observed_at) >= self.policy.video_start_timeout {
                VideoRuntimeStatus::Stalled {
                    reason: "video source stalled".to_string(),
                }
            } else {
                VideoRuntimeStatus::Streaming
            }
        } else if self.video_started_at.is_some() {
            VideoRuntimeStatus::Starting {
                attempt: self.video_attempts,
            }
        } else {
            VideoRuntimeStatus::Idle
        };

        let control = if self.control_next_retry.is_some() {
            ControlRuntimeStatus::Recovering {
                reason: self.control_last_error.clone().unwrap_or_default(),
                attempt: self.control_attempts,
            }
        } else if self.control_last_error.is_some() {
            ControlRuntimeStatus::Failed {
                reason: self.control_last_error.clone().unwrap_or_default(),
                attempts: self.control_attempts,
            }
        } else if self.manager.state() == SessionState::Running {
            ControlRuntimeStatus::Ready
        } else {
            ControlRuntimeStatus::Idle
        };

        let intent = match (video.phase(), control.phase()) {
            (RuntimePhase::Idle, RuntimePhase::Idle) => self.status().intent,
            (RuntimePhase::Failed, RuntimePhase::Failed) => SessionIntent::Failed,
            (RuntimePhase::Failed, _) | (_, RuntimePhase::Failed) => SessionIntent::Failed,
            (RuntimePhase::Recovering, _) | (_, RuntimePhase::Recovering) => {
                SessionIntent::Recovering
            }
            (RuntimePhase::Starting, _) | (_, RuntimePhase::Starting) => SessionIntent::Recovering,
            _ => SessionIntent::Running,
        };
        self.publish_status(SupervisorStatus::new(intent, video, control));
    }

    fn publish_status(&self, status: SupervisorStatus) {
        self.status_tx.send_replace(status);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::{Duration, Instant};

    use ipkvm_core::{InputError, InputResult, KeyEvent, MouseMode, PointerEvent};
    use ipkvm_video::{
        FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource,
    };

    use super::*;
    use crate::rfb_connection::RfbClientId;

    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        recorded: Arc<Mutex<Recorded>>,
    }

    #[derive(Clone, Debug, Default)]
    struct Recorded {
        key_batches: usize,
        fail_next_key: bool,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
            let mut recorded = self.recorded.lock().unwrap();
            if std::mem::take(&mut recorded.fail_next_key) {
                return Err(InputError::RolloverLimitExceeded);
            }
            recorded.key_batches += 1;
            Ok(())
        }

        fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            Ok(())
        }
    }

    fn frame(seq: u64) -> Arc<VideoFrame> {
        Arc::new(VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            2,
            1,
            8,
            PixelFormat::Bgra8888,
            Arc::from(vec![0; 8].into_boxed_slice()),
        ))
    }

    fn policy() -> RecoveryPolicy {
        RecoveryPolicy {
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_attempts: 1,
            tick: Duration::from_millis(1),
            video_start_timeout: Duration::from_millis(10),
        }
    }

    async fn yield_until(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..2_000 {
            if condition() {
                return true;
            }
            tokio::task::yield_now().await;
        }
        condition()
    }

    #[tokio::test]
    async fn control_failure_preserves_video_subscription_and_work_view() {
        let gate = RfbConnectionGate::new();
        let mut supervisor = SessionSupervisor::new(gate, policy());
        let source = Arc::new(MockFrameSource::new());
        let sink = RecordingSink::default();
        sink.recorded.lock().unwrap().fail_next_key = true;
        let mut frame_rx = supervisor.subscribe_frames();

        supervisor
            .start_at(
                || Ok(source.clone() as Arc<dyn FrameSource>),
                || Ok(sink.clone()),
                Instant::now(),
            )
            .await;
        source.publish_frame(frame(1));
        frame_rx.changed().await.unwrap();
        assert_eq!(frame_rx.borrow().as_ref().unwrap().seq, 1);

        let sender = supervisor
            .event_publisher()
            .borrow()
            .clone()
            .expect("control starts with an event sender");
        let client_id = RfbClientId::local_desktop();
        sender
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr: "127.0.0.1:5900".parse().unwrap(),
                shared: true,
            })
            .await
            .unwrap();
        sender
            .send(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            })
            .await
            .unwrap();
        assert!(
            yield_until(|| supervisor.manager.state() == SessionState::Stopped).await,
            "input pump failure should stop only the control runtime"
        );

        supervisor
            .tick_at(
                || panic!("video should not be reopened for control failure"),
                || Err("serial still unavailable".to_string()),
                Instant::now() + Duration::from_millis(2),
            )
            .await;

        let status = supervisor.status();
        assert_eq!(status.video, VideoRuntimeStatus::Streaming);
        assert!(matches!(
            status.control,
            ControlRuntimeStatus::Recovering { .. } | ControlRuntimeStatus::Failed { .. }
        ));
        assert!(status.should_show_work_view());
        assert_eq!(frame_rx.borrow().as_ref().unwrap().seq, 1);
        assert!(supervisor.event_publisher().borrow().is_none());
    }

    #[tokio::test]
    async fn video_reopen_keeps_existing_subscription_and_control_ready() {
        let gate = RfbConnectionGate::new();
        let mut supervisor = SessionSupervisor::new(gate, policy());
        let first = Arc::new(MockFrameSource::new());
        let second = Arc::new(MockFrameSource::new());
        let second_for_video = Arc::clone(&second);
        let opens = Arc::new(AtomicUsize::new(0));
        let opens_for_video = Arc::clone(&opens);
        let mut frame_rx = supervisor.subscribe_frames();
        let start = Instant::now();

        supervisor
            .start_at(
                || Ok(first.clone() as Arc<dyn FrameSource>),
                || Ok(RecordingSink::default()),
                start,
            )
            .await;
        first.publish_frame(frame(1));
        frame_rx.changed().await.unwrap();
        assert_eq!(frame_rx.borrow().as_ref().unwrap().seq, 1);
        supervisor
            .tick_at(
                || panic!("fresh video should not be reopened"),
                || panic!("fresh control should not be reopened"),
                start + Duration::from_millis(1),
            )
            .await;

        supervisor
            .tick_at(
                || panic!("retry delay has not elapsed yet"),
                || panic!("control should not be reopened for video stall"),
                start + Duration::from_millis(20),
            )
            .await;
        supervisor
            .tick_at(
                move || {
                    opens_for_video.fetch_add(1, Ordering::SeqCst);
                    Ok(second_for_video.clone() as Arc<dyn FrameSource>)
                },
                || panic!("control should not be reopened for video stall"),
                start + Duration::from_millis(21),
            )
            .await;
        second.publish_frame(frame(2));
        assert!(
            yield_until(|| frame_rx
                .borrow()
                .as_ref()
                .is_some_and(|frame| frame.seq == 2))
            .await,
            "new source should publish through the existing receiver"
        );
        supervisor
            .tick_at(
                || panic!("new video should not be reopened"),
                || panic!("control should not be reopened for video recovery"),
                start + Duration::from_millis(22),
            )
            .await;

        assert_eq!(opens.load(Ordering::SeqCst), 1);
        assert_eq!(frame_rx.borrow().as_ref().unwrap().seq, 2);
        let status = supervisor.status();
        assert_eq!(status.video, VideoRuntimeStatus::Streaming);
        assert_eq!(status.control, ControlRuntimeStatus::Ready);
    }

    #[tokio::test]
    async fn manual_stop_prevents_tick_revival() {
        let gate = RfbConnectionGate::new();
        let mut supervisor = SessionSupervisor::new(gate, policy());
        let source = Arc::new(MockFrameSource::new());
        let video_opens = Arc::new(AtomicUsize::new(0));
        let control_opens = Arc::new(AtomicUsize::new(0));

        supervisor
            .start_at(
                || Ok(source.clone() as Arc<dyn FrameSource>),
                || Ok(RecordingSink::default()),
                Instant::now(),
            )
            .await;
        supervisor.stop_manual().await.unwrap();

        let video_opens_for_tick = Arc::clone(&video_opens);
        let control_opens_for_tick = Arc::clone(&control_opens);
        supervisor
            .tick_at(
                move || {
                    video_opens_for_tick.fetch_add(1, Ordering::SeqCst);
                    Ok(source.clone() as Arc<dyn FrameSource>)
                },
                move || {
                    control_opens_for_tick.fetch_add(1, Ordering::SeqCst);
                    Ok(RecordingSink::default())
                },
                Instant::now() + Duration::from_secs(1),
            )
            .await;

        assert_eq!(video_opens.load(Ordering::SeqCst), 0);
        assert_eq!(control_opens.load(Ordering::SeqCst), 0);
        assert_eq!(supervisor.status().intent, SessionIntent::ManualStopped);
        assert!(!supervisor.status().should_show_work_view());
    }
}
