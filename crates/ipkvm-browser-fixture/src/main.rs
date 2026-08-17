use std::{
    io::{self, BufRead, Write},
    sync::{Arc, Mutex},
};

use ipkvm_core::{
    InputError, InputResult, InputSink, KeyEvent, MouseMode, PointerButton, PointerEvent,
};
use ipkvm_device::StaticDeviceInventoryProvider;
use ipkvm_headless::{
    rfb_connection::RfbConnectionGate,
    rfb_input::RfbInputNotice,
    rfb_ws::RfbWebSocketConfig,
    settings::SettingsStore,
    supervisor::{RecoveryPolicy, SessionSupervisor},
    web::{HeadlessWebService, HeadlessWebServiceError, SessionFactory, SessionSelection},
};
use ipkvm_video::{
    FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource,
};
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
    mouse_mode: MouseMode,
}

impl RecordingInputSink {
    fn new(output: LineWriter) -> Self {
        Self {
            output,
            mouse_mode: MouseMode::Absolute,
        }
    }
}

impl InputSink for RecordingInputSink {
    fn initial_mouse_mode(&self) -> Option<MouseMode> {
        Some(self.mouse_mode)
    }

    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()> {
        self.mouse_mode = mode;
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
            let mismatch = match (self.mouse_mode, event) {
                (MouseMode::Absolute, PointerEvent::RelativeMove { .. }) => Some("relative move"),
                (MouseMode::Relative, PointerEvent::AbsoluteMove { .. }) => Some("absolute move"),
                _ => None,
            };
            if let Some(event) = mismatch {
                return Err(InputError::PointerModeMismatch {
                    mode: self.mouse_mode,
                    event,
                });
            }
        }
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

/// 会话工厂占位：fixture 不触发 `POST /api/session`，仅满足
/// `HeadlessWebService::new` 签名。
struct FixtureFactory;

impl SessionFactory<RecordingInputSink> for FixtureFactory {
    fn build_video(&self, _selection: &SessionSelection) -> Result<Arc<dyn FrameSource>, String> {
        // 浏览器测试会经 `POST /api/session` 触发 create/restart：工厂构建的
        // 新帧源必须先发布一帧，否则 RFB 连接会在初始帧检查处立即失败。
        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(fixture_frame());
        Ok(source as Arc<dyn FrameSource>)
    }

    fn build_control(&self, _selection: &SessionSelection) -> Result<RecordingInputSink, String> {
        Ok(RecordingInputSink::new(LineWriter::new()))
    }
}

#[derive(Debug, Error)]
enum FixtureError {
    #[error("fixture I/O failed")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Web(#[from] HeadlessWebServiceError),
    #[error("fixture task failed")]
    Join(#[from] JoinError),
    #[error("fixture stop channel closed unexpectedly")]
    StopChannel,
    #[error("HTTP service stopped before the fixture shutdown signal")]
    HttpStoppedEarly,
}

enum Trigger {
    Stop(Result<io::Result<()>, oneshot::error::RecvError>),
    Http(Result<Result<(), HeadlessWebServiceError>, JoinError>),
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

    // READY 在构造 HeadlessWebService 之前输出：引入后台会话任务后，构造 +
    // spawn 之后的 stdout 写入于 Windows 测试 pipe 下偶发不可达读端（手动运行
    // 正常，仅子进程受影响）。listener 已 bind，连接在 serve 起来前进 TCP 积压
    // 队列，不丢失。用 println! + 显式 flush（而非 output.line 的持久 handle），
    // 后者在测试 pipe 下首写偶发滞留。
    {
        use std::io::Write;
        println!("READY\thttp://{address}\t{FRAME_WIDTH}\t{FRAME_HEIGHT}");
        let _ = std::io::stdout().flush();
    }

    // 传输层订阅端来自共享 supervisor；输入事件由 supervisor 内部泵写入
    // RecordingInputSink，fixture 只负责把事件打印到 stdout 供浏览器测试断言。
    let gate = RfbConnectionGate::new();
    let mut supervisor = SessionSupervisor::new(gate.clone(), RecoveryPolicy::default());
    let source_for_start = Arc::clone(&source);
    let sink_output = output.clone();
    supervisor
        .start_at(
            move || Ok(source_for_start.clone() as Arc<dyn FrameSource>),
            move || Ok(RecordingInputSink::new(sink_output.clone())),
            std::time::Instant::now(),
        )
        .await;
    let (notice_tx, mut notice_rx) = mpsc::unbounded_channel();
    supervisor.set_notice_mirror(Some(notice_tx));
    let frame_hub = Arc::new(supervisor.frame_source());
    let event_publisher = supervisor.event_publisher();
    let supervisor = Arc::new(tokio::sync::Mutex::new(supervisor));
    let factory: Arc<dyn SessionFactory<RecordingInputSink> + Send + Sync> =
        Arc::new(FixtureFactory);
    let settings = Arc::new(
        SettingsStore::load_from(std::env::temp_dir().join(format!(
            "ipkvm-headless-fixture-settings-{}",
            std::process::id()
        )))
        .0,
    );
    let service = HeadlessWebService::new(
        frame_hub,
        supervisor,
        factory,
        Arc::new(StaticDeviceInventoryProvider::new(Vec::new(), Vec::new())),
        event_publisher,
        RfbWebSocketConfig::default(),
        shutdown_rx,
        gate,
        None, // auth：fixture 未配置 token，不启用鉴权
        settings,
        None,
    )?;

    let mut http_task = tokio::spawn(service.serve(listener));
    let notice_output = output.clone();
    let notice_task = tokio::spawn(async move {
        while let Some(notice) = notice_rx.recv().await {
            if matches!(notice, RfbInputNotice::ControllerReleased { .. }) {
                notice_output.line("CONTROLLER_RELEASED");
            }
        }
    });
    let stop_rx = spawn_stdin_waiter();

    tokio::pin!(stop_rx);
    let trigger = tokio::select! {
        result = &mut stop_rx => Trigger::Stop(result),
        result = &mut http_task => Trigger::Http(result),
    };
    shutdown_tx.send_replace(true);
    notice_task.abort();
    let _ = notice_task.await;

    match trigger {
        Trigger::Stop(result) => {
            result.map_err(|_| FixtureError::StopChannel)??;
            flatten_http(http_task.await)?;
            output.line("STOPPED");
            Ok(())
        }
        Trigger::Http(result) => {
            let http_result = flatten_http(result);
            http_result?;
            Err(FixtureError::HttpStoppedEarly)
        }
    }
}

fn flatten_http(
    result: Result<Result<(), HeadlessWebServiceError>, JoinError>,
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
