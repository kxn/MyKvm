# 输入状态机分叉治理调研

日期：2026-08-09

关联背景：`#57`、`#59`、`#61`、`#63`、`#65`、`#67`、`#73`

拆单结果：父单 `#77`；子单 `#78`、`#79`、`#80`、`#81`、`#82`。

## 背景

此前 `#67` 已经把“桌面输入 -> RFB 输入泵 -> mapper -> CH9329 sink”的理想协议契约写清，并修复了滚轮后绝对移动失效、相对按钮位丢失等局部问题。随后用户在 Windows 桌面客户端连接 macOS 目标机继续复测，仍观察到：

- 绝对模式拖拽能开始移动，但很快停止；本地继续移动时远端指针仍移动。
- 相对模式下曾出现移动可见但点击无效。
- 桌面 UI 中绝对/相对切换、profile 加载、快捷键切换、窗口失焦、Raw Input 捕获状态表现不一致。
- 键盘侧曾出现 Shift/Caps 大写、Alt、功能键等映射疑虑。

本次调研目标不是再补一个 bug，而是把实现中仍存在的行为分叉、重复状态、SSOT 缺口和跨层隐式契约整理到可以拆单开工的状态。实现代码本次不修改。

## 已有事实基础

`docs/superpowers/specs/2026-08-09-input-state-machine-audit-design.md` 已确认的长期契约继续有效：

- 绝对模式只发送标准 RFB `Pointer`，滚轮用 button 4/5 瞬时边沿表达。
- 相对模式只发送扩展 RFB `PointerRelative(mask, dx, dy, wheel)`。
- `SetMouseMode` 是 FIFO 屏障，模式切换后 pump 重置 pointer mapper。
- 相对 mapper 顺序是 `Button -> RelativeMove -> Wheel`，保证拖拽位移携带按钮位。
- 鼠标模式切换不应调用键盘 `release_all()`；控制者释放和输入源关闭才释放全量键鼠。

`docs/superpowers/specs/2026-08-09-input-diagnostics-logging-design.md` 已落地日志系统，用户提供的两份日志表明诊断链路已经能串起桌面入口、desktop-core 队列、session mapper、CH9329 report 和串口 frame/ack。

## 用户日志结论

日志文件：

- `/data/dl/my_ipkvm/.codex-remote/inbox/feishu-files/om_x100b684bb21320a4b3472f77edc2037/ipkvm-input-diag.log`
- `/data/dl/my_ipkvm/.codex-remote/inbox/feishu-files/om_x100b68b45e78fca0b1111e277ca67a5/ipkvm-input-diag.log`

关键观察：

- 第二份拖拽日志中，第一次长拖拽从 `mono_ms=3844` 到 `mono_ms=5064`，入口、RFB mapper、CH9329 report 均持续带 `mask/buttons=0x01`，并产生连续 `0x04` 绝对报告。
- 同一段日志中没有 `pointer_ignored`、`mouse_mode` 漂移、通道 pending 堵塞或串口 ACK 完全停滞的证据。
- 这说明“软件是否持续发送左键按住移动报告”在该场景下不是唯一根因；还需要验证 CH9329 绝对报告在 macOS 目标端的拖拽语义、目标控件拖拽阈值、坐标映射边界和真实 HID 描述符兼容性。
- 第一份日志存在大量 `absolute_map_failed reason=outside_or_unready`，说明 UI 坐标映射、视频区域边界和进入远程输入状态仍应作为调试维度保留。

结论：拖拽问题不能只归因于限流，也不能只靠源码看按钮位。后续必须把“日志回放 + CH9329 最终帧 + macOS 实机验证”列为验收。

## 分层分叉清单

### 桌面 iced 输入入口

已确认风险：

- `crates/ipkvm-desktop-iced/src/app.rs:1084-1098`、`1101-1121`、`1809-1824` 分别实现状态栏/profile、模态、快捷键模式切换。三条路径的错误处理、`active_profile`、相对源停止、采样器和本地 cursor 同步不一致。
- `crates/ipkvm-desktop-iced/src/app.rs:1144-1149` 加载 profile 时直接替换 `self.connection`，在线状态下没有调用 `controller.set_mouse_mode()`，也没有按模式切换事务同步 cursor/relative source。
- `crates/ipkvm-desktop-iced/src/app.rs:1012-1017` 恢复默认值只替换连接配置，在线状态下同样不保证 sink 实际模式同步。
- `crates/ipkvm-desktop-iced/src/app.rs:1314-1346` 在判断点击是否位于视频区前更新 `pointer_mask`。菜单、弹窗、状态栏等 iced 全局鼠标事件可能污染远端按钮状态。
- `crates/ipkvm-desktop-iced/src/app.rs:1426-1442` 退出远程输入会清 pointer/sampler/source 并 release，但 `disconnect()` 的清理路径 `1834-1847` 少清 `pointer_mask`、`last_pointer_sent`、`last_pointer_sent_at`、`relative_sampler`。
- `crates/ipkvm-desktop-iced/src/app.rs:1268-1273` 维护 `last_modifiers`，但 `exit_remote_input()` 未复位该字段。退出后 release 事件被 `remote_input=false` 过滤，重新进入可能产生错误修饰键 diff。
- `crates/ipkvm-desktop-iced/src/platform/windows.rs:53-79` 在 Raw Input 初始化完成前先设置全局 `TX`，初始化超时或线程失败时可能留下全局发送端或脱管线程。
- `crates/ipkvm-desktop-iced/src/platform/cursor.rs:115-140` 使用 `GetForegroundWindow()` 作为 ClipCursor 窗口归属，未绑定 iced 的实际窗口 id；多窗口、焦点切换和 DPI 场景下归属不够明确。
- `crates/ipkvm-desktop-core/src/config.rs:31-40` 同时持久化 `MouseProfile` 与 `MouseMode`，反序列化又以 `profile.resolve_mode()` 为准，形成双字段 SSOT 风险。

拆单判断：这些风险共享同一个根因，即桌面输入状态没有统一事务入口。应先收敛桌面状态转换 SSOT，再处理更细的按钮/滚轮调度。

### 前端 pointer 调度与拖拽

已确认风险：

- 桌面绝对移动仍由 `POINTER_MIN_INTERVAL`、`last_pointer_sent`、`last_pointer_sent_at` 控制；日志证明当前拖拽中仍有连续发送，但边界处仍可能有 `absolute_map_failed` 或去重抑制影响首/尾事件。
- `crates/ipkvm-desktop-iced/src/input.rs:120-127` 对每个 Pixel wheel 事件独立除以 50 后四舍五入，没有跨事件累计；高精度触摸板或小步滚轮可能全部变成 0。
- `crates/ipkvm-desktop-iced/src/app.rs:1375-1392` 相对和绝对滚轮路径分叉，绝对路径已按标准 RFB 发送，相对路径仍直接忽略 `send_pointer_relative` 错误。
- `third_party/novnc/1.7.0/core/rfb.js:1241-1244` headless 相对移动先对每个浏览器事件乘灵敏度并截断，再累计；低灵敏度下小位移会永久丢失。桌面端是浮点累计后取整。
- `third_party/novnc/1.7.0/core/rfb.js:1359-1389` headless 相对滚轮用浏览器事件 `bmask` 发包，但不更新 `_mouseButtonMask`；下一次相对移动仍用旧持久 mask，可能造成按钮状态回退/重按。
- `crates/ipkvm-headless/web/modules/pointer.js:110-119`、`147-149` pointer lock 退出和窗口 blur 只清本地锁定，不保证远端按钮释放；若浏览器丢失 mouseup，远端可能卡住按钮。

拆单判断：桌面与 headless 都需要一个“前端 pointer event builder”语义，但实现可以分别落地。macOS 拖拽的硬件/profile 验证应归入同一个子单，避免继续只改软件路径。

### Session/core 状态所有权

已确认风险：

- `crates/ipkvm-session/src/rfb_input/pump.rs:210-243` 创建 `TextInputService::new(sink.clone(), ...)`。`Ch9329InputSink` 的 clone 会复制 `KeyboardState` 与 `MouseState`，但共享同一串口队列。
- `crates/ipkvm-session/src/rfb_input/text.rs:209-226` 文本任务直接调用 `handle_key_batch`，`256-267` 取消时调用独立 sink 的 `release_all()`。这会绕过主 `RfbKeyboardMapper` 与主 sink 状态。
- `crates/ipkvm-session/src/rfb_connection/driver.rs:416-424` `send_event()` 忽略 `sender.send(event)` 错误，输入泵退出后 RFB 连接仍可能表现在线但输入静默丢失。
- `crates/ipkvm-session/src/rfb_input/pump.rs:558-576` 相对模式下标准绝对 Pointer 被忽略但仍返回 `RfbInputNotice::Pointer`，诊断统计可能显示“有输入”却没有 CH9329 输出。
- `crates/ipkvm-core/src/ch9329/input.rs:301-323` sink 在命令成功入队后提交内部状态；如果串口 worker 后续恢复时清掉待发送 batch 并发零报告，软件状态与设备状态会分叉。
- `crates/ipkvm-core/src/input.rs` 的 `InputSink::set_mouse_mode()` 契约只声明切换模式，pump 额外假设成功后旧模式按钮已释放并重置 pointer mapper。该假设目前只由 CH9329 sink 隐式满足。
- 队列 full、无效 pointer 坐标和真实 sink 错误目前都可能被提升为输入泵致命错误，缺少“瞬时背压/坏事件/设备故障”的错误分类。

拆单判断：这是核心层最大的 SSOT 缺口，必须独立成子单；它会影响鼠标、键盘、文本和恢复状态，不应夹在 UI 修复里完成。

### 键盘、锁定键与特殊键

已确认风险：

- `crates/ipkvm-desktop-iced/src/keymap.rs:101-108` 使用 `modified_key` 得到 Shift/Caps 后字符，session 的 `crates/ipkvm-session/src/rfb_input/keyboard.rs:56-91` 又按 keysym 合成或抑制 Shift。该路径解决了部分大写问题，但锁定键状态仍未建模。
- `crates/ipkvm-session/src/rfb_input/keymap.rs` 中 CapsLock/NumLock/ScrollLock 被映射为 `IgnoredLock`，桌面和 noVNC 仍会发送这些 keysym，远端锁定状态无法通过输入链路控制。
- `crates/ipkvm-desktop-iced/src/app.rs:1220-1273` 中 `KeyPressed` 可按物理键发送右侧修饰键，而 `ModifiersChanged` 通过 `crates/ipkvm-desktop-iced/src/input.rs:96-109` 统一发左侧修饰键，右 Alt/AltGr/Option 和右 Shift 可能形成重复或不一致事件。
- `crates/ipkvm-desktop-iced/src/app.rs:1215-1218` 全局键盘监听只检查 `remote_input`，没有检查模态和 UI TextInput 焦点；远程输入模式下打开设置或保存 profile 时，UI 输入可能同时转发到远端。
- `crates/ipkvm-desktop-iced/src/input.rs:63-83` 和 `crates/ipkvm-headless/web/modules/special-keys.js:320-331` 特殊键直接发送完整 Down/Up 序列，没有和用户当前按住的修饰键做所有权/引用计数协调。
- 桌面、session、headless/noVNC 分别维护 keysym 常量和功能键覆盖。桌面和 session 当前覆盖 F1-F20，noVNC keysym 表可产生更高 F 键，超出 session 映射后会被拒绝。

拆单判断：键盘问题横跨桌面、headless、session 和文本服务，应独立成“键盘输入状态 SSOT 与覆盖矩阵”子单。锁定键和右侧修饰键是优先验收。

### Headless/noVNC 与外部 RFB

已确认风险：

- `crates/ipkvm-headless/web/modules/keyboard.js:41-63` 拦截器只在 canvas 聚焦时工作，只做 `preventDefault`，不负责转发和释放。
- noVNC 键盘状态在 canvas blur、工具栏操作、特殊键菜单、断开和 pointer lock 之间没有和后端 release 形成统一契约。
- `crates/ipkvm-headless/src/web/service.rs:756-764` 鼠标 profile API 用 selection/settings 判断是否需要发送 `SetMouseMode`，但 input pump 会因实际收到的 Pointer/PointerRelative 自动切换模式，二者可能漂移。
- 外部 RFB 客户端可以发送相对扩展消息使 pump 进入相对模式，但普通标准 Pointer 无法显式切回绝对；网络 RFB 路径没有模式协商事件。

拆单判断：headless 既有浏览器硬限制，也有 noVNC patch 自身状态问题。它应作为独立子单对齐前端捕获、释放、profile API 和外部 RFB 模式协商。

## 拆单方案

采用一个父单加五个子单。父单负责治理目标、依赖顺序和总体验收；子单负责可独立实现和测试的边界。

### 父单：输入状态机分叉治理与 SSOT 收敛

目标：

- 把所有输入入口、模式切换、键鼠释放、文本输入和恢复状态统一到明确状态所有者。
- 消除“同一行为多入口多实现”的路径。
- 建立拖拽、相对点击、滚轮、Shift/Caps/Alt/F 键的自动化和人工验收矩阵。

验收：

- 五个子单全部关闭。
- 文档更新后能回答“哪个层拥有哪个状态、谁能修改、失败如何回滚”。
- macOS 目标拖拽有明确结论：软件报告正确但目标不接受、或软件/协议仍有问题，并给出对应 profile/协议修复计划。

### 子单 1：桌面输入模式转换与捕获状态 SSOT

GitHub issue：`#79`

范围：

- `app.rs` 中 profile 切换、模态切换、快捷键切换、加载 profile、恢复默认值、退出远程输入、断开连接统一走一个事务入口。
- 统一清理 `relative_source`、`relative_rx`、`relative_sampler`、`relative_wheel`、`pointer_mask`、`last_relative_mask`、`last_pointer_sent`、`last_pointer_sent_at`、`last_modifiers`、`active_keysyms` 和 cursor clip。
- Windows Raw Input lifecycle 与 ClipCursor 窗口归属纳入同一工作。
- `MouseProfile` / `MouseMode` 双字段契约收敛：明确持久化兼容字段和运行时事实源。

验收：

- 在线加载 profile、恢复默认值、状态栏选择、连接设置、快捷键切换都触发同一模式切换路径。
- 切换失败不提交 UI 假状态。
- 从相对切绝对必停 Raw Input、释放 cursor clip、清 sampler。
- 从绝对切相对进入“待捕获”状态，不携带旧按钮和旧 delta。
- Windows Raw Input 初始化失败后可再次启动，不残留全局 `TX` 或脱管线程。

### 子单 2：前端 pointer 调度、滚轮与 macOS 拖拽验证

GitHub issue：`#82`

范围：

- 桌面绝对/相对的 move/button/wheel builder 语义统一：按钮和滚轮是 barrier，移动限频不吞边沿。
- 滚轮像素增量跨事件累计，绝对和相对路径都保留发送错误。
- headless 相对移动改为浮点累计后取整，滚轮保持持久按钮 mask 一致。
- pointer lock/blur/退出相对模式要释放或归零远端按钮状态。
- macOS 拖拽建立日志回放和实机验证步骤。

验收：

- 桌面和 headless 在相同抽象序列下产生一致的 RFB 指针事件。
- `drag` 序列最终 CH9329 绝对/相对报告在按住期间持续携带按钮位。
- 高精度滚轮小步累计能产生预期 wheel step。
- macOS 目标至少验证：Finder 窗口标题栏拖拽、窗口内列表项拖拽、Dock/菜单栏非目标区域、按住滚轮移动、滚轮后继续拖拽。

### 子单 3：session/core 输入状态所有权与错误传播

GitHub issue：`#78`

范围：

- TextInput 不再持有独立 CH9329 状态机；文本输入与物理/RFB 键盘输入串行化到同一个状态所有者。
- RFB driver 对事件 channel closed 返回现有 `EventChannelClosed` 或等价错误，不静默吞事件。
- 串口恢复后输入状态与设备零报告重新同步。
- `InputSink::set_mouse_mode` 契约显式声明按钮释放、状态提交和失败回滚。
- 区分 queue full、坏 pointer、设备错误；避免瞬时背压直接杀掉输入泵。
- 诊断 notice 区分 applied/ignored/rejected。

验收：

- “按住键/鼠标时粘贴文本、取消文本、串口恢复、继续输入”的序列不会产生旧状态复活。
- 输入泵退出后 RFB 客户端得到明确断开或错误。
- 串口恢复后软件状态与设备状态一致，后续 key-up/button-up 不依赖旧状态猜测。

### 子单 4：键盘输入状态、锁定键和特殊键 SSOT

GitHub issue：`#81`

范围：

- 桌面键盘入口在 `KeyPressed` 与 `ModifiersChanged` 之间选定唯一修饰键事实源，避免右侧修饰键重复。
- `last_modifiers`、`active_keysyms`、UI 焦点、模态和远程输入状态统一生命周期。
- CapsLock/NumLock/ScrollLock 制定明确策略：真实发送 HID lock、基于目标 LED/profile 同步，或在 UI 中明确作为特殊状态管理；不能继续静默吞掉。
- 特殊键序列需要与当前按住键做所有权协调。
- 功能键、Fn/FnLock、媒体键、keypad、国际布局建立覆盖矩阵，至少明确 unsupported 的可见反馈。
- keysym 常量和映射表收敛到单一事实来源或生成式测试。

验收：

- Windows 桌面宿主上验证 Shift、CapsLock、左右 Shift、左右 Alt/AltGr、Ctrl、Logo、F1-F20、方向/导航键。
- Headless 对无法捕获的键给出明确 UI 反馈或特殊键路径。
- UI TextInput 获焦时不再把文本转发到远端。
- 特殊键不会提前释放用户仍按住的修饰键。

### 子单 5：headless/noVNC 捕获、释放与外部 RFB 模式协商

GitHub issue：`#80`

范围：

- Pointer Lock 状态、selected profile、actual pump mode、RFB connection state 建立统一前端状态机。
- 画布失焦、工具栏操作、特殊键菜单、窗口 blur、pointer lock 退出时释放键鼠状态。
- `/api/input/mouse-profile` 以实际 pump mode 为准判断是否需要 `SetMouseMode`，不能只看 selection/settings。
- 外部 RFB 客户端的绝对/相对模式切换契约明确：标准 Pointer 是否允许切回绝对，或必须有扩展/控制 API。
- noVNC 键盘 reject/unsupported notice 可见化。

验收：

- Pointer lock 退出后不会卡远端按钮。
- headless 相对模式滚轮不会破坏持久按钮状态。
- 外部客户端从相对回绝对有明确路径，或诊断明确提示不支持并不会静默忽略。
- 浏览器无法捕获的键路径有可见说明和可操作替代。

## 依赖顺序

推荐顺序：

1. 子单 3：核心状态所有权，先避免 TextInput、串口恢复和输入 channel 静默错误继续污染现象。
2. 子单 1：桌面模式切换和捕获 SSOT，消除 Windows 桌面客户端当前最明显的入口分叉。
3. 子单 2：pointer 调度和 macOS 拖拽验证，在核心和桌面生命周期稳定后复测拖拽。
4. 子单 4：键盘状态 SSOT，解决 Shift/Caps/Alt/特殊键完整矩阵。
5. 子单 5：headless/noVNC 对齐，可与子单 4 的键盘部分并行，但 pointer 语义应复用子单 2 的结论。

如果资源允许，子单 3 与子单 1 可以并行；子单 2 不应在二者完成前下结论，否则拖拽日志会继续混入状态分叉噪声。

## 测试与验证策略

自动化优先：

- Rust 单元测试：状态转换表、mapper/sink 状态提交和失败回滚、TextInput 交错、driver channel closed。
- 桌面 app 测试：profile/快捷键/模态/断开/退出远程输入路径的状态清理。
- noVNC/browser 测试：relative move/wheel/button/pointer lock lifecycle。
- 日志回放测试：从 logfmt 提取拖拽序列，断言入口 mask、mapper incoming/committed、CH9329 report buttons 连续一致。

人工验证：

- macOS 目标拖拽必须实机验证，自动化不能证明目标 OS 接受 HID 语义。
- Windows 桌面宿主 Raw Input/ClipCursor 需要本机手工验证，因为 CI 不稳定具备真实窗口焦点和鼠标设备。
- 浏览器键盘捕获限制需要按 Chrome/Edge/Firefox 与全屏/非全屏分别记录。

## 自审

协议角度：

- 新拆单不再允许绝对/相对混用作为默认 hack；任何目标 OS 差异必须进入 profile 或明确协议扩展。
- 仍不确定的是 macOS 是否接受 CH9329 绝对 report 中“移动帧携带按钮位”的拖拽语义，需要子单 2 实机确认。

状态机角度：

- 最大已确认 SSOT 缺口是 TextInput 独立 sink、桌面多模式切换入口、键盘修饰键双入口。
- 子单 3 和子单 1 是前置，不应被局部 bug fix 绕过。

顺序与缓冲角度：

- `SetMouseMode` FIFO 契约已存在，但 UI 入口并不总是发送它。
- pointer/button/wheel 的 barrier 语义在 headless 相对滚轮和 pointer lock 退出仍不完整。

目标 OS/硬件角度：

- 用户日志已经证明部分拖拽序列软件链路持续输出左键按住移动报告；目标 macOS 行为仍需用实机矩阵定性。
- 串口恢复和 CH9329 实际 pending 丢弃会影响所有目标 OS，不能只在 macOS profile 中处理。

维护者角度：

- 父单负责防止再次“补丁摞补丁”；子单按状态所有权和入口边界拆分，便于每个 PR 用测试证明一类分叉已经消失。
- 后续计划必须引用本调研文档和对应子单，不应直接从单个用户现象跳到实现改动。
