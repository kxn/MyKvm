# RFB 指针状态映射与原子指针批次设计

## 1. 文档状态

- 关联 issue：`#9`
- 状态：已完成调研、设计和自审，等待实施
- 适用范围：经典 RFB `PointerEvent` 到绝对 CH9329 鼠标输入
- 前置依赖：`#5` 已完成的 `RfbTcpEvent::Pointer`，以及 `#7` 扩展后的 `InputSink`

## 2. 目标

本阶段在 RFB TCP 事件与 `ipkvm-core` 指针事件之间建立可复用的状态化适配层：

1. 把 RFB 帧缓冲像素坐标交给 core 的绝对坐标路径。
2. 把按钮 1、2、3 映射为左键、中键、右键，并根据完整按钮掩码计算状态差分。
3. 把按钮 4、5 的上升沿映射为垂直滚轮向上、向下一步。
4. 明确处理水平滚轮和侧键等当前硬件抽象不能表示的按钮。
5. 一次 RFB 指针消息产生的移动、按钮和滚轮变化只通过一个原子批次提交。
6. 任意校验、报告构造或队列错误都不提交 CH9329 状态和 RFB 映射器状态。
7. 使用纯内存 sink 和真实 `Ch9329InputSink<FakeCommandQueue>` 自动验证行为。

## 3. 不在本阶段

- RFB TCP 事件泵。
- 多客户端控制权分配和单控制者生命周期。
- 断线、控制权切换或进程退出时的 `release_all()`。
- 相对鼠标、QEMU Pointer Motion Change 和浏览器 Pointer Lock。
- ExtendedMouseButtons 扩展。
- 水平滚轮和鼠标前进、后退侧键的 CH9329 注入。
- 鼠标移动限速；客户端和后续事件泵分别负责前端采样及队列反压。
- 安全、鉴权和审计。

## 4. 调研结论

### 4.1 RFC 6143

RFC 6143 第 7.5.5 节规定：

- `PointerEvent` 同时表示移动、按钮按下和按钮释放。
- `x-position`、`y-position` 是指针当前所在的帧缓冲坐标。
- `button-mask` 的 0 到 7 位分别表示按钮 1 到 8 的当前完整状态，而不是本次变化。
- 按钮 1、2、3 分别是左键、中键、右键。
- 滚轮向上一步由按钮 4 的按下和释放表示，向下一步由按钮 5 的按下和释放表示。

因此服务端必须保存上一次成功提交的按钮掩码，不能把每条消息的置位位直接当成新的按下事件。

### 4.2 RFB 社区规格

社区规格补充了经典掩码中各位的通行语义：

| 位 | 按钮 | 本阶段处理 |
|---|---|---|
| 0 | 左键 | 支持 |
| 1 | 中键 | 支持 |
| 2 | 右键 | 支持 |
| 3 | 垂直滚轮向上 | 支持上升沿 |
| 4 | 垂直滚轮向下 | 支持上升沿 |
| 5 | 水平滚轮向左 | 忽略并报告 |
| 6 | 水平滚轮向右 | 忽略并报告 |
| 7 | 后退侧键；扩展协商后也可作标记位 | 未协商扩展时忽略并报告 |

本项目当前没有协商 ExtendedMouseButtons，因此不能把位 7 解析为扩展消息标记。

### 4.3 noVNC 实际行为

固定调研版本：noVNC 提交 `7c36fabe599e053c5a81e98e091ac636f6c1e174`。

- 普通移动和按钮消息发送当前完整按钮掩码。
- 垂直滚轮达到一步阈值时，先发送带按钮 4 或 5 的掩码，再发送去掉该位的掩码。
- 滚轮消息仍携带当前坐标和左、中、右按钮状态。
- 非扩展 `PointerEvent` 发送路径会清除位 7。

映射器必须只在滚轮位从 0 变为 1 时产生一步滚动；滚轮释放消息只更新映射器掩码，不产生反向滚动。

## 5. 现有代码约束

### 5.1 RFB TCP 层

`RfbTcpEvent::Pointer` 已提供：

```rust
Pointer {
    client_id: RfbClientId,
    button_mask: u8,
    x: u16,
    y: u16,
}
```

TCP 层只负责按序交付协议值，不应引入 HID 或 CH9329 概念。

### 5.2 core 指针抽象

`PointerEvent` 已有：

```rust
AbsoluteMove {
    x: u32,
    y: u32,
    framebuffer_size: FramebufferSize,
}
Button {
    button: PointerButton,
    down: bool,
}
Wheel {
    delta: i16,
}
```

其中：

- 绝对移动接受帧缓冲像素坐标。
- CH9329 sink 内部负责映射到 `0..=4095`。
- 按钮事件依赖已知的绝对位置。
- 滚轮正数表示向上，负数表示向下。
- 单个 `handle_pointer` 调用目前各自入队，无法保证一条 RFB 消息的多步变化原子提交。

## 6. 原子指针批次

### 6.1 `InputSink` 契约

增加：

```rust
fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()>;
```

保留单事件入口，并让默认实现委托到批次：

```rust
fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
    self.handle_pointer_batch(std::slice::from_ref(&event))
}
```

所有生产 sink 和测试 sink 必须实现批次方法，不能用默认循环逐条调用模拟原子性。

### 6.2 CH9329 批次语义

`Ch9329InputSink::handle_pointer_batch`：

1. 复制当前 `MouseState` 为候选状态。
2. 按事件顺序在候选状态上构造全部 CH9329 命令。
3. 任一事件无效时，不入队、不修改原状态。
4. 将所有非空命令放入一个 `CommandBatch`。
5. 队列接受后才替换 `MouseState`。
6. 队列拒绝时保留原状态，整个批次可以重试。

与键盘批次不同，指针批次不能只比较最终状态：

- 按下再释放虽然最终按钮状态不变，但它表示一次点击。
- 向上再向下虽然净滚轮值为零，但两个离散滚轮步骤都必须保留。
- 相对位移和滚轮可能需要拆成多个 CH9329 报告。

空批次和只包含零相对位移、零滚轮、重复按钮状态的批次不入队。

### 6.3 兼容现有行为

单事件方法继续通过批次实现，现有测试行为保持不变：

- 绝对移动始终发送当前位置。
- 重复按钮状态不发送报告。
- 零相对位移和零滚轮不发送报告。
- 相对移动和大滚轮值继续拆包。
- `release_all()` 和鼠标模式切换维持现有独立原子事务。

## 7. RFB 指针映射器

### 7.1 公共接口

在 `ipkvm-headless::rfb_input` 增加：

```rust
#[derive(Debug, Default)]
pub struct RfbPointerMapper {
    committed_button_mask: u8,
}

impl RfbPointerMapper {
    pub fn new() -> Self;

    pub fn handle_pointer(
        &mut self,
        sink: &mut impl InputSink,
        button_mask: u8,
        x: u16,
        y: u16,
        framebuffer_size: FramebufferSize,
    ) -> Result<RfbPointerOutcome, RfbPointerError>;
}
```

映射器不持有 sink，不创建队列，也不负责客户端身份和控制权。

### 7.2 结果和错误

```rust
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

忽略位通过结果显式返回，供后续事件泵累计指标或记录诊断；不把正常客户端的水平滚轮升级成连接级错误。

### 7.3 状态边界

映射器只保存一个 `u8`：

```rust
committed_button_mask: u8
```

它代表最后一次 sink 成功接受的 RFB 完整掩码。状态大小固定，不随输入增长。

## 8. 单条消息映射算法

给定新掩码、坐标和当前帧尺寸，构造一个 `Vec<PointerEvent>`：

1. 始终加入一个 `AbsoluteMove`，确保首次消息即使是按钮按下也有已知位置。
2. 对按钮 1、2、3，先按左、中、右顺序加入所有释放事件。
3. 再按左、中、右顺序加入所有按下事件。
4. 若按钮 4 是上升沿，加入 `Wheel { delta: 1 }`。
5. 若按钮 5 是上升沿，加入 `Wheel { delta: -1 }`。
6. 位 5、6、7 不生成 core 事件，但写入 outcome 的忽略掩码。
7. 一次调用 `sink.handle_pointer_batch(&events)`。
8. sink 成功后更新 `committed_button_mask`，失败时不更新。

释放先于按下可以确定地处理“左键切换为右键”等单条完整状态变化。滚轮放在按钮变化之后，使滚轮报告携带新消息声明的左、中、右按钮状态。

## 9. 坐标策略

### 9.1 不在适配层重复换算

映射器把 `x`、`y` 转为 `u32`，连同 `FramebufferSize` 写入 `AbsoluteMove`。CH9329 sink 继续作为唯一的 `0..=4095` 换算位置。

这样桌面端和 RFB 端不会形成两套取整公式。

core 继续使用已经按厂家资料固定的公式：

```text
mapped = floor(4096 * coordinate / extent)
```

因此离散帧缓冲的最后一个像素不一定恰好映射为 `4095`。例如 `1920x1080` 的 `(1919, 1079)` 映射为 `(4093, 4092)`。本阶段测试公式结果和合法范围，不把“最后像素必须等于 4095”作为新契约。

### 9.2 越界时拒绝，不裁剪

若：

- 帧尺寸任一维为零；
- `x >= width`；
- `y >= height`；

core 返回现有 `InputError`，整个原子批次失败。

不做静默裁剪，原因是：

- 越界通常表示尺寸更新竞争、过期客户端状态或协议异常。
- 裁剪会把错误坐标变成屏幕边缘点击，风险高于拒绝本次输入。
- 后续事件泵可以记录错误并等待下一条有效坐标，不需要断开连接。

## 10. 失败原子性

### 10.1 core 内部失败

以下任一情况必须保证没有命令入队、`MouseState` 不变：

- 批次中后续事件坐标非法。
- 事件和鼠标模式不匹配。
- 绝对按钮或滚轮在未知位置执行。
- CH9329 报告构造失败。
- 命令队列拒绝批次。

### 10.2 mapper 失败

`RfbPointerMapper` 仅在 `handle_pointer_batch` 成功后提交新掩码。

例如滚轮按下消息被拒绝时，重试同一掩码仍应产生滚轮上升沿，不能因 mapper 提前记录位 3 而丢失滚动。

## 11. 测试设计

### 11.1 core 原子批次

- 首次绝对移动加按钮按下只形成一个命令批次。
- 批次中后续非法事件不提交前面的移动。
- 队列拒绝后同一批次可以重试。
- 点击的按下和释放不会因最终状态相同而被折叠。
- 空批次和全无效变化不入队。
- 单事件 API 与批次 API 行为一致。

### 11.2 mapper 纯内存测试

- 第一条左键按下按“移动、左键按下”顺序提交。
- 左键切换为右键按“移动、左键释放、右键按下”顺序提交。
- 重复按钮状态不产生按钮事件，但仍提交消息坐标。
- 按钮 4 按下产生滚轮 `+1`，释放不产生第二步。
- 按钮 5 按下产生滚轮 `-1`。
- 同时出现按钮 4、5 时保留两个离散步骤。
- 水平滚轮和侧键被显式报告为忽略。
- sink 拒绝后重试仍产生相同事件。

### 11.3 真实 CH9329 sink 集成测试

- 帧缓冲四角符合既有厂家换算公式和合法坐标范围。
- 首次消息可以直接按下按钮。
- 一条消息的全部报告位于一个 `CommandBatch`。
- 队列拒绝后 mapper 和 sink 均可原样重试。
- 越界坐标不入队，后续有效消息仍可处理。

## 12. 备选方案及否决

### 12.1 逐条调用现有 `handle_pointer`

否决。移动成功而按钮失败时，sink 已改变位置但 mapper 不能确认整条 RFB 消息，无法可靠重试。

### 12.2 在 mapper 内直接生成 CH9329 报告

否决。会把设备协议、坐标换算和鼠标模式泄漏到 RFB 层，并复制 core 已有状态机。

### 12.3 把滚轮位视为每条消息的一步

否决。noVNC 和 RFC 使用按下、释放对；若按置位消息重复计数，移动期间保持位 3 会产生额外滚动。

### 12.4 静默裁剪越界坐标

否决。可能把过期或恶意坐标转换成屏幕边缘点击。

### 12.5 把未知按钮作为错误

当前否决。水平滚轮和侧键是正常客户端输入，忽略并返回可观测 outcome 比中断控制链路更稳妥。

## 13. 实施顺序

1. 为 `InputSink` 和 CH9329 sink 增加原子指针批次。
2. 使用原有单事件测试和新增失败测试固定兼容性。
3. 实现 `RfbPointerMapper` 的按钮差分、滚轮边沿和忽略位结果。
4. 增加公共 API 与真实 CH9329 sink 集成测试。
5. 回写 README 和粗粒度设计状态。
6. 运行统一本地验证并通过 PR 合并。

## 14. 自审清单

- [x] RFC 掩码按完整状态解释，而不是变化位。
- [x] 滚轮只在上升沿产生步骤。
- [x] 首次按钮消息先建立绝对位置。
- [x] 按钮释放先于按下，滚轮位于按钮变化之后。
- [x] 越界坐标拒绝且不提交部分状态。
- [x] sink 失败时 core 和 mapper 均可重试。
- [x] 点击和滚轮瞬时事件不按净状态折叠。
- [x] 未支持按钮有显式、可观测结果。
- [x] mapper 状态固定大小。
- [x] core 不出现 RFB 类型或命名。
- [x] 不新增外部依赖。
- [x] 所有文档说明使用中文。

## 15. 资料

- [RFC 6143 第 7.5.5 节 PointerEvent](https://www.rfc-editor.org/rfc/rfc6143#section-7.5.5)
- [RFB 社区规格 PointerEvent](https://github.com/rfbproto/rfbproto/blob/152107db63cd34b3536ad8ddf54a0cfc9017a9f9/rfbproto.rst#pointerevent)
- [noVNC 固定版本的鼠标和滚轮发送逻辑](https://github.com/novnc/noVNC/blob/7c36fabe599e053c5a81e98e091ac636f6c1e174/core/rfb.js#L1119-L1185)
- [现有粗粒度设计](../../ipkvm-coarse-design.md)
