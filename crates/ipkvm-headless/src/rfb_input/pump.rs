use std::net::SocketAddr;

use ipkvm_core::{FramebufferSize, InputError, InputSink};
use ipkvm_rfb::RfbRectangle;
use thiserror::Error;
use tokio::sync::mpsc;

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
    CutTextIgnored {
        client_id: RfbClientId,
        byte_count: usize,
    },
    ContinuousUpdatesIgnored {
        client_id: RfbClientId,
        enable: bool,
        rectangle: RfbRectangle,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbInputEventKind {
    Connected,
    Key,
    Pointer,
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
    #[error("input sink rejected {operation:?} for RFB client {client_id:?}")]
    Sink {
        client_id: RfbClientId,
        operation: RfbInputOperation,
        #[source]
        source: InputError,
    },
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveController {
    client_id: RfbClientId,
    peer_addr: SocketAddr,
}

#[derive(Debug)]
pub struct RfbInputPump<S> {
    sink: S,
    active: Option<ActiveController>,
    keyboard: RfbKeyboardMapper,
    pointer: RfbPointerMapper,
}

impl<S: InputSink> RfbInputPump<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            active: None,
            keyboard: RfbKeyboardMapper::new(),
            pointer: RfbPointerMapper::new(),
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
        while let Some(event) = receiver.recv().await {
            let notice = self.handle_event(event)?;
            observe(&notice);
        }
        if let Some(notice) = self
            .release_with_reason(RfbControllerReleaseReason::EventSourceClosed)
            .map_err(RfbInputRunError::SourceClosedRelease)?
        {
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
            RfbServerEvent::CutText { client_id, bytes } => {
                self.require_active(*client_id, RfbInputEventKind::CutText)?;
                Ok(RfbInputNotice::CutTextIgnored {
                    client_id: *client_id,
                    byte_count: bytes.len(),
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
        self.sink
            .release_all()
            .map_err(|source| RfbInputError::Sink {
                client_id: active.client_id,
                operation: RfbInputOperation::Release,
                source,
            })?;
        self.active = None;
        self.keyboard = RfbKeyboardMapper::new();
        self.pointer = RfbPointerMapper::new();
        Ok(Some(RfbInputNotice::ControllerReleased {
            client_id: active.client_id,
            peer_addr: active.peer_addr,
            reason,
        }))
    }
}

#[cfg(test)]
mod tests {
    use ipkvm_core::{
        Ch9329InputSink, CommandQueueError, FramebufferSize, InputResult, KeyEvent, MouseMode,
        PointerButton, PointerEvent, fake_serial::FakeCommandQueue,
    };
    use ipkvm_rfb::RfbSize;

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingSink {
        key_batches: Vec<Vec<KeyEvent>>,
        pointer_batches: Vec<Vec<PointerEvent>>,
        release_count: usize,
        fail_next_key: bool,
        fail_next_pointer: bool,
        fail_next_release: bool,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
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

    #[test]
    fn routes_controller_keyboard_and_pointer_events() {
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

    #[test]
    fn reports_client_rejections_and_ignored_events_without_stopping() {
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
            Ok(RfbInputNotice::CutTextIgnored {
                client_id,
                byte_count: 3,
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

    #[test]
    fn sink_failures_return_the_original_event_and_can_be_retried() {
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

    #[test]
    fn rejects_events_that_break_the_single_controller_lifecycle() {
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

    #[test]
    fn pre_handshake_disconnect_does_not_release_input() {
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

    #[test]
    fn disconnect_releases_input_and_resets_mappers_for_the_next_controller() {
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

    #[test]
    fn failed_disconnect_release_keeps_the_event_and_controller_for_retry() {
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

    #[test]
    fn real_ch9329_sink_orders_input_and_final_release_batches() {
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
        assert_eq!(batches[1].frames().len(), 2);
        assert_eq!(batches[1].frames()[0].data()[1], 0);
        assert_eq!(batches[1].frames()[1].data()[1], 1);
        let release = batches[2].frames();
        assert_eq!(release.len(), 2);
        assert_eq!(release[0].data(), &[0; 8]);
        assert_eq!(release[1].command(), 0x04);
        assert_eq!(release[1].data()[1], 0);
        assert_eq!(pump.active_client(), None);
    }

    #[test]
    fn real_ch9329_release_failure_rolls_back_sink_and_pump_together() {
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

    #[test]
    fn real_ch9329_pointer_uses_the_size_carried_by_the_tcp_event() {
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
}
