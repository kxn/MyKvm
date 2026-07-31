# RFB 输入事件泵与控制者释放生命周期设计

## 1. 文档状态

- 关联 issue：`#11`
- 状态：已实施并通过本地自动化验证
- 适用范围：单活动 RFB TCP 客户端到 `InputSink` 的输入组装
- 前置依赖：`#5` 的 RFB TCP 事件流、`#7` 的键盘映射器、`#9` 的指针映射器

## 2. 目标

本阶段把已经存在但彼此独立的 TCP 事件流、RFB 输入映射器和 CH9329 输入接口组成一个可自动验证的闭环：

1. 持续消费有界 `RfbTcpEvent` 通道，不重排、不丢弃键盘、指针和生命周期事件。
2. 一个事件泵同一时间只允许一个活动控制者。
3. 只接受当前控制者的键盘、指针、剪贴板和连续更新事件。
4. 控制者断线时调用一次 `InputSink::release_all()`。
5. 事件发送端异常关闭而控制者仍活动时，也调用 `release_all()`。
6. 释放成功后才清空控制者、键盘映射器和指针映射器。
7. sink 或生命周期错误返回原始事件，调用方可以在修复外部故障后重试。
8. 不支持的键值、暂未实现的剪贴板和连续更新请求均产生可观测结果，不静默吞掉。
9. RFB 指针事件携带其产生时的已公告帧尺寸，避免动态分辨率期间读取“最新视频尺寸”造成坐标竞态。

## 3. 不在本阶段

- 多查看者并发连接和控制权抢占。
- 控制者排队、授权或人工切换。
- `ClientCutText` 到模拟键入。
- `EnableContinuousUpdates` 的发送策略变更。
- HTTP、WebSocket/noVNC 和可直接运行的后台进程。
- 真实串口、串口重连和真实视频采集。
- 安全、鉴权、审计和输入日志。
- 桌面端与 RFB 端之间的会话级控制权仲裁。

当前 `RfbTcpServer` 顺序服务客户端，同一时间只驱动一条 TCP 连接。本阶段固定这条已实现的约束，不提前实现尚不存在的多查看者模型。以后并发查看者进入范围时，由 `ipkvm-session` 增加跨入口控制权仲裁，不能把本事件泵误当成完整会话仲裁器。

## 4. 调研结论

### 4.1 现有 TCP 事件顺序

RFB TCP 层已经保证：

- 握手完成后先发送一次 `Connected`。
- 正常输入按协议解码顺序进入有界 `mpsc` 通道。
- 通道满时等待容量，不使用 `try_send` 丢事件。
- 每个已接受连接最多发送一次 `Disconnected`。
- 握手前失败的连接只有 `Disconnected`，没有 `Connected`。
- 普通连接错误之后 server 会继续接受下一个客户端。
- 当前 server 顺序处理连接，不会同时产生两个已连接控制者的输入。

因此事件泵不需要再次排序或建立无界缓冲，只需要逐条消费并验证生命周期。

### 4.2 Tokio 通道关闭语义

项目锁定的 Tokio `1.53.1` 中：

- `Receiver::recv()` 会先排空已缓冲事件。
- 只有所有 sender 已丢弃或 receiver 被关闭，且缓冲区为空时才返回 `None`。
- `recv()` 具有取消安全性。

事件泵可以使用简单的 `while let Some(event) = receiver.recv().await` 循环。循环结束表示不会再收到 `Disconnected`，若仍有活动控制者，必须由事件泵补做释放。

### 4.3 指针坐标空间不能事后推断

RFB `PointerEvent` 的 `x`、`y` 属于客户端当前已知的帧缓冲坐标空间。视频源最新尺寸和客户端已收到的 `DesktopSize` 不是同一个时刻：

- 视频源可能已经切换到新尺寸。
- server 可能尚未向客户端公告新尺寸。
- 同一批 TCP 输入中可能先有更新请求，再有客户端按旧尺寸发送的指针消息。

事件泵若在收到指针事件后读取最新视频帧尺寸，会把旧坐标错误映射到新尺寸。尺寸必须在 RFB 连接核心产生指针事件时固化。

### 4.4 键盘映射错误分类

现有 `RfbKeyboardError` 包含两类不同性质：

- 客户端输入无法表示：不支持的 keysym、活动字符 Shift 需求冲突。
- sink 拒绝：CH9329 报告、队列或输入状态错误。

前一类不应终止整个事件泵，否则一个扩展键就会让后续键盘释放和断线释放无人处理。后一类表示输入链路未完成提交，必须停止当前循环并交给上层处理。

## 5. 模块边界

### 5.1 `ipkvm-rfb`

负责：

- 在 `RfbEvent::Pointer` 中附带当前客户端输入坐标尺寸。
- 继续保持传输无关，不依赖 Tokio、headless 或 CH9329。

不负责：

- HID 映射。
- 控制者生命周期。
- 调用 `InputSink`。

### 5.2 `ipkvm-headless::rfb_tcp`

负责：

- 把 `RfbEvent::Pointer` 的帧尺寸原样复制到 `RfbTcpEvent::Pointer`。
- 保留客户端编号和 TCP 生命周期顺序。

不负责：

- 读取最新视频帧来补尺寸。
- 键鼠状态和释放策略。

### 5.3 `ipkvm-headless::rfb_input`

新增 `pump` 模块，负责：

- 当前 RFB 控制者。
- `RfbKeyboardMapper` 和 `RfbPointerMapper` 的连接级状态。
- RFB TCP 事件到 `InputSink` 的串行分发。
- 断线和事件源关闭时释放。
- 可继续的客户端拒绝、被忽略事件和成功结果的可观测通知。
- 失败事件的原样返还。

### 5.4 `ipkvm-core`

本阶段不修改 `InputSink` 契约。事件泵只调用：

- `handle_key_batch()`，由键盘映射器间接调用。
- `handle_pointer_batch()`，由指针映射器间接调用。
- `release_all()`。

## 6. 指针坐标时期

`RfbEvent::Pointer` 和 `RfbTcpEvent::Pointer` 增加：

```rust
framebuffer_size: RfbSize
```

`RfbConnectionCore` 增加三个不同概念：

```text
announced_size           服务端已经成功排队的最新 DesktopSize
input_coordinate_size    当前客户端输入字节流使用的坐标尺寸
pending_input_size       等待输入解码器跨过已有半包后生效的新尺寸
```

`RfbConnectionCore::push_normal_input()` 在把已解码消息转换为 `RfbEvent` 时读取 `input_coordinate_size`。这样：

- 首次握手后的指针使用 `ServerInit` 公告的尺寸。
- `DesktopSize` 成功排队且输入解码器没有半包时，后续解码的指针使用新尺寸。
- `DesktopSize` 成功排队但解码器保留着旧时期开始的半包时，把新尺寸保存到 `pending_input_size`。
- 半包和同一次 `push_input()` 随后已经到达的消息继续使用旧尺寸；解码器缓冲清空后，新尺寸从下一次 `push_input()` 开始生效。
- 同一次 `push_input()` 已经解码出的多个事件使用该次处理开始时的输入坐标尺寸；应用层随后处理更新请求时不会回写已经产生的指针事件。
- 事件泵不持有 `FrameSource`，也不读取视频尺寸。

RFB 没有客户端对 `DesktopSize` 的确认消息。上述规则选择保守边界：已经开始接收的半包和同批输入仍按旧尺寸解释，只有清晰的后续 `push_input()` 才切换。它不能推断客户端实际读取服务端响应的时刻，但可以保证同一消息不会因 TCP 分片和应用调度跨越两个坐标时期。

事件泵将 `RfbSize` 无损转换为：

```rust
FramebufferSize {
    width: u32::from(size.width()),
    height: u32::from(size.height()),
}
```

`RfbSize` 已保证宽高非零。坐标是否越界仍由 core 的既有校验拒绝，不在事件泵裁剪。

## 7. 事件泵状态

```text
RfbInputPump<S>
  sink: S
  active: Option<ActiveController>
  keyboard: RfbKeyboardMapper
  pointer: RfbPointerMapper

ActiveController
  client_id: RfbClientId
  peer_addr: SocketAddr
```

约束：

- `active == None` 时两个映射器必须是默认空状态。
- `active != None` 时只有相同 `client_id` 的连接级事件可以改变输入状态。
- 只提供只读 `sink()` 和 `active_client()`，便于状态展示和测试。
- 不提供活动状态下取走或可变借用 sink 的接口，防止绕过事件泵并破坏状态对应关系。
- 不在 `Drop` 中调用 `release_all()`；析构无法可靠上报错误，正确关闭必须显式运行事件泵或调用释放方法。

## 8. 公共处理接口

建议公共类型：

```rust
pub struct RfbInputPump<S> { /* ... */ }

impl<S: InputSink> RfbInputPump<S> {
    pub fn new(sink: S) -> Self;
    pub fn active_client(&self) -> Option<RfbClientId>;
    pub fn sink(&self) -> &S;
    pub fn handle_event(
        &mut self,
        event: RfbTcpEvent,
    ) -> Result<RfbInputNotice, RfbInputEventError>;

    pub fn release_active(
        &mut self,
    ) -> Result<Option<RfbInputNotice>, RfbInputError>;

    pub async fn run<F>(
        &mut self,
        receiver: &mut mpsc::Receiver<RfbTcpEvent>,
        observe: F,
    ) -> Result<(), RfbInputRunError>
    where
        F: FnMut(&RfbInputNotice);
}
```

`handle_event` 取得事件所有权。失败时 `RfbInputEventError` 保存：

- 原始 `RfbTcpEvent`。
- 类型化 `RfbInputError`。

原事件只在失败路径装箱，避免大型 `RfbDisconnectReason` 扩大每次正常处理返回的 `Result`。

调用方可取回原事件并重试，不能因为从通道取出事件就永久丢失尚未提交的按键、按钮或释放。

`run` 借用 pump 和 receiver，不消费它们。失败返回后：

- pump 的活动状态仍可检查。
- receiver 中尚未消费的事件仍保留。
- 失败对象持有当前失败事件。
- 调用方可以修复 sink、重试失败事件，再继续调用 `run`。

观察回调是同步、无失败返回的轻量通知出口。当前阶段用它记录测试结果和未来状态统计，不在事件泵内部创建第二个无界队列。

## 9. 事件处理表

### 9.1 `Connected`

无活动控制者：

1. 确认两个 mapper 为空状态。
2. 保存 `client_id` 和 `peer_addr`。
3. 在 `ControllerAcquired` 通知中保留本次连接的 `shared`。

已有活动控制者：

- 返回 `ControllerAlreadyActive` 生命周期错误。
- 不释放旧控制者。
- 不接受新控制者。

当前顺序 server 不应产生该错误。将其视为组装契约被破坏，比隐式抢占更安全。

### 9.2 `Key`

要求 `client_id` 等于活动控制者：

- `Applied`、重复按下、未知释放和锁定键忽略均返回 `Keyboard` 通知。
- 不支持 keysym 返回 `KeyboardRejected::UnsupportedKeysym` 通知并继续。
- Shift 需求冲突返回 `KeyboardRejected::ConflictingShiftRequirements` 通知并继续。
- `RfbKeyboardError::Input` 转换为 sink 错误并返还原事件。

mapper 已保证拒绝和 sink 失败时不提交内部状态。

### 9.3 `Pointer`

要求 `client_id` 等于活动控制者：

1. 把事件携带的 `RfbSize` 转为 `FramebufferSize`。
2. 调用 `RfbPointerMapper`。
3. `Applied` 和 `AppliedIgnoringButtons` 均返回 `Pointer` 通知。
4. `RfbPointerError::Input` 转换为 sink 错误并返还原事件。

未支持按钮保持非致命、可观测；坐标越界和 sink 错误保持致命、可重试。

### 9.4 `CutText`

要求来自活动控制者。当前不模拟键入，返回：

```text
CutTextIgnored { client_id, byte_count }
```

通知只包含长度，不复制文本内容。

### 9.5 `ContinuousUpdates`

要求来自活动控制者。该事件属于显示更新能力，不改变输入状态，返回包含 `enable` 和矩形的忽略通知，供未来视频策略接入。

### 9.6 `Disconnected`

没有活动控制者：

- 这是合法的握手前断线。
- 不调用 `release_all()`。
- 返回 `PreHandshakeDisconnected`。

活动控制者编号不同：

- 返回 `WrongController` 生命周期错误。
- 不释放当前控制者。

活动控制者编号相同但地址不同：

- 返回 `PeerAddressChanged` 生命周期错误。
- 不释放当前控制者。

活动控制者完全匹配：

1. 调用 `sink.release_all()`。
2. 成功后清空活动控制者。
3. 用全新的默认 mapper 替换键盘和指针 mapper。
4. 返回带断线原因的 `ControllerReleased`。

释放失败时不执行步骤 2 和 3，原始 `Disconnected` 随错误返回。重试同一事件会再次尝试释放。

### 9.7 事件源关闭

`receiver.recv()` 返回 `None` 前已排空通道。

- 没有活动控制者：`run` 正常返回。
- 有活动控制者：调用与断线相同的释放路径，原因为 `EventSourceClosed`。
- 释放成功：发出 `ControllerReleased` 通知并正常返回。
- 释放失败：返回 `SourceClosedRelease`，保留活动控制者和 mapper；调用方可使用 `release_active()` 重试。

## 10. 通知模型

`RfbInputNotice` 至少包含：

- `ControllerAcquired`
- `Keyboard`
- `KeyboardRejected`
- `Pointer`
- `CutTextIgnored`
- `ContinuousUpdatesIgnored`
- `PreHandshakeDisconnected`
- `ControllerReleased`

`RfbControllerReleaseReason`：

- `Disconnected(RfbDisconnectReason)`
- `EventSourceClosed`
- `Explicit`

通知用于状态、诊断和测试，不改变控制流。安全审计和持久化日志不在本阶段。

## 11. 错误模型

### 11.1 生命周期错误

`RfbInputLifecycleError`：

- `ControllerAlreadyActive { active, incoming }`
- `NoActiveController { incoming, event_kind }`
- `WrongController { active, incoming, event_kind }`
- `PeerAddressChanged { client_id, expected, actual }`

`event_kind` 使用固定枚举，不把整个事件格式化进错误文本。

### 11.2 sink 错误

`RfbInputError::Sink` 保存：

- `client_id`
- `operation`：键盘、指针或释放
- 原始 `InputError`

不在事件泵内无限重试。真实串口重连策略尚未设计，自动重试可能重复已经到达硬件但应答丢失的输入。

### 11.3 事件与运行错误

`RfbInputEventError` 保存原事件和 `RfbInputError`。

`RfbInputRunError` 区分：

- `Event(RfbInputEventError)`
- `SourceClosedRelease(RfbInputError)`

所有错误必须向调用方返回，不允许只打印后继续。

## 12. 失败原子性

### 12.1 键盘和指针事件

- mapper 先在候选状态计算。
- core sink 先在候选状态生成命令。
- 命令队列接受后，core 和 mapper 才依次提交状态。
- 事件泵没有额外的提前提交状态。
- 失败对象保留原事件。

### 12.2 释放

`Ch9329InputSink::release_all()` 已保证队列接受前不清空键鼠状态。事件泵在它成功前也不清空控制者和 mapper，因此失败后可以重试完整释放。

### 12.3 新控制者

新连接只能在旧控制者已经成功释放并清空后获得控制权。禁止发生：

- 旧控制者释放失败。
- 事件泵仍把新连接设为活动控制者。
- 两套 mapper 状态共用同一个 sink。

## 13. 自动化测试

### 13.1 RFB 协议核心

- 首次指针事件携带 `ServerInit` 公告尺寸。
- `DesktopSize` 成功提交后，后续指针事件携带新尺寸。
- 同批次更新请求之后的指针事件保留该批输入开始时的旧尺寸。
- 指针消息任意分片后尺寸字段稳定。

### 13.2 TCP 事件转换

- `RfbEvent::Pointer` 的尺寸不变地进入 `RfbTcpEvent::Pointer`。
- 动态分辨率期间不从 `FrameSource` 重新查询指针尺寸。
- 现有输入事件顺序和事件通道反压测试继续通过。

### 13.3 纯内存事件泵

- 握手前断线不释放。
- 连接后键盘和指针只进入一个 sink。
- 不支持 keysym 产生拒绝通知，后续合法输入继续处理。
- 剪贴板和连续更新产生忽略通知。
- 无活动控制者的输入被拒绝。
- 非当前控制者事件被拒绝。
- 重复连接被拒绝且不隐式抢占。
- 断线调用一次释放，成功后 mapper 重置。
- 第二个控制者可以再次按下与前一个相同的键和按钮。
- sink 拒绝键盘或指针时，错误返还原事件，重试结果相同。
- 释放失败时控制者和 mapper 不清空，重试断线事件后成功释放。
- 事件源关闭时仍活动会释放。
- 事件源关闭时释放失败可通过显式释放重试。

### 13.4 真实 CH9329 sink

使用 `Ch9329InputSink<FakeCommandQueue>`：

- 连接、键盘按下、指针按下、断线形成有序命令批次。
- 最后一个批次同时包含全零键盘报告和鼠标按钮释放报告。
- 释放批次队列失败时 sink 与事件泵均保留状态。

### 13.5 真实回环 TCP 闭环

启动：

- `MockFrameSource`
- `RfbTcpServer`
- 有界事件通道
- `RfbInputPump<Ch9329InputSink<FakeCommandQueue>>`
- 独立实现的最小 RFB 测试客户端

验证：

1. 完成 RFB 3.8 握手。
2. 发送键盘按下和指针左键按下。
3. 关闭客户端。
4. server 发送 `Disconnected`。
5. 事件泵提交键盘、指针和最终释放批次。
6. server shutdown 后事件通道关闭，事件泵正常返回。

## 14. 备选方案及否决

### 14.1 事件泵读取最新视频尺寸

否决。动态尺寸切换时，最新视频尺寸不等于客户端已公告尺寸，会产生边缘误点击。

### 14.2 只在正常 `Disconnected` 时释放

否决。server 任务异常退出或 sender 被关闭时可能没有后续断线事件，必须在事件源关闭时补做释放。

### 14.3 mapper 遇到任何错误都终止事件泵

否决。不支持的扩展 keysym 和 Shift 冲突属于单条客户端输入拒绝，终止会阻止后续释放事件。

### 14.4 sink 错误后自动无限重试

否决。真实串口尚未定义幂等确认边界，无限重试可能重复注入，且会永久阻塞事件通道。

### 14.5 释放失败后直接清空软件状态

否决。目标机可能仍保持按键或按钮，丢掉 mapper 和控制者状态后无法可靠重试。

### 14.6 在 `Drop` 中释放

否决。析构无法返回 `InputError`，也不能保证异步进程退出顺序。释放必须是显式生命周期步骤。

### 14.7 在本阶段实现多查看者控制权

否决。当前 TCP server 是顺序单连接模型。提前增加观察者集合和抢占策略没有可运行入口，还会混淆 RFB 连接级状态与未来 session 级仲裁。

## 15. 实施顺序

1. 给 RFB 指针事件增加已公告帧尺寸并固定动态尺寸语义。
2. 定义事件泵通知、错误和控制者状态。
3. 用红灯测试实现逐事件生命周期和映射分发。
4. 实现通道运行循环与事件源关闭释放。
5. 使用真实 CH9329 fake 队列验证释放原子性。
6. 使用真实回环 TCP 验证完整输入闭环。
7. 回写 README、粗粒度设计和专项设计状态。
8. 运行统一本地验证、自审并通过 PR 合并。

## 16. 自审清单

- [x] 指针尺寸来自客户端已公告状态，不读取最新视频尺寸。
- [x] 跨越尺寸切换的输入半包不会在消息中途切换坐标时期。
- [x] 协议 core 仍不依赖 Tokio、headless 或 CH9329。
- [x] 当前只承诺 RFB 单活动控制者，不冒充 session 级仲裁。
- [x] 通道有界并沿用现有无损反压。
- [x] 不支持键值为可继续、可观测拒绝。
- [x] sink 和生命周期错误不被吞掉。
- [x] 失败对象保留原始事件。
- [x] 断线和事件源关闭都触发释放。
- [x] 释放成功前不清空控制者或 mapper。
- [x] 新控制者不会覆盖未释放的旧控制者。
- [x] 不在 `Drop` 中执行不可报告的释放。
- [x] 不新增外部依赖。
- [x] 关键路径均可使用自动化测试覆盖。
- [x] 所有文档说明使用中文。

## 17. 资料

- [RFC 6143 第 7.5.5 节 PointerEvent](https://www.rfc-editor.org/rfc/rfc6143#section-7.5.5)
- [RFC 6143 第 7.8.2 节 DesktopSize](https://www.rfc-editor.org/rfc/rfc6143#section-7.8.2)
- [Tokio 1.53.1 有界 mpsc Receiver 文档](https://docs.rs/tokio/1.53.1/tokio/sync/mpsc/struct.Receiver.html#method.recv)
- [现有 RFB TCP 设计](2026-07-31-rfb-tcp-transport-design.md)
- [现有 RFB 键盘映射设计](2026-07-31-rfb-keyboard-mapping-design.md)
- [现有 RFB 指针映射设计](2026-07-31-rfb-pointer-mapping-design.md)
- [现有粗粒度设计](../../ipkvm-coarse-design.md)
