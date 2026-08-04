use crate::protocol::client::{ClientMessage, ClientMessageDecoder};
use crate::protocol::server::{
    AUTH_FAILED_REASON, NONE_SECURITY_TYPES, PROTOCOL_VERSION, SECURITY_RESULT_FAILED,
    SECURITY_RESULT_OK, VNC_SECURITY_TYPES, checked_output_len, encode_desktop_size_update,
    encode_empty_update, encode_raw_update, encode_server_init,
};
use crate::security::{RfbSecurity, constant_time_eq, vnc_expected_response};
use crate::{
    BgraFrameView, FramebufferUpdateRequest, RfbPixelFormat, RfbProtocolError, RfbProtocolLimits,
    RfbRectangle, RfbSize,
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbConnectionConfig {
    pub desktop_name: String,
    pub initial_size: RfbSize,
    pub limits: RfbProtocolLimits,
    pub security: RfbSecurity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbConnectionState {
    AwaitingVersion,
    AwaitingSecuritySelection,
    AwaitingChallengeResponse,
    AwaitingClientInit,
    Normal,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbEvent {
    HandshakeCompleted {
        shared: bool,
    },
    FramebufferUpdateRequested(FramebufferUpdateRequest),
    Key {
        down: bool,
        keysym: u32,
    },
    Pointer {
        button_mask: u8,
        x: u16,
        y: u16,
        framebuffer_size: RfbSize,
    },
    PointerRelative {
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    },
    CutText(Vec<u8>),
    EnableContinuousUpdates {
        enable: bool,
        rectangle: RfbRectangle,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramebufferUpdateOutcome {
    RawQueued { rectangle: RfbRectangle },
    EmptyQueued,
    ResizeAnnounced { size: RfbSize },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbEncodeError {
    #[error("framebuffer spans {actual} bytes, maximum is {maximum}")]
    FramebufferTooLarge { actual: usize, maximum: usize },
    #[error("framebuffer update length overflow")]
    LengthOverflow,
    #[error("output queue would grow to {attempted} bytes, maximum is {maximum}")]
    OutputQueueFull { attempted: usize, maximum: usize },
    #[error("RFB handshake is not complete")]
    HandshakeNotComplete,
    #[error("framebuffer changed from {announced:?} to {actual:?} without DesktopSize negotiation")]
    DesktopSizeNotNegotiated { announced: RfbSize, actual: RfbSize },
}

pub struct RfbConnectionCore {
    config: RfbConnectionConfig,
    state: RfbConnectionState,
    vnc_challenge: [u8; 16],
    handshake_input: Vec<u8>,
    client_decoder: ClientMessageDecoder,
    output: Vec<u8>,
    server_init: Vec<u8>,
    pixel_format: RfbPixelFormat,
    encoding_preferences: Vec<i32>,
    announced_size: RfbSize,
    input_coordinate_size: RfbSize,
    pending_input_size: Option<RfbSize>,
}

impl RfbConnectionCore {
    pub fn new(config: RfbConnectionConfig) -> Result<Self, RfbConfigError> {
        validate_config(&config)?;
        let mut vnc_challenge = [0_u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut vnc_challenge);
        let pixel_format = RfbPixelFormat::default_bgrx8888();
        let server_init =
            encode_server_init(config.initial_size, pixel_format, &config.desktop_name)?;
        let announced_size = config.initial_size;
        let input_coordinate_size = config.initial_size;
        Ok(Self {
            client_decoder: ClientMessageDecoder::new(config.limits),
            config,
            state: RfbConnectionState::AwaitingVersion,
            vnc_challenge,
            handshake_input: Vec::new(),
            output: PROTOCOL_VERSION.to_vec(),
            server_init,
            pixel_format,
            encoding_preferences: Vec::new(),
            announced_size,
            input_coordinate_size,
            pending_input_size: None,
        })
    }

    pub fn push_input(&mut self, bytes: &[u8]) -> Vec<Result<RfbEvent, RfbProtocolError>> {
        if self.state == RfbConnectionState::Failed {
            return vec![Err(RfbProtocolError::ConnectionFailed)];
        }
        if self.state == RfbConnectionState::Normal {
            return self.push_normal_input(bytes);
        }

        let Some(attempted) = self.handshake_input.len().checked_add(bytes.len()) else {
            return self.fail(RfbProtocolError::LengthOverflow);
        };
        if attempted > self.config.limits.max_buffered_input_bytes {
            return self.fail(RfbProtocolError::InputBufferLimitExceeded {
                attempted,
                maximum: self.config.limits.max_buffered_input_bytes,
            });
        }
        self.handshake_input.extend_from_slice(bytes);

        let mut results = Vec::new();
        loop {
            match self.state {
                RfbConnectionState::AwaitingVersion => {
                    if self.handshake_input.len() < PROTOCOL_VERSION.len() {
                        break;
                    }
                    let mut version = [0_u8; 12];
                    version.copy_from_slice(&self.handshake_input[..12]);
                    self.handshake_input.drain(..12);
                    if &version != PROTOCOL_VERSION {
                        results.extend(self.fail(RfbProtocolError::UnsupportedVersion(version)));
                        break;
                    }
                    self.output.extend_from_slice(match &self.config.security {
                        RfbSecurity::None => &NONE_SECURITY_TYPES,
                        RfbSecurity::Vnc { .. } => &VNC_SECURITY_TYPES,
                    });
                    self.state = RfbConnectionState::AwaitingSecuritySelection;
                }
                RfbConnectionState::AwaitingSecuritySelection => {
                    let Some(selection) = self.handshake_input.first().copied() else {
                        break;
                    };
                    self.handshake_input.drain(..1);
                    match (selection, &self.config.security) {
                        (1, RfbSecurity::None) => {
                            self.output.extend_from_slice(&SECURITY_RESULT_OK);
                            self.state = RfbConnectionState::AwaitingClientInit;
                        }
                        (2, RfbSecurity::Vnc { .. }) => {
                            self.output.extend_from_slice(&self.vnc_challenge);
                            self.state = RfbConnectionState::AwaitingChallengeResponse;
                        }
                        _ => {
                            results.extend(
                                self.fail(RfbProtocolError::UnsupportedSecurityType(selection)),
                            );
                            break;
                        }
                    }
                }
                RfbConnectionState::AwaitingChallengeResponse => {
                    if self.handshake_input.len() < 16 {
                        break;
                    }
                    let mut response = [0_u8; 16];
                    response.copy_from_slice(&self.handshake_input[..16]);
                    self.handshake_input.drain(..16);
                    let RfbSecurity::Vnc { password } = self.config.security else {
                        unreachable!("AwaitingChallengeResponse 只在 Vnc 配置下进入");
                    };
                    let expected = vnc_expected_response(password, self.vnc_challenge);
                    if constant_time_eq(&expected, &response) {
                        self.output.extend_from_slice(&SECURITY_RESULT_OK);
                        self.state = RfbConnectionState::AwaitingClientInit;
                    } else {
                        // RFC 6143 §7.2.3：失败发 SecurityResultFailed + 原因，
                        // 随后连接关闭（状态置 Failed）。
                        let reason = AUTH_FAILED_REASON;
                        self.output.extend_from_slice(&SECURITY_RESULT_FAILED);
                        self.output
                            .extend_from_slice(&(reason.len() as u32).to_be_bytes());
                        self.output.extend_from_slice(reason);
                        results.extend(self.fail(RfbProtocolError::AuthenticationFailed));
                        break;
                    }
                }
                RfbConnectionState::AwaitingClientInit => {
                    let Some(shared) = self.handshake_input.first().copied() else {
                        break;
                    };
                    self.handshake_input.drain(..1);
                    self.output.extend_from_slice(&self.server_init);
                    self.state = RfbConnectionState::Normal;
                    results.push(Ok(RfbEvent::HandshakeCompleted {
                        shared: shared != 0,
                    }));

                    if !self.handshake_input.is_empty() {
                        let remaining = std::mem::take(&mut self.handshake_input);
                        results.extend(self.push_normal_input(&remaining));
                    }
                    break;
                }
                RfbConnectionState::Normal | RfbConnectionState::Failed => break,
            }
        }
        results
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    pub fn state(&self) -> RfbConnectionState {
        self.state
    }

    pub fn pixel_format(&self) -> RfbPixelFormat {
        self.pixel_format
    }

    pub fn encoding_preferences(&self) -> &[i32] {
        &self.encoding_preferences
    }

    pub fn supports_desktop_size(&self) -> bool {
        self.encoding_preferences.contains(&-223)
    }

    pub fn queue_framebuffer_update(
        &mut self,
        frame: BgraFrameView<'_>,
        request: FramebufferUpdateRequest,
    ) -> Result<FramebufferUpdateOutcome, RfbEncodeError> {
        if self.state != RfbConnectionState::Normal {
            return Err(RfbEncodeError::HandshakeNotComplete);
        }
        if frame.byte_span() > self.config.limits.max_framebuffer_bytes {
            return Err(RfbEncodeError::FramebufferTooLarge {
                actual: frame.byte_span(),
                maximum: self.config.limits.max_framebuffer_bytes,
            });
        }
        if frame.size() != self.announced_size {
            if !self.supports_desktop_size() {
                return Err(RfbEncodeError::DesktopSizeNotNegotiated {
                    announced: self.announced_size,
                    actual: frame.size(),
                });
            }
            let size = frame.size();
            self.queue_output(encode_desktop_size_update(size))?;
            self.announced_size = size;
            if self.client_decoder.is_idle() {
                self.input_coordinate_size = size;
                self.pending_input_size = None;
            } else {
                self.pending_input_size = Some(size);
            }
            return Ok(FramebufferUpdateOutcome::ResizeAnnounced { size });
        }

        let Some(rectangle) = request.rectangle.intersection(frame.size()) else {
            self.queue_output(encode_empty_update())?;
            return Ok(FramebufferUpdateOutcome::EmptyQueued);
        };
        let message = encode_raw_update(frame, rectangle, self.pixel_format)?;
        self.queue_output(message)?;
        Ok(FramebufferUpdateOutcome::RawQueued { rectangle })
    }

    fn push_normal_input(&mut self, bytes: &[u8]) -> Vec<Result<RfbEvent, RfbProtocolError>> {
        let input_coordinate_size = self.input_coordinate_size;
        let mut results = Vec::new();
        let mut failed = false;
        for decoded in self.client_decoder.push(bytes) {
            match decoded {
                Ok(ClientMessage::SetPixelFormat(format)) => self.pixel_format = format,
                Ok(ClientMessage::SetEncodings(encodings)) => {
                    self.encoding_preferences = encodings;
                }
                Ok(ClientMessage::FramebufferUpdateRequest(request)) => {
                    results.push(Ok(RfbEvent::FramebufferUpdateRequested(request)));
                }
                Ok(ClientMessage::Key { down, keysym }) => {
                    results.push(Ok(RfbEvent::Key { down, keysym }));
                }
                Ok(ClientMessage::Pointer { button_mask, x, y }) => {
                    results.push(Ok(RfbEvent::Pointer {
                        button_mask,
                        x,
                        y,
                        framebuffer_size: input_coordinate_size,
                    }));
                }
                Ok(ClientMessage::PointerRelative {
                    button_mask,
                    dx,
                    dy,
                    wheel,
                }) => {
                    results.push(Ok(RfbEvent::PointerRelative {
                        button_mask,
                        dx,
                        dy,
                        wheel,
                    }));
                }
                Ok(ClientMessage::CutText(bytes)) => {
                    results.push(Ok(RfbEvent::CutText(bytes)));
                }
                Ok(ClientMessage::EnableContinuousUpdates { enable, rectangle }) => {
                    results.push(Ok(RfbEvent::EnableContinuousUpdates { enable, rectangle }));
                }
                Err(error) => {
                    self.state = RfbConnectionState::Failed;
                    failed = true;
                    results.push(Err(error));
                    break;
                }
            }
        }
        if !failed
            && self.client_decoder.is_idle()
            && let Some(size) = self.pending_input_size.take()
        {
            self.input_coordinate_size = size;
        }
        results
    }

    fn fail(&mut self, error: RfbProtocolError) -> Vec<Result<RfbEvent, RfbProtocolError>> {
        self.state = RfbConnectionState::Failed;
        self.handshake_input.clear();
        vec![Err(error)]
    }

    fn queue_output(&mut self, message: Vec<u8>) -> Result<(), RfbEncodeError> {
        let attempted = checked_output_len(self.output.len(), message.len())?;
        if attempted > self.config.limits.max_queued_output_bytes {
            return Err(RfbEncodeError::OutputQueueFull {
                attempted,
                maximum: self.config.limits.max_queued_output_bytes,
            });
        }
        self.output.extend_from_slice(&message);
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbConfigError {
    #[error("protocol limit {0} must be non-zero")]
    ZeroLimit(&'static str),
    #[error("framebuffer limit {actual} cannot hold one BGRA pixel")]
    FramebufferLimitTooSmall { actual: usize },
    #[error("initial BGRA framebuffer requires {required} bytes, maximum is {maximum}")]
    InitialFramebufferTooLarge { required: usize, maximum: usize },
    #[error("desktop name has {actual} bytes, maximum is {maximum}")]
    DesktopNameTooLong { actual: usize, maximum: usize },
    #[error("input capacity is {actual} bytes, at least {required} are required")]
    InputCapacityTooSmall { actual: usize, required: usize },
    #[error("output capacity is {actual} bytes, at least {required} are required")]
    OutputCapacityTooSmall { actual: usize, required: usize },
    #[error("protocol limit calculation overflow")]
    LimitOverflow,
}

fn validate_config(config: &RfbConnectionConfig) -> Result<(), RfbConfigError> {
    let limits = config.limits;
    for (name, value) in [
        ("max_desktop_name_bytes", limits.max_desktop_name_bytes),
        ("max_encodings", limits.max_encodings),
        ("max_cut_text_bytes", limits.max_cut_text_bytes),
        ("max_buffered_input_bytes", limits.max_buffered_input_bytes),
        ("max_queued_output_bytes", limits.max_queued_output_bytes),
        ("max_framebuffer_bytes", limits.max_framebuffer_bytes),
    ] {
        if value == 0 {
            return Err(RfbConfigError::ZeroLimit(name));
        }
    }
    if limits.max_framebuffer_bytes < 4 {
        return Err(RfbConfigError::FramebufferLimitTooSmall {
            actual: limits.max_framebuffer_bytes,
        });
    }

    let name_length = config.desktop_name.len();
    if name_length > limits.max_desktop_name_bytes || u32::try_from(name_length).is_err() {
        return Err(RfbConfigError::DesktopNameTooLong {
            actual: name_length,
            maximum: limits.max_desktop_name_bytes.min(u32::MAX as usize),
        });
    }

    let initial_frame_bytes = usize::from(config.initial_size.width())
        .checked_mul(usize::from(config.initial_size.height()))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(RfbConfigError::LimitOverflow)?;
    if initial_frame_bytes > limits.max_framebuffer_bytes {
        return Err(RfbConfigError::InitialFramebufferTooLarge {
            required: initial_frame_bytes,
            maximum: limits.max_framebuffer_bytes,
        });
    }

    let cut_text_message = limits
        .max_cut_text_bytes
        .checked_add(8)
        .ok_or(RfbConfigError::LimitOverflow)?;
    let encodings_message = limits
        .max_encodings
        .checked_mul(4)
        .and_then(|body| body.checked_add(4))
        .ok_or(RfbConfigError::LimitOverflow)?;
    let required_input = cut_text_message.max(encodings_message).max(20);
    if limits.max_buffered_input_bytes < required_input {
        return Err(RfbConfigError::InputCapacityTooSmall {
            actual: limits.max_buffered_input_bytes,
            required: required_input,
        });
    }

    let raw_update = limits
        .max_framebuffer_bytes
        .checked_add(16)
        .ok_or(RfbConfigError::LimitOverflow)?;
    let handshake = 12_usize
        .checked_add(2)
        .and_then(|length| length.checked_add(4))
        .and_then(|length| length.checked_add(24))
        .and_then(|length| length.checked_add(name_length))
        .ok_or(RfbConfigError::LimitOverflow)?;
    let required_output = raw_update.max(handshake);
    if limits.max_queued_output_bytes < required_output {
        return Err(RfbConfigError::OutputCapacityTooSmall {
            actual: limits.max_queued_output_bytes,
            required: required_output,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn config_with_security(security: RfbSecurity) -> RfbConnectionConfig {
        RfbConnectionConfig {
            desktop_name: "my_ipkvm".to_owned(),
            initial_size: RfbSize::new(640, 480).unwrap(),
            limits: RfbProtocolLimits::default(),
            security,
        }
    }

    fn config() -> RfbConnectionConfig {
        config_with_security(RfbSecurity::None)
    }

    fn vnc_config() -> RfbConnectionConfig {
        config_with_security(RfbSecurity::Vnc {
            password: *b"12345678",
        })
    }

    /// 完整 VNC 密码握手：读 challenge 并生成响应（协议测试里直接用产品
    /// 实现生成响应是自证——这里特意用 des crate + 内联派生交叉验证）。
    fn vnc_response(password: &[u8; 8], challenge: &[u8; 16]) -> [u8; 16] {
        use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
        let mut key = [0_u8; 8];
        for (index, byte) in password.iter().copied().enumerate() {
            key[index] = byte.reverse_bits();
        }
        let cipher = des::Des::new_from_slice(&key).unwrap();
        let mut response = *challenge;
        for chunk in response.chunks_exact_mut(8) {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }
        response
    }

    #[test]
    fn vnc_handshake_succeeds_with_correct_password() {
        let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
        assert_eq!(connection.take_output(), b"RFB 003.008\n");

        assert!(connection.push_input(b"RFB 003.008\n").is_empty());
        assert_eq!(
            connection.state(),
            RfbConnectionState::AwaitingSecuritySelection
        );
        // 安全类型列表只提供 VNC（一个类型：2）。
        assert_eq!(connection.take_output(), [1, 2]);

        assert!(connection.push_input(&[2]).is_empty());
        assert_eq!(
            connection.state(),
            RfbConnectionState::AwaitingChallengeResponse
        );
        let challenge: [u8; 16] = connection.take_output().try_into().unwrap();

        let response = vnc_response(b"12345678", &challenge);
        assert!(connection.push_input(&response).is_empty());
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
        assert_eq!(connection.state(), RfbConnectionState::AwaitingClientInit);

        assert_eq!(
            connection.push_input(&[1]),
            vec![Ok(RfbEvent::HandshakeCompleted { shared: true })]
        );
        assert_eq!(connection.state(), RfbConnectionState::Normal);
        assert!(!connection.take_output().is_empty());
    }

    #[test]
    fn vnc_handshake_rejects_wrong_password() {
        let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
        connection.take_output();
        connection.push_input(b"RFB 003.008\n");
        connection.take_output();
        connection.push_input(&[2]);
        let challenge: [u8; 16] = connection.take_output().try_into().unwrap();

        let wrong = vnc_response(b"wrongpas", &challenge);
        assert_eq!(
            connection.push_input(&wrong),
            vec![Err(RfbProtocolError::AuthenticationFailed)]
        );
        assert_eq!(connection.state(), RfbConnectionState::Failed);
        // 失败响应：SecurityResultFailed + 原因字符串（RFC 6143 §7.2.3）。
        let output = connection.take_output();
        let mut expected = vec![0, 0, 0, 1];
        expected.extend_from_slice(&(21_u32).to_be_bytes());
        expected.extend_from_slice(b"authentication failed");
        assert_eq!(output, expected);
        assert_eq!(
            connection.push_input(&[1]),
            vec![Err(RfbProtocolError::ConnectionFailed)]
        );
    }

    #[test]
    fn vnc_response_accepted_across_arbitrary_chunks() {
        let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
        connection.take_output();
        connection.push_input(b"RFB 003.008\n");
        connection.take_output();
        connection.push_input(&[2]);
        let challenge: [u8; 16] = connection.take_output().try_into().unwrap();
        let response = vnc_response(b"12345678", &challenge);

        // 7 + 9 字节分块到达。
        assert!(connection.push_input(&response[..7]).is_empty());
        assert!(connection.push_input(&response[7..]).is_empty());
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
        assert_eq!(connection.state(), RfbConnectionState::AwaitingClientInit);
    }

    #[test]
    fn vnc_challenge_is_random_across_connections() {
        // 两个 core 的 challenge 必须不同（CSPRNG）。相同概率 2^-128，
        // 可忽略；该断言防「challenge 硬编码」回归。
        let first: [u8; 16] = {
            let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
            connection.take_output();
            connection.push_input(b"RFB 003.008\n");
            connection.take_output();
            connection.push_input(&[2]);
            connection.take_output().try_into().unwrap()
        };
        let second: [u8; 16] = {
            let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
            connection.take_output();
            connection.push_input(b"RFB 003.008\n");
            connection.take_output();
            connection.push_input(&[2]);
            connection.take_output().try_into().unwrap()
        };
        assert_ne!(first, second);
    }

    #[test]
    fn vnc_mode_rejects_none_selection_and_none_mode_rejects_vnc() {
        let mut vnc = RfbConnectionCore::new(vnc_config()).unwrap();
        vnc.take_output();
        vnc.push_input(b"RFB 003.008\n");
        vnc.take_output();
        assert_eq!(
            vnc.push_input(&[1]),
            vec![Err(RfbProtocolError::UnsupportedSecurityType(1))]
        );

        let mut none = RfbConnectionCore::new(config_with_security(RfbSecurity::None)).unwrap();
        none.take_output();
        none.push_input(b"RFB 003.008\n");
        none.take_output();
        assert_eq!(
            none.push_input(&[2]),
            vec![Err(RfbProtocolError::UnsupportedSecurityType(2))]
        );
    }

    #[test]
    fn rejects_internally_inconsistent_limits() {
        let mut input = config();
        input.limits.max_buffered_input_bytes = 20;
        input.limits.max_cut_text_bytes = 20;
        assert!(matches!(
            RfbConnectionCore::new(input),
            Err(RfbConfigError::InputCapacityTooSmall { .. })
        ));

        let mut output = config();
        output.initial_size = RfbSize::new(1, 1).unwrap();
        output.limits.max_framebuffer_bytes = 1024;
        output.limits.max_queued_output_bytes = 1024;
        assert!(matches!(
            RfbConnectionCore::new(output),
            Err(RfbConfigError::OutputCapacityTooSmall { .. })
        ));

        let mut initial_frame = config();
        initial_frame.limits.max_framebuffer_bytes = 1024;
        assert!(matches!(
            RfbConnectionCore::new(initial_frame),
            Err(RfbConfigError::InitialFramebufferTooLarge {
                required: 1_228_800,
                maximum: 1024,
            })
        ));
    }

    #[test]
    fn rejects_zero_limits_and_oversized_desktop_name() {
        let mut zero = config();
        zero.limits.max_encodings = 0;
        assert!(matches!(
            RfbConnectionCore::new(zero),
            Err(RfbConfigError::ZeroLimit("max_encodings"))
        ));

        let mut name = config();
        name.limits.max_desktop_name_bytes = 3;
        assert!(matches!(
            RfbConnectionCore::new(name),
            Err(RfbConfigError::DesktopNameTooLong { .. })
        ));
    }

    fn complete(config: RfbConnectionConfig) -> RfbConnectionCore {
        let mut connection = RfbConnectionCore::new(config).unwrap();
        assert_eq!(connection.take_output(), b"RFB 003.008\n");
        assert!(connection.push_input(b"RFB 003.008\n").is_empty());
        assert_eq!(connection.take_output(), [1, 1]);
        assert!(connection.push_input(&[1]).is_empty());
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
        assert!(matches!(
            connection.push_input(&[1]).as_slice(),
            [Ok(RfbEvent::HandshakeCompleted { shared: true })]
        ));
        assert!(!connection.take_output().is_empty());
        connection
    }

    fn completed_connection() -> RfbConnectionCore {
        complete(config())
    }

    fn config_with_size(width: u16, height: u16) -> RfbConnectionConfig {
        RfbConnectionConfig {
            desktop_name: "my_ipkvm".to_owned(),
            initial_size: RfbSize::new(width, height).unwrap(),
            limits: RfbProtocolLimits::default(),
            security: RfbSecurity::None,
        }
    }

    fn completed_connection_with_size(width: u16, height: u16) -> RfbConnectionCore {
        complete(config_with_size(width, height))
    }

    fn set_rgb565(connection: &mut RfbConnectionCore) {
        let format = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
        let mut message = vec![0, 0, 0, 0];
        message.extend_from_slice(&format.to_wire());
        assert!(connection.push_input(&message).is_empty());
        assert_eq!(connection.pixel_format(), format);
    }

    fn negotiate_desktop_size(connection: &mut RfbConnectionCore) {
        let mut message = vec![2, 0, 0, 1];
        message.extend_from_slice(&(-223_i32).to_be_bytes());
        assert!(connection.push_input(&message).is_empty());
        assert!(connection.supports_desktop_size());
    }

    fn queue_full_frame(
        connection: &mut RfbConnectionCore,
        frame: BgraFrameView<'_>,
    ) -> Result<FramebufferUpdateOutcome, RfbEncodeError> {
        let size = frame.size();
        connection.queue_framebuffer_update(
            frame,
            FramebufferUpdateRequest {
                incremental: false,
                rectangle: RfbRectangle {
                    x: 0,
                    y: 0,
                    width: size.width(),
                    height: size.height(),
                },
            },
        )
    }

    #[test]
    fn completes_rfb_38_none_handshake_across_arbitrary_chunks() {
        let mut connection = RfbConnectionCore::new(config()).unwrap();
        assert_eq!(connection.state(), RfbConnectionState::AwaitingVersion);
        assert_eq!(connection.take_output(), b"RFB 003.008\n");

        assert!(connection.push_input(b"RFB 003.").is_empty());
        assert!(connection.push_input(b"008\n").is_empty());
        assert_eq!(
            connection.state(),
            RfbConnectionState::AwaitingSecuritySelection
        );
        assert_eq!(connection.take_output(), [1, 1]);

        assert!(connection.push_input(&[1]).is_empty());
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
        assert_eq!(connection.state(), RfbConnectionState::AwaitingClientInit);

        assert_eq!(
            connection.push_input(&[1]),
            vec![Ok(RfbEvent::HandshakeCompleted { shared: true })]
        );
        assert_eq!(connection.state(), RfbConnectionState::Normal);
        let server_init = connection.take_output();
        assert_eq!(&server_init[..4], [0x02, 0x80, 0x01, 0xe0]);
    }

    #[test]
    fn rejects_other_versions_and_security_types() {
        let mut version = RfbConnectionCore::new(config()).unwrap();
        version.take_output();
        assert!(matches!(
            version.push_input(b"RFB 003.007\n").as_slice(),
            [Err(RfbProtocolError::UnsupportedVersion(_))]
        ));
        assert_eq!(version.state(), RfbConnectionState::Failed);

        let mut security = RfbConnectionCore::new(config()).unwrap();
        security.take_output();
        security.push_input(b"RFB 003.008\n");
        security.take_output();
        assert_eq!(
            security.push_input(&[2]),
            vec![Err(RfbProtocolError::UnsupportedSecurityType(2))]
        );
        assert_eq!(security.state(), RfbConnectionState::Failed);
    }

    #[test]
    fn handshake_input_limit_is_checked_before_append() {
        let mut limited = config();
        limited.limits.max_encodings = 1;
        limited.limits.max_cut_text_bytes = 1;
        limited.limits.max_buffered_input_bytes = 20;
        let mut connection = RfbConnectionCore::new(limited).unwrap();
        connection.take_output();

        assert_eq!(
            connection.push_input(&[0_u8; 21]),
            vec![Err(RfbProtocolError::InputBufferLimitExceeded {
                attempted: 21,
                maximum: 20,
            })]
        );
        assert_eq!(connection.state(), RfbConnectionState::Failed);
        assert!(connection.take_output().is_empty());
    }

    #[test]
    fn pipelined_bytes_continue_into_the_normal_decoder() {
        let mut connection = RfbConnectionCore::new(config()).unwrap();
        connection.take_output();
        let mut bytes = b"RFB 003.008\n".to_vec();
        bytes.extend_from_slice(&[1, 1]);
        bytes.extend_from_slice(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d]);

        assert_eq!(
            connection.push_input(&bytes),
            vec![
                Ok(RfbEvent::HandshakeCompleted { shared: true }),
                Ok(RfbEvent::Key {
                    down: true,
                    keysym: 0xff0d,
                }),
            ]
        );
        assert_eq!(connection.state(), RfbConnectionState::Normal);
    }

    #[test]
    fn completed_connection_helper_reaches_normal_state() {
        assert_eq!(completed_connection().state(), RfbConnectionState::Normal);
    }

    #[test]
    fn applies_negotiation_messages_and_emits_input_events() {
        let mut connection = completed_connection();
        let mut messages = vec![2, 0, 0, 2];
        messages.extend_from_slice(&0_i32.to_be_bytes());
        messages.extend_from_slice(&(-223_i32).to_be_bytes());
        messages.extend_from_slice(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d]);

        assert_eq!(
            connection.push_input(&messages),
            vec![Ok(RfbEvent::Key {
                down: true,
                keysym: 0xff0d,
            })]
        );
        assert_eq!(connection.encoding_preferences(), &[0, -223]);
        assert!(connection.supports_desktop_size());

        assert!(connection.push_input(&[2, 0, 0, 0]).is_empty());
        assert!(connection.encoding_preferences().is_empty());
        assert!(!connection.supports_desktop_size());

        let mut unknown_encodings = vec![2, 0, 0, 2];
        unknown_encodings.extend_from_slice(&12_345_i32.to_be_bytes());
        unknown_encodings.extend_from_slice(&(-313_i32).to_be_bytes());
        assert!(connection.push_input(&unknown_encodings).is_empty());
        assert_eq!(connection.encoding_preferences(), &[12_345, -313]);
        assert!(!connection.supports_desktop_size());
        assert!(connection.take_output().is_empty());
    }

    #[test]
    fn applies_pixel_format_and_emits_remaining_messages_in_order() {
        let mut connection = completed_connection();
        let format = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
        let mut messages = vec![0, 9, 8, 7];
        messages.extend_from_slice(&format.to_wire());
        messages.extend_from_slice(&[3, 0, 0, 1, 0, 2, 0, 3, 0, 4]);
        messages.extend_from_slice(&[5, 3, 0, 10, 0, 20]);
        messages.extend_from_slice(&[8, 3, 0x00, 0x0c, 0xff, 0xfc, 0x02]);
        messages.extend_from_slice(&[6, 0, 0, 0, 0, 0, 0, 2, 0x41, 0xff]);
        messages.extend_from_slice(&[150, 1, 0, 5, 0, 6, 0, 7, 0, 8]);

        assert_eq!(
            connection.push_input(&messages),
            vec![
                Ok(RfbEvent::FramebufferUpdateRequested(
                    FramebufferUpdateRequest {
                        incremental: false,
                        rectangle: RfbRectangle {
                            x: 1,
                            y: 2,
                            width: 3,
                            height: 4,
                        },
                    }
                )),
                Ok(RfbEvent::Pointer {
                    button_mask: 3,
                    x: 10,
                    y: 20,
                    framebuffer_size: RfbSize::new(640, 480).unwrap(),
                }),
                Ok(RfbEvent::PointerRelative {
                    button_mask: 3,
                    dx: 12,
                    dy: -4,
                    wheel: 2,
                }),
                Ok(RfbEvent::CutText(vec![0x41, 0xff])),
                Ok(RfbEvent::EnableContinuousUpdates {
                    enable: true,
                    rectangle: RfbRectangle {
                        x: 5,
                        y: 6,
                        width: 7,
                        height: 8,
                    },
                }),
            ]
        );
        assert_eq!(connection.pixel_format(), format);
    }

    #[test]
    fn preserves_events_before_a_fatal_message_and_then_stays_failed() {
        let mut connection = completed_connection();
        assert_eq!(
            connection.push_input(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d, 99]),
            vec![
                Ok(RfbEvent::Key {
                    down: true,
                    keysym: 0xff0d,
                }),
                Err(RfbProtocolError::UnsupportedClientMessageType(99)),
            ]
        );
        assert_eq!(connection.state(), RfbConnectionState::Failed);
        assert_eq!(
            connection.push_input(&[4, 0, 0, 0, 0, 0, 0xff, 0x0d]),
            vec![Err(RfbProtocolError::ConnectionFailed)]
        );
    }

    #[test]
    fn queues_cropped_raw_update_in_default_pixel_format() {
        let mut connection = completed_connection_with_size(2, 2);
        let pixels = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();

        assert_eq!(
            connection.queue_framebuffer_update(
                frame,
                FramebufferUpdateRequest {
                    incremental: true,
                    rectangle: RfbRectangle {
                        x: 1,
                        y: 0,
                        width: 1,
                        height: 2,
                    },
                },
            ),
            Ok(FramebufferUpdateOutcome::RawQueued {
                rectangle: RfbRectangle {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 2,
                },
            })
        );

        assert_eq!(
            connection.take_output(),
            [
                0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 2, 0, 0, 0, 0, 4, 5, 6, 0, 10, 11, 12, 0,
            ]
        );
    }

    #[test]
    fn raw_update_uses_stride_and_negotiated_rgb565() {
        let mut connection = completed_connection_with_size(2, 2);
        set_rgb565(&mut connection);
        let pixels = [
            0, 0, 255, 0, 0, 255, 0, 0, 99, 99, 99, 99, 255, 0, 0, 0, 255, 255, 255, 0,
        ];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 12, &pixels).unwrap();

        queue_full_frame(&mut connection, frame).unwrap();
        let output = connection.take_output();
        assert_eq!(
            &output[16..],
            [0x00, 0xf8, 0xe0, 0x07, 0x1f, 0x00, 0xff, 0xff]
        );
    }

    #[test]
    fn empty_intersection_queues_zero_rectangle_update() {
        let mut connection = completed_connection_with_size(2, 2);
        let pixels = [0_u8; 16];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();
        let outcome = connection
            .queue_framebuffer_update(
                frame,
                FramebufferUpdateRequest {
                    incremental: true,
                    rectangle: RfbRectangle {
                        x: 10,
                        y: 10,
                        width: 1,
                        height: 1,
                    },
                },
            )
            .unwrap();

        assert_eq!(outcome, FramebufferUpdateOutcome::EmptyQueued);
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
    }

    #[test]
    fn resize_requires_desktop_size_negotiation() {
        let mut connection = completed_connection_with_size(2, 2);
        let pixels = [0_u8; 24];
        let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();

        assert_eq!(
            queue_full_frame(&mut connection, frame),
            Err(RfbEncodeError::DesktopSizeNotNegotiated {
                announced: RfbSize::new(2, 2).unwrap(),
                actual: RfbSize::new(3, 2).unwrap(),
            })
        );
        assert!(connection.take_output().is_empty());
    }

    #[test]
    fn negotiated_resize_is_a_standalone_desktop_size_update() {
        let mut connection = completed_connection_with_size(2, 2);
        negotiate_desktop_size(&mut connection);
        let pixels = [0_u8; 24];
        let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();

        assert_eq!(
            queue_full_frame(&mut connection, frame),
            Ok(FramebufferUpdateOutcome::ResizeAnnounced {
                size: RfbSize::new(3, 2).unwrap(),
            })
        );
        assert_eq!(
            connection.take_output(),
            [0, 0, 0, 1, 0, 0, 0, 0, 0, 3, 0, 2, 0xff, 0xff, 0xff, 0x21,]
        );

        assert!(matches!(
            queue_full_frame(&mut connection, frame),
            Ok(FramebufferUpdateOutcome::RawQueued { .. })
        ));
    }

    #[test]
    fn pointer_events_carry_the_current_input_coordinate_size() {
        let mut connection = completed_connection_with_size(2, 2);

        assert_eq!(
            connection.push_input(&[5, 0, 0, 1, 0, 1]),
            vec![Ok(RfbEvent::Pointer {
                button_mask: 0,
                x: 1,
                y: 1,
                framebuffer_size: RfbSize::new(2, 2).unwrap(),
            })]
        );

        negotiate_desktop_size(&mut connection);
        let pixels = [0_u8; 24];
        let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();
        assert!(matches!(
            queue_full_frame(&mut connection, frame),
            Ok(FramebufferUpdateOutcome::ResizeAnnounced { .. })
        ));

        assert_eq!(
            connection.push_input(&[5, 0, 0, 2, 0, 1]),
            vec![Ok(RfbEvent::Pointer {
                button_mask: 0,
                x: 2,
                y: 1,
                framebuffer_size: RfbSize::new(3, 2).unwrap(),
            })]
        );
    }

    #[test]
    fn pointer_coordinate_epoch_changes_only_after_a_buffered_message_finishes() {
        let mut connection = completed_connection_with_size(2, 2);
        negotiate_desktop_size(&mut connection);

        assert!(connection.push_input(&[5, 0, 0]).is_empty());

        let pixels = [0_u8; 24];
        let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();
        assert!(matches!(
            queue_full_frame(&mut connection, frame),
            Ok(FramebufferUpdateOutcome::ResizeAnnounced { .. })
        ));

        assert_eq!(
            connection.push_input(&[1, 0, 1, 5, 0, 0, 1, 0, 1]),
            vec![
                Ok(RfbEvent::Pointer {
                    button_mask: 0,
                    x: 1,
                    y: 1,
                    framebuffer_size: RfbSize::new(2, 2).unwrap(),
                }),
                Ok(RfbEvent::Pointer {
                    button_mask: 0,
                    x: 1,
                    y: 1,
                    framebuffer_size: RfbSize::new(2, 2).unwrap(),
                }),
            ]
        );
        assert_eq!(
            connection.push_input(&[5, 0, 0, 2, 0, 1]),
            vec![Ok(RfbEvent::Pointer {
                button_mask: 0,
                x: 2,
                y: 1,
                framebuffer_size: RfbSize::new(3, 2).unwrap(),
            })]
        );
    }

    #[test]
    fn oversized_frame_error_leaves_output_unchanged() {
        let mut config = config_with_size(2, 2);
        config.limits.max_framebuffer_bytes = 16;
        config.limits.max_queued_output_bytes = 64;
        let mut connection = complete(config);
        assert!(connection.take_output().is_empty());

        let pixels = [0_u8; 24];
        let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();
        assert_eq!(
            queue_full_frame(&mut connection, frame),
            Err(RfbEncodeError::FramebufferTooLarge {
                actual: 24,
                maximum: 16,
            })
        );
        assert!(connection.take_output().is_empty());
    }

    #[test]
    fn update_before_handshake_does_not_consume_banner() {
        let mut connection = RfbConnectionCore::new(config_with_size(2, 2)).unwrap();
        let pixels = [0_u8; 16];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();

        assert_eq!(
            queue_full_frame(&mut connection, frame),
            Err(RfbEncodeError::HandshakeNotComplete)
        );
        assert_eq!(connection.take_output(), b"RFB 003.008\n");
    }

    #[test]
    fn output_capacity_error_preserves_the_first_queued_update() {
        let mut config = config_with_size(2, 2);
        config.limits.max_framebuffer_bytes = 16;
        config.limits.max_queued_output_bytes = 50;
        let mut connection = complete(config);
        let pixels = [0_u8; 16];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();

        queue_full_frame(&mut connection, frame).unwrap();
        assert_eq!(
            queue_full_frame(&mut connection, frame),
            Err(RfbEncodeError::OutputQueueFull {
                attempted: 64,
                maximum: 50,
            })
        );
        assert_eq!(
            connection.take_output(),
            [
                0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn failed_desktop_size_queue_does_not_commit_new_size() {
        let mut config = config_with_size(2, 2);
        config.limits.max_framebuffer_bytes = 24;
        config.limits.max_queued_output_bytes = 50;
        let mut connection = complete(config);
        negotiate_desktop_size(&mut connection);

        let old_pixels = [0_u8; 16];
        let old_frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &old_pixels).unwrap();
        queue_full_frame(&mut connection, old_frame).unwrap();
        connection
            .queue_framebuffer_update(
                old_frame,
                FramebufferUpdateRequest {
                    incremental: true,
                    rectangle: RfbRectangle {
                        x: 10,
                        y: 10,
                        width: 1,
                        height: 1,
                    },
                },
            )
            .unwrap();

        let new_pixels = [0_u8; 24];
        let new_frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &new_pixels).unwrap();
        assert_eq!(
            queue_full_frame(&mut connection, new_frame),
            Err(RfbEncodeError::OutputQueueFull {
                attempted: 52,
                maximum: 50,
            })
        );
        assert_eq!(
            connection.push_input(&[5, 0, 0, 1, 0, 1]),
            vec![Ok(RfbEvent::Pointer {
                button_mask: 0,
                x: 1,
                y: 1,
                framebuffer_size: RfbSize::new(2, 2).unwrap(),
            })]
        );
        assert_eq!(connection.take_output().len(), 36);
        assert!(matches!(
            queue_full_frame(&mut connection, new_frame),
            Ok(FramebufferUpdateOutcome::ResizeAnnounced { size })
                if size == RfbSize::new(3, 2).unwrap()
        ));
        assert_eq!(
            connection.push_input(&[5, 0, 0, 2, 0, 1]),
            vec![Ok(RfbEvent::Pointer {
                button_mask: 0,
                x: 2,
                y: 1,
                framebuffer_size: RfbSize::new(3, 2).unwrap(),
            })]
        );
    }

    #[test]
    fn protocol_version_accepts_every_split_boundary() {
        for split in 0..=12 {
            let mut connection = RfbConnectionCore::new(config()).unwrap();
            connection.take_output();
            assert!(connection.push_input(&b"RFB 003.008\n"[..split]).is_empty());
            assert!(connection.push_input(&b"RFB 003.008\n"[split..]).is_empty());
            assert_eq!(
                connection.state(),
                RfbConnectionState::AwaitingSecuritySelection
            );
            assert_eq!(connection.take_output(), [1, 1]);
        }
    }

    proptest! {
        #[test]
        fn output_capacity_failure_keeps_first_update_and_connection_state(
            pixels in prop::array::uniform16(any::<u8>())
        ) {
            let mut config = config_with_size(2, 2);
            config.limits.max_framebuffer_bytes = 16;
            config.limits.max_queued_output_bytes = 50;
            let mut connection = complete(config);
            let frame =
                BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();
            let before_state = connection.state();
            let before_format = connection.pixel_format();
            let before_encodings = connection.encoding_preferences().to_vec();

            queue_full_frame(&mut connection, frame).unwrap();
            prop_assert_eq!(
                queue_full_frame(&mut connection, frame),
                Err(RfbEncodeError::OutputQueueFull {
                    attempted: 64,
                    maximum: 50,
                })
            );

            let mut expected = vec![
                0, 0, 0, 1,
                0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0,
            ];
            for pixel in pixels.chunks_exact(4) {
                expected.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0]);
            }
            prop_assert_eq!(connection.take_output(), expected);
            prop_assert_eq!(connection.state(), before_state);
            prop_assert_eq!(connection.pixel_format(), before_format);
            prop_assert_eq!(
                connection.encoding_preferences(),
                before_encodings.as_slice()
            );

            prop_assert!(queue_full_frame(&mut connection, frame).is_ok());
        }
    }
}
