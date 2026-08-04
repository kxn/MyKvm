# headless Web 控制台技术调研

> 关联设计文档：`docs/superpowers/specs/2026-08-04-headless-web-ui-design.md`。
> 本文按开发流程“大设计先调研、后开单”产出，是重开 issue 与排实施计划的依据。
> 调研对象（读源码）：`crates/ipkvm-headless/src/`、`crates/ipkvm-rfb/src/protocol/client.rs`、
> `crates/ipkvm-session/src/rfb_connection/driver.rs`、`rfb_input/pump.rs`、
> `third_party/novnc/1.7.0/core/rfb.js`、`core/input/keyboard.js`、`browser-tests/novnc-browser.mjs`。

## 1. 结论摘要

- 后端 API 已具备设备枚举/会话创建/状态/截图，缺设置读写、手动停止标记与配置目录 helper；
- 相对指针协议扩展（0x08）可行且**已有半截预留**：`RfbServerEvent::PointerRelative` 已定义
  并被 pump 处理，但当前没有任何生产者（协议层没有对应客户端消息）；
- noVNC 相对鼠标必须本地 patch：键盘/指针事件入口明确（`Keyboard.grab`、`_handleMouseMove`、
  `RFB.messages.pointerEvent`），patch 点清晰；
- 浏览器保留快捷键不可靠拦截：页面能拦的走 keydown capture + preventDefault，拦不住
  （含 OS 级）必须靠特殊键菜单；
- 设置持久化建议独立运行时文件，分层 CLI > config > runtime > 默认；
- 真实浏览器测试基建已存在（playwright-core + 固定 fixture 流程），新 UI 需扩展。

## 2. 后端现状事实

### 2.1 已有 API（`crates/ipkvm-headless/src/web/service.rs`）

- `GET /api/status`：`service`/`video{source,frame,stalled}`/`controller`/
  `session{state: absent|stopped|running, input_events, dropped_frames, serial, input_offline}`；
- `GET /api/devices`：`{video:[{id,display_name}], serial:[{id,display_name}]}`（串口 id=路径）；
- `POST /api/session`：`create`（仅 absent 可用）、`restart`（video/serial 覆盖，失败回滚到
  上一选择）、`stop`；
- `GET /api/screenshot`：JPEG（无帧 503）。

### 2.2 配置（`config.rs`）

- 顶层：`[server]`、`[video]{camera,assets,fps}`、`[input]{serial,baud}`、`[auth]`；
- 合并优先级：CLI > 文件 > 默认；
- **没有配置目录 helper**：`--config` 是只读输入，运行时设置需要新增配置目录
  （可参考 `crates/ipkvm-desktop/src/config.rs` 的私有 `config_base_dir`，抽公共或复刻）。

### 2.3 会话与恢复（`recovery.rs` + `main.rs`）

- 恢复循环只在 `session.state == "stopped"` 且（`input_offline` 存在 或 视频从未出帧超 5s）
  时按指数退避重建；`api.selection == None` 时不重建；
- 手动 `stop` 后 `selection` 仍为 `Some`，恢复循环可能在满足条件时“复活”会话
  → 需要手动停止标记（运行态，AtomicBool 即可）；`create/restart` 或重启清除；
- 启动自动连接：`main.rs` 用 CLI/文件配置构造会话并 create+start；未指定设备时
  `--serial` 缺省用模拟队列、视频缺省开第一台相机——**“未指定即不连”是行为变更**，
  需要显式策略（配置指定才建会话）。

## 3. RFB 相对指针扩展调研

### 3.1 现状

- `ipkvm-rfb/src/protocol/client.rs`：`ClientMessageDecoder` 按 type 分派固定/变长消息
  （0=像素格式、2=编码、3=更新请求、4=键、5=指针、6=剪贴板、150=连续更新），
  未知 type 立即 fatal；消息类型 **0x08 空闲未占用**；
- `ipkvm-session/src/rfb_connection/mod.rs:99` 已定义 `RfbServerEvent::PointerRelative`，
  `rfb_input/pump.rs:378` 已按 `{client_id, button_mask, dx, dy, wheel}` 路由到
  `handle_relative_pointer`（sink 相对移动/按钮/滚轮均已实现并有单测）；
- 但全仓库没有任何代码产出 `RfbServerEvent::PointerRelative` → **通道预留、接线缺失**；
- 驱动映射：`rfb_connection/driver.rs` 把 `RfbEvent::Pointer` → `RfbServerEvent::Pointer`，
  同理新增 `RfbEvent::PointerRelative` 即可。

### 3.2 0x08 消息设计（建议）

- type `0x08`；长度 7：`button_mask u8 + dx i16(be) + dy i16(be) + wheel i8`；
- 协议层：`decode_relative_pointer` + `ClientMessage::PointerRelative`，补非法长度/分片测试；
- 事件链：decoder → `RfbEvent::PointerRelative` → driver → `RfbServerEvent::PointerRelative`
  → pump（复用现有相对路径，按钮/滚轮语义与 `handle_relative_pointer` 一致）；
- 掩码约定：Web 端直接用 noVNC/RFB 位序（bit1=Left、bit2=Middle、bit3=Right、
  bit4/5=滚轮上/下），与桌面端 #133 的换算互不影响（桌面端在 input 边界换算）。

### 3.3 noVNC patch 点（`third_party/novnc/1.7.0/core/rfb.js`）

- 指针事件入口：`_handleMouse`（mousedown/mousemove/…）→ `_handleMouseMove(x,y)`
  → `RFB.messages.pointerEvent`（标准绝对 0x06）；
- patch：`requestPointerLock()` 成功后，`mousemove` 走 `movementX/movementY` 增量，
  用新 `RFB.messages.relativePointerEvent`（本地新增，发 0x08）；按钮/滚轮沿用现有
  bmask/滚轮步进逻辑；
- `pointerlockchange`/`pointerlockerror` 处理：失焦/Esc 退出时恢复绝对模式并提示；
- 键盘：`core/input/keyboard.js` `Keyboard.grab()` 把 keydown/keyup 绑到 canvas，
  `_handleKeyDown` → `onkeyevent` → `rfb.sendKey`；特殊键菜单复用公开 `sendKey`。

## 4. 浏览器输入约束

- 键盘：noVNC Keyboard 需要画布聚焦；页面级 keydown capture 可拦截部分组合并
  `preventDefault` 转发（Ctrl+C/V/X、方向键、F 键等）；浏览器保留组合（Ctrl+W/T/N/
  Shift+T、Ctrl+R/F5/Shift+F5、Ctrl+Tab/Shift+Tab、Ctrl+Shift+I/J/C/U、Ctrl+P/S/F/
  H/D/O、F1/F11、Ctrl+Shift+Delete 等）与 OS 级（Alt+Tab/Win/Ctrl+Alt+Del）只能靠菜单；
- 指针锁定：`requestPointerLock()` 必须在用户手势内调用；Esc/失焦自动退出；
  `pointerlockchange` 事件驱动状态；`unadjustedMovement`（Chrome）可拿原始增量；
  触屏无指针锁定 → 相对模式置灰；
- 剪贴板：`navigator.clipboard.readText()`/`ClipboardItem` 需要 secure context
  （localhost/https）与用户手势，失败要提示；截图下载用 `<a download>` + blob，无权限问题。

## 5. 设置持久化

- 新增配置目录（Windows `%APPDATA%\my_ipkvm`、Linux `$XDG_CONFIG_HOME/my_ipkvm`、
  macOS `~/Library/Application Support/my_ipkvm`），`headless-settings.toml`；
- 分层：CLI > `--config` > 运行时设置 > 默认；
- 原子写（临时文件 + rename），损坏回退默认并 diag 记录；axum handler 内串行写
  （`tokio::sync::Mutex` 或文件锁）；
- 字段：baud_rate、auto_baud、preview_fps、mouse_mode（默认 absolute）、
  relative_sensitivity、scale_mode。

## 6. 前端结构与测试基建

- 页面：`web/index.html` + `app.css` + `app.js`（现状极简：连接状态 + 重连/断开 +
  许可证，打开即连 `/rfb`）；静态资源走 `web/assets.rs` include_dir + 白名单路由；
- 约束：不引 CDN、不引运行时 npm 依赖（`scripts/web-assets-tools.sh` 门禁校验
  `browser-tests/package.json` 仅 playwright-core 固定版本）；
- 测试：`browser-tests/novnc-browser.mjs` 已有 fixture 流程（READY 行、像素断言、
  键盘/指针序列、布局断言、断开/重连）；新 UI 需扩展为多页面/多 API 的测试夹具
  （连接页、设置、特殊键、多标签、截图、相对指针）。

## 7. 建议拆单结构（调研后，重开时按此粒度）

后端前置（相互独立，可并行）：

- A1 运行时设置：配置目录 helper + `GET/POST /api/settings` + `headless-settings.toml`；
- A2 手动停止标记：`/api/session stop` 置标记 + 恢复循环尊重 + status 暴露；
- A3 相对指针 0x08：协议解码 + `RfbEvent::PointerRelative` 接线 + pump 端到端测试
  （noVNC patch 属于前端，见 B4，但协议联调可同单或分单）；
- A4 #133 位序统一（既有 bug，独立前置）。

主实施（前端，依赖如上）：

- B1 页面骨架：工具栏/连接页/视频页/状态轮询/多标签/语言/截图下载（依赖 A2）；
- B2 设置弹层接入（依赖 A1）；
- B3 特殊键全量菜单 + 键盘拦截转发（独立）；
- B4 相对模式：pointer lock + noVNC 0x08 patch + 复制截图（依赖 A3、A4）。

收尾：browser-tests 扩展、Chrome/Edge 兼容矩阵、人工验收（真实相机 + CH9329）。

## 8. 风险与待验证

- 0x08 端到端：协议/驱动/pump/noVNC 四段联调，真实浏览器验证 movement 与按钮；
- 浏览器保留键的可拦截性存在浏览器差异（以 Chrome/Edge 目标，菜单兜底）；
- pointer lock 在非聚焦/iframe/全屏下的行为差异；
- `settings.toml` 并发写与损坏恢复；
- “未指定设备即不连”是行为变更，需确认 CLI 默认（无 --serial/--camera）语义。
