# RFB 键盘映射与原子键盘批次设计

## 1. 文档状态

- 关联 issue：`#7`
- 状态：已实施并通过本地自动化验证
- 适用范围：en-US 目标键盘上的 RFB `KeyEvent`
- 前置依赖：`#5` 已完成的 `RfbTcpEvent::Key` 和 `ipkvm-core::InputSink`

## 2. 目标

本阶段把 RFB 的 X11 keysym 转换为现有 CH9329 输入核心能够消费的 USB HID 键盘事件，并固定失败原子性：

1. 支持 en-US 键盘上的可打印 ASCII、左右修饰键、导航键、F1-F20、系统键和常用数字小键盘键。
2. 遵守 RFC 6143 的字符语义：大小写和符号由 keysym 决定，客户端当前 Shift 状态只作为提示。
3. 必要时合成 Shift；客户端 Shift 与字符语义冲突时，暂时抑制 Shift。
4. 一次 RFB 按键事件产生的多个 HID 状态变更通过一个原子批次提交。
5. sink 拒绝批次时，CH9329 键盘状态和 RFB 映射器状态都保持原值。
6. 重复按下、未知释放、不可映射键和无法同时表示的字符组合具有确定结果。

本阶段只建立键盘适配。RFB 指针、滚轮、剪贴板、事件接收循环、控制者生命周期和断线 `release_all` 分到后续 issue。

## 3. 调研结论

### 3.1 RFC 6143

RFC 6143 第 7.5.4 节规定：

- RFB `KeyEvent` 使用 X11 keysym，不是平台虚拟键码或 USB HID usage。
- 大写和小写 keysym 不等价。收到大写 `A` 时，即使客户端没有发送 Shift，也应产生大写 `A`。
- Shift 状态只是解释字符的提示。目标布局需要 Shift 而客户端未按 Shift 时，server 应内部合成 Shift。
- Control 和 Alt 是组合键语义的一部分，不应像 Shift 一样根据字符自动消除。
- CapsLock 和 NumLock 等锁定键应尽可能忽略，字符应按自身 keysym 的大小写解释。
- `ISO_Left_Tab` 应兼容为 Shift+Tab。

因此，简单的 `keysym -> KeyboardUsage` 无状态查表不符合协议要求。

### 3.2 USB HID

CH9329 发送的是 USB HID boot keyboard 报告：

- `0xe0..=0xe7` 是左右 Control、Shift、Alt 和 GUI 修饰键。
- 普通键最多同时保存六个 usage。
- 修饰键状态对报告中的所有普通键同时生效。

这意味着同一份 HID 报告不能同时表达“需要 Shift 的字符”和“不需要 Shift 的字符”。如果两个这类字符同时保持按下，必须返回确定错误，不能任意选择一种 Shift 状态。

### 3.3 kvm-serial

`kvm-serial` 提供了可参考的 HID usage 表和 en-US/en-GB 字符表，但其 Qt 路径直接从 `QKeyEvent` 构造报告：

- 普通键释放会发送只保留修饰键的报告。
- 字符 Shift 由 Qt 文本和布局表决定。
- 状态模型不处理 RFB keysym、多个普通键同时保持和 sink 失败回滚。

本项目只参考其经过实际使用的 usage 取值，不复用 Qt/Python 状态实现。

## 4. 方案比较

### 4.1 方案 A：无状态查表

每个 keysym 直接映射为一个 `KeyboardUsage`，原样转发客户端 Shift。

优点：

- 实现最小。
- 特殊键和同布局客户端通常可用。

缺点：

- 大写 `A` 未显式携带 Shift 时会输入小写。
- 不同客户端和目标键盘布局的符号会错误。
- 明确违反 RFC 6143 的互操作建议。

不采用。

### 4.2 方案 B：状态化映射器和原子键盘批次

维护活动 keysym 集合，根据所有活动键重新计算目标 HID 状态，生成旧状态到新状态的差异批次。字符 keysym 带有明确的 Shift 需求；批次成功后才提交映射器状态。

优点：

- 满足 RFC 的字符大小写和符号语义。
- 能正确处理重复事件、别名和失败回滚。
- 纯 Rust、无新增外部依赖。
- 映射和 CH9329 协议边界清晰。

缺点：

- 需要扩展 `InputSink` 的原子批次契约。
- 必须定义 HID 无法表达的冲突组合。

采用此方案。

### 4.3 方案 C：引入 xkbcommon 或平台键盘 API

使用完整键盘布局引擎把 Unicode/keysym 转为物理键序列。

优点：

- 可扩展到多布局、死键和组合字符。

缺点：

- 增加本地动态库、平台差异、打包和许可证审计。
- Windows、Linux、macOS 的行为边界不同。
- 当前只承诺 en-US，复杂度与阶段目标不匹配。

本阶段不采用。以后增加多布局时重新评估，不在现有映射器中堆叠平台特例。

## 5. 架构边界

### 5.1 `ipkvm-core`

只增加通用键盘批次能力：

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

批次语义：

- `events` 描述从当前键盘状态到一个新状态的原子变更。
- 中间状态不向目标机发送；成功时只发送最终 HID 报告。
- 空批次或最终状态与当前状态相同返回成功且不入队。
- 任一事件非法、超过 6KRO 或命令队列拒绝时，状态和队列都不部分提交。
- `handle_key` 保持现有单事件调用方式，并委托给长度为 1 的批次。

`ipkvm-core` 不出现 keysym、RFB 或键盘布局概念。

### 5.2 `ipkvm-headless::rfb_input`

新增：

```rust
pub struct RfbKeyboardMapper {
    // 私有活动 keysym 和已提交 HID 状态
}

impl RfbKeyboardMapper {
    pub fn new() -> Self;

    pub fn handle_key(
        &mut self,
        sink: &mut impl InputSink,
        down: bool,
        keysym: u32,
    ) -> Result<RfbKeyboardOutcome, RfbKeyboardError>;
}
```

映射器不持有 sink：

- 后续控制者生命周期可以拥有一个 sink，并把同一个 mapper 绑定到当前控制者。
- 测试可以注入记录批次或拒绝批次的 sink。
- sink 调用成功前不修改映射器状态。

## 6. 映射模型

### 6.1 内部键分类

每个支持的 keysym 映射为以下内部类型之一：

```rust
enum MappedKey {
    Direct(KeyboardUsage),
    Character {
        usage: KeyboardUsage,
        shift: ShiftRequirement,
    },
    IgnoredLock,
}
```

- `Direct`：修饰键、导航键、功能键、系统键和数字小键盘键。
- `Character`：en-US 可打印 ASCII 和 `ISO_Left_Tab`，包含目标字符是否需要 Shift。
- `IgnoredLock`：CapsLock 和 NumLock。

映射常量使用 X11 `keysymdef.h` 和 USB HID Usage Tables 的数值。项目不复制第三方实现代码。

### 6.2 可打印 ASCII

支持 `0x20..=0x7e`：

- `a..z` -> `0x04..=0x1d`，不需要 Shift。
- `A..Z` -> 同一 usage，需要 Shift。
- `1..0` -> `0x1e..=0x27`，不需要 Shift。
- `!@#$%^&*()` -> 对应数字键，需要 Shift。
- 空格和 en-US 标点按 HID usage `0x2c..=0x38` 映射。

Latin-1 扩展字符、Unicode keysym、死键和组合键不在本阶段支持范围内。收到这些键的 down 事件返回 `UnsupportedKeysym`。

### 6.3 修饰键

映射：

| X11 keysym | HID usage |
| --- | --- |
| Shift_L / Shift_R | `0xe1` / `0xe5` |
| Control_L / Control_R | `0xe0` / `0xe4` |
| Alt_L / Alt_R | `0xe2` / `0xe6` |
| Meta_L / Meta_R | `0xe3` / `0xe7` |
| Super_L / Super_R | `0xe3` / `0xe7` |

Meta 和 Super 是同一 HID GUI usage 的别名。活动键按 keysym 分别记录，目标 usage 按引用语义合并；释放其中一个别名不会错误释放仍由另一个别名保持的 GUI 键。

Control 和 Alt 永远按客户端状态转发，不参与字符 Shift 修正。

### 6.4 特殊键

第一版支持：

- BackSpace、Tab、Return、Escape、Insert、Delete。
- Home、End、PageUp、PageDown、四个方向键。
- F1-F20，其中 F1-F12 使用 HID usage `0x3a..0x45`，F13-F20 使用 `0x68..0x6f`。
- Print/SysReq、ScrollLock、Pause、Menu。
- `ISO_Left_Tab` 作为需要 Shift 的 Tab。
- KP Enter、KP Divide、Multiply、Subtract、Add、Decimal、Equal 和 KP 0-9。
- KP Home/End/PageUp/PageDown/Insert/Delete/方向键映射为不依赖 NumLock 的普通导航 usage。

F21 及以上、媒体键、XF86 键、Compose、ModeSwitch 和语言输入键返回 `UnsupportedKeysym`。

CapsLock 和 NumLock 的 down/up 都返回 `IgnoredLock`，不改变 mapper 或 sink。字符正确性仍以 keysym 为准。

## 7. 状态和 Shift 算法

### 7.1 活动状态

映射器维护：

- `active_keys: BTreeMap<u32, MappedKey>`：当前活动的远端 keysym。
- `committed_usages: BTreeSet<KeyboardUsage>`：sink 已接受的目标 HID 状态。

集合大小由支持的映射表上限约束，不随任意未知 keysym 增长。

### 7.2 down 和 up

down：

1. 如果 keysym 已活动，返回 `DuplicateDown`，不调用 sink。
2. 解析 keysym；锁定键返回 `IgnoredLock`。
3. 在活动状态副本中加入该键。
4. 计算目标 HID 状态。
5. 生成差异批次；非空时调用 sink。
6. 差异为空或 sink 成功后，提交活动 keysym 状态并返回 `Applied`。

up：

1. 如果 keysym 不在活动状态，返回 `UnknownRelease`，不解析、不调用 sink。
2. 在状态副本中移除该键。
3. 计算差异；非空时提交 sink，随后提交 mapper 状态。

这样，不可映射键的孤立 up 不会使连接失败，也不会释放其他键。

### 7.3 目标 Shift

根据所有活动 `Character`：

- 没有字符：按客户端直接 Shift 键状态发送。
- 全部字符需要 Shift：
  - 客户端有直接 Shift 时使用其左右 Shift。
  - 客户端没有 Shift 时合成 LeftShift。
- 全部字符不需要 Shift：暂时从目标状态移除客户端直接 Shift。
- 同时存在需要和不需要 Shift 的字符：返回 `ConflictingShiftRequirements`。

冲突是 USB boot keyboard 的表达能力限制。失败时，新事件不加入活动状态，sink 不被调用。

当最后一个字符释放后，之前暂时抑制的客户端 Shift 会自动恢复；当最后一个需要合成 Shift 的字符释放后，合成 LeftShift 自动移除。

### 7.4 差异批次

从 `committed_usages` 到目标集合生成事件：

1. 普通键 Up。
2. 修饰键 Up。
3. 修饰键 Down。
4. 普通键 Down。

`Ch9329InputSink::handle_key_batch` 只发送最终状态报告，因此中间顺序不会暴露给目标机；固定顺序用于记录型测试和其他 sink 实现的一致性。

如果多个活动 keysyms 映射为同一 usage，usage 只在最后一个别名释放时产生 Up。

## 8. 错误与结果

```rust
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

`Applied` 包括成功提交一个非空批次，也包括别名变化但最终 HID 集合不变的成功状态提交。后者不调用 sink，但仍需记录活动 keysym，保证后续释放正确。

## 9. 失败原子性

### 9.1 core

`Ch9329InputSink` 在局部 `KeyboardState` 副本上应用整个批次：

1. 任一事件失败时丢弃副本。
2. 最终状态未变化时直接返回。
3. 最终报告编码成功后构造一个命令批次。
4. 命令队列接受后才替换内部 keyboard 状态。

### 9.2 mapper

映射器同样在活动状态副本上计算：

1. 映射或 Shift 冲突失败时不调用 sink。
2. sink 失败时不替换活动状态和 `committed_usages`。
3. 调用方可以重试同一个 RFB 事件；它仍会得到同一个待提交批次。

后续事件泵遇到错误时是否断开客户端、记录状态或调用 `release_all`，由控制者生命周期 issue 决定，本阶段不吞掉错误。

## 10. 测试设计

### 10.1 core 单元测试

- 单事件 `handle_key` 继续工作。
- 多事件批次只入队一个最终键盘报告。
- 空批次和净状态不变不入队。
- 批次内部超过 6KRO 时不入队且不提交状态。
- 命令队列拒绝时不提交状态，重试产生相同报告。
- 修饰键和普通键可以在一个批次中原子改变。

### 10.2 ASCII 映射测试

- 表驱动覆盖 `0x20..=0x7e` 每个字符的 usage 和 Shift 需求。
- 大写 `A` 在无客户端 Shift 时合成 LeftShift。
- 小写 `a` 在客户端 Shift 保持时暂时抑制 Shift，释放后恢复。
- en-US 的 `@`、`#`、`_`、`+`、`{`、`|` 等符号使用正确键和 Shift。

### 10.3 特殊键测试

- 左右八个修饰键逐项映射。
- 导航键、F1-F20 和系统键逐项映射。
- 主键区数字和 KP 数字使用不同 usage。
- `ISO_Left_Tab` 产生 Shift+Tab。
- CapsLock 和 NumLock 被忽略。
- F21、Unicode keysym 和未知 keysym 被拒绝。

### 10.4 状态测试

- duplicate down 不产生第二个 sink 调用。
- unknown up 不释放任何 usage。
- Meta/Super 别名引用正确。
- 同 Shift 需求的多个字符可同时保持。
- 相反 Shift 需求同时保持时拒绝新事件，原状态可继续正常释放。
- sink 失败后重试同一事件成功，证明 mapper 未提前提交。
- 第七个普通键由 sink 拒绝后，mapper 状态不包含该键。

### 10.5 命令级验证

```powershell
.\scripts\verify.ps1
```

本阶段没有必须人工参与的测试。真实 VNC/noVNC 键盘兼容性留到 WebSocket/noVNC 集成阶段，不能替代字节级和状态级自动化测试。

## 11. 文档和阶段状态

实施完成后：

- `README.md` 增加“已有 RFB en-US 键盘映射核心”，但不得声称事件泵或真实串口已接通。
- `docs/ipkvm-coarse-design.md` 将阶段 0 的 RFB 键盘映射标为完成。
- issue `#7` 记录设计、红灯测试、提交和本地统一验证。

## 12. 后续任务

按以下顺序继续拆分：

1. RFB PointerEvent 到绝对移动、按钮差异和滚轮脉冲的状态化映射。
2. RFB 事件泵、单控制者生命周期和断开 `release_all`。
3. 剪贴板到独立文本键入服务；不恢复含义不清的 `InputSink::type_text`。

## 13. 资料链接

- RFC 6143 第 7.5.4 节：https://www.rfc-editor.org/rfc/rfc6143#section-7.5.4
- X.Org `keysymdef.h`：https://gitlab.freedesktop.org/xorg/proto/xorgproto/-/blob/master/include/X11/keysymdef.h
- USB HID Usage Tables 1.7：https://usb.org/document-library/hid-usage-tables-17
- kvm-serial：https://github.com/sjmf/kvm-serial
