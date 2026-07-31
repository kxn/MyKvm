# CH9329 协议与输入核心实施计划

> **供自动化执行者使用：** 必须使用 `executing-plans` 或 `subagent-driven-development` 按任务执行，并用复选框跟踪步骤。

**目标：** 完成可自动化验证的 CH9329 命令帧、应答解码、有序命令队列、键盘 6KRO 和鼠标状态机。

**架构：** `ipkvm-core` 保持同步和平台无关。协议层负责字节编解码，输入层用复制状态计算下一状态和命令批次，只有整个批次被 `CommandQueue` 接受后才提交状态；真实串口和设备应答策略不进入本计划。

**技术栈：** Rust 1.89、edition 2024、`thiserror`、`proptest`、Cargo workspace、标准库同步原语。

## 全局约束

- 仓库内自写文档使用中文。
- 关联 Gitea issue `#1`。
- 不实现真实串口、设备探测、波特率修改、桌面、RFB 或 Web 接入。
- CH9329 帧数据区最大为 64 字节。
- 串口默认值为 9600，不自动修改芯片参数。
- 所有生产代码先有能按预期失败的测试。
- 每个任务完成后运行该 crate 测试，并形成独立提交。
- 最终运行 `cargo fmt --all --check` 和 `cargo test --workspace --all-features`。

## 文件结构

```text
crates/ipkvm-core/src/
  ch9329/
    mod.rs       协议公共类型、命令和应答
    frame.rs     完整帧编码与校验
    decoder.rs   串口字节流增量解帧
    report.rs    键盘和鼠标报告
    input.rs     CH9329 键鼠状态机
  input.rs       设备无关输入事件和 InputSink
  serial.rs      CommandBatch 和 CommandQueue
  fake_serial.rs fake 命令队列
  lib.rs         公共导出和跨模块测试
```

---

### 任务 1：CH9329 完整帧与类型化报告

**文件：**

- 修改：`Cargo.toml`
- 修改：`Cargo.lock`
- 修改：`crates/ipkvm-core/Cargo.toml`
- 删除：`crates/ipkvm-core/src/ch9329.rs`
- 新建：`crates/ipkvm-core/src/ch9329/mod.rs`
- 新建：`crates/ipkvm-core/src/ch9329/frame.rs`
- 新建：`crates/ipkvm-core/src/ch9329/report.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`

**接口：**

- 产出：`Ch9329Frame::new`、`Ch9329Frame::parse`、`Ch9329Command::to_frame`。
- 产出：`KeyboardReport`、`AbsoluteMouseReport`、`RelativeMouseReport`。
- 产出：报告构造失败时使用的 `Ch9329ReportError`。

- [ ] **步骤 1：加入性质测试开发依赖**

根工作区：

```toml
[workspace.dependencies]
proptest = "1"
```

`ipkvm-core`：

```toml
[dev-dependencies]
proptest.workspace = true
```

- [ ] **步骤 2：写帧上限和解析失败测试**

在 `frame.rs` 的测试模块写入：

```rust
#[test]
fn rejects_payload_larger_than_protocol_limit() {
    assert_eq!(
        Ch9329Frame::new(0, 2, &[0; 65]),
        Err(Ch9329FrameError::DataTooLong(65))
    );
}

#[test]
fn parses_vendor_keyboard_frame() {
    let bytes = [0x57, 0xab, 0, 2, 8, 0, 0, 4, 0, 0, 0, 0, 0, 0x10];
    let frame = Ch9329Frame::parse(&bytes).unwrap();
    assert_eq!(frame.address(), 0);
    assert_eq!(frame.command(), 2);
    assert_eq!(frame.data(), &[0, 0, 4, 0, 0, 0, 0, 0]);
}

#[test]
fn rejects_bad_checksum() {
    let bytes = [0x57, 0xab, 0, 1, 0, 0xff];
    assert!(matches!(
        Ch9329Frame::parse(&bytes),
        Err(Ch9329FrameError::ChecksumMismatch { .. })
    ));
}
```

- [ ] **步骤 3：运行测试确认按预期失败**

运行：

```powershell
cargo test -p ipkvm-core ch9329::frame
```

预期：编译失败，提示 `Ch9329FrameError` 或 `parse` 尚未定义。

- [ ] **步骤 4：实现完整帧**

实现以下公共签名：

```rust
pub const MAX_DATA_LEN: usize = 64;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329FrameError {
    DataTooLong(usize),
    FrameTooShort(usize),
    InvalidHeader([u8; 2]),
    LengthMismatch { declared: usize, actual: usize },
    ChecksumMismatch { expected: u8, actual: u8 },
}

impl Ch9329Frame {
    pub fn new(address: u8, command: u8, data: &[u8]) -> Result<Self, Ch9329FrameError>;
    pub fn parse(bytes: &[u8]) -> Result<Self, Ch9329FrameError>;
    pub fn address(&self) -> u8;
    pub fn command(&self) -> u8;
    pub fn data(&self) -> &[u8];
    pub fn as_bytes(&self) -> &[u8];
}
```

`parse` 要求输入恰好包含一帧，总长度等于 `6 + LEN`，并校验帧头、64 字节上限和累加和。

- [ ] **步骤 5：写类型化报告金样测试**

在 `report.rs` 写入 WCH 示例：

```rust
#[test]
fn keyboard_a_matches_vendor_frame() {
    let command = Ch9329Command::Keyboard(KeyboardReport::new(0, [4, 0, 0, 0, 0, 0]));
    assert_eq!(
        command.to_frame(0).unwrap().as_bytes(),
        &[0x57, 0xab, 0, 2, 8, 0, 0, 4, 0, 0, 0, 0, 0, 0x10]
    );
}

#[test]
fn absolute_mouse_matches_vendor_example() {
    let report = AbsoluteMouseReport::new(0, 320, 533, 0).unwrap();
    assert_eq!(
        Ch9329Command::MouseAbsolute(report)
            .to_frame(0)
            .unwrap()
            .as_bytes(),
        &[0x57, 0xab, 0, 4, 7, 2, 0, 0x40, 1, 0x15, 2, 0, 0x67]
    );
}
```

- [ ] **步骤 6：运行报告测试确认失败**

运行：

```powershell
cargo test -p ipkvm-core ch9329::report
```

预期：编译失败，提示报告和命令类型尚未定义。

- [ ] **步骤 7：实现报告与命令**

实现：

```rust
pub enum Ch9329Command {
    GetInfo,
    Keyboard(KeyboardReport),
    MouseAbsolute(AbsoluteMouseReport),
    MouseRelative(RelativeMouseReport),
}

impl Ch9329Command {
    pub fn to_frame(&self, address: u8) -> Result<Ch9329Frame, Ch9329FrameError>;
}
```

报告构造器拒绝按钮掩码高 5 位、绝对坐标大于 4095，以及相对字段 `-128`。报告字段保持私有，只通过 `data()` 生成固定长度数据。

实现以下签名，确保无效报告不能进入 `Ch9329Command`：

```rust
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329ReportError {
    InvalidButtonMask(u8),
    CoordinateOutOfRange { axis: &'static str, value: u16 },
    RelativeValueOutOfRange { field: &'static str, value: i8 },
}

impl KeyboardReport {
    pub fn new(modifiers: u8, keys: [u8; 6]) -> Self;
    pub fn data(&self) -> [u8; 8];
}

impl AbsoluteMouseReport {
    pub fn new(
        buttons: u8,
        x: u16,
        y: u16,
        wheel: i8,
    ) -> Result<Self, Ch9329ReportError>;
    pub fn data(&self) -> [u8; 7];
}

impl RelativeMouseReport {
    pub fn new(
        buttons: u8,
        dx: i8,
        dy: i8,
        wheel: i8,
    ) -> Result<Self, Ch9329ReportError>;
    pub fn data(&self) -> [u8; 5];
}
```

`RelativeValueOutOfRange` 专门表示 CH9329 不接受的 `-128`；`field` 仅取 `dx`、`dy` 或 `wheel`。

- [ ] **步骤 8：运行任务测试并提交**

运行：

```powershell
cargo test -p ipkvm-core ch9329
git add Cargo.toml Cargo.lock crates/ipkvm-core
git commit -m "feat: add CH9329 protocol frames (#1)"
```

预期：CH9329 帧和报告测试全部通过。

---

### 任务 2：应答解析与增量解帧

**文件：**

- 新建：`crates/ipkvm-core/src/ch9329/decoder.rs`
- 修改：`crates/ipkvm-core/src/ch9329/mod.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`

**接口：**

- 消费：`Ch9329Frame::parse`。
- 产出：`Ch9329Decoder::push`、`Ch9329Response::parse`、`Ch9329Info`、`CommandStatus`。
- 产出：`Ch9329ResponseError`，区分未知应答命令与数据长度错误。

- [ ] **步骤 1：写应答解析测试**

```rust
#[test]
fn parses_get_info_led_state() {
    let frame = Ch9329Frame::new(0, 0x81, &[0x31, 1, 0b0000_0011, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(
        Ch9329Response::parse(&frame).unwrap(),
        Ch9329Response::Info(Ch9329Info {
            version: 0x31,
            usb_enumerated: true,
            leds: LockLedState {
                num_lock: true,
                caps_lock: true,
                scroll_lock: false,
            },
            reserved: [0; 5],
        })
    );
}

#[test]
fn preserves_unknown_status_code() {
    let frame = Ch9329Frame::new(0, 0xc2, &[0xaa]).unwrap();
    assert!(matches!(
        Ch9329Response::parse(&frame),
        Ok(Ch9329Response::Error {
            command: 2,
            status: CommandStatus::Unknown(0xaa)
        })
    ));
}
```

- [ ] **步骤 2：运行应答测试确认失败**

运行：

```powershell
cargo test -p ipkvm-core response
```

预期：编译失败，提示应答类型尚未定义。

- [ ] **步骤 3：实现应答类型**

实现 `CommandStatus` 的 `0x00` 和 `0xe1..=0xe6` 映射；正常 `0x81` 必须有 8 字节，`0x82`、`0x84`、`0x85` 必须有 1 字节；异常应答命令码按 `command & 0x3f` 恢复原命令。

使用以下公共形状：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    Success,
    SerialTimeout,
    InvalidHeader,
    InvalidCommand,
    ChecksumError,
    InvalidParameters,
    OperationFailed,
    Unknown(u8),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ch9329Response {
    Info(Ch9329Info),
    Acknowledgement { command: u8, status: CommandStatus },
    Error { command: u8, status: CommandStatus },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329ResponseError {
    UnexpectedCommand(u8),
    InvalidDataLength {
        command: u8,
        expected: usize,
        actual: usize,
    },
}

impl Ch9329Response {
    pub fn parse(frame: &Ch9329Frame) -> Result<Self, Ch9329ResponseError>;
}
```

`0x81` 解析为 `Info`；`0x82`、`0x84`、`0x85` 解析为 `Acknowledgement`；`0xc1`、`0xc2`、`0xc4`、`0xc5` 解析为 `Error`。其他命令返回 `UnexpectedCommand`，不猜测未来协议。

- [ ] **步骤 4：写增量解帧测试**

```rust
#[test]
fn decodes_frame_at_every_split_boundary() {
    let bytes = Ch9329Frame::new(0, 1, &[]).unwrap().as_bytes().to_vec();
    for split in 0..bytes.len() {
        let mut decoder = Ch9329Decoder::new();
        assert!(decoded_frames(decoder.push(&bytes[..split])).is_empty());
        let events = decoder.push(&bytes[split..]);
        assert_eq!(decoded_frames(events), vec![Ch9329Frame::new(0, 1, &[]).unwrap()]);
    }
}

#[test]
fn recovers_after_noise_and_bad_checksum() {
    let good = Ch9329Frame::new(0, 1, &[]).unwrap();
    let mut bytes = vec![1, 2, 3, 0x57, 0xab, 0, 1, 0, 0xff];
    bytes.extend_from_slice(good.as_bytes());
    let mut decoder = Ch9329Decoder::new();
    assert_eq!(decoded_frames(decoder.push(&bytes)), vec![good]);
    assert!(decoder.buffered_len() <= 1);
}
```

`decoded_frames` 只在测试模块中过滤成功帧；不完整输入允许返回空列表。

测试辅助函数明确为：

```rust
fn decoded_frames(
    events: Vec<Result<Ch9329Frame, Ch9329DecodeError>>,
) -> Vec<Ch9329Frame> {
    events.into_iter().filter_map(Result::ok).collect()
}
```

- [ ] **步骤 5：运行解帧测试确认失败**

运行：

```powershell
cargo test -p ipkvm-core decoder
```

预期：编译失败，提示 `Ch9329Decoder` 尚未定义。

- [ ] **步骤 6：实现增量解帧**

实现：

```rust
pub enum Ch9329DecodeError {
    NoiseDiscarded(usize),
    Frame(Ch9329FrameError),
}

pub struct Ch9329Decoder {
    buffer: Vec<u8>,
}

impl Ch9329Decoder {
    pub fn new() -> Self;
    pub fn push(
        &mut self,
        bytes: &[u8],
    ) -> Vec<Result<Ch9329Frame, Ch9329DecodeError>>;
    pub fn buffered_len(&self) -> usize;
}
```

无帧头时最多保留尾部单个 `0x57`；非法长度或校验失败后至少丢弃一个字节再继续同步，不能因坏帧阻塞后续好帧。

- [ ] **步骤 7：运行任务测试并提交**

```powershell
cargo test -p ipkvm-core ch9329
git add crates/ipkvm-core/src/ch9329 crates/ipkvm-core/src/lib.rs
git commit -m "feat: decode CH9329 responses (#1)"
```

---

### 任务 3：有序命令批次与 fake 队列

**文件：**

- 修改：`crates/ipkvm-core/src/serial.rs`
- 修改：`crates/ipkvm-core/src/fake_serial.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`

**接口：**

- 产出：`CommandBatch`、`CommandQueue`、`QueueStats`、`CommandQueueError`。
- 产出：空批次构造失败时使用的 `CommandBatchError`。
- 产出：`FakeCommandQueue` 的批次记录和下一批失败注入。
- 产出：`CommandBatch::frames(&self) -> &[Ch9329Frame]`。
- 产出：可克隆的 `FakeCommandQueue`，克隆值共享同一内部状态。

- [ ] **步骤 1：写批次和失败注入测试**

```rust
#[test]
fn fake_queue_preserves_batch_boundaries_and_order() {
    let queue = FakeCommandQueue::new();
    let first = CommandBatch::new(vec![frame(2), frame(4)]).unwrap();
    let second = CommandBatch::new(vec![frame(5)]).unwrap();
    queue.enqueue_batch(first.clone()).unwrap();
    queue.enqueue_batch(second.clone()).unwrap();
    assert_eq!(queue.accepted_batches(), vec![first, second]);
    assert_eq!(queue.stats().batches_accepted, 2);
    assert_eq!(queue.stats().frames_accepted, 3);
}

#[test]
fn fake_queue_rejects_configured_batch_without_recording_it() {
    let queue = FakeCommandQueue::new();
    queue.fail_next(CommandQueueError::Closed);
    assert_eq!(
        queue.enqueue_batch(CommandBatch::new(vec![frame(2)]).unwrap()),
        Err(CommandQueueError::Closed)
    );
    assert!(queue.accepted_batches().is_empty());
}
```

- [ ] **步骤 2：运行测试确认失败**

```powershell
cargo test -p ipkvm-core fake_queue
```

预期：编译失败，提示新队列类型尚未定义。

- [ ] **步骤 3：实现队列契约**

```rust
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandBatchError {
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandBatch {
    frames: Vec<Ch9329Frame>,
}

impl CommandBatch {
    pub fn new(frames: Vec<Ch9329Frame>) -> Result<Self, CommandBatchError>;
    pub fn frames(&self) -> &[Ch9329Frame];
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandQueueError {
    Closed,
}

pub type CommandQueueResult<T> = Result<T, CommandQueueError>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueStats {
    pub batches_accepted: u64,
    pub frames_accepted: u64,
}

pub trait CommandQueue {
    fn enqueue_batch(&self, batch: CommandBatch) -> CommandQueueResult<()>;
    fn stats(&self) -> QueueStats;
}
```

`CommandBatch::new` 拒绝空批次。`FakeCommandQueue` 使用 `Arc<Mutex<_>>` 同时保护失败注入、批次记录和统计，保证所有克隆值观察同一状态，且批次整体接受或拒绝。

- [ ] **步骤 4：删除旧接口并运行全 crate 测试**

删除 `SerialWriter`、`SerialStats`、`FakeSerialWriter` 及其旧测试，更新公共导出。

运行：

```powershell
cargo test -p ipkvm-core --all-features
git add crates/ipkvm-core/src
git commit -m "refactor: add atomic command queue (#1)"
```

---

### 任务 4：键盘 6KRO 状态机

**文件：**

- 修改：`crates/ipkvm-core/src/input.rs`
- 新建：`crates/ipkvm-core/src/ch9329/input.rs`
- 修改：`crates/ipkvm-core/src/ch9329/mod.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`

**接口：**

- 产出：`KeyboardUsage`、不含 `type_text` 的 `InputSink`。
- 产出：`Ch9329InputSink<Q: CommandQueue>`。
- 产出：包含输入校验、队列拒绝和协议构造来源的 `InputError`。

- [ ] **步骤 1：先写完整键盘行为测试**

```rust
#[test]
fn keyboard_usage_rejects_reserved_zero() {
    assert_eq!(
        KeyboardUsage::new(0),
        Err(InputError::InvalidKeyUsage(0))
    );
}

#[test]
fn keyboard_sink_accepts_regular_key_down() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    sink.handle_key(KeyEvent::Down {
        usage: KeyboardUsage::new(0x04).unwrap(),
    })
    .unwrap();
    assert_eq!(queue.accepted_batches().len(), 1);
}

#[test]
fn modifier_uses_modifier_byte_without_regular_key_slot() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    sink.handle_key(KeyEvent::Down {
        usage: KeyboardUsage::new(0xe1).unwrap(),
    })
    .unwrap();
    assert_eq!(
        queue.accepted_batches().last().unwrap().frames()[0].data(),
        &[0x02, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn seventh_regular_key_is_rejected_without_state_change() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    for value in 0x04..=0x09 {
        sink.handle_key(KeyEvent::Down {
            usage: KeyboardUsage::new(value).unwrap(),
        })
        .unwrap();
    }
    assert_eq!(
        sink.handle_key(KeyEvent::Down {
            usage: KeyboardUsage::new(0x0a).unwrap(),
        }),
        Err(InputError::RolloverLimitExceeded)
    );
    assert_eq!(queue.accepted_batches().len(), 6);

    sink.handle_key(KeyEvent::Up {
        usage: KeyboardUsage::new(0x04).unwrap(),
    })
    .unwrap();
    sink.handle_key(KeyEvent::Down {
        usage: KeyboardUsage::new(0x0a).unwrap(),
    })
    .unwrap();
    assert_eq!(
        queue.accepted_batches().last().unwrap().frames()[0].data(),
        &[0, 0, 5, 6, 7, 8, 9, 10]
    );
}

#[test]
fn duplicate_down_does_not_enqueue_another_batch() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let event = KeyEvent::Down {
        usage: KeyboardUsage::new(0x04).unwrap(),
    };
    sink.handle_key(event).unwrap();
    sink.handle_key(event).unwrap();
    assert_eq!(queue.accepted_batches().len(), 1);
}

#[test]
fn queue_failure_does_not_commit_key_state() {
    let queue = FakeCommandQueue::new();
    queue.fail_next(CommandQueueError::Closed);
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let key = KeyboardUsage::new(4).unwrap();
    assert!(sink.handle_key(KeyEvent::Down { usage: key }).is_err());
    sink.handle_key(KeyEvent::Down { usage: key }).unwrap();
    assert_eq!(queue.accepted_batches().len(), 1);
}

proptest! {
    #[test]
    fn keyboard_state_never_contains_duplicates_or_more_than_six_keys(
        events in proptest::collection::vec((0x04u8..=0x20, any::<bool>()), 0..128)
    ) {
        let mut state = KeyboardState::default();
        for (value, down) in events {
            let usage = KeyboardUsage::new(value).unwrap();
            let event = if down {
                KeyEvent::Down { usage }
            } else {
                KeyEvent::Up { usage }
            };
            match state.apply_key(event) {
                Ok(Some((next, _))) => state = next,
                Ok(None) | Err(InputError::RolloverLimitExceeded) => {}
                Err(error) => panic!("unexpected keyboard error: {error}"),
            }
            let occupied: Vec<_> = state.keys.iter().copied().filter(|key| *key != 0).collect();
            let unique: std::collections::HashSet<_> = occupied.iter().copied().collect();
            prop_assert_eq!(occupied.len(), unique.len());
            prop_assert!(occupied.len() <= 6);
        }
    }
}
```

同时删除 `RecordingSink::type_text`，使旧 trait 实现按预期编译失败。

- [ ] **步骤 2：运行测试确认失败**

```powershell
cargo test -p ipkvm-core keyboard
```

预期：编译失败，提示 `KeyboardUsage` 或 `Ch9329InputSink` 尚未定义。

- [ ] **步骤 3：实现键盘状态和事务提交**

先把设备无关输入接口收紧为：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardUsage(u8);

impl KeyboardUsage {
    pub fn new(value: u8) -> InputResult<Self>;
    pub fn get(self) -> u8;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum InputError {
    InvalidKeyUsage(u8),
    RolloverLimitExceeded,
    InvalidFramebufferSize { width: u32, height: u32 },
    PointerOutOfBounds { coordinate: u32, extent: u32 },
    PointerPositionUnknown,
    CommandQueue(#[from] CommandQueueError),
    Frame(#[from] Ch9329FrameError),
    Report(#[from] Ch9329ReportError),
}

pub trait InputSink {
    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()>;
    fn handle_key(&mut self, event: KeyEvent) -> InputResult<()>;
    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()>;
    fn release_all(&mut self) -> InputResult<()>;
}
```

`KeyboardUsage::new` 拒绝 HID 保留值 `0x00..=0x03`。`KeyEvent` 的字段统一为 `usage: KeyboardUsage`，不再让调用者绕过校验。

实现私有：

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct KeyboardState {
    modifiers: u8,
    keys: [u8; 6],
}

fn apply_key(
    &self,
    event: KeyEvent,
) -> InputResult<Option<(KeyboardState, KeyboardReport)>>;
```

`0xe0..=0xe7` 映射 modifier bit；普通键按下放入第一个空槽，释放后向左压紧。重复按下和幽灵释放返回 `Ok(None)`。

- [ ] **步骤 4：运行键盘测试确认通过**

```powershell
cargo test -p ipkvm-core keyboard
```

实现时先在副本上计算报告，创建单帧 `CommandBatch`，成功入队后再替换 `self.keyboard`。

- [ ] **步骤 5：写强制释放测试并确认失败**

```rust
#[test]
fn release_all_enqueues_zero_keyboard_even_when_state_is_empty() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    sink.release_all().unwrap();
    assert_eq!(
        queue.accepted_batches().last().unwrap().frames()[0].data(),
        &[0; 8]
    );
}
```

运行：

```powershell
cargo test -p ipkvm-core release_all_enqueues_zero_keyboard_even_when_state_is_empty
```

预期：失败，因为 `release_all` 尚未生成键盘释放批次。

- [ ] **步骤 6：实现键盘强制释放并提交**

`release_all()` 即使本地为空也提交全零键盘报告；鼠标释放在任务 5 扩展到同一批次。

```powershell
cargo test -p ipkvm-core keyboard --all-features
git add crates/ipkvm-core/src
git commit -m "feat: add CH9329 keyboard state (#1)"
```

---

### 任务 5：鼠标状态机和相对位移拆包

**文件：**

- 修改：`crates/ipkvm-core/src/ch9329/input.rs`
- 修改：`crates/ipkvm-core/src/geometry.rs`
- 修改：`crates/ipkvm-core/src/input.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`

**接口：**

- 消费：`AbsoluteMouseReport`、`RelativeMouseReport`、`CommandBatch`。
- 产出：绝对坐标、按钮、滚轮、模式切换和释放行为。
- 产出私有辅助：`split_relative(dx: i16, dy: i16, wheel: i16) -> Vec<(i8, i8, i8)>`。

- [ ] **步骤 1：写厂家坐标公式测试**

```rust
#[test]
fn pointer_mapping_matches_vendor_formula() {
    assert_eq!(
        map_framebuffer_axis(100, 1280).unwrap(),
        320
    );
    assert_eq!(
        map_framebuffer_axis(1279, 1280).unwrap(),
        4092
    );
}

#[test]
fn pointer_mapping_rejects_invalid_coordinate() {
    assert!(map_framebuffer_axis(1280, 1280).is_err());
    assert!(map_framebuffer_axis(0, 0).is_err());
}

proptest! {
    #[test]
    fn mapped_coordinates_stay_in_range(coordinate in any::<u16>(), extent in 1u16..) {
        let coordinate = u32::from(coordinate) % u32::from(extent);
        prop_assert!(
            map_framebuffer_axis(coordinate, u32::from(extent)).unwrap() <= 4095
        );
    }
}
```

- [ ] **步骤 2：运行坐标测试确认失败并实现**

```powershell
cargo test -p ipkvm-core pointer_mapping
```

实现 `floor(4096 * coordinate / extent)`，使用 `u64` 中间值避免溢出。删除旧的视图矩形映射，因为 DPI 和黑边换算属于桌面适配层。

- [ ] **步骤 3：写鼠标按钮和位置测试**

覆盖：

```rust
#[test]
fn absolute_button_requires_known_position() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    assert_eq!(
        sink.handle_pointer(PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        }),
        Err(InputError::PointerPositionUnknown)
    );
    assert!(queue.accepted_batches().is_empty());
}

#[test]
fn absolute_move_carries_held_buttons() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let size = FramebufferSize {
        width: 1280,
        height: 720,
    };
    sink.handle_pointer(PointerEvent::AbsoluteMove {
        x: 100,
        y: 100,
        framebuffer_size: size,
    })
    .unwrap();
    sink.handle_pointer(PointerEvent::Button {
        button: PointerButton::Left,
        down: true,
    })
    .unwrap();
    sink.handle_pointer(PointerEvent::AbsoluteMove {
        x: 200,
        y: 100,
        framebuffer_size: size,
    })
    .unwrap();
    assert_eq!(
        queue.accepted_batches().last().unwrap().frames()[0].data()[1],
        0x01
    );
}

#[test]
fn releasing_one_button_preserves_other_buttons() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
    for button in [PointerButton::Left, PointerButton::Right] {
        sink.handle_pointer(PointerEvent::Button { button, down: true })
            .unwrap();
    }
    sink.handle_pointer(PointerEvent::Button {
        button: PointerButton::Left,
        down: false,
    })
    .unwrap();
    assert_eq!(
        queue.accepted_batches().last().unwrap().frames()[0].data()[1],
        0x02
    );
}
```

- [ ] **步骤 4：运行测试确认失败并实现鼠标状态**

私有状态：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
struct MouseState {
    mode: MouseMode,
    buttons: u8,
    last_absolute: Option<(u16, u16)>,
}
```

绝对模式的按钮和滚轮使用最后坐标；相对模式使用零位移相对报告。重复按钮事件不产生命令。

- [ ] **步骤 5：写大位移守恒测试**

```rust
#[test]
fn relative_motion_is_split_without_losing_distance() {
    assert_eq!(
        split_relative(200, -200, 0),
        vec![(127, -127, 0), (73, -73, 0)]
    );
}

proptest! {
    #[test]
    fn relative_chunks_preserve_totals(
        dx in any::<i16>(),
        dy in any::<i16>(),
        wheel in any::<i16>()
    ) {
        let chunks = split_relative(dx, dy, wheel);
        prop_assert!(chunks.iter().all(|(x, y, w)| {
            *x != -128 && *y != -128 && *w != -128
        }));
        let totals = chunks.iter().fold((0i32, 0i32, 0i32), |sum, part| {
            (
                sum.0 + i32::from(part.0),
                sum.1 + i32::from(part.1),
                sum.2 + i32::from(part.2),
            )
        });
        prop_assert_eq!(
            totals,
            (i32::from(dx), i32::from(dy), i32::from(wheel))
        );
    }
}
```

- [ ] **步骤 6：运行测试确认失败并实现拆包**

每次从剩余值取 `clamp(-127, 127)`，直到 `dx`、`dy` 和 `wheel` 全为零。零位移事件不产生命令。

- [ ] **步骤 7：写模式切换、失败回滚和强制释放测试**

```rust
#[test]
fn mode_switch_releases_buttons_in_old_mode() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
    sink.handle_pointer(PointerEvent::Button {
        button: PointerButton::Left,
        down: true,
    })
    .unwrap();
    sink.set_mouse_mode(MouseMode::Absolute).unwrap();
    let release = &queue.accepted_batches().last().unwrap().frames()[0];
    assert_eq!(release.command(), 0x05);
    assert_eq!(release.data(), &[1, 0, 0, 0, 0]);
}

#[test]
fn failed_button_batch_does_not_commit_button_state() {
    let queue = FakeCommandQueue::new();
    queue.fail_next(CommandQueueError::Closed);
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
    let event = PointerEvent::Button {
        button: PointerButton::Left,
        down: true,
    };
    assert!(sink.handle_pointer(event).is_err());
    sink.handle_pointer(event).unwrap();
    assert_eq!(
        queue.accepted_batches().last().unwrap().frames()[0].data()[1],
        0x01
    );
}

#[test]
fn release_all_always_contains_keyboard_and_mouse_release() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    sink.release_all().unwrap();
    let frames = queue.accepted_batches().last().unwrap().frames();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].command(), 0x02);
    assert_eq!(frames[0].data(), &[0; 8]);
    assert_eq!(frames[1].command(), 0x05);
    assert_eq!(frames[1].data(), &[1, 0, 0, 0, 0]);
}
```

运行这些测试并确认：模式切换和强制释放测试因鼠标释放尚未加入批次而失败，失败回滚测试因鼠标状态过早提交而失败。

- [ ] **步骤 8：实现并提交**

```powershell
cargo test -p ipkvm-core --all-features
git add crates/ipkvm-core/src
git commit -m "feat: add CH9329 mouse state (#1)"
```

---

### 任务 6：默认配置和文档收口

**文件：**

- 修改：`crates/ipkvm-session/src/lib.rs`
- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`

**接口：**

- 产出：会话默认波特率 9600。

- [ ] **步骤 1：先修改会话默认值测试**

把默认配置断言从 115200 改为 9600，并重命名为：

```rust
#[test]
fn session_config_defaults_to_factory_serial_baud() {
    let config = ConsoleSessionConfig::default_for_devices("video0", "COM3");
    assert_eq!(config.baud_rate(), 9_600);
}
```

运行：

```powershell
cargo test -p ipkvm-session session_config_defaults_to_factory_serial_baud
```

预期：失败，实际值仍为 115200。

- [ ] **步骤 2：修改默认值并更新文档状态**

将 `ConsoleSessionConfig::default_for_devices` 改为 9600。README 说明 CH9329 协议编解码和输入状态核心已完成，真实串口仍未实现；粗粒度设计的阶段 0 对应条目标记为已完成描述，不使用模糊占位符。

- [ ] **步骤 3：完整验证并提交**

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
git diff --check
git add crates/ipkvm-session/src/lib.rs README.md docs/ipkvm-coarse-design.md
git commit -m "docs: record CH9329 core status (#1)"
```

预期：完整工作区测试通过，文档无冲突标记或占位符。
