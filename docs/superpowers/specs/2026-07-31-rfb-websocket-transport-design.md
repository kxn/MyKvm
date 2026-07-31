# 共享 RFB 连接驱动与 WebSocket 传输设计

## 1. 文档状态

- 关联 issue：`#15`
- 状态：已实施
- 适用阶段：无头版 WebSocket 网络闭环
- 前置依赖：`#5` 已完成的 RFB TCP 传输、`#11` 已完成的 RFB 输入事件泵、`#13` 已完成的依赖许可证门禁
- 后续阶段：noVNC 静态资源集成与真实浏览器闭环

### 1.1 实施事实核对

实现位于 `ipkvm-headless`：公共 `RfbConnectionSettings`、`RfbClientId`、`RfbServerEvent`、`RfbDisconnectReason`、`RfbFrameError` 与 `RfbConnectionGate` 均在 `rfb_connection`。`RfbConnectionGate::new()` 和 `Default::default()` 都创建容量为 1、客户端标识从 1 开始的独立连接闸门。闸门使用“未激活预约”和“已激活租约”两阶段所有权；预约普通析构会释放，租约只有在 `Disconnected` 成功入队后才显式释放，异常析构会关闭并毒化闸门。生产组装通过 `RfbTcpServer::new(listener, frame_source, event_tx, config, gate)` 与 `RfbWebSocketService::new(frame_source, event_tx, config, shutdown, gate)` 显式接收同一个连接闸门；后者的 `router()` 只提供 `/rfb`，监听器和 `ConnectInfo<SocketAddr>` 由上层组装。

错误分类保持类型化：TCP I/O 映射为 `RfbDisconnectReason::Io(ErrorKind)`，WebSocket 传输层故障在私有错误链中保留底层来源，对外映射为稳定的 `WebSocket`，Text 消息映射为 `UnexpectedTextMessage`；普通关闭、关闭信号、协议、编码和帧错误保留各自分类。`/rfb` 无子协议时返回不含 `Sec-WebSocket-Protocol` 的 `101`，请求 `binary` 时选择 `binary`；已有活动连接返回 `409`，关闭信号、事件接收端关闭、客户端标识耗尽和闸门中毒返回 `503`。单条 WebSocket 消息与帧的大小都使用 `max_buffered_input_bytes`，没有第二套重复上限。

自动化测试按实际测试文件分组记录在第 14 节：`rfb_websocket.rs` 有 16 个 WebSocket 集成测试，`rfb_transport_exclusion.rs` 有 2 个跨传输层排他性测试，`rfb_tcp.rs` 有 8 个 TCP 回归测试，`rfb_input_pump.rs` 有 2 个 RFB 输入泵集成测试。依赖锁定为 `axum 0.8.9`、`tokio-tungstenite 0.29.0` 与 `futures-util 0.3.33`，均来自 crates.io，并由许可证与来源门禁审计。后两项是本 crate 的直接开发依赖，同时也通过 axum WebSocket 功能进入正常生产依赖树。noVNC 仅作为固定线级样本来源，不进入仓库或产物；完整网页、真实浏览器、真实视频采集、真实串口、鉴权、TLS 和可运行的无头二进制不属于本次已实施内容。

## 2. 目标

本阶段在不复制 RFB 协议和连接行为的前提下增加 WebSocket 入口：

1. 把 TCP 模块中的传输无关连接逻辑抽到共享 `rfb_connection` 模块。
2. 保持现有 RFB TCP 握手、帧更新、输入事件、反压、关闭和重连行为不变。
3. 在 `ipkvm-headless` 中提供可组合的 axum `/rfb` WebSocket 路由。
4. WebSocket 收发的二进制负载直接承载连续 RFB 字节流，忽略 WebSocket 消息边界。
5. 默认不要求 WebSocket 子协议；客户端请求 `binary` 时在响应中选择 `binary`。
6. TCP 与 WebSocket 共用一个连接许可，同一时刻全局只允许一个活动 RFB 控制连接。
7. 使用锁定到 noVNC 1.7.0 源码提交的线级初始化样本完成自动化兼容性验收。
8. 所有新增依赖必须通过现有许可证、来源和锁文件审计。

本阶段交付的是可供后续 Web UI 组合的 RFB WebSocket 核心，不是完整网页产品。

## 3. 不做范围

- 不嵌入或分发 noVNC JavaScript、CSS、字体和图片。
- 不增加 HTML 页面、前端构建系统、Node.js 运行时或浏览器自动化。
- 不实现设置页、设备选择、运行时配置文件或完整 headless 进程组装。
- 不实现鉴权、TLS、来源校验、访问控制、会话令牌和公网暴露。
- 不实现多个同时活动的查看者或控制权仲裁。
- 不增加 `Tight`、`ZRLE`、JPEG、H.264 等 RFB 编码。
- 不增加视频压缩、转码或 FFmpeg。
- 不改变 CH9329 输入映射和视频采集边界。
- 不为旧的预发布 `RfbTcpEvent`、`RfbTcpFrameError` 公共名称保留兼容别名。

## 4. 调研结论

### 4.1 noVNC 传输行为

锁定兼容目标为 noVNC 1.7.0 源码提交
`63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`。

noVNC 的 `Websock` 具有以下行为：

- 使用浏览器原生 `WebSocket`。
- 把 `binaryType` 设置为 `arraybuffer`。
- 用二进制 WebSocket 消息发送 RFB 字节。
- 把每条收到的二进制消息追加到连续接收队列，WebSocket 消息边界不具有 RFB 语义。
- `wsProtocols` 默认为空数组，因此服务端不得把 `binary` 子协议作为连接前提。

noVNC 完成 `ServerInit` 后立即发送：

1. `SetPixelFormat`：32 bpp、24 depth、小端、true color，RGB 最大值均为 255，位移依次为 0、8、16。
2. `SetEncodings`：包含多个服务端当前不支持的普通编码和伪编码，同时包含 `Raw` 与 `DesktopSize`。
3. 覆盖整个 framebuffer 的非增量 `FramebufferUpdateRequest`。

服务端必须忽略未实现但合法的编码声明，并选择已实现的 `Raw`。noVNC 协商的像素格式使一个 RGB 像素在线上的四个字节为 `R, G, B, 0`。

### 4.2 axum WebSocket 行为

采用 axum 0.8.9，并只启用 `http1`、`tokio` 和 `ws` 功能开关：

- `WebSocketUpgrade::protocols(["binary"])` 只会在客户端提供该值时选择它；未提供子协议的客户端仍可升级。
- `max_message_size` 和 `max_frame_size` 可在升级前限制单条输入。
- axum/tungstenite 会处理 WebSocket 分片，并把完整消息交给应用层。
- `Ping`/`Pong` 是控制消息，不属于 RFB 字节流；连接驱动器忽略它们。
- `send(Message::Binary(...)).await` 是输出反压点，不增加应用层无界发送队列。

### 4.3 依赖与许可证

直接生产依赖：

- `axum = 0.8.9`，MIT。

直接开发依赖：

- `tokio-tungstenite = 0.29.0`，MIT，与 axum 0.8.9 的依赖版本保持一致。
- `futures-util = 0.3.33`，MIT 或 Apache-2.0，用于 WebSocket 测试客户端的 `SinkExt` 和 `StreamExt`。

`cargo tree -p ipkvm-headless -e normal` 同时显示 `tokio-tungstenite` 与
`futures-util` 由 axum 的 `ws` 功能传递进入正常生产依赖树。因此“直接开发依赖”只描述
本 crate 的 `Cargo.toml` 声明位置，不表示它们不会进入生产构建。

实施时以 `Cargo.lock` 的实际解析结果为准，并运行现有 `cargo deny` 门禁。noVNC 的 MPL-2.0 资源不在本阶段进入仓库或产物，因此本阶段不触发 noVNC 分发边界。

## 5. 已比较方案

### 5.1 方案 A：抽取共享异步连接驱动，TCP 与 WebSocket 各自适配

共享驱动器持有 RFB 核心、视频接收器、帧请求合并器和事件发送器。传输适配器只提供收取字节、发送二进制字节和关闭三个能力。

优点：

- TCP 和 WebSocket 具有同一份状态机与错误顺序。
- noVNC 与原生 VNC 客户端共享同一协议实现。
- 可以直接复用已有 TCP 行为测试验证抽取没有回归。
- 传输适配器足够小，可分别进行确定性测试。

缺点：

- 需要一次有控制的公共类型重命名。
- Rust 异步传输接口需要保持静态泛型，避免不必要的装箱和 `async-trait` 依赖。

### 5.2 方案 B：为 WebSocket 复制现有 TCP 连接循环

优点：

- 首次改动表面上较少。

缺点：

- 握手超时、帧序号、请求合并、事件顺序、尺寸调整和反压立即出现两份实现。
- 后续修复必须同步两个循环，极易产生协议行为分叉。
- 与“从根因修复、禁止补丁式扩展”的项目规范冲突。

### 5.3 方案 C：WebSocket 到本机 TCP 的内部代理

优点：

- 不改现有 TCP 连接循环。

缺点：

- 增加内部端口、第二份缓冲、额外任务和关闭竞态。
- 无法自然统一客户端标识、事件通道和单活动连接状态。
- 测试只能间接观察错误，诊断能力较差。

### 5.4 结论

采用方案 A。当前 TCP 驱动已经证明共享逻辑具有稳定边界，继续把它留在 `rfb_tcp` 或复制到 WebSocket 都会固化错误的所有权关系。

## 6. 模块边界

### 6.1 `rfb_connection`

新增 `crates/ipkvm-headless/src/rfb_connection/`，负责：

- 传输无关公共配置。
- 客户端标识、应用事件和断开原因。
- TCP 与 WebSocket 共用的单活动连接许可和客户端标识分配。
- 视频帧到 RFB framebuffer 的适配。
- 未完成帧缓冲区请求的有界合并。
- RFB 核心、帧接收器、序号和握手超时状态。
- 把 RFB 核心事件按顺序发送到有界应用事件通道。
- 通用异步连接循环。

建议文件：

```text
rfb_connection/
  mod.rs
  gate.rs
  driver.rs
  frame.rs
  pending.rs
  transport.rs
```

`transport.rs` 中的接口保持 crate 内私有。它不成为库的长期公共 API。

### 6.2 `rfb_tcp`

保留：

- `RfbTcpConfig` 与 TCP 读取块大小校验。
- `TcpStream` 到共享传输接口的适配。
- `TcpListener` 接受连接循环。
- 顺序服务客户端和 TCP 服务端级错误。

移出：

- RFB 核心状态。
- 帧适配和请求合并。
- 公共 RFB 事件与断开原因。

### 6.3 `rfb_ws`

新增 `crates/ipkvm-headless/src/rfb_ws/`，负责：

- `RfbWebSocketConfig`。
- `/rfb` axum 路由。
- 可组合的 `RfbWebSocketService<S>`。
- WebSocket 升级参数和可选 `binary` 子协议。
- 单活动连接许可。
- 客户端标识分配。
- WebSocket 消息到共享传输接口的适配。
- HTTP 层拒绝状态。

建议文件：

```text
rfb_ws/
  mod.rs
  service.rs
  transport.rs
```

### 6.4 `rfb_input`

输入事件泵改为消费共享的 `RfbServerEvent`。键盘、指针、剪贴板、控制者生命周期和 `release_all` 行为不变。

## 7. 公共类型

### 7.1 共享连接配置

```rust
pub struct RfbConnectionSettings {
    pub desktop_name: String,
    pub handshake_timeout: Duration,
    pub protocol_limits: RfbProtocolLimits,
}
```

默认值：

- `desktop_name = "my_ipkvm"`
- `handshake_timeout = 10 秒`
- `protocol_limits = RfbProtocolLimits::default()`

`validate` 至少拒绝零握手超时。RFB 核心继续负责桌面名和协议上限之间的完整一致性校验。

### 7.2 TCP 配置

```rust
pub struct RfbTcpConfig {
    pub connection: RfbConnectionSettings,
    pub read_buffer_bytes: usize,
}
```

默认读取块为 16 KiB。读取块必须大于零，且不得超过
`connection.protocol_limits.max_buffered_input_bytes`。

### 7.3 WebSocket 配置

```rust
pub struct RfbWebSocketConfig {
    pub connection: RfbConnectionSettings,
}
```

单条 WebSocket 消息和单个 WebSocket 帧的最大值均使用
`connection.protocol_limits.max_buffered_input_bytes`，不维护第二个含义重复的上限。

### 7.4 共享事件

`RfbTcpEvent` 重命名为 `RfbServerEvent`，字段和顺序语义保持不变：

```rust
pub enum RfbServerEvent {
    Connected { client_id, peer_addr, shared },
    Key { client_id, down, keysym },
    Pointer {
        client_id,
        button_mask,
        x,
        y,
        framebuffer_size,
    },
    CutText { client_id, bytes },
    ContinuousUpdates { client_id, enable, rectangle },
    Disconnected { client_id, peer_addr, reason },
}
```

公共 `RfbClientId` 和 `RfbDisconnectReason` 一并移动到
`rfb_connection`。不保留旧名称别名，以免继续向调用方暴露错误的
TCP 所有权关系。

### 7.5 全局连接许可

```rust
#[derive(Clone)]
pub struct RfbConnectionGate { /* 私有共享状态 */ }
```

上层在组装 TCP 与 WebSocket 入口时创建一个连接闸门，并把副本分别传给
`RfbTcpServer` 和 `RfbWebSocketService`。连接闸门内部持有：

- 容量为 1 的 `Semaphore`。
- 一个 `AtomicU64` 下一个客户端标识；`0` 是标识空间已经耗尽的哨兵值。

取得闸门时同时分配 `RfbClientId`。返回的私有预约持有信号量许可；预约在 HTTP 升级失败
或进入连接驱动前被取消时可以普通析构并释放。共享 owner 在第一个 `.await` 前把预约激活
为不可克隆的租约，并取消信号量的自动归还。租约只有在共享收尾函数把对应
`Disconnected` 成功送入事件队列后才同步显式释放。

已激活 owner、连接驱动或共享收尾被取消，或者事件接收端关闭时，租约析构会关闭
semaphore 并把闸门置为 `Poisoned`。等待中的 TCP 任务被唤醒并返回
`RfbTcpServerError::ConnectionGatePoisoned`，新的 WebSocket 升级返回空 `503`；上层必须
重启服务实例，不能在旧输入状态未确认释放时继续接纳控制者。客户端标识从 1 开始，使用
`checked_add`，永不回绕或复用。

不提供“每个入口自动创建独立连接闸门”的便捷构造，因为那会允许 TCP 和 WebSocket
同时成为控制者。所有生产组装和测试必须显式传入连接闸门。

### 7.6 共享错误

`RfbTcpFrameError` 重命名为 `RfbFrameError`。

`RfbDisconnectReason` 保留现有分类，并增加：

```rust
WebSocket,
UnexpectedTextMessage,
```

规则：

- TCP `std::io::Error` 继续降级为 `Io(ErrorKind)`。
- axum/tungstenite 的不可克隆错误保留在私有 `RfbTransportError` 错误链中，对外降级为
  稳定的 `WebSocket` 分类。
- 收到 Text 消息使用独立的 `UnexpectedTextMessage`，便于区分应用层误用和底层连接故障。
- `Ping`、`Pong` 和正常 `Close` 不属于错误。
- 事件接收端已关闭时无法再可靠发送 `Disconnected`，因此不制造虚假的断开事件。

`RfbConnectionSettingsError`、`RfbConnectionGateError`、`RfbTcpConfigError`、
`RfbTcpServerError` 和 `RfbWebSocketServiceError` 分别保留各层可处理的错误，
不得用字符串替代类型化分类。

## 8. 私有传输接口

共享驱动使用静态泛型私有接口，概念签名如下：

```rust
trait RfbTransport {
    async fn receive_into(
        &mut self,
        buffer: &mut Vec<u8>,
    ) -> Result<RfbTransportRead, RfbTransportError>;

    async fn send_binary(
        &mut self,
        bytes: Vec<u8>,
    ) -> Result<(), RfbTransportError>;

    async fn close(&mut self);
}

enum RfbTransportRead {
    Data,
    Continue,
    Closed,
}
```

约束：

- `Data` 时 `buffer` 必须非空。
- `Continue` 表示已消费不属于 RFB 的控制消息，驱动继续等待。
- `Closed` 表示客户端正常关闭。
- 传输实现每次接收前清空 `buffer`，不得把旧数据重复交给核心。
- 驱动器忽略传输层消息边界，每次 `Data` 都调用同一个
  `RfbConnectionCore::push_input`。
- 生产代码不引入 `async-trait`，不使用动态 trait object。

TCP 适配器把读取块大小保存在自身，复用驱动提供的 `Vec<u8>`；读取零字节返回
`Closed`。WebSocket 适配器把 `Binary` 追加到同一个 `Vec<u8>`，`Ping`/`Pong` 返回
`Continue`，`Close` 或流结束返回 `Closed`，`Text` 返回类型化错误。

## 9. 共享连接驱动

### 9.1 初始化

驱动接收：

- `RfbClientId`
- `SocketAddr`
- 一个实现私有传输接口的值
- 当前连接自己的 `FrameReceiver`
- `mpsc::Sender<RfbServerEvent>`
- `RfbConnectionSettings`
- `watch::Receiver<bool>` 关闭信号

初始化顺序保持现有 TCP 行为：

1. 启动时已收到关闭信号则直接结束。
2. 读取并校验当前视频帧；没有帧时不发送虚假帧缓冲区。
3. 用当前尺寸创建 `RfbConnectionCore`。
4. 立即发送 RFB 握手横幅。
5. 启动握手截止时间。

### 9.2 主循环

主循环同时等待：

- 传输层输入。
- 存在待处理增量请求时的视频帧变化。
- 关闭信号。
- 事件接收端关闭。
- 握手未完成时的截止时间。

每次输入处理顺序：

1. `push_input`。
2. 立即发送核心已产生的输出。
3. 按核心返回顺序处理事件。
4. 更新请求在驱动器内合并和响应。
5. 其他事件通过有界 mpsc 无损发送。
6. 再次发送事件处理期间产生的核心输出。

同一输入批次中，协议错误之前已经完成的有效事件必须先交付。这一行为与现有
TCP 驱动一致。

### 9.3 帧请求和尺寸调整

现有行为原样迁移：

- 非增量请求即使帧序号未变化也发送。
- 增量请求等待更大的帧序号。
- 多个增量请求合并成一个常量空间的最小外接矩形。
- 帧序号倒退断开连接。
- 已协商 `DesktopSize` 时先发送尺寸调整，下一请求再发新尺寸像素。
- 未协商 `DesktopSize` 时返回编码错误并断开。
- 不发送 unsolicited framebuffer update。

## 10. TCP 行为保持

`RfbTcpServer` 仍然顺序接受连接，并在开始 RFB 连接前取得共享连接闸门：

- 当前客户端结束前，积压队列中的第二个 TCP 客户端不收到 RFB 握手横幅。
- 如果 WebSocket 正在活动，已接受的 TCP 客户端等待连接闸门，等待期间不收到
  RFB 握手横幅，握手超时也尚未开始。
- 等待连接闸门时同时监听关闭信号和事件接收端关闭。
- 取得连接闸门后分配全局唯一、不可回绕的客户端标识。
- 普通连接错误只关闭当前连接并发送一次 `Disconnected`。
- 接受连接失败、客户端标识耗尽、闸门中毒和事件接收端关闭仍是服务端级错误。
- 关闭信号结束当前连接后停止接受连接。
- `Disconnected` 成功入队后才释放连接闸门；输入泵因此总能在下一连接取得许可前处理完
  前一连接的生命周期事件。

现有回环 TCP 集成测试是抽取重构的回归门禁。除公共类型重命名和配置嵌套外，
不得修改测试所断言的线级行为。

## 11. WebSocket 服务

### 11.1 可组合路由

```rust
pub struct RfbWebSocketService<S> { /* 私有状态 */ }

impl<S: FrameSource + 'static> RfbWebSocketService<S> {
    pub fn new(
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbServerEvent>,
        config: RfbWebSocketConfig,
        shutdown: watch::Receiver<bool>,
        gate: RfbConnectionGate,
    ) -> Result<Self, RfbWebSocketServiceError>;

    pub fn router(&self) -> Router;
}
```

`router()` 返回包含 `/rfb` 的 axum `Router`，供后续与静态资源、设置 API 和健康检查
合并。实际监听器由上层持有：

```rust
axum::serve(
    listener,
    service
        .router()
        .into_make_service_with_connect_info::<SocketAddr>(),
)
```

`ConnectInfo<SocketAddr>` 是事件中 `peer_addr` 的来源。上层未按该方式提供连接信息时，
升级请求应被 axum 拒绝，而不是伪造地址。

### 11.2 升级规则

`/rfb` 使用 axum WebSocket extractor，并配置：

- 支持 HTTP/1.1 GET WebSocket 升级。
- `protocols(["binary"])`。
- `max_message_size(max_buffered_input_bytes)`。
- `max_frame_size(max_buffered_input_bytes)`。

响应语义：

- 无子协议请求：`101 Switching Protocols`，响应不含
  `Sec-WebSocket-Protocol`。
- 请求包含 `binary`：`101 Switching Protocols`，响应选择 `binary`。
- 已收到关闭信号：`503 Service Unavailable`。
- 事件接收端已关闭：`503 Service Unavailable`。
- 已有活动连接：`409 Conflict`。
- 连接闸门中的客户端标识已耗尽：`503 Service Unavailable`，且永不回绕。

HTTP 拒绝没有 RFB 客户端标识，不产生 `Connected` 或 `Disconnected`。

### 11.3 全局单活动连接

WebSocket 服务使用上层传入的共享 `RfbConnectionGate`：

1. 升级处理器调用连接闸门的 `try_acquire()`；连接闸门内部使用拥有所有权的信号量许可。
2. 未取得许可时立即返回 `409`，不等待当前客户端。
3. 成功取得许可时由连接闸门分配客户端标识。
4. 未激活预约移动到 `on_upgrade` 任务；升级失败或回调首次 poll 前取消会自动释放预约。
5. 回调首次 poll 时在任何 `.await` 前激活租约，覆盖完整 WebSocket/RFB 生命周期。
6. 连接驱动结束并完成断开事件发送后，由共享收尾同步释放租约。
7. 已激活任务异常消失或断开事件无法入队时毒化闸门，后续升级返回 `503`。
8. 无论当前活动连接来自 TCP 还是 WebSocket，新的 WebSocket 请求都得到 `409`。
9. 当前连接正常释放后，后续 WebSocket 请求或已接受并等待的 TCP 客户端可以取得许可。

预约在 HTTP 升级期间已经占用闸门，避免两个并发升级都进入 RFB 握手。

### 11.4 客户端标识

客户端标识由共享连接闸门分配：

- 从 1 开始。
- TCP 与 WebSocket 使用同一个序列。
- 每个获得单连接许可并进入升级流程的请求分配一次。
- 使用 compare-exchange 循环和 `checked_add` 分配；`u64::MAX` 被成功分配后把原子值更新
  为 `0`。
- 分配过程不使用锁，也不执行 `.await`。
- HTTP 升级失败可以留下标识空洞，但标识永不复用。

### 11.5 生命周期

WebSocket 升级成功后：

1. 为连接订阅独立 `FrameReceiver`。
2. 共享 owner 在第一个 `.await` 前激活预约，再调用共享连接驱动。
3. 共享驱动器返回前调用传输层的 `close()`；WebSocket 传输层尝试发送 `Close` 帧。
4. 共享收尾在事件通道仍可用时发送且只发送一次 `Disconnected`。
5. `Disconnected` 成功入队后同步释放租约；收尾取消或发送失败会毒化闸门。

`Close` 帧或 WebSocket 流正常结束映射为 `ClientClosed`。关闭信号映射为
`ServerShutdown`。底层协议错误映射为 `WebSocket`。收到 Text 映射为
`UnexpectedTextMessage`。

## 12. 反压和内存边界

### 12.1 输入

- axum 在消息和帧层拒绝超过协议输入上限的数据。
- WebSocket 适配器最多复制当前一条 Binary 消息到共享接收 `Vec<u8>`。
- 核心的 `max_buffered_input_bytes` 继续限制跨消息保留的 RFB 半包。
- 事件通道满时共享驱动器停止读取传输层，反压传播到 WebSocket/TCP。
- 不使用 `try_send` 丢弃键盘释放、鼠标释放或生命周期事件。

### 12.2 输出

- 核心的 `max_queued_output_bytes` 限制一条待发送 RFB 输出。
- 每次 `take_output` 后立即 `send_binary(...).await`。
- TCP 使用 `write_all`，WebSocket 使用单条 Binary 消息。
- 不拆分 WebSocket，不创建独立写入任务，不增加无界发送通道。
- 慢客户端使当前连接停在发送点；视频 `watch` 接收器仍只保留最新帧。

WebSocket 消息边界只是一种承载：服务端可以把一次核心输出放在一条 Binary 消息中，
客户端输入可以任意拆成多条 Binary 消息。

## 13. noVNC 1.7.0 兼容样本

集成测试保存一份由固定提交源码推导出的中文注释线级测试样本，不复制 noVNC 源文件。

测试场景：

1. 不携带子协议建立 `/rfb` WebSocket。
2. 完成 RFB 3.8、SecurityType None 握手。
3. 校验 `ServerInit` 尺寸和名称。
4. 按 noVNC 1.7.0 发送 20 字节 `SetPixelFormat`：
   32 bpp、24 depth、小端、true color、三个 max 为 255、shift 为 0/8/16。
5. 发送 noVNC 无 WebCodecs H.264 能力时的 `SetEncodings` 顺序，其中同时包含服务端
   不实现的编码、`Raw` 和 `DesktopSize`。
6. 发送覆盖完整 framebuffer 的非增量更新请求。
7. 校验服务端选择 `Raw`，并把输入 BGRA 像素转换为 noVNC 所需的 `R, G, B, 0`。
8. 发送后续增量请求，发布新帧，校验更新继续工作。

H.264 是否进入 noVNC 编码列表取决于浏览器 WebCodecs 能力。本阶段测试样本使用没有
H.264 的确定性分支；协议核心对未知编码的既有测试以及下一阶段真实浏览器测试覆盖
带 H.264 的分支。

## 14. 自动化测试实施结果

### 14.1 共享配置、帧适配和连接闸门

- `RfbConnectionSettings` 默认值和零超时校验。
- `RfbTcpConfig` 默认读取块、零读取块和超上限读取块校验。
- `RfbWebSocketConfig` 继承共享连接配置校验。
- 帧适配覆盖像素格式、尺寸、行跨度、长度和帧序号。
- `RfbConnectionGate` 覆盖 `Default` 与 `new()` 的等价初始状态、单预约、容量循环守恒、
  未激活析构、显式释放、已激活异常析构中毒并唤醒等待者，以及 `u64::MAX` 仅分配一次且
  不回绕。
- 共享收尾覆盖满事件通道反压、成功入队后释放、等待期间取消、无断开原因和接收端已关闭；
  失败路径均保持无虚假事件并毒化闸门。
- 真实 TCP owner 在完成握手并产生 `Connected` 后被中止，闸门确定性中毒；后续 TCP 服务
  返回类型化 `ConnectionGatePoisoned`。

### 14.2 共享连接驱动

- 分片握手产生一次 `Connected`。
- 握手超时使用 Tokio 暂停时钟验证，不依赖真实短暂休眠。
- 同一批次中有效输入事件先于后续协议错误交付；键盘、指针、剪贴板和连续更新事件不重排。
- 非增量更新、增量等待、请求合并和动态尺寸行为保持不变。
- 关闭信号、帧错误、协议错误和事件通道关闭得到确定结果；输出发送失败映射为对应传输层错误。
- 传输层控制消息的 `Continue` 不进入 RFB 核心。

### 14.3 TCP 回归：`crates/ipkvm-headless/tests/rfb_tcp.rs`

当前文件有 8 个测试，覆盖事件接收端关闭、初始帧缺失或无效后的重连、协商 RGB565、
有界事件通道反压、关闭信号、协议错误后继续服务，以及第二个 TCP 客户端在首个连接
断开后才收到 RFB 握手横幅。

### 14.4 WebSocket 集成：`crates/ipkvm-headless/tests/rfb_websocket.rs`

当前文件有 16 个测试，使用真实 `127.0.0.1:0` 监听器、axum 服务端和
`tokio-tungstenite` 客户端，不模拟 HTTP 升级：

1. 未请求子协议时升级成功且响应不选择子协议。
2. 仅在请求包含 `binary` 时选择 `binary`。
3. 分散在多个 Binary 消息中的逐字节 RFB 输入可以完成握手。
4. 服务端所有 RFB 输出均为 Binary 消息。
5. noVNC 1.7.0 无 H.264 线级样本得到初始和增量 `Raw` 更新。
6. 一条 Binary 消息中的多个 RFB 事件保持顺序。
7. `Ping`/`Pong` 不污染 RFB 输入，连接可继续完成握手。
8. Text 消息断开一次并产生 `UnexpectedTextMessage`。
9. `Close` 消息断开一次并产生 `ClientClosed`。
10. 活动连接存在时第二次升级返回 `409`。
11. `Disconnected` 事件成功入队后连接闸门才重新开放。
12. 关闭信号在升级前返回空 `503` 响应。
13. 关闭信号结束活动连接并产生 `ServerShutdown`。
14. 事件接收端关闭使后续升级返回 `503`。
15. 超过输入上限的 WebSocket 消息以 `WebSocket` 原因断开。
16. 无关子协议不会在响应中回显。

### 14.5 跨传输层排他性：`crates/ipkvm-headless/tests/rfb_transport_exclusion.rs`

当前文件有 2 个测试：TCP 活动时 WebSocket 升级返回 `409`；WebSocket 活动时 TCP
客户端在 `Disconnected` 入队并释放共享连接闸门前不收到 RFB 握手横幅。

### 14.6 RFB 输入泵：`crates/ipkvm-headless/tests/rfb_input_pump.rs`

当前文件有 2 个测试，覆盖公共输入泵契约，以及真实 TCP 客户端驱动 CH9329 输入并在
断开时释放输入状态。

### 14.7 命令级验证

```powershell
.\scripts\verify.ps1
```

该命令覆盖格式、全工作区测试、Clippy、Rust 文档、依赖许可证和来源审计，以及工作区和暂存区 diff 检查。本阶段没有人工验证例外。

## 15. 文档收口

- `README.md` 记录已有原生 TCP 与 WebSocket 两种 RFB 入口，但不得声称完整网页已完成。
- `docs/ipkvm-coarse-design.md` 标记共享连接驱动器和 WebSocket 传输层已完成。
- 长期文档使用 `RfbServerEvent` 和 `RfbFrameError` 新名称。
- 当前公共 API、依赖和测试范围以本文与代码为准；历史设计文档不回写为当前事实。

## 16. 风险与排除措施

### 16.1 抽取导致 TCP 行为漂移

措施：先把公共类型和共享状态迁移到不改变行为的测试通过提交，再增加 WebSocket；
现有真实 TCP 回环测试作为每一步门禁。

### 16.2 WebSocket 消息边界被误当作 RFB 包边界

措施：共享驱动只接收连续字节；集成测试把握手和客户端消息逐字节拆到不同 Binary
消息。

### 16.3 noVNC 默认子协议假设错误

措施：默认无子协议是固定兼容条件；同时测试无子协议和显式 `binary` 两种升级。

### 16.4 多连接绕过输入单控制者

措施：TCP 与 WebSocket 必须显式使用同一个连接闸门；共享 owner 把预约激活为关闭失败
的租约，只有共享收尾在 `Disconnected` 入队后才能释放。异常取消毒化闸门，不会接纳第二
个控制者。并发真实连接测试同时覆盖 TCP/TCP、WS/WS、TCP/WS 和 WS/TCP，确定性单元测试
覆盖取消与中毒。

### 16.5 慢客户端形成无界输出

措施：不拆分写入器、不建立输出通道；每次核心输出只在一次等待发送中
存在，视频源继续使用最新值 `watch`。

### 16.6 关闭信号与断开事件竞态

措施：共享驱动器只返回一个 `ConnectionEnd`，共享收尾统一发送一次 `Disconnected` 并
同步释放租约；发送等待期间的 future 取消会毒化闸门。测试关闭信号、`Close`、错误、
事件通道关闭和收尾取消路径。

### 16.7 WebSocket 库错误不可稳定比较

措施：对外只暴露稳定错误分类，底层错误保留在私有 `Error::source()` 链中但不进入可克隆
事件；Text 单独分类，其他 WebSocket 故障统一为 `WebSocket`。

### 16.8 新依赖扩大许可证或来源风险

措施：锁定精确兼容版本，先跑许可证负例与当前锁文件审计，再运行全量验证；任何新出现
的未许可标识或非 crates.io 来源都阻止合并。

## 17. 资料链接

- noVNC 1.7.0 发布页：
  <https://github.com/novnc/noVNC/releases/tag/v1.7.0>
- noVNC 固定提交的 WebSocket 实现：
  <https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/core/websock.js>
- noVNC 固定提交的 RFB 初始化与消息实现：
  <https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/core/rfb.js>
- noVNC 固定提交的编码常量：
  <https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/core/encodings.js>
- noVNC API 文档：
  <https://novnc.com/noVNC/docs/API.html>
- axum 0.8.9 WebSocket 模块：
  <https://docs.rs/axum/0.8.9/axum/extract/ws/>
- axum 0.8.9 `WebSocketUpgrade`：
  <https://docs.rs/axum/0.8.9/axum/extract/ws/struct.WebSocketUpgrade.html>
- RFC 6455 WebSocket：
  <https://www.rfc-editor.org/rfc/rfc6455>
- RFC 6143 RFB：
  <https://www.rfc-editor.org/rfc/rfc6143>
