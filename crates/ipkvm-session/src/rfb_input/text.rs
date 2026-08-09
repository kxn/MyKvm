//! 文本键入服务：把 RFB 剪切板文本逐字符转模拟键入（en-US 键盘映射）。
//!
//! 服务只负责映射、节流和生成动作，不直接持有 `InputSink`。所有键盘状态提交
//! 都回到 `RfbInputPump` 的主 sink，避免文本键入与物理/RFB 输入维护两套 CH9329
//! 状态机。非 ASCII / 不可映射字符跳过并计入 `chars_skipped`；控制者断开/释放
//! 时取消进行中的键入，并让 pump 在主 sink 上执行 release。
//!
//! 锁定键状态源设计为注入点但本轮不实现：当前总是假设锁定键未按下。

use std::collections::VecDeque;
use std::time::Duration;

use ipkvm_core::{KeyEvent, KeyboardUsage};
use tokio::sync::{mpsc, oneshot};

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
    /// pump 应用文本动作时遇到设备/队列错误：键入已停止并 release_all，剩余文本丢弃。
    Error {
        client_id: RfbClientId,
        error: String,
    },
}

/// 文本键入服务句柄：向独立任务发送键入/取消命令。
#[derive(Debug)]
pub struct TextInputService {
    tx: mpsc::UnboundedSender<TextInputCommand>,
    // 持有任务句柄以便 task 存活，任务终止依赖命令通道关闭。
    _task: Option<tokio::task::JoinHandle<()>>,
}

impl Clone for TextInputService {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            // 克隆句柄不需要重复持有任务句柄。
            _task: None,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TextInputBatchResult {
    Continue,
    Abort,
}

#[derive(Debug)]
pub(super) enum TextInputAction {
    KeyBatch {
        client_id: RfbClientId,
        events: Vec<KeyEvent>,
        result: oneshot::Sender<TextInputBatchResult>,
    },
    ReleaseAll {
        client_id: RfbClientId,
    },
    Notice(TextInputNotice),
}

impl TextInputService {
    /// 请求把文本逐字符转模拟键入（不等待键入完成）。
    pub async fn type_text(&self, client_id: RfbClientId, text: String) {
        let _ = self.tx.send(TextInputCommand::TypeText { client_id, text });
    }

    /// 取消指定控制者的进行中键入，并请求 pump 在主 sink 上 release_all。
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

    /// 启动独立键入任务，返回服务句柄与文本动作接收端。
    pub(super) fn new(config: TextInputConfig) -> (Self, mpsc::Receiver<TextInputAction>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (action_tx, action_rx) = mpsc::channel(16);
        let task = tokio::spawn(run_service(config, command_rx, action_tx));
        (
            Self {
                tx: command_tx,
                _task: Some(task),
            },
            action_rx,
        )
    }
}

enum TypeTextOutcome {
    Completed { typed: usize, skipped: usize },
    Aborted { typed: usize, skipped: usize },
    Stopped,
    OutputClosed,
}

enum SendKeyBatchError {
    Aborted,
    OutputClosed,
}

/// 独立任务：顺序消费命令，逐字符节流键入。
async fn run_service(
    config: TextInputConfig,
    mut commands: mpsc::UnboundedReceiver<TextInputCommand>,
    actions: mpsc::Sender<TextInputAction>,
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
                    config.inter_char_delay,
                    client_id,
                    &text,
                    &mut commands,
                    &mut stash,
                    &actions,
                )
                .await;
                let notice = match outcome {
                    TypeTextOutcome::Completed { typed, skipped }
                    | TypeTextOutcome::Aborted { typed, skipped } => TextInputNotice::Typed {
                        client_id,
                        chars_typed: typed,
                        chars_skipped: skipped,
                    },
                    TypeTextOutcome::Stopped => continue,
                    TypeTextOutcome::OutputClosed => return,
                };
                if actions.send(TextInputAction::Notice(notice)).await.is_err() {
                    return;
                }
            }
            TextInputCommand::Cancel { client_id } => {
                if actions
                    .send(TextInputAction::ReleaseAll { client_id })
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

/// 逐字符：press 批次 → 节流 → release 批次 → 节流。
///
/// 节流间隔内收到匹配的 Cancel 或命令通道关闭时立即中止，请求 pump 用
/// `release_all` 复位当前按下的键，剩余文本丢弃。
async fn type_text(
    inter_char_delay: Duration,
    client_id: RfbClientId,
    text: &str,
    commands: &mut mpsc::UnboundedReceiver<TextInputCommand>,
    stash: &mut VecDeque<TextInputCommand>,
    actions: &mpsc::Sender<TextInputAction>,
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
        if let Err(error) = send_key_batch(actions, client_id, down_events).await {
            return match error {
                SendKeyBatchError::Aborted => TypeTextOutcome::Stopped,
                SendKeyBatchError::OutputClosed => TypeTextOutcome::OutputClosed,
            };
        }
        if let Some(outcome) = check_throttle(
            client_id,
            typed,
            skipped,
            inter_char_delay,
            commands,
            stash,
            actions,
        )
        .await
        {
            return outcome;
        }
        if let Err(error) = send_key_batch(actions, client_id, up_events).await {
            return match error {
                SendKeyBatchError::Aborted => TypeTextOutcome::Stopped,
                SendKeyBatchError::OutputClosed => TypeTextOutcome::OutputClosed,
            };
        }
        typed += 1;
        if let Some(outcome) = check_throttle(
            client_id,
            typed,
            skipped,
            inter_char_delay,
            commands,
            stash,
            actions,
        )
        .await
        {
            return outcome;
        }
    }
    TypeTextOutcome::Completed { typed, skipped }
}

/// 节流间隔内监听取消命令；返回 `Some(outcome)` 表示键入被中止。
async fn check_throttle(
    client_id: RfbClientId,
    typed: usize,
    skipped: usize,
    delay: Duration,
    commands: &mut mpsc::UnboundedReceiver<TextInputCommand>,
    stash: &mut VecDeque<TextInputCommand>,
    actions: &mpsc::Sender<TextInputAction>,
) -> Option<TypeTextOutcome> {
    match tokio::time::timeout(delay, commands.recv()).await {
        Err(_elapsed) => None,
        Ok(None) => {
            // 命令通道关闭：服务句柄被丢弃，立即复位并结束。
            if actions
                .send(TextInputAction::ReleaseAll { client_id })
                .await
                .is_err()
            {
                return Some(TypeTextOutcome::OutputClosed);
            }
            Some(TypeTextOutcome::Aborted { typed, skipped })
        }
        Ok(Some(TextInputCommand::Cancel {
            client_id: cancelled,
        })) if cancelled == client_id => {
            if actions
                .send(TextInputAction::ReleaseAll { client_id })
                .await
                .is_err()
            {
                return Some(TypeTextOutcome::OutputClosed);
            }
            Some(TypeTextOutcome::Aborted { typed, skipped })
        }
        Ok(Some(other)) => {
            stash.push_back(other);
            None
        }
    }
}

async fn send_key_batch(
    actions: &mpsc::Sender<TextInputAction>,
    client_id: RfbClientId,
    events: Vec<KeyEvent>,
) -> Result<(), SendKeyBatchError> {
    let (result_tx, result_rx) = oneshot::channel();
    actions
        .send(TextInputAction::KeyBatch {
            client_id,
            events,
            result: result_tx,
        })
        .await
        .map_err(|_| SendKeyBatchError::OutputClosed)?;
    match result_rx.await {
        Ok(TextInputBatchResult::Continue) => Ok(()),
        Ok(TextInputBatchResult::Abort) => Err(SendKeyBatchError::Aborted),
        Err(_) => Err(SendKeyBatchError::OutputClosed),
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
    use ipkvm_core::{KeyEvent, KeyboardUsage};
    use tokio::sync::mpsc;

    use super::*;

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

    async fn receive_action(actions: &mut mpsc::Receiver<TextInputAction>) -> TextInputAction {
        tokio::time::timeout(Duration::from_secs(1), actions.recv())
            .await
            .expect("text action within timeout")
            .expect("action channel open")
    }

    async fn collect_key_batches_until_notice(
        actions: &mut mpsc::Receiver<TextInputAction>,
    ) -> (Vec<Vec<KeyEvent>>, TextInputNotice) {
        let mut batches = Vec::new();
        loop {
            match receive_action(actions).await {
                TextInputAction::KeyBatch { events, result, .. } => {
                    result
                        .send(TextInputBatchResult::Continue)
                        .expect("text service should wait for batch result");
                    batches.push(events);
                }
                TextInputAction::ReleaseAll { .. } => panic!("unexpected release action"),
                TextInputAction::Notice(notice) => return (batches, notice),
            }
        }
    }

    async fn expect_key_batch(
        actions: &mut mpsc::Receiver<TextInputAction>,
        expected_client: RfbClientId,
        expected_events: Vec<KeyEvent>,
    ) {
        match receive_action(actions).await {
            TextInputAction::KeyBatch {
                client_id,
                events,
                result,
            } => {
                assert_eq!(client_id, expected_client);
                assert_eq!(events, expected_events);
                result
                    .send(TextInputBatchResult::Continue)
                    .expect("text service should wait for batch result");
            }
            other => panic!("expected key batch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn text_input_typing_presses_and_releases_each_char() {
        let (service, mut actions) = TextInputService::new(TextInputConfig {
            inter_char_delay: Duration::ZERO,
        });
        service.type_text(client(1), "ab".to_string()).await;

        let (batches, notice) = collect_key_batches_until_notice(&mut actions).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 2,
                chars_skipped: 0,
            }
        );
        assert_eq!(
            batches,
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
        let (service, mut actions) = TextInputService::new(TextInputConfig {
            inter_char_delay: Duration::ZERO,
        });
        service.type_text(client(1), "A!".to_string()).await;

        let (batches, notice) = collect_key_batches_until_notice(&mut actions).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 2,
                chars_skipped: 0,
            }
        );
        assert_eq!(
            batches,
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
        let (service, mut actions) = TextInputService::new(TextInputConfig {
            inter_char_delay: Duration::ZERO,
        });
        service.type_text(client(1), "a\nb\tc".to_string()).await;

        let (batches, notice) = collect_key_batches_until_notice(&mut actions).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 5,
                chars_skipped: 0,
            }
        );
        assert_eq!(
            batches,
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
        let (service, mut actions) = TextInputService::new(TextInputConfig {
            inter_char_delay: Duration::ZERO,
        });
        // 'é' 与 '中' 非 ASCII、'\r' 不可映射 → 全部跳过。
        service
            .type_text(client(1), "a\u{e9}\u{4e2d}\r".to_string())
            .await;

        let (batches, notice) = collect_key_batches_until_notice(&mut actions).await;
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 1,
                chars_skipped: 3,
            }
        );
        assert_eq!(batches, vec![vec![down(0x04)], vec![up(0x04)]]);
    }

    #[tokio::test]
    async fn text_input_cancel_interrupts_in_flight_typing_and_releases_all() {
        let (service, mut actions) = TextInputService::new(TextInputConfig {
            inter_char_delay: Duration::from_millis(20),
        });
        let _typing = tokio::spawn({
            let service = service.clone();
            async move { service.type_text(client(1), "abc".to_string()).await }
        });
        // 让服务按下第一个字符并进入节流间隔。
        tokio::task::yield_now().await;
        service.cancel(client(1)).await;

        // 取消发生在第一个字符释放之前：0 个字符完整键入，剩余文本丢弃。
        expect_key_batch(&mut actions, client(1), vec![down(0x04)]).await;
        assert!(matches!(
            receive_action(&mut actions).await,
            TextInputAction::ReleaseAll {
                client_id: found
            } if found == client(1)
        ));
        let TextInputAction::Notice(notice) = receive_action(&mut actions).await else {
            panic!("expected typed notice");
        };
        assert_eq!(
            notice,
            TextInputNotice::Typed {
                client_id: client(1),
                chars_typed: 0,
                chars_skipped: 0,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn text_input_inter_char_delay_gates_press_and_release() {
        let (service, mut actions) = TextInputService::new(TextInputConfig {
            inter_char_delay: Duration::from_millis(100),
        });
        let _typing = tokio::spawn({
            let service = service.clone();
            async move { service.type_text(client(1), "ab".to_string()).await }
        });

        // press(a) 在键入开始时立即发出。
        expect_key_batch(&mut actions, client(1), vec![down(0x04)]).await;
        // 100ms 节流间隔未到：release 尚未发出。
        tokio::time::advance(Duration::from_millis(50)).await;
        tokio::task::yield_now().await;
        assert!(
            actions.try_recv().is_err(),
            "release should wait for the full inter-char delay"
        );
        // 越过 release 节流点后再越过下一个 press 节流点。
        tokio::time::advance(Duration::from_millis(50)).await;
        expect_key_batch(&mut actions, client(1), vec![up(0x04)]).await;
        tokio::time::advance(Duration::from_millis(100)).await;
        expect_key_batch(&mut actions, client(1), vec![down(0x05)]).await;
    }
}
