# 单客户端 RFB TCP 传输与模拟帧源设计

## 1. 文档状态

- 关联 issue：`#5`
- 状态：设计已细化，等待按实施计划执行
- 适用阶段：硬件到货前的网络闭环
- 依赖前置：`#2` 已完成的 RFB 3.8 传输无关协议核心

## 2. 目标

本阶段建立第一个真实 TCP 网络闭环：

1. 在 `127.0.0.1` 的临时或配置端口接受 RFB TCP 客户端。
2. 使用现有 `RfbConnectionCore` 完成 RFB 3.8 `SecurityType None` 握手和消息处理。
3. 从实现 `FrameSource` 的视频源读取最新 BGRA8888 帧。
4. 响应客户端的 `FramebufferUpdateRequest`，发送 `Raw` 或 `DesktopSize` 更新。
5. 将键盘、鼠标、剪贴板和连续更新请求送入有界事件通道。
6. 同一时刻只处理一个活动客户端；断开后继续接受下一个客户端。
7. 使用真实回环 TCP 和模拟帧源完成全自动端到端验收。

本阶段不是完整的无头版产品。它只固定 TCP 传输、帧源适配和生命周期边界，为后续输入映射、WebSocket、noVNC 和多客户端策略提供稳定基础。

## 3. 不做范围

- WebSocket 和 noVNC 静态资源。
- 多个同时活动的 RFB 客户端。
- 控制权仲裁和只读客户端。
- RFB keysym 到 HID usage 的映射。
- RFB 按钮状态到 `PointerEvent` 的转换。
- `ClientCutText` 到模拟键入。
- CH9329 真实串口传输。
- Windows 或 Linux 真实视频采集。
- `ZRLE`、`Tight`、JPEG 或视频编码。
- 鉴权、TLS、访问控制和公网暴露。
- 可直接用于生产的命令行参数和配置文件。

## 4. 已比较方案

### 4.1 方案 A：TCP 传输放在 `ipkvm-headless`

`ipkvm-headless` 依赖 `ipkvm-rfb` 和 `ipkvm-video`，负责 Tokio TCP、连接生命周期、帧适配和事件通道。`ipkvm-rfb` 继续只处理字节协议和编码。

优点：

- 保持协议核心无运行时和操作系统 I/O 依赖。
- TCP 与后续 WebSocket 都在应用边界接入同一个协议核心。
- `ipkvm-rfb` 的性质测试和分片测试不受异步调度影响。
- 符合粗粒度设计中“异步边界放在 headless 组装层”的约束。

缺点：

- `ipkvm-headless` 需要持有少量 RFB 连接驱动逻辑。
- 后续 WebSocket 接入时需要抽取可复用的事件处理函数。

### 4.2 方案 B：给 `ipkvm-rfb` 增加 Tokio TCP feature

优点：

- TCP 服务代码和协议代码位于同一个 crate。

缺点：

- 协议 crate 开始依赖具体运行时。
- feature 组合会扩大测试矩阵。
- WebSocket 不是普通 `AsyncRead + AsyncWrite` 字节流，仍需额外适配。
- 桌面端若只复用协议和类型，也会承担不需要的网络依赖面。

### 4.3 方案 C：新增 `ipkvm-rfb-transport` crate

优点：

- 协议与传输在 crate 级别完全隔离。

缺点：

- 当前只有一个 TCP 实现，新增 crate 会增加发布、依赖和公共 API 成本。
- 现阶段的连接驱动规模不足以证明独立 crate 的必要性。

### 4.4 结论

采用方案 A。只有在 TCP 和 WebSocket 驱动出现明确、稳定且可独立测试的共享逻辑后，才考虑拆出新 crate。本阶段不为未来可能出现的抽象提前增加层级。

## 5. 模块边界

### 5.1 `ipkvm-rfb`

保持现有职责：

- RFB 3.8 握手状态机。
- 客户端消息增量解码。
- 像素格式协商。
- `Raw` 和 `DesktopSize` 编码。
- 协议输入、输出和帧缓冲上限。

本阶段不增加 Tokio、TCP、视频源或事件通道依赖。

### 5.2 `ipkvm-video`

调整并固定视频帧契约：

- `PixelFormat::Rgb` 改为 `PixelFormat::Bgra8888`。
- `Bgra8888` 明确表示每像素四字节，内存顺序为 B、G、R、A/X。
- RFB 路径忽略第四字节的 alpha 语义，但要求该字节存在。
- `FrameSource` 增加 `Send + Sync` 约束，允许在 Tokio 任务和桌面线程之间共享。
- `VideoFrame::seq` 是单个帧源内单调递增的序号；允许跳号，不允许倒退。
- `stride` 是字节数，必须至少为 `width * 4` 才能作为 BGRA8888 交给 RFB。

不在视频 crate 中依赖 `ipkvm-rfb`。视频帧到 `BgraFrameView` 的转换属于 headless 适配层。

### 5.3 `ipkvm-headless`

新增 `rfb_tcp` 模块，负责：

- TCP listener 驱动。
- 单活动客户端生命周期。
- `VideoFrame` 到 `BgraFrameView` 的校验和借用。
- 调用 `RfbConnectionCore`。
- RFB 客户端事件的有界输出。
- 握手超时、关闭和错误分类。

`ipkvm-headless` 不解析或重新编码 RFB 字段。所有协议字节仍由 `ipkvm-rfb` 处理。

## 6. 依赖和许可证

本阶段不增加新的顶层第三方 crate：

- `ipkvm-headless` 使用工作区已有的 Tokio 1.53.1。
- 为 headless 启用 `net`、`io-util`、`macros` 和 `rt` feature。
- 继续复用已有的 `sync` 和 `time` feature。
- 使用工作区已有的 `thiserror` 定义类型化错误。
- 测试可在 headless 的 Tokio dev-dependency 上启用 `test-util`，使用暂停时钟测试握手超时，不使用真实固定延时。

Tokio 和 thiserror 均为 MIT 许可，已经位于当前依赖树。启用 Tokio 网络 feature 可能激活 `bytes`、`mio`、`socket2` 和平台支持包；实施提交必须检查 `Cargo.lock` 的实际变化，但本阶段不引入 LGPL 或 GPL 边界。

## 7. 公共类型

### 7.1 TCP 配置

`ipkvm-headless` 增加 `RfbTcpConfig`：

```rust
pub struct RfbTcpConfig {
    pub desktop_name: String,
    pub read_buffer_bytes: usize,
    pub handshake_timeout: Duration,
    pub protocol_limits: RfbProtocolLimits,
}
```

约束：

- `desktop_name` 和协议上限继续由 `RfbConnectionCore::new` 校验。
- `read_buffer_bytes` 必须大于零，且不得大于 `max_buffered_input_bytes`。
- `handshake_timeout` 必须大于零。
- 默认读取缓冲为 16 KiB。
- 默认握手超时为 10 秒。

监听地址不放入 `RfbTcpConfig`。服务接受一个已经绑定的 `TcpListener`，生产组装层使用 `HeadlessConfig.bind_address` 和 `RfbServerConfig.tcp_port` 绑定，测试使用 `127.0.0.1:0`。

### 7.2 客户端编号

```rust
pub struct RfbClientId(u64);
```

- server 从 1 开始为每次 `accept` 分配编号。
- 编号只保证单个 server 实例生命周期内不重复。
- 编号溢出是 server 终止错误，不回绕复用。

### 7.3 事件出口

调用方创建有界 `tokio::sync::mpsc` 通道，并把 `Sender<RfbTcpEvent>` 交给 server。

事件包含：

```rust
pub enum RfbTcpEvent {
    Connected {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        shared: bool,
    },
    Key {
        client_id: RfbClientId,
        down: bool,
        keysym: u32,
    },
    Pointer {
        client_id: RfbClientId,
        button_mask: u8,
        x: u16,
        y: u16,
    },
    CutText {
        client_id: RfbClientId,
        bytes: Vec<u8>,
    },
    ContinuousUpdates {
        client_id: RfbClientId,
        enable: bool,
        rectangle: RfbRectangle,
    },
    Disconnected {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        reason: RfbDisconnectReason,
    },
}
```

断开原因使用可复制、可比较的分类，不把不可克隆的 `std::io::Error` 放入事件：

```rust
pub enum RfbDisconnectReason {
    ClientClosed,
    ServerShutdown,
    HandshakeTimeout,
    CoreConfig(RfbConfigError),
    Protocol(RfbProtocolError),
    Encode(RfbEncodeError),
    Frame(RfbTcpFrameError),
    Io(std::io::ErrorKind),
}
```

规则：

- `FramebufferUpdateRequested` 由 TCP 连接驱动消费，不送到应用事件通道。
- `HandshakeCompleted` 转换为 `Connected`。
- 键盘、鼠标和剪贴板事件保持 RFB 原始语义，本阶段不映射 HID。
- `EnableContinuousUpdates` 只作为能力请求上报；本阶段仍严格按普通 update request 发送画面。
- 通道满时使用 `send().await` 施加反压，不丢输入、不创建无界缓冲。
- 事件接收端关闭视为 server 组装错误，停止接受和处理客户端。

## 8. 服务生命周期

`RfbTcpServer` 持有：

- 已绑定的 `TcpListener`。
- `Arc<S>`，其中 `S: FrameSource + 'static`。
- `mpsc::Sender<RfbTcpEvent>`。
- `RfbTcpConfig`。
- 下一个 `RfbClientId`。

`run` 接受 `watch::Receiver<bool>` 作为关闭信号：

- `false` 表示继续运行。
- `true` 表示停止接受新连接，并结束当前连接。
- 如果启动时已经为 `true`，不调用 `accept`。
- shutdown sender 被异常丢弃时，按关闭处理。
- 等待新连接时同时监听 `event_tx.closed()`；事件接收端关闭后，即使没有活动客户端也立即返回 `EventChannelClosed`。

server 同一时间只执行一个连接任务。第二个客户端可以完成 TCP 建连并停留在操作系统 backlog 中，但在当前客户端结束前不会收到 RFB banner。当前连接结束后，server 接受下一个客户端。

这个行为具有以下性质：

- 不会同时存在两个输入控制者。
- 不需要为“忙”客户端增加第二套协议错误响应。
- 不创建客户端任务集合和多客户端帧队列。
- 后续单控制者/多查看者设计可以整体替换 accept 调度，不影响 `RfbConnectionCore`。

## 9. 单连接状态机

### 9.1 接受连接

每次 `accept` 后：

1. 分配 `RfbClientId`。
2. 从 `FrameSource::subscribe()` 获得该连接自己的 watch receiver。
3. 克隆 receiver 当前持有的最新帧。
4. 校验帧格式、尺寸、stride 和数据长度。
5. 使用当前帧尺寸创建 `RfbConnectionCore`。
6. 立即写出 core 初始输出中的 RFB banner。
7. 启动握手截止时间。

如果当前没有帧，或者当前帧不是合法 BGRA8888：

- 不发送虚假的固定尺寸画面。
- 关闭该客户端。
- 产生带类型化原因的 `Disconnected`。
- server 继续接受后续客户端。

视频会话组装层以后必须在启动 TCP server 前准备真实帧、黑帧或错误帧。TCP 层不负责合成画面。

### 9.2 握手

握手阶段同时等待：

- socket 可读。
- server shutdown。
- 握手截止时间。

每次读取到的字节原样传给 `RfbConnectionCore::push_input`。core 产生的输出立即通过 `write_all` 写出。

收到 `HandshakeCompleted` 后：

- 取消握手超时分支。
- 发送一次 `Connected` 事件。
- 进入正常消息阶段。

握手超时、协议错误、读取零字节或 I/O 错误都会结束连接。

### 9.3 正常消息阶段

正常阶段同时等待：

- socket 可读。
- 当存在待处理增量更新请求时，视频 watch receiver 发生变化。
- server shutdown。
- 事件接收端关闭。

从 socket 读到的字节交给 core。对 core 事件按原顺序处理：

1. 已完成握手后的输入事件进入有界事件通道。
2. `FramebufferUpdateRequested` 进入更新请求合并器。
3. 任一协议错误终止后续处理。
4. core 输出在每轮处理后立即取出并写入 socket。

有效事件出现在同一批次的协议错误之前时，先交付有效事件，再关闭连接。这与 `RfbConnectionCore` 已固定的错误语义一致。

## 10. 帧适配

headless 中定义私有或 crate 内可见的 `BgraFrameAdapter`，转换规则如下：

1. `pixel_format` 必须为 `PixelFormat::Bgra8888`。
2. `width` 和 `height` 必须位于 `1..=u16::MAX`。
3. `stride` 必须可转换为 `usize`。
4. 使用 `RfbSize::new` 校验非零尺寸。
5. 使用 `BgraFrameView::new` 校验 stride、数据长度和算术溢出。
6. 连接已观察到帧序号后，新帧序号不得小于该序号。

重复序号表示帧内容没有发生可观察变化：

- 非增量请求仍可重新发送该帧。
- 增量请求继续等待更大的序号。
- watch receiver 报告变化但序号倒退时，连接以 `FrameSequenceRegressed` 结束。

适配器只借用 `VideoFrame.data`，不复制整帧。调用 `queue_framebuffer_update` 时持有对应的 `Arc<VideoFrame>`，编码完成后释放该借用。

## 11. FramebufferUpdateRequest 语义

RFC 6143 允许多个 outstanding `FramebufferUpdateRequest`，并允许一个更新满足多个请求。因此本阶段不以“同时存在多个请求”为协议错误。

连接维护一个有界 `PendingFramebufferRequest`：

- 只保存一个矩形、是否必须完整更新，以及是否已有请求。
- 多个增量请求合并为覆盖所有请求矩形的最小外接矩形。
- 每个新请求先与 pending 状态合并；只要合并结果包含非增量请求，就立即使用最新帧响应整个合并区域并清空 pending。
- 多个等待中的增量请求由一个后续更新满足。
- 合并运算使用 `u32` 中间值并裁剪到当前 RFB 尺寸，禁止 `u16` 回绕。
- 待处理状态大小恒定，不随客户端请求数量增长。

发送规则：

### 11.1 非增量请求

先把请求与已有 pending 增量请求合并，再立即使用 receiver 当前最新帧调用 `queue_framebuffer_update`。即使帧序号与上次发送相同，也必须响应；这个更新同时满足此前已合并的增量请求。

### 11.2 增量请求

- 如果当前最新帧序号大于上次发送序号，立即响应。
- 如果该连接尚未发送过帧，立即响应当前帧。
- 如果没有新帧，把请求合并到 pending 状态。
- pending 存在时，等待 watch receiver 变化；序号增大后发送一次更新并清空 pending。

### 11.3 动态尺寸

帧尺寸变化只在存在客户端请求时通知，不发送 unsolicited update。

- 客户端已声明 `DesktopSize`：core 发送独立的 DesktopSize 更新，该请求被视为已满足；客户端下一次请求再收到新尺寸像素。
- 客户端未声明 `DesktopSize`：core 返回 `DesktopSizeNotNegotiated`，连接关闭并记录类型化原因。

该行为直接复用 `RfbConnectionCore` 已有的原子性：尺寸编码失败不会提交新尺寸。

## 12. 输出和反压

### 12.1 协议输出

- core 的 `max_queued_output_bytes` 限制单次协议输出。
- TCP 驱动每次修改 core 后立即 `take_output`。
- 使用 `write_all` 完整写出当前消息。
- 慢客户端会让连接任务停在 socket 写入，形成 TCP 反压。
- 写入期间不继续读取请求，也不排队新视频帧。
- 视频 watch 通道只保留最新帧，因此慢客户端恢复后不会重放过时帧。

单连接最多同时持有：

- 一份共享 `VideoFrame`。
- 一份 core 输出 `Vec<u8>`。
- 固定大小 TCP 读取缓冲。
- 一个固定大小 pending request。

### 12.2 输入事件

- 使用调用方提供的有界 mpsc 通道。
- 通道满时连接暂停读取，依靠 TCP 接收窗口施加反压。
- 不使用 `try_send` 丢弃键盘释放、按钮释放或生命周期事件。
- 接收端关闭时 server 停止，因为继续接收远程输入已没有正确消费者。

## 13. 关闭和重连

连接结束原因至少区分：

- 客户端正常关闭。
- server shutdown。
- 握手超时。
- RFB 协议错误。
- RFB 编码错误。
- 视频帧错误。
- socket I/O 错误类型。

每个已接受客户端最多产生一次 `Disconnected` 事件。是否完成过握手由此前是否存在 `Connected` 事件判断。

结束顺序：

1. 停止处理新的 socket 和帧事件。
2. 尝试关闭 TCP 写方向。
3. 发送一次 `Disconnected`。
4. 释放连接持有的帧和 core。
5. 非 server 致命错误时回到 `accept`。

listener accept 错误、客户端编号溢出和事件接收端关闭属于 server 致命错误；普通客户端协议错误、帧错误和 I/O 错误只结束该连接。

## 14. 错误模型

### 14.1 配置错误

`RfbTcpConfigError`：

- `ZeroReadBuffer`
- `ReadBufferExceedsInputLimit`
- `ZeroHandshakeTimeout`

### 14.2 帧错误

`RfbTcpFrameError`：

- `FrameUnavailable`
- `UnsupportedPixelFormat`
- `WidthOutOfRange`
- `HeightOutOfRange`
- `StrideOutOfRange`
- `InvalidBgraFrame(RfbFramebufferError)`
- `FrameSequenceRegressed`

### 14.3 连接错误

`RfbTcpConnectionError`：

- `HandshakeTimeout`
- `CoreConfig(RfbConfigError)`
- `Protocol(RfbProtocolError)`
- `Encode(RfbEncodeError)`
- `Frame(RfbTcpFrameError)`
- `Io(std::io::Error)`
- `EventChannelClosed`

### 14.4 server 错误

`RfbTcpServerError`：

- `Config(RfbTcpConfigError)`
- `Accept(std::io::Error)`
- `ClientIdOverflow`
- `EventChannelClosed`

错误不得被吞掉或只打印。普通连接错误转换为 `Disconnected.reason`；`EventChannelClosed` 因为已经没有事件消费者，直接升级为 server 致命错误，无法再发送 `Disconnected`。server 致命错误由 `run` 返回给未来的进程生命周期管理器。

## 15. 自动化测试设计

### 15.1 视频契约单元测试

- `Bgra8888` 明确记录每像素四字节。
- mock 帧源在跨任务使用时满足 `Send + Sync`。
- 模拟源订阅者只观察最新帧。

### 15.2 帧适配单元测试

- 拒绝非 BGRA8888。
- 拒绝零尺寸和超过 `u16::MAX` 的尺寸。
- 拒绝过短 stride。
- 拒绝过短数据。
- 接受带行尾 padding 的 BGRA8888。
- 序号倒退产生确定错误。

### 15.3 配置单元测试

- 零读取缓冲被拒绝。
- 读取缓冲超过协议输入上限被拒绝。
- 零握手超时被拒绝。
- 默认值符合设计。

### 15.4 回环 TCP 集成测试

每个测试绑定 `127.0.0.1:0`，不占用固定端口。测试客户端直接按 RFC 构造和解析字节，不复用 server 编码函数。

覆盖：

1. 完整 RFB 3.8 None 握手。
2. 客户端逐字节发送握手和消息，验证 TCP 分片无关。
3. 默认 BGRX8888 Raw 更新的精确字节。
4. 协商 RGB565 后 Raw 更新的精确字节。
5. 非增量请求即使帧序号未变也收到响应。
6. 增量请求在没有新帧时不收到 unsolicited update。
7. 发布新帧后 pending 增量请求收到更新。
8. 多个 outstanding 增量请求被合并为一个有界更新。
9. 协商 DesktopSize 后尺寸变化先收到独立 resize，再由下一请求获得像素。
10. 未协商 DesktopSize 时尺寸变化产生确定断线原因。
11. Key、Pointer、CutText 和 ContinuousUpdates 事件保持顺序。
12. 客户端断开后 server 接受并服务第二个客户端。
13. 第二个 TCP 客户端在首个活动连接结束前不收到 banner。
14. server shutdown 结束当前连接并返回。
15. 事件接收端关闭使 server 返回致命错误。
16. 容量为 1 的事件通道产生反压；接收端恢复读取后，输入事件无丢失且顺序不变。
17. 后续帧超过 `max_framebuffer_bytes` 时产生确定断线原因且不写半条消息；`RfbConnectionCore` 已保证合法配置下单次 Raw 更新可被输出上限完整容纳。

### 15.5 确定性时间测试

握手超时使用 Tokio `test-util` 的暂停时钟和时间推进，不使用依赖机器负载的短 sleep。

### 15.6 命令级验证

```powershell
.\scripts\verify.ps1
```

本阶段没有必须人工执行的测试。第三方 VNC 客户端可以作为后续兼容性观察，但不能替代自动化字节级验收。

## 16. 文档更新

实施完成后：

- `README.md` 更新为“已有单客户端 RFB TCP 库闭环”，不得声称无头二进制已可直接用于真实设备。
- `docs/ipkvm-coarse-design.md` 把阶段 0 的普通 TCP 模拟帧闭环标为完成。
- 本文档记录最终公共类型和行为；若实施发现契约需要调整，先回写本文档再改实现。
- issue `#5` 记录红灯测试、实施提交和最终验证证据。

## 17. 后续顺序

本阶段完成后按独立 issue 推进：

1. RFB keysym、按钮状态和剪贴板到 `ipkvm-core` 输入接口的适配。
2. 单控制者生命周期与断开 `release_all`。
3. 依赖许可证白名单和自动审计门禁。
4. RFB over WebSocket 与 noVNC。
5. 多查看者、慢客户端丢帧策略和状态统计。

TCP 和 WebSocket 入口不得重新实现握手、消息解码、像素格式或 framebuffer 编码。
