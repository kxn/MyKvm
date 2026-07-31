# RFB 连接取消安全设计

## 1. 文档状态

- 关联 issue：`#15`
- 状态：已实施
- 适用范围：TCP 与 WebSocket 共用的单活动 RFB 连接生命周期
- 前置设计：`2026-07-31-rfb-websocket-transport-design.md`

## 2. 问题定义

现有 `RfbConnectionPermit` 直接持有 Tokio `OwnedSemaphorePermit`。这能保证正常路径在
`Disconnected` 成功进入事件队列后释放闸门，但无法区分以下两种析构：

1. WebSocket 升级失败、TCP 尚未进入 RFB 生命周期，此时应当释放预约。
2. RFB 连接已经激活，但连接任务、服务任务或断开收尾任务被取消或 panic，此时不能释放。

第二种情况下，Rust future 被丢弃会连带析构 `OwnedSemaphorePermit` 并自动归还容量。
输入泵可能仍保存旧控制者和已按下的键鼠状态，新连接却已经可以发送 `Connected`。输入泵
会因 `ControllerAlreadyActive` 停止，且无法保证旧输入状态已经 `release_all`。这是连接
所有权模型的错误，不是给某个调用点补一条错误处理可以解决的问题。

## 3. 安全不变量

1. 未激活的连接预约被丢弃时必须自动归还容量。
2. 已激活连接只有在对应 `Disconnected` 成功进入事件队列后才能归还容量。
3. 已激活连接的任务被取消、panic、事件接收端关闭或收尾 future 被丢弃时必须关闭失败，
   即把该服务实例的闸门置为可观察的中毒状态。
4. `Disconnected` 入队成功与闸门归还之间不得存在 `.await` 或其他取消点。
5. TCP 与 WebSocket 必须调用同一份断开收尾函数，不能各自实现释放顺序。
6. 公共 `RfbDisconnectReason` 只保留稳定、可克隆的错误分类；底层 WebSocket 错误源必须
   在私有错误链中保留，供诊断使用。

## 4. 所有权模型

连接闸门内部继续使用容量为 1 的 `Semaphore`，但把令牌拆成两个阶段。

### 4.1 预约

`RfbConnectionReservation` 持有：

- `Option<OwnedSemaphorePermit>`
- 共享 `GateInner`
- 已分配的 `RfbClientId`

预约用于 TCP 已接受但尚未进入连接驱动，以及 WebSocket 已返回升级响应但
`on_upgrade` 回调尚未开始的阶段。预约普通析构会按 Tokio 语义自动归还容量。

### 4.2 已激活租约

`RfbConnectionReservation::activate()` 同步执行：

1. 取出 `OwnedSemaphorePermit`。
2. 调用 `forget()`，永久取消 Tokio 的自动归还行为。
3. 返回 `RfbConnectionLease`，其中只保存共享 `GateInner` 和客户端标识。

`RfbConnectionLease` 不实现 `Clone`，全部字段私有。普通析构不归还容量，而是关闭
semaphore，把闸门置为永久中毒状态并唤醒正在等待的 TCP 任务。唯一归还接口消费租约、
清除析构时的中毒动作并同步调用 `Semaphore::add_permits(1)`。该接口使用
`pub(in crate::rfb_connection)`，只允许共享连接模块调用。

`RfbConnectionGateError` 增加 `Poisoned`。中毒后，WebSocket 升级返回空的 `503`，
等待中的 TCP 任务立即醒来并返回类型化服务端错误，新进入的 TCP/WS 也得到相同的不可用
结果。这样监督器可以观察异常并重启整个服务实例，而不会把异常状态误报为普通连接冲突。

这一设计有意选择关闭失败：一旦已激活任务以未证明清理完成的方式消失，该服务实例不再
接纳控制连接。当前系统还没有输入泵恢复握手，自动放行会破坏更重要的单控制者与输入释放
不变量。

## 5. 共享断开收尾

新增 `rfb_connection/finalize.rs`。共享 owner 返回不可克隆的
`RfbConnectionCompletion`，其中封装租约、对端地址和 `ConnectionEnd`；crate 私有的
`finalize_connection(event_tx, completion)` 消费该完成值：

1. 从 `ConnectionEnd` 取得稳定断开原因；事件通道已关闭时返回类型化错误。
2. 使用租约中的客户端标识发送唯一一次 `RfbServerEvent::Disconnected`。
3. 发送成功后立即同步消费租约并归还闸门容量。

发送等待期间租约由收尾 future 持有。future 被取消或发送失败时租约普通析构，闸门进入
中毒状态。发送成功后的同一次 poll 内没有新的 `.await`，因此不会出现事件已入队但许可
未归还的中间取消点。

共享驱动在任何 `.await` 前接收预约并完成激活，返回一个同时持有 `ConnectionEnd` 与
租约的不可克隆完成值；TCP 和 WebSocket 都不能直接取得或释放租约。共享收尾函数消费该
完成值，并在成功时返回 `ConnectionEnd`，因此 TCP 仍能判断是否因 `ServerShutdown`
退出接受循环。

TCP 服务把收尾错误映射为现有 `RfbTcpServerError::EventChannelClosed`。WebSocket
升级任务没有可向 HTTP 调用方返回的错误通道；它仍调用同一收尾函数，错误会导致闸门关闭，
而不是静默放行新的控制者。

## 6. WebSocket 错误链

私有 `RfbTransportError::WebSocket` 必须保存一个满足
`Error + Send + Sync + 'static` 的底层错误源。axum WebSocket 的接收和发送错误原样
装箱进入该变体。共享驱动只在生成公共事件时把它降级为
`RfbDisconnectReason::WebSocket`。

这同时满足：

- 公共事件可克隆且不依赖 axum 类型。
- 内部错误链保留根因。
- TCP 连接驱动不反向依赖 WebSocket 模块。

## 7. 自动化验证

闸门单元测试必须确定性证明：

1. 未激活预约析构后可以再次取得预约。
2. 已激活租约显式释放后可以再次取得预约。
3. 每次正常释放后持有第一份新预约时，第二次取得仍为 `Busy`；循环多个生命周期后容量
   始终严格等于 1。
4. 已激活租约普通析构后，等待者被唤醒且所有后续取得都返回 `Poisoned`。

共享收尾单元测试必须使用容量为 1 且已填满的事件通道：

1. 首次 poll 为 `Pending` 时闸门保持 `Busy`。
2. 释放通道容量后收到准确的 `Disconnected`，随后闸门开放。
3. 首次 poll 为 `Pending` 后直接丢弃收尾 future，闸门进入 `Poisoned`。
4. `ConnectionEnd::EventChannelClosed` 没有断开原因时返回类型化错误，不发送事件并毒化
   闸门。
5. 事件接收端已关闭导致发送失败时返回类型化错误，不发送事件并毒化闸门。

共享连接 owner 测试必须先完成真实 RFB 握手并观察 `Connected`，再中止实际 owner
future，断言闸门进入 `Poisoned`。TCP 与 WebSocket 调用点都只能把预约交给这一共享
owner，保证激活发生在连接驱动的第一个 `.await` 前。

WebSocket 传输单元测试或共享驱动测试必须遍历 `std::error::Error::source()`，证明私有
错误链保留底层错误，同时公共断开原因仍等于 `WebSocket`。现有 TCP、WebSocket、noVNC
固定样本和跨传输排他性集成测试全部保留。跨传输回环测试只负责端到端行为，不再承担证明
取消时序的职责。

## 8. 同步修正

- `RfbFrameError::UnsupportedPixelFormat` 文案改为传输无关的
  “RFB requires BGRA8888”。
- 文档明确 `tokio-tungstenite` 与 `futures-util` 是直接开发依赖；前者由 axum `ws`
  功能引入正常生产依赖树，后者是 axum 的正常依赖并且还经 tower 进入。不能声称它们
  不进入生产依赖。
- 前置 WebSocket 设计中的许可生命周期、风险和测试说明按本设计修正。

## 9. 自审结论

- **正确性：** 两阶段类型使“尚未激活”和“已经激活”在类型与析构语义上不可混淆，租约
  不可克隆且只能由共享收尾消费。
- **取消安全：** 连接驱动中的任意 `.await` 被取消都会通过租约析构毒化闸门；正常收尾的
  入队与释放区间只有 `Disconnected` 发送一个取消点，发送成功后同步释放。
- **并发性：** 激活和显式释放都是同步操作，不增加锁和异步竞态；循环测试固定 semaphore
  容量不漂移。
- **兼容性：** RFB 线级协议和正常路径 HTTP 状态不变；新增 `Poisoned` 服务级错误与
  WebSocket `503` 使异常状态可观察。
- **可测试性：** 核心不变量由直接 poll future 的确定性单元测试覆盖，不依赖网络调度。
- **恢复性：** 异常取消后返回可观察的 `Poisoned`/`503`/TCP 服务级错误，需要重启服务
  实例；这是当前架构下明确且可监督的关闭失败策略。

结论：设计可实施。
