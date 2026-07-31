# 共享 RFB 连接驱动与 WebSocket 传输设计

## 1. 文档状态

- 关联 issue：`#15`
- 状态：已批准，待实施
- 适用阶段：无头版 WebSocket 网络闭环
- 前置依赖：`#5` 已完成的 RFB TCP 传输、`#11` 已完成的 RFB 输入事件泵、`#13` 已完成的依赖许可证门禁
- 后续阶段：noVNC 静态资源集成与真实浏览器闭环

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

采用 axum 0.8.9，并只启用 `http1`、`tokio` 和 `ws` feature：

- `WebSocketUpgrade::protocols(["binary"])` 只会在客户端提供该值时选择它；未提供子协议的客户端仍可升级。
- `max_message_size` 和 `max_frame_size` 可在 upgrade 前限制单条输入。
- axum/tungstenite 会处理 WebSocket 分片，并把完整消息交给应用层。
- Ping/Pong 是控制消息，不属于 RFB 字节流；连接驱动忽略它们。
- `send(Message::Binary(...)).await` 是输出反压点，不增加应用层无界发送队列。

### 4.3 依赖与许可证

生产依赖：

- `axum = 0.8.9`，MIT。

测试依赖：

- `tokio-tungstenite = 0.29.0`，MIT，与 axum 0.8.9 的依赖版本保持一致。
- `futures-util = 0.3.33`，MIT 或 Apache-2.0，只用于 WebSocket 测试客户端的 `SinkExt` 和 `StreamExt`。

实施时以 `Cargo.lock` 的实际解析结果为准，并运行现有 `cargo deny` 门禁。noVNC 的 MPL-2.0 资源不在本阶段进入仓库或产物，因此本阶段不触发 noVNC 分发边界。

## 5. 已比较方案

### 5.1 方案 A：抽取共享异步连接驱动，TCP 与 WebSocket 各自适配

共享驱动持有 RFB core、视频 receiver、帧请求合并器和事件发送器。传输适配器只提供收取字节、发送二进制字节和关闭三个能力。

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

- 握手超时、帧序号、请求合并、事件顺序、resize 和反压立即出现两份实现。
- 后续修复必须同步两个循环，极易产生协议行为分叉。
- 与“从根因修复、禁止补丁式扩展”的项目规范冲突。

### 5.3 方案 C：WebSocket 到本机 TCP 的内部代理

优点：

- 不改现有 TCP 连接循环。

缺点：

- 增加内部端口、第二份缓冲、额外任务和关闭竞态。
- 无法自然统一 client id、事件通道和单活动连接状态。
- 测试只能间接观察错误，诊断能力较差。

### 5.4 结论

采用方案 A。当前 TCP 驱动已经证明共享逻辑具有稳定边界，继续把它留在 `rfb_tcp` 或复制到 WebSocket 都会固化错误的所有权关系。

## 6. 模块边界

### 6.1 `rfb_connection`

新增 `crates/ipkvm-headless/src/rfb_connection/`，负责：

- 传输无关公共配置。
- client id、应用事件和断开原因。
- TCP 与 WebSocket 共用的单活动连接许可和 client id 分配。
- 视频帧到 RFB framebuffer 的适配。
- outstanding framebuffer request 的有界合并。
- RFB core、帧 receiver、序号和握手超时状态。
- 把 RFB core 事件按顺序发送到有界应用事件通道。
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
- `TcpListener` accept 循环。
- 顺序服务客户端和 TCP server 级错误。

移出：

- RFB core 状态。
- 帧适配和请求合并。
- 公共 RFB 事件与断开原因。

### 6.3 `rfb_ws`

新增 `crates/ipkvm-headless/src/rfb_ws/`，负责：

- `RfbWebSocketConfig`。
- `/rfb` axum route。
- 可组合的 `RfbWebSocketService<S>`。
- WebSocket upgrade 参数和可选 `binary` 子协议。
- 单活动连接许可。
- client id 分配。
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

`validate` 至少拒绝零握手超时。RFB core 继续负责桌面名和协议上限之间的完整一致性校验。

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

单条 WebSocket 消息和单个 WebSocket frame 的最大值均使用
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
#[derive(Clone, Debug)]
pub struct RfbConnectionGate { /* 私有共享状态 */ }
```

上层在组装 TCP 与 WebSocket 入口时创建一个 gate，并把 clone 分别传给
`RfbTcpServer` 和 `RfbWebSocketService`。gate 内部持有：

- 容量为 1 的 `Semaphore`。
- 一个 `AtomicU64` 下一个 client id；`0` 是 id 空间已经耗尽的哨兵值。

取得许可时同时分配 `RfbClientId`。返回的私有 permit 持有 semaphore permit 和
client id，直到对应 `Disconnected` 成功进入事件队列后才释放。client id 从 1 开始，
使用 `checked_add`，永不回绕或复用。

不提供“每个入口自动创建独立 gate”的便捷构造，因为那会允许 TCP 和 WebSocket
同时成为控制者。所有生产组装和测试必须显式传入 gate。

### 7.6 共享错误

`RfbTcpFrameError` 重命名为 `RfbFrameError`。

`RfbDisconnectReason` 保留现有分类，并增加：

```rust
WebSocket,
UnexpectedTextMessage,
```

规则：

- TCP `std::io::Error` 继续降级为 `Io(ErrorKind)`。
- axum/tungstenite 的不可克隆错误降级为稳定的 `WebSocket` 分类。
- 收到 Text 消息使用独立的 `UnexpectedTextMessage`，便于区分应用层误用和底层连接故障。
- Ping、Pong 和正常 Close 不属于错误。
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
- 传输实现每次接收前清空 `buffer`，不得把旧数据重复交给 core。
- 驱动忽略 transport 消息边界，每次 `Data` 都调用同一个
  `RfbConnectionCore::push_input`。
- 生产代码不引入 `async-trait`，不使用动态 trait object。

TCP 适配器把读取块大小保存在自身，复用驱动提供的 `Vec<u8>`；读取零字节返回
`Closed`。WebSocket 适配器把 Binary 追加到同一个 `Vec<u8>`，Ping/Pong 返回
`Continue`，Close 或流结束返回 `Closed`，Text 返回类型化错误。

## 9. 共享连接驱动

### 9.1 初始化

驱动接收：

- `RfbClientId`
- `SocketAddr`
- 一个实现私有传输接口的值
- 当前连接自己的 `FrameReceiver`
- `mpsc::Sender<RfbServerEvent>`
- `RfbConnectionSettings`
- `watch::Receiver<bool>` shutdown

初始化顺序保持现有 TCP 行为：

1. 启动时已 shutdown 则直接结束。
2. 读取并校验当前视频帧；没有帧时不发送虚假 framebuffer。
3. 用当前尺寸创建 `RfbConnectionCore`。
4. 立即发送 RFB banner。
5. 启动握手截止时间。

### 9.2 主循环

主循环同时等待：

- transport 输入。
- 存在 pending 增量请求时的视频帧变化。
- shutdown。
- 事件接收端关闭。
- 握手未完成时的截止时间。

每次输入处理顺序：

1. `push_input`。
2. 立即发送 core 已产生的输出。
3. 按 core 返回顺序处理事件。
4. update request 在驱动内合并和响应。
5. 其他事件通过有界 mpsc 无损发送。
6. 再次发送事件处理期间产生的 core 输出。

同一输入批次中，协议错误之前已经完成的有效事件必须先交付。这一行为与现有
TCP 驱动一致。

### 9.3 帧请求和 resize

现有行为原样迁移：

- 非增量请求即使帧序号未变化也发送。
- 增量请求等待更大的帧序号。
- 多个增量请求合并成一个常量空间的最小外接矩形。
- 帧序号倒退断开连接。
- 已协商 `DesktopSize` 时先发送 resize，下一请求再发新尺寸像素。
- 未协商 `DesktopSize` 时返回编码错误并断开。
- 不发送 unsolicited framebuffer update。

## 10. TCP 行为保持

`RfbTcpServer` 仍然顺序 accept，并在开始 RFB 连接前取得共享 gate：

- 当前客户端结束前，backlog 中的第二个 TCP 客户端不收到 RFB banner。
- 如果 WebSocket 正在活动，已经 accept 的 TCP 客户端等待 gate，等待期间不收到
  RFB banner，握手超时也尚未开始。
- 等待 gate 时同时监听 shutdown 和事件接收端关闭。
- 取得 gate 后分配全局唯一、不可回绕的 id。
- 普通连接错误只关闭当前连接并发送一次 `Disconnected`。
- accept 失败、client id 耗尽和事件接收端关闭仍是 server 级错误。
- shutdown 结束当前连接后停止 accept。
- `Disconnected` 成功入队后才释放 gate；输入泵因此总能在下一连接取得许可前处理完
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
upgrade 请求应被 axum 拒绝，而不是伪造地址。

### 11.2 upgrade 规则

`/rfb` 使用 axum WebSocket extractor，并配置：

- 支持 HTTP/1.1 GET WebSocket upgrade。
- `protocols(["binary"])`。
- `max_message_size(max_buffered_input_bytes)`。
- `max_frame_size(max_buffered_input_bytes)`。

响应语义：

- 无子协议请求：`101 Switching Protocols`，响应不含
  `Sec-WebSocket-Protocol`。
- 请求包含 `binary`：`101 Switching Protocols`，响应选择 `binary`。
- shutdown 已请求：`503 Service Unavailable`。
- 事件接收端已关闭：`503 Service Unavailable`。
- 已有活动连接：`409 Conflict`。
- gate 中的 client id 已耗尽：`503 Service Unavailable`，且永不回绕。

HTTP 拒绝没有 RFB client id，不产生 `Connected` 或 `Disconnected`。

### 11.3 全局单活动连接

WebSocket 服务使用上层传入的共享 `RfbConnectionGate`：

1. upgrade handler 通过 gate 使用 `try_acquire_owned`。
2. 未取得许可时立即返回 `409`，不等待当前客户端。
3. 成功取得许可时由 gate 分配 client id。
4. 许可移动到 `on_upgrade` 任务，覆盖完整 WebSocket/RFB 生命周期。
5. 连接驱动结束并完成断开事件发送后释放许可。
6. 无论当前活动连接来自 TCP 还是 WebSocket，新的 WebSocket 请求都得到 `409`。
7. 当前连接释放后，后续 WebSocket 请求或已经 accept 并等待的 TCP 客户端可以取得许可。

许可在 HTTP upgrade 期间已经占用，避免两个并发 upgrade 都进入 RFB 握手。

### 11.4 client id

client id 由共享 gate 分配：

- 从 1 开始。
- TCP 与 WebSocket 使用同一个序列。
- 每个获得单连接许可并进入 upgrade 流程的请求分配一次。
- 使用 compare-exchange 循环和 `checked_add` 分配；`u64::MAX` 被成功分配后把原子值更新
  为 `0`。
- 分配过程不使用锁，也不执行 `.await`。
- HTTP upgrade 失败可以留下 id 空洞，但 id 永不复用。

### 11.5 生命周期

WebSocket upgrade 成功后：

1. 为连接订阅独立 `FrameReceiver`。
2. 调用共享连接驱动。
3. 驱动返回后尝试发送 WebSocket Close。
4. 事件通道仍可用时发送且只发送一次 `Disconnected`。
5. `Disconnected` 成功入队后释放单连接许可。

Close frame 或 WebSocket 流正常结束映射为 `ClientClosed`。shutdown 映射为
`ServerShutdown`。底层协议错误映射为 `WebSocket`。收到 Text 映射为
`UnexpectedTextMessage`。

## 12. 反压和内存边界

### 12.1 输入

- axum 在消息和 frame 层拒绝超过协议输入上限的数据。
- WebSocket 适配器最多复制当前一条 Binary 消息到共享接收 `Vec<u8>`。
- core 的 `max_buffered_input_bytes` 继续限制跨消息保留的 RFB 半包。
- 事件通道满时共享驱动停止读取 transport，反压传播到 WebSocket/TCP。
- 不使用 `try_send` 丢弃键盘释放、鼠标释放或生命周期事件。

### 12.2 输出

- core 的 `max_queued_output_bytes` 限制一条待发送 RFB 输出。
- 每次 `take_output` 后立即 `send_binary(...).await`。
- TCP 使用 `write_all`，WebSocket 使用单条 Binary 消息。
- 不 split WebSocket，不创建独立 writer task，不增加无界发送 channel。
- 慢客户端使当前连接停在发送点；视频 watch receiver 仍只保留最新帧。

WebSocket 消息边界只是一种承载：服务端可以把一次 core 输出放在一条 Binary 消息中，
客户端输入可以任意拆成多条 Binary 消息。

## 13. noVNC 1.7.0 兼容样本

集成测试保存一份由固定提交源码推导出的中文注释线级 fixture，不复制 noVNC 源文件。

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

H.264 是否进入 noVNC 编码列表取决于浏览器 WebCodecs 能力。本阶段 fixture 使用没有
H.264 的确定性分支；协议 core 对未知编码的既有测试以及下一阶段真实浏览器测试覆盖
带 H.264 的分支。

## 14. 自动化测试设计

### 14.1 共享配置和帧适配

- `RfbConnectionSettings` 默认值和零超时校验。
- `RfbTcpConfig` 默认读取块、零读取块和超上限读取块校验。
- `RfbWebSocketConfig` 继承共享连接校验。
- 原有格式、尺寸、stride、长度和帧序号测试迁移后全部通过。

### 14.2 共享驱动

- 分片握手产生一次 `Connected`。
- 同一批次中有效输入事件先于后续协议错误交付。
- 键盘、指针、剪贴板和连续更新事件不重排。
- 非增量更新、增量等待、请求合并和动态尺寸行为不变。
- shutdown、握手超时、帧错误、协议错误和事件通道关闭得到确定结果。
- 输出发送失败映射为对应 transport 错误。
- transport 控制消息的 `Continue` 不进入 RFB core。

### 14.3 TCP 回归

- 现有 `crates/ipkvm-headless/tests/rfb_tcp.rs` 全部通过。
- 现有 `crates/ipkvm-headless/tests/rfb_input_pump.rs` 全部通过。
- 第二个 TCP 客户端仍在首个连接结束后才收到 banner。
- TCP I/O 错误仍记录 `Io(ErrorKind)`。
- WebSocket 活动时 TCP 客户端可以建立 TCP 连接，但在共享 gate 释放前不收到 banner。

### 14.4 WebSocket 集成

使用真实 `127.0.0.1:0` listener、axum server 和
`tokio-tungstenite` 客户端，不 mock HTTP upgrade：

1. `/rfb` 无子协议 upgrade 成功且响应不选择子协议。
2. 请求 `binary` 时响应选择 `binary`。
3. 不同 Binary 消息中的逐字节 RFB 输入可以完成握手。
4. 服务端 RFB 输出全部是 Binary。
5. noVNC 1.7.0 初始化 fixture 获得正确 Raw 像素。
6. Key 和 Pointer 事件进入 `RfbServerEvent`，坐标携带输入时的 framebuffer 尺寸。
7. Ping/Pong 不污染 RFB 输入，连接继续工作。
8. Text 消息断开并产生 `UnexpectedTextMessage`。
9. 正常 Close 产生 `ClientClosed`。
10. 握手超时使用 Tokio 暂停时钟，不使用真实短 sleep。
11. shutdown 结束活动连接并产生 `ServerShutdown`。
12. 事件接收端关闭使活动连接结束，后续 upgrade 返回 `503`。
13. 活动连接存在时第二个 upgrade 返回 `409`。
14. 首个连接断开后下一个 upgrade 成功。
15. 超过输入上限的 WebSocket 消息由 WebSocket 层拒绝且内存有界。
16. client id 到 `u64::MAX` 后不回绕。
17. TCP 活动时 WebSocket upgrade 返回 `409`。
18. WebSocket 活动时 TCP 客户端不收到 banner；前一连接的 `Disconnected` 入队后，
    TCP 才完成握手并成为控制者。

### 14.5 命令级验证

```powershell
.\scripts\verify.ps1
```

该命令必须覆盖格式、全工作区测试、Clippy、文档、依赖许可证和来源审计。本阶段没有
必须人工执行的测试。

## 15. 文档和 issue 收口

实施完成后：

- `README.md` 记录已有原生 TCP 与 WebSocket 两种 RFB 入口，但不得声称完整网页已完成。
- `docs/ipkvm-coarse-design.md` 标记共享连接驱动和 WebSocket transport 已完成。
- 长期文档使用 `RfbServerEvent` 和 `RfbFrameError` 新名称。
- 旧的已实施设计文档保留当时的历史名称，不伪造历史状态；需要理解当前 API 时以本文和
  代码为准。
- issue `#15` 记录设计、红灯测试、实现提交、依赖审计和全量验证证据。

## 16. 风险与排除措施

### 16.1 抽取导致 TCP 行为漂移

措施：先把公共类型和共享状态迁移到不改变行为的测试通过提交，再增加 WebSocket；
现有真实 TCP 回环测试作为每一步门禁。

### 16.2 WebSocket 消息边界被误当作 RFB 包边界

措施：共享驱动只接收连续字节；集成测试把握手和客户端消息逐字节拆到不同 Binary
消息。

### 16.3 noVNC 默认子协议假设错误

措施：默认无子协议是固定兼容条件；同时测试无子协议和显式 `binary` 两种 upgrade。

### 16.4 多连接绕过输入单控制者

措施：TCP 与 WebSocket 必须显式使用同一个 gate；许可覆盖完整连接生命周期，并在
`Disconnected` 入队后释放。并发真实连接测试同时覆盖 TCP/TCP、WS/WS、TCP/WS 和
WS/TCP。

### 16.5 慢客户端形成无界输出

措施：不 split writer、不建立输出 channel；每次 core 输出只在一次 awaited send 中
存在，视频源继续使用最新值 watch。

### 16.6 shutdown 与断开事件竞态

措施：共享驱动只返回一个 `ConnectionEnd`，外围统一关闭 transport 和发送一次
`Disconnected`；测试 shutdown、Close 和错误三条路径。

### 16.7 WebSocket 库错误不可稳定比较

措施：对外只暴露稳定错误分类，底层错误不进入可克隆事件；Text 单独分类，其他
WebSocket 故障统一为 `WebSocket`。

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
