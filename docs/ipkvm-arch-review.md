# my_ipkvm 架构与脚手架审查意见

日期：2026-07-31

审查对象：工作区脚手架（`Cargo.toml`、`crates/*`），对照 `docs/ipkvm-coarse-design.md` 的架构设计。

结论：模块拆分和依赖图方向正确（core 零依赖、video 独立、session 组合、两个 bin 在最上层，无循环依赖）。问题集中在几个接口契约上，现在修改成本接近零，等桌面端和 RFB 实现长出来再改代价会大很多。

最高优先级三项：**异步运行时决策、`InputSink` 契约修正、`VideoFrame` 所有权模型**。这三项是后续所有代码的地基。

## 一、实质问题

### 1.1 `InputSink` trait 契约错配

位置：`crates/ipkvm-core/src/input.rs`

- 方法与枚举重复编码：`key_down(KeyEvent)` 接受的 `KeyEvent` 自身又分 `Down`/`Up`，导致 `sink.key_down(KeyEvent::Up{..})` 这类自相矛盾的调用在类型上合法。`pointer_move(PointerEvent)`、`pointer_button(PointerEvent)`、`wheel(PointerEvent)` 同样如此，每个方法都能收到不属于自己的 variant。
  - 建议二选一：收敛为 `handle_key(KeyEvent)` / `handle_pointer(PointerEvent)` 各一个方法；或方法签名改为具体参数（`key_down(usage: u8)`、`pointer_move(x, y)`）。
- 无错误返回：设计文档明确"串口断开时输入返回错误"，但 trait 全部返回 `()`。输入路径是本项目最容易出故障的路径（拔线、波特率错误、BIOS 不识别），`Result` 必须进入 trait，否则错误无法到达 RFB 客户端和桌面状态栏。
- 坐标语义不明：`PointerEvent::Move { x: u16, y: u16 }` 没有说明是帧缓冲像素坐标还是 CH9329 的 0..4095 坐标。建议：Move 传帧缓冲像素坐标（u32），CH9329 换算留在 sink 实现内（复用 `geometry.rs` 的 `map_pointer_to_ch9329`）。这样 RFB 端（天然帧缓冲坐标）和桌面端（窗口坐标转帧缓冲坐标）共用同一路径，DPI 缩放问题只在一处处理。
- 滚轮方向约定缺失：`Wheel { delta: i16 }` 未定义符号约定（向上为正还是向下为正），需要写入契约注释。

### 1.2 `VideoFrame` 所有权模型不满足设计目标

位置：`crates/ipkvm-video/src/lib.rs`

- `data: Vec<u8>` 且 `latest_frame()` 按值返回：桌面渲染器和每个 RFB 客户端各复制一份 1080p 帧（约 8MB）。在"多查看者 + 每客户端独立节流"模型下，帧必须是共享所有权：`Arc<[u8]>`，`latest_frame` 返回 `Option<Arc<VideoFrame>>`。
- 缺帧序号：慢客户端"只拿最新帧、丢旧帧"需要单调递增 seq 判断帧是否已消费；后续脏块检测也依赖 seq 对齐。现在添加只是一个字段。
- 缺 stride：YUY2/NV12 实际行宽不一定等于 width，Media Foundation 和 V4L2 都会给出对齐后的 stride，渲染和 RFB 编码都会遇到。
- `derive(Eq, PartialEq)` 会逐字节比较整个 `data`，8MB 的相等判断没有用途，建议从 `VideoFrame` 上去掉。
- `timestamp_millis: u64` 丢失时钟来源语义。采集侧需要单调时钟（计算帧率和延迟），建议改为明确的单调时钟类型或注释说明，避免被误用为墙钟。

### 1.3 `ipkvm-rfb` 依赖 `ipkvm-session` 层次反向

按设计文档定位，"`ipkvm-rfb` 是对外协议入口，不是内部核心"。RFB 应只消费 `core` 的 `InputSink` 和 `video` 的 `FrameSource` 抽象，由 `ipkvm-headless` 把 session 的组件组装给它。

当前 rfb 依赖 session，等于协议层反向依赖编排层。以后 session 需要驱动 RFB（例如分辨率变化时通知 DesktopSize）将形成事实上的循环依赖。

建议：rfb 的依赖改为 `ipkvm-core + ipkvm-video`，session 的组装留给 headless/desktop。

连带问题：`RfbServerConfig` 包含 `http_port`（`crates/ipkvm-rfb/src/lib.rs:6`）。HTTP/noVNC 是 headless 职责，该字段应移至 headless 的配置。

### 1.4 异步运行时决策缺失

RFB TCP、WebSocket、HTTP、串口读写、采集回调全部是并发 I/O。tokio 还是裸线程加 channel，这个决策会渗透进每个 trait 的签名（async fn、回调、channel），是所有接口中最难后改的一个，必须在脚手架阶段定下来。

建议 tokio：serialport（tokio-serial）、WebSocket（tokio-tungstenite）、HTTP（axum）生态默认 tokio，逆生态走等于自研周边。决策应固定进 workspace 配置和设计文档。

### 1.5 `ConsoleSessionConfig` 与设计文档不符

位置：`crates/ipkvm-session/src/lib.rs`

设计的设置页需要选择分辨率、帧率、波特率、键盘布局；设计审查（`ipkvm-design-review.md`）又补充了鼠标模式和动态分辨率。当前 config 只有 `video_device_id` 和 `serial_port` 两个字符串。

建议至少包含：`VideoFormat`（或格式选择策略）、`baud_rate`。否则阶段 1 开工即需重构。

## 二、较小的问题

- mock 没有落脚点：阶段 0 明确承诺"模拟视频源、模拟串口、模拟帧缓冲跑通 VNC 客户端"，脚手架中没有对应位置。建议 `ipkvm-video` 增加 `mock` feature 提供测试帧源（渐变/彩条），core 增加基于脚本应答的 fake serial。这直接决定阶段 0 能否开工。
- `Ch9329Frame::new` 对超长数据 `expect` panic：协议构造库 panic 可接受，但建议返回 `Result`。后续解析响应帧（读信息回包）必然需要 fallible 代码。core 目前没有任何 Error 类型，建议引入 `thiserror`（MIT/Apache-2.0，符合白名单）统一错误定义。
- 只有 pull 没有订阅：设计写了"最新帧缓存和订阅接口"，脚手架只有 `latest_frame()`。拉模型对桌面渲染够用，RFB 节流推送用 watch channel 更自然，接口上应预留位置。
- 工作区卫生：`[workspace.dependencies]` 为空，第一批外部依赖（tokio、serialport、thiserror）进来时应集中管理版本。README 写了 `cargo fmt --check` 和 `cargo test`，但没有 CI 配置，建议尽早补一个最小配置。

## 三、确认合理的部分

- 依赖图方向正确：core 零依赖，video 独立，session 组合 core+video，desktop/headless 两个 bin 在最上层，无循环依赖。
- `Ch9329Frame` 构造与校验和逻辑正确，测试向量经验算无误。
- `map_axis` 使用 i64 中间值防溢出，clamp 边界处理正确。
- 许可证白名单、edition 2024、rust-version 配置干净。

## 四、建议的修改顺序

1. 确定异步运行时（tokio）并写入 workspace 和设计文档。
2. 修正 `InputSink` 契约（方法签名、Result、坐标语义）和 `VideoFrame` 所有权模型（Arc、seq、stride）。
3. 调整依赖方向：rfb 只依赖 core+video，`http_port` 移出 rfb 配置。
4. 补齐 `ConsoleSessionConfig` 字段。
5. 添加 mock 视频源和 fake serial，为阶段 0 铺路。
6. 建立 `[workspace.dependencies]` 和最小 CI。
