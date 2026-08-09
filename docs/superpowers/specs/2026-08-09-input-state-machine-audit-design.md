# 端到端键鼠状态机审计与契约收敛

关联 issue：`#67`

日期：2026-08-09

## 背景

桌面端在 macOS 目标机上暴露出多类输入异常：大写输入、相对模式点击、绝对模式滚轮后移动失效、拖拽只能移动很短距离。这些现象分布在桌面事件适配、RFB 事件队列、输入泵、RFB 指针/键盘 mapper 和 CH9329 sink 多层之间，不能用单点修补解释。

本审计把输入链路当成一组串联状态机处理：先确认每层只维护自己的状态，再确认跨层事件类型、顺序和状态复位契约一致。目标不是为某个操作系统加 profile hack，而是把绝对/相对模式、按钮、滚轮、键盘修饰键和缓冲机制的基础协议路径收敛清楚。

## 目标与范围

目标：

- 记录桌面本地输入、RFB 输入、输入泵、mapper、CH9329 sink 的状态事实。
- 明确绝对模式和相对模式的跨层事件契约。
- 明确键盘 Shift、Caps Lock、Alt、Logo、F 键等映射当前依赖的输入信息。
- 补充端到端回归测试，覆盖滚轮、拖拽、相对按钮、模式切换和 pending 队列边界。
- 修正审计中确认的根因缺陷。
- 给后续 OS 兼容性 profile 和原生捕获工作留下可执行边界。

不在本次范围内：

- 不引入新的鼠标 OS profile。
- 不改 CH9329 串口命令格式。
- 不重写桌面 UI 框架。
- 不声明所有宿主 OS 全局快捷键都可以被桌面端拦截。
- 不把手工目标机兼容性矩阵一次性补齐；本次先把自动化可覆盖的协议状态机钉住。

## 分层事实

### 桌面 iced 入口

- 键盘事件来自 `iced::keyboard::Event`。
- 字符键优先使用 `modified_key`，因此 Shift 和 Caps Lock 影响后的单字符 ASCII 会作为 keysym 进入 session；非 ASCII 和非字符键回退到物理 `Code`。
- 修饰键边沿来自 `ModifiersChanged`，按 `Shift -> Ctrl -> Alt -> Logo` 顺序生成 keysym down/up。
- `Ctrl+Alt+K` 和 `Ctrl+Alt+M` 是本地控制组合，不转发触发键本身；修饰键边沿是否已转发取决于宿主窗口系统是否先送出 `ModifiersChanged`。
- F 键在桌面入口按 `Code::F1..=Code::F20` 映射到 X11 `XK_F1..=XK_F20`。
- 绝对鼠标模式发送标准 RFB `Pointer`：`button_mask + x/y + framebuffer_size`。
- 相对鼠标模式发送扩展 RFB `PointerRelative`：`button_mask + dx/dy + wheel`。
- 绝对移动保留 `POINTER_MIN_INTERVAL = 8 ms` 的移动限频；按钮 mask 变化和滚轮边沿不应被降级为相对事件。
- 绝对滚轮必须用 RFB button 4/5 的瞬时边沿表达：`mask|0x08 -> mask` 或 `mask|0x10 -> mask`，不能发送 `PointerRelative`。

### 桌面会话控制器

- `DesktopSessionController` 将桌面本地输入包装为 `RfbServerEvent`，统一送入 session supervisor 的输入泵。
- 通道满时事件进入 `pending_events`，随后由 `flush_pending()` 按 FIFO 补送。
- `SetMouseMode` 是 FIFO 屏障：它必须位于旧模式事件之后、新模式事件之前，不能被 pending 补送重排。
- `release_all()` 在桌面控制器层用 `Disconnected -> Connected` 重新建立本地控制者，由输入泵执行 release。

### RFB 输入泵

- `RfbInputPump` 串行处理单活动控制者事件。
- `mouse_mode` 是已确认/已使用的指针协议模式，首个指针事件也可以触发模式收敛。
- 标准 `Pointer` 在 pump 当前为相对模式时必须被忽略，避免绝对事件污染相对捕获。
- `PointerRelative` 进入前必须确保 sink 已切到相对模式。
- 鼠标模式切换只应调用 `InputSink::set_mouse_mode(mode)`，由 sink 在硬件层释放旧模式鼠标按钮；pump 不应借 `release_all()` 释放键盘。
- 鼠标模式切换成功后必须重置 `RfbPointerMapper`，因为 sink 的鼠标按钮状态已被切模式边界清零。
- 控制者断开、事件源关闭或显式释放才调用 `release_all()`，并在释放成功后重置键盘和指针 mapper。

### RFB 指针 mapper

- 绝对 `Pointer` 的 `button_mask` 语义是当前位置的持久按钮位，加上 RFB wheel up/down 瞬时按钮位。
- 绝对路径顺序为 `AbsoluteMove -> 按钮边沿 -> Wheel`：先移动到目标坐标，再按下/释放按钮，滚轮使用当前位置。
- 相对 `PointerRelative` 的 `button_mask` 语义是本次相对位移发生时的按钮状态。
- 相对路径顺序为 `按钮边沿 -> RelativeMove -> Wheel`：先应用新按钮状态，再发相对位移，保证拖拽位移携带按钮位。
- mapper 只有在 sink 接受 batch 后才提交 `committed_button_mask`，失败可重试。
- 不支持的高位按钮只报告忽略，不改变低三位按钮和滚轮语义。

### RFB 键盘 mapper

- 输入是 X11 keysym，输出是 USB HID usage。
- ASCII 大写字符通过 `ShiftRequirement::Required` 合成左 Shift；小写字符通过 `ShiftRequirement::NotRequired` 抑制远端 Shift。
- 直接修饰键 keysym 映射到 HID modifier usage：Ctrl、Shift、Alt、Logo 都进入 modifier byte。
- Caps Lock 和 Num Lock 当前映射为 `IgnoredLock`，因为锁定键状态应由宿主 `modified_key` 或目标机 LED/profile 后续处理，不能在 mapper 内盲目切换目标锁定状态。
- F1..F12 映射 HID `0x3a..0x45`，F13..F20 映射 HID `0x68..0x6f`。
- 同一 usage 的别名会合并，最后一个 keysym release 才释放 usage。

### CH9329 sink

- `KeyboardState` 维护 modifier byte 和最多 6 个普通键；失败不提交内部状态。
- `MouseState` 维护硬件模式、按钮位和最后绝对坐标。
- 绝对移动输出 CH9329 `0x04`，并携带当前按钮位。
- 相对移动输出 CH9329 `0x05`，并携带当前按钮位；大位移和大滚轮拆成多个报告。
- 绝对按钮和绝对滚轮需要已知最后绝对坐标。
- `set_mouse_mode()` 若模式变化，会用旧模式释放鼠标按钮，并把 mouse buttons 清零；它不释放键盘。
- `release_all()` 同时发送键盘零报告和当前模式下的鼠标释放报告；失败不提交内部状态。

## 状态表

### 桌面指针状态

| 状态字段 | 所属层 | 更新事件 | 审计结论 |
| --- | --- | --- | --- |
| `remote_input` | iced | 进入/退出视频输入 | 退出必须释放远端并清空本地待发状态。 |
| `connection.mouse_mode` | iced | 用户切换/profile | 决定发 `Pointer` 还是 `PointerRelative`。 |
| `pointer_mask` | iced | 鼠标按钮 down/up | 持久按钮位，绝对和相对共享。 |
| `relative_wheel` | iced | 相对滚轮 | 只允许在相对模式累计；绝对滚轮不得写入该状态。 |
| `last_pointer_sent` | iced | 绝对移动/绝对滚轮 | 用于绝对移动去重和限频，滚轮后应回到持久 mask。 |
| `relative_sampler` | iced | 相对移动采样 | 只处理相对位移，控制边沿前必须 flush。 |

### RFB pump 指针状态

| 状态字段 | 更新事件 | 必须满足的契约 |
| --- | --- | --- |
| `mouse_mode` | 首个指针事件、显式 `SetMouseMode`、释放 | 只记录 sink 已确认的模式；`None` 表示等待收敛。 |
| `pointer` mapper | 指针事件、模式切换、控制者释放 | 模式切换成功后重置；释放成功后重置。 |
| `keyboard` mapper | 键盘事件、控制者释放 | 鼠标模式切换不得重置键盘。 |
| `active` | connected/disconnected | 只有活动控制者可改变输入状态。 |

### RFB 指针 mapper 状态

| 模式 | 输入 | 输出顺序 | `committed_button_mask` 提交时机 |
| --- | --- | --- | --- |
| 绝对 | `Pointer(mask,x,y,size)` | move，button diff，wheel edge | sink batch 成功后 |
| 相对 | `PointerRelative(mask,dx,dy,wheel)` | button diff，move，wheel | sink batch 成功后 |

### CH9329 鼠标状态

| 状态字段 | 更新事件 | 风险 |
| --- | --- | --- |
| `mode` | `set_mouse_mode` | 切换失败必须保留旧模式和旧按钮。 |
| `buttons` | button event、set mode、release all | mapper 与 sink 任一方清零都必须同步另一方。 |
| `last_absolute` | absolute move | 绝对按钮/滚轮没有坐标时应拒绝，而不是猜坐标。 |

### 键盘状态

| 层 | 状态 | 审计结论 |
| --- | --- | --- |
| iced | `active_keysyms`、`last_modifiers` | release 使用按下时登记的 keysym，避免当前 modifiers 改变导致错放。 |
| RFB mapper | active keysyms、committed usages | Shift 需求由所有活动字符共同决定，冲突时拒绝且不提交。 |
| CH9329 | modifier byte、6KRO keys | 报告集合语义，不要求普通键槽位顺序稳定。 |

### 事件缓冲状态

| 层 | 缓冲 | 审计结论 |
| --- | --- | --- |
| iced relative sampler | 浮点累计位移 | 控制边沿前强制取出整数位移。 |
| desktop-core pending | `VecDeque<RfbServerEvent>` | FIFO，不得跨 `SetMouseMode` 重排。 |
| tokio channel | 有界 mpsc | 满时等待或 pending，不丢 key-up/button-up。 |
| serial queue | `CommandBatch` | 一个 pointer batch 内的 CH9329 帧不能和别的输入交错。 |

## 已确认问题与修复

### 1. 绝对滚轮误发相对事件

路径：

```text
iced WheelScrolled(Absolute)
  -> relative_wheel += step
  -> send_absolute()
  -> send_pointer_relative(mask,0,0,wheel)
  -> RfbInputPump::handle_pointer_relative()
  -> ensure_mouse_mode(Relative)
  -> 后续 Pointer 被相对模式过滤
```

修复：

- 绝对滚轮改为标准 RFB `Pointer` button 4/5 瞬时边沿。
- `send_absolute()` 不再读取或发送 `relative_wheel`。
- 绝对滚轮发送后把 `last_pointer_sent` 记录为持久 mask 坐标，保证后续绝对移动继续工作。

### 2. 相对模式切换后按钮位丢失

路径：

```text
绝对模式 button_mask=1
  -> pump pointer mapper committed_button_mask=1
SetMouseMode(Relative)
  -> sink release_all/set_mouse_mode 后 mouse buttons=0
  -> pump pointer mapper 仍认为 committed_button_mask=1
PointerRelative(button_mask=1, dx, dy)
  -> mapper 不产生 ButtonDown
  -> CH9329 相对移动 frame buttons=0
```

修复：

- pump 切鼠标模式后重置 `RfbPointerMapper`。
- pump 切鼠标模式不再调用 `release_all()`，避免提前释放键盘。
- 相对 mapper 先应用按钮边沿，再发送相对位移；拖拽位移携带新 mask。

### 3. 文档与实现契约不一致

旧调度文档仍写着相对 mapper 顺序为 `RelativeMove -> Button -> Wheel`，且 mode switch 执行 `release_all -> set_mouse_mode`。本次将长期文档改为当前契约：相对是 `Button -> RelativeMove -> Wheel`，模式切换只切鼠标并重置 pointer mapper。

## 端到端测试矩阵

| 场景 | 自动化覆盖 | 断言 |
| --- | --- | --- |
| 桌面绝对滚轮后继续移动 | `absolute_wheel_keeps_absolute_pointer_mode_and_allows_later_moves` | 不产生 `PointerRelative`，后续绝对 move 到达。 |
| 桌面绝对拖拽 | `absolute_drag_preserves_button_mask_across_desktop_to_sink_path` | 按住移动的绝对事件保留按钮 mask。 |
| desktop-core pending 屏障 | `flush_pending_events_keeps_mode_switch_as_fifo_barrier` | `SetMouseMode` 不跨旧/新模式事件重排。 |
| RFB 绝对拖拽到 CH9329 | `real_ch9329_absolute_drag_carries_button_on_move_frames` | 全部为 `0x04`，按住移动携带按钮位。 |
| RFB 绝对滚轮到 CH9329 | `real_ch9329_absolute_wheel_stays_absolute_and_allows_later_moves` | wheel up 进入绝对帧 wheel 字段，后续 move 仍为绝对。 |
| 显式绝对->相对切换 | `explicit_mode_switch_releases_old_buttons_before_relative_motion` | 旧模式释放按钮；新相对按钮 down 后位移携带按钮。 |
| 鼠标模式切换不释放键盘 | `mouse_mode_switch_preserves_keyboard_until_key_release` | 键盘 release 在真实 key-up 时发生，不被 set mode 提前触发。 |
| RFB 相对 mapper 顺序 | `relative_move_emits_delta_and_preserves_buttons` | 相对 batch 顺序为 ButtonDown 后 RelativeMove。 |
| CH9329 输入核心属性 | `ch9329::input` 测试组 | 键盘/鼠标状态失败回滚、拆包和报告模型匹配参考模型。 |

## 后续兼容性验证建议

不要再把某个 BIOS 或 OS 的临时现象直接固化为默认 hack。后续如果目标系统确实需要差异，应按 profile 进入：

- Windows、Linux、macOS、BIOS/UEFI 各自记录绝对和相对模式下：单击、双击、拖拽、滚轮、按住滚轮移动、边缘坐标、分辨率切换。
- 桌面宿主 Windows/macOS/Linux 各自记录 iced 可收到的键盘事件：Shift、Caps Lock、Alt、AltGr/Option、Logo、F1..F20、方向键、PrintScreen、Pause、ContextMenu。
- Headless/noVNC 单独记录浏览器拦截限制；浏览器无法捕获的组合键必须走特殊键菜单或后续原生捕获器。
- 高轮询率鼠标只影响宿主事件频率；协议层用相对累计和绝对限频承载，不应靠丢事件维持稳定。

## 五个角度自审

协议角度：

- 绝对滚轮回到 RFB 标准 button 4/5 边沿，没有再混用 `PointerRelative`。
- 相对扩展消息的按钮 mask 被视为本次位移状态，而不是位移后的补充边沿。

状态机角度：

- 每个状态清零都有对应层：sink 切模式清鼠标，pump 重置 pointer mapper；控制者释放才清键盘 mapper。
- `release_all()` 不再被鼠标模式切换滥用，避免键盘状态暗中漂移。

顺序和缓冲角度：

- pending FIFO 明确覆盖 `Pointer -> SetMouseMode -> PointerRelative`。
- CH9329 batch 保持同一逻辑事件内帧不交错；相对按钮 down 和位移允许拆为两帧，但顺序固定。

目标 OS/硬件角度：

- macOS 拖拽依赖按住期间每个移动报告携带按钮位，本次覆盖绝对和相对两条路径。
- CH9329 绝对按钮/滚轮依赖最后坐标，本次没有引入“未知坐标猜测”。

维护者角度：

- 文档把原先隐含在多层代码里的模式切换和按钮顺序写成表，后续改 UI 或 Tauri 前端时可按同一契约接入。
- 新测试集中覆盖最容易回归的跨层状态边界，而不是只覆盖单个 bug 的表象。

## 后续拆分建议

本次 #67 已收敛基础协议状态机。后续如继续推进，应按以下顺序拆，不要并行预开互相覆盖的单子：

1. 宿主键盘捕获能力调研：分别评估 iced/winit、Tauri 原生插件、平台全局低层 hook 在全屏/锁定模式下的可捕获键集合。
2. 目标 OS 鼠标 profile 验证：在当前契约上做 Windows/Linux/macOS/BIOS 兼容矩阵，只有出现稳定差异时才新增 profile 行为。
3. 特殊键发送面板扩展：把宿主 OS 必然拦截的组合键统一通过菜单或屏幕键盘发送，避免伪装成普通键盘事件。
4. 手工硬件验证台账：真实 CH9329、真实 macOS 目标机和高轮询鼠标的人工步骤应形成独立记录，再决定是否继续自动化。
