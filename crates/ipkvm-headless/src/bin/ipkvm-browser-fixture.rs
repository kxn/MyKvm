use std::{
    io::{self, BufRead, Write},
    sync::{Arc, Mutex},
};

use ipkvm_core::{InputResult, InputSink, KeyEvent, MouseMode, PointerButton, PointerEvent};
use ipkvm_headless::{
    rfb_connection::{RfbConnectionGate, RfbServerEvent},
    rfb_input::{RfbInputNotice, RfbInputPump, RfbInputRunError},
    rfb_ws::RfbWebSocketConfig,
    web::{HeadlessWebService, HeadlessWebServiceError},
};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
    task::JoinError,
};

const FRAME_WIDTH: u32 = 320;
const FRAME_HEIGHT: u32 = 180;

#[derive(Clone)]
struct LineWriter {
    stdout: Arc<Mutex<io::Stdout>>,
}

impl LineWriter {
    fn new() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(io::stdout())),
        }
    }

    fn line(&self, value: impl AsRef<str>) {
        let mut stdout = self.stdout.lock().expect("fixture stdout lock poisoned");
        writeln!(stdout, "{}", value.as_ref()).expect("failed to write fixture stdout");
        stdout.flush().expect("failed to flush fixture stdout");
    }
}

#[derive(Clone)]
struct RecordingInputSink {
    output: LineWriter,
}

impl RecordingInputSink {
    fn new(output: LineWriter) -> Self {
        Self { output }
    }
}

impl InputSink for RecordingInputSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
        Ok(())
    }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
        for event in events {
            match event {
                KeyEvent::Down { usage } => {
                    self.output.line(format!("KEY\tDOWN\t{}", usage.get()));
                }
                KeyEvent::Up { usage } => {
                    self.output.line(format!("KEY\tUP\t{}", usage.get()));
                }
            }
        }
        Ok(())
    }

    fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
        for event in events {
            match event {
                PointerEvent::AbsoluteMove {
                    x,
                    y,
                    framebuffer_size,
                } => self.output.line(format!(
                    "POINTER\tMOVE\t{x}\t{y}\t{}\t{}",
                    framebuffer_size.width, framebuffer_size.height
                )),
                PointerEvent::RelativeMove { dx, dy } => {
                    self.output.line(format!("POINTER\tRELATIVE\t{dx}\t{dy}"));
                }
                PointerEvent::Button { button, down } => self.output.line(format!(
                    "POINTER\tBUTTON\t{}\t{}",
                    button_name(*button),
                    if *down { "DOWN" } else { "UP" }
                )),
                PointerEvent::Wheel { delta } => {
                    self.output.line(format!("POINTER\tWHEEL\t{delta}"));
                }
            }
        }
        Ok(())
    }

    fn release_all(&mut self) -> InputResult<()> {
        self.output.line("RELEASE");
        Ok(())
    }
}

fn button_name(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Left => "LEFT",
        PointerButton::Middle => "MIDDLE",
        PointerButton::Right => "RIGHT",
    }
}

#[derive(Debug, Error)]
enum FixtureError {
    #[error("fixture I/O failed")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Web(#[from] HeadlessWebServiceError),
    #[error(transparent)]
    Input(#[from] RfbInputRunError),
    #[error("fixture task failed")]
    Join(#[from] JoinError),
    #[error("fixture stop channel closed unexpectedly")]
    StopChannel,
    #[error("HTTP service stopped before the fixture shutdown signal")]
    HttpStoppedEarly,
    #[error("input pump stopped before the fixture shutdown signal")]
    InputStoppedEarly,
}

enum Trigger {
    Stop(Result<io::Result<()>, oneshot::error::RecvError>),
    Http(Result<Result<(), HeadlessWebServiceError>, JoinError>),
    Input(Result<Result<(), RfbInputRunError>, JoinError>),
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), FixtureError> {
    let output = LineWriter::new();
    let source = Arc::new(MockFrameSource::new());
    source.publish_frame(fixture_frame());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (event_tx, mut event_rx) = mpsc::channel::<RfbServerEvent>(64);
    let service = HeadlessWebService::new(
        source,
        event_tx,
        RfbWebSocketConfig::default(),
        shutdown_rx,
        RfbConnectionGate::new(),
        None, // auth：Task 5 接入 HTTP 鉴权后由调用方注入
    )?;

    let mut http_task = tokio::spawn(service.serve(listener));
    let input_output = output.clone();
    let notice_output = output.clone();
    let mut input_task = tokio::spawn(async move {
        let mut pump = RfbInputPump::new(RecordingInputSink::new(input_output));
        pump.run(&mut event_rx, |notice| {
            if matches!(notice, RfbInputNotice::ControllerReleased { .. }) {
                notice_output.line("CONTROLLER_RELEASED");
            }
        })
        .await
    });
    let stop_rx = spawn_stdin_waiter();

    output.line(format!(
        "READY\thttp://{address}\t{FRAME_WIDTH}\t{FRAME_HEIGHT}"
    ));

    tokio::pin!(stop_rx);
    let trigger = tokio::select! {
        result = &mut stop_rx => Trigger::Stop(result),
        result = &mut http_task => Trigger::Http(result),
        result = &mut input_task => Trigger::Input(result),
    };
    shutdown_tx.send_replace(true);

    match trigger {
        Trigger::Stop(result) => {
            result.map_err(|_| FixtureError::StopChannel)??;
            flatten_http(http_task.await)?;
            flatten_input(input_task.await)?;
            output.line("STOPPED");
            Ok(())
        }
        Trigger::Http(result) => {
            let http_result = flatten_http(result);
            flatten_input(input_task.await)?;
            http_result?;
            Err(FixtureError::HttpStoppedEarly)
        }
        Trigger::Input(result) => {
            let input_result = flatten_input(result);
            flatten_http(http_task.await)?;
            input_result?;
            Err(FixtureError::InputStoppedEarly)
        }
    }
}

fn flatten_http(
    result: Result<Result<(), HeadlessWebServiceError>, JoinError>,
) -> Result<(), FixtureError> {
    result??;
    Ok(())
}

fn flatten_input(
    result: Result<Result<(), RfbInputRunError>, JoinError>,
) -> Result<(), FixtureError> {
    result??;
    Ok(())
}

fn spawn_stdin_waiter() -> oneshot::Receiver<io::Result<()>> {
    let (sender, receiver) = oneshot::channel();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        let result = loop {
            match lines.next() {
                Some(Ok(line)) if line.trim() == "STOP" => break Ok(()),
                Some(Ok(_)) => {}
                Some(Err(error)) => break Err(error),
                None => break Ok(()),
            }
        };
        let _ = sender.send(result);
    });
    receiver
}

fn fixture_frame() -> Arc<VideoFrame> {
    let mut bytes = Vec::with_capacity((FRAME_WIDTH * FRAME_HEIGHT * 4) as usize);
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            let bgra = match (x < FRAME_WIDTH / 2, y < FRAME_HEIGHT / 2) {
                (true, true) => [0, 0, 255, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [255, 0, 0, 255],
                (false, false) => [255, 255, 255, 255],
            };
            bytes.extend_from_slice(&bgra);
        }
    }
    Arc::new(VideoFrame::new(
        1,
        MonotonicTimestamp::from_nanos(1),
        FRAME_WIDTH,
        FRAME_HEIGHT,
        FRAME_WIDTH * 4,
        PixelFormat::Bgra8888,
        Arc::from(bytes.into_boxed_slice()),
    ))
}
