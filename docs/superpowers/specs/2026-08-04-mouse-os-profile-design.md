# 键鼠 OS profile 与原始模式选择设计

日期：2026-08-04

状态：已确认产品方向，待实施计划。

范围：Iced 桌面端和 headless Web。egui 桌面端不纳入本功能，保留现状，后续清理或对齐另行处理。

## 1. 背景

当前输入核心只有 `MouseMode::Absolute` 和 `MouseMode::Relative` 两种底层模式。Iced 桌面端状态栏只显示鼠标状态，Web 状态栏已有相对指针锁定按钮；连接设置和 Web 运行时设置也只保存绝对/相对值。

真实测试表明，不同目标环境需要不同的 CH9329 鼠标发送模式：Windows 10 和 BIOS 当前使用绝对模式效果良好，Ubuntu 当前优先使用相对模式。Android 和 macOS 尚未完成实测，第一版先按绝对模式提供预设，后续可以独立调整映射。

本功能在用户界面中增加一个统一的鼠标选择器，允许用户选择目标 OS 预设或直接选择原始模式。OS profile 是目标机 OS 的输入兼容预设，不是主控机 OS 的自动检测结果。

## 2. 目标与非目标

### 2.1 目标

- 在 Iced 状态栏和 Web 视频状态栏中提供一个鼠标选择下拉框；
- 在 Iced 连接设置、默认设置和 Web 连接页/设置中提供同一组选项；
- 用一套共享的 profile 标识解析为当前实际的 `MouseMode`；
- profile 选择改变实际模式时，安全释放已有键鼠状态并同步本地光标捕获状态；
- 保留原始绝对/相对模式，便于硬件排查和覆盖 OS 预设；
- 兼容现有只保存 `mouse_mode` 的配置和连接 profile 文件；
- 让 Windows、Linux、BIOS、Android、macOS 的预设保持独立，即使当前有相同映射。

### 2.2 非目标

- 第一版不自动识别目标 OS；
- 第一版不根据 profile 自动调整相对灵敏度、DPI、采样周期或坐标校准；
- 第一版不重新设计 CH9329 协议和 HID 描述符；
- 第一版不修改 egui 桌面端；
- 第一版不声称 Linux、Android 或 macOS 的最终兼容性结论；
- 不把 OS profile 与用户保存的连接 profile（设备、波特率等）混成同一类对象。

## 3. 用户模型

### 3.1 选择项

下拉框按两组显示：

```text
目标 OS 预设
  Windows   （绝对）
  Linux     （相对）
  BIOS      （绝对）
  Android   （绝对）
  macOS     （绝对，暂定）

原始模式
  绝对
  相对
```

内部选择值不能只保存解析后的绝对/相对值，而应保留用户选择的是哪个 profile：

```text
Preset(Windows)
Preset(Linux)
Preset(Bios)
Preset(Android)
Preset(MacOs)
Raw(Absolute)
Raw(Relative)
```

当前映射为：

```text
Windows -> MouseMode::Absolute
Linux   -> MouseMode::Relative
BIOS    -> MouseMode::Absolute
Android -> MouseMode::Absolute
macOS   -> MouseMode::Absolute
```

解析函数只负责得到实际 `MouseMode`。后续可以在 profile 解析结果中增加灵敏度、坐标映射、捕获策略或按钮策略，而不改变用户选择值的语义。

### 3.2 默认值

默认选择为 `Raw(Absolute)`，保持当前行为，同时避免在用户没有确认目标 OS 时假定目标机是 Windows。选择器显示为“原始模式：绝对”或等价的本地化文本。

### 3.3 profile 与连接 profile 的关系

用户保存的连接 profile 仍然表示一组连接参数，包括视频设备、串口设备、波特率和鼠标选择。内置 OS profile 只是其中的一个字段值，不进入现有的“最近使用 profile”列表，也不占用 `active_profile` 的命名空间。

手动选择或修改鼠标项不应清除用户连接 profile 的设备选择；是否仍显示“当前连接 profile”由现有连接 profile 逻辑决定。鼠标项的手动修改只表示该连接 profile 的当前参数已被覆盖。

## 4. 交互设计

### 4.1 Iced 状态栏

当前状态栏的鼠标字段由纯文本改为可操作的下拉选择器。选中值显示 profile 名称和解析后的模式，例如：

```text
鼠标：Linux · 相对
```

相对模式的本地输入捕获状态与 profile 选择分开表示。状态栏需要能区分：

- 已选择相对模式，但视频区尚未获得输入焦点；
- 已选择相对模式且本地光标已被捕获/隐藏；
- 当前是绝对模式。

点击选择器后，选择结果立即应用于当前连接。若当前没有连接，则只更新当前连接参数，待连接时使用。

Iced 连接设置模态使用同一个选择器：

- 主页“连接设置”修改当前连接参数；已连接时立即应用；
- 菜单“设置”修改默认连接参数，只影响后续没有被连接 profile 覆盖的连接；
- 加载或保存用户连接 profile 时，鼠标选择跟随连接 profile 一起读写。

### 4.2 Web 视频状态栏

Web 视频状态栏增加同一组选项。当前已有的相对模式按钮不能简单删除，因为 profile 选择和浏览器 Pointer Lock 是两个状态：

- 下拉框决定目标端使用绝对还是相对输入；
- 独立的锁定按钮决定浏览器是否已经捕获和隐藏本地指针。

当当前 profile 解析为绝对模式时，Pointer Lock 按钮禁用或隐藏；当解析为相对模式时，按钮显示“锁定/退出锁定”状态。选择相对 profile 不强制调用 Pointer Lock，用户仍需通过用户手势触发浏览器锁定。这样可以处理浏览器的权限、焦点和 `Esc` 退出约束。

Web 连接页增加鼠标选择项，作为本次连接的草稿覆盖值。连接页选择项初始继承 Web 默认设置；提交连接时随 `POST /api/session` 发送，不能因为用户只调整连接草稿就意外修改全局默认设置。

Web 设置模态保存的是默认鼠标选择。默认设置用于没有连接页覆盖值的后续会话；视频页状态栏选择是当前会话的即时修改。

### 4.3 状态展示

状态栏不再只显示“相对模式”或绝对坐标。至少要能看到：

- 当前选择名称；
- 解析后的实际模式；
- Web/Iced 本地捕获状态（仅在适用时显示）。

profile 相同但实际模式相同的切换（例如 `Windows` 切换到 `BIOS`）只更新选择名称，不重复发送无必要的模式切换命令。

## 5. 共享数据模型与配置

### 5.1 共享层

在 `ipkvm-core` 增加与平台无关的鼠标 profile 枚举和解析方法。`MouseMode` 继续作为 CH9329 输入 sink 的底层运行时模式，不被删除或改名。

共享类型至少提供：

- 所有内置 profile 和原始模式的稳定标识；
- profile 到 `MouseMode` 的解析；
- 用于配置/API 校验的字符串标识解析；
- 未知标识的明确错误。

`ipkvm-core` 当前不依赖 serde，因此序列化适配留在桌面配置和 headless 设置模块；两者都调用共享枚举的稳定标识和解析方法，避免分别复制 Windows/Linux 等映射。

### 5.2 Iced 配置

桌面 `ConnectionSettings` 增加鼠标 profile 字段，运行时需要使用 profile 解析结果调用 `InputSink::set_mouse_mode`。现有 `mouse_mode` 配置作为兼容读取入口保留迁移逻辑：

- 旧配置只有 `mouse_mode = "absolute"` 时转换为 `Raw(Absolute)`；
- 旧配置只有 `mouse_mode = "relative"` 时转换为 `Raw(Relative)`；
- 新配置写入 profile 标识，并可同时写入解析后的 `mouse_mode` 作为旧版本读取兼容字段；
- profile 字段存在但未知时拒绝该配置并沿用现有错误提示/回退策略。

用户保存的 TOML 连接 profile 和上次手动连接快照都要经过同一套迁移逻辑。

### 5.3 Web 配置与 API

`WebSettings` 增加 `mouse_profile`，并保留 `mouse_mode` 的兼容表示。服务端创建 sink 时始终以 profile 解析出的模式为准。

接口约定：

- `GET/POST /api/settings` 读写默认 `mouse_profile`；
- `POST /api/session` 可选携带 `mouse_profile`，作为连接页本次会话的覆盖值；
- 增加当前会话鼠标 profile 的即时切换接口，接口执行释放、切换和状态更新的原子流程；
- `/api/status` 返回当前选择的 profile、解析后的模式和必要的捕获状态信息，供状态栏在轮询后保持一致；
- 旧客户端只发送 `mouse_mode` 时，服务端将其解释为对应的 `Raw` profile。

当前 headless 是单会话控制台，因此 profile 属于当前服务会话，而不是某个 RFB 客户端的独立偏好。RFB 输入泵已有 `SetMouseMode` 底层事件，新的 API 需要复用同一条输入状态切换路径，避免 API 和 RFB 事件产生两套释放/切换逻辑。

## 6. 切换流程与错误处理

### 6.1 实际模式发生变化

当旧 profile 和新 profile 解析出的 `MouseMode` 不同，执行以下顺序：

1. 暂停接收新的鼠标切换操作，保留 UI 的旧选择作为回滚值；
2. 调用输入 sink 的 `release_all()`，清除目标端键盘、鼠标按钮和滚轮状态；
3. 调用 `set_mouse_mode(new_mode)`；
4. 成功后提交新的 profile 选择；
5. 同步本地光标捕获：切到绝对模式时释放捕获并显示光标，切到相对模式时进入待捕获状态；
6. 失败时保持旧 profile 和旧本地捕获状态，并显示错误。

`release_all()` 可能释放用户正在按住的键，这是有意行为，因为模式选择本身会导致焦点或捕获状态变化，优先保证目标机没有卡键。

### 6.2 只改变 profile 名称

例如 `Windows -> BIOS` 都解析为绝对模式时，不需要发送 CH9329 模式命令，也不需要释放键鼠状态。只提交新的 profile 名称，后续 profile 扩展字段再决定是否需要额外动作。

### 6.3 Web Pointer Lock

Web 选择相对 profile 后只进入“待锁定”状态。用户点击锁定按钮后请求 Pointer Lock；浏览器触发 `pointerlockchange` 后再更新“已锁定”状态。窗口失焦、按 `Esc` 或 Pointer Lock 失败时，只退出本地捕获，不自动切回绝对 profile。

Web 选择绝对 profile 时立即退出 Pointer Lock，并停止发送相对 RFB `0x08` 消息。

### 6.4 离线和并发

- 没有活动会话时，选择器可以修改本地草稿或默认设置，不发送设备命令；
- 当前会话已经离线时，API 返回明确错误，前端保留当前实际 profile，不把失败选择显示成成功；
- Web 即时切换和会话重启必须串行化，避免旧会话释放与新会话创建交错；
- 连接页的 profile 覆盖值在会话创建失败时保留，便于用户修正设备而不丢设置；
- 状态轮询返回的服务端 profile 是最终事实，前端在成功请求后也要用响应或下一次状态轮询校正显示。

## 7. 输入与兼容性注意事项

- OS profile 是目标端输入栈的经验预设，不代表 OS 标准强制要求该模式；
- `Linux -> 相对` 当前主要依据 Ubuntu 实测，后续可独立修改 Linux 映射；
- `macOS -> 绝对` 和 `Android -> 绝对` 在第一版属于暂定映射；
- CH9329 绝对帧按钮位在本项目已有硬件实测中不可靠，绝对移动和按钮发送仍应遵循现有输入核心策略；
- 相对模式速度仍由主控 Raw Input、采样、DPI/视频比例、灵敏度和目标 OS 加速度共同决定，profile 第一版不隐藏这些参数；
- 不因 Windows 和 BIOS 当前都使用绝对模式就合并成一个 profile；
- 不通过 profile 名称自动检测或修改目标机 OS 设置。

## 8. 测试计划

### 8.1 共享模型测试

- 五个 OS profile 和两个原始模式都有稳定标识；
- 映射结果与当前产品约定完全一致；
- Windows/BIOS 等同模式 profile 可以独立比较；
- 未知 profile 标识返回明确错误；
- 旧 `mouse_mode` 值迁移为对应 `Raw` profile；
- 新 profile 序列化、反序列化和默认值正确。

### 8.2 Iced 测试

- 状态栏选择器包含两组完整选项；
- 连接设置和默认设置使用相同选项并写入正确作用域；
- 选择实际模式变化时按 `release_all -> set_mouse_mode` 顺序调用；
- sink 切换失败时 profile、状态栏和光标捕获状态回滚；
- Windows/BIOS 等同模式 profile 切换不发送多余模式命令；
- 保存/加载连接 profile 后鼠标选择保持；
- 相对模式失焦、退出和重新聚焦的捕获状态正确；
- egui 不加入本次回归范围。

### 8.3 Web 测试

- `/api/settings` 新旧字段兼容，默认 profile 正确；
- `/api/session` 的 profile 覆盖值参与会话 sink 构造；
- 即时切换 API 在成功和失败时都保持状态一致，并验证释放顺序；
- `/api/status` 返回当前 profile 和实际模式；
- 连接页、设置模态和视频状态栏显示同一套选项；
- 选择相对 profile 不会绕过浏览器用户手势要求直接假定 Pointer Lock 已建立；
- Pointer Lock 退出、失焦和切换绝对模式时停止相对消息；
- 浏览器测试验证 dropdown 选择、相对锁定按钮和连接重建后的 profile 保持。

### 8.4 人工硬件验证

自动化只能验证选择、状态和命令路径，不能替代目标 OS 的真实输入栈验证。实现完成后至少在已知环境复测：

- BIOS：绝对和相对；
- Windows 10：绝对 profile；
- Ubuntu：相对 profile，并记录 `evtest`/`libinput` 结果；
- Android 和 macOS：暂定绝对 profile；
- Windows 7：保持暂停，不作为本功能验收环境。

## 9. 实施边界

第一版实现应优先建立共享 profile 类型和解析测试，再分别接入 Iced、Web 配置/API、状态栏选择器和切换生命周期。egui 文件不改，避免在即将放弃的 UI 路径上产生新的兼容负担。

实现完成后，长期文档和 OS 实测记录继续沿用：OS profile 的具体映射可以独立更新，不需要重新设计 UI 或改变底层 `MouseMode` 接口。
