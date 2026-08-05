//! CH9329 真实串口传输：单一串口所有者、异步发送、持续读取响应和故障恢复。

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serialport::{ClearBuffer, SerialPort};

use crate::ch9329::{
    Ch9329Command, Ch9329DecodeError, Ch9329Decoder, Ch9329Frame, Ch9329Response, CommandStatus,
    KeyboardReport, RelativeMouseReport,
};
use crate::{CommandBatch, CommandQueue, CommandQueueError, QueueStats};

const DEFAULT_INTER_FRAME_DELAY: Duration = Duration::from_millis(2);
const SERIAL_READ_TIMEOUT: Duration = Duration::from_millis(20);
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(300);
const RECOVERY_DELAY: Duration = Duration::from_secs(2);
const MAX_PENDING_FRAMES: usize = 4;
const MAX_QUEUED_BATCHES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SerialHealthState {
    Opening,
    Synchronizing,
    Ready,
    Recovering,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SerialHealth {
    pub state: SerialHealthState,
    pub pending_frames: usize,
    pub queued_batches: usize,
    pub timeouts: u64,
    pub protocol_errors: u64,
    pub device_errors: u64,
    pub resets: u64,
    pub reopens: u64,
    pub dropped_batches: u64,
}

impl Default for SerialHealth {
    fn default() -> Self {
        Self {
            state: SerialHealthState::Opening,
            pending_frames: 0,
            queued_batches: 0,
            timeouts: 0,
            protocol_errors: 0,
            device_errors: 0,
            resets: 0,
            reopens: 0,
            dropped_batches: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SerialCommandQueueError {
    #[error("failed to open serial port {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: serialport::Error,
    },
    #[error("failed to start CH9329 serial worker: {0}")]
    Worker(#[source] std::io::Error),
}

#[derive(Clone)]
pub struct SerialCommandQueue {
    tx: SyncSender<CommandBatch>,
    stats: Arc<Mutex<QueueStats>>,
    health: Arc<Mutex<SerialHealth>>,
}

impl SerialCommandQueue {
    pub fn open(path: &str, baud: u32) -> Result<Self, SerialCommandQueueError> {
        let port = open_port(path, baud).map_err(|source| SerialCommandQueueError::Open {
            path: path.to_owned(),
            source,
        })?;
        let (tx, rx) = mpsc::sync_channel(MAX_QUEUED_BATCHES);
        let stats = Arc::new(Mutex::new(QueueStats::default()));
        let health = Arc::new(Mutex::new(SerialHealth::default()));
        let worker_health = Arc::clone(&health);
        let worker_path = path.to_owned();
        thread::Builder::new()
            .name("ch9329-serial".to_owned())
            .spawn(move || run_worker(worker_path, baud, port, rx, worker_health))
            .map_err(SerialCommandQueueError::Worker)?;

        Ok(Self { tx, stats, health })
    }

    pub fn open_default(path: &str) -> Result<Self, SerialCommandQueueError> {
        Self::open(path, crate::DEFAULT_BAUD_RATE)
    }

    pub fn health(&self) -> SerialHealth {
        self.health
            .lock()
            .map(|health| *health)
            .unwrap_or(SerialHealth {
                state: SerialHealthState::Offline,
                ..SerialHealth::default()
            })
    }

    pub fn wait_until_ready(&self, timeout: Duration) -> Option<SerialHealth> {
        let deadline = Instant::now() + timeout;
        loop {
            let health = self.health();
            match health.state {
                SerialHealthState::Ready => return Some(health),
                SerialHealthState::Offline => return None,
                SerialHealthState::Opening
                | SerialHealthState::Synchronizing
                | SerialHealthState::Recovering => {}
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_until_idle(&self, timeout: Duration) -> Option<SerialHealth> {
        let deadline = Instant::now() + timeout;
        loop {
            let health = self.health();
            match health.state {
                SerialHealthState::Ready
                    if health.pending_frames == 0 && health.queued_batches == 0 =>
                {
                    return Some(health);
                }
                SerialHealthState::Offline => return None,
                SerialHealthState::Opening
                | SerialHealthState::Synchronizing
                | SerialHealthState::Ready
                | SerialHealthState::Recovering => {}
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl CommandQueue for SerialCommandQueue {
    fn enqueue_batch(&self, batch: CommandBatch) -> Result<(), CommandQueueError> {
        let frame_count = batch.frames().len() as u64;
        increment_health(&self.health, |health| {
            health.queued_batches = health.queued_batches.saturating_add(1);
        });
        match self.tx.try_send(batch) {
            Ok(()) => {
                let mut stats = self.stats.lock().map_err(|_| CommandQueueError::Closed)?;
                stats.batches_accepted = stats.batches_accepted.saturating_add(1);
                stats.frames_accepted = stats.frames_accepted.saturating_add(frame_count);
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                decrement_queued_batches(&self.health);
                Err(CommandQueueError::Full)
            }
            Err(TrySendError::Disconnected(_)) => {
                decrement_queued_batches(&self.health);
                Err(CommandQueueError::Closed)
            }
        }
    }

    fn stats(&self) -> QueueStats {
        self.stats.lock().map(|stats| *stats).unwrap_or_default()
    }
}

struct PendingCommand {
    command: u8,
    sent_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportStateKind {
    Opening,
    Synchronizing,
    Ready,
    Recovering,
    Offline,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
enum TransportFault {
    #[error("CH9329 command {command:#04x} response timed out")]
    Timeout { command: u8 },
    #[error("CH9329 response command mismatch: expected {expected:#04x}, got {actual:#04x}")]
    ResponseMismatch { expected: u8, actual: u8 },
    #[error("CH9329 device rejected command {command:#04x}: {status:?}")]
    DeviceError { command: u8, status: CommandStatus },
    #[error("unexpected CH9329 response")]
    UnexpectedResponse,
    #[error("CH9329 response parse failed: {0}")]
    ResponseParse(String),
    #[error("CH9329 frame decode failed: {0}")]
    Decode(String),
    #[error("CH9329 serial I/O failed: {0}")]
    Io(String),
}

struct TransportState {
    kind: TransportStateKind,
    pending: VecDeque<PendingCommand>,
    response_timeout: Duration,
}

impl TransportState {
    fn new(response_timeout: Duration) -> Self {
        Self {
            kind: TransportStateKind::Opening,
            pending: VecDeque::new(),
            response_timeout,
        }
    }

    fn record_sent(&mut self, command: u8, sent_at: Instant) {
        self.pending.push_back(PendingCommand { command, sent_at });
    }

    fn check_timeout(&self, now: Instant) -> Option<TransportFault> {
        self.pending.front().and_then(|pending| {
            (now.saturating_duration_since(pending.sent_at) >= self.response_timeout).then_some(
                TransportFault::Timeout {
                    command: pending.command,
                },
            )
        })
    }

    fn accept_response(&mut self, response: &Ch9329Response) -> Result<u8, TransportFault> {
        let expected = self
            .pending
            .front()
            .map(|pending| pending.command)
            .ok_or(TransportFault::UnexpectedResponse)?;
        let (actual, status) = match response {
            Ch9329Response::Info(_) => (0x01, CommandStatus::Success),
            Ch9329Response::Acknowledgement { command, status }
            | Ch9329Response::Error { command, status } => (*command, *status),
        };
        if actual != expected {
            return Err(TransportFault::ResponseMismatch { expected, actual });
        }
        self.pending.pop_front();
        if status != CommandStatus::Success {
            return Err(TransportFault::DeviceError {
                command: actual,
                status,
            });
        }
        Ok(actual)
    }
}

fn open_port(path: &str, baud: u32) -> Result<Box<dyn SerialPort>, serialport::Error> {
    serialport::new(path, baud)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(SERIAL_READ_TIMEOUT)
        .open()
}

fn run_worker(
    path: String,
    baud: u32,
    mut port: Box<dyn SerialPort>,
    rx: Receiver<CommandBatch>,
    health: Arc<Mutex<SerialHealth>>,
) {
    let mut transport = TransportState::new(RESPONSE_TIMEOUT);
    transport.kind = TransportStateKind::Synchronizing;
    set_health_state(&health, SerialHealthState::Synchronizing, 0);
    let mut outbound = VecDeque::new();
    outbound.push_back(info_frame());
    let mut decoder = Ch9329Decoder::new();
    let mut channel_closed = false;

    loop {
        if transport.kind == TransportStateKind::Offline {
            return;
        }

        if transport.kind != TransportStateKind::Recovering {
            if transport.kind == TransportStateKind::Ready {
                receive_batches(&rx, &mut outbound, &transport, &health, &mut channel_closed);
            }
            if let Some(fault) = write_available(&mut port, &mut outbound, &mut transport, &health)
            {
                log_fault(&path, &health, &fault);
                let allow_reset = !matches!(fault, TransportFault::Io(_));
                if let Some(recovered_port) =
                    recover(path.as_str(), baud, port, &health, allow_reset)
                {
                    port = recovered_port;
                    transport = TransportState::new(RESPONSE_TIMEOUT);
                    transport.kind = TransportStateKind::Ready;
                    decoder = Ch9329Decoder::new();
                    outbound.clear();
                    clear_queued_batches(&rx, &health);
                    continue;
                }
                set_offline(&health, &fault);
                return;
            }

            let mut buffer = [0_u8; 256];
            match port.read(&mut buffer) {
                Ok(read) if read > 0 => {
                    for event in decoder.push(&buffer[..read]) {
                        match event {
                            Ok(frame) => match Ch9329Response::parse(&frame) {
                                Ok(response) => match transport.accept_response(&response) {
                                    Ok(command) => {
                                        if command == 0x01 {
                                            transport.kind = TransportStateKind::Ready;
                                            set_health_state(
                                                &health,
                                                SerialHealthState::Ready,
                                                transport.pending.len(),
                                            );
                                        } else {
                                            update_pending(
                                                &health,
                                                outbound.len() + transport.pending.len(),
                                            );
                                        }
                                    }
                                    Err(fault) => {
                                        log_fault(&path, &health, &fault);
                                        if let Some(recovered_port) =
                                            recover(path.as_str(), baud, port, &health, true)
                                        {
                                            port = recovered_port;
                                            transport = TransportState::new(RESPONSE_TIMEOUT);
                                            transport.kind = TransportStateKind::Ready;
                                            decoder = Ch9329Decoder::new();
                                            outbound.clear();
                                            clear_queued_batches(&rx, &health);
                                            break;
                                        }
                                        set_offline(&health, &fault);
                                        return;
                                    }
                                },
                                Err(error) => {
                                    let fault = TransportFault::ResponseParse(error.to_string());
                                    log_fault(&path, &health, &fault);
                                    if let Some(recovered_port) =
                                        recover(path.as_str(), baud, port, &health, true)
                                    {
                                        port = recovered_port;
                                        transport = TransportState::new(RESPONSE_TIMEOUT);
                                        transport.kind = TransportStateKind::Ready;
                                        decoder = Ch9329Decoder::new();
                                        outbound.clear();
                                        clear_queued_batches(&rx, &health);
                                        break;
                                    }
                                    set_offline(&health, &fault);
                                    return;
                                }
                            },
                            Err(Ch9329DecodeError::NoiseDiscarded(count)) => {
                                eprintln!("[ch9329:{path}] discarded {count} noise bytes")
                            }
                            Err(error) => {
                                let fault = TransportFault::Decode(error.to_string());
                                log_fault(&path, &health, &fault);
                                if let Some(recovered_port) =
                                    recover(path.as_str(), baud, port, &health, true)
                                {
                                    port = recovered_port;
                                    transport = TransportState::new(RESPONSE_TIMEOUT);
                                    transport.kind = TransportStateKind::Ready;
                                    decoder = Ch9329Decoder::new();
                                    outbound.clear();
                                    clear_queued_batches(&rx, &health);
                                    break;
                                }
                                set_offline(&health, &fault);
                                return;
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(error)
                    if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {}
                Err(error) => {
                    let fault = TransportFault::Io(error.to_string());
                    log_fault(&path, &health, &fault);
                    if let Some(recovered_port) = recover(path.as_str(), baud, port, &health, false)
                    {
                        port = recovered_port;
                        transport = TransportState::new(RESPONSE_TIMEOUT);
                        transport.kind = TransportStateKind::Ready;
                        decoder = Ch9329Decoder::new();
                        outbound.clear();
                        clear_queued_batches(&rx, &health);
                        continue;
                    }
                    set_offline(&health, &fault);
                    return;
                }
            }

            if let Some(fault) = transport.check_timeout(Instant::now()) {
                log_fault(&path, &health, &fault);
                if let Some(recovered_port) = recover(path.as_str(), baud, port, &health, true) {
                    port = recovered_port;
                    transport = TransportState::new(RESPONSE_TIMEOUT);
                    transport.kind = TransportStateKind::Ready;
                    decoder = Ch9329Decoder::new();
                    clear_queued_batches(&rx, &health);
                    continue;
                }
                set_offline(&health, &fault);
                return;
            }
        }

        if channel_closed && outbound.is_empty() && transport.pending.is_empty() {
            return;
        }
    }
}

fn receive_batches(
    rx: &Receiver<CommandBatch>,
    outbound: &mut VecDeque<Ch9329Frame>,
    transport: &TransportState,
    health: &Arc<Mutex<SerialHealth>>,
    channel_closed: &mut bool,
) {
    while outbound.len() < MAX_PENDING_FRAMES {
        match rx.try_recv() {
            Ok(batch) => {
                outbound.extend(batch.frames().iter().cloned());
                decrement_queued_batches(health);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                *channel_closed = true;
                break;
            }
        }
    }
    update_pending(health, outbound.len() + transport.pending.len());
}

fn write_available(
    port: &mut Box<dyn SerialPort>,
    outbound: &mut VecDeque<Ch9329Frame>,
    transport: &mut TransportState,
    health: &Arc<Mutex<SerialHealth>>,
) -> Option<TransportFault> {
    while transport.pending.len() < MAX_PENDING_FRAMES {
        let Some(frame) = outbound.pop_front() else {
            break;
        };
        if let Err(error) = port.write_all(frame.as_bytes()).and_then(|_| port.flush()) {
            return Some(TransportFault::Io(error.to_string()));
        }
        transport.record_sent(frame.command(), Instant::now());
        update_pending(health, outbound.len() + transport.pending.len());
        if !DEFAULT_INTER_FRAME_DELAY.is_zero() && !outbound.is_empty() {
            thread::sleep(DEFAULT_INTER_FRAME_DELAY);
        }
    }
    None
}

fn recover(
    path: &str,
    baud: u32,
    port: Box<dyn SerialPort>,
    health: &Arc<Mutex<SerialHealth>>,
    allow_in_place_reset: bool,
) -> Option<Box<dyn SerialPort>> {
    recover_with_timing(
        path,
        baud,
        port,
        health,
        allow_in_place_reset,
        RECOVERY_DELAY,
        RESPONSE_TIMEOUT,
    )
}

fn recover_with_timing(
    path: &str,
    baud: u32,
    mut port: Box<dyn SerialPort>,
    health: &Arc<Mutex<SerialHealth>>,
    allow_in_place_reset: bool,
    recovery_delay: Duration,
    response_timeout: Duration,
) -> Option<Box<dyn SerialPort>> {
    set_health_state(health, SerialHealthState::Recovering, 0);
    if allow_in_place_reset {
        increment_health(health, |health| {
            health.resets = health.resets.saturating_add(1)
        });
        eprintln!("[ch9329:{path}] recovery reset");

        let in_place =
            port.clear(ClearBuffer::All).is_ok() && write_command(&mut port, reset_frame()).is_ok();
        if in_place {
            let reset_ack = wait_for_response(&mut port, 0x0f, response_timeout);
            thread::sleep(recovery_delay);
            if reset_ack && probe_and_release(&mut port, response_timeout) {
                eprintln!("[ch9329:{path}] recovery ready");
                set_health_state(health, SerialHealthState::Ready, 0);
                return Some(port);
            }
        }
    } else {
        eprintln!("[ch9329:{path}] serial I/O fault; reopening without software reset");
    }

    drop(port);
    increment_health(health, |health| {
        health.reopens = health.reopens.saturating_add(1)
    });
    eprintln!("[ch9329:{path}] recovery reopening serial port");
    let mut reopened = match open_port(path, baud) {
        Ok(port) => port,
        Err(error) => {
            eprintln!("[ch9329:{path}] recovery reopen failed: {error}");
            return None;
        }
    };
    if reopened.clear(ClearBuffer::All).is_ok()
        && probe_and_release(&mut reopened, response_timeout)
    {
        eprintln!("[ch9329:{path}] recovery ready after reopen");
        set_health_state(health, SerialHealthState::Ready, 0);
        Some(reopened)
    } else {
        None
    }
}

fn probe_and_release(port: &mut Box<dyn SerialPort>, response_timeout: Duration) -> bool {
    if port.clear(ClearBuffer::Input).is_err() || write_command(port, info_frame()).is_err() {
        return false;
    }
    if !wait_for_response(port, 0x01, response_timeout) {
        return false;
    }
    let keyboard = Ch9329Command::Keyboard(KeyboardReport::new(0, [0; 6]))
        .to_frame(0)
        .expect("zero keyboard report is valid");
    let mouse = Ch9329Command::MouseRelative(
        RelativeMouseReport::new(0, 0, 0, 0).expect("zero mouse report is valid"),
    )
    .to_frame(0)
    .expect("zero mouse report is valid");
    write_command(port, keyboard).is_ok()
        && wait_for_response(port, 0x02, response_timeout)
        && write_command(port, mouse).is_ok()
        && wait_for_response(port, 0x05, response_timeout)
}

fn wait_for_response(
    port: &mut Box<dyn SerialPort>,
    expected: u8,
    response_timeout: Duration,
) -> bool {
    let deadline = Instant::now() + response_timeout;
    let mut decoder = Ch9329Decoder::new();
    let mut buffer = [0_u8; 256];
    while Instant::now() < deadline {
        match port.read(&mut buffer) {
            Ok(read) if read > 0 => {
                for event in decoder.push(&buffer[..read]) {
                    let Ok(frame) = event else {
                        continue;
                    };
                    let Ok(response) = Ch9329Response::parse(&frame) else {
                        return false;
                    };
                    let (command, status) = match response {
                        Ch9329Response::Info(_) => (0x01, CommandStatus::Success),
                        Ch9329Response::Acknowledgement { command, status }
                        | Ch9329Response::Error { command, status } => (command, status),
                    };
                    return command == expected && status == CommandStatus::Success;
                }
            }
            Ok(_) => {}
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {}
            Err(_) => return false,
        }
    }
    false
}

fn write_command(port: &mut Box<dyn SerialPort>, frame: Ch9329Frame) -> std::io::Result<()> {
    port.write_all(frame.as_bytes())?;
    port.flush()
}

fn info_frame() -> Ch9329Frame {
    Ch9329Command::GetInfo
        .to_frame(0)
        .expect("GetInfo frame is valid")
}

fn reset_frame() -> Ch9329Frame {
    Ch9329Command::Reset
        .to_frame(0)
        .expect("Reset frame is valid")
}

fn clear_queued_batches(rx: &Receiver<CommandBatch>, health: &Arc<Mutex<SerialHealth>>) {
    while rx.try_recv().is_ok() {
        increment_health(health, |health| {
            health.dropped_batches = health.dropped_batches.saturating_add(1)
        });
        decrement_queued_batches(health);
    }
}

fn set_health_state(health: &Arc<Mutex<SerialHealth>>, state: SerialHealthState, pending: usize) {
    if let Ok(mut health) = health.lock() {
        health.state = state;
        health.pending_frames = pending;
    }
}

fn update_pending(health: &Arc<Mutex<SerialHealth>>, pending: usize) {
    if let Ok(mut health) = health.lock() {
        health.pending_frames = pending;
    }
}

fn decrement_queued_batches(health: &Arc<Mutex<SerialHealth>>) {
    increment_health(health, |health| {
        health.queued_batches = health.queued_batches.saturating_sub(1);
    });
}

fn increment_health(health: &Arc<Mutex<SerialHealth>>, update: impl FnOnce(&mut SerialHealth)) {
    if let Ok(mut health) = health.lock() {
        update(&mut health);
    }
}

fn log_fault(path: &str, health: &Arc<Mutex<SerialHealth>>, fault: &TransportFault) {
    increment_health(health, |health| match fault {
        TransportFault::Timeout { .. } => health.timeouts = health.timeouts.saturating_add(1),
        TransportFault::DeviceError { .. } => {
            health.device_errors = health.device_errors.saturating_add(1)
        }
        _ => health.protocol_errors = health.protocol_errors.saturating_add(1),
    });
    eprintln!("[ch9329:{path}] transport fault: {fault}")
}

fn set_offline(health: &Arc<Mutex<SerialHealth>>, _fault: &TransportFault) {
    set_health_state(health, SerialHealthState::Offline, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{self, Read, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeSerialState {
        writes: Vec<Vec<u8>>,
        reads: VecDeque<Vec<u8>>,
    }

    struct FakeSerialPort {
        state: Arc<Mutex<FakeSerialState>>,
        timeout: Duration,
        respond: bool,
    }

    impl FakeSerialPort {
        fn new() -> (Self, Arc<Mutex<FakeSerialState>>) {
            let state = Arc::new(Mutex::new(FakeSerialState::default()));
            (
                Self {
                    state: Arc::clone(&state),
                    timeout: Duration::from_millis(1),
                    respond: true,
                },
                state,
            )
        }
    }

    impl Read for FakeSerialPort {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let mut state = self.state.lock().expect("fake state lock");
            let Some(mut response) = state.reads.pop_front() else {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "fake timeout"));
            };
            let count = response.len().min(buffer.len());
            buffer[..count].copy_from_slice(&response[..count]);
            if count < response.len() {
                response.drain(..count);
                state.reads.push_front(response);
            }
            Ok(count)
        }
    }

    impl Write for FakeSerialPort {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let frame = Ch9329Frame::parse(buffer)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            let response = match frame.command() {
                0x0f => Ch9329Frame::new(0, 0x8f, &[0]).ok(),
                0x01 => Ch9329Frame::new(0, 0x81, &[1, 1, 0, 0, 0, 0, 0, 0]).ok(),
                0x02 => Ch9329Frame::new(0, 0x82, &[0]).ok(),
                0x05 => Ch9329Frame::new(0, 0x85, &[0]).ok(),
                _ => None,
            };
            let mut state = self.state.lock().expect("fake state lock");
            state.writes.push(buffer.to_vec());
            if self.respond
                && let Some(response) = response
            {
                state.reads.push_back(response.as_bytes().to_vec());
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SerialPort for FakeSerialPort {
        fn name(&self) -> Option<String> {
            Some("fake-ch9329".into())
        }

        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(9600)
        }

        fn data_bits(&self) -> serialport::Result<serialport::DataBits> {
            Ok(serialport::DataBits::Eight)
        }

        fn flow_control(&self) -> serialport::Result<serialport::FlowControl> {
            Ok(serialport::FlowControl::None)
        }

        fn parity(&self) -> serialport::Result<serialport::Parity> {
            Ok(serialport::Parity::None)
        }

        fn stop_bits(&self) -> serialport::Result<serialport::StopBits> {
            Ok(serialport::StopBits::One)
        }

        fn timeout(&self) -> Duration {
            self.timeout
        }

        fn set_baud_rate(&mut self, _baud_rate: u32) -> serialport::Result<()> {
            Ok(())
        }

        fn set_data_bits(&mut self, _data_bits: serialport::DataBits) -> serialport::Result<()> {
            Ok(())
        }

        fn set_flow_control(
            &mut self,
            _flow_control: serialport::FlowControl,
        ) -> serialport::Result<()> {
            Ok(())
        }

        fn set_parity(&mut self, _parity: serialport::Parity) -> serialport::Result<()> {
            Ok(())
        }

        fn set_stop_bits(&mut self, _stop_bits: serialport::StopBits) -> serialport::Result<()> {
            Ok(())
        }

        fn set_timeout(&mut self, timeout: Duration) -> serialport::Result<()> {
            self.timeout = timeout;
            Ok(())
        }

        fn write_request_to_send(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }

        fn write_data_terminal_ready(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }

        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }

        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn bytes_to_read(&self) -> serialport::Result<u32> {
            let state = self.state.lock().expect("fake state lock");
            Ok(state.reads.iter().map(|frame| frame.len() as u32).sum())
        }

        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }

        fn clear(&self, _buffer_to_clear: ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }

        fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
            Err(serialport::Error::new(
                serialport::ErrorKind::NoDevice,
                "fake port cannot clone",
            ))
        }

        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }

        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn response_command_must_match_fifo_pending_command() {
        let mut state = TransportState::new(Duration::from_millis(10));
        state.record_sent(0x02, Instant::now());
        assert_eq!(
            state.accept_response(&Ch9329Response::Acknowledgement {
                command: 0x04,
                status: CommandStatus::Success,
            }),
            Err(TransportFault::ResponseMismatch {
                expected: 0x02,
                actual: 0x04,
            })
        );
    }

    #[test]
    fn timeout_is_reported_for_oldest_pending_command() {
        let now = Instant::now();
        let mut state = TransportState::new(Duration::from_millis(10));
        state.record_sent(0x02, now - Duration::from_millis(20));
        assert_eq!(
            state.check_timeout(now),
            Some(TransportFault::Timeout { command: 0x02 })
        );
    }

    #[test]
    fn info_response_completes_get_info_pending_command() {
        let mut state = TransportState::new(Duration::from_millis(10));
        state.record_sent(0x01, Instant::now());
        let info = Ch9329Response::Info(crate::Ch9329Info {
            version: 1,
            usb_enumerated: true,
            leds: crate::LockLedState {
                num_lock: false,
                caps_lock: false,
                scroll_lock: false,
            },
            reserved: [0; 5],
        });
        assert_eq!(state.accept_response(&info), Ok(0x01));
        assert!(state.pending.is_empty());
    }

    #[test]
    fn recovery_writes_reset_probe_and_release_sequence() {
        let (port, state) = FakeSerialPort::new();
        let health = Arc::new(Mutex::new(SerialHealth::default()));

        let recovered = recover_with_timing(
            "fake",
            9600,
            Box::new(port),
            &health,
            true,
            Duration::ZERO,
            Duration::from_millis(10),
        );

        assert!(recovered.is_some());
        let commands: Vec<u8> = state
            .lock()
            .expect("fake state lock")
            .writes
            .iter()
            .map(|bytes| Ch9329Frame::parse(bytes).expect("fake frame").command())
            .collect();
        assert_eq!(commands, vec![0x0f, 0x01, 0x02, 0x05]);
        let health = health.lock().expect("health lock");
        assert_eq!(health.state, SerialHealthState::Ready);
        assert_eq!(health.resets, 1);
        assert_eq!(health.reopens, 0);
    }

    #[test]
    fn recovery_without_reset_ack_returns_failure() {
        let (mut port, _state) = FakeSerialPort::new();
        port.respond = false;
        let health = Arc::new(Mutex::new(SerialHealth::default()));

        let recovered = recover_with_timing(
            "path-that-is-not-a-real-port",
            9600,
            Box::new(port),
            &health,
            true,
            Duration::ZERO,
            Duration::from_millis(1),
        );

        assert!(recovered.is_none());
        let health = health.lock().expect("health lock");
        assert_eq!(health.state, SerialHealthState::Recovering);
        assert_eq!(health.resets, 1);
        assert_eq!(health.reopens, 1);
    }
}
