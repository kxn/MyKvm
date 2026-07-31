# RFB 指针映射实施计划

> 关联 issue：`#9`
>
> 设计依据：`docs/superpowers/specs/2026-07-31-rfb-pointer-mapping-design.md`
>
> 执行约束：按任务顺序推进；每个行为先写测试并确认红灯，再写最小完整实现；所有测试在本机执行。

## 1. 实施目标

完成以下闭环：

```text
RFB PointerEvent
    -> RfbPointerMapper
    -> 一批 PointerEvent
    -> InputSink::handle_pointer_batch
    -> Ch9329InputSink
    -> 一个 CommandBatch
```

关键保证：

- RFB 掩码按完整当前状态解释。
- 按钮 1、2、3 做状态差分。
- 按钮 4、5 只在上升沿产生垂直滚轮步骤。
- 位 5、6、7 被忽略但结果可观测。
- 一条 RFB 指针消息只调用一次 sink 批次接口。
- core 或 mapper 任一层失败都不提交部分状态。
- 越界坐标拒绝，不裁剪。
- 不新增外部依赖。

## 2. 预期文件

```text
crates/ipkvm-core/src/input.rs
crates/ipkvm-core/src/lib.rs
crates/ipkvm-core/src/ch9329/input.rs
crates/ipkvm-headless/src/rfb_input/mod.rs
crates/ipkvm-headless/src/rfb_input/keyboard.rs
crates/ipkvm-headless/src/rfb_input/pointer.rs
crates/ipkvm-headless/tests/rfb_keyboard.rs
crates/ipkvm-headless/tests/rfb_pointer.rs
README.md
docs/ipkvm-coarse-design.md
docs/superpowers/specs/2026-07-31-rfb-pointer-mapping-design.md
docs/superpowers/plans/2026-07-31-rfb-pointer-mapping.md
```

---

## 任务 1：给 InputSink 增加原子指针批次

**文件：**

- 修改：`crates/ipkvm-core/src/input.rs`
- 修改：`crates/ipkvm-core/src/lib.rs`
- 修改：`crates/ipkvm-core/src/ch9329/input.rs`
- 修改：现有测试 sink 实现

### 接口

新增：

```rust
fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()>;
```

单事件方法默认委托：

```rust
fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
    self.handle_pointer_batch(std::slice::from_ref(&event))
}
```

### 步骤

- [x] **步骤 1：写入 CH9329 指针批次红灯测试**

在 `crates/ipkvm-core/src/ch9329/input.rs` 增加：

```rust
#[test]
fn pointer_batch_enqueues_all_reports_atomically() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let size = FramebufferSize {
        width: 100,
        height: 100,
    };

    sink.handle_pointer_batch(&[
        PointerEvent::AbsoluteMove {
            x: 10,
            y: 20,
            framebuffer_size: size,
        },
        PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        },
        PointerEvent::Wheel { delta: 1 },
    ])
    .unwrap();

    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].frames().len(), 3);
    assert_eq!(batches[0].frames()[0].data()[1], 0);
    assert_eq!(batches[0].frames()[1].data()[1], 1);
    assert_eq!(batches[0].frames()[2].data()[1], 1);
    assert_eq!(batches[0].frames()[2].data()[6], 1);
}
```

再增加：

- 批次后半段坐标非法时，前面的有效移动不入队。
- 队列拒绝后，同一批次可重试且状态不漂移。
- 按下再释放必须保留两个报告，不能按最终净状态折叠。
- 空批次和全无变化批次不入队。

- [x] **步骤 2：确认红灯**

```powershell
cargo test -p ipkvm-core pointer_batch
```

预期：编译失败，指出 `handle_pointer_batch` 尚不存在。

- [x] **步骤 3：扩展 InputSink 契约**

修改 trait，并更新所有测试 sink：

- `crates/ipkvm-core/src/lib.rs`
- `crates/ipkvm-headless/src/rfb_input/keyboard.rs`
- `crates/ipkvm-headless/tests/rfb_keyboard.rs`

记录型 sink 直接保存整个事件切片；不得用循环调用单事件方法冒充原子批次。

- [x] **步骤 4：把 MouseState 改成候选状态计算**

在 `MouseState` 上实现：

```rust
fn apply_events(
    &self,
    events: &[PointerEvent],
) -> InputResult<(MouseState, Vec<Ch9329Command>)>;
```

内部按顺序处理：

- `AbsoluteMove`：验证模式和坐标，生成带当前按钮的绝对报告，更新候选绝对位置。
- `RelativeMove`：验证模式，沿用现有拆包。
- `Button`：重复状态不生成命令；变化时先更新候选按钮，再生成报告。
- `Wheel`：零值不生成命令；其他值沿用现有拆包，并携带候选按钮和位置。

任一后续事件失败时直接返回错误，尚未发生队列操作。

- [x] **步骤 5：实现 CH9329 原子提交**

```rust
pub fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
    self.handle_pointer_batch(std::slice::from_ref(&event))
}

pub fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
    let (next, commands) = self.mouse.apply_events(events)?;
    if commands.is_empty() {
        return Ok(());
    }
    self.enqueue_commands(commands)?;
    self.mouse = next;
    Ok(())
}
```

更新 `impl InputSink for Ch9329InputSink<Q>`。

- [x] **步骤 6：运行 core 验证**

```powershell
cargo test -p ipkvm-core
cargo clippy -p ipkvm-core --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

- [x] **步骤 7：提交**

```powershell
git add crates/ipkvm-core crates/ipkvm-headless
git commit -m "feat: add atomic pointer input batches (#9)"
```

---

## 任务 2：实现状态化 RFB 指针映射器

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_input/mod.rs`
- 新建：`crates/ipkvm-headless/src/rfb_input/pointer.rs`

### 公共接口

```rust
pub use pointer::RfbPointerMapper;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbPointerOutcome {
    Applied,
    AppliedIgnoringButtons { button_mask: u8 },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbPointerError {
    #[error("input sink rejected RFB pointer state")]
    Input(#[from] InputError),
}
```

### 步骤

- [x] **步骤 1：写入映射器红灯测试**

在 `pointer.rs` 使用只记录批次的 `RecordingSink`，增加：

```rust
#[test]
fn first_left_press_moves_before_pressing_button() {
    let mut mapper = RfbPointerMapper::new();
    let mut sink = RecordingSink::default();
    let size = FramebufferSize {
        width: 1920,
        height: 1080,
    };

    assert_eq!(
        mapper.handle_pointer(&mut sink, 0x01, 100, 200, size),
        Ok(RfbPointerOutcome::Applied)
    );
    assert_eq!(
        sink.batches,
        vec![vec![
            absolute_move(100, 200, size),
            button(PointerButton::Left, true),
        ]]
    );
}
```

再增加：

- 左键切换为右键时先释放左键再按下右键。
- 同一按钮状态的新坐标只有移动事件。
- 左、中、右同时变化时顺序稳定。

- [x] **步骤 2：确认映射器红灯**

```powershell
cargo test -p ipkvm-headless rfb_input::pointer
```

预期：模块或 `RfbPointerMapper::handle_pointer` 不存在。

- [x] **步骤 3：实现按钮差分**

`RfbPointerMapper` 只保存：

```rust
committed_button_mask: u8
```

常量：

```rust
const PERSISTENT_BUTTON_MASK: u8 = 0b0000_0111;
const WHEEL_UP_MASK: u8 = 1 << 3;
const WHEEL_DOWN_MASK: u8 = 1 << 4;
const UNSUPPORTED_BUTTON_MASK: u8 = 0b1110_0000;
```

构造顺序：

1. `AbsoluteMove`
2. 左、中、右释放
3. 左、中、右按下
4. 滚轮向上上升沿
5. 滚轮向下上升沿

- [x] **步骤 4：写入滚轮边沿红灯测试**

覆盖：

- `0x08 -> 0x00` 只产生一次 `Wheel { delta: 1 }`。
- `0x10 -> 0x00` 只产生一次 `Wheel { delta: -1 }`。
- 重复 `0x08 -> 0x08` 不重复滚动。
- `0x18` 同时产生 `+1` 和 `-1` 两个离散事件。
- 左键保持按下时滚轮事件不产生多余按钮变化。

- [x] **步骤 5：实现滚轮、忽略位和 outcome**

使用：

```rust
let pressed_edges = button_mask & !self.committed_button_mask;
```

位 5、6、7 不生成 core 事件。若 `button_mask & UNSUPPORTED_BUTTON_MASK != 0`，成功结果为：

```rust
RfbPointerOutcome::AppliedIgnoringButtons {
    button_mask: ignored,
}
```

- [x] **步骤 6：写入 sink 失败回滚测试**

记录 sink 第一次返回错误：

1. 首次滚轮上升沿失败。
2. 重试相同掩码仍包含同一个滚轮事件。
3. 成功后重复相同掩码不再滚动。

这验证 mapper 只在 sink 成功后提交掩码。

- [x] **步骤 7：运行 headless 单元验证并提交**

```powershell
cargo test -p ipkvm-headless rfb_input::pointer
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git add crates/ipkvm-headless/src/rfb_input
git commit -m "feat: map RFB pointer state atomically (#9)"
```

---

## 任务 3：增加公共 API 和真实 CH9329 集成边界

**文件：**

- 新建：`crates/ipkvm-headless/tests/rfb_pointer.rs`
- 必要时修改：`crates/ipkvm-headless/src/rfb_input/pointer.rs`

### 步骤

- [ ] **步骤 1：建立公共 API 集成测试**

只使用：

- `ipkvm_headless::rfb_input::{RfbPointerMapper, ...}`
- `ipkvm_core` 的公共输入类型

覆盖：

- 结果类型和错误类型可以从 crate 外使用。
- 水平滚轮及侧键返回准确的忽略掩码。
- 帧尺寸为零和坐标越界时返回 `RfbPointerError::Input`。

- [ ] **步骤 2：使用真实 CH9329 sink 验证四角**

对 `1920x1080`：

- `(0, 0)` 映射到 `(0, 0)`。
- `(1919, 1079)` 映射到 `(4095, 4095)`。
- 每条 RFB 消息只产生一个 `CommandBatch`。

检查 CH9329 绝对报告字节中的小端坐标。

- [ ] **步骤 3：验证首次按钮消息和批次边界**

第一条消息直接为左键按下：

- 同一批次包含移动报告和按钮报告。
- 第二个报告携带按钮位 `0x01`。
- 两个报告使用相同坐标。

- [ ] **步骤 4：验证队列失败的双层回滚**

使用 `FakeCommandQueue::fail_next(CommandQueueError::Closed)`：

1. 左键加滚轮消息失败。
2. 队列没有接受任何批次。
3. 重试同一消息时仍生成移动、左键按下和滚轮。
4. 成功后再发相同掩码，不重复按钮和滚轮，只发送移动。

- [ ] **步骤 5：验证越界后恢复**

先发送 `x == width`：

- 返回 `PointerOutOfBounds`。
- 队列为空。
- mapper 不提交按钮掩码。

随后发送有效的同一按钮掩码：

- 按钮按下仍会产生。
- 队列和 sink 状态正常。

- [ ] **步骤 6：运行 headless 全量验证并提交**

```powershell
cargo test -p ipkvm-headless
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git add crates/ipkvm-headless
git commit -m "test: cover RFB pointer mapping boundaries (#9)"
```

---

## 任务 4：回写中文文档并完成验收

**文件：**

- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-pointer-mapping-design.md`
- 修改：`docs/superpowers/plans/2026-07-31-rfb-pointer-mapping.md`

### 步骤

- [ ] **步骤 1：核对测试清单**

```powershell
cargo test -p ipkvm-core -- --list
cargo test -p ipkvm-headless -- --list
```

必须能定位：

- 指针批次后半段失败和队列拒绝回滚。
- 按钮完整状态差分和确定顺序。
- 滚轮上升沿及释放。
- 未支持按钮 outcome。
- 四角、零尺寸、越界和恢复。
- 真实 CH9329 sink 的双层回滚。

- [ ] **步骤 2：更新 README**

当前状态增加：

- 原子指针批次。
- RFB 绝对指针状态映射。
- 映射器尚未接到运行时事件泵。

- [ ] **步骤 3：更新粗粒度设计**

阶段 0：

- 把 RFB `PointerEvent` 映射加入已完成。
- 从待完成移除指针映射。
- 保留事件泵、控制者生命周期和断线 `release_all()`。

输入映射章节写明：

- 按钮 1 到 5 的当前支持范围。
- 水平滚轮和侧键当前忽略但可观测。
- 越界坐标拒绝策略。

- [ ] **步骤 4：回写专项设计状态**

- 设计状态改为“已实施并通过本地自动化验证”。
- 所有实际步骤改为 `[x]`。
- 扫描 `TBD`、`TODO`、临时说明和中英文混杂段落。

- [ ] **步骤 5：运行完整本地验证**

```powershell
.\scripts\verify.ps1
```

必须通过：

- UTF-8 无 BOM 检查。
- Rust 格式。
- 全工作区全 feature 测试。
- Clippy `-D warnings`。
- Rust 文档 `-D warnings`。
- 工作区和暂存区 `git diff --check`。

- [ ] **步骤 6：提交**

```powershell
git add README.md docs
git commit -m "docs: record RFB pointer mapping completion (#9)"
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
- core 没有 RFB 类型或命名。
- mapper 状态固定为一个 `u8`。
- 没有用最终净状态折叠点击或滚轮。
- 每条 RFB 消息只调用一次 sink 批次接口。
- 所有失败路径均有自动化测试。
