# 视频采集到传输链路性能调研

> 关联：Gitea `kxn/my_ipkvm`，本调研收口 issue #162。
> 本文是视频链路性能优化的长期事实来源（现状代码事实 + 实测基线 + 性能预算 +
> 分阶段方案 + 测试矩阵）。后续实施单以本文为依据拆分。

## 1. 背景与目标

当前视频链路以"行为闭环为主"，性能优化尚未进入实施阶段。release 实测：

- 混合 640×360/1280×720 Y4M demo 素材在 `--fps 30` 下，未连接 RFB 客户端约 23.9 FPS，
  连接 Raw RFB 后约 21.7 FPS；Raw 输出约 50.4 MB/s，进程单核等效 CPU 约从 6.1% 升到 14.4%。
- `--fps 10` 下约 9.4 FPS、Raw 输出约 19.0 MB/s。
- `/api/status` 的 `last_frame_ns` 是观察时间，不是采集帧时间，不能直接作为端到端帧率指标。
- 1080p desktop 路径实测（`perf-1080p.result.log`）：avg 44ms / p95 59ms，
  超过 34ms / 40ms 阈值；源实际只跑到 ~22 FPS（目标 30）。

目标：把视频链路做成跨平台（含未来 Linux ARM 盒子）、跨前端（noVNC / iced desktop）
可用的形态。本次调研的产物是分阶段优化方案，**不直接引入 H.264/WebRTC**，
不在没有基准和回归测试前做大规模重构。

非目标：

- 不改变鼠标/键盘协议语义。
- 不绑定特定 SoC 的硬件编码 API；保持通用 aarch64 可移植，按平台选最优后端。
- 不做大规模重构。

## 2. 链路总览与现状代码事实

```
采集设备 ── 采集线程(阻塞) ──> 像素格式转换(→BGRA8888)
   │                                │
   │  DirectShow Receive / nokhwa frame() / Y4M 文件
   │                                ▼
   │                        Arc::from(Vec) wrap   ← 每帧整帧拷贝 + 分配
   │                                ▼
   │                  VideoFrame{ seq, ts, data: Arc<[u8]> }
   │                                │
   │                   tokio::sync::watch (单槽 latest-value)
   │                                │ refcount bump，不拷贝像素
   └────────────────┬───────────────┼───────────────┬──────────────┐
                    ▼               ▼               ▼              ▼
              RFB 编码         iced desktop      /api/status    screenshot
            (仅 Raw)         BGRA→RGBA+上传       observe       on-demand JPEG
                    │
                    ▼
         encode_raw_update   ← per-pixel + scale 乘法 + 每帧新 Vec
                    │
                    ▼
         TCP write_all / WS send  ← 无 TCP_NODELAY
```

分发层（watch + `Arc<VideoFrame>` + `Arc<[u8]>`）的设计是正确的：多消费者只做 refcount
bump，不拷贝像素，慢消费者自动丢中间帧不阻塞生产者。问题集中在两端——采集转换段
和编码传输段。

下面逐段给出代码事实（文件:行号，对应 commit `f0e3892`）。

### 2.1 采集段

**Windows DirectShow**（`crates/ipkvm-video/src/camera.rs`、`dshow_sink.rs`）：

- 阻塞式、事件驱动。`IMemInputPin::Receive` 在 DirectShow 流线程上把 `IMediaSample`
  拷进可复用的 slot（`dshow_sink.rs:672-691`），拷贝是必须的（sample buffer 属于
  allocator，`Receive` 返回后可能被复用）。空闲时 `Condvar::wait_timeout`，
  CPU≈0（`camera.rs:271-308`，`dshow_sink.rs:195-201`）。这段写得不错。
- **`frames_per_second` 参数被验证非零后直接丢弃**（`camera.rs:97`
  `let _ = frames_per_second;`）。帧率完全由设备/DirectShow 图决定，无应用层节流。

**Linux/macOS nokhwa**（`crates/ipkvm-video/src/camera_nokhwa.rs`）：

- `camera.frame()` 阻塞拿帧，复用 `rgba_buf` 解码、原地 R↔B 交换
  （`camera_nokhwa.rs:90, 106-119`）。**同样忽略 `frames_per_second`**（仅验证非零，
  `camera_nokhwa.rs:42-45`）。读错时 50ms 退避重试（`:96-102`）。

**文件/Y4M demo 源**（`crates/ipkvm-video/src/file_source.rs`、`looping.rs`、`y4m.rs`）：

- tokio task 跑循环播放，`tokio::time::sleep(interval)` 节流（`file_source.rs:43,68`）。
  这是唯一有应用层 FPS 控制的源。
- Y4M 解析只支持 8-bit 4:2:0（`y4m.rs:67-72`），`frame_bgra` 每次**新 `Vec`** 做 YUV→BGRA
  （`y4m.rs:136`），loop 里对同一批帧反复转换，没有缓存。

**采集端无任何指标**：整个 `ipkvm-video` crate 没有 FPS / 转换耗时 / 丢帧计数器，
只有 `seq` 字段（`lib.rs:59`）。`examples/camera_perf.rs`、`camera_probe.rs` 是外部
示例，不属于运行时指标。

### 2.2 像素格式转换段（热瓶颈之一）

所有真实路径都把采集格式转成 **BGRA8888** 再发布：

- DirectShow `convert_to_bgra`（`dshow_sink.rs:846-986`）覆盖 NV12/YUY2/RGB24/RGB32→BGRA。
  **全部标量 `for`/`while`，每像素 BT.601 整数矩阵 + `clamp(0,255)`**，无 SIMD、
  无 autovectorize 友好结构。例：`nv12_to_bgra_inner`（`:889-909`）、
  `yuy2_to_bgra_inner`（`:912-954`，含奇宽尾处理）。
- nokhwa 路径是已解码 RGBA 的原地 `swap(0,2)` + alpha=255（`camera_nokhwa.rs:116-119`），
  相对便宜。
- Y4M 路径用同样 BT.601 标量公式（`y4m.rs:151-159`）。

**对 ARM 的影响**：YUY2/NV12→BGRA 标量在 ARM 弱单核上很疼。720p30 ≈ 2700 万次像素
运算/秒，标量 + clamp 分支几乎不能向量化，会吃掉一个小核的全部预算。

### 2.3 分发段（设计正确，无需动）

- `VideoFrame.data: Arc<[u8]>`（`lib.rs:66`），`SharedVideoFrame = Arc<VideoFrame>`
  （`lib.rs:106`）。多消费者只做 refcount bump，不拷贝像素。
- 分发用 `tokio::sync::watch`（单槽 latest-value，`lib.rs:107`）。慢消费者自动丢中间帧、
  不阻塞生产者，对"最新帧覆盖"语义正确。
- publish 是双写：`RwLock<Option<SharedVideoFrame>>`（同步 `latest_frame()` 用）+
  `watch::Sender`（异步 `subscribe()` 用），例 `camera.rs:295-298`、`mock.rs:22-25`。
- **没有丢帧计数**：watch 静默 coalesce，crate 自己不知道丢了几个，下游只能靠 `seq`
  比对发现跳号（RFB driver `driver.rs:310-316`；iced preview `app.rs:745-749`）。

### 2.4 分配热点（可低成本回收）

热路径上每帧都有新分配，无对象池：

- `convert_to_bgra` 每帧新 `vec![0u8; w*h*4]`（`dshow_sink.rs:850`）。
- `Arc::from(bgra)`：把 Vec 转 `Arc<[u8]>`，**又一次堆分配 + 整块拷贝**
  （`camera.rs:293`；nokhwa `camera_nohwa.rs:130`）。
- `Arc::new(VideoFrame)`（`camera.rs:295`）。

即每帧除 YUV→BGRA 运算外，还要付一次"分配 + 整帧 memcpy"的 Arc wrap。720p ≈ 3.5MB、
1080p ≈ 8MB，每帧。可复用的 scratch buffer 只存在于 raw 捕获字节
（`dshow_sink.rs` slot、`raw_buf` `camera.rs:270`、nokhwa `rgba_buf`）。

### 2.5 RFB 编码段（全链路最大瓶颈）

只支持两个编码：**Raw** 和 **DesktopSize**（`crates/ipkvm-rfb/src/protocol/server.rs`）。
没有 Tight / ZRLE / Hextile / JPEG / MJPEG。客户端可声明任意编码（`SetEncodings` 解码于
`protocol/client.rs:169`），但 list 只用于 `supports_desktop_size()`（`connection.rs:249-251`），
**Raw 永远被使用，不管客户端要什么**。

`encode_raw_update`（`server.rs:67-103`）关键问题：

- **嵌套 per-pixel/per-row 循环**（`:96-99`），每像素调 `pixel_format.write_bgr(...)`。
- `write_bgr`（`pixel_format.rs:214-230`）即使源 8bpc 且目标 8bpc，**仍跑一遍
  `scale_channel` 乘法**（`pixel_format.rs:259-261`）。这个乘法挡住 autovectorize，
  使本可 `memcpy` 的恒等变换退化成逐像素运算。alpha 字节被丢弃（`write_bgr` 只取 B/G/R）。
- **每次 update 分配新 `Vec::with_capacity(w*h*bpp+16)`**（`server.rs:77`），再
  `extend_from_slice` 整块拷进连接输出缓冲（`connection.rs:361-371`）。一帧像素被写两遍。
- **无脏区/差分**：每次 update 永远发整个请求矩形（裁剪到帧尺寸，`connection.rs:286-289`），
  永远 1 个矩形（`server.rs:78` 硬编码 `[0,0,0,1]`）。`incremental` 标志只决定"发不发"
  （`driver.rs:279-283`），不决定"发哪块"，增量请求照样发全帧。
- 请求合并是唯一的"合并"逻辑：多个 outstanding request 的矩形 union 成一个外接框
  （`pending.rs:9-18, 44-67`），仍是一个矩形。

### 2.6 传输段

- TCP：`TcpTransport::send_binary` 单次 `write_all`（`rfb_tcp/transport.rs:38-41`），
  **没设 `TCP_NODELAY`**（全 workspace grep 无 `nodelay`/`NoDelay`）。Nagle 对 RFB 这种
  请求/响应小包是延迟杀手。无 `writev`/vectored write，无 `BufWriter`。
- 节流方式：**纯请求/响应，无时钟、无 FPS、无 Fence**。`EnableContinuousUpdates`
  被解码但**未生效**（`driver.rs:257-263`），仍要求客户端维持 outstanding request。
- **单客户端闸门**：`Semaphore::new(1)`（`gate.rs:79`），系统全局只允许一个 RFB 连接
  （TCP 或 WS），不支持多观察者。
- 反压：`write_all`/`socket.send` 的 `await` 自然 stall 整个连接 task，期间 watch
  coalesce 掉中间帧——慢客户端丢帧但不堆积。

### 2.7 iced desktop 渲染段

- `bgra_to_rgba`（`crates/ipkvm-desktop-iced/src/frames.rs:6-44`）：**每帧新 `Vec` +
  逐像素 4 字节 shuffle**，1080p ≈ 2M 次交换/帧，在 GUI 线程 `update` 同步做，无 SIMD。
- `FrameReady` 每次重建 `Handle::from_rgba` + 全屏纹理上传（`app.rs:487-497`），
  **无 seq 去重**（对比：预览路径有 seq 去重 `app.rs:745-749`，实时路径没有）。
- 帧到达由 watch 事件驱动（`frames.rs:38-69`），非固定 tick。

### 2.8 desktop JPEG 路径澄清

desktop (iced) 实时渲染路径**不做 JPEG/PNG 编码**，`Handle::from_rgba` 是未压缩 RGBA
（`frames.rs:41`）。`jpeg-encoder` 仅用于：desktop 截图存文件（`ipkvm-desktop/src/clipboard.rs:55-65`，
quality 85）、headless `GET /api/screenshot`（`web/service.rs:481-493`，quality 85）。
`zune-jpeg` 是 dev-dependency 仅用于测试断言（`tests/web_http.rs:719`），不进运行时二进制，
也不用于 MJPEG 相机解码。

### 2.9 指标缺口

- `encode_raw_update` 无耗时/字节计数；`send_binary` 丢弃长度；无 updates/sec。
- `last_sent_seq` 只用于正确性（regression 检测 `driver.rs:310-316`），不暴露为统计。
- `/api/status` 的 `last_frame_ns` **是 observe 时间不是采集时间**
  （`console_session.rs:178-184`，`now_ns()` 是 process-relative）。
- `dropped_frames` 只在有人 poll status（`refresh_stats` → `observe_frame`）时才推进，
  无 polling 客户端时会失真。

## 3. 实测基线

| 场景 | FPS | Raw 输出 | 单核等效 CPU | 备注 |
|------|-----|----------|-------------|------|
| 640×360/720p Y4M, --fps 30, 无客户端 | 23.9 | — | 6.1% | demo 素材 |
| 同上 + Raw RFB | 21.7 | 50.4 MB/s | 14.4% | 编码吃额外 ~8% 单核 |
| 同上, --fps 10 | 9.4 | 19.0 MB/s | — | |
| 1080p desktop (iced), 30fps 目标 | ~22 实际 | — | 31.7% | `perf-1080p.result.log`，
avg 44ms / p95 59ms 超阈值；dropped 0 |

注意：1080p 测的是"源 push 速率"，源线程本身只跑到 ~22fps（目标 30），说明 GUI 处理
反压回 source；`FrameStats` 测的是 `FrameReady` 到达间隔，混杂了源节流与消费端处理时间，
不是纯采集数。

## 4. 性能预算（ARM 盒子场景，以 1080p30 为参考）

假设目标：RK3588/树莓派 5 级别 ARM 盒子，单核中等偏弱，目标 1080p30 显示 + 单 RFB 客户端
经千兆或 WiFi。

| 链路段 | 现状预算（标量） | 可达预算（优化后） | 备注 |
|--------|-----------------|-------------------|------|
| YUY2→BGRA 转换 | ~1 小核满 | NEON SIMD 后 < 15% 小核 | 现标量挡不住 |
| NV12→BGRA | ~0.8 小核满 | < 10% 小核 | |
| Arc wrap 整帧拷贝 | 每帧 8MB memcpy | 池化后趋零 | 1080p 每帧分配 8MB |
| Raw 编码 per-pixel | ~2M 次乘法/帧 | bulk copy 后趋零 | scale 乘法是主因 |
| Raw 网络带宽 | ~240 MB/s (1080p30) | — | 千兆/WiFi 都顶不住，**必须编码压缩** |
| iced BGRA→RGBA | ~2M shuffle/帧 + 8MB 上传 | SIMD 或跳过 | desktop 独有 |

**结论**：Raw 全帧在 1080p 下网络不可行（240 MB/s），**编码压缩（Tight+JPEG 或硬件
编码透传）是 ARM 可用性的硬前提**，而非可选优化。这与 issue #162 把 H.264 列为非目标
并不矛盾——本期只到 Tight+软件 JPEG，硬件编码透传是后续独立大方向。

## 5. 瓶颈优先级排序

| 排名 | 瓶颈 | 位置 | 对 ARM 的影响 |
|------|------|------|--------------|
| P0 | 只支持 Raw 全帧编码 | `server.rs:67-103` | 240 MB/s 网络 + 双倍 memcpy，WiFi/千兆都顶不住 |
| P0 | YUV→BGRA 标量转换 | `dshow_sink.rs:846-986` | ARM 弱单核跑标量 YUV 是硬伤，吃满一个核 |
| P1 | Raw 编码 per-pixel + scale 乘法 | `server.rs:96-99`, `pixel_format.rs:214-261` | 恒等变换退化成逐像素 |
| P1 | 无脏区/差分 | `connection.rs:286-289` | 静止帧仍发全屏（KVM 待机场景） |
| P2 | 每帧 BGRA Vec + Arc wrap 分配 | `dshow_sink.rs:850`, `camera.rs:293` | GC 压力 + 每帧整帧拷贝 |
| P2 | iced BGRA→RGBA per-frame + 全屏上传 | `frames.rs:6-44` | desktop 路径独有 |
| P2 | 无 TCP_NODELAY | `transport.rs` | 影响交互延迟而非吞吐 |
| P3 | 无丢帧/编码/传输指标 | 全链路 | 挡住后续优化验证 |
| P3 | `last_frame_ns` 语义错误 | `console_session.rs:178` | 指标不可信 |

## 6. 分阶段优化方案

每个阶段拆成可独立验证、独立 PR 的实施单。顺序按"先建可测量基线 → 低风险高收益 →
编码升级"。

### 阶段 0：指标埋点（必须最先做，是所有后续优化的回归基线）

对应 issue #162 验收标准 2（为每个关键指标指定测量点）。

测量点：

| 指标 | 测量点 | 现状 |
|------|--------|------|
| 源 FPS（实际发布） | `VideoSource` publish 处 atomic 计数 | 无 |
| 转换耗时 | `convert_to_bgra` / `frame_bgra` 前后 `Instant` | 无 |
| 采集耗时 | `next_frame_into` 拿到帧的等待时长 | 无 |
| watch 丢帧数 | publish 时 seq 跳跃（已知上一帧 seq） | 无（仅下游推算） |
| RFB 编码耗时 | `encode_raw_update` 前后 `Instant` | 无 |
| 输出字节数 | `send_binary` 返回的长度累计 | 丢弃 |
| updates/sec | driver 每次成功 send 计数 | 无 |
| CPU / 内存 | 外部采样（已有 PowerShell 脚本） | 已有 |
| 浏览器渲染耗时 | noVNC client / iced `FrameReady` | iced 已有 `FrameStats`；noVNC 无 |

并修 `last_frame_ns` 语义：改成 frame.timestamp（采集时间），或在 `/api/status`
同时暴露 `capture_ns`（来自 frame）和 `observe_ns`（当前值），让两者可区分。

### 阶段 1：在当前协议和依赖内可做（低风险，高收益）

**1.1 Raw 编码热路径零成本清理**（独立 issue）

- 识别"源 8bpc 且目标 8bpc"的恒等情况，直接 `extend_from_slice`/bulk copy，跳过
  `scale_channel`。
- `encode_raw_update` 复用输出 buffer（`take()` 后的 Vec 复用，避免每帧分配）。
- 设 `TCP_NODELAY`。
- 预期：720p30 编码 CPU 降一半以上，延迟下降。有现成 RFB 编码测试可回归。

**1.2 采集端 YUV→BGRA SIMD 化 + buffer 池化**（独立 issue）

- YUY2/NV12→BGRA 用可移植 SIMD（`std::simd` 或 `wide` crate），x86 SSE / ARM NEON 通用。
- `convert_to_bgra` 输出 buffer 用可复用槽；`Arc::from` 改成预分配的 `Arc<[u8]>` 池，
  消除每帧 wrap 分配。
- 在 ARM 上这一步收益最大。Y4M demo 路径的 `frame_bgra` 可缓存转换结果（loop 重复帧）。

**1.3 采集 FPS 配置真正生效**（独立 issue）

- DirectShow：通过 `IAMStreamConfig::SetFormat` 协商设备帧率（当前参数被丢弃）。
- nokhwa：`RequestedFormat` 带 FPS，验证 V4L2/AVFoundation 实际协商。
- ARM 上必须能真正协商设备帧率，否则采集线程空转。

### 阶段 2：RFB 编码升级（中等风险，决定能不能上 ARM）

**2.1 脏区追踪 + 多矩形 update**（独立 issue）

- 发布帧时附带 dirty rects（采集端对相邻帧做轻量行 hash 比对，或先用全屏占位）。
- `FramebufferUpdate` 支持多矩形（当前硬编码 1 矩形 `server.rs:78`）。
- `incremental` 请求真正发差分。

**2.2 Tight + JPEG 子编码**（独立 issue）

- noVNC 原生支持 Tight/JPEG。KVM 场景"内容大半在变"时，JPEG 质量可调把带宽从 ~240MB/s
  压到几 MB/s。`jpeg-encoder` 已是 workspace 依赖。
- **ARM 长期落点**：RK3588/树莓派等有 V4L2 M2M 硬件 JPEG/H.264 编码器，Tight/JPEG 是
  承接硬件编码的天然落点。

### 阶段 3：架构级（超出 issue #162 范围，单独立项）

**3.1 硬件视频编码（MJPEG 透传 / H.264）**——issue #162 明确列为非目标，是 ARM 盒子的
终极方案。需要先确认目标 SoC 平台，再按"通用 aarch64 + 平台最优后端"trait 分流：
通用平台走 Tight+软件 JPEG；有硬件编解码器的平台通过 feature flag 接入 MPP / V4L2 M2M。
headless 二进制在任意 aarch64 上能跑，有硬件的平台自动拿收益。

**架构前提**：当前单编码单客户端（`Semaphore::new(1)`）模型在引入多编码后端后依然成立，
阶段 2 做 Tight/JPEG 时不用先重构分发层（watch + Arc 共享设计正确）。

## 7. 测试矩阵

对应 issue #162 验收标准 4。

| 平台 | 采集源 | 客户端 | 自动化 | 备注 |
|------|--------|--------|--------|------|
| Windows | OBS 虚拟摄像头 | 无 / TCP Raw RFB / WS noVNC / iced desktop | demo release + 自动 | 阶段 1 主回归 |
| Windows | 真实采集卡 | 同上 | 人工 | OBS 不等于真卡 |
| Linux (Debian, x86) | demo Y4M | 同上 | 自动 | 无视频设备也能跑 demo |
| Linux (aarch64 ARM 盒子) | V4L2 真机 | 同上 | 人工为主 | 目标平台，阶段 1/2 收益验收主战场；先代码审查 + 配置验证 |
| macOS | AVFoundation | 同上 | 人工 | 至少代码审查 |

分辨率/FPS 组合：640×360、1280×720、1920×1080 × {10, 30} FPS。

无法稳定自动化的项：

- ARM 盒子真机 V4L2 协商、硬件编码器可用性：需目标设备，记录人工步骤。
- 真实采集卡（非 OBS）：硬件依赖。
- 浏览器端 noVNC 渲染耗时：需 browser-tests，可半自动。

Rust 改动统一遵守 `cargo fmt --all --check` 与 `cargo test --workspace --all-features`。

## 8. 后续实施单拆分（确认方案后开）

按依赖顺序，确认本调研后开以下 issue（标题/范围草案）：

1. **#issue-A：视频链路指标埋点**（阶段 0）。范围：publish_fps / convert_ns / encode_ns /
   send_bytes / updates、修 `last_frame_ns` 语义、`/api/status` 暴露 capture_ns vs observe_ns。
   依赖：无。是后续所有优化的回归基线。
2. **#issue-B：Raw 编码热路径清理 + TCP_NODELAY**（阶段 1.1）。依赖 A（需基线验证收益）。
3. **#issue-C：YUV→BGRA 可移植 SIMD + buffer 池化**（阶段 1.2）。依赖 A。
4. **#issue-D：采集 FPS 配置真正生效**（阶段 1.3）。依赖 A。
5. **#issue-E：脏区追踪 + 多矩形 update**（阶段 2.1）。依赖 A、B。
6. **#issue-F：Tight + JPEG 子编码**（阶段 2.2）。依赖 A、E；ARM 可用性硬前提。
7. **#issue-G：硬件视频编码后端（MJPEG/H.264 透传）**（阶段 3.1）。依赖 F；超 #162 范围，
   需先确认目标 SoC。

每个实施单必须包含：背景、目标、范围、验收标准、测试计划、文档影响，并 link 回本调研文档。

## 9. 文档影响

- 本调研收口后，同步 `docs/ipkvm-coarse-design.md` 的性能阶段与风险表（列出阶段 0–3
  与对应实施单）。
- 阶段 1.1/1.2 实施时，更新 DirectShow sink filter 设计文档（`2026-08-02-directshow-sink-filter-camera-capture-design.md`）的转换段。
- 阶段 2.2 实施时，更新 RFB 协议设计文档的编码协商段。

收口：本调研 PR 使用 `Closes #162`。
