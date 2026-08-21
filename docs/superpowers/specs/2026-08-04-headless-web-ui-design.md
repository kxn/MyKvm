# headless Web 控制台设计（浏览器版全对齐）

> 关联：Gitea `kxn/my_ipkvm`；本文是 headless 浏览器版 UI 的长期事实来源。
> 前置调研：`crates/ipkvm-headless/src/web/service.rs`、`recovery.rs`、`config.rs`、
> `browser-tests/`、`third_party/novnc/1.7.0/core/rfb.js`。

> **取代声明（2026-08-21，#97/#104）**：本文第 4 节「页面与组件」及第 5.2 节的
> UI 组织内容已被 `2026-08-17-headless-web-ui-redesign-design.md` 全面取代
> （悬浮控制条、连接向导、设置分区、特殊键面板、状态抽屉、双主题与 UI 实施规范
> 以新文档为准）。本文其余部分——API 契约（第 6 节）、输入设计（第 7 节）、
> 设置持久化分层（第 8 节）、会话与自动连接策略（第 5 节）、错误恢复与测试策略——
> 仍然有效。

## 1. 背景与目标

headless 版目前只有极简页面：一条状态栏（连接状态/重连/断开/许可证）+ noVNC 画布，
打开即连 `/rfb`，没有设备选择、设置、菜单、状态细节。后端已有
`GET /api/status`、`GET /api/devices`、`POST /api/session`（restart/create/stop）、
`GET /api/screenshot`，以及会话自动恢复循环。

目标：把浏览器版做成与桌面 iced 端功能**全对齐**的控制台，但按浏览器惯例组织 UI：

- 连接页：设备选择/探测/连接（无配置或要切换设备时）；
- 设置：单层弹层（连接参数默认值 + 缩放），不做 profile；
- 会话：配置指定设备 → 服务端自动连；未指定 → 显示连接页；
- 输入：全量特殊键菜单（浏览器会拦截的组合）、绝对/相对鼠标（相对用 pointer lock）、粘贴；
- 截图：下载为主、复制到剪贴板为辅；
- 状态：轮询 `/api/status`，多标签状态一致；
- 语言：跟随浏览器 + 工具栏手动切换。

## 2. 明确不做（YAGNI）

- 不做桌面式菜单栏：用 Web 惯例顶部工具栏 + 下拉/弹层（用户已确认）。
- 不做 profile 保存/加载/最近使用：机器级配置由服务端 config + 运行时设置承担。
- 不引入前端构建链/CDN/运行时 npm 依赖：继续纯原生 ES 模块 + `include_dir` 嵌入。
- 不重写 noVNC：保留上游，相对鼠标扩展走本地 patch（见 §7.3）。

## 3. 总体架构

```
浏览器（原生 ES 模块，无构建）
  ├─ index.html / app.css / app.js（入口与装配）
  ├─ modules/：toolbar / connection / video / settings / special-keys /
  │            status / session / screenshot / i18n / input
  ├─ /vendor/novnc/…（noVNC 画布与键盘/指针，本地最小 patch）
  └─ fetch 调用后端 JSON API；画布走 /rfb WebSocket
服务端（axum，已有）
  ├─ 静态资源 include_dir
  ├─ /api/status /api/devices /api/screenshot（已有）
  ├─ /api/session（已有，需补语义：手动停止标记，见 §6.3）
  └─ /api/settings（新增：GET/POST 运行时设置）
```

数据流：

- 页面轮询 `GET /api/status`（连接页 1s、视频页 2s；失败退避 5s）驱动视图与状态栏；
- 连接/切换设备 = `POST /api/session`（restart/create），断开 = `stop`；
- 设置读写 = `GET/POST /api/settings`；
- 画面与按键（可转发部分）走 noVNC `/rfb`；被浏览器拦截的组合走特殊键菜单。

## 4. 页面与组件

### 4.1 顶部工具栏（常驻）

- 左侧：标题 + 会话状态指示（运行中/停止/恢复中/错误 + 当前设备摘要）；
- 按钮组：连接/断开、设置、特殊键▾、截图▾、语言▾、许可证链接。

### 4.2 连接页（会话不存在或用户点“连接/切换设备”时显示）

- 视频设备下拉 + 刷新 + 探测状态；
- 串口（CH9329）下拉 + 刷新 + 探测状态；
- “连接”按钮（两者就绪才亮）；
- 连接参数只读摘要（来自设置，避免与设置弹层重复编辑）。

### 4.3 视频页（会话运行中）

- noVNC 画布（缩放适配、点击聚焦）；
- 底部状态栏：视频分辨率、控制设备、会话状态/错误、串口统计、输入统计、消息。

### 4.4 设置弹层（单层）

字段（对齐桌面表单，服务端保存）：

- 波特率 1200..=115200（数字输入）；
- 自动波特率（布尔）；
- 预览 FPS 1..=60；
- 默认鼠标模式（绝对/相对，**浏览器默认绝对**）；
- 相对灵敏度 0.1..=5.0；
- 缩放模式（适配窗口/原始大小/窗口跟随视频）；
- 恢复默认值按钮。

### 4.5 特殊键弹层（全量）

分组（详见 §7.1 按键表）：桌面四键、标签页类、刷新/导航类、开发者/查看类、其它。

### 4.6 截图

- “保存截图”：请求 `/api/screenshot` → `image/jpeg` → `<a download>` 下载；
- “复制截图”：`ClipboardItem`（需 secure context + 用户手势，失败提示）；
- 无帧（`/api/status.video.frame == null`）时两项置灰。

### 4.7 语言

- 默认 `navigator.language`（zh 系 → zh-CN，否则 en）；
- 工具栏切换（中文/English/跟随浏览器），选择存 localStorage；
- 文案走 `modules/i18n.js`（zh/en 两份对象，不引第三方 i18n）。

## 5. 会话与自动连接（混合策略）

### 5.1 服务端启动

- 配置（CLI 或 config 文件）指定了视频/串口 → 启动即建会话（沿用现有 `create` 路径）；
- 未指定 → 不建会话（`absent`），Web 显示连接页；
- 自动恢复循环维持现有语义，并新增“手动停止”保护（§6.3）。

### 5.2 Web 连接/切换

- 连接页点“连接” = `POST /api/session {"action":"create"}`（absent 时）或
  `{"action":"restart","video":…,"serial":…}`（切换设备）；
- 工具栏“断开” = `POST /api/session {"action":"stop"}`，并标记手动停止；
- 切换设备失败（历史语义）：服务端现有“回滚上一会话选择”逻辑保留，前端展示错误原因并回到连接页。

#55 已更新该行为：video/control 打开失败进入共享 supervisor 的恢复状态，Web 保持视频页，
在状态栏显示输入或视频失败；前端不因单链路失败回连接页。

决策（2026-08-04 已确认）：手动停止后保持停止，直到再次 create/restart 或服务重启。
实现见前置单 #136。

### 5.3 视图状态机

```
absent/stopped（非手动） ──连接──▶ running ──断开/设备切换──▶ …
      │                                   │
      └──── 连接页 ◀──被切换/停止/失败────┘
running 且多标签：状态轮询驱动；任一标签断开/切换，其它标签同步回连接页并提示原因。
```

## 6. API 契约

### 6.1 现有接口（沿用）

- `GET /api/status` → `{service, video:{source,frame,stalled}, controller, session:{state,stats,serial,input_offline}}`；
- `GET /api/devices` → `{video:[{id,display_name}], serial:[{id,display_name}]}`；
- `GET /api/screenshot` → JPEG（无帧 503）；
- `POST /api/session`：`create`（仅 absent/manual stopped）、`restart`（带 video/serial 覆盖，
  构建失败进入 supervisor 恢复态）、`stop`。

### 6.2 新增 `GET/POST /api/settings`

DTO（与设置弹层字段一致）：

```json
{
  "baud_rate": 115200,
  "auto_baud": true,
  "preview_fps": 30,
  "mouse_mode": "absolute",
  "relative_sensitivity": 1.0,
  "scale_mode": "fit_window"
}
```

持久化：写运行时设置文件（见 §8），不覆盖用户 `--config` 文件。

### 6.3 `/api/session` 语义补充：手动停止标记

- `stop` 设置“手动停止”标记（运行时会话状态，非持久化），恢复循环不复活；
- “手动停止”在下次 `create/restart` 或服务重启时清除；
- 恢复循环仅在非手动停止且满足既有条件（输入离线 / 视频从未出帧）时重建；
- 状态响应暴露 `session.manual_stop`，前端显示“已手动停止”。实现见前置单 #136。

## 7. 输入设计

### 7.1 键盘与特殊键

浏览器保留快捷键不可靠拦截，分两类：

- **页面可拦截**（keydown capture + `preventDefault` 后走 noVNC 转发）：普通键、
  常见组合（Ctrl+C/V/X、方向键、F 键等）；
- **浏览器/OS 保留、必须靠菜单**：
  - 标签页/窗口：Ctrl+W、Ctrl+T、Ctrl+N、Ctrl+Shift+T、Ctrl+Tab/Ctrl+Shift+Tab、
    Ctrl+Shift+N；
  - 刷新/导航：F5、Ctrl+R、Ctrl+Shift+R、Alt+←/→；
  - 开发者/查看：Ctrl+Shift+I/J/C、Ctrl+U、F11、F1；
  - 其它：Ctrl+P/S/F/H/D/O、Ctrl+Shift+Delete；
  - OS 级：Ctrl+Alt+Del、Win、Alt+Tab、PrintScreen。

特殊键菜单 = 桌面四键 + 上述浏览器保留组合；菜单项复用现有
`special_key_sequence` 式的“按键序列发送”通道（Web 端实现为 noVNC `sendKey` 序列）。

### 7.2 鼠标

- 默认绝对模式（设置可改默认值）：noVNC 原生绝对指针，点击画布直接发送；
- 相对模式：相对 profile 生效后，视频画布的首次用户点击自动在手势上下文内调用
  `requestPointerLock()`；锁定后把 `movementX/Y` 增量经相对指针扩展发送。独立的
  相对模式按钮仍用于显式锁定/退出；`pointerlockchange`/失焦/Esc 退出并释放；
- 触屏/无指针锁定环境：相对模式置灰并提示；
- 位序：遵循 §11 的右/中键位序统一修复（#133）后再定前端掩码映射。

### 7.3 相对指针协议扩展（决策：随首版实现）

决策（2026-08-04 已确认）：相对模式随首版一起做，不后置。实现见前置单 #134。

现状：RFB 标准客户端消息只有绝对指针（0x06）；`rfb_input::pump` 已支持
`RfbServerEvent::PointerRelative`，但那是桌面端内部事件，WebSocket 客户端没有相对路径。
浏览器相对模式需要：

- RFB 协议新增客户端→服务端消息（建议 type 0x08：button_mask + dx + dy + wheel），
  在 `ipkvm-rfb/protocol/client.rs` 解析并路由到 pump 的相对事件；
- noVNC 本地 patch：pointer lock 激活时改用相对消息发送 movementX/Y；
- 该扩展只影响 headless Web 相对模式，桌面端不变。

### 7.4 粘贴

- 工具栏“粘贴”：`navigator.clipboard.readText()`（secure context/localhost）→
  走 noVNC `clipboardPasteFrom()`（服务端已有 cut text 通道）；
- 权限失败/非 secure context：置灰并提示。

## 8. 设置持久化与分层

优先级：CLI > `--config` 文件 > 运行时设置（`/api/settings`） > 默认值。

- 运行时设置文件：`<配置目录>/headless-settings.toml`（Windows `%APPDATA%\my_ipkvm`，
  Linux `$XDG_CONFIG_HOME/my_ipkvm`，macOS `~/Library/Application Support/my_ipkvm`），
  原子写（临时文件 + rename），损坏时回退默认并记录；
- 只持久化 Web 设置字段；视频/串口设备仍由 CLI/`--config` 指定（连接页切换是运行时会话，
  不写盘，除非将来用户要求“记住设备”）。

## 9. 错误处理与恢复展示

- 轮询失败：状态栏显示“状态获取失败”，按退避重试，不弹窗；
- 会话错误：展示 `input_offline.reason` / `session.state` / 切换失败 detail；
- 恢复中：显示“正在恢复（第 n 次重试）”，视频页保持但置半透明遮罩提示；
- 截图无帧：按钮置灰；
- 所有 API 非 2xx：解析 `{error, detail}` 展示，不吞错。

## 10. 测试策略

- 后端单测：`/api/settings` 读写/原子写/损坏回退、`/api/session` 手动停止标记、
  相对指针消息解析与路由（rfb protocol + pump）、恢复循环不复活手动停止；
- 真实浏览器测试（沿用 `browser-tests/` + playwright-core 门禁）：
  - 连接页：枚举/选择/连接/断开/切换设备/失败回滚展示；
  - 设置：读写/恢复默认/跨标签一致；
  - 特殊键：菜单发送组合键到达 RFB 协议层（断言服务端收到对应 key 序列）；
  - 多标签：A 标签断开 → B 标签状态同步；
  - 截图：下载文件名/内容、无帧置灰；
  - 语言：跟随浏览器 + 手动切换；
  - 相对指针：pointer lock 后 movement 经 0x08 消息到达 pump（Phase 3）。
- 人工验收清单：真机/真实相机 + CH9329、浏览器兼容矩阵（Chrome/Edge 为主）。

## 11. 待讨论问题（设计阶段发现）

## 11. 已确认决策与前置单（2026-08-04）

1. 手动停止标记：接受（#136）。
2. 相对指针协议扩展 + noVNC patch：随首版一起做（#134）。
3. 设置写盘：独立运行时设置文件，不与 `--config` 混写（用户不持异议，保持 §8 方案，#135）。
4. 触屏：相对模式置灰；首版只保证 Chrome/Edge 桌面浏览器。
5. 右/中键位序：以 #133 统一修复为准，前端掩码映射依赖它。

前置单清单（排计划时一起排，2026-08-04 重开后最终粒度）：
- #133：桌面右/中键位序（独立桌面 bug）；
- #141：headless 后端前置合并单（运行时设置 API + 手动停止标记 + 相对指针 0x08 协议）；
- #140：headless 前端全量主实施（骨架/设置/特殊键/相对模式/截图/测试）。

## #159 组装边界更新（2026-08-04）

Web API 的 `/api/devices` 通过构造时注入的 `DeviceInventoryProvider` 枚举设备，JSON
字段和错误状态码不变。正式 app 注入真实 provider，browser fixture 注入静态 provider。
Web/RFB library 不打开硬件；打开 camera/serial 的职责仍在各 app 的 `SessionFactory`。
#55 后，stop/释放/重建顺序仍由共享 supervisor 保证；新设备打开失败不再按上一成功选择
回滚，而是进入对应链路的恢复态并由 `/api/status` 暴露。

## 12. 建议里程碑（供后续 writing-plans 拆分）

- 前置批次：#133（位序）→ #141（后端前置合并单：设置/手动停止/0x08 协议）；
- 主实施：#140（前端全量：骨架/设置/特殊键/相对模式/截图/语言/多标签）；
- 收尾：browser-tests 全量闭环与 Chrome/Edge 兼容矩阵验收（并入 #140）。

## 13. #151/#154/#156 输入控制实现边界（2026-08-04）

headless Web 的目标端 profile、RFB 实际鼠标模式和浏览器本地 Pointer Lock 是三个不同
状态：profile 决定目标端模式，输入泵确认 sink 已应用的实际模式，Pointer Lock 只由用户
手势建立并由浏览器事件维护。服务端状态接口不返回虚构的本地捕获值。

当前会话 profile 通过 `POST /api/input/mouse-profile` 切换。接口把模式事件送入当前
RFB 输入泵并等待 sink 确认；确认前不更新 session selection，失败则返回错误。连接页的
profile 是会话草稿覆盖值，设置页的 profile 是默认值，两者不会互相覆盖。

相对移动在 noVNC 侧累计并按 33 ms 调度，控制事件先冲刷待发移动；退出 Pointer Lock、
绝对 profile、RFB disconnect 和 DOM 清理都会清除相对定时器与累计量。相对 profile 生效后，
输入泵必须保持目标端的 `MouseMode::Relative`。Pointer Lock 建立前 noVNC 可能产生一笔
绝对过渡事件，这类事件不得把 sink 降级为绝对模式，也不得使输入泵离线；只有 Pointer
Lock 后的相对消息才驱动目标端移动。
