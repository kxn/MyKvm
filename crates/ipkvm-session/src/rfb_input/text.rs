//! 文本键入服务：把 RFB 剪切板文本逐字符转模拟键入（en-US 键盘映射）。
//!
//! 独立于物理键盘状态机运行（异步逐字符节流是慢操作，不阻塞 pump 事件循环）。
//! 非 ASCII / 不可映射字符跳过并计入 `chars_skipped`；设备错误立即停止并
//! `release_all`；控制者断开/释放时取消进行中的键入。
//!
//! 锁定键状态源设计为注入点但本轮不实现：当前总是假设锁定键未按下。

use std::collections::VecDeque;
use std::marker::PhantomData;
use std::time::Duration;

use ipkvm_core::{InputError, InputSink, KeyEvent, KeyboardUsage};
use tokio::sync::mpsc;

use super::keymap::{MappedKey, ShiftRequirement, map_keysym};
use crate::rfb_connection::RfbClientId;

/// 文本键入节流配置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextInputConfig {
    /// 相邻键盘批次之间的最小间隔（默认按 9600 波特留足余量，每字符约 30ms）。
    pub inter_char_delay: Duration,
}

impl Default for TextInputConfig {
    fn default() -> Self {
        Self {
            inter_char_delay: Duration::from_millis(30),
        }
    }
}

/// 文本键入处理结果通知。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextInputNotice {
    /// 文本键入完成（或被取消）：`chars_typed` 为成功按下并释放的字符数。
    Typed {
        client_id: RfbClientId,
        chars_typed: usize,
        chars_skipped: usize,
    },
    /// 设备/队列错误：键入已停止并 release_all，剩余文本丢弃。
    Error {
        client_id: RfbClientId,
        error: String,
    },
}

/// 文本键入服务句柄：向独立任务发送键入/取消命令。
///
/// `S` 仅作为 sink 类型的标记（sink 本身已移入独立任务）。
#[derive(Debug)]
pub struct TextInputService<S: InputSink> {
    tx: mpsc::UnboundedSender<TextInputCommand>,
    // 持有任务句柄以便 task 存活，任务终止依赖命令通道关闭。
    _task: Option<tokio::task::JoinHandle<()>>,
    marker: PhantomData<S>,
}

impl<S: InputSink> Clone for TextInputService<S> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            // 克隆句柄不需要重复持有任务句柄。
            _task: None,
            marker: PhantomData,
        }
    }
}

pub(super) enum TextInputCommand {
    TypeText {
        client_id: RfbClientId,
        text: String,
    },
    Cancel {
        client_id: RfbClientId,
    },
}

impl<S: InputSink> TextInputService<S> {
    /// 请求把文本逐字符转模拟键入（不等待键入完成）。
    pub async fn type_text(&self, client_id: RfbClientId, text: String) {
        let _ = self.tx.send(TextInputCommand::TypeText { client_id, text });
    }

    /// 取消指定控制者的进行中键入并 release_all。
    pub async fn cancel(&self, client_id: RfbClientId) {
        let _ = self.tx.send(TextInputCommand::Cancel { client_id });
    }

    /// pump 同步转发路径：命令通道仅在任务退出后关闭，失败即任务已终止。
    pub(super) fn try_type_text(
        &self,
        client_id: RfbClientId,
        text: String,
    ) -> Result<(), mpsc::error::SendError<TextInputCommand>> {
        self.tx.send(TextInputCommand::TypeText { client_id, text })
    }

    pub(super) fn try_cancel(
        &self,
        client_id: RfbClientId,
    ) -> Result<(), mpsc::error::SendError<TextInputCommand>> {
        self.tx.send(TextInputCommand::Cancel { client_id })
    }
}

impl<S: InputSink + Send + 'static> TextInputService<S> {
    /// 启动独立键入任务，返回服务句柄与处理结果通知接收端。
    pub fn new(sink: S, config: TextInputConfig) -> (Self, mpsc::Receiver<TextInputNotice>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (notice_tx, notice_rx) = mpsc::channel(16);
        let task = tokio::spawn(run_service(sink, config, command_rx, notice_tx));
        (
            Self {
                tx: command_tx,
                _task: Some(task),
                marker: PhantomData,
            },
            notice_rx,
        )
    }
}

enum TypeTextOutcome {
    Completed { typed: usize, skipped: usize },
    Aborted { typed: usize, skipped: usize },
    Failed { error: InputError },
}

/// 独立任务：顺序消费命令，逐字符节流键入。
async fn run_service<S: InputSink>(
    mut sink: S,
    config: TextInputConfig,
    mut commands: mpsc::UnboundedReceiver<TextInputCommand>,
    notices: mpsc::Sender<TextInputNotice>,
) {
    // 键入节流期间提前取出的命令，保证 FIFO 顺序不丢。
    let mut stash: VecDeque<TextInputCommand> = VecDeque::new();
    loop {
        let command = match stash.pop_front() {
            Some(command) => command,
            None => match commands.recv().await {
                Some(command) => command,
                None => return,
            },
        };
        match command {
            TextInputCommand::TypeText { client_id, text } => {
                let outcome = type_text(
                    &mut sink,
                    config.inter_char_delay,
                    client_id,
                    &text,
                    &mut commands,
                    &mut stash,
                )
                .await;
                let notice = match outcome {
                    TypeTextOutcome::Completed { typed, skipped }
                    | TypeTextOutcome::Aborted { typed, skipped } => TextInputNotice::Typed {
                        client_id,
                        chars_typed: typed,
                        chars_skipped: skipped,
                    },
                    TypeTextOutcome::Failed { error } => {
                        let _ = sink.release_all();
                        TextInputNotice::Error {
                            client_id,
                            error: error.to_string(),
                        }
                    }
                };
                let _ = notices.send(notice).await;
            }
            TextInputCommand::Cancel { .. } => {
                // 防御性复位：中止路径已 release_all 时这里多写一次复位报告，无害。
                let _ = sink.release_all();
            }
        }
    }
}

/// 逐字符：press 批次 → 节流 → release 批次 → 节流。
///
/// 节流间隔内收到匹配的 Cancel 或命令通道关闭时立即中止（当前按下的键由
/// `release_all` 复位），剩余文本丢弃。
async fn type_text<S: InputSink>(
    sink: &mut S,
    inter_char_delay: Duration,
    client_id: RfbClientId,
    text: &str,
    commands: &mut mpsc::UnboundedReceiver<TextInputCommand>,
    stash: &mut VecDeque<TextInputCommand>,
) -> TypeTextOutcome {
    let mut typed = 0usize;
    let mut skipped = 0usize;
    for character in text.chars() {
        let mapped = match map_keysym(char_to_keysym(character)) {
            Ok(MappedKey::IgnoredLock) | Err(_) => {
                skipped += 1;
                continue;
            }
            Ok(mapped) => mapped,
        };
        let (down_events, up_events) = key_events(mapped);
        if let Err(error) = sink.handle_key_batch(&down_events) {
            return TypeTextOutcome::Failed { error };
        }
        if let Some(outcome) = check_throttle(
            sink,
            client_id,
            typed,
            skipped,
            inter_char_delay,
            commands,
            stash,
        )
        .await
        {
            return outcome;
        }
        if let Err(error) = sink.handle_key_batch(&up_events) {
            return TypeTextOutcome::Failed { error };
        }
        typed += 1;
        if let Some(outcome) = check_throttle(
            sink,
            client_id,
            typed,
            skipped,
            inter_char_delay,
            commands,
            stash,
        )
        .await
        {
            return outcome;
        }
    }
    TypeTextOutcome::Completed { typed, skipped }
}

/// 节流间隔内监听取消命令；返回 `Some(outcome)` 表示键入被中止。
async fn check_throttle<S: InputSink>(
    sink: &mut S,
    client_id: RfbClientId,
    typed: usize,
    skipped: usize,
    delay: Duration,
    commands: &mut mpsc::UnboundedReceiver<TextInputCommand>,
    stash: &mut VecDeque<TextInputCommand>,
) -> Option<TypeTextOutcome> {
    match tokio::time::timeout(delay, commands.recv()).await {
        Err(_elapsed) => None,
        Ok(None) => {
            // 命令通道关闭：服务句柄被丢弃，立即复位并结束。
            let _ = sink.release_all();
            Some(TypeTextOutcome::Aborted { typed, skipped })
        }
        Ok(Some(TextInputCommand::Cancel {
            client_id: cancelled,
        })) if cancelled == client_id => {
            let _ = sink.release_all();
            Some(TypeTextOutcome::Aborted { typed, skipped })
        }
        Ok(Some(other)) => {
            stash.push_back(other);
            None
        }
    }
}

/// 非打印控制字符先转成标准 keysym，其余字符直接用 Unicode 码点（RFB 约定）。
fn char_to_keysym(character: char) -> u32 {
    match character {
        '\n' => 0xff0d,
        '\t' => 0xff09,
        _ => character as u32,
    }
}

/// 把映射结果展开成 press / release 两批键盘事件；需要 Shift 的字符
/// 先按左 Shift（0xe1）再字符，释放时反向。
fn key_events(mapped: MappedKey) -> (Vec<KeyEvent>, Vec<KeyEvent>) {
    match mapped {
        MappedKey::Direct(usage) => (vec![KeyEvent::Down { usage }], vec![KeyEvent::Up { usage }]),
        MappedKey::Character { usage, shift } => match shift {
            ShiftRequirement::Required => (
                vec![
                    KeyEvent::Down {
                        usage: left_shift(),
                    },
                    KeyEvent::Down { usage },
                ],
                vec![
                    KeyEvent::Up { usage },
                    KeyEvent::Up {
                        usage: left_shift(),
                    },
                ],
            ),
            ShiftRequirement::NotRequired => {
                (vec![KeyEvent::Down { usage }], vec![KeyEvent::Up { usage }])
            }
        },
        MappedKey::IgnoredLock => unreachable!("chars never map to lock keys"),
    }
}

fn left_shift() -> KeyboardUsage {
    KeyboardUsage::new(0xe1).expect("left shift HID usage is valid")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ipkvm_core::{
        InputError, InputResult, InputSink, KeyEvent, KeyboardUsage, MouseMode, PointerEvent,
    };

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingSink {
        state: Arc<Mutex<RecordingSinkState>>,
    }

    #[derive(Default)]
    struct RecordingSinkState {
        key_batches: Vec<Vec<KeyEvent>>,
        release_count: usize,
        fail_next: bool,
    }

    impl RecordingSink {
        fn key_batches(&self) -> Vec<Vec<KeyEvent>> {
            self.state
                .lock()
                .expect("recording sink lock poisoned")
                .key_batches
                .clone()
        }

        fn release_count(&self) -> usize {
            self.state
                .lock()
                .expect("recording sink lock poisoned")
                .release_count
        }

        fn fail_next(&self) {
            self.state
                .lock()
                .expect("recording sink lock poisoned")
                .fail_next = true;
        }
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
            let mut state = self.state.lock().expect("recording sink lock poisoned");
            if std::mem::take(&mut state.fail_next) {
                return Err(InputError::RolloverLimitExceeded);
            }
            state.key_batches.push(events.to_vec());
            Ok(())
        }

        fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            self.state
                .lock()
                .expect("recording sink lock poisoned")
                .release_count += 1;
            Ok(())
        }
    }

    fn down(usage: u8) -> KeyEvent {
        KeyEvent::Down {
            usage: KeyboardUsage::new(usage).unwrap(),
        }
    }

    fn up(usage: u8) -> KeyEvent {
        KeyEvent::Up {
            usage: KeyboardUsage::new(usage).unwrap(),
        }
    }

    fn client(value: u64) -> RfbClientId {
        RfbClientId::for_test(value)
    }

    async fn receive_notice(notices: &mut mpsc::Receiver<TextInputNotice>) -> TextInputNotice {
        tokio::time::timeout(Duration::from_secs(1), notices.recv())
            .await
            .expect("typed notice within timeout")
            .expect("notice channel open")
    }

    #[tokio::test]
    async fn text_input_typing_presses_and_releases_each_char() {
        let sink = RecordingSink::default();
        let (service, mut notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::ZERO,
            },
        );
        service.type_text(client(1), "ab".to_string()).await;

        let notice = receive_notice(&mut notices).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 2,
                chars_skipped: 0,
            }
        );
        assert_eq!(
            sink.key_batches(),
            vec![
                vec![down(0x04)],
                vec![up(0x04)],
                vec![down(0x05)],
                vec![up(0x05)],
            ]
        );
    }

    #[tokio::test]
    async fn text_input_synthesizes_shift_for_required_characters() {
        let sink = RecordingSink::default();
        let (service, mut notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::ZERO,
            },
        );
        service.type_text(client(1), "A!".to_string()).await;

        let notice = receive_notice(&mut notices).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 2,
                chars_skipped: 0,
            }
        );
        assert_eq!(
            sink.key_batches(),
            vec![
                vec![down(0xe1), down(0x04)],
                vec![up(0x04), up(0xe1)],
                vec![down(0xe1), down(0x1e)],
                vec![up(0x1e), up(0xe1)],
            ]
        );
    }

    #[tokio::test]
    async fn text_input_maps_newline_and_tab_to_enter_and_tab() {
        let sink = RecordingSink::default();
        let (service, mut notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::ZERO,
            },
        );
        service.type_text(client(1), "a\nb\tc".to_string()).await;

        let notice = receive_notice(&mut notices).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 5,
                chars_skipped: 0,
            }
        );
        assert_eq!(
            sink.key_batches(),
            vec![
                vec![down(0x04)],
                vec![up(0x04)],
                vec![down(0x28)],
                vec![up(0x28)],
                vec![down(0x05)],
                vec![up(0x05)],
                vec![down(0x2b)],
                vec![up(0x2b)],
                vec![down(0x06)],
                vec![up(0x06)],
            ]
        );
    }

    #[tokio::test]
    async fn text_input_skips_unmappable_characters() {
        let sink = RecordingSink::default();
        let (service, mut notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::ZERO,
            },
        );
        // 'é' 与 '中' 非 ASCII、'\r' 不可映射 → 全部跳过。
        service
            .type_text(client(1), "a\u{e9}\u{4e2d}\r".to_string())
            .await;

        let notice = receive_notice(&mut notices).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 1,
                chars_skipped: 3,
            }
        );
        assert_eq!(sink.key_batches(), vec![vec![down(0x04)], vec![up(0x04)]]);
    }

    #[tokio::test]
    async fn text_input_device_error_stops_releases_and_reports_error() {
        let sink = RecordingSink::default();
        let (service, mut notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::ZERO,
            },
        );
        sink.fail_next();
        service.type_text(client(1), "ab".to_string()).await;

        let notice = receive_notice(&mut notices).await;
        assert_eq!(
            notice,
            TextInputNotice::Error {
                client_id: client(1),
                error: InputError::RolloverLimitExceeded.to_string(),
            }
        );
        // 首个批次失败后立即停止：剩余文本不再键入，且已 release_all。
        assert!(sink.key_batches().is_empty());
        assert_eq!(sink.release_count(), 1);
        // 服务仍存活但不再产生通知。
        assert!(
            tokio::time::timeout(Duration::from_millis(50), notices.recv())
                .await
                .is_err(),
            "no further notices after an error"
        );
    }

    #[tokio::test]
    async fn text_input_cancel_interrupts_in_flight_typing_and_releases_all() {
        let sink = RecordingSink::default();
        let (service, mut notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::from_millis(20),
            },
        );
        let _typing = tokio::spawn({
            let service = service.clone();
            async move { service.type_text(client(1), "abc".to_string()).await }
        });
        // 让服务按下第一个字符并进入节流间隔。
        tokio::task::yield_now().await;
        service.cancel(client(1)).await;

        // 取消发生在第一个字符释放之前：0 个字符完整键入，剩余文本丢弃。
        let notice = receive_notice(&mut notices).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 0,
                chars_skipped: 0,
            }
        );
        assert_eq!(sink.key_batches(), vec![vec![down(0x04)]]);
        // 中止路径（匹配的 Cancel 在节流内被消费）release_all 一次。
        assert_eq!(sink.release_count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn text_input_inter_char_delay_gates_press_and_release() {
        let sink = RecordingSink::default();
        let (service, _notices) = TextInputService::new(
            sink.clone(),
            TextInputConfig {
                inter_char_delay: Duration::from_millis(100),
            },
        );
        let _typing = tokio::spawn({
            let service = service.clone();
            async move { service.type_text(client(1), "ab".to_string()).await }
        });

        async fn wait_for_batches(sink: &RecordingSink, expected: Vec<Vec<KeyEvent>>) {
            tokio::time::timeout(Duration::from_millis(500), async {
                loop {
                    if sink.key_batches() == expected {
                        return;
                    }
                    tokio::time::advance(Duration::from_millis(5)).await;
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("batches reached within paused-clock deadline");
        }

        // press(a) 在键入开始时立即发出。
        wait_for_batches(&sink, vec![vec![down(0x04)]]).await;
        // 100ms 节流间隔未到：release 尚未发出。
        tokio::time::advance(Duration::from_millis(50)).await;
        tokio::task::yield_now().await;
        assert_eq!(sink.key_batches(), vec![vec![down(0x04)]]);
        // 越过 release 节流点后再越过下一个 press 节流点。
        wait_for_batches(&sink, vec![vec![down(0x04)], vec![up(0x04)]]).await;
        wait_for_batches(
            &sink,
            vec![vec![down(0x04)], vec![up(0x04)], vec![down(0x05)]],
        )
        .await;
    }
}
