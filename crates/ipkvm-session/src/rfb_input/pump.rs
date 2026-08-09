use std::net::SocketAddr;

use ipkvm_core::{
    FramebufferSize, InputError, InputSink, MouseMode,
    diag::{self, DiagCategory, DiagLevel},
};
use ipkvm_rfb::RfbRectangle;
use thiserror::Error;
use tokio::sync::{mpsc, watch};

use super::text::{TextInputConfig, TextInputNotice, TextInputService};
use super::{
    RfbKeyboardError, RfbKeyboardMapper, RfbKeyboardOutcome, RfbPointerError, RfbPointerMapper,
    RfbPointerOutcome,
};
use crate::rfb_connection::{RfbClientId, RfbDisconnectReason, RfbServerEvent};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbControllerReleaseReason {
    Disconnected(RfbDisconnectReason),
    EventSourceClosed,
    Explicit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbKeyboardRejection {
    UnsupportedKeysym(u32),
    ConflictingShiftRequirements,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbInputNotice {
    ControllerAcquired {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        shared: bool,
    },
    Keyboard {
        client_id: RfbClientId,
        outcome: RfbKeyboardOutcome,
    },
    KeyboardRejected {
        client_id: RfbClientId,
        rejection: RfbKeyboardRejection,
    },
    Pointer {
        client_id: RfbClientId,
        outcome: RfbPointerOutcome,
    },
    TextDispatched {
        client_id: RfbClientId,
        char_count: usize,
    },
    TextTyped {
        client_id: RfbClientId,
        chars_typed: usize,
        chars_skipped: usize,
    },
    TextInputFailed {
        client_id: RfbClientId,
        error: String,
    },
    ContinuousUpdatesIgnored {
        client_id: RfbClientId,
        enable: bool,
        rectangle: RfbRectangle,
    },
    /// 观察到一次 FramebufferUpdate 成功发送的统计通知（调研阶段 0 埋点）。
    /// observe 闭包据此累计 updates_sent 并刷新 encode 统计快照。
    FrameUpdateStatsObserved {
        client_id: RfbClientId,
        encode: ipkvm_rfb::RfbEncodeStatsSnapshot,
    },
    PreHandshakeDisconnected {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        reason: RfbDisconnectReason,
    },
    ControllerReleased {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        reason: RfbControllerReleaseReason,
    },
    MouseModeChanged {
        client_id: RfbClientId,
        mode: MouseMode,
    },
}

impl From<TextInputNotice> for RfbInputNotice {
    fn from(notice: TextInputNotice) -> Self {
        match notice {
            TextInputNotice::Typed {
                client_id,
                chars_typed,
                chars_skipped,
            } => RfbInputNotice::TextTyped {
                client_id,
                chars_typed,
                chars_skipped,
            },
            TextInputNotice::Error { client_id, error } => {
                RfbInputNotice::TextInputFailed { client_id, error }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbInputEventKind {
    Connected,
    Key,
    Pointer,
    PointerRelative,
    SetMouseMode,
    CutText,
    ContinuousUpdates,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbInputOperation {
    Keyboard,
    Pointer,
    Release,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbInputLifecycleError {
    #[error("RFB controller {active:?} is still active when client {incoming:?} connected")]
    ControllerAlreadyActive {
        active: RfbClientId,
        incoming: RfbClientId,
    },
    #[error("RFB client {incoming:?} sent {event_kind:?} without an active controller")]
    NoActiveController {
        incoming: RfbClientId,
        event_kind: RfbInputEventKind,
    },
    #[error("RFB client {incoming:?} sent {event_kind:?} while controller {active:?} is active")]
    WrongController {
        active: RfbClientId,
        incoming: RfbClientId,
        event_kind: RfbInputEventKind,
    },
    #[error(
        "RFB client {client_id:?} disconnected from {actual}, expected peer address {expected}"
    )]
    PeerAddressChanged {
        client_id: RfbClientId,
        expected: SocketAddr,
        actual: SocketAddr,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbInputError {
    #[error("invalid RFB controller lifecycle: {0}")]
    Lifecycle(#[from] RfbInputLifecycleError),
    #[error("input sink rejected {operation:?} for RFB client {client_id:?}: {source}")]
    Sink {
        client_id: RfbClientId,
        operation: RfbInputOperation,
        #[source]
        source: InputError,
    },
    #[error("text input service is unavailable for RFB client {client_id:?}")]
    TextInputUnavailable { client_id: RfbClientId },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("failed to process an RFB input event: {source}")]
pub struct RfbInputEventError {
    event: Box<RfbServerEvent>,
    #[source]
    source: RfbInputError,
}

impl RfbInputEventError {
    pub fn event(&self) -> &RfbServerEvent {
        self.event.as_ref()
    }

    pub fn error(&self) -> &RfbInputError {
        &self.source
    }

    pub fn into_parts(self) -> (RfbServerEvent, RfbInputError) {
        (*self.event, self.source)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbInputRunError {
    #[error(transparent)]
    Event(#[from] RfbInputEventError),
    #[error("failed to release RFB input after the event source closed")]
    SourceClosedRelease(#[source] RfbInputError),
    #[error("failed to release RFB input after the session stopped")]
    StopRelease(#[source] RfbInputError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveController {
    client_id: RfbClientId,
    peer_addr: SocketAddr,
}

#[derive(Debug)]
pub struct RfbInputPump<S: InputSink> {
    sink: S,
    active: Option<ActiveController>,
    /// 当前控制连接最后一次声明/使用的指针协议模式。RFB 没有在标准
    /// Pointer 消息中携带模式字段，因此网页端切换 Pointer Lock 时，
    /// 以实际到达的消息类型同步 sink，避免设置值与首个事件竞态。
    mouse_mode: Option<MouseMode>,
    keyboard: RfbKeyboardMapper,
    pointer: RfbPointerMapper,
    text: TextInputService<S>,
    text_notices: mpsc::Receiver<TextInputNotice>,
    mouse_mode_observer: Option<watch::Sender<Option<MouseMode>>>,
}

impl<S: InputSink> RfbInputPump<S> {
    /// 创建输入泵；内部以 `sink` 的克隆启动独立文本键入服务（见 `TextInputService`）。
    /// 调用方必须运行在 tokio runtime 上下文中（内部 `tokio::spawn`）。
    pub fn new(sink: S) -> Self
    where
        S: Clone + Send + 'static,
    {
        Self::with_mouse_mode_observer(sink, None)
    }

    /// 创建输入泵，并可把 sink 已确认的鼠标模式发布给会话控制面。
    pub fn with_mouse_mode_observer(
        sink: S,
        mouse_mode_observer: Option<watch::Sender<Option<MouseMode>>>,
    ) -> Self
    where
        S: Clone + Send + 'static,
    {
        let initial_mouse_mode = sink.initial_mouse_mode();
        let (text, text_notices) = TextInputService::new(sink.clone(), TextInputConfig::default());
        Self {
            sink,
            active: None,
            mouse_mode: initial_mouse_mode,
            keyboard: RfbKeyboardMapper::new(),
            pointer: RfbPointerMapper::new(),
            text,
            text_notices,
            mouse_mode_observer,
        }
    }

    pub fn active_client(&self) -> Option<RfbClientId> {
        self.active.map(|active| active.client_id)
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn handle_event(
        &mut self,
        event: RfbServerEvent,
    ) -> Result<RfbInputNotice, RfbInputEventError> {
        match self.try_handle_event(&event) {
            Ok(notice) => Ok(notice),
            Err(source) => Err(RfbInputEventError {
                event: Box::new(event),
                source,
            }),
        }
    }

    pub fn release_active(&mut self) -> Result<Option<RfbInputNotice>, RfbInputError> {
        self.release_with_reason(RfbControllerReleaseReason::Explicit)
    }

    pub async fn run<F>(
        &mut self,
        receiver: &mut mpsc::Receiver<RfbServerEvent>,
        mut observe: F,
    ) -> Result<(), RfbInputRunError>
    where
        F: FnMut(&RfbInputNotice),
    {
        loop {
            let event = tokio::select! {
                event = receiver.recv() => event,
                notice = self.text_notices.recv() => {
                    if let Some(notice) = notice {
                        observe(&RfbInputNotice::from(notice));
                    }
                    continue;
                }
            };
            let Some(event) = event else {
                break;
            };
            let notice = self.handle_event(event)?;
            observe(&notice);
        }
        self.finish_run(
            RfbControllerReleaseReason::EventSourceClosed,
            RfbInputRunError::SourceClosedRelease,
            &mut observe,
        )
    }

    /// 运行输入泵直到收到停止信号或事件源关闭。
    ///
    /// 与会话生命周期配套：`ConsoleSession::stop()` 通过 watch 停止信号停泵，
    /// 不依赖事件发送端全部释放（传输层可能长期持有 `event_tx` 克隆）。
    /// 停止信号触发时以 `Explicit` 原因释放活动控制器，channel 关闭路径仍
    /// 保持 `EventSourceClosed` 语义。
    pub async fn run_until_stopped<F>(
        &mut self,
        receiver: &mut mpsc::Receiver<RfbServerEvent>,
        mut stop: watch::Receiver<bool>,
        mut observe: F,
    ) -> Result<(), RfbInputRunError>
    where
        F: FnMut(&RfbInputNotice),
    {
        loop {
            if *stop.borrow() {
                break;
            }
            tokio::select! {
                event = receiver.recv() => {
                    let Some(event) = event else { break };
                    let notice = self.handle_event(event)?;
                    observe(&notice);
                }
                notice = self.text_notices.recv() => {
                    if let Some(notice) = notice {
                        observe(&RfbInputNotice::from(notice));
                    }
                }
                _ = stop.changed() => break,
            }
        }
        let (reason, on_release_error) = if *stop.borrow() {
            (
                RfbControllerReleaseReason::Explicit,
                RfbInputRunError::StopRelease as fn(RfbInputError) -> RfbInputRunError,
            )
        } else {
            (
                RfbControllerReleaseReason::EventSourceClosed,
                RfbInputRunError::SourceClosedRelease as fn(RfbInputError) -> RfbInputRunError,
            )
        };
        self.finish_run(reason, on_release_error, &mut observe)
    }

    /// 收尾：释放活动控制器（如有）并把 release 通知交给 observe。
    fn finish_run<F>(
        &mut self,
        reason: RfbControllerReleaseReason,
        on_release_error: fn(RfbInputError) -> RfbInputRunError,
        observe: &mut F,
    ) -> Result<(), RfbInputRunError>
    where
        F: FnMut(&RfbInputNotice),
    {
        if let Some(notice) = self.release_with_reason(reason).map_err(on_release_error)? {
            observe(&notice);
        }
        Ok(())
    }

    fn try_handle_event(
        &mut self,
        event: &RfbServerEvent,
    ) -> Result<RfbInputNotice, RfbInputError> {
        match event {
            RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared,
            } => self.connect(*client_id, *peer_addr, *shared),
            RfbServerEvent::Key {
                client_id,
                down,
                keysym,
            } => self.handle_key(*client_id, *down, *keysym),
            RfbServerEvent::Pointer {
                client_id,
                button_mask,
                x,
                y,
                framebuffer_size,
            } => self.handle_pointer(
                *client_id,
                *button_mask,
                *x,
                *y,
                FramebufferSize {
                    width: u32::from(framebuffer_size.width()),
                    height: u32::from(framebuffer_size.height()),
                },
            ),
            RfbServerEvent::PointerRelative {
                client_id,
                button_mask,
                dx,
                dy,
                wheel,
            } => self.handle_pointer_relative(*client_id, *button_mask, *dx, *dy, *wheel),
            RfbServerEvent::SetMouseMode { client_id, mode } => {
                self.set_mouse_mode(*client_id, *mode)
            }
            RfbServerEvent::CutText { client_id, bytes } => {
                self.require_active(*client_id, RfbInputEventKind::CutText)?;
                let text = String::from_utf8_lossy(bytes).into_owned();
                let char_count = text.chars().count();
                self.text.try_type_text(*client_id, text).map_err(|_| {
                    RfbInputError::TextInputUnavailable {
                        client_id: *client_id,
                    }
                })?;
                Ok(RfbInputNotice::TextDispatched {
                    client_id: *client_id,
                    char_count,
                })
            }
            RfbServerEvent::ContinuousUpdates {
                client_id,
                enable,
                rectangle,
            } => {
                self.require_active(*client_id, RfbInputEventKind::ContinuousUpdates)?;
                Ok(RfbInputNotice::ContinuousUpdatesIgnored {
                    client_id: *client_id,
                    enable: *enable,
                    rectangle: *rectangle,
                })
            }
            RfbServerEvent::FrameUpdateSent { client_id, encode } => {
                Ok(RfbInputNotice::FrameUpdateStatsObserved {
                    client_id: *client_id,
                    encode: *encode,
                })
            }
            RfbServerEvent::Disconnected {
                client_id,
                peer_addr,
                reason,
            } => self.disconnect(*client_id, *peer_addr, reason.clone()),
        }
    }

    fn connect(
        &mut self,
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        shared: bool,
    ) -> Result<RfbInputNotice, RfbInputError> {
        if let Some(active) = self.active {
            return Err(RfbInputLifecycleError::ControllerAlreadyActive {
                active: active.client_id,
                incoming: client_id,
            }
            .into());
        }
        self.keyboard = RfbKeyboardMapper::new();
        self.pointer = RfbPointerMapper::new();
        self.active = Some(ActiveController {
            client_id,
            peer_addr,
        });
        diag::log(
            DiagLevel::Info,
            DiagCategory::LIFECYCLE,
            "session.input_pump",
            "controller_acquired",
            &[
                ("client", format!("{client_id:?}")),
                ("peer", peer_addr.to_string()),
                ("shared", shared.to_string()),
            ],
        );
        Ok(RfbInputNotice::ControllerAcquired {
            client_id,
            peer_addr,
            shared,
        })
    }

    fn handle_key(
        &mut self,
        client_id: RfbClientId,
        down: bool,
        keysym: u32,
    ) -> Result<RfbInputNotice, RfbInputError> {
        self.require_active(client_id, RfbInputEventKind::Key)?;
        diag::log(
            DiagLevel::Trace,
            DiagCategory::KEYBOARD,
            "session.input_pump",
            "key",
            &[
                ("client", format!("{client_id:?}")),
                ("down", down.to_string()),
                ("keysym", format!("0x{keysym:08x}")),
            ],
        );
        match self.keyboard.handle_key(&mut self.sink, down, keysym) {
            Ok(outcome) => Ok(RfbInputNotice::Keyboard { client_id, outcome }),
            Err(RfbKeyboardError::UnsupportedKeysym(keysym)) => {
                Ok(RfbInputNotice::KeyboardRejected {
                    client_id,
                    rejection: RfbKeyboardRejection::UnsupportedKeysym(keysym),
                })
            }
            Err(RfbKeyboardError::ConflictingShiftRequirements) => {
                Ok(RfbInputNotice::KeyboardRejected {
                    client_id,
                    rejection: RfbKeyboardRejection::ConflictingShiftRequirements,
                })
            }
            Err(RfbKeyboardError::Input(source)) => Err(RfbInputError::Sink {
                client_id,
                operation: RfbInputOperation::Keyboard,
                source,
            }),
        }
    }

    fn handle_pointer(
        &mut self,
        client_id: RfbClientId,
        button_mask: u8,
        x: u16,
        y: u16,
        framebuffer_size: FramebufferSize,
    ) -> Result<RfbInputNotice, RfbInputError> {
        self.require_active(client_id, RfbInputEventKind::Pointer)?;
        diag::log(
            DiagLevel::Trace,
            DiagCategory::POINTER,
            "session.input_pump",
            "pointer_absolute",
            &[
                ("client", format!("{client_id:?}")),
                ("mask", format_mask(button_mask)),
                ("x", x.to_string()),
                ("y", y.to_string()),
                (
                    "fb",
                    format!("{}x{}", framebuffer_size.width, framebuffer_size.height),
                ),
            ],
        );
        if self.mouse_mode == Some(MouseMode::Relative) {
            diag::log(
                DiagLevel::Warn,
                DiagCategory::POINTER,
                "session.input_pump",
                "pointer_ignored",
                &[
                    ("client", format!("{client_id:?}")),
                    ("kind", "absolute".into()),
                    ("mode", "relative".into()),
                    ("mask", format_mask(button_mask)),
                ],
            );
            return Ok(RfbInputNotice::Pointer {
                client_id,
                outcome: RfbPointerOutcome::IgnoredForMouseMode {
                    mode: MouseMode::Relative,
                },
            });
        }
        self.ensure_mouse_mode(client_id, MouseMode::Absolute)?;
        match self
            .pointer
            .handle_pointer(&mut self.sink, button_mask, x, y, framebuffer_size)
        {
            Ok(outcome) => Ok(RfbInputNotice::Pointer { client_id, outcome }),
            Err(RfbPointerError::Input(source)) => Err(RfbInputError::Sink {
                client_id,
                operation: RfbInputOperation::Pointer,
                source,
            }),
        }
    }

    fn handle_pointer_relative(
        &mut self,
        client_id: RfbClientId,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    ) -> Result<RfbInputNotice, RfbInputError> {
        self.require_active(client_id, RfbInputEventKind::PointerRelative)?;
        diag::log(
            DiagLevel::Trace,
            DiagCategory::POINTER,
            "session.input_pump",
            "pointer_relative",
            &[
                ("client", format!("{client_id:?}")),
                ("mask", format_mask(button_mask)),
                ("dx", dx.to_string()),
                ("dy", dy.to_string()),
                ("wheel", wheel.to_string()),
            ],
        );
        self.ensure_mouse_mode(client_id, MouseMode::Relative)?;
        match self
            .pointer
            .handle_relative_pointer(&mut self.sink, button_mask, dx, dy, wheel)
        {
            Ok(outcome) => Ok(RfbInputNotice::Pointer { client_id, outcome }),
            Err(RfbPointerError::Input(source)) => Err(RfbInputError::Sink {
                client_id,
                operation: RfbInputOperation::Pointer,
                source,
            }),
        }
    }

    fn set_mouse_mode(
        &mut self,
        client_id: RfbClientId,
        mode: MouseMode,
    ) -> Result<RfbInputNotice, RfbInputError> {
        self.require_active(client_id, RfbInputEventKind::SetMouseMode)?;
        self.switch_mouse_mode(client_id, mode)?;
        Ok(RfbInputNotice::MouseModeChanged { client_id, mode })
    }

    fn ensure_mouse_mode(
        &mut self,
        client_id: RfbClientId,
        mode: MouseMode,
    ) -> Result<(), RfbInputError> {
        if self.mouse_mode == Some(mode) {
            return Ok(());
        }
        self.switch_mouse_mode(client_id, mode)
    }

    fn switch_mouse_mode(
        &mut self,
        client_id: RfbClientId,
        mode: MouseMode,
    ) -> Result<(), RfbInputError> {
        if self.mouse_mode == Some(mode) {
            return Ok(());
        }
        self.sink
            .set_mouse_mode(mode)
            .map_err(|source| RfbInputError::Sink {
                client_id,
                operation: RfbInputOperation::Pointer,
                source,
            })?;
        self.pointer = RfbPointerMapper::new();
        self.mouse_mode = Some(mode);
        diag::log(
            DiagLevel::Info,
            DiagCategory::LIFECYCLE,
            "session.input_pump",
            "mouse_mode",
            &[
                ("client", format!("{client_id:?}")),
                ("mode", mouse_mode_name(mode).into()),
            ],
        );
        if let Some(observer) = &self.mouse_mode_observer {
            let _ = observer.send(Some(mode));
        }
        Ok(())
    }

    fn disconnect(
        &mut self,
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        reason: RfbDisconnectReason,
    ) -> Result<RfbInputNotice, RfbInputError> {
        let Some(active) = self.active else {
            return Ok(RfbInputNotice::PreHandshakeDisconnected {
                client_id,
                peer_addr,
                reason,
            });
        };
        self.require_active(client_id, RfbInputEventKind::Disconnected)?;
        if active.peer_addr != peer_addr {
            return Err(RfbInputLifecycleError::PeerAddressChanged {
                client_id,
                expected: active.peer_addr,
                actual: peer_addr,
            }
            .into());
        }
        self.release_with_reason(RfbControllerReleaseReason::Disconnected(reason))
            .map(|notice| notice.expect("a matching disconnected event has an active controller"))
    }

    fn require_active(
        &self,
        incoming: RfbClientId,
        event_kind: RfbInputEventKind,
    ) -> Result<ActiveController, RfbInputError> {
        let Some(active) = self.active else {
            return Err(RfbInputLifecycleError::NoActiveController {
                incoming,
                event_kind,
            }
            .into());
        };
        if active.client_id != incoming {
            return Err(RfbInputLifecycleError::WrongController {
                active: active.client_id,
                incoming,
                event_kind,
            }
            .into());
        }
        Ok(active)
    }

    fn release_with_reason(
        &mut self,
        reason: RfbControllerReleaseReason,
    ) -> Result<Option<RfbInputNotice>, RfbInputError> {
        let Some(active) = self.active else {
            return Ok(None);
        };
        diag::log(
            DiagLevel::Warn,
            DiagCategory::LIFECYCLE,
            "session.input_pump",
            "release_all",
            &[
                ("client", format!("{:?}", active.client_id)),
                ("peer", active.peer_addr.to_string()),
                ("reason", format!("{reason:?}")),
            ],
        );
        self.sink
            .release_all()
            .map_err(|source| RfbInputError::Sink {
                client_id: active.client_id,
                operation: RfbInputOperation::Release,
                source,
            })?;
        // 取消进行中的文本键入（服务内部 release_all 复位其 sink）。
        let _ = self.text.try_cancel(active.client_id);
        self.active = None;
        self.mouse_mode = None;
        if let Some(observer) = &self.mouse_mode_observer {
            let _ = observer.send(None);
        }
        self.keyboard = RfbKeyboardMapper::new();
        self.pointer = RfbPointerMapper::new();
        Ok(Some(RfbInputNotice::ControllerReleased {
            client_id: active.client_id,
            peer_addr: active.peer_addr,
            reason,
        }))
    }
}

fn mouse_mode_name(mode: MouseMode) -> &'static str {
    match mode {
        MouseMode::Absolute => "absolute",
        MouseMode::Relative => "relative",
    }
}

fn format_mask(mask: u8) -> String {
    format!("0x{mask:02x}")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use ipkvm_core::{
        Ch9329InputSink, CommandQueueError, FramebufferSize, InputResult, KeyEvent, MouseMode,
        PointerButton, PointerEvent, fake_serial::FakeCommandQueue,
    };
    use ipkvm_rfb::RfbSize;

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        key_batches: Vec<Vec<KeyEvent>>,
        pointer_batches: Vec<Vec<PointerEvent>>,
        mouse_modes: Vec<MouseMode>,
        release_count: usize,
        fail_next_key: bool,
        fail_next_pointer: bool,
        fail_next_release: bool,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()> {
            self.mouse_modes.push(mode);
            Ok(())
        }

        fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
            if std::mem::take(&mut self.fail_next_key) {
                return Err(InputError::PointerPositionUnknown);
            }
            self.key_batches.push(events.to_vec());
            Ok(())
        }

        fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
            if std::mem::take(&mut self.fail_next_pointer) {
                return Err(InputError::PointerPositionUnknown);
            }
            self.pointer_batches.push(events.to_vec());
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            if std::mem::take(&mut self.fail_next_release) {
                return Err(InputError::PointerPositionUnknown);
            }
            self.release_count += 1;
            Ok(())
        }
    }

    fn client(value: u64) -> RfbClientId {
        RfbClientId::for_test(value)
    }

    fn peer(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn connected(client_id: RfbClientId, peer_addr: SocketAddr) -> RfbServerEvent {
        RfbServerEvent::Connected {
            client_id,
            peer_addr,
            shared: true,
        }
    }

    #[tokio::test]
    async fn routes_controller_keyboard_and_pointer_events() {
        let client_id = client(1);
        let peer_addr = peer(5900);
        let mut pump = RfbInputPump::new(RecordingSink::default());

        assert_eq!(
            pump.handle_event(connected(client_id, peer_addr)),
            Ok(RfbInputNotice::ControllerAcquired {
                client_id,
                peer_addr,
                shared: true,
            })
        );
        assert_eq!(pump.active_client(), Some(client_id));

        assert_eq!(
            pump.handle_event(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            }),
            Ok(RfbInputNotice::Keyboard {
                client_id,
                outcome: RfbKeyboardOutcome::Applied,
            })
        );
        assert_eq!(pump.sink().key_batches.len(), 1);
        assert_eq!(pump.sink().key_batches[0].len(), 1);

        assert_eq!(
            pump.handle_event(RfbServerEvent::Pointer {
                client_id,
                button_mask: 1,
                x: 100,
                y: 200,
                framebuffer_size: RfbSize::new(1920, 1080).unwrap(),
            }),
            Ok(RfbInputNotice::Pointer {
                client_id,
                outcome: RfbPointerOutcome::Applied,
            })
        );
        assert_eq!(
            pump.sink().pointer_batches,
            vec![vec![
                PointerEvent::AbsoluteMove {
                    x: 100,
                    y: 200,
                    framebuffer_size: FramebufferSize {
                        width: 1920,
                        height: 1080,
                    },
                },
                PointerEvent::Button {
                    button: PointerButton::Left,
                    down: true,
                },
            ]]
        );
    }

    #[tokio::test]
    async fn reports_client_rejections_and_ignored_events_without_stopping() {
        let client_id = client(2);
        let peer_addr = peer(5901);
        let mut pump = RfbInputPump::new(RecordingSink::default());
        pump.handle_event(connected(client_id, peer_addr)).unwrap();

        assert_eq!(
            pump.handle_event(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x0100_0100,
            }),
            Ok(RfbInputNotice::KeyboardRejected {
                client_id,
                rejection: RfbKeyboardRejection::UnsupportedKeysym(0x0100_0100),
            })
        );
        assert_eq!(
            pump.handle_event(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x41,
            }),
            Ok(RfbInputNotice::Keyboard {
                client_id,
                outcome: RfbKeyboardOutcome::Applied,
            })
        );
        assert_eq!(
            pump.handle_event(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            }),
            Ok(RfbInputNotice::KeyboardRejected {
                client_id,
                rejection: RfbKeyboardRejection::ConflictingShiftRequirements,
            })
        );
        assert_eq!(
            pump.handle_event(RfbServerEvent::CutText {
                client_id,
                bytes: b"abc".to_vec(),
            }),
            Ok(RfbInputNotice::TextDispatched {
                client_id,
                char_count: 3,
            })
        );
        let rectangle = RfbRectangle {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        assert_eq!(
            pump.handle_event(RfbServerEvent::ContinuousUpdates {
                client_id,
                enable: true,
                rectangle,
            }),
            Ok(RfbInputNotice::ContinuousUpdatesIgnored {
                client_id,
                enable: true,
                rectangle,
            })
        );
    }

    #[tokio::test]
    async fn cut_text_typing_result_is_reported_as_a_notice() {
        let client_id = client(17);
        let peer_addr = peer(5916);
        let (tx, mut rx) = mpsc::channel(2);
        let mut pump = RfbInputPump::new(RecordingSink::default());
        let mut notices = Vec::new();
        let driver = tokio::spawn(async move {
            tx.send(connected(client_id, peer_addr)).await.unwrap();
            tx.send(RfbServerEvent::CutText {
                client_id,
                bytes: b"ab".to_vec(),
            })
            .await
            .unwrap();
            // 默认 30ms/字符，两个字符约 120ms；留足余量再关闭事件通道。
            tokio::time::sleep(Duration::from_millis(500)).await;
            drop(tx);
        });
        pump.run(&mut rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap();
        driver.await.unwrap();

        let dispatched = notices
            .iter()
            .position(|notice| {
                matches!(
                    notice,
                    RfbInputNotice::TextDispatched {
                        client_id: found,
                        char_count: 2,
                    } if *found == client_id
                )
            })
            .expect("text dispatch notice");
        let typed = notices
            .iter()
            .position(|notice| {
                matches!(
                    notice,
                    RfbInputNotice::TextTyped {
                        client_id: found,
                        chars_typed: 2,
                        chars_skipped: 0,
                    } if *found == client_id
                )
            })
            .expect("text typed notice");
        assert!(dispatched < typed);
        assert_eq!(pump.sink().release_count, 1);
        // 键入走服务自己的 sink 副本，不触碰 pump 的 sink。
        assert!(pump.sink().key_batches.is_empty());
        assert_eq!(pump.active_client(), None);
    }

    #[tokio::test]
    async fn disconnect_cancels_in_flight_text_typing() {
        let client_id = client(18);
        let peer_addr = peer(5917);
        let (tx, mut rx) = mpsc::channel(4);
        let mut pump = RfbInputPump::new(RecordingSink::default());
        let mut notices = Vec::new();
        // 100 个字符按默认 30ms/字符约 6s，断开发生在键入中途。
        let long_text = "x".repeat(100);
        let driver = tokio::spawn(async move {
            tx.send(connected(client_id, peer_addr)).await.unwrap();
            tx.send(RfbServerEvent::CutText {
                client_id,
                bytes: long_text.into_bytes(),
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(80)).await;
            tx.send(RfbServerEvent::Disconnected {
                client_id,
                peer_addr,
                reason: RfbDisconnectReason::ClientClosed,
            })
            .await
            .unwrap();
            // 留足余量让取消后的部分键入结果通知到达 pump。
            tokio::time::sleep(Duration::from_millis(150)).await;
            drop(tx);
        });
        pump.run(&mut rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap();
        driver.await.unwrap();

        assert!(matches!(
            notices.as_slice(),
            [
                RfbInputNotice::ControllerAcquired { .. },
                RfbInputNotice::TextDispatched { .. },
                ..
            ]
        ));
        let (typed, skipped) = notices
            .iter()
            .find_map(|notice| match notice {
                RfbInputNotice::TextTyped {
                    client_id: found,
                    chars_typed,
                    chars_skipped,
                } if *found == client_id => Some((*chars_typed, *chars_skipped)),
                _ => None,
            })
            .expect("partial typing result notice after disconnect");
        assert_eq!(skipped, 0);
        assert!(
            typed < 100,
            "disconnect should cancel typing mid-flight, typed {typed}"
        );
        assert_eq!(pump.sink().release_count, 1);
        assert_eq!(pump.active_client(), None);
    }

    #[tokio::test]
    async fn sink_failures_return_the_original_event_and_can_be_retried() {
        let client_id = client(3);
        let peer_addr = peer(5902);
        let mut pump = RfbInputPump::new(RecordingSink {
            fail_next_key: true,
            fail_next_pointer: true,
            ..RecordingSink::default()
        });
        pump.handle_event(connected(client_id, peer_addr)).unwrap();

        let key_event = RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x61,
        };
        let error = pump.handle_event(key_event.clone()).unwrap_err();
        assert_eq!(error.event(), &key_event);
        assert_eq!(
            error.error(),
            &RfbInputError::Sink {
                client_id,
                operation: RfbInputOperation::Keyboard,
                source: InputError::PointerPositionUnknown,
            }
        );
        let (retry, _) = error.into_parts();
        assert_eq!(
            pump.handle_event(retry),
            Ok(RfbInputNotice::Keyboard {
                client_id,
                outcome: RfbKeyboardOutcome::Applied,
            })
        );

        let pointer_event = RfbServerEvent::Pointer {
            client_id,
            button_mask: 1,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        };
        let error = pump.handle_event(pointer_event.clone()).unwrap_err();
        assert_eq!(error.event(), &pointer_event);
        assert!(matches!(
            error.error(),
            RfbInputError::Sink {
                operation: RfbInputOperation::Pointer,
                ..
            }
        ));
        let (retry, _) = error.into_parts();
        assert!(pump.handle_event(retry).is_ok());
        assert_eq!(pump.active_client(), Some(client_id));
    }

    #[tokio::test]
    async fn rejects_events_that_break_the_single_controller_lifecycle() {
        let first = client(4);
        let second = client(5);
        let first_peer = peer(5903);
        let second_peer = peer(5904);
        let mut pump = RfbInputPump::new(RecordingSink::default());

        let no_controller = pump
            .handle_event(RfbServerEvent::Key {
                client_id: first,
                down: true,
                keysym: 0x61,
            })
            .unwrap_err();
        assert_eq!(
            no_controller.error(),
            &RfbInputError::Lifecycle(RfbInputLifecycleError::NoActiveController {
                incoming: first,
                event_kind: RfbInputEventKind::Key,
            })
        );

        pump.handle_event(connected(first, first_peer)).unwrap();
        let duplicate = pump
            .handle_event(connected(second, second_peer))
            .unwrap_err();
        assert_eq!(
            duplicate.error(),
            &RfbInputError::Lifecycle(RfbInputLifecycleError::ControllerAlreadyActive {
                active: first,
                incoming: second,
            })
        );

        let wrong_key = pump
            .handle_event(RfbServerEvent::Key {
                client_id: second,
                down: true,
                keysym: 0x61,
            })
            .unwrap_err();
        assert_eq!(
            wrong_key.error(),
            &RfbInputError::Lifecycle(RfbInputLifecycleError::WrongController {
                active: first,
                incoming: second,
                event_kind: RfbInputEventKind::Key,
            })
        );

        let wrong_disconnect = pump
            .handle_event(RfbServerEvent::Disconnected {
                client_id: second,
                peer_addr: second_peer,
                reason: RfbDisconnectReason::ClientClosed,
            })
            .unwrap_err();
        assert!(matches!(
            wrong_disconnect.error(),
            RfbInputError::Lifecycle(RfbInputLifecycleError::WrongController {
                event_kind: RfbInputEventKind::Disconnected,
                ..
            })
        ));

        let changed_peer = pump
            .handle_event(RfbServerEvent::Disconnected {
                client_id: first,
                peer_addr: second_peer,
                reason: RfbDisconnectReason::ClientClosed,
            })
            .unwrap_err();
        assert_eq!(
            changed_peer.error(),
            &RfbInputError::Lifecycle(RfbInputLifecycleError::PeerAddressChanged {
                client_id: first,
                expected: first_peer,
                actual: second_peer,
            })
        );
        assert_eq!(pump.active_client(), Some(first));
        assert_eq!(pump.sink().release_count, 0);
    }

    #[tokio::test]
    async fn pre_handshake_disconnect_does_not_release_input() {
        let client_id = client(6);
        let peer_addr = peer(5905);
        let mut pump = RfbInputPump::new(RecordingSink::default());

        assert_eq!(
            pump.handle_event(RfbServerEvent::Disconnected {
                client_id,
                peer_addr,
                reason: RfbDisconnectReason::HandshakeTimeout,
            }),
            Ok(RfbInputNotice::PreHandshakeDisconnected {
                client_id,
                peer_addr,
                reason: RfbDisconnectReason::HandshakeTimeout,
            })
        );
        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().release_count, 0);
    }

    #[tokio::test]
    async fn disconnect_releases_input_and_resets_mappers_for_the_next_controller() {
        let first = client(7);
        let second = client(8);
        let first_peer = peer(5906);
        let second_peer = peer(5907);
        let mut pump = RfbInputPump::new(RecordingSink::default());

        pump.handle_event(connected(first, first_peer)).unwrap();
        pump.handle_event(RfbServerEvent::Key {
            client_id: first,
            down: true,
            keysym: 0x61,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::Pointer {
            client_id: first,
            button_mask: 1,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        })
        .unwrap();

        assert_eq!(
            pump.handle_event(RfbServerEvent::Disconnected {
                client_id: first,
                peer_addr: first_peer,
                reason: RfbDisconnectReason::ClientClosed,
            }),
            Ok(RfbInputNotice::ControllerReleased {
                client_id: first,
                peer_addr: first_peer,
                reason: RfbControllerReleaseReason::Disconnected(RfbDisconnectReason::ClientClosed),
            })
        );
        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().release_count, 1);

        pump.handle_event(connected(second, second_peer)).unwrap();
        pump.handle_event(RfbServerEvent::Key {
            client_id: second,
            down: true,
            keysym: 0x61,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::Pointer {
            client_id: second,
            button_mask: 1,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        })
        .unwrap();

        assert_eq!(pump.sink().key_batches.len(), 2);
        assert_eq!(pump.sink().pointer_batches.len(), 2);
    }

    #[tokio::test]
    async fn failed_disconnect_release_keeps_the_event_and_controller_for_retry() {
        let client_id = client(9);
        let peer_addr = peer(5908);
        let mut pump = RfbInputPump::new(RecordingSink {
            fail_next_release: true,
            ..RecordingSink::default()
        });
        pump.handle_event(connected(client_id, peer_addr)).unwrap();
        pump.handle_event(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x61,
        })
        .unwrap();
        let disconnect = RfbServerEvent::Disconnected {
            client_id,
            peer_addr,
            reason: RfbDisconnectReason::ClientClosed,
        };

        let error = pump.handle_event(disconnect.clone()).unwrap_err();
        assert_eq!(error.event(), &disconnect);
        assert!(matches!(
            error.error(),
            RfbInputError::Sink {
                operation: RfbInputOperation::Release,
                ..
            }
        ));
        assert_eq!(pump.active_client(), Some(client_id));
        assert_eq!(pump.sink().release_count, 0);

        let (retry, _) = error.into_parts();
        assert!(pump.handle_event(retry).is_ok());
        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().release_count, 1);
    }

    #[tokio::test]
    async fn channel_close_releases_an_active_controller_after_draining_events() {
        let client_id = client(10);
        let peer_addr = peer(5909);
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        tx.send(connected(client_id, peer_addr)).await.unwrap();
        tx.send(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x61,
        })
        .await
        .unwrap();
        drop(tx);

        let mut pump = RfbInputPump::new(RecordingSink::default());
        let mut notices = Vec::new();
        pump.run(&mut rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap();

        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().key_batches.len(), 1);
        assert_eq!(pump.sink().release_count, 1);
        assert!(matches!(
            notices.last(),
            Some(RfbInputNotice::ControllerReleased {
                reason: RfbControllerReleaseReason::EventSourceClosed,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn channel_close_release_failure_can_be_retried_explicitly() {
        let client_id = client(11);
        let peer_addr = peer(5910);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.send(connected(client_id, peer_addr)).await.unwrap();
        drop(tx);

        let mut pump = RfbInputPump::new(RecordingSink {
            fail_next_release: true,
            ..RecordingSink::default()
        });
        assert!(matches!(
            pump.run(&mut rx, |_| {}).await,
            Err(RfbInputRunError::SourceClosedRelease(RfbInputError::Sink {
                operation: RfbInputOperation::Release,
                ..
            }))
        ));
        assert_eq!(pump.active_client(), Some(client_id));

        assert!(matches!(
            pump.release_active(),
            Ok(Some(RfbInputNotice::ControllerReleased {
                reason: RfbControllerReleaseReason::Explicit,
                ..
            }))
        ));
        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().release_count, 1);
    }

    #[tokio::test]
    async fn run_returns_the_failed_event_and_leaves_later_events_in_the_channel() {
        let client_id = client(12);
        let peer_addr = peer(5911);
        let failed_key = RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x61,
        };
        let later_pointer = RfbServerEvent::Pointer {
            client_id,
            button_mask: 0,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(3);
        tx.send(connected(client_id, peer_addr)).await.unwrap();
        tx.send(failed_key.clone()).await.unwrap();
        tx.send(later_pointer).await.unwrap();
        drop(tx);

        let mut pump = RfbInputPump::new(RecordingSink {
            fail_next_key: true,
            ..RecordingSink::default()
        });
        let mut notices = Vec::new();
        let error = pump
            .run(&mut rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap_err();
        let RfbInputRunError::Event(error) = error else {
            panic!("expected an event processing error");
        };
        assert_eq!(error.event(), &failed_key);
        assert_eq!(rx.len(), 1);
        assert_eq!(pump.active_client(), Some(client_id));

        let (retry, _) = error.into_parts();
        notices.push(pump.handle_event(retry).unwrap());
        pump.run(&mut rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap();

        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().key_batches.len(), 1);
        assert_eq!(pump.sink().pointer_batches.len(), 1);
        assert_eq!(pump.sink().release_count, 1);
        assert!(matches!(
            notices.as_slice(),
            [
                RfbInputNotice::ControllerAcquired { .. },
                RfbInputNotice::Keyboard { .. },
                RfbInputNotice::Pointer { .. },
                RfbInputNotice::ControllerReleased {
                    reason: RfbControllerReleaseReason::EventSourceClosed,
                    ..
                },
            ]
        ));
    }

    #[tokio::test]
    async fn run_until_stopped_exits_and_releases_on_stop_signal() {
        let client_id = client(19);
        let peer_addr = peer(5918);
        let sink = RecordingSink::default();
        let mut pump = RfbInputPump::new(sink.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        // 测试 sink 是值拷贝，driver 无法直接观察到 pump 的写入，用共享标志
        // 表示「键盘事件已被 pump 处理」。
        let key_seen = Arc::new(AtomicBool::new(false));

        let driver = tokio::spawn({
            let key_seen = Arc::clone(&key_seen);
            async move {
                tx.send(connected(client_id, peer_addr)).await.unwrap();
                tx.send(RfbServerEvent::Key {
                    client_id,
                    down: true,
                    keysym: 0x61,
                })
                .await
                .unwrap();
                // 事件到达后再触发停止；发送端保持存活，证明停止不依赖 channel 关闭。
                loop {
                    if key_seen.load(Ordering::SeqCst) {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                stop_tx.send(true).unwrap();
            }
        });

        let mut notices = Vec::new();
        pump.run_until_stopped(&mut rx, stop_rx, {
            let key_seen = Arc::clone(&key_seen);
            let notices = &mut notices;
            move |notice: &RfbInputNotice| {
                notices.push(notice.clone());
                if matches!(notice, RfbInputNotice::Keyboard { .. }) {
                    key_seen.store(true, Ordering::SeqCst);
                }
            }
        })
        .await
        .unwrap();
        driver.await.unwrap();

        assert_eq!(pump.active_client(), None);
        assert_eq!(pump.sink().release_count, 1);
        assert!(matches!(
            notices.last(),
            Some(RfbInputNotice::ControllerReleased {
                reason: RfbControllerReleaseReason::Explicit,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn run_until_stopped_with_pre_set_stop_signal_returns_without_events() {
        let mut pump = RfbInputPump::new(RecordingSink::default());
        let (_tx, mut rx) = tokio::sync::mpsc::channel(1);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        stop_tx.send(true).unwrap();

        let mut notices = Vec::new();
        pump.run_until_stopped(&mut rx, stop_rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap();

        assert!(notices.is_empty());
        assert_eq!(pump.sink().release_count, 0);
    }

    #[tokio::test]
    async fn real_ch9329_sink_orders_input_and_final_release_batches() {
        let client_id = client(13);
        let peer_addr = peer(5912);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);

        pump.handle_event(connected(client_id, peer_addr)).unwrap();
        pump.handle_event(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x61,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::Pointer {
            client_id,
            button_mask: 1,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::Disconnected {
            client_id,
            peer_addr,
            reason: RfbDisconnectReason::ClientClosed,
        })
        .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].frames().len(), 1);
        assert_eq!(batches[0].frames()[0].command(), 0x02);
        // Pointer batch：严格绝对模式下移动与左键按下都使用 0x04。
        assert_eq!(batches[1].frames().len(), 2);
        assert_eq!(batches[1].frames()[0].command(), 0x04);
        assert_eq!(batches[1].frames()[0].data()[1], 0);
        assert_eq!(batches[1].frames()[1].command(), 0x04);
        assert_eq!(batches[1].frames()[1].data()[1], 1);
        // 最终释放 batch：键盘全 0 + 鼠标绝对 buttons=0。
        let release = batches[2].frames();
        assert_eq!(release.len(), 2);
        assert_eq!(release[0].data(), &[0; 8]);
        assert_eq!(release[1].command(), 0x04);
        assert_eq!(release[1].data()[1], 0);
        assert_eq!(pump.active_client(), None);
    }

    #[tokio::test]
    async fn real_ch9329_release_failure_rolls_back_sink_and_pump_together() {
        let client_id = client(14);
        let peer_addr = peer(5913);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        pump.handle_event(connected(client_id, peer_addr)).unwrap();
        pump.handle_event(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x61,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::Pointer {
            client_id,
            button_mask: 1,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        })
        .unwrap();
        let disconnect = RfbServerEvent::Disconnected {
            client_id,
            peer_addr,
            reason: RfbDisconnectReason::ClientClosed,
        };
        queue.fail_next(CommandQueueError::Closed);

        let error = pump.handle_event(disconnect).unwrap_err();
        assert!(matches!(
            error.error(),
            RfbInputError::Sink {
                operation: RfbInputOperation::Release,
                source: InputError::CommandQueue(CommandQueueError::Closed),
                ..
            }
        ));
        assert_eq!(queue.accepted_batches().len(), 2);
        assert_eq!(pump.active_client(), Some(client_id));

        let (retry, _) = error.into_parts();
        pump.handle_event(retry).unwrap();
        assert_eq!(queue.accepted_batches().len(), 3);
        assert_eq!(pump.active_client(), None);

        let second = client(15);
        pump.handle_event(connected(second, peer(5914))).unwrap();
        assert_eq!(pump.active_client(), Some(second));
    }

    #[tokio::test]
    async fn real_ch9329_pointer_uses_the_size_carried_by_the_tcp_event() {
        let client_id = client(16);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        pump.handle_event(connected(client_id, peer(5915))).unwrap();

        pump.handle_event(RfbServerEvent::Pointer {
            client_id,
            button_mask: 0,
            x: 199,
            y: 99,
            framebuffer_size: RfbSize::new(200, 100).unwrap(),
        })
        .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            &batches[0].frames()[0].data()[2..6],
            &[0xeb, 0x0f, 0xd7, 0x0f]
        );
    }

    #[tokio::test]
    async fn real_ch9329_absolute_drag_carries_button_on_move_frames() {
        let client_id = client(25);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        let size = RfbSize::new(100, 100).unwrap();
        pump.handle_event(connected(client_id, peer(5925))).unwrap();

        for (button_mask, x, y) in [(0, 10, 20), (1, 10, 20), (1, 20, 30), (0, 20, 30)] {
            pump.handle_event(RfbServerEvent::Pointer {
                client_id,
                button_mask,
                x,
                y,
                framebuffer_size: size,
            })
            .unwrap();
        }

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 4);
        assert!(
            batches
                .iter()
                .flat_map(|batch| batch.frames())
                .all(|frame| frame.command() == 0x04),
            "绝对拖拽路径不得混入 0x05 相对鼠标帧"
        );
        assert_eq!(batches[0].frames()[0].data()[1], 0);
        assert_eq!(batches[1].frames()[1].data()[1], 1);
        assert_eq!(
            batches[2].frames()[0].data()[1],
            1,
            "按住移动的绝对帧必须继续携带按钮位"
        );
        assert_eq!(batches[3].frames()[1].data()[1], 0);
    }

    #[tokio::test]
    async fn real_ch9329_absolute_wheel_stays_absolute_and_allows_later_moves() {
        let client_id = client(26);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        let size = RfbSize::new(100, 100).unwrap();
        pump.handle_event(connected(client_id, peer(5926))).unwrap();

        for (button_mask, x, y) in [(0, 10, 20), (0x08, 10, 20), (0, 10, 20), (0, 20, 30)] {
            pump.handle_event(RfbServerEvent::Pointer {
                client_id,
                button_mask,
                x,
                y,
                framebuffer_size: size,
            })
            .unwrap();
        }

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 4);
        assert!(
            batches
                .iter()
                .flat_map(|batch| batch.frames())
                .all(|frame| frame.command() == 0x04),
            "RFB 绝对滚轮必须保持 0x04 绝对鼠标帧，不能切成 0x05 相对帧"
        );
        assert_eq!(
            batches[1].frames()[1].data()[6] as i8,
            1,
            "RFB wheel-up 边沿必须进入绝对帧 wheel 字段"
        );
        assert_eq!(
            batches[3].frames()[0].data()[1],
            0,
            "滚轮释放后后续移动仍应作为普通绝对移动发出"
        );
    }

    #[tokio::test]
    async fn explicit_mode_switch_releases_old_buttons_before_relative_motion() {
        let client_id = client(27);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        pump.handle_event(connected(client_id, peer(5927))).unwrap();

        pump.handle_event(RfbServerEvent::Pointer {
            client_id,
            button_mask: 1,
            x: 10,
            y: 20,
            framebuffer_size: RfbSize::new(100, 100).unwrap(),
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::SetMouseMode {
            client_id,
            mode: MouseMode::Relative,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::PointerRelative {
            client_id,
            button_mask: 1,
            dx: 4,
            dy: -2,
            wheel: 0,
        })
        .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].frames()[1].command(), 0x04);
        assert_eq!(batches[0].frames()[1].data()[1], 1);
        assert_eq!(
            batches[1].frames().last().unwrap().command(),
            0x04,
            "切换相对模式前必须先用旧绝对模式释放按钮"
        );
        assert_eq!(batches[1].frames().last().unwrap().data()[1], 0);
        let relative_frames = batches[2].frames();
        assert_eq!(relative_frames.len(), 2);
        assert_eq!(relative_frames[0].command(), 0x05);
        assert_eq!(relative_frames[0].data(), &[0x01, 1, 0, 0, 0]);
        assert_eq!(relative_frames[1].command(), 0x05);
        assert_eq!(relative_frames[1].data(), &[0x01, 1, 4, -2i8 as u8, 0]);
    }

    #[tokio::test]
    async fn mouse_mode_switch_preserves_keyboard_until_key_release() {
        let client_id = client(28);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        pump.handle_event(connected(client_id, peer(5928))).unwrap();

        pump.handle_event(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 'a' as u32,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::SetMouseMode {
            client_id,
            mode: MouseMode::Relative,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::PointerRelative {
            client_id,
            button_mask: 0,
            dx: 1,
            dy: 0,
            wheel: 0,
        })
        .unwrap();
        pump.handle_event(RfbServerEvent::Key {
            client_id,
            down: false,
            keysym: 'a' as u32,
        })
        .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].frames()[0].command(), 0x02);
        assert_eq!(batches[0].frames()[0].data(), &[0, 0, 4, 0, 0, 0, 0, 0]);
        assert_eq!(
            batches[1].frames()[0].command(),
            0x05,
            "鼠标模式切换不得通过 release_all 提前释放键盘"
        );
        assert_eq!(batches[2].frames()[0].command(), 0x02);
        assert_eq!(batches[2].frames()[0].data(), &[0; 8]);
    }

    #[tokio::test]
    async fn ch9329_relative_mode_ignores_absolute_pointer_transition() {
        let client_id = client(19);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        let mut pump = RfbInputPump::new(sink);
        pump.handle_event(connected(client_id, peer(5919))).unwrap();

        assert!(matches!(
            pump.handle_event(RfbServerEvent::Pointer {
                client_id,
                button_mask: 0,
                x: 10,
                y: 20,
                framebuffer_size: RfbSize::new(100, 100).unwrap(),
            }),
            Ok(RfbInputNotice::Pointer {
                outcome: RfbPointerOutcome::IgnoredForMouseMode {
                    mode: MouseMode::Relative,
                },
                ..
            })
        ));
        assert!(
            queue.accepted_batches().is_empty(),
            "relative mode must ignore the pre-lock absolute pointer event"
        );
        pump.handle_event(RfbServerEvent::PointerRelative {
            client_id,
            button_mask: 0,
            dx: 3,
            dy: -2,
            wheel: 0,
        })
        .expect("a later pointer-lock event must switch the sink back to relative");

        assert!(
            !queue.accepted_batches().is_empty(),
            "a later relative pointer event must reach CH9329"
        );
        assert!(
            queue
                .accepted_batches()
                .iter()
                .flat_map(|batch| batch.frames())
                .all(|frame| frame.command() != 0x04),
            "relative mode must not emit an absolute CH9329 frame"
        );
    }

    #[tokio::test]
    async fn routes_relative_pointer_events_after_connect() {
        let client_id = client(20);
        let peer_addr = peer(5920);
        let mut pump = RfbInputPump::new(RecordingSink::default());
        pump.handle_event(connected(client_id, peer_addr)).unwrap();

        assert_eq!(
            pump.handle_event(RfbServerEvent::PointerRelative {
                client_id,
                button_mask: 1,
                dx: 12,
                dy: -4,
                wheel: 2,
            }),
            Ok(RfbInputNotice::Pointer {
                client_id,
                outcome: RfbPointerOutcome::Applied,
            })
        );
        assert_eq!(
            pump.sink().pointer_batches,
            vec![vec![
                PointerEvent::Button {
                    button: PointerButton::Left,
                    down: true,
                },
                PointerEvent::RelativeMove { dx: 12, dy: -4 },
                PointerEvent::Wheel { delta: 2 },
            ]]
        );
    }

    #[tokio::test]
    async fn real_ch9329_relative_buttons_use_protocol_button_bits() {
        let client_id = client(21);
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        let mut pump = RfbInputPump::new(sink);
        pump.handle_event(connected(client_id, peer(5921))).unwrap();

        for (rfb_mask, ch9329_mask) in [(0b001, 0x01), (0b100, 0x02), (0b010, 0x04)] {
            pump.handle_event(RfbServerEvent::PointerRelative {
                client_id,
                button_mask: rfb_mask,
                dx: 0,
                dy: 0,
                wheel: 0,
            })
            .unwrap();
            pump.handle_event(RfbServerEvent::PointerRelative {
                client_id,
                button_mask: 0,
                dx: 0,
                dy: 0,
                wheel: 0,
            })
            .unwrap();

            let batches = queue.accepted_batches();
            let press = batches[batches.len() - 2].frames();
            assert_eq!(press.len(), 1);
            assert_eq!(press[0].command(), 0x05);
            assert_eq!(press[0].data(), &[0x01, ch9329_mask, 0, 0, 0]);

            let release = batches[batches.len() - 1].frames();
            assert_eq!(release.len(), 1);
            assert_eq!(release[0].command(), 0x05);
            assert_eq!(release[0].data(), &[0x01, 0, 0, 0, 0]);
        }
    }

    #[tokio::test]
    async fn relative_pointer_without_active_controller_is_rejected() {
        let client_id = client(24);
        let mut pump = RfbInputPump::new(RecordingSink::default());

        let error = pump
            .handle_event(RfbServerEvent::PointerRelative {
                client_id,
                button_mask: 0,
                dx: 1,
                dy: 0,
                wheel: 0,
            })
            .unwrap_err();
        assert!(matches!(
            error.error(),
            RfbInputError::Lifecycle(RfbInputLifecycleError::NoActiveController {
                event_kind: RfbInputEventKind::PointerRelative,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn set_mouse_mode_routes_to_sink_and_notifies() {
        let client_id = client(22);
        let peer_addr = peer(5922);
        let mut pump = RfbInputPump::new(RecordingSink::default());
        pump.handle_event(connected(client_id, peer_addr)).unwrap();

        assert_eq!(
            pump.handle_event(RfbServerEvent::SetMouseMode {
                client_id,
                mode: MouseMode::Absolute,
            }),
            Ok(RfbInputNotice::MouseModeChanged {
                client_id,
                mode: MouseMode::Absolute,
            })
        );
        assert_eq!(
            pump.sink().release_count,
            0,
            "鼠标模式切换不能借 release_all 影响键盘和非鼠标状态"
        );
        assert_eq!(pump.sink().mouse_modes, vec![MouseMode::Absolute]);
    }

    #[tokio::test]
    async fn confirmed_mouse_mode_is_published_only_after_sink_update() {
        let client_id = client(23);
        let peer_addr = peer(5923);
        let (mode_tx, mode_rx) = watch::channel(None);
        let mut pump =
            RfbInputPump::with_mouse_mode_observer(RecordingSink::default(), Some(mode_tx));
        pump.handle_event(connected(client_id, peer_addr)).unwrap();
        assert_eq!(*mode_rx.borrow(), None);

        pump.handle_event(RfbServerEvent::SetMouseMode {
            client_id,
            mode: MouseMode::Relative,
        })
        .unwrap();

        assert_eq!(*mode_rx.borrow(), Some(MouseMode::Relative));
    }
}
