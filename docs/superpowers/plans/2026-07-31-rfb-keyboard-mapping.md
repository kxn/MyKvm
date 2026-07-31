# RFB 键盘映射实施计划

> **面向自动化执行：** 必须使用 `executing-plans` 按任务顺序实施；每个行为先写测试并确认红灯，再写生产代码。步骤使用复选框跟踪。

**目标：** 为 en-US 目标键盘实现符合 RFC 6143 字符语义的状态化 RFB keysym 映射，并用原子键盘批次保证 CH9329 和 mapper 的失败回滚。

**架构：** `ipkvm-core` 只增加通用的原子键盘批次；`ipkvm-headless::rfb_input` 持有 RFB/X11 专用映射表和状态机。mapper 在活动状态副本上计算目标 HID usage 集合，只有非空差异批次被 sink 接受后才提交内部状态。

**技术栈：** Rust 1.89、edition 2024、标准库 `BTreeMap/BTreeSet`、thiserror 2、现有 fake command queue、本地统一验证脚本。

## 全局约束

- 所有项目文档使用中文；外部类型名、协议名、代码和链接保持原文。
- 不增加 xkbcommon、平台键盘 API 或其他外部依赖。
- 只支持 en-US 目标键盘；Unicode、死键、组合字符和输入法不猜测。
- RFB 映射只位于 `ipkvm-headless`，`ipkvm-core` 不出现 keysym 或 RFB 类型。
- 每个生产行为必须先观察对应自动化测试因缺少该行为而失败。
- 不使用固定 sleep、真实串口、真实硬件或人工输入作为验收。
- 不修改主工作区中用户未提交的 `AGENTS.md`。

## 文件结构

```text
crates/ipkvm-core/src/input.rs
    InputSink 原子键盘批次契约

crates/ipkvm-core/src/ch9329/input.rs
    CH9329 键盘状态的原子批次应用

crates/ipkvm-headless/src/rfb_input/mod.rs
    公共 mapper、结果和错误类型导出

crates/ipkvm-headless/src/rfb_input/keymap.rs
    en-US ASCII、X11 特殊键和 HID usage 的纯映射

crates/ipkvm-headless/src/rfb_input/keyboard.rs
    活动 keysym、目标 HID 状态、Shift 规则和事务提交
```

---

### 任务 1：给 InputSink 增加原子键盘批次

**文件：**

- 修改：`crates/ipkvm-core/src/input.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`
- 修改：`crates/ipkvm-core/src/ch9329/input.rs`

**接口：**

- 产出：`InputSink::handle_key_batch(&mut self, events: &[KeyEvent])`
- 保留：`InputSink::handle_key(&mut self, event: KeyEvent)`
- 保证：批次只发送最终 HID 报告；失败不提交内部状态

- [x] **步骤 1：写入 trait 和 CH9329 红灯测试**

在 `crates/ipkvm-core/src/ch9329/input.rs` 增加：

```rust
#[test]
fn key_batch_enqueues_one_final_report() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let shift = KeyboardUsage::new(0xe1).unwrap();
    let a = KeyboardUsage::new(0x04).unwrap();

    sink.handle_key_batch(&[
        KeyEvent::Down { usage: shift },
        KeyEvent::Down { usage: a },
    ])
    .unwrap();

    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].frames().len(), 1);
    assert_eq!(
        batches[0].frames()[0].as_bytes(),
        &[0x57, 0xab, 0, 2, 8, 0x02, 0, 0x04, 0, 0, 0, 0, 0, 0x12]
    );
}

#[test]
fn failed_key_batch_does_not_commit_partial_state() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let keys = (0x04..=0x0a)
        .map(|usage| KeyEvent::Down {
            usage: KeyboardUsage::new(usage).unwrap(),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        sink.handle_key_batch(&keys),
        Err(InputError::RolloverLimitExceeded)
    );
    assert!(queue.accepted_batches().is_empty());

    sink.handle_key(KeyEvent::Down {
        usage: KeyboardUsage::new(0x0a).unwrap(),
    })
    .unwrap();
    assert_eq!(queue.accepted_batches().len(), 1);
}

#[test]
fn empty_or_net_unchanged_key_batch_does_not_enqueue() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let a = KeyboardUsage::new(0x04).unwrap();

    sink.handle_key_batch(&[]).unwrap();
    sink.handle_key_batch(&[
        KeyEvent::Down { usage: a },
        KeyEvent::Up { usage: a },
    ])
    .unwrap();

    assert!(queue.accepted_batches().is_empty());
}
```

在 `crates/ipkvm-core/src/lib.rs` 的 `RecordingSink` 中暂时不实现新方法，让 trait 变更测试形成编译红灯。

- [x] **步骤 2：确认批次 API 红灯**

运行：

```powershell
cargo test -p ipkvm-core key_batch
```

预期：编译失败，指出 `Ch9329InputSink` 没有 `handle_key_batch`。

- [x] **步骤 3：扩展 InputSink 契约**

把 `KeyboardUsage` 派生扩展为：

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyboardUsage(u8);
```

把 trait 改为：

```rust
pub trait InputSink {
    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()>;

    fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
        self.handle_key_batch(std::slice::from_ref(&event))
    }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()>;
    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()>;
    fn release_all(&mut self) -> InputResult<()>;
}
```

`RecordingSink` 增加：

```rust
fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
    self.keys.extend_from_slice(events);
    Ok(())
}
```

移除其单独的 `handle_key` 实现，测试默认单事件方法确实委托到批次。

- [x] **步骤 4：实现 CH9329 原子批次**

把 `KeyboardState::apply_key` 改为：

```rust
fn apply_keys(
    &self,
    events: &[KeyEvent],
) -> InputResult<Option<(KeyboardState, KeyboardReport)>> {
    let mut next = *self;
    for event in events {
        match *event {
            KeyEvent::Down { usage } => {
                next.press(usage.get())?;
            }
            KeyEvent::Up { usage } => {
                next.release(usage.get());
            }
        }
    }
    if next == *self {
        return Ok(None);
    }
    Ok(Some((next, next.report())))
}
```

在 `Ch9329InputSink` 增加：

```rust
pub fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
    self.handle_key_batch(std::slice::from_ref(&event))
}

pub fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
    let Some((next, report)) = self.keyboard.apply_keys(events)? else {
        return Ok(());
    };
    self.enqueue_commands(vec![Ch9329Command::Keyboard(report)])?;
    self.keyboard = next;
    Ok(())
}
```

`impl InputSink for Ch9329InputSink<Q>` 实现 `handle_key_batch`，单事件方法可继续显式委托固有方法。

- [x] **步骤 5：增加队列失败重试测试**

```rust
#[test]
fn rejected_key_batch_can_be_retried_without_state_drift() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let events = [
        KeyEvent::Down {
            usage: KeyboardUsage::new(0xe1).unwrap(),
        },
        KeyEvent::Down {
            usage: KeyboardUsage::new(0x04).unwrap(),
        },
    ];
    queue.fail_next(CommandQueueError::Closed);

    assert!(sink.handle_key_batch(&events).is_err());
    sink.handle_key_batch(&events).unwrap();

    assert_eq!(queue.accepted_batches().len(), 1);
    assert_eq!(
        queue.accepted_batches()[0].frames()[0].as_bytes()[5..13],
        [0x02, 0, 0x04, 0, 0, 0, 0, 0]
    );
}
```

- [x] **步骤 6：运行 core 测试并确认绿灯**

```powershell
cargo test -p ipkvm-core
cargo clippy -p ipkvm-core --all-targets --all-features -- -D warnings
```

预期：全部通过。

- [x] **步骤 7：提交**

```powershell
git add crates/ipkvm-core
git commit -m "feat: add atomic keyboard input batches (#7)"
```

---

### 任务 2：建立 RFB 键盘公共契约和 ASCII 映射

**文件：**

- 修改：`crates/ipkvm-headless/Cargo.toml`
- 修改：`crates/ipkvm-headless/src/lib.rs`
- 新建：`crates/ipkvm-headless/src/rfb_input/mod.rs`
- 新建：`crates/ipkvm-headless/src/rfb_input/keymap.rs`
- 新建：`crates/ipkvm-headless/src/rfb_input/keyboard.rs`

**接口：**

- 产出：`RfbKeyboardMapper`
- 产出：`RfbKeyboardOutcome`
- 产出：`RfbKeyboardError`
- 内部产出：`MappedKey`、`ShiftRequirement` 和 `map_keysym`

- [x] **步骤 1：写入 ASCII 映射红灯测试**

在 `keymap.rs` 先写测试模块，调用尚不存在的 `map_keysym`：

```rust
#[test]
fn maps_every_printable_ascii_key_for_en_us() {
    let cases = [
        ('a', 0x04, false),
        ('z', 0x1d, false),
        ('A', 0x04, true),
        ('Z', 0x1d, true),
        ('1', 0x1e, false),
        ('0', 0x27, false),
        ('!', 0x1e, true),
        ('@', 0x1f, true),
        ('#', 0x20, true),
        ('-', 0x2d, false),
        ('_', 0x2d, true),
        ('=', 0x2e, false),
        ('+', 0x2e, true),
        ('[', 0x2f, false),
        ('{', 0x2f, true),
        (']', 0x30, false),
        ('}', 0x30, true),
        ('\\', 0x31, false),
        ('|', 0x31, true),
        (';', 0x33, false),
        (':', 0x33, true),
        ('\'', 0x34, false),
        ('"', 0x34, true),
        ('`', 0x35, false),
        ('~', 0x35, true),
        (',', 0x36, false),
        ('<', 0x36, true),
        ('.', 0x37, false),
        ('>', 0x37, true),
        ('/', 0x38, false),
        ('?', 0x38, true),
        (' ', 0x2c, false),
    ];

    for (character, usage, shift) in cases {
        assert_eq!(
            map_keysym(character as u32).unwrap(),
            MappedKey::Character {
                usage: KeyboardUsage::new(usage).unwrap(),
                shift: ShiftRequirement::from_required(shift),
            }
        );
    }
}

#[test]
fn rejects_non_ascii_character_keysyms() {
    assert_eq!(map_keysym(0x00e9), Err(RfbKeyboardError::UnsupportedKeysym(0x00e9)));
    assert_eq!(
        map_keysym(0x0101_f642),
        Err(RfbKeyboardError::UnsupportedKeysym(0x0101_f642))
    );
}
```

- [x] **步骤 2：确认模块和映射红灯**

运行：

```powershell
cargo test -p ipkvm-headless maps_every_printable_ascii_key_for_en_us
```

预期：编译失败，因为 `rfb_input`、`MappedKey` 和 `map_keysym` 尚不存在。

- [x] **步骤 3：增加直接依赖和公共模块**

`Cargo.toml` 增加：

```toml
[dependencies]
ipkvm-core = { path = "../ipkvm-core" }

[dev-dependencies]
ipkvm-core = { path = "../ipkvm-core", features = ["mock"] }
```

测试 feature 只用于最终 6KRO 事务测试；生产依赖不启用 mock。

`src/lib.rs` 增加：

```rust
pub mod rfb_input;
```

`rfb_input/mod.rs`：

```rust
mod keyboard;
mod keymap;

use ipkvm_core::InputError;
use thiserror::Error;

pub use keyboard::RfbKeyboardMapper;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbKeyboardOutcome {
    Applied,
    DuplicateDown,
    UnknownRelease,
    IgnoredLock,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbKeyboardError {
    #[error("unsupported RFB keysym: {0:#010x}")]
    UnsupportedKeysym(u32),
    #[error("active RFB characters require conflicting Shift states")]
    ConflictingShiftRequirements,
    #[error("input sink rejected RFB keyboard state")]
    Input(#[from] InputError),
}
```

`keyboard.rs` 先定义可构造占位类型，仅用于让 keymap 测试编译：

```rust
#[derive(Debug, Default)]
pub struct RfbKeyboardMapper;

impl RfbKeyboardMapper {
    pub fn new() -> Self {
        Self
    }
}
```

- [x] **步骤 4：实现 ASCII 纯映射**

`keymap.rs` 定义：

```rust
use ipkvm_core::KeyboardUsage;

use super::RfbKeyboardError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShiftRequirement {
    Required,
    NotRequired,
}

impl ShiftRequirement {
    #[cfg(test)]
    fn from_required(required: bool) -> Self {
        if required { Self::Required } else { Self::NotRequired }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MappedKey {
    Direct(KeyboardUsage),
    Character {
        usage: KeyboardUsage,
        shift: ShiftRequirement,
    },
    IgnoredLock,
}
```

实现 `map_ascii`：

- 字母用区间算术映射到 `0x04..=0x1d`。
- 数字用显式 `"1234567890"` 位置映射到 `0x1e..=0x27`。
- 标点使用两个 21 项常量表，分别记录字符、usage 和 Shift 需求。
- `map_keysym` 先尝试 `char::from_u32` 且只接受 `0x20..=0x7e`，其他值暂时返回 `UnsupportedKeysym`。

不得根据 Rust 字符大小写临时猜测标点位置。

- [x] **步骤 5：补齐 ASCII 全范围覆盖**

测试遍历 `0x20..=0x7e`，断言每个值都返回 `Character`：

```rust
#[test]
fn every_printable_ascii_value_is_supported() {
    for keysym in 0x20..=0x7e {
        assert!(matches!(
            map_keysym(keysym),
            Ok(MappedKey::Character { .. })
        ), "missing keysym {keysym:#x}");
    }
}
```

- [x] **步骤 6：运行映射测试并确认绿灯**

```powershell
cargo test -p ipkvm-headless rfb_input::keymap
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
```

预期：全部通过。

- [x] **步骤 7：提交**

```powershell
git add crates/ipkvm-headless
git commit -m "feat: add en-US RFB keysym table (#7)"
```

---

### 任务 3：实现状态化 Shift 和事务提交

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_input/keyboard.rs`
- 修改：`crates/ipkvm-headless/src/rfb_input/keymap.rs`

**接口：**

- 消费：`InputSink::handle_key_batch`
- 消费：`map_keysym`
- 完成：`RfbKeyboardMapper::handle_key`

- [x] **步骤 1：写入记录 sink 和 Shift 红灯测试**

在 `keyboard.rs` 测试模块定义：

```rust
#[derive(Default)]
struct RecordingSink {
    batches: Vec<Vec<KeyEvent>>,
    fail_next: bool,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> { Ok(()) }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
        if std::mem::take(&mut self.fail_next) {
            return Err(InputError::RolloverLimitExceeded);
        }
        self.batches.push(events.to_vec());
        Ok(())
    }

    fn handle_pointer(&mut self, _event: PointerEvent) -> InputResult<()> { Ok(()) }
    fn release_all(&mut self) -> InputResult<()> { Ok(()) }
}
```

增加：

```rust
#[test]
fn uppercase_without_remote_shift_is_one_atomic_batch() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    assert_eq!(
        mapper.handle_key(&mut sink, true, 'A' as u32),
        Ok(RfbKeyboardOutcome::Applied)
    );
    assert_eq!(
        sink.batches,
        vec![vec![
            down(0xe1),
            down(0x04),
        ]]
    );

    mapper.handle_key(&mut sink, false, 'A' as u32).unwrap();
    assert_eq!(sink.batches[1], vec![up(0x04), up(0xe1)]);
}

#[test]
fn lowercase_temporarily_suppresses_remote_shift() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    mapper.handle_key(&mut sink, true, 0xffe1).unwrap();
    mapper.handle_key(&mut sink, true, 'a' as u32).unwrap();
    mapper.handle_key(&mut sink, false, 'a' as u32).unwrap();

    assert_eq!(sink.batches[0], vec![down(0xe1)]);
    assert_eq!(sink.batches[1], vec![up(0xe1), down(0x04)]);
    assert_eq!(sink.batches[2], vec![up(0x04), down(0xe1)]);
}
```

- [x] **步骤 2：确认 mapper 红灯**

```powershell
cargo test -p ipkvm-headless rfb_input::keyboard::tests::uppercase_without_remote_shift_is_one_atomic_batch -- --exact
```

预期：编译失败，因为 `handle_key` 尚未定义。

- [x] **步骤 3：建立 mapper 状态和目标集合计算**

`RfbKeyboardMapper`：

```rust
#[derive(Debug, Default)]
pub struct RfbKeyboardMapper {
    active_keys: BTreeMap<u32, MappedKey>,
    committed_usages: BTreeSet<KeyboardUsage>,
}
```

实现私有函数：

```rust
fn target_usages(
    active: &BTreeMap<u32, MappedKey>,
) -> Result<BTreeSet<KeyboardUsage>, RfbKeyboardError>;

fn diff_usages(
    current: &BTreeSet<KeyboardUsage>,
    target: &BTreeSet<KeyboardUsage>,
) -> Vec<KeyEvent>;

fn is_modifier(usage: KeyboardUsage) -> bool {
    (0xe0..=0xe7).contains(&usage.get())
}
```

`target_usages` 必须：

- 收集所有 `Direct` usage。
- 收集所有 `Character` usage。
- 统计字符 Shift 需求。
- 全 Required 且没有直接 Shift 时加入 `0xe1`。
- 全 NotRequired 时移除 `0xe1` 和 `0xe5`。
- 两种需求同时存在时返回 `ConflictingShiftRequirements`。

`diff_usages` 严格按普通 Up、修饰 Up、修饰 Down、普通 Down 排序；同组按 usage 数值升序。

- [x] **步骤 4：实现事务化 handle_key**

```rust
pub fn handle_key(
    &mut self,
    sink: &mut impl InputSink,
    down: bool,
    keysym: u32,
) -> Result<RfbKeyboardOutcome, RfbKeyboardError> {
    if down && self.active_keys.contains_key(&keysym) {
        return Ok(RfbKeyboardOutcome::DuplicateDown);
    }
    if !down && !self.active_keys.contains_key(&keysym) {
        return Ok(RfbKeyboardOutcome::UnknownRelease);
    }

    let mut next_active = self.active_keys.clone();
    if down {
        match map_keysym(keysym)? {
            MappedKey::IgnoredLock => return Ok(RfbKeyboardOutcome::IgnoredLock),
            mapped => {
                next_active.insert(keysym, mapped);
            }
        }
    } else {
        next_active.remove(&keysym);
    }

    let target = target_usages(&next_active)?;
    let events = diff_usages(&self.committed_usages, &target);
    if !events.is_empty() {
        sink.handle_key_batch(&events)?;
    }
    self.active_keys = next_active;
    self.committed_usages = target;
    Ok(RfbKeyboardOutcome::Applied)
}
```

- [x] **步骤 5：写入冲突和 sink 回滚测试**

```rust
#[test]
fn opposite_shift_characters_are_rejected_without_state_commit() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();
    mapper.handle_key(&mut sink, true, 'A' as u32).unwrap();

    assert_eq!(
        mapper.handle_key(&mut sink, true, 'b' as u32),
        Err(RfbKeyboardError::ConflictingShiftRequirements)
    );
    assert_eq!(sink.batches.len(), 1);
    assert_eq!(
        mapper.handle_key(&mut sink, false, 'b' as u32),
        Ok(RfbKeyboardOutcome::UnknownRelease)
    );
    mapper.handle_key(&mut sink, false, 'A' as u32).unwrap();
    assert_eq!(sink.batches[1], vec![up(0x04), up(0xe1)]);
}

#[test]
fn rejected_sink_batch_can_be_retried() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink {
        fail_next: true,
        ..RecordingSink::default()
    };

    assert!(matches!(
        mapper.handle_key(&mut sink, true, 'A' as u32),
        Err(RfbKeyboardError::Input(_))
    ));
    assert_eq!(
        mapper.handle_key(&mut sink, true, 'A' as u32),
        Ok(RfbKeyboardOutcome::Applied)
    );
    assert_eq!(sink.batches, vec![vec![down(0xe1), down(0x04)]]);
}
```

- [x] **步骤 6：运行状态测试并确认绿灯**

```powershell
cargo test -p ipkvm-headless rfb_input::keyboard
```

预期：全部通过。

- [x] **步骤 7：提交**

```powershell
git add crates/ipkvm-headless/src/rfb_input
git commit -m "feat: map RFB character shift state atomically (#7)"
```

---

### 任务 4：补齐特殊键、别名和边界

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_input/keymap.rs`
- 修改：`crates/ipkvm-headless/src/rfb_input/keyboard.rs`
- 修改：`crates/ipkvm-headless/tests/rfb_keyboard.rs`

**接口：**

- 完成：设计中承诺的 X11 特殊键范围
- 保证：别名引用、锁定键忽略、6KRO 回滚和错误类型稳定

- [ ] **步骤 1：建立公共 API 集成测试**

新建 `crates/ipkvm-headless/tests/rfb_keyboard.rs`，实现只依赖公共 API 的 `RecordingSink`，覆盖：

```rust
#[test]
fn public_mapper_handles_modifiers_function_keys_and_keypad() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    for (keysym, usage) in [
        (0xffe1, 0xe1), (0xffe2, 0xe5),
        (0xffe3, 0xe0), (0xffe4, 0xe4),
        (0xffe9, 0xe2), (0xffea, 0xe6),
        (0xffeb, 0xe3), (0xffec, 0xe7),
        (0xffbe, 0x3a), (0xffc9, 0x45),
        (0xff8d, 0x58), (0xffb0, 0x62), (0xffb9, 0x61),
    ] {
        mapper.handle_key(&mut sink, true, keysym).unwrap();
        mapper.handle_key(&mut sink, false, keysym).unwrap();
        assert_eq!(sink.last_usage_pair(), (usage, usage));
    }
}
```

再增加：

- 所有 RFC 常用特殊键表驱动测试。
- KP 数字与主键区数字 usage 不同。
- `ISO_Left_Tab` 产生 `0xe1 + 0x2b`。
- CapsLock/NumLock down 和 up 返回 `IgnoredLock` 或 `UnknownRelease`，且不调用 sink。
- F13、Unicode 和未知 down 返回 `UnsupportedKeysym`。

- [ ] **步骤 2：确认特殊键红灯**

```powershell
cargo test -p ipkvm-headless --test rfb_keyboard
```

预期：测试失败，指出特殊 keysym 仍为 `UnsupportedKeysym`。

- [ ] **步骤 3：实现特殊键映射表**

在 `map_keysym` 的 ASCII 分支之后增加显式 match，至少包含：

```rust
0xff08 => direct(0x2a), // BackSpace
0xff09 => direct(0x2b), // Tab
0xff0d => direct(0x28), // Return
0xff13 => direct(0x48), // Pause
0xff14 => direct(0x47), // ScrollLock
0xff15 => direct(0x46), // SysReq
0xff1b => direct(0x29), // Escape
0xff50 => direct(0x4a), // Home
0xff51 => direct(0x50), // Left
0xff52 => direct(0x52), // Up
0xff53 => direct(0x4f), // Right
0xff54 => direct(0x51), // Down
0xff55 => direct(0x4b), // PageUp
0xff56 => direct(0x4e), // PageDown
0xff57 => direct(0x4d), // End
0xff61 => direct(0x46), // Print
0xff63 => direct(0x49), // Insert
0xff67 => direct(0x65), // Menu
0xffff => direct(0x4c), // Delete
0xffe1 => direct(0xe1),
0xffe2 => direct(0xe5),
0xffe3 => direct(0xe0),
0xffe4 => direct(0xe4),
0xffe7 | 0xffeb => direct(0xe3),
0xffe8 | 0xffec => direct(0xe7),
0xffe9 => direct(0xe2),
0xffea => direct(0xe6),
0xffe5 | 0xff7f => Ok(MappedKey::IgnoredLock),
```

使用区间算术实现 F1-F12 和 KP 0-9；其他 KP 键显式 match。`0xfe20` 映射为需要 Shift 的 Tab。

- [ ] **步骤 4：增加状态边界红灯测试**

在 `keyboard.rs` 增加：

- Meta_L 和 Super_L 同时 down 只产生一个 GUI down，释放第一个不产生 sink 批次，释放第二个产生 GUI up。
- duplicate down 返回 `DuplicateDown`。
- unknown up 返回 `UnknownRelease`。
- 两个同为 Required 的字符共享合成 Shift，释放一个时不释放 Shift。
- 七个普通键中的第七个被真实 `Ch9329InputSink<FakeCommandQueue>` 拒绝，释放第七个返回 `UnknownRelease`，前六个仍可逐个释放。

- [ ] **步骤 5：实现别名和边界所需修正**

只修正红灯暴露出的状态计算问题：

- 目标集合使用 `BTreeSet` 去重别名。
- 空差异不调用 sink，但仍提交 `active_keys`。
- sink 错误前不替换两个 mapper 字段。
- 未知 up 在调用 `map_keysym` 前返回。

不得为测试增加生产环境状态读取方法。

- [ ] **步骤 6：运行 headless 全量测试**

```powershell
cargo test -p ipkvm-headless
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
```

预期：全部通过。

- [ ] **步骤 7：提交**

```powershell
git add crates/ipkvm-headless
git commit -m "test: cover RFB keyboard mappings and rollback (#7)"
```

---

### 任务 5：回写中文文档并完成验收

**文件：**

- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-keyboard-mapping-design.md`
- 修改：`docs/superpowers/plans/2026-07-31-rfb-keyboard-mapping.md`

**接口：**

- 消费：前四项全部实现
- 产出：准确阶段状态和完整本地验证证据

- [ ] **步骤 1：核对测试清单**

```powershell
cargo test -p ipkvm-core -- --list
cargo test -p ipkvm-headless -- --list
```

必须能定位：

- 原子键盘批次和队列失败回滚。
- 全部可打印 ASCII。
- Shift 合成、抑制、恢复和冲突。
- 左右修饰键、特殊键、F1-F12、KP 键。
- duplicate down、unknown up、锁定键和 unsupported keysym。
- alias 引用和真实 6KRO sink 拒绝后的 mapper 回滚。

- [ ] **步骤 2：更新 README**

当前状态改为包含：

```markdown
当前工程已完成 CH9329 协议与输入核心、传输无关的 RFB 3.8 协议核心、单客户端 RFB TCP 库闭环，以及 en-US RFB keysym 到 HID 的状态化键盘映射。RFB 事件泵、断线 release_all、指针映射、真实串口和真实视频采集仍待实现。
```

明确 mapper 尚未接到可运行无头进程。

- [ ] **步骤 3：更新阶段计划**

在 `docs/ipkvm-coarse-design.md` 的阶段 0：

- 把 RFB en-US 键盘映射和原子键盘批次加入已完成。
- 从待完成项移除“写 HID 用法编号到桌面和 RFB 键值的映射基础表”。
- 增加待完成的 RFB PointerEvent 状态映射和控制者生命周期。

- [ ] **步骤 4：回写设计与计划状态**

- 设计状态改为“已实施”。
- 所有实际完成步骤改为 `[x]`。
- 删除与最终公共名称不一致的草案名称。
- 扫描 `TBD`、`TODO`、临时说明和中英文混杂段落。

- [ ] **步骤 5：运行完整本地验证**

```powershell
.\scripts\verify.ps1
```

预期：

- UTF-8 无 BOM 检查通过。
- Rust 格式通过。
- 全工作区全 feature 测试通过。
- Clippy `-D warnings` 通过。
- Rust 文档 `-D warnings` 通过。
- 工作区和暂存区 `git diff --check` 通过。

- [ ] **步骤 6：提交**

```powershell
git add README.md docs
git commit -m "docs: record RFB keyboard mapping completion (#7)"
```

- [ ] **步骤 7：最终自审**

```powershell
git status --short
git diff --check main...HEAD
git diff --stat main...HEAD
git log --oneline main..HEAD
```

确认：

- 没有修改用户未提交的 `AGENTS.md`。
- 没有新增外部依赖。
- 没有在 core 引入 RFB 概念。
- 没有未受控状态增长或无界事件队列。
- 没有把 mapper 描述成已接通真实串口。
- issue、PR 和本地提交使用同一个 `#7` 关联。
