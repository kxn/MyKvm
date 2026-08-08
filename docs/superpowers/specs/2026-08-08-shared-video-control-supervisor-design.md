# 共享视频与控制恢复状态机调研设计

日期：2026-08-08

关联 issue：[#55](https://github.com/kxn/MyKvm/issues/55)

状态：已实现（#55 分支）。本文保留实施前调研事实，并记录落地后的共享状态机行为、剩余边界和验证要求。

## 1. 背景

当前桌面端在目标机进入 Windows、重启或 CH9329/USB HID 短暂掉电时，偶发从工作画面直接退回连接界面。退回后连接页预览仍然能正常工作，说明视频采集路径可能已经恢复，真正触发主会话退出的是控制输入链路或会话状态机。

用户期望的行为是：

- 视频和控制是两条独立生命周期。
- 一条链路异常时，另一条链路不能被无条件掐掉。
- 除人工停止和明确不可恢复错误外，底层应尽量自动恢复。
- 重试耗尽后仍停留在视频页，显示控制设备失败提示，不回连接页。
- web 和 desktop 必须使用同一套底层恢复逻辑，不能各自维护一套状态机。

本设计采用彻底方案：拆分视频链路和控制链路的恢复职责，并在共享层实现统一 supervisor。中间方案（只把 headless 恢复循环搬给 desktop 复用）虽然改动小，但仍把视频和控制绑在同一个 session 里，会继续为后续挖坑，因此不采用。

## 1.1 已落地结论

- `ipkvm-session` 提供共享 `FrameHub` 和 `SessionSupervisor`，headless 与 desktop 都通过同一套 supervisor 驱动恢复。
- 视频源由 `FrameHub` 提供稳定订阅；底层视频源替换或清空时，上层 `watch` 订阅不关闭。
- 控制链路由 supervisor 内部的 `SessionManager` 管理；控制 sink 创建失败或输入泵失败只影响控制状态，不销毁 `FrameHub`。
- headless `SessionFactory` 与 desktop `DesktopSessionFactory` 要求显式实现 `build_video()` 和 `build_control()`，避免 adapter 用一次性构造悄悄把两条链路重新耦合。
- RFB WS/TCP 在控制 sender 缺失或关闭时仍允许观看连接，输入事件按只读语义忽略。
- headless 删除私有 `web/recovery.rs` 和 `SwitchableFrameSource`；`/api/status` 暴露 `video.runtime` 与 `session.control`。
- desktop 删除输入泵失败后的自动 `stop()` 路径；主界面是否停留工作页由 `SupervisorStatus::should_show_work_view()` 决定。
- 初始 connect/create/restart 时 video/control 构建错误也进入 supervisor 恢复态，不再直接 500 回滚或退回连接页。
- 人工 `stop` 是返回连接页的明确路径；控制恢复耗尽后仍停留视频/工作页并显示控制失败。

## 2. 实施前代码事实

### 2.1 session 层仍把视频和输入组装成一个会话

`ipkvm-session::SessionManager` 当前只有三个状态：

- `Absent`
- `Stopped`
- `Running`

`ConsoleSession::new(frame_source, sink, gate, event_publisher)` 同时持有 `FrameSource` 和 `InputSink`。输入泵失败时，`ConsoleSession` 把 `running` 置为 `false`，并在 `stats.input_offline` 里记录原因。`SessionManager::state()` 只从 `ConsoleSession::is_running()` 推断状态，因此输入泵退出后整个 session 变成 `Stopped`，而不是“控制离线、视频仍可用”。

### 2.2 headless 有自己的恢复循环

`crates/ipkvm-headless/src/web/recovery.rs` 维护了 headless 私有恢复策略：

- `manual_stop` 为 true 时不恢复。
- session `running` 时清空失败计数。
- session `stopped` 且 `input_offline` 存在时，按指数退避重建整个会话。
- session `stopped` 且视频从未出帧超过阈值时，也重建整个会话。
- 视频曾出帧后停滞只报告，不重启。

这解释了网页版更稳：前端 `status.js` 只有 `manual_stop` 或 `absent` 才切回连接页；`stopped` 仍保持视频页等待恢复。

但这套逻辑仍然是 headless 私有，而且重建的是整个 `(frame_source, sink)` 会话，不是真正拆分视频和控制。

### 2.3 desktop 有另一套相反逻辑

`crates/ipkvm-desktop-iced/src/app.rs::sync_status()` 当前在发现 `!controller.is_control_online()` 且 `input_offline_reason().is_some()` 时，自动调用 `controller.stop()`。

`DesktopSessionController::stop()` 会同时清理：

- `event_tx`
- `frame_source`
- pending input events
- `SessionManager` 持有的整个会话

随后 iced `main_view()` 只看 `controller.is_control_online()`。控制离线后主界面直接切到连接页。预览能正常工作，是因为连接页预览走独立 `PreviewRuntime`，不依赖正式 session 的 `frame_source`。

### 2.4 #53 只修了帧订阅冻结，不是恢复状态机

`fix/52-frame-freeze` / PR #53 修复的是：

- `FrameClosed` 把 `subscribed=false` 后，成功 `Connect` 没有恢复为 true。
- 旧帧 `handle/frame_size/latest_frame` 没有一起清理。

这个修复是必要的，但它只解决“帧源关闭后手动重连主画面永久冻结”。它不解决输入链路失败导致整个 session 被销毁、desktop 回连接页的问题。

### 2.5 headless 的 `SwitchableFrameSource` 不是稳定帧 hub

`SwitchableFrameSource::set_current()` 可以替换当前帧源，但 `subscribe()` 返回的是“当前源”的 watch receiver。已经建立的订阅不会自动迁移到新源。它适合 headless 当前“新 RFB 连接读新帧源”的模型，但不适合彻底拆分后让 desktop 和 web 都持有长期稳定的视频订阅。

共享层需要一个真正稳定的 `FrameHub`：上层订阅 hub，底层视频 runtime 替换具体 frame source 时，hub 继续向同一个 watch channel 发布新帧或无信号状态。

## 3. 目标

- 在共享层实现同一套视频与控制生命周期状态机，desktop/headless 只做 UI/API 适配。
- 视频 runtime 与控制 runtime 分离：任一方失败不能无条件销毁另一方。
- 控制失败时保留视频页和视频订阅；输入暂停，状态显示恢复中或失败。
- 视频失败时保留控制状态；输入是否允许继续由状态策略明确决定，默认在无有效帧坐标时暂停绝对指针。
- 自动恢复使用统一退避策略；人工停止不自动复活。
- 重试耗尽后停留在视频页，显示失败状态，不回连接页。
- RFB、desktop 本地输入、状态 API 和 iced 状态栏都从同一份 supervisor snapshot 派生。
- 不把鼠标绝对坐标、OS profile 或 Pointer Lock 问题混入本次状态机重构。

## 4. 非目标

- 不在本 issue 中重新设计 BIOS 绝对鼠标坐标域。
- 不改变 CH9329 `0x04/0x05` 报文字段语义。
- 不要求无限重试；需要明确可配置或固定的重试耗尽语义。
- 不要求一次性完成所有 UI 文案美化；本 issue 只要求状态行为正确且可诊断。

## 5. 目标架构

### 5.1 新共享层：`SessionSupervisor`

建议放在 `ipkvm-session`，因为它已经依赖：

- `ipkvm-core`
- `ipkvm-video`
- `ipkvm-rfb`

且不依赖 desktop UI、HTTP、设备枚举或平台 adapter。具体设备创建通过 trait 注入，避免共享层依赖 `ipkvm-device` 或 iced/axum。

核心结构：

```text
SessionSupervisor
  ├─ VideoRuntime
  │   ├─ VideoFactory
  │   ├─ FrameHub
  │   └─ VideoRecoveryPolicy
  ├─ ControlRuntime
  │   ├─ ControlFactory
  │   ├─ RfbConnectionGate
  │   ├─ event_publisher
  │   └─ ControlRecoveryPolicy
  ├─ current_selection
  ├─ intent/manual_stop
  ├─ status watch
  └─ action API: start/restart/stop/change_selection/change_mouse_mode
```

### 5.2 `FrameHub`

`FrameHub` 替代 headless 私有 `SwitchableFrameSource`，并提供稳定订阅语义：

- 上层永远订阅 `FrameHub`。
- `VideoRuntime` 可以替换内部 `Arc<dyn FrameSource>`。
- hub 内部任务把当前源的新帧转发到自己的 watch sender。
- 当前源关闭时，hub 发布 `VideoState::Recovering` 或 `NoSignal`，但不关闭上层订阅。
- RFB 和 desktop 都不需要处理“源替换导致订阅断开”。

这能从结构上消掉 #53 同类问题，而不是每个 UI 自己维护 `subscribed`。

### 5.3 `VideoRuntime`

`VideoRuntime` 独立管理视频设备：

```text
Idle
  -> Starting
  -> Streaming
  -> Stalled
  -> Recovering
  -> Failed
```

建议规则：

- 创建源失败：进入 `Recovering`，按退避重试打开视频源。
- 源从未出帧超过阈值：进入 `Recovering`，可重建视频源。
- 曾经出帧后短暂停滞：进入 `Stalled`，默认不立刻重建，以适配目标机重启/BIOS 切分辨率。
- 源明确关闭或采集线程退出：进入 `Recovering`，重建视频源。
- 重试耗尽：进入 `Failed`，但 `FrameHub` 仍存在；UI 停留在视频页，显示视频失败或无信号。

### 5.4 `ControlRuntime`

`ControlRuntime` 独立管理输入 sink 和输入泵：

```text
Idle
  -> Starting
  -> Ready
  -> Recovering
  -> Failed
```

建议规则：

- `InputSink` 创建失败：进入 `Recovering`，按退避重试控制设备。
- `RfbInputPump` 因 sink 错误退出：进入 `Recovering`，销毁并重建控制 sink/输入泵。
- 恢复期间不接受新输入；desktop 直接拒绝/忽略，web/RFB 对输入 no-op 或返回可诊断 notice。
- 恢复成功后发布新的 `event_publisher` sender，并以空输入状态继续。
- 恢复成功后先尽力发送全释放；如果失败，不依赖旧状态，仍以空输入状态作为软件基线。
- 重试耗尽：进入 `Failed`，视频页保持，状态显示控制设备失败；用户可以手动重试、刷新设备或重新选择控制设备。

### 5.5 顶层 `SupervisorStatus`

共享状态快照建议包含：

```rust
pub struct SupervisorStatus {
    pub intent: SessionIntent,
    pub video: VideoStatus,
    pub control: ControlStatus,
    pub recovery: RecoveryStatus,
    pub active_controller: Option<ActiveController>,
}
```

语义示例：

```text
SessionIntent:
  ManualStopped
  Running
  Recovering
  Failed

VideoStatus:
  Idle
  Starting
  Streaming { frame, source, stats }
  Stalled { last_frame_ns, source }
  Recovering { reason, attempt, next_retry }
  Failed { reason, attempts_exhausted }

ControlStatus:
  Idle
  Starting
  Ready { mouse_mode, serial_stats }
  Recovering { reason, attempt, next_retry }
  Failed { reason, attempts_exhausted }
```

顶层 view 选择不能再由 `control online` 决定：

- `ManualStopped` 或未选择任何设备：连接页。
- 已有 session intent 且 video/control 任一处于 Starting/Streaming/Stalled/Recovering/Failed：工作页。
- 控制 `Failed` 不切连接页。

## 6. RFB 需要同步调整

当前 RFB WS/TCP 服务在没有 `event_tx` 时拒绝连接，返回 `503`。彻底拆分后，视频可用但控制失败时仍应允许浏览器看视频。因此 RFB 需要区分“观看连接”和“控制输入通道”：

- RFB 连接可以在 video 可用时建立。
- control `Ready` 时，RFB 输入事件送入当前 `event_tx`。
- control `Recovering/Failed` 时，RFB 输入事件被忽略或报告为 input disabled；视频仍继续推帧。
- `RfbConnectionGate` 应只仲裁控制权，不应阻止只读观看连接。

这里是本方案最大的改动点之一。实现可以分两步：

1. 先允许 RFB 在 control unavailable 时建立只读连接，输入事件 no-op。
2. 再考虑控制恢复后现有 RFB 连接是否自动获得输入能力，或要求前端 RFB 重连。

倾向先做“控制恢复后通过状态通知触发 RFB 重连”，因为现有 RFB driver 持有固定 `event_tx`，动态切换 event sender 的侵入更大。但这个取舍需要在实施前进一步确认。

## 7. desktop 需要同步调整

`DesktopSessionController` 应从“会话所有者”变成 supervisor 的 UI adapter：

- `connect(request)` 改为 `supervisor.start(selection)`。
- `stop()` 改为 `supervisor.stop(Manual)`。
- `is_control_online()` 不再决定主页面。
- `subscribe_frames()` 订阅 `FrameHub`，不订阅具体 session frame source。
- `send_key/send_pointer/send_pointer_relative` 先检查 `ControlStatus::Ready`；非 Ready 时拒绝或忽略，并更新状态栏。
- iced `sync_status()` 不再自动 `controller.stop()`。
- `ConnectionStatus::ControlOffline` 需要由 supervisor snapshot 直接驱动，不能依赖当前几乎不可达的 `derive_status(online, input_offline_reason)`。

## 8. headless 需要同步调整

headless 应删除或瘦身以下私有状态：

- `web/recovery.rs`
- `ApiState.manual_stop`
- `ApiState.disconnect_deadline` 中与会话生命周期混在一起的逻辑
- `ApiState.frame_source` 私有 `SwitchableFrameSource`

`/api/status` 改为序列化 supervisor snapshot：

- `video.state`
- `control.state`
- `session.intent`
- `recovery.next_retry`
- `control.input_offline` 或更通用的失败原因

`/api/session` 的 `create/restart/stop` 调用 supervisor action，不直接操作 `SessionManager`。

浏览器前端继续保持原则：

- 手动停止或没有选择时显示连接页。
- 视频/控制恢复中或失败时显示视频页与状态提示。
- 控制失败时禁用输入控件，但不刷新到连接页。

## 9. 实施步骤建议

1. 在 `ipkvm-session` 增加共享状态类型、`FrameHub` 和纯单元测试。
2. 把 `ConsoleSession` 中与 `FrameSource` 强耦合的字段拆出，形成更清晰的 `ControlSession` 或输入泵管理器。
3. 新增 `VideoRuntime`，用 fake frame source 覆盖启动成功、从未出帧、曾出帧后停滞、源关闭、重试耗尽。
4. 新增 `ControlRuntime`，用 fake sink 覆盖创建失败、输入泵失败、恢复成功、恢复耗尽、人工停止不复活。
5. 新增 `SessionSupervisor`，覆盖视频失败不影响控制、控制失败不影响视频、重试耗尽保持工作页状态。
6. 改 headless：把 recovery/API/session 操作迁到 supervisor，保留 HTTP 契约并扩展状态字段。
7. 改 RFB：支持 video-only 连接或明确的输入禁用状态。
8. 改 desktop-core/iced：接入 supervisor snapshot，删除 `sync_status()` 自动销毁路径，主页面按 intent/video state 决定。
9. 合入 #53 或复用其帧状态清理测试，确保老的 `FrameClosed/subscribed` 问题不回归。
10. 更新长期文档和状态机说明。

## 10. 测试计划

### 10.1 共享层单元测试

- 视频 runtime 重试耗尽后状态为 `VideoStatus::Failed`，`FrameHub` 订阅不关闭。
- 控制 runtime 重试耗尽后状态为 `ControlStatus::Failed`，视频状态保持不变。
- `stop(Manual)` 后视频和控制都停止，恢复循环不自动复活。
- 清除 manual stop 后，显式 start 才恢复。
- 控制恢复后发布新的 event sender。
- 输入泵失败不会销毁 `FrameHub`。
- 视频源替换不会关闭上层 frame subscription。

### 10.2 headless 测试

- `/api/status` 能同时表达 `video.streaming + control.failed`。
- `control.failed` 时前端仍停留视频页。
- RFB 在控制失败时仍能建立只读画面，或按最终设计触发受控重连。
- `POST /api/session stop` 置为 manual stop，恢复循环不复活。
- `POST /api/session restart/create` 清除 manual stop 并启动 supervisor。

### 10.3 desktop 测试

- 模拟 sink 下一次输入失败后，iced 不回连接页，主画面仍显示或显示无信号。
- 状态栏显示控制恢复中/失败。
- 控制失败时键鼠输入不排队重放。
- 控制恢复后键鼠重新可用。
- 视频源关闭/替换后主订阅不永久冻结。

### 10.4 集成与人工验证

- 目标机重启进入 BIOS/Windows 期间，视频页不因 CH9329 短暂失败回连接页。
- 拔插或短断 CH9329 供电，观察控制状态恢复，视频页保持。
- 如果控制设备长时间不存在，重试耗尽后仍停在视频页并显示控制失败。
- 若只拔视频设备，控制状态不被误报为失败；UI 显示视频失败或无信号。

## 11. 风险与未决问题

### 11.1 RFB 动态控制恢复语义

最需要进一步确认的是：已有 RFB 连接在 control 从 `Failed/Recovering` 恢复到 `Ready` 后，是否应自动获得输入能力。

候选：

- 现有连接保持只读，前端看到状态恢复后主动重连 RFB。
- RFB driver 内部订阅 event sender watch，动态切换输入出口。

前者实现简单且风险小；后者体验更好但会明显侵入 RFB driver 和 gate 语义。

### 11.2 `RfbConnectionGate` 是否拆成 viewer 和 controller

当前 gate 表达“单活动控制者”，但 RFB 连接本身同时承担观看和输入。彻底拆分后，应允许多个只读观看者还是继续单观看者，需要和产品策略对齐。当前最小变更可以保持单 RFB 观看连接，但 gate 只在 control Ready 时授予输入控制。

### 11.3 输入离线期间的事件处理

控制非 Ready 时，desktop 和 web 输入事件不能无限缓存，否则恢复后会重放过时键鼠。建议策略：

- 控制离线时直接拒绝/忽略新输入。
- 状态栏/API 记录被拒绝输入计数。
- 恢复后从空输入状态继续。

### 11.4 视频失败时绝对鼠标坐标不可用

视频无有效 frame size 时，绝对鼠标无法映射。即使控制 Ready，也应暂停绝对指针输入；键盘是否允许继续需要明确。默认建议在视频非 Streaming/Stalled 且无 last frame 时暂停所有远程输入，避免误操作。

### 11.5 `FrameHub` 转发任务生命周期

hub 需要避免旧 frame source 任务泄漏。每次替换源时必须取消旧转发任务，并保证旧源释放后独占摄像头句柄可重新打开。

### 11.6 退避耗尽策略

用户已确认：重试耗尽后停在视频页，显示控制设备失败提示，不回连接页。

仍需决定具体参数：

- 最大重试次数或最大持续时间。
- 用户手动重试是否重置计数。
- 设备列表刷新是否自动重新尝试。

## 12. 验收标准

- desktop 和 headless 不再各自维护恢复循环；恢复策略位于共享层。
- 视频和控制状态可独立表达、独立恢复。
- 控制链路失败不会销毁视频链路，不会让 desktop 自动回连接页。
- 视频链路失败不会误报控制失败。
- 重试耗尽后保持视频页，明确显示失败状态。
- 手动停止后不会自动复活。
- 共享层、headless、desktop 均有覆盖失败与恢复路径的自动化测试。
- 长期文档更新，说明新状态机、UI 行为和剩余硬件验证边界。

## 13. 实现结果

### 13.1 共享层

`ipkvm-session::FrameHub` 是视频链路的稳定入口。上层始终订阅 hub；supervisor 重新打开视频源时先停止旧转发任务并等待释放，再把新源接入同一个 hub。`FrameHub::clear()` 只在曾经有源或已有帧时发布 `None`，避免空会话初始化阶段产生无意义的帧变更。

hub 对外发布的是稳定帧流 seq，不是底层源本地 seq。源替换后即使新源从 `seq=1` 开始，hub 对外 seq 仍保持单调递增，避免 RFB 把正常重建误判为帧序倒退。

`ipkvm-session::SessionSupervisor` 维护同一份 `SupervisorStatus`，包含 `SessionIntent`、`VideoRuntimeStatus` 与 `ControlRuntimeStatus`。控制失败、控制恢复中、视频恢复中和重试耗尽都不会自动变成连接页；只有人工停止或没有选择时，上层才应显示连接页。

控制链路仍复用既有 `SessionManager` 和输入泵，但 `SessionManager` 的帧源是稳定 `FrameHub`，不是一次会话私有的视频源。因此输入泵退出只会把控制状态推进到恢复或失败，不会释放视频订阅。

### 13.2 headless 与 RFB

headless 的 HTTP 服务不再维护私有恢复循环。`HeadlessWebService` 持有共享 `SessionSupervisor`、`FrameHub` 和当前选择；`/api/session create/restart/stop` 直接调用 supervisor action。后台恢复循环只负责按统一 `RecoveryPolicy` tick supervisor，不再自行重建整套 `(frame_source, sink)`。

`/api/status` 仍保留兼容字段，同时增加结构化运行态：

- `video.runtime.state/reason/attempt/attempts`
- `session.control.state/reason/attempt/attempts`
- `session.intent`

前端继续把人工停止或无选择视为连接页；控制恢复中或失败时留在视频页，并在视频栏显示输入链路状态。

RFB WS/TCP 已支持 video-only：没有当前控制 sender 时不拒绝连接，也不因 sender 关闭而断开观看路径。输入事件在控制不可用时忽略。RFB driver 仍是在连接建立时捕获当时的 sender，不在单条连接内动态切换；Web 前端在 `control` 从非 `ready` 恢复到 `ready` 后会断开旧 RFB，让状态轮询触发下一次自动连接，从而拿到新的输入 sender。非浏览器 RFB 客户端如果要在控制恢复后重新获得输入能力，也需要主动关闭并重连。

### 13.3 desktop

`DesktopSessionController` 改为 supervisor adapter。生产实现通过 `DesktopSessionFactory` 分别打开视频和控制设备：视频打开成功但控制打开失败时，`connect()` 仍进入工作页，视频保持可用，控制状态进入恢复。

iced 主界面使用 `controller.should_show_work_view()` 选页，不再用 `is_control_online()` 决定是否返回连接页。`UiTick` 推进共享 supervisor 恢复并刷新本地 event sender；控制恢复后 desktop 本地输入重新获得新的 sender。控制离线期间，UI 退出远程输入模式并拒绝继续发送键鼠事件，避免恢复后重放过时输入。

### 13.4 剩余边界

- RFB 已有连接不会在 driver 内动态切换输入 sender；当前产品路径通过 Web 前端受控重连恢复输入。若未来要让第三方 RFB 客户端无感恢复输入，需要把 driver 改成订阅 sender watch，并重新审视 gate 语义。
- `RecoveryPolicy::default()` 参数仍是固定策略，暂未提供 UI/API 配置。
- BIOS/Windows 重启、CH9329 短断电和视频设备拔插仍需要真实硬件验证；自动化测试已覆盖状态机和 adapter 行为，但不能替代硬件链路验证。
- 视频失败时绝对鼠标坐标的禁用策略仍需结合 BIOS 坐标域问题单独收敛，本 issue 不改变鼠标坐标映射。
