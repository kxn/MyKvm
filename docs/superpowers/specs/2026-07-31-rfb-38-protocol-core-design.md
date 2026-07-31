# RFB 3.8 纯协议核心设计

## 1. 文档目的

本文定义 `ipkvm-rfb` 的第一个可执行增量：实现同步、传输无关、可自动化验证的 RFB 3.8 服务器协议核心。

本设计关联 Gitea issue `#2`。实施前必须先完成独立实施计划和计划自审，不允许直接从本文跳到编码。

## 2. 背景

当前 `ipkvm-rfb` 只有标准端口配置，尚无协议实现。粗粒度设计同时列出了握手、消息解析、Raw 编码、TCP、WebSocket、多客户端、输入控制权和 noVNC 接入。一次实现这些内容会把字节协议、网络生命周期、并发和业务策略耦合在一起，自动化测试也难以定位问题。

本次先完成纯协议核心。后续 TCP 和 WebSocket 入口只负责搬运字节，必须复用同一个协议状态机，不再复制握手或消息解析逻辑。

## 3. 资料优先级

协议判断按以下优先级处理：

1. RFC 6143 的 RFB 3.8 基础协议。
2. RFB 社区规格对 `DesktopSize` 和已知扩展的兼容性说明。
3. noVNC 和普通 VNC 客户端的实测行为，留到 TCP 接入阶段验证。
4. 第三方 Rust 实现只作为测试案例和风险线索，不作为协议权威。

本地长期参考：

- `docs/references/RFC6143-rfb-protocol.txt`
- `docs/references/rfbproto-community-spec.rst`

外部参考：

- RFC 6143：https://www.rfc-editor.org/rfc/rfc6143
- RFB 社区规格：https://github.com/rfbproto/rfbproto/blob/master/rfbproto.rst
- `rustvncserver` 协议模块：https://docs.rs/rustvncserver/latest/rustvncserver/protocol/

## 4. 方案选择

### 4.1 不采用完整第三方 VNC 服务 crate

`rustvncserver 2.2.1` 使用 Apache-2.0，但要求 Rust 1.90，高于本项目锁定的 Rust 1.89，并同时引入 Tokio 网络、VNC 认证、DES、压缩和多种编码。旧 `rfb 0.1.0` 发布于 2022 年，公共文档覆盖率为零。

本项目当前只需要基础 RFB 3.8 子集。引入完整服务会扩大许可证清单、依赖面和行为面，并且仍需额外适配项目自己的视频帧和输入控制权模型。

### 4.2 不采用协议与 TCP 合并的单体实现

如果握手状态机直接持有 `TcpStream`：

- WebSocket 入口无法复用相同逻辑。
- 任意分块测试必须启动真实 socket。
- 协议错误、网络错误和业务错误会混在同一层。
- 多客户端和反压会过早进入本次范围。

### 4.3 采用分层纯协议核心

协议核心只消费输入字节并产生：

- 需要写回客户端的字节。
- 交给上层处理的类型化事件。
- 确定性的协议错误。

该方案与 `ipkvm-core` 的同步纯逻辑边界一致，也允许 TCP 和 WebSocket 使用同一套测试过的状态机。

## 5. 目标

本次必须实现：

- RFB 3.8 版本协商。
- `SecurityType None` 协商和成功结果。
- `ClientInit` 与 `ServerInit`。
- 客户端消息增量解码。
- 客户端像素格式校验和状态更新。
- encoding 偏好与 `DesktopSize` 能力记录。
- `Raw` framebuffer 更新。
- 独立的 `DesktopSize` framebuffer 更新。
- 任意输入分块、连续多消息和不完整消息处理。
- 输入和输出长度上限、检查运算和失败终态。

## 6. 非目标

本次不实现：

- TCP 监听和 socket 生命周期。
- WebSocket、HTTP 和 noVNC 静态资源。
- 多客户端、首个控制者、只读观察者和断开释放。
- 帧率限制、网络反压、旧帧丢弃和脏块检测。
- keysym 到 USB HID 的映射。
- RFB 按钮状态到 `PointerEvent` 的转换。
- `ClientCutText` 到模拟键入。
- VNC Authentication、TLS 或其他安全类型。
- `CopyRect`、Hextile、ZRLE、Tight、JPEG 或视频编码。
- 连续更新扩展的执行语义。
- `ExtendedDesktopSize` 和客户端发起的 `SetDesktopSize`。
- 真实视频源到 BGRA 帧的转换。

以上内容分别进入后续 issue，不在本次保留半实现分支或无测试占位接口。

## 7. 模块边界

计划将 `ipkvm-rfb` 拆为：

```text
crates/ipkvm-rfb/src/
  lib.rs
  connection.rs
  framebuffer.rs
  protocol/
    mod.rs
    wire.rs
    pixel_format.rs
    client.rs
    server.rs
```

职责：

- `wire.rs`：大端整数读取、检查切片、检查长度和写入辅助。
- `pixel_format.rs`：RFB `PIXEL_FORMAT`、校验、通道缩放和像素写出。
- `client.rs`：客户端消息类型和增量解码器。
- `server.rs`：服务器握手消息、`ServerInit` 和 framebuffer 更新编码。
- `framebuffer.rs`：经过校验的 BGRA8888 帧视图和矩形运算。
- `connection.rs`：连接状态机、输出队列、协商状态和外部事件。
- `lib.rs`：现有配置和稳定公共导出，不放协议实现细节。

`ipkvm-rfb` 在本次移除尚未使用的 `ipkvm-core` 和 `ipkvm-video` 依赖。正常依赖只增加工作区已有的 `thiserror`；性质测试使用工作区已有的 `proptest` 开发依赖。

## 8. 公共数据类型

### 8.1 RFB 尺寸和矩形

```rust
pub struct RfbSize {
    width: u16,
    height: u16,
}

pub struct RfbRectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
```

`RfbSize::new(width, height) -> Result<RfbSize, RfbFramebufferError>` 拒绝零宽或零高；`width()` 和 `height()` 返回只读值，调用者不能绕过不变量。矩形允许零宽或零高，因为合法客户端可以请求空区域；所有 `x + width` 和 `y + height` 使用 `u32` 或检查运算。

更新请求使用：

```rust
pub struct FramebufferUpdateRequest {
    pub incremental: bool,
    pub rectangle: RfbRectangle,
}
```

### 8.2 连接配置

```rust
pub struct RfbConnectionConfig {
    pub desktop_name: String,
    pub initial_size: RfbSize,
    pub limits: RfbProtocolLimits,
}

pub struct RfbProtocolLimits {
    pub max_desktop_name_bytes: usize,
    pub max_encodings: usize,
    pub max_cut_text_bytes: usize,
    pub max_buffered_input_bytes: usize,
    pub max_queued_output_bytes: usize,
    pub max_framebuffer_bytes: usize,
}
```

默认限制：

| 项目 | 默认值 |
|---|---:|
| desktop name | 1024 字节 |
| encoding 数量 | 4096 |
| `ClientCutText` | 1 MiB |
| 待解析输入缓存 | 2 MiB |
| 待发送输出队列 | 256 MiB |
| 单个 framebuffer 视图有效数据 | 128 MiB |

配置构造时验证：

- desktop name 使用 UTF-8 字节长度，不超过限制和 `u32::MAX`。
- 所有限制必须大于零，`max_framebuffer_bytes` 至少为 4，保证可以表达一个 BGRA 像素。
- 初始尺寸的最紧密 BGRA 帧 `width * height * 4` 必须不超过 `max_framebuffer_bytes`，避免创建一个永远无法发送初始帧的连接。
- `max_buffered_input_bytes` 必须至少能容纳 `max_cut_text_bytes + 8`、`max_encodings * 4 + 4` 和 20 字节 `SetPixelFormat` 中的最大值。
- `max_queued_output_bytes` 必须至少能容纳 `max_framebuffer_bytes + 16`，其中 16 字节是单矩形 Raw update 的消息头和 rectangle header。
- `max_queued_output_bytes` 还必须能同时容纳完整握手期间的全部服务器输出：12 字节版本、2 字节安全类型列表、4 字节 `SecurityResult` 和 `24 + desktop_name.len()` 字节 `ServerInit`。这样即使调用者到握手结束才第一次取走输出，也不会因合法握手进入容量错误。
- 上述加法和乘法在配置校验时也必须使用检查运算。

### 8.3 BGRA 帧视图

```rust
pub struct BgraFrameView<'a> {
    size: RfbSize,
    stride: usize,
    pixels: &'a [u8],
}
```

构造要求：

- 每像素固定 4 字节，顺序为 B、G、R、A。
- `stride >= width * 4`。
- `pixels.len()` 至少覆盖 `(height - 1) * stride + width * 4`，不强制最后一行带 stride 尾部填充。
- 所有乘法和加法使用 `checked_mul`、`checked_add`。

公开构造签名为：

```rust
pub fn new(
    size: RfbSize,
    stride: usize,
    pixels: &'a [u8],
) -> Result<BgraFrameView<'a>, RfbFramebufferError>;
```

构造器只验证帧的结构。连接在排队更新时再用自己的 `max_framebuffer_bytes` 检查该视图会访问的字节跨度，避免让帧视图依赖某个连接配置。

Alpha 字节不发送到目标颜色通道。默认 32 位自然格式的未使用高字节写零。

## 9. 像素格式

### 9.1 默认自然格式

服务器在 `ServerInit` 中声明：

```text
bits-per-pixel = 32
depth = 24
big-endian = false
true-color = true
red-max = 255
green-max = 255
blue-max = 255
red-shift = 16
green-shift = 8
blue-shift = 0
```

该格式的线中字节为 B、G、R、0，可从 BGRA 输入快速转换。

像素格式是经过校验的值类型：

```rust
pub struct RfbPixelFormat { /* 私有字段 */ }

impl RfbPixelFormat {
    pub fn new(
        bits_per_pixel: u8,
        depth: u8,
        big_endian: bool,
        red_max: u16,
        green_max: u16,
        blue_max: u16,
        red_shift: u8,
        green_shift: u8,
        blue_shift: u8,
    ) -> Result<Self, RfbPixelFormatError>;

    pub fn default_bgrx8888() -> Self;
}
```

`true-color=false` 来自 wire 解析时直接返回 `RfbPixelFormatError::ColorMapUnsupported`，因此公共构造器不接受一个没有合法含义的 `true_color` 参数。各字段通过只读 getter 暴露。

### 9.2 客户端格式校验

支持条件：

- `bits-per-pixel` 只能是 8、16 或 32。
- `depth` 大于零且不大于 `bits-per-pixel`。
- `true-color` 必须为 true；不实现色表。
- 每个 `channel-max` 必须大于零并满足 `2^N - 1`。
- `shift + N` 不得超过 `bits-per-pixel`。
- 红、绿、蓝位掩码不得重叠。
- 三个通道占用位数不得超过 `depth`；允许历史客户端把 `depth` 声明为 `bits-per-pixel`。

像素转换按以下步骤进行：

1. 将 8 位输入通道按 `(value * channel_max + 127) / 255` 缩放到 `0..=channel-max`。
2. 左移到目标位域。
3. 合并为 `u32` 像素值。
4. 按 `bits-per-pixel` 和大小端标记输出 1、2 或 4 字节。

测试必须覆盖：

- 默认 32 位小端。
- 32 位大端。
- RGB565。
- RGB332。
- 非 8 位满值通道缩放。
- 非法 max、重叠 mask、越界 shift 和色表格式。

## 10. 客户端消息解码

正常阶段支持以下消息：

| 类型 | 名称 | 固定头长度 | 可变部分 |
|---:|---|---:|---|
| 0 | `SetPixelFormat` | 20 | 无 |
| 2 | `SetEncodings` | 4 | `count * 4` |
| 3 | `FramebufferUpdateRequest` | 10 | 无 |
| 4 | `KeyEvent` | 8 | 无 |
| 5 | `PointerEvent` | 6 | 无 |
| 6 | `ClientCutText` | 8 | `length` |
| 150 | `EnableContinuousUpdates` | 10 | 无 |

增量解码器要求：

- 任意消息可以跨任意数量的输入块。
- 一个输入块可以包含任意数量的完整消息和一个不完整尾部。
- 不完整消息不返回错误，只保留必要字节。
- 解析循环用游标累计已消费长度，循环结束后一次性移除已消费前缀，避免连续小消息触发反复搬移和平方级开销。
- 追加新输入前先用检查加法验证 `现有缓存长度 + bytes.len()`；超限时不追加任何新字节，直接返回致命错误，禁止先分配再检查。
- `SetEncodings` 在等待完整 body 前先检查数量上限和乘法溢出。
- `ClientCutText` 在等待完整 body 前先检查长度上限和加法溢出。
- 输入缓存超过限制时返回致命错误。
- padding 字节不要求为零，以兼容现有客户端。
- 非零布尔字段统一解释为 true。

未知消息类型无法确定长度，返回 `UnsupportedClientMessageType`，连接进入失败终态，不尝试扫描后续字节重新同步。

每条 `SetEncodings` 消息完整替换上一条 encoding 偏好并保持客户端给出的顺序。未知正数 encoding 和未知负数 pseudo-encoding 均保留在偏好列表中，但不产生错误。`DesktopSize` 能力取决于当前列表是否包含 `-223`，后续列表移除它时能力也随之撤销。服务器始终可以使用 Raw，因为 RFC 要求客户端支持 Raw。

`EnableContinuousUpdates` 被完整解析为类型化事件，但本次不启用连续更新，也不通过 `EndOfContinuousUpdates` 宣告服务器支持该扩展。这样即使客户端违规提前发送该消息，后续字节流也不会失去同步。

`ClientCutText` 保存原始 Latin-1 字节为 `Vec<u8>`，本层不执行 UTF-8 解码。

## 11. 连接状态机

### 11.1 状态

```text
等待客户端版本
→ 等待安全类型选择
→ 等待 ClientInit
→ 正常
→ 失败
```

公开状态类型固定为：

```rust
pub enum RfbConnectionState {
    AwaitingVersion,
    AwaitingSecuritySelection,
    AwaitingClientInit,
    Normal,
    Failed,
}
```

只实现 RFB 3.8：

1. 新建连接后，输出队列包含 `RFB 003.008\n`。
2. 客户端必须回复 `RFB 003.008\n`。
3. 服务器输出 `[1, 1]`，表示一个安全类型 `None`。
4. 客户端必须选择 `[1]`。
5. 服务器输出 4 字节成功 `SecurityResult`。
6. 客户端发送 1 字节 `ClientInit`。
7. 服务器输出 `ServerInit`，进入正常状态。

不接受 3.3、3.7 和非标准版本。版本协商失败返回明确错误并进入失败状态；网络层后续负责关闭连接。本次不生成跨版本失败字符串，避免错误套用另一版本的握手格式。

### 11.2 公共接口

```rust
pub struct RfbConnectionCore { /* 私有字段 */ }

impl RfbConnectionCore {
    pub fn new(config: RfbConnectionConfig) -> Result<Self, RfbConfigError>;

    pub fn push_input(
        &mut self,
        bytes: &[u8],
    ) -> Vec<Result<RfbEvent, RfbProtocolError>>;

    pub fn take_output(&mut self) -> Vec<u8>;

    pub fn queue_framebuffer_update(
        &mut self,
        frame: BgraFrameView<'_>,
        request: FramebufferUpdateRequest,
    ) -> Result<FramebufferUpdateOutcome, RfbEncodeError>;

    pub fn state(&self) -> RfbConnectionState;
    pub fn pixel_format(&self) -> RfbPixelFormat;
    pub fn encoding_preferences(&self) -> &[i32];
    pub fn supports_desktop_size(&self) -> bool;
}
```

`push_input` 与现有 CH9329 增量解帧接口保持相似：

- 一个输入块可以产生多个事件。
- 握手中间步骤只追加输出，不一定产生事件。
- 进入正常状态时产生 `HandshakeCompleted`。
- 致命错误作为结果列表最后一个 `Err` 返回，并立即切换到失败状态。
- 致命错误前已完整解析的事件仍按顺序保留。
- 失败状态收到后续输入时返回 `ConnectionFailed`，不继续解释字节。

`take_output` 原子取走当前输出队列并将队列清空。所有生成方法必须先完整构造并检查长度，成功后才追加；失败不得留下部分消息或改变协商状态。

构造器已经保证完整握手输出能够全部进入队列，所以 `push_input` 不会因合法配置和合法握手产生输出容量错误。正常阶段的客户端消息不自动生成 framebuffer 输出；显式排队更新的容量错误由 `queue_framebuffer_update` 返回。

### 11.3 外部事件

```rust
pub enum RfbEvent {
    HandshakeCompleted { shared: bool },
    FramebufferUpdateRequested(FramebufferUpdateRequest),
    Key { down: bool, keysym: u32 },
    Pointer { button_mask: u8, x: u16, y: u16 },
    CutText(Vec<u8>),
    EnableContinuousUpdates {
        enable: bool,
        rectangle: RfbRectangle,
    },
}
```

`SetPixelFormat` 和 `SetEncodings` 由状态机内部应用，不额外产生业务事件。调用者可以通过 getter 读取当前协商状态。

## 12. Framebuffer 更新

### 12.1 请求区域

`FramebufferUpdateRequest` 保存：

- `incremental`。
- 请求矩形。

Raw 编码使用请求矩形与当前帧边界的交集：

- 非空交集生成一个 Raw 矩形。
- 空交集生成合法的零矩形 `FramebufferUpdate`，用于结束本次请求。
- 本次没有脏块跟踪；`incremental=true` 仍发送交集内完整像素。

矩形逐行读取 BGRA 输入，忽略 stride 尾部。像素输出长度必须与：

```text
width * height * bytes_per_pixel
```

完全一致。

### 12.2 分辨率变化

连接记录最后一次向客户端声明的尺寸。

当传入帧尺寸与已声明尺寸不同：

- 客户端未请求 `DesktopSize(-223)`：返回 `DesktopSizeNotNegotiated`，不追加输出，不改变已声明尺寸。
- 客户端已请求 `DesktopSize`：生成一个只含单个 `DesktopSize` 矩形的独立 `FramebufferUpdate`。
- `DesktopSize` 矩形必须是该更新的最后一个矩形；本设计直接让它成为唯一矩形。
- 线格式固定为 4 字节 `FramebufferUpdate` 头、`x=0`、`y=0`、新宽高和有符号编码值 `-223`，rectangle 后没有像素 body。
- 本次调用不追加 Raw 数据。
- 完整消息成功进入输出队列后，才提交新的已声明尺寸。
- 返回 `FramebufferUpdateOutcome::ResizeAnnounced`。
- 客户端下一次请求再获得新尺寸下的 Raw 更新。

只有实际尺寸变化才发送 `DesktopSize`，避免客户端尺寸更新循环。

### 12.3 更新结果

```rust
pub enum FramebufferUpdateOutcome {
    RawQueued { rectangle: RfbRectangle },
    EmptyQueued,
    ResizeAnnounced { size: RfbSize },
}
```

调用成功只表示完整消息已进入协议核心输出队列，不表示网络已经写出。

## 13. 错误模型

错误分为三类：

### 13.1 配置错误

`RfbConfigError`：

- desktop name 超限。
- 输入限制无法容纳其声明允许的最大可变消息。
- 输出限制无法容纳其声明允许的最大 framebuffer 加消息头。
- 输出限制无法容纳构造参数对应的完整握手输出。
- 限制为零或 framebuffer 限制小于一个 BGRA 像素。
- 初始尺寸的最紧密 BGRA 帧超过 framebuffer 限制。
- 限制计算溢出。

构造连接失败，不创建半初始化状态机。

### 13.2 Framebuffer 值错误

`RfbFramebufferError`：

- 零宽或零高尺寸。
- stride 小于一行 BGRA 像素。
- 像素切片不足以覆盖最后一个有效像素。
- 帧跨度计算溢出。

`RfbSize` 和 `BgraFrameView` 构造失败时返回该错误，不产生无效值。

### 13.3 像素格式错误

`RfbPixelFormatError`：

- bits-per-pixel 不是 8、16、32。
- depth 为零或大于 bits-per-pixel。
- 色表格式不受支持。
- channel max 不是 `2^N - 1`。
- 通道 shift 越界。
- 通道位掩码重叠。
- 通道位数超过 depth。

客户端消息中的非法像素格式会被包装为致命输入协议错误。应用直接构造格式时可以得到独立的精确错误。

### 13.4 输入协议错误

`RfbProtocolError`：

- 不支持的版本。
- 不支持的安全类型。
- 握手字段长度或顺序错误。
- 未知客户端消息类型。
- encoding 数量超限。
- cut text 长度超限。
- 输入缓存超限。
- 非法或不支持的像素格式。
- 长度计算溢出。
- 连接已经失败。

这些错误使连接进入失败终态。

### 13.5 输出编码错误

`RfbEncodeError`：

- framebuffer 数据超限。
- 像素或消息长度计算溢出。
- 输出队列容量不足。
- 尚未完成握手。
- 未协商 `DesktopSize` 的尺寸变化。

输出错误不自动使连接失败，因为它们可能来自上层提供的无效帧。所有输出操作保持事务性：错误时输出队列、已声明尺寸和其他连接状态不变。

## 14. 自动化测试

### 14.1 协议金样

- `RFB 003.008\n` 版本字节。
- `None` 安全类型列表和选择。
- 成功 `SecurityResult`。
- `ServerInit` 默认像素格式和 UTF-8 desktop name。
- RFC 中各客户端消息的固定字节布局。
- Raw 和 `DesktopSize` rectangle header。

### 14.2 增量解码

- 握手每一个字节切分边界。
- 每种客户端消息每一个切分边界。
- 一个块内连续多消息。
- 完整消息加不完整尾部。
- 未知消息后不继续解析。
- `EnableContinuousUpdates` 后紧跟 `KeyEvent`，验证没有流错位。
- 未知 encoding 值后连接保持正常。

### 14.3 状态机转录

使用公开 API 完成一条完整内存转录：

```text
服务器版本
客户端版本
安全类型列表
选择 None
SecurityResult
ClientInit
ServerInit
SetPixelFormat
SetEncodings
FramebufferUpdateRequest
Raw FramebufferUpdate
```

转录不启动 socket，不依赖时间或线程。

### 14.4 像素和帧

- 默认 32 位小端精确字节。
- 32 位大端。
- RGB565 和 RGB332。
- 非紧密 stride。
- 请求区域裁剪。
- 空交集。
- 短缓冲和 stride 过小。
- 尺寸变化未协商失败且状态不变。
- 尺寸变化已协商时只输出 `DesktopSize`。
- 下一次请求输出新尺寸 Raw。

### 14.5 性质测试

- 同一合法消息按任意分块输入，结果与一次输入相同。
- 随机合法 RGB 和目标格式输出长度正确。
- 随机请求矩形的交集不越过帧边界。
- 随机字节输入不会 panic。
- 输入缓存不会超过配置上限后仍继续增长。
- 任意输出编码失败不改变输出队列和已声明尺寸。

### 14.6 最终验证

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

只有完全无法自动化的真实 VNC 客户端兼容性才留到 TCP 接入阶段人工参与。本次没有必需的人工测试。

## 15. 后续边界

本次完成后，建议按独立 issue 推进：

1. 单客户端 TCP 传输和模拟帧源端到端测试。
2. keysym、PointerEvent 和 `ClientCutText` 到 `ipkvm-core` 的输入适配。
3. 多客户端、控制权、断开 `release_all` 和反压。
4. WebSocket 与 noVNC。
5. 脏块检测和压缩编码评估。

TCP 和 WebSocket 入口不得重新实现版本协商、消息解码、像素格式或 framebuffer 编码。
