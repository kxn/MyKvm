# 运行时会话选择实施计划

> **给自动化协作者：** 本计划需要按任务逐项执行；步骤使用 checkbox（`- [x]`）记录进度。

**目标：** 让 headless 在运行时通过 `/api/session` 选择并切换视频/串口设备，同时为 desktop 版本固定同一套会话重启设计。

**架构：** 使用 `SessionFactory` 把请求中的设备选择转换为帧源和输入 sink；运行时真实设备切换先由 `SessionManager::stop_and_destroy` 停旧并释放独占资源，再构建新组件并 `create` + `start`；`SwitchableFrameSource` 让状态、截图和新 RFB 连接读取当前帧源。旧连接不做无缝迁移。

**技术栈：** Rust、tokio、axum、ipkvm-session、ipkvm-video、ipkvm-core。

## 全局约束

- 仓库内自写文档使用中文。
- 非平凡改动围绕 Gitea issue #32 和 #33。
- 新增或修改核心逻辑先补失败测试，再实现。
- Rust 验证默认运行 `cargo fmt --all --check` 与 `cargo test --workspace --all-features`。
- 不实现真正无缝热替换；运行时换设备采用会话级停旧启新。
- 运行时换真实设备必须先释放旧帧源/sink，再打开新设备；新设备构建失败时尝试回滚上一成功选择。

---

### 任务 1：固定运行时换视频的 API 契约

**文件：**
- 修改：`crates/ipkvm-headless/tests/web_http.rs`

**接口：**
- 消费：当前 `POST /api/session`。
- 产出：失败测试 `api_session_restart_with_video_switches_current_frame_source`。

- [x] **步骤 1：编写失败测试**

在 `web_http.rs` 的 session 测试附近加入：启动时帧源为 2x1，`restart` 带 `video:"wide"` 后，`/api/status` 的当前帧变为 4x1，且新源名为 `wide`。

- [x] **步骤 2：运行测试确认失败**

Run: `cargo test -p ipkvm-headless --all-features --test web_http api_session_restart_with_video_switches_current_frame_source -- --nocapture`

Expected: FAIL，当前实现返回 501 或仍报告旧帧源。

- [x] **步骤 3：在任务 2/3 后实现**

本任务只锁定契约；实现放在后续任务。

### 任务 2：增加可切换当前帧源

**文件：**
- 新建：`crates/ipkvm-headless/src/frame_source.rs`
- 修改：`crates/ipkvm-headless/src/lib.rs`

**接口：**
- 产出：`SwitchableFrameSource::new(Arc<dyn FrameSource>)`、`SwitchableFrameSource::set_current(Arc<dyn FrameSource>)`。

- [x] **步骤 1：编写单元测试**

测试 `latest_frame()`、`source_info()`、新 `subscribe()` 在切换后来自新帧源。

- [x] **步骤 2：确认红灯**

Run: `cargo test -p ipkvm-headless --all-features frame_source::tests::switching_changes_latest_frame_and_new_subscribers -- --nocapture`

Expected: FAIL，模块不存在。

- [x] **步骤 3：实现**

实现一个内部持 `RwLock<Arc<dyn FrameSource>>` 的轻量委托类型。已有订阅不迁移，新订阅读取当前源。

### 任务 3：SessionManager 支持替换并启动

**文件：**
- 修改：`crates/ipkvm-session/src/session_manager.rs`

**接口：**
- 产出：`SessionManager::replace_and_start(frame_source, sink, gate) -> impl Future<Output = Result<(), SessionError>>`。
- 产出：`SessionManager::stop_and_destroy() -> impl Future<Output = Result<(), SessionError>>`。

- [x] **步骤 1：编写失败测试**

覆盖 running 状态替换、stopped 状态替换、empty 状态替换。

- [x] **步骤 2：确认红灯**

Run: `cargo test -p ipkvm-session --all-features session_manager::tests::replace_and_start -- --nocapture`

Expected: FAIL，方法不存在。

- [x] **步骤 3：实现**

如有运行中会话，先 `stop()` + `wait_stopped()`；然后替换 `ConsoleSession` 并 `start()`。

### 任务 4：接线 `/api/session` 到真实设备选择

**文件：**
- 修改：`crates/ipkvm-headless/src/web/service.rs`
- 修改：`crates/ipkvm-headless/src/main.rs`
- 修改：受影响测试与 fixture 调用方

**接口：**
- 消费：`SwitchableFrameSource`、`SessionManager::stop_and_destroy`、`SessionManager::create`、`SessionManager::start`。
- 产出：`SessionSelection { video, serial }` 和 `SessionFactory::build(&SessionSelection)`。

- [x] **步骤 1：把测试从 501 改为成功重启**

删除“device switching not implemented”期望，改为成功切换。

- [x] **步骤 2：实现 web/service**

`restart` 先 `stop_and_destroy` 并把 `SwitchableFrameSource` 临时切到空帧源，释放旧独占资源；随后构建新组件，`create` + `start` 成功后 `switchable.set_current(new_source)`。构建失败时按上一成功选择尝试回滚。

- [x] **步骤 3：实现 main factory**

请求中的 `video` 覆盖 `Options.camera_name` 并清空 `assets_dir`；`serial` 覆盖 `Options.serial_path`，空字符串表示模拟队列。

### 任务 5：修复 browser fixture 生命周期

**文件：**
- 修改：`crates/ipkvm-headless/src/bin/ipkvm-browser-fixture.rs`
- 测试：`crates/ipkvm-headless/tests/browser_fixture.rs`

**接口：**
- 消费：现有 fixture 测试。
- 产出：STOP/EOF 后事件通道关闭，输入泵退出。

- [x] **步骤 1：用现有失败测试作为红灯**

Run: `cargo test -p ipkvm-headless --all-features --test browser_fixture -- --test-threads=1 --nocapture`

Expected: FAIL in current tree.

- [x] **步骤 2：实现**

保留 watch sender；停止时先发布 `None` 并 drop 所有 `event_tx` clone，再等待 input task。

### 任务 6：文档与全量验证

**文件：**
- 修改：`docs/superpowers/specs/2026-08-02-product-apps-wiring-design.md`
- 修改：`README.md`

**接口：**
- 产出：文档状态与代码一致。

- [x] **步骤 1：运行格式检查**

Run: `cargo fmt --all --check`

- [x] **步骤 2：运行工作区测试**

Run: `cargo test --workspace --all-features`

- [x] **步骤 3：运行最终验证**

Run: `scripts/verify.ps1`

### 任务 7：审查修正与回归测试

**文件：**
- 修改：`crates/ipkvm-session/src/rfb_connection/finalize.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/server.rs`
- 修改：`crates/ipkvm-headless/src/rfb_ws/service.rs`
- 修改：`crates/ipkvm-headless/tests/headless_process.rs`
- 修改：`crates/ipkvm-headless/tests/web_http.rs`

**接口：**
- 产出：会话结束型旧事件通道关闭不毒化 `RfbConnectionGate`。
- 产出：运行时重启先释放旧资源，再调用工厂构建新资源。

- [x] **步骤 1：活动连接重启回归**

覆盖活动 TCP/WS 连接期间 `POST /api/session restart` 后，旧连接关闭并释放 gate，新 TCP/WS 连接能重新握手。

- [x] **步骤 2：独占资源释放顺序回归**

覆盖旧帧源未 drop 时工厂会失败的场景，保证 restart 在构建新资源前已释放旧帧源/sink。

- [x] **步骤 3：状态统计与 status 一致性**

`/api/status` 从同一个当前帧源读取 `source_info` 和 `latest_frame`，并在读取 session stats 前刷新丢帧与串口队列统计。

## 自审

- 覆盖了 #32 的设备列表、会话切换、状态扩展和鉴权测试。
- #33 只落设计，不在本轮实现窗口 UI。
- 没有真正无缝热替换承诺；旧连接行为明确为可断开重连。
- 每个核心生产改动都有对应失败测试入口。
