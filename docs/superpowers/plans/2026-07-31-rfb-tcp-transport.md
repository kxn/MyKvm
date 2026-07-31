# 单客户端 RFB TCP 传输实施计划

> **供自动化执行者使用：** 必须使用 `superpowers:subagent-driven-development` 或 `superpowers:executing-plans`，按任务顺序执行并逐项更新复选框。

**目标：** 在不依赖真实硬件的情况下，实现单活动客户端 RFB TCP server，把现有 RFB 协议核心接到共享 BGRA8888 模拟帧源和有界客户端事件通道。

**架构：** `ipkvm-rfb` 继续保持传输无关；`ipkvm-video` 固定 BGRA8888 和跨任务共享契约；`ipkvm-headless::rfb_tcp` 负责 Tokio TCP、帧适配、更新请求合并、连接生命周期和事件反压。server 使用顺序 accept 模型，同一时间只驱动一个连接，断开后再服务下一个。

**技术栈：** Rust 1.89、edition 2024、Tokio 1.53.1、thiserror 2、RFB 3.8、真实回环 TCP、`tokio::sync::watch`、有界 `tokio::sync::mpsc`。

## 全局约束

- 所有仓库内自写文档使用中文。
- 所有实现围绕 Gitea issue `#5`，提交信息包含 `(#5)`。
- 严格执行测试先行：每项行为先观察失败，再写最小实现。
- 不修改 `ipkvm-rfb` 的协议状态机、握手、消息解码和 framebuffer 编码职责。
- 不增加新的顶层第三方 crate；只启用已有 Tokio feature，并复用 thiserror。
- TCP 层只接受 `PixelFormat::Bgra8888`，不隐式猜测 `Rgb` 的字节布局。
- 输入和生命周期事件使用有界通道，禁止无界队列和静默丢弃。
- 不使用固定 sleep 掩盖异步竞态；时间行为使用 Tokio 暂停时钟。
- 不实现 WebSocket、noVNC、多活动客户端、HID 映射、真实视频或真实串口。
- 每次提交前运行与任务匹配的定向测试；最终运行 `.\scripts\verify.ps1`。

---

## 文件结构

实施完成后的主要文件：

```text
crates/ipkvm-video/src/lib.rs
    视频格式、共享帧和 FrameSource 跨任务契约

crates/ipkvm-video/src/mock.rs
    测试帧发布器

crates/ipkvm-headless/src/lib.rs
    HeadlessConfig 和 rfb_tcp 公共模块入口

crates/ipkvm-headless/src/rfb_tcp/mod.rs
    公共配置、客户端编号、事件、错误和模块导出

crates/ipkvm-headless/src/rfb_tcp/frame.rs
    VideoFrame 到 BgraFrameView 的校验和借用

crates/ipkvm-headless/src/rfb_tcp/pending.rs
    多个 FramebufferUpdateRequest 的有界合并

crates/ipkvm-headless/src/rfb_tcp/connection.rs
    单个 TCP 连接的握手、输入、帧更新和关闭状态机

crates/ipkvm-headless/src/rfb_tcp/server.rs
    顺序 accept、客户端编号、重连和 server shutdown

crates/ipkvm-headless/tests/rfb_tcp.rs
    公共 server API 的真实回环 TCP 端到端测试

crates/ipkvm-headless/tests/support/mod.rs
    不复用 server 编码器的最小 RFB 测试客户端
```

`connection.rs` 和 `server.rs` 分开，避免 socket 协议循环与 listener 生命周期相互缠绕。`frame.rs` 和 `pending.rs` 都是同步纯逻辑，先用单元测试固定后再接入异步循环。

---

### 任务 1：固定 BGRA8888 视频帧契约

**文件：**

- 修改：`crates/ipkvm-video/src/lib.rs`
- 修改：`crates/ipkvm-video/src/mock.rs`

**接口：**

- 产出：`PixelFormat::Bgra8888`
- 产出：`pub trait FrameSource: Send + Sync`
- 保持：`FrameSource::latest_frame()` 和 `FrameSource::subscribe()` 签名不变

- [x] **步骤 1：写入会失败的契约测试**

在 `crates/ipkvm-video/src/lib.rs` 测试模块中把现有 RGB 测试改为：

```rust
#[test]
fn video_frame_records_explicit_bgra8888_layout() {
    let bytes: Arc<[u8]> = Arc::from(vec![1, 2, 3, 4].into_boxed_slice());
    let frame = VideoFrame::new(
        42,
        MonotonicTimestamp::from_nanos(1_000),
        1,
        1,
        4,
        PixelFormat::Bgra8888,
        Arc::clone(&bytes),
    );

    assert_eq!(frame.pixel_format, PixelFormat::Bgra8888);
    assert_eq!(frame.stride, 4);
    assert!(Arc::ptr_eq(&frame.data, &bytes));
}

#[test]
fn frame_sources_are_send_and_sync() {
    fn assert_send_sync<T: FrameSource + Send + Sync>() {}

    #[cfg(feature = "mock")]
    assert_send_sync::<crate::mock::MockFrameSource>();
}
```

- [x] **步骤 2：运行测试并确认红灯**

运行：

```powershell
cargo test -p ipkvm-video --all-features
```

预期：编译失败，指出 `PixelFormat::Bgra8888` 尚不存在。

- [x] **步骤 3：实现明确格式和跨任务约束**

把 `PixelFormat::Rgb` 替换为：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Yuy2,
    Nv12,
    Bgra8888,
    Mjpeg,
    H264,
    Unknown,
}
```

把 trait 声明改为：

```rust
pub trait FrameSource: Send + Sync {
    fn latest_frame(&self) -> Option<SharedVideoFrame>;
    fn subscribe(&self) -> FrameReceiver;
}
```

同步修改测试构造的像素格式，不保留含义不明确的 `Rgb` 兼容别名。

- [x] **步骤 4：运行定向测试并确认绿灯**

运行：

```powershell
cargo test -p ipkvm-video --all-features
```

预期：全部通过。

- [x] **步骤 5：提交**

```powershell
git add crates/ipkvm-video/src/lib.rs crates/ipkvm-video/src/mock.rs
git commit -m "refactor: make BGRA video frames explicit (#5)"
```

---

### 任务 2：建立 TCP 配置、事件和帧适配

**文件：**

- 修改：`Cargo.toml`
- 修改：`crates/ipkvm-headless/Cargo.toml`
- 修改：`crates/ipkvm-headless/src/lib.rs`
- 新建：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`
- 新建：`crates/ipkvm-headless/src/rfb_tcp/frame.rs`

**接口：**

- 消费：`PixelFormat::Bgra8888`、`VideoFrame`、`RfbProtocolLimits`
- 产出：`RfbTcpConfig`
- 产出：`RfbClientId`
- 产出：`RfbTcpEvent`
- 产出：`RfbDisconnectReason`
- 产出：`RfbTcpConfigError`、`RfbTcpFrameError`
- 产出：`frame_view(&VideoFrame) -> Result<BgraFrameView<'_>, RfbTcpFrameError>`

- [x] **步骤 1：写入配置和帧适配失败测试**

在 `rfb_tcp/mod.rs` 和 `rfb_tcp/frame.rs` 的测试模块中定义以下测试：

```rust
#[test]
fn default_tcp_config_is_bounded() {
    let config = RfbTcpConfig::default();

    assert_eq!(config.read_buffer_bytes, 16 * 1024);
    assert_eq!(config.handshake_timeout, Duration::from_secs(10));
    assert!(config.validate().is_ok());
}

#[test]
fn tcp_config_rejects_zero_and_oversized_read_buffers() {
    let mut zero = RfbTcpConfig::default();
    zero.read_buffer_bytes = 0;
    assert_eq!(zero.validate(), Err(RfbTcpConfigError::ZeroReadBuffer));

    let mut oversized = RfbTcpConfig::default();
    oversized.read_buffer_bytes = oversized.protocol_limits.max_buffered_input_bytes + 1;
    assert!(matches!(
        oversized.validate(),
        Err(RfbTcpConfigError::ReadBufferExceedsInputLimit { .. })
    ));

    let mut no_timeout = RfbTcpConfig::default();
    no_timeout.handshake_timeout = Duration::ZERO;
    assert_eq!(
        no_timeout.validate(),
        Err(RfbTcpConfigError::ZeroHandshakeTimeout)
    );
}

#[test]
fn frame_adapter_accepts_padded_bgra() {
    let frame = video_frame(1, 2, 2, 12, PixelFormat::Bgra8888, vec![0; 20]);
    let view = frame_view(&frame).unwrap();

    assert_eq!(view.size(), RfbSize::new(2, 2).unwrap());
    assert_eq!(view.stride(), 12);
}

#[test]
fn frame_adapter_rejects_ambiguous_or_invalid_frames() {
    let wrong = video_frame(1, 1, 1, 4, PixelFormat::Mjpeg, vec![0; 4]);
    assert!(matches!(
        frame_view(&wrong),
        Err(RfbTcpFrameError::UnsupportedPixelFormat(PixelFormat::Mjpeg))
    ));

    let wide = video_frame(
        2,
        u32::from(u16::MAX) + 1,
        1,
        4,
        PixelFormat::Bgra8888,
        vec![0; 4],
    );
    assert!(matches!(
        frame_view(&wide),
        Err(RfbTcpFrameError::WidthOutOfRange(_))
    ));

    let short_stride =
        video_frame(3, 2, 1, 7, PixelFormat::Bgra8888, vec![0; 8]);
    assert!(matches!(
        frame_view(&short_stride),
        Err(RfbTcpFrameError::InvalidBgraFrame(
            RfbFramebufferError::StrideTooSmall { .. }
        ))
    ));

    let short_data =
        video_frame(4, 2, 1, 8, PixelFormat::Bgra8888, vec![0; 7]);
    assert!(matches!(
        frame_view(&short_data),
        Err(RfbTcpFrameError::InvalidBgraFrame(
            RfbFramebufferError::PixelDataTooShort { .. }
        ))
    ));
}
```

测试 helper 使用完整构造：

```rust
fn video_frame(
    seq: u64,
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
    data: Vec<u8>,
) -> VideoFrame {
    VideoFrame::new(
        seq,
        MonotonicTimestamp::from_nanos(seq),
        width,
        height,
        stride,
        pixel_format,
        Arc::from(data.into_boxed_slice()),
    )
}
```

- [x] **步骤 2：运行测试并确认红灯**

运行：

```powershell
cargo test -p ipkvm-headless rfb_tcp
```

预期：编译失败，指出 `rfb_tcp` 模块和相关类型尚不存在。

- [x] **步骤 3：增加依赖 feature**

工作区保留现有 Tokio 版本。`crates/ipkvm-headless/Cargo.toml` 增加：

```toml
[dependencies]
ipkvm-rfb = { path = "../ipkvm-rfb" }
ipkvm-session = { path = "../ipkvm-session" }
ipkvm-video = { path = "../ipkvm-video" }
thiserror.workspace = true
tokio = { workspace = true, features = ["io-util", "macros", "net", "rt"] }

[dev-dependencies]
ipkvm-video = { path = "../ipkvm-video", features = ["mock"] }
tokio = { workspace = true, features = ["test-util"] }
```

`src/lib.rs` 增加：

```rust
pub mod rfb_tcp;
```

- [x] **步骤 4：实现公共类型和配置校验**

`rfb_tcp/mod.rs` 定义设计文档第 7、13、14 节中的类型。配置校验核心为：

```rust
impl RfbTcpConfig {
    pub fn validate(&self) -> Result<(), RfbTcpConfigError> {
        if self.read_buffer_bytes == 0 {
            return Err(RfbTcpConfigError::ZeroReadBuffer);
        }
        if self.read_buffer_bytes > self.protocol_limits.max_buffered_input_bytes {
            return Err(RfbTcpConfigError::ReadBufferExceedsInputLimit {
                actual: self.read_buffer_bytes,
                maximum: self.protocol_limits.max_buffered_input_bytes,
            });
        }
        if self.handshake_timeout.is_zero() {
            return Err(RfbTcpConfigError::ZeroHandshakeTimeout);
        }
        Ok(())
    }
}
```

`RfbClientId` 提供只读值：

```rust
impl RfbClientId {
    pub fn get(self) -> u64 {
        self.0
    }
}
```

- [x] **步骤 5：实现帧适配**

`frame.rs` 的转换必须通过 `RfbSize` 和 `BgraFrameView` 复用协议层校验：

```rust
pub(super) fn frame_view(
    frame: &VideoFrame,
) -> Result<BgraFrameView<'_>, RfbTcpFrameError> {
    if frame.pixel_format != PixelFormat::Bgra8888 {
        return Err(RfbTcpFrameError::UnsupportedPixelFormat(
            frame.pixel_format,
        ));
    }
    let width = u16::try_from(frame.width)
        .map_err(|_| RfbTcpFrameError::WidthOutOfRange(frame.width))?;
    let height = u16::try_from(frame.height)
        .map_err(|_| RfbTcpFrameError::HeightOutOfRange(frame.height))?;
    let stride = usize::try_from(frame.stride)
        .map_err(|_| RfbTcpFrameError::StrideOutOfRange(frame.stride))?;
    let size = RfbSize::new(width, height)?;
    Ok(BgraFrameView::new(size, stride, &frame.data)?)
}
```

- [x] **步骤 6：运行定向测试并确认绿灯**

运行：

```powershell
cargo test -p ipkvm-headless rfb_tcp
cargo test -p ipkvm-video --all-features
```

预期：全部通过。

- [x] **步骤 7：提交**

```powershell
git add Cargo.toml Cargo.lock crates/ipkvm-headless crates/ipkvm-video
git commit -m "feat: add RFB TCP contracts and frame adapter (#5)"
```

---

### 任务 3：实现有界更新请求合并

**文件：**

- 新建：`crates/ipkvm-headless/src/rfb_tcp/pending.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`

**接口：**

- 消费：`FramebufferUpdateRequest`、`RfbRectangle`、`RfbSize`
- 产出：`PendingFramebufferRequest::merge`
- 产出：`PendingFramebufferRequest::take`

- [x] **步骤 1：写入请求合并失败测试**

覆盖多个 outstanding request 和整数边界：

```rust
#[test]
fn incremental_requests_merge_into_one_bounding_rectangle() {
    let size = RfbSize::new(100, 80).unwrap();
    let mut pending = PendingFramebufferRequest::default();
    pending.merge(request(true, 10, 20, 20, 10), size);
    pending.merge(request(true, 25, 5, 30, 25), size);

    assert_eq!(
        pending.take(),
        Some(request(true, 10, 5, 45, 25))
    );
}

#[test]
fn non_incremental_request_upgrades_pending_union() {
    let size = RfbSize::new(100, 80).unwrap();
    let mut pending = PendingFramebufferRequest::default();
    pending.merge(request(true, 10, 10, 10, 10), size);
    pending.merge(request(false, 30, 20, 10, 10), size);

    assert_eq!(
        pending.take(),
        Some(request(false, 10, 10, 30, 20))
    );
}

#[test]
fn merge_clips_without_u16_wraparound() {
    let size = RfbSize::new(u16::MAX, u16::MAX).unwrap();
    let mut pending = PendingFramebufferRequest::default();
    pending.merge(request(true, u16::MAX - 2, u16::MAX - 2, 20, 20), size);

    assert_eq!(
        pending.take(),
        Some(request(true, u16::MAX - 2, u16::MAX - 2, 2, 2))
    );
}
```

- [x] **步骤 2：运行测试并确认红灯**

运行：

```powershell
cargo test -p ipkvm-headless pending
```

预期：编译失败，指出 `PendingFramebufferRequest` 尚不存在。

- [x] **步骤 3：实现常量空间合并器**

使用一个 `Option<FramebufferUpdateRequest>` 保存状态。每个矩形先调用 `intersection(size)`；完全位于画面外的请求归一化为零面积矩形：

```rust
#[derive(Default)]
pub(super) struct PendingFramebufferRequest {
    request: Option<FramebufferUpdateRequest>,
}

impl PendingFramebufferRequest {
    pub(super) fn merge(
        &mut self,
        incoming: FramebufferUpdateRequest,
        size: RfbSize,
    ) {
        let incoming = normalize(incoming, size);
        self.request = Some(match self.request.take() {
            None => incoming,
            Some(current) => FramebufferUpdateRequest {
                incremental: current.incremental && incoming.incremental,
                rectangle: union(current.rectangle, incoming.rectangle, size),
            },
        });
    }

    pub(super) fn get(&self) -> Option<FramebufferUpdateRequest> {
        self.request
    }

    pub(super) fn take(&mut self) -> Option<FramebufferUpdateRequest> {
        self.request.take()
    }
}
```

`union` 使用 `u32` 计算 right/bottom，并限制到 `size.width()` 和 `size.height()`。两个零面积矩形合并后仍为零面积，不制造全屏请求。

- [x] **步骤 4：运行测试并确认绿灯**

运行：

```powershell
cargo test -p ipkvm-headless pending
```

预期：全部通过。

- [x] **步骤 5：提交**

```powershell
git add crates/ipkvm-headless/src/rfb_tcp
git commit -m "feat: coalesce RFB update requests (#5)"
```

---

### 任务 4：实现单连接握手和客户端事件

**文件：**

- 新建：`crates/ipkvm-headless/src/rfb_tcp/connection.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`

**接口：**

- 消费：`RfbTcpConfig`、`FrameReceiver`、`mpsc::Sender<RfbTcpEvent>`
- 产出：`run_connection`
- 产出：内部 `drive_connection`
- 产出：`ConnectionEnd`
- 保证：core 输出立即写出，输入事件按协议顺序进入有界通道

- [x] **步骤 1：写入真实 socket 握手失败测试**

在 `connection.rs` 内部测试模块绑定回环 listener，客户端逐字节发送：

```rust
#[tokio::test]
async fn fragmented_handshake_emits_connected_event() {
    let frame_source = MockFrameSource::new();
    frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[1, 2, 3, 0, 4, 5, 6, 0]));
    let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);

    let task = tokio::spawn(run_connection(
        RfbClientId(1),
        peer_addr,
        server_stream,
        frame_source.subscribe(),
        event_tx,
        RfbTcpConfig::default(),
        shutdown_rx,
    ));

    assert_eq!(read_exact_vec(&mut client_stream, 12).await, b"RFB 003.008\n");
    write_fragmented(&mut client_stream, b"RFB 003.008\n").await;
    assert_eq!(read_exact_vec(&mut client_stream, 2).await, [1, 1]);
    client_stream.write_all(&[1]).await.unwrap();
    assert_eq!(read_exact_vec(&mut client_stream, 4).await, [0, 0, 0, 0]);
    client_stream.write_all(&[1]).await.unwrap();
    let server_init = read_server_init(&mut client_stream).await;
    assert_eq!(server_init.size, (2, 1));

    assert!(matches!(
        event_rx.recv().await,
        Some(RfbTcpEvent::Connected {
            client_id: RfbClientId(1),
            shared: true,
            ..
        })
    ));

    drop(client_stream);
    assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
}
```

同一模块再写：

- `pipelined_input_events_preserve_order`
- `protocol_error_ends_connection_after_prior_valid_events`
- `handshake_timeout_uses_paused_clock`
- `shutdown_ends_handshake`

握手超时测试使用：

```rust
#[tokio::test(start_paused = true)]
async fn handshake_timeout_uses_paused_clock() {
    // 建立连接并读取 banner，不发送客户端版本。
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(matches!(
        task.await.unwrap(),
        ConnectionEnd::Failed(RfbTcpConnectionError::HandshakeTimeout)
    ));
}
```

- [x] **步骤 2：运行测试并确认红灯**

运行：

```powershell
cargo test -p ipkvm-headless connection
```

预期：编译失败，指出 `run_connection` 和 `ConnectionEnd` 尚不存在。

- [x] **步骤 3：实现连接初始化和输出写入**

`run_connection` 负责把内部可失败驱动器归一化为单一结束状态：

```rust
pub(super) async fn run_connection(
    client_id: RfbClientId,
    peer_addr: SocketAddr,
    stream: TcpStream,
    frame_rx: FrameReceiver,
    event_tx: mpsc::Sender<RfbTcpEvent>,
    config: RfbTcpConfig,
    shutdown: watch::Receiver<bool>,
) -> ConnectionEnd {
    match drive_connection(
        client_id,
        peer_addr,
        stream,
        frame_rx,
        event_tx,
        config,
        shutdown,
    )
    .await
    {
        Ok(end) => end,
        Err(error) => ConnectionEnd::Failed(error),
    }
}
```

内部 `drive_connection` 返回 `Result<ConnectionEnd, RfbTcpConnectionError>`，因此初始化和循环可以使用 `?`。开始顺序固定为：

```rust
let initial_frame = frame_rx
    .borrow()
    .clone()
    .ok_or(RfbTcpFrameError::FrameUnavailable)?;
let initial_view = frame_view(&initial_frame)?;
let mut core = RfbConnectionCore::new(RfbConnectionConfig {
    desktop_name: config.desktop_name.clone(),
    initial_size: initial_view.size(),
    limits: config.protocol_limits,
})?;
write_core_output(&mut stream, &mut core).await?;
```

`write_core_output` 只在输出非空时调用 `write_all`：

```rust
async fn write_core_output(
    stream: &mut TcpStream,
    core: &mut RfbConnectionCore,
) -> Result<(), RfbTcpConnectionError> {
    let output = core.take_output();
    if !output.is_empty() {
        stream.write_all(&output).await?;
    }
    Ok(())
}
```

`ConnectionEnd::reason(&self)` 把完整连接错误转换为可复制的事件分类。事件通道关闭时返回 `None`，由 server 升级为致命错误，不使用 `unreachable!` 掩盖调用顺序：

```rust
impl ConnectionEnd {
    pub(super) fn reason(&self) -> Option<RfbDisconnectReason> {
        Some(match self {
            Self::ClientClosed => RfbDisconnectReason::ClientClosed,
            Self::ServerShutdown => RfbDisconnectReason::ServerShutdown,
            Self::Failed(RfbTcpConnectionError::HandshakeTimeout) => {
                RfbDisconnectReason::HandshakeTimeout
            }
            Self::Failed(RfbTcpConnectionError::CoreConfig(error)) => {
                RfbDisconnectReason::CoreConfig(error.clone())
            }
            Self::Failed(RfbTcpConnectionError::Protocol(error)) => {
                RfbDisconnectReason::Protocol(error.clone())
            }
            Self::Failed(RfbTcpConnectionError::Encode(error)) => {
                RfbDisconnectReason::Encode(error.clone())
            }
            Self::Failed(RfbTcpConnectionError::Frame(error)) => {
                RfbDisconnectReason::Frame(error.clone())
            }
            Self::Failed(RfbTcpConnectionError::Io(error)) => {
                RfbDisconnectReason::Io(error.kind())
            }
            Self::Failed(RfbTcpConnectionError::EventChannelClosed) => {
                return None;
            }
        })
    }
}
```

- [x] **步骤 4：实现握手状态循环**

使用 `tokio::select!` 等待 socket、shutdown、event receiver 关闭和握手 deadline。每次 `push_input`：

1. 先按顺序处理返回事件。
2. 将 `HandshakeCompleted` 转成 `Connected`。
3. 将 Key、Pointer、CutText、ContinuousUpdates 转成对应 `RfbTcpEvent`。
4. 遇到协议错误从 `drive_connection` 返回 `Err`，由 `run_connection` 统一转换为 `ConnectionEnd::Failed`。
5. 每轮调用 `write_core_output`。

事件发送统一经过：

```rust
async fn send_event(
    sender: &mpsc::Sender<RfbTcpEvent>,
    event: RfbTcpEvent,
) -> Result<(), RfbTcpConnectionError> {
    sender
        .send(event)
        .await
        .map_err(|_| RfbTcpConnectionError::EventChannelClosed)
}
```

- [x] **步骤 5：运行测试并确认绿灯**

运行：

```powershell
cargo test -p ipkvm-headless connection
```

预期：握手、事件顺序、协议错误、暂停时钟超时和 shutdown 测试全部通过。

- [x] **步骤 6：提交**

```powershell
git add Cargo.toml Cargo.lock crates/ipkvm-headless
git commit -m "feat: drive one RFB TCP connection (#5)"
```

---

### 任务 5：接入请求驱动的帧更新和动态尺寸

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_tcp/connection.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/pending.rs`

**接口：**

- 消费：`PendingFramebufferRequest`
- 消费：`RfbConnectionCore::queue_framebuffer_update`
- 产出：非增量立即更新、增量等待最新帧、DesktopSize 更新

- [x] **步骤 1：写入帧更新失败测试**

在连接模块测试中增加：

- `non_incremental_request_resends_same_frame`
- `incremental_request_waits_for_new_sequence`
- `outstanding_incremental_requests_coalesce`
- `desktop_size_is_sent_before_new_pixels`
- `resize_without_negotiation_ends_connection`
- `regressed_frame_sequence_ends_connection`
- `framebuffer_limit_never_writes_partial_update`

关键增量测试行为：

```rust
client.send_set_encodings(&[-223]).await;
client.send_update_request(true, 0, 0, 2, 1).await;
client.send_key(true, 0x41).await;
assert!(matches!(
    event_rx.recv().await,
    Some(RfbTcpEvent::Key { keysym: 0x41, .. })
));
assert!(client.try_read_update().await.is_none());

frame_source.publish_frame(shared_bgra_frame(
    2,
    2,
    1,
    &[10, 20, 30, 0, 40, 50, 60, 0],
));

let update = client.read_update().await;
assert_eq!(update.raw_pixels(), &[10, 20, 30, 0, 40, 50, 60, 0]);
```

`try_read_update` 不使用真实 sleep。测试在 update request 后发送一个 KeyEvent，并等待对应 `RfbTcpEvent::Key`，以此证明 server 已经处理完前面的 update request；随后调用 `TcpStream::try_read`，`WouldBlock` 表示没有 unsolicited update。

- [x] **步骤 2：运行测试并确认红灯**

运行：

```powershell
cargo test -p ipkvm-headless connection
```

预期：测试失败，因为连接循环尚未消费 framebuffer 请求和帧 watch 变化。

- [x] **步骤 3：实现帧序号和 pending 状态**

连接状态增加：

```rust
let mut pending = PendingFramebufferRequest::default();
let mut last_observed_seq = initial_frame.seq;
let mut last_sent_seq: Option<u64> = None;
```

读取当前帧的 helper 必须检查倒退：

```rust
fn latest_frame(
    receiver: &FrameReceiver,
    last_observed_seq: &mut u64,
) -> Result<SharedVideoFrame, RfbTcpFrameError> {
    let frame = receiver
        .borrow()
        .clone()
        .ok_or(RfbTcpFrameError::FrameUnavailable)?;
    if frame.seq < *last_observed_seq {
        return Err(RfbTcpFrameError::FrameSequenceRegressed {
            previous: *last_observed_seq,
            actual: frame.seq,
        });
    }
    *last_observed_seq = frame.seq;
    Ok(frame)
}
```

- [x] **步骤 4：实现请求处理**

每个 `FramebufferUpdateRequested`：

```rust
let frame = latest_frame(&frame_rx, &mut last_observed_seq)?;
let size = frame_view(&frame)?.size();
pending.merge(request, size);
let merged = pending.get().expect("request was just merged");
let should_send = !merged.incremental || last_sent_seq != Some(frame.seq);
if should_send {
    let request = pending.take().expect("pending request exists");
    queue_and_write_frame(&mut stream, &mut core, &frame, request).await?;
    last_sent_seq = Some(frame.seq);
}
```

正常循环的 `tokio::select!` 只在 `pending.get().is_some()` 时启用 `frame_rx.changed()` 分支。变化后：

1. 克隆最新帧。
2. 检查序号倒退。
3. 序号等于上次发送时继续等待。
4. 序号增大时取出 pending，编码并写出。

`queue_and_write_frame` 持有 `Arc<VideoFrame>` 到编码结束，避免悬空借用。

- [x] **步骤 5：运行测试并确认绿灯**

运行：

```powershell
cargo test -p ipkvm-headless connection
```

预期：所有连接和帧更新测试通过。

- [x] **步骤 6：提交**

```powershell
git add crates/ipkvm-headless/src/rfb_tcp
git commit -m "feat: serve request-driven RFB frames (#5)"
```

---

### 任务 6：实现顺序 server 生命周期和公共回环测试

**文件：**

- 新建：`crates/ipkvm-headless/src/rfb_tcp/server.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`
- 新建：`crates/ipkvm-headless/tests/support/mod.rs`
- 新建：`crates/ipkvm-headless/tests/rfb_tcp.rs`

**接口：**

- 产出：`RfbTcpServer<S>::new`
- 产出：`RfbTcpServer<S>::run`
- 保证：单活动客户端、断线事件一次、断开后重连、关闭后无残留任务

- [ ] **步骤 1：建立独立字节级测试客户端**

`tests/support/mod.rs` 实现：

```rust
pub struct TestRfbClient {
    stream: TcpStream,
}

impl TestRfbClient {
    pub async fn connect(address: SocketAddr) -> Self;
    pub async fn handshake(&mut self, shared: bool) -> ServerInit;
    pub async fn set_encodings(&mut self, encodings: &[i32]);
    pub async fn set_rgb565(&mut self);
    pub async fn request_update(
        &mut self,
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    );
    pub async fn read_update(&mut self) -> FramebufferUpdate;
    pub async fn send_key(&mut self, down: bool, keysym: u32);
    pub async fn send_pointer(&mut self, buttons: u8, x: u16, y: u16);
    pub async fn send_cut_text(&mut self, bytes: &[u8]);
}
```

所有整数显式使用 RFB 大端序。测试客户端不得调用 `ipkvm-rfb` 的 server 编码函数，防止 server 和测试共享同一个错误。

- [ ] **步骤 2：写入公共 server 失败测试**

`tests/rfb_tcp.rs` 覆盖：

```rust
#[tokio::test]
async fn server_accepts_next_client_after_disconnect() {
    let fixture = ServerFixture::start().await;

    let mut first = TestRfbClient::connect(fixture.address()).await;
    first.handshake(true).await;
    let first_id = fixture.expect_connected().await;
    drop(first);
    fixture.expect_disconnect(first_id).await;

    let mut second = TestRfbClient::connect(fixture.address()).await;
    let init = second.handshake(true).await;
    let second_id = fixture.expect_connected().await;
    assert_eq!(init.width, 2);
    assert_eq!(init.height, 1);
    assert_ne!(first_id, second_id);

    fixture.shutdown().await.unwrap();
}
```

再覆盖：

- 第二个客户端在第一个结束前不收到 banner。
- RGB565 精确像素字节。
- 输入事件有界通道容量为 1 时不丢失且保持顺序。
- 初始帧为空或格式非法时产生确定 `Disconnected`，server 随后仍可服务合法帧连接。
- 客户端协议错误产生一次 `Disconnected`，server 继续运行。
- 事件 receiver 被丢弃后 `run` 返回 `EventChannelClosed`。
- shutdown 结束当前连接并使 `run` 返回 `Ok(())`。

`server.rs` 内部单元测试另外覆盖：

- `next_client_id == u64::MAX` 时分配最后一个编号，下一次分配返回 `ClientIdOverflow`，不回绕。
- shutdown 初始值为 `true` 时不接受连接。
- shutdown sender 被丢弃时按关闭处理。

`ServerFixture::expect_connected()` 从实际 `Connected` 事件提取 client id；测试不增加只供测试构造编号的生产 API。

- [ ] **步骤 3：运行测试并确认红灯**

运行：

```powershell
cargo test -p ipkvm-headless --test rfb_tcp
```

预期：编译失败，指出 `RfbTcpServer` 尚不存在。

- [ ] **步骤 4：实现 server 构造**

```rust
pub struct RfbTcpServer<S> {
    listener: TcpListener,
    frame_source: Arc<S>,
    event_tx: mpsc::Sender<RfbTcpEvent>,
    config: RfbTcpConfig,
    next_client_id: u64,
}

impl<S: FrameSource + 'static> RfbTcpServer<S> {
    pub fn new(
        listener: TcpListener,
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbTcpEvent>,
        config: RfbTcpConfig,
    ) -> Result<Self, RfbTcpServerError> {
        config.validate()?;
        Ok(Self {
            listener,
            frame_source,
            event_tx,
            config,
            next_client_id: 1,
        })
    }
}
```

- [ ] **步骤 5：实现顺序 accept 循环**

等待连接时同时处理 shutdown 和 event receiver 关闭：

```rust
loop {
    if shutdown_is_requested(&shutdown) {
        return Ok(());
    }

    let (stream, peer_addr) = tokio::select! {
        result = self.listener.accept() => result.map_err(RfbTcpServerError::Accept)?,
        _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
        _ = self.event_tx.closed() => return Err(RfbTcpServerError::EventChannelClosed),
    };

    let client_id = self.allocate_client_id()?;
    let end = run_connection(
        client_id,
        peer_addr,
        stream,
        self.frame_source.subscribe(),
        self.event_tx.clone(),
        self.config.clone(),
        shutdown.clone(),
    )
    .await;

    if matches!(
        &end,
        ConnectionEnd::Failed(RfbTcpConnectionError::EventChannelClosed)
    ) {
        return Err(RfbTcpServerError::EventChannelClosed);
    }

    let reason = end
        .reason()
        .ok_or(RfbTcpServerError::EventChannelClosed)?;
    self.send_disconnected(client_id, peer_addr, reason).await?;
    if matches!(&end, ConnectionEnd::ServerShutdown) {
        return Ok(());
    }
}
```

`send_disconnected` 是每个 accepted client 唯一发送断线事件的位置。`run_connection` 不自行发送 `Disconnected`，从结构上防止重复。

- [ ] **步骤 6：运行公共集成测试并确认绿灯**

运行：

```powershell
cargo test -p ipkvm-headless --test rfb_tcp
cargo test -p ipkvm-headless
```

预期：全部通过，无挂起测试和后台残留任务。

- [ ] **步骤 7：提交**

```powershell
git add crates/ipkvm-headless
git commit -m "feat: add single-client RFB TCP server (#5)"
```

---

### 任务 7：补齐边界测试和长期文档

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_tcp/connection.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/server.rs`
- 修改：`crates/ipkvm-headless/tests/rfb_tcp.rs`
- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-tcp-transport-design.md`
- 修改：`docs/superpowers/plans/2026-07-31-rfb-tcp-transport.md`

**接口：**

- 消费：前六项全部实现
- 产出：完整验收证据和准确阶段状态

- [ ] **步骤 1：按设计验收表核对测试名称**

运行：

```powershell
cargo test -p ipkvm-headless -- --list
```

必须能从输出中定位以下行为：

- 分片握手。
- BGRX8888 和 RGB565。
- 非增量重复响应。
- 增量等待新帧。
- outstanding request 合并。
- DesktopSize 成功和未协商失败。
- 输入顺序和事件通道反压。
- 握手超时。
- 协议错误。
- 断线重连。
- server shutdown。
- 事件 receiver 关闭。
- 帧格式、尺寸、stride、长度和序号错误。

缺少任一项时，先增加能失败的定向测试，再做最小实现修正。

- [ ] **步骤 2：运行性质和错误路径检查**

运行：

```powershell
cargo test -p ipkvm-rfb
cargo test -p ipkvm-video --all-features
cargo test -p ipkvm-headless
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
```

预期：全部通过，且现有 RFB 协议性质测试没有回归。

- [ ] **步骤 3：更新 README**

把当前状态更新为：

```markdown
当前工程已完成工作区脚手架、CH9329 协议与输入核心、传输无关的 RFB 3.8 协议核心，以及使用模拟 BGRA 帧源的单客户端 RFB TCP 库闭环。真实串口、真实视频采集、输入映射、可直接运行的无头进程、WebSocket/noVNC 和桌面界面仍按阶段计划继续实现。
```

明确说明当前 `ipkvm-headless` 二进制仍是脚手架，不能声称已经能控制真实机器。

- [ ] **步骤 4：更新阶段设计**

在 `docs/ipkvm-coarse-design.md` 的阶段 0：

- 增加“单客户端 RFB TCP、模拟 BGRA 帧和回环客户端闭环”到已完成。
- 把“普通 VNC 客户端和 noVNC”拆开，保留第三方客户端/noVNC 兼容性为待完成。
- 保留 RFB 输入映射和许可证自动审计为后续独立项。

- [ ] **步骤 5：回写设计与计划状态**

- 设计文档状态改为“已实施”。
- 计划复选框按实际完成状态更新为 `[x]`。
- 若公共类型名与计划不同，回写最终名称，不保留两套说法。
- 扫描并删除任何临时调试说明。

- [ ] **步骤 6：运行完整本地验证**

运行：

```powershell
.\scripts\verify.ps1
```

预期：

- UTF-8 无 BOM 检查通过。
- `cargo fmt --all --check` 通过。
- 全工作区全 feature 测试通过。
- Clippy `-D warnings` 通过。
- Rust 文档 `-D warnings` 通过。
- 工作区和暂存区 `git diff --check` 通过。

- [ ] **步骤 7：提交**

```powershell
git add README.md docs crates Cargo.toml Cargo.lock
git commit -m "test: harden RFB TCP transport (#5)"
```

- [ ] **步骤 8：最终自审**

运行：

```powershell
git status --short
git log --oneline main..HEAD
git diff --check main...HEAD
git diff --stat main...HEAD
```

逐项确认：

- 没有修改用户未提交的 `AGENTS.md`。
- 没有 Gitea runner 依赖。
- 没有无界通道。
- 没有固定 sleep。
- 没有复制整帧作为队列。
- 没有在 headless 重写 RFB 编解码。
- 没有把 mock server 描述成真实硬件产品。
