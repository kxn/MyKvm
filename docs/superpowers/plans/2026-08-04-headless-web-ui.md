# headless Web 控制台实施计划（#133/#141/#140）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成桌面右/中键位序统一（#133）、headless 后端前置（#141：设置 API/手动停止/相对指针 0x08）、headless 前端全量（#140：骨架/设置/特殊键/相对模式/截图/测试）。

**Architecture:** 三个 issue 各自独立分支与 PR（一个 PR 收口一个 issue）；#141 是 #140 的后端依赖，#133 与 #141 并行排期；实现按子任务派发 implementer，评审后合入。

**Tech Stack:** Rust workspace（iced 0.14、axum 0.8、tokio、ipkvm-rfb/session）、原生 ES 模块前端（无构建/无 CDN）、noVNC 1.7.0（本地 patch）、playwright-core 浏览器测试。

## Global Constraints

- 门禁：`cargo fmt --all --check`、`cargo test --workspace --all-features`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo doc -D warnings`；Web 资源门禁 `scripts/web-assets-tools.sh`（browser-tests 仅 playwright-core）。
- 设计/调研事实来源：`docs/superpowers/specs/2026-08-04-headless-web-ui-design.md` 与 `-research.md`；实现不得偏离已确认决策（手动停止标记、相对指针随首版、设置独立文件、语言跟随浏览器等）。
- 前端约束：不引 CDN、不引运行时 npm 依赖；页面仍 include_dir 嵌入；noVNC 只做本地最小 patch。
- 修复从根因：位序在桌面输入边界统一（Web 端 noVNC 已是 RFB 位序，不得在 pump 层换算）。
- 协议扩展：0x08 消息只影响 headless Web 相对模式，桌面端行为不变。
- 提交信息用英文 conventional commit，PR 描述含关联 issue/改动/测试/文档影响/人工验证例外；tea 写中文前设 UTF-8。

---

## Task 1: #133 桌面右/中键位序统一

**Files:**
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`（`mouse_button_bit` 映射改为 RFB 位序：Left=0b001、Right=0b100、Middle=0b010）
- Modify: `crates/ipkvm-desktop/src/app.rs`（egui 同款 `pointer_button_mask` 映射）
- Test: 映射单测 + iced RecordingSink 右/中键端到端（Right→sink 收到 RFB bit3；Middle→bit2）

**Acceptance:** 远程右击/中键语义正确（真机）；全量门禁绿。

**分支:** `codex/issue133-button-mask`；提交 `fix(desktop): align pointer button mask with RFB convention (#133)`。

---

## Task 2: #141 headless 后端前置

### 2a 运行时设置（配置目录 + /api/settings + 独立设置文件）

**Files:**
- Modify/Create: `crates/ipkvm-headless/src/settings.rs`（配置目录 helper + Settings DTO + headless-settings.toml 原子读写 + 校验）
- Modify: `crates/ipkvm-headless/src/web/service.rs`（GET/POST /api/settings 路由，axum State 注入设置存储，并发写串行化）
- Modify: `crates/ipkvm-headless/src/main.rs`（构造时注入设置存储；分层 CLI > config > runtime > 默认）
- Test: settings 读写/越界 400/损坏回退/原子写

### 2b 手动停止标记

**Files:**
- Modify: `crates/ipkvm-headless/src/web/service.rs`（ApiState 增加 manual_stop AtomicBool；stop 置位；create/restart 清除；status 暴露 session.manual_stop）
- Modify: `crates/ipkvm-headless/src/web/recovery.rs`（手动停止时不重建）
- Test: 手动停止不被复活、create/restart 后恢复自动恢复

### 2c 相对指针 0x08 协议

**Files:**
- Modify: `crates/ipkvm-rfb/src/protocol/client.rs`（type 0x08 解码：button_mask u8 + dx i16 + dy i16 + wheel i8，大端；ClientMessage::PointerRelative；非法长度/分片测试）
- Modify: `crates/ipkvm-session/src/rfb_connection/mod.rs`/`driver.rs`（RfbEvent::PointerRelative + driver 映射 RfbServerEvent::PointerRelative，带 client_id）
- Modify: `crates/ipkvm-session/src/rfb_input/pump.rs`（确认事件类型路由；若事件枚举已有则接线）
- Test: 解码器单测 + pump 端到端（0x08 → 相对移动/按钮/滚轮到达 sink）

**Acceptance:** 三块单测全绿；门禁绿。

**分支:** `codex/issue141-headless-backend`；提交 2-3 个（settings / manual-stop / protocol），PR 收口 `Closes #141`。

---

## Task 3: #140 headless 前端全量

### 3a 页面骨架与状态

**Files (web/):**
- Modify: `index.html`（工具栏/连接页/视频页/状态栏结构）
- Modify: `app.css`
- Create: `modules/app.js`（装配）、`modules/api.js`（fetch 封装）、`modules/status.js`（轮询/多标签状态机）、`modules/i18n.js`（zh/en）、`modules/screenshot.js`（下载，无帧置灰）
- Modify: `crates/ipkvm-headless/src/web/assets.rs`（新增静态路由）

### 3b 连接页与设置弹层

- Create: `modules/connection.js`（设备枚举/选择/探测状态/连接）；依赖 `POST /api/session`
- Create: `modules/settings.js`（弹层 + /api/settings 读写 + 恢复默认值）

### 3c 特殊键与键盘

- Create: `modules/special-keys.js`（全量组合菜单，复用 noVNC `sendKey` 序列）
- Create: `modules/keyboard.js`（keydown capture + preventDefault 转发可拦截组合）

### 3d 相对模式与截图复制

- Modify: `third_party/novnc/1.7.0/core/rfb.js`（本地 patch：pointer lock 时 mousemove 发 0x08；新增 relativePointerEvent 消息构造）
- Create: `modules/pointer.js`（requestPointerLock/pointerlockchange/失焦退出；触屏相对置灰）
- Create: `modules/clipboard.js`（navigator.clipboard 粘贴 + ClipboardItem 复制截图）

### 3e 浏览器测试

- Modify: `browser-tests/novnc-browser.mjs`（连接流程/设置/特殊键/多标签/截图/语言/相对指针）
- 若需要新增 fixture 支持（如相对指针协议注入），改 `crates/ipkvm-headless/src/bin/ipkvm-browser-fixture.rs`

**Acceptance:** 浏览器测试闭环 + Chrome/Edge 验收；门禁绿（含 web-assets-tools）。

**分支:** `codex/issue140-headless-web-ui`；PR 收口 `Closes #140`。

---

## 执行顺序

1. Task 1（#133）→ 2. Task 2（#141，2a→2b→2c）→ 3. Task 3（#140，3a→3b→3c→3d→3e）。
每任务：实现子代理 TDD → 评审 → 全量门禁 → PR 合并 → issue 关闭 → 台账/HANDOFF 同步。
