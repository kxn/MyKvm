# 共享 RFB 连接驱动与 WebSocket 传输实施计划

> **供自动化协作者使用：** 必须使用 `subagent-driven-development` 或
> `executing-plans` 按任务实施；每个任务严格执行测试先行、独立提交和独立审查。

**目标：** 把现有 TCP 专属 RFB 连接循环抽成共享驱动，保留全部 TCP 行为，并增加
兼容 noVNC 1.7.0 的 axum `/rfb` WebSocket 入口。

**架构：** `rfb_connection` 持有全局单连接 gate、公共事件、RFB core、视频帧状态和
传输无关连接循环；`rfb_tcp` 与 `rfb_ws` 只实现各自的承载适配。TCP 与 WebSocket
必须显式共享同一个 `RfbConnectionGate`，并在前一连接的 `Disconnected` 成功入队后
才允许下一连接进入 RFB 握手。

**技术栈：** Rust 1.89、Tokio 1、axum 0.8.9、tokio-tungstenite 0.29.0、
futures-util 0.3.33、RFB 3.8、noVNC 1.7.0 固定提交
`63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`。

## 全局约束

- 关联 Gitea issue 为 `#15`。
- 仓库自写文档、issue 和 PR 说明必须使用中文；代码标识符和第三方名称保留原文。
- 先写能证明缺失行为的测试并确认失败，再写最小实现，再确认通过。
- 修改必须解决架构根因，不复制 TCP 连接循环，不建立 WebSocket 到本地 TCP 的代理。
- `RfbConnectionCore` 继续是唯一的 RFB 握手、解析、协商和 framebuffer 编码实现。
- TCP 与 WebSocket 使用同一个 `RfbConnectionGate`，全局最多一个活动控制连接。
- WebSocket 默认不要求子协议；仅在客户端请求 `binary` 时选择 `binary`。
- WebSocket 只把 Binary 负载交给 RFB core；Text 必须确定性断开；Ping/Pong 不进入 core。
- 单条 WebSocket message 和 frame 上限等于
  `RfbProtocolLimits::max_buffered_input_bytes`。
- 不增加无界 channel、独立 WebSocket writer task、固定延时或吞错。
- 本阶段不嵌入 noVNC 静态资源，不增加 Node.js 和浏览器自动化。
- 新依赖必须来自 crates.io，并通过 `deny.toml` 与 `.\scripts\verify-licenses.ps1`。
- 所有验证在本机运行，不依赖 Gitea runner。

---

## 文件结构

新增或调整后的职责如下：

```text
crates/ipkvm-headless/src/
  lib.rs                         # 导出 rfb_connection、rfb_tcp、rfb_ws
  rfb_connection/
    mod.rs                       # 公共配置、事件、断开原因和共享错误
    gate.rs                      # 全局单连接许可和 client id
    transport.rs                 # crate 内私有异步 transport 接口
    driver.rs                    # 传输无关 RFB 连接循环
    frame.rs                     # VideoFrame 到 BgraFrameView
    pending.rs                   # 有界 framebuffer 请求合并
  rfb_tcp/
    mod.rs                       # TCP 配置和 server 错误
    transport.rs                 # TcpStream 适配
    server.rs                    # accept、gate 等待和断开事件
  rfb_ws/
    mod.rs                       # WebSocket 配置和 service 错误
    transport.rs                 # axum WebSocket 适配
    service.rs                   # /rfb route、upgrade 和 gate

crates/ipkvm-headless/tests/
  rfb_tcp.rs                     # TCP 行为回归
  rfb_input_pump.rs              # 输入生命周期回归
  rfb_websocket.rs               # 真实 WebSocket 与 noVNC 线级验收
  rfb_transport_exclusion.rs     # TCP/WS 共享 gate 交叉验收
  support/mod.rs                 # 回环服务和协议测试辅助函数
```

---

### 任务 1：建立传输无关公共类型和全局连接 gate

**文件：**

- 新建：`crates/ipkvm-headless/src/rfb_connection/mod.rs`
- 新建：`crates/ipkvm-headless/src/rfb_connection/gate.rs`
- 修改：`crates/ipkvm-headless/src/lib.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/connection.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/frame.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/server.rs`
- 修改：`crates/ipkvm-headless/src/rfb_input/pump.rs`
- 修改：`crates/ipkvm-headless/tests/rfb_tcp.rs`
- 修改：`crates/ipkvm-headless/tests/rfb_input_pump.rs`

**接口：**

- 产出：`RfbConnectionSettings`
- 产出：`RfbConnectionSettingsError`
- 产出：`RfbConnectionGate`
- 产出：`RfbConnectionGateError`
- 产出：`RfbClientId`
- 产出：`RfbServerEvent`
- 产出：`RfbDisconnectReason`
- 产出：`RfbFrameError`
- 修改：`RfbTcpConfig { connection, read_buffer_bytes }`
- 修改：`RfbTcpServer::new(listener, frame_source, event_tx, config, gate)`

- [ ] **步骤 1：写共享配置和 gate 的红灯测试**

在 `rfb_connection/mod.rs` 的单元测试中固定：

```rust
#[test]
fn default_connection_settings_are_bounded() {
    let settings = RfbConnectionSettings::default();
    assert_eq!(settings.desktop_name, "my_ipkvm");
    assert_eq!(settings.handshake_timeout, Duration::from_secs(10));
    assert!(settings.validate().is_ok());
}

#[test]
fn zero_handshake_timeout_is_rejected() {
    let settings = RfbConnectionSettings {
        handshake_timeout: Duration::ZERO,
        ..RfbConnectionSettings::default()
    };
    assert_eq!(
        settings.validate(),
        Err(RfbConnectionSettingsError::ZeroHandshakeTimeout)
    );
}
```

在 `gate.rs` 中使用 Tokio 测试固定：

```rust
#[tokio::test]
async fn gate_allows_exactly_one_permit() {
    let gate = RfbConnectionGate::new();
    let first = gate.try_acquire().unwrap();
    assert_eq!(first.client_id().get(), 1);
    assert_eq!(
        gate.try_acquire().unwrap_err(),
        RfbConnectionGateError::Busy
    );
    drop(first);
    assert_eq!(gate.try_acquire().unwrap().client_id().get(), 2);
}

#[tokio::test]
async fn gate_allocates_u64_max_once_and_never_wraps() {
    let gate = RfbConnectionGate::new();
    gate.inner.next_client_id.store(u64::MAX, Ordering::Relaxed);
    let last = gate.try_acquire().unwrap();
    assert_eq!(last.client_id().get(), u64::MAX);
    drop(last);
    assert_eq!(
        gate.try_acquire().unwrap_err(),
        RfbConnectionGateError::ClientIdOverflow
    );
}
```

- [ ] **步骤 2：运行红灯测试**

运行：

```powershell
cargo test -p ipkvm-headless rfb_connection
```

预期：编译失败，提示 `rfb_connection`、`RfbConnectionSettings` 和
`RfbConnectionGate` 尚不存在。

- [ ] **步骤 3：实现共享类型和 gate**

`rfb_connection/mod.rs` 定义：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbConnectionSettings {
    pub desktop_name: String,
    pub handshake_timeout: Duration,
    pub protocol_limits: RfbProtocolLimits,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbConnectionSettingsError {
    #[error("RFB handshake timeout must be non-zero")]
    ZeroHandshakeTimeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbServerEvent {
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
        framebuffer_size: RfbSize,
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

`RfbDisconnectReason` 保留 `ClientClosed`、`ServerShutdown`、
`HandshakeTimeout`、`CoreConfig`、`Protocol`、`Encode`、`Frame`、`Io`，
并新增 `WebSocket` 与 `UnexpectedTextMessage`。把 `RfbTcpFrameError` 的现有变体
原样迁移到 `RfbFrameError`。

`gate.rs` 使用 `Arc<GateInner>`、`Semaphore::new(1)` 和从 1 开始的
`AtomicU64`。`0` 表示 id 耗尽。核心接口为：

```rust
impl RfbConnectionGate {
    pub fn new() -> Self;
    pub(crate) async fn acquire(
        &self,
    ) -> Result<RfbConnectionPermit, RfbConnectionGateError>;
    pub(crate) fn try_acquire(
        &self,
    ) -> Result<RfbConnectionPermit, RfbConnectionGateError>;
}

impl RfbConnectionPermit {
    pub(crate) fn client_id(&self) -> RfbClientId;
}
```

取得 semaphore permit 后用 compare-exchange 循环分配 id：

```rust
fn allocate_client_id(&self) -> Result<RfbClientId, RfbConnectionGateError> {
    let mut current = self.inner.next_client_id.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return Err(RfbConnectionGateError::ClientIdOverflow);
        }
        let next = current.checked_add(1).unwrap_or(0);
        match self.inner.next_client_id.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(RfbClientId(current)),
            Err(actual) => current = actual,
        }
    }
}
```

如果 id 分配失败，已取得的 semaphore permit 随函数返回立即释放。

- [ ] **步骤 4：迁移公共名称和 TCP 配置**

执行完整语义迁移：

- `RfbTcpEvent` 改为 `RfbServerEvent`。
- `RfbTcpFrameError` 改为 `RfbFrameError`。
- `RfbClientId`、`RfbDisconnectReason` 和事件从 `rfb_tcp` 移到
  `rfb_connection`。
- `rfb_input::pump` 只从 `crate::rfb_connection` 导入共享类型。
- `RfbTcpConfig` 改为：

```rust
pub struct RfbTcpConfig {
    pub connection: RfbConnectionSettings,
    pub read_buffer_bytes: usize,
}
```

- `RfbTcpConfig::validate` 先调用 `connection.validate()`，再校验 TCP 读取块。
- `rfb_tcp::connection` 读取
  `config.connection.desktop_name`、`handshake_timeout` 和 `protocol_limits`。
- 不保留 `RfbTcpEvent` 或 `RfbTcpFrameError` 类型别名。

- [ ] **步骤 5：让 TCP server 使用共享 gate**

删除 `RfbTcpServer.next_client_id`。`RfbTcpServer` 保存传入的
`RfbConnectionGate`，每次 accept 后通过下面的选择等待 permit：

```rust
let permit = tokio::select! {
    result = self.gate.acquire() => {
        result.map_err(|error| match error {
            RfbConnectionGateError::ClientIdOverflow =>
                RfbTcpServerError::ClientIdOverflow,
            RfbConnectionGateError::Busy =>
                unreachable!("awaited gate acquisition cannot be busy"),
        })?
    }
    _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
    _ = self.event_tx.closed() =>
        return Err(RfbTcpServerError::EventChannelClosed),
};
let client_id = permit.client_id();
```

运行连接、发送 `Disconnected`，然后显式 `drop(permit)`。不得在
`send_disconnected(...).await` 完成前释放 permit。

- [ ] **步骤 6：运行任务测试**

运行：

```powershell
cargo test -p ipkvm-headless rfb_connection
cargo test -p ipkvm-headless --test rfb_tcp
cargo test -p ipkvm-headless --test rfb_input_pump
```

预期：全部通过；TCP 线级断言只发生类型名称和配置构造迁移，没有行为变化。

- [ ] **步骤 7：提交**

```powershell
git add crates/ipkvm-headless
git commit -m "refactor: share RFB connection types and gate (#15)"
```

---

### 任务 2：抽取共享连接驱动并把 TCP 降为 transport 适配

**文件：**

- 新建：`crates/ipkvm-headless/src/rfb_connection/transport.rs`
- 新建：`crates/ipkvm-headless/src/rfb_connection/driver.rs`
- 移动：`crates/ipkvm-headless/src/rfb_tcp/frame.rs` 到
  `crates/ipkvm-headless/src/rfb_connection/frame.rs`
- 移动：`crates/ipkvm-headless/src/rfb_tcp/pending.rs` 到
  `crates/ipkvm-headless/src/rfb_connection/pending.rs`
- 新建：`crates/ipkvm-headless/src/rfb_tcp/transport.rs`
- 删除：`crates/ipkvm-headless/src/rfb_tcp/connection.rs`
- 修改：`crates/ipkvm-headless/src/rfb_connection/mod.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/server.rs`
- 修改：`crates/ipkvm-headless/tests/rfb_tcp.rs`

**接口：**

- 产出：私有 `RfbTransport`
- 产出：私有 `RfbTransportRead`
- 产出：私有 `RfbTransportError`
- 产出：私有 `run_connection<T: RfbTransport>(...) -> ConnectionEnd`
- 产出：私有 `TcpTransport`

- [ ] **步骤 1：写 fake transport 的红灯测试**

在 `rfb_connection/driver.rs` 测试中定义一个记录发送内容、可排队输入结果的
`FakeTransport`，固定以下行为：

```rust
#[tokio::test]
async fn control_messages_do_not_enter_the_rfb_core() {
    let transport = FakeTransport::from_reads([
        FakeRead::Continue,
        FakeRead::Data(b"RFB 003.008\n".to_vec()),
        FakeRead::Closed,
    ]);
    let result = run_test_connection(transport).await;
    assert!(result.sent.starts_with(&[b"RFB 003.008\n".to_vec()]));
    assert!(matches!(result.end, ConnectionEnd::ClientClosed));
}

#[tokio::test]
async fn transport_text_error_has_a_stable_disconnect_reason() {
    let transport = FakeTransport::from_error(
        RfbTransportError::UnexpectedTextMessage,
    );
    let result = run_test_connection(transport).await;
    assert_eq!(
        result.end.reason(),
        Some(RfbDisconnectReason::UnexpectedTextMessage)
    );
}

#[tokio::test]
async fn transport_websocket_error_has_a_stable_disconnect_reason() {
    let transport = FakeTransport::from_error(RfbTransportError::WebSocket);
    let result = run_test_connection(transport).await;
    assert_eq!(
        result.end.reason(),
        Some(RfbDisconnectReason::WebSocket)
    );
}
```

- [ ] **步骤 2：运行红灯测试**

运行：

```powershell
cargo test -p ipkvm-headless rfb_connection::driver
```

预期：编译失败，提示 `RfbTransport` 和共享 driver 尚不存在。

- [ ] **步骤 3：定义私有 transport 接口**

`transport.rs` 定义：

```rust
pub(crate) enum RfbTransportRead {
    Data,
    Continue,
    Closed,
}

#[derive(Debug, Error)]
pub(crate) enum RfbTransportError {
    #[error("TCP I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("WebSocket transport error")]
    WebSocket,
    #[error("RFB over WebSocket does not accept text messages")]
    UnexpectedTextMessage,
}

pub(crate) trait RfbTransport {
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
```

约束写入 trait 文档并由测试固定：`Data` 时 buffer 非空，`Continue` 和
`Closed` 时 buffer 为空，每次接收不得保留上次输入。

- [ ] **步骤 4：迁移共享连接状态机**

把现有 `ConnectionState`、`ConnectionEnd`、连接错误、core 事件映射、帧更新和
shutdown 辅助函数移到 `rfb_connection/driver.rs`。公共入口精确为：

```rust
pub(crate) async fn run_connection<T: RfbTransport>(
    client_id: RfbClientId,
    peer_addr: SocketAddr,
    mut transport: T,
    frame_rx: FrameReceiver,
    event_tx: mpsc::Sender<RfbServerEvent>,
    settings: RfbConnectionSettings,
    shutdown: watch::Receiver<bool>,
) -> ConnectionEnd
```

driver 内部复用一个 `Vec<u8>`：

```rust
let mut input = Vec::new();
match transport.receive_into(&mut input).await? {
    RfbTransportRead::Data => {
        debug_assert!(!input.is_empty());
        let events = state.core.push_input(&input);
        write_core_output(&mut transport, &mut state.core).await?;
        if let Some(error) =
            state.handle_core_events(&mut transport, &event_tx, events).await?
        {
            return Err(error.into());
        }
        write_core_output(&mut transport, &mut state.core).await?;
    }
    RfbTransportRead::Continue => {}
    RfbTransportRead::Closed => return Ok(ConnectionEnd::ClientClosed),
}
```

无论正常或错误结束都调用一次 `transport.close().await`。关闭失败不覆盖已经确定的首个
连接结束原因。

- [ ] **步骤 5：实现 TCP transport**

`TcpTransport` 保存 `TcpStream` 和 `read_buffer_bytes`。输入实现：

```rust
buffer.clear();
buffer.resize(self.read_buffer_bytes, 0);
let count = self.stream.read(buffer).await?;
buffer.truncate(count);
if count == 0 {
    Ok(RfbTransportRead::Closed)
} else {
    Ok(RfbTransportRead::Data)
}
```

输出使用 `write_all(&bytes).await`，关闭使用 `AsyncWriteExt::shutdown` 并忽略关闭阶段
错误。`rfb_tcp::server` 构造 `TcpTransport` 后调用共享 `run_connection`。

- [ ] **步骤 6：迁移并运行全部 TCP 行为测试**

共享 driver 单元测试继续覆盖：

- 分片握手和事件顺序。
- 非增量重复发送。
- 增量等待与请求合并。
- DesktopSize 两阶段更新。
- 指针尺寸 epoch。
- 帧序号倒退。
- 输出上限不写半包。
- 暂停时钟握手超时。

运行：

```powershell
cargo test -p ipkvm-headless rfb_connection::driver
cargo test -p ipkvm-headless --test rfb_tcp
cargo test -p ipkvm-headless --test rfb_input_pump
```

预期：全部通过。

- [ ] **步骤 7：提交**

```powershell
git add crates/ipkvm-headless/src crates/ipkvm-headless/tests
git commit -m "refactor: extract shared RFB connection driver (#15)"
```

---

### 任务 3：实现 axum WebSocket transport 与 `/rfb` 服务

**文件：**

- 修改：`Cargo.toml`
- 修改：`crates/ipkvm-headless/Cargo.toml`
- 修改：`Cargo.lock`
- 新建：`crates/ipkvm-headless/src/rfb_ws/mod.rs`
- 新建：`crates/ipkvm-headless/src/rfb_ws/transport.rs`
- 新建：`crates/ipkvm-headless/src/rfb_ws/service.rs`
- 修改：`crates/ipkvm-headless/src/lib.rs`
- 新建：`crates/ipkvm-headless/tests/rfb_websocket.rs`
- 修改：`crates/ipkvm-headless/tests/support/mod.rs`

**接口：**

- 消费：`RfbTransport`、`run_connection`、`RfbConnectionGate`
- 产出：`RfbWebSocketConfig`
- 产出：`RfbWebSocketServiceError`
- 产出：`RfbWebSocketService<S>::new(...)`
- 产出：`RfbWebSocketService<S>::router() -> Router`

- [ ] **步骤 1：增加锁定依赖并运行许可证门禁**

工作区依赖增加：

```toml
axum = { version = "0.8.9", default-features = false, features = ["http1", "tokio", "ws"] }
futures-util = "0.3.33"
tokio-tungstenite = { version = "0.29.0", default-features = false, features = ["connect", "handshake"] }
```

`ipkvm-headless` 生产依赖只加入 `axum.workspace = true`；另外两个只放
`dev-dependencies`。

运行：

```powershell
cargo check -p ipkvm-headless --all-features
.\scripts\verify-licenses.ps1
```

预期：依赖解析成功，许可证和来源审计通过。此步骤只建立测试与实现所需依赖，不增加
生产行为。

- [ ] **步骤 2：写真实 WebSocket 红灯测试**

在 `tests/rfb_websocket.rs` 启动真实 `127.0.0.1:0` listener，并用
`tokio_tungstenite::connect_async` 覆盖：

```rust
#[tokio::test]
async fn websocket_upgrade_does_not_require_a_subprotocol() {
    let server = TestWebSocketServer::start().await;
    let (socket, response) = server.connect_without_protocol().await;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert!(response.headers().get(SEC_WEBSOCKET_PROTOCOL).is_none());
    drop(socket);
}

#[tokio::test]
async fn websocket_upgrade_selects_binary_only_when_requested() {
    let server = TestWebSocketServer::start().await;
    let (socket, response) = server.connect_with_protocols("chat, binary").await;
    assert_eq!(
        response.headers()[SEC_WEBSOCKET_PROTOCOL],
        HeaderValue::from_static("binary")
    );
    drop(socket);
}

#[tokio::test]
async fn an_unrelated_protocol_is_not_echoed() {
    let server = TestWebSocketServer::start().await;
    let (socket, response) = server.connect_with_protocols("chat").await;
    assert!(response.headers().get(SEC_WEBSOCKET_PROTOCOL).is_none());
    drop(socket);
}
```

同一红灯批次还必须覆盖：

- RFB banner、握手响应和 framebuffer update 全部为 Binary。
- RFB version banner 被拆成 12 条单字节 Binary 消息仍能完成握手。
- 一条 Binary 消息包含多个 RFB 客户端消息时事件保持顺序。
- Ping 后仍可继续握手。
- Text 产生一次 `Disconnected(UnexpectedTextMessage)`。
- Close 产生一次 `Disconnected(ClientClosed)`。
- 第二个 WS 在首个活动时收到 HTTP `409`。
- 首个连接关闭并收到 `Disconnected` 后第二个 WS 成功。
- shutdown 产生 `Disconnected(ServerShutdown)`。
- 事件接收端关闭后 upgrade 返回 `503`。
- 小输入上限下，超限消息关闭连接并产生 `Disconnected(WebSocket)`。

每个测试使用事件、WebSocket 响应或暂停时钟建立同步点，不使用固定 sleep。

- [ ] **步骤 3：运行红灯测试**

运行：

```powershell
cargo test -p ipkvm-headless --test rfb_websocket
```

预期：编译失败，提示 `rfb_ws`、`RfbWebSocketConfig` 和
`RfbWebSocketService` 尚不存在。依赖已经可用，失败原因不是测试环境缺包。

- [ ] **步骤 4：实现 WebSocket transport**

`rfb_ws/transport.rs` 的 `WebSocketTransport` 持有 axum `WebSocket`：

```rust
match self.socket.recv().await {
    Some(Ok(Message::Binary(bytes))) if !bytes.is_empty() => {
        buffer.extend_from_slice(&bytes);
        Ok(RfbTransportRead::Data)
    }
    Some(Ok(Message::Binary(_)))
    | Some(Ok(Message::Ping(_)))
    | Some(Ok(Message::Pong(_))) => Ok(RfbTransportRead::Continue),
    Some(Ok(Message::Close(_))) | None => Ok(RfbTransportRead::Closed),
    Some(Ok(Message::Text(_))) =>
        Err(RfbTransportError::UnexpectedTextMessage),
    Some(Err(_)) => Err(RfbTransportError::WebSocket),
}
```

每次 match 前 `buffer.clear()`。输出：

```rust
self.socket
    .send(Message::Binary(bytes.into()))
    .await
    .map_err(|_| RfbTransportError::WebSocket)
```

关闭发送 `Message::Close(None)`，忽略关闭阶段错误。Ping 的 Pong 由
axum/tungstenite 自动产生，应用层 Ping/Pong 不进入 RFB core。

- [ ] **步骤 5：实现 WebSocket 配置和 service**

`rfb_ws/mod.rs` 定义：

```rust
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RfbWebSocketConfig {
    pub connection: RfbConnectionSettings,
}

#[derive(Debug, Error)]
pub enum RfbWebSocketServiceError {
    #[error("invalid RFB WebSocket configuration: {0}")]
    Config(#[from] RfbConnectionSettingsError),
}
```

`RfbWebSocketService::new` 精确参数为：

```rust
pub fn new(
    frame_source: Arc<S>,
    event_tx: mpsc::Sender<RfbServerEvent>,
    config: RfbWebSocketConfig,
    shutdown: watch::Receiver<bool>,
    gate: RfbConnectionGate,
) -> Result<Self, RfbWebSocketServiceError>
```

handler 先检查 shutdown 和 `event_tx.is_closed()`，再
`gate.try_acquire()`。Busy 返回空 body `409`，id 耗尽返回空 body `503`。成功时：

```rust
ws.protocols(["binary"])
    .max_message_size(limit)
    .max_frame_size(limit)
    .on_upgrade(move |socket| run_upgraded_connection(state, socket, peer, permit))
    .into_response()
```

`router()` 使用 `/rfb` 和 `any(handler::<S>)`，并注入 `Arc<ServiceState<S>>`。逐个运行
步骤 2 中的红灯；每次只补足当前测试要求的最小生产行为，再处理下一个红灯。

- [ ] **步骤 6：运行 WebSocket 与依赖测试**

运行：

```powershell
cargo test -p ipkvm-headless --test rfb_websocket
cargo test -p ipkvm-headless
.\scripts\verify-licenses.ps1
```

预期：全部通过。

- [ ] **步骤 7：提交**

```powershell
git add Cargo.toml Cargo.lock crates/ipkvm-headless
git commit -m "feat: add RFB WebSocket transport (#15)"
```

---

### 任务 4：固定 noVNC 1.7.0 线级样本和跨传输互斥

**文件：**

- 修改：`crates/ipkvm-headless/tests/rfb_websocket.rs`
- 新建：`crates/ipkvm-headless/tests/rfb_transport_exclusion.rs`
- 修改：`crates/ipkvm-headless/tests/support/mod.rs`

**接口：**

- 消费：`RfbTcpServer`
- 消费：`RfbWebSocketService`
- 消费：同一个 `RfbConnectionGate`
- 产出：noVNC 1.7.0 初始化 fixture
- 产出：TCP/WS 交叉单控制者验收

- [ ] **步骤 1：写 noVNC 初始化红灯测试**

测试 fixture 使用固定提交中无 WebCodecs H.264 能力的确定分支。`SetPixelFormat`
精确字节为：

```rust
const NOVNC_1_7_SET_PIXEL_FORMAT: [u8; 20] = [
    0, 0, 0, 0,
    32, 24, 0, 1,
    0, 255, 0, 255, 0, 255,
    0, 8, 16,
    0, 0, 0,
];
```

编码顺序为：

```rust
const NOVNC_1_7_ENCODINGS_WITHOUT_H264: [i32; 24] = [
    1, 7, -260, 16, 21, 5, 2, 6, 0,
    -26, -254, -223, -224, -258, -261, -308, -309, -312,
    -313, -307, 0xc0a1e5ceu32 as i32, -316,
    0x574d5664, -239,
];
```

该数组长度和元素已经按固定源码精确列出，不允许删除或添加元素。质量等级为默认 6，
因此 `-32 + 6 = -26`；压缩等级为默认 2，因此
`-256 + 2 = -254`。

测试完成握手后依次发送该 pixel format、由编码数组构造的 `SetEncodings` 和：

```rust
let mut initial_request = vec![3, 0, 0, 0, 0, 0];
initial_request.extend_from_slice(&width.to_be_bytes());
initial_request.extend_from_slice(&height.to_be_bytes());
```

模拟帧使用 BGRA 像素 `[30, 20, 10, 255]`，断言 Raw 像素为
`[10, 20, 30, 0]`。

- [ ] **步骤 2：运行 noVNC 红灯测试**

运行：

```powershell
cargo test -p ipkvm-headless --test rfb_websocket novnc_1_7
```

预期：如果 WebSocket 实现尚未兼容完整编码序列或像素格式，测试失败并显示第一处
线级差异；修复必须位于协议 core 或共享 driver 的真实责任层。

- [ ] **步骤 3：实现 noVNC fixture 所需行为**

预期不新增协议分支。现有 core 应：

- 接受未知编码并在列表中识别 `Raw` 与 `DesktopSize`。
- 接受 noVNC 32/24 小端 true color 像素格式。
- 发送按红、绿、蓝位移编码的 Raw 更新。

若红灯暴露缺陷，先在 `ipkvm-rfb` 增加最小单元回归，再修正 core；不得在
WebSocket 层改写 `SetEncodings` 或像素。

- [ ] **步骤 4：写 TCP/WS 交叉互斥红灯测试**

`rfb_transport_exclusion.rs` 用同一个 frame source、event channel、shutdown 和
`RfbConnectionGate` 同时启动 TCP 与 WebSocket：

```rust
#[tokio::test]
async fn active_tcp_rejects_websocket_upgrade() {
    let system = TestDualTransportSystem::start().await;
    let tcp = system.connect_tcp_and_finish_handshake().await;
    assert_eq!(
        system.try_connect_websocket().await.unwrap_err().status(),
        Some(StatusCode::CONFLICT)
    );
    drop(tcp);
}

#[tokio::test]
async fn active_websocket_keeps_tcp_waiting_until_disconnected_is_enqueued() {
    let system = TestDualTransportSystem::start_with_event_capacity(1).await;
    let ws = system.connect_websocket_and_finish_handshake().await;
    let mut tcp = system.connect_tcp().await;
    assert!(matches!(
        tcp.try_read(&mut [0; 1]),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    ));
    system.close_websocket(ws).await;
    assert!(matches!(
        system.events.recv().await,
        Some(RfbServerEvent::Disconnected { .. })
    ));
    assert_eq!(system.read_tcp_banner(&mut tcp).await, b"RFB 003.008\n");
}
```

在 `try_read` 前先通过 WebSocket 输入事件并等待对应事件，证明服务端已进入正常状态，
不用 sleep 猜测调度。

- [ ] **步骤 5：运行兼容与互斥测试**

运行：

```powershell
cargo test -p ipkvm-headless --test rfb_websocket
cargo test -p ipkvm-headless --test rfb_transport_exclusion
cargo test -p ipkvm-headless --test rfb_tcp
cargo test -p ipkvm-headless --test rfb_input_pump
```

预期：全部通过。

- [ ] **步骤 6：提交**

```powershell
git add crates/ipkvm-headless/tests crates/ipkvm-rfb
git commit -m "test: verify noVNC and cross-transport compatibility (#15)"
```

---

### 任务 5：文档、依赖审计和全量验收收口

**文件：**

- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-websocket-transport-design.md`
- 修改：`docs/references/README.md`

**接口：**

- 产出：当前实现状态和 noVNC 固定版本的中文长期记录
- 产出：本机完整验证证据

- [ ] **步骤 1：更新长期文档**

写明：

- 已有 TCP 与 WebSocket 两个 RFB 承载入口。
- 两者共享连接驱动、事件模型和全局单连接 gate。
- `/rfb` 已通过 noVNC 1.7.0 线级初始化样本，但完整网页和真实浏览器闭环属于下一
  issue。
- 生产组装必须向 `RfbTcpServer` 与 `RfbWebSocketService` 传入同一个 gate。
- 当前仍没有真实视频采集、真实串口、鉴权、TLS 和完整 headless 二进制。
- noVNC 固定提交、axum 0.8.9 和协议资料链接。

把设计文档状态改为“已实施”，并记录实际公共接口与计划存在的任何已验证差异。

- [ ] **步骤 2：检查中文、占位项和格式**

运行：

```powershell
rg -n "TBD|TODO|FIXME|待定|稍后实现" README.md docs
git diff --check
```

预期：本次新增文档没有占位项；`git diff --check` 无输出。

- [ ] **步骤 3：运行格式与定向验证**

运行：

```powershell
cargo fmt --all
cargo fmt --all --check
cargo test -p ipkvm-headless
cargo test -p ipkvm-rfb
.\scripts\verify-licenses.ps1
```

预期：全部通过。

- [ ] **步骤 4：运行全工作区验证**

运行：

```powershell
.\scripts\verify.ps1
```

预期：格式、测试、Clippy、文档、许可证和来源审计全部通过。

- [ ] **步骤 5：提交**

```powershell
git add README.md docs
git commit -m "docs: record RFB WebSocket compatibility (#15)"
```

- [ ] **步骤 6：最终分支审查**

从分支起点 `e84086d` 到当前 HEAD 生成完整 diff 审查包，检查：

- 设计中每项验收是否都有自动化测试。
- TCP 现有行为是否没有弱化。
- gate 是否覆盖 TCP 和 WebSocket，且释放顺序正确。
- WebSocket 是否没有无界缓冲或第二套 RFB 解析。
- noVNC fixture 是否精确对应固定提交。
- 新依赖是否最小且许可证门禁通过。

修复所有阻断合并的问题后重新运行 `.\scripts\verify.ps1`。

- [ ] **步骤 7：PR 和合并后验证**

创建中文 PR，描述必须包含：

- `Closes #15`
- 改动摘要
- 自动化测试证据
- 许可证与来源审计结果
- 文档影响
- “没有人工验证例外”

合并后在主工作区执行：

```powershell
.\scripts\verify.ps1
git status --short
```

预期：全量验证通过；主工作区只保留用户原有、未由本任务修改的 `AGENTS.md` 改动。
