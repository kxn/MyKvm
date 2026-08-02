# 会话核心归位 ipkvm-session 实现计划（issue #30）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `rfb_connection` 与 `rfb_input` 从 ipkvm-headless 机械搬入 ipkvm-session，headless 用 `pub use` 重新导出保持 API 兼容，行为不变。

**Architecture:** session 成为两个产品 app（headless/desktop）共享的会话核心：连接驱动 + 事件模型 + 仲裁 + 输入泵整体上移；headless 保留传输适配（rfb_tcp/rfb_ws）与 Web 服务。搬迁为文件级移动 + import 路径调整，不改变任何行为。

**Tech Stack:** Rust、tokio（mpsc/watch）、serialport（serial feature）、ipkvm-rfb/ipkvm-video/ipkvm-core。

## Global Constraints

- 自写文档为中文。
- 依赖单向：`headless → session → core/video/rfb`。
- 搬入后 headless lib 对外 API 完全兼容（`use ipkvm_headless::rfb_*` 仍可用）。
- 行为不变：所有现有测试不破（`cargo test --workspace --all-features` 全绿）。
- `scripts/verify.ps1` 通过（含真实浏览器闭环）。
- 提交信息用 conventional commit 风格。

---

## 文件结构

**session crate（搬入 + 新增）：**
- `crates/ipkvm-session/src/lib.rs` — 重写为完整模块声明 + `HeadlessConfig`（从 headless lib 迁入）
- `crates/ipkvm-session/src/rfb_connection/` — 从 headless 整目录搬入（driver/finalize/frame/gate/pending/transport/mod.rs）
- `crates/ipkvm-session/src/rfb_input/` — 从 headless 整目录搬入（keyboard/keymap/mod.rs/pointer/pump/text.rs）
- `crates/ipkvm-session/src/devices.rs` — 新增：视频 + 串口设备枚举
- `crates/ipkvm-session/src/console_session.rs` — 新增：ConsoleSession 真实组装（帧源+输入泵+gate+事件出口）
- `crates/ipkvm-session/src/serial_stats.rs` — 新增：串口统计类型（从 CommandQueue::stats 映射）
- `crates/ipkvm-session/src/session_manager.rs` — 新增：会话创建/重启/停止（委托 ConsoleSession）
- `crates/ipkvm-session/Cargo.toml` — 新增 ipkvm-rfb、tokio、serialport（serial feature）、thiserror

**headless crate（搬运后调整）：**
- `crates/ipkvm-headless/src/lib.rs` — 移除 rfb_connection/rfb_input 模块，改为 `pub use ipkvm_session::{rfb_connection, rfb_input, ...}` 重新导出 + 保留 HeadlessConfig
- `crates/ipkvm-headless/src/rfb_tcp/` `rfb_ws/` `web/` `main.rs` `bin/` — `crate::rfb_connection` → `ipkvm_session::rfb_connection`，`crate::rfb_input` → `ipkvm_session::rfb_input`
- `crates/ipkvm-headless/tests/` — 不改（引用 `ipkvm_headless::rfb_*`，由重新导出满足）
- `crates/ipkvm-headless/Cargo.toml` — 移除随迁的测试/dev-deps 视搬迁后编译结果

---

## Task 1: 迁入 rfb_connection 模块到 session

**Files:**
- Create: `crates/ipkvm-session/src/rfb_connection/`（从 headless 整目录复制：driver.rs, finalize.rs, frame.rs, gate.rs, pending.rs, transport.rs, mod.rs）
- Delete: `crates/ipkvm-headless/src/rfb_connection/`（原目录删除）
- Modify: `crates/ipkvm-session/src/lib.rs`
- Modify: `crates/ipkvm-session/Cargo.toml`

**Interfaces:**
- Produces: `ipkvm_session::rfb_connection::{RfbConnectionGate, RfbServerEvent, RfbTransportKind, RfbClientId, RfbDisconnectReason, RfbConnectionSettings, RfbConnectionSettingsError, RfbFrameError}`（与 headless 原路径完全同名同型）
- Consumes: 无（纯搬迁，类型来自 ipkvm-rfb/ipkvm-video）

- [ ] **Step 1: 复制目录**

```bash
mkdir -p crates/ipkvm-session/src
cp -r crates/ipkvm-headless/src/rfb_connection crates/ipkvm-session/src/
```

- [ ] **Step 2: 修改 session lib.rs 声明模块**

```rust
pub mod rfb_connection;
```

（当前 session lib.rs 只有 `use ipkvm_core::MouseMode;` 和 `ConsoleSession` 空壳，在文件头加模块声明。）

- [ ] **Step 3: 修改 session Cargo.toml 加依赖**

```toml
[dependencies]
ipkvm-rfb = { path = "../ipkvm-rfb" }
tokio = { workspace = true, features = ["io-util", "net", "rt", "sync"] }
# 原 ipkvm-core / ipkvm-video 保留
```

- [ ] **Step 4: 删除 headless 原目录**

```bash
rm -rf crates/ipkvm-headless/src/rfb_connection
```

- [ ] **Step 5: 编译验证**

Run: `cargo check -p ipkvm-session`
Expected: 编译通过，`rfb_connection` 模块可用。

- [ ] **Step 6: 提交**

```bash
git add crates/ipkvm-session crates/ipkvm-headless
git commit -m "refactor(session): 迁入 rfb_connection 连接驱动与事件模型到 ipkvm-session"
```

---

## Task 2: 迁入 rfb_input 模块到 session

**Files:**
- Create: `crates/ipkvm-session/src/rfb_input/`（从 headless 整目录复制：keyboard.rs, keymap.rs, mod.rs, pointer.rs, pump.rs, text.rs）
- Delete: `crates/ipkvm-headless/src/rfb_input/`（原目录删除）
- Modify: `crates/ipkvm-session/src/lib.rs`（加 `pub mod rfb_input;`）

**Interfaces:**
- Produces: `ipkvm_session::rfb_input::{RfbInputPump, RfbInputRunError, RfbInputError, RfbInputNotice, RfbKeyboardMapper, RfbPointerMapper, TextInputService, TextInputConfig}` 等（与 headless 原路径同名同型）
- Consumes: `ipkvm_session::rfb_connection`（pump.rs 引用 `RfbClientId/RfbDisconnectReason/RfbServerEvent`——**注意 import 路径**：`crate::rfb_connection` → `crate::rfb_connection` 在 session 内相同，无需改）

- [ ] **Step 1: 复制目录**

```bash
cp -r crates/ipkvm-headless/src/rfb_input crates/ipkvm-session/src/
```

- [ ] **Step 2: 修改 session lib.rs 声明模块**

```rust
pub mod rfb_input;
```

- [ ] **Step 3: 检查 import 路径**

pump.rs 内的 `crate::rfb_connection::{...}` 在 session 内同样指向 `crate::rfb_connection`，无需改动。`crate::rfb_connection` 在 headless 原为 headless 内部模块，现在 session 内部，引用一致。

- [ ] **Step 4: 删除 headless 原目录**

```bash
rm -rf crates/ipkvm-headless/src/rfb_input
```

- [ ] **Step 5: 编译验证**

Run: `cargo check -p ipkvm-session`
Expected: 编译通过。

- [ ] **Step 6: 提交**

```bash
git add crates/ipkvm-session crates/ipkvm-headless
git commit -m "refactor(session): 迁入 rfb_input 输入泵与映射器到 ipkvm-session"
```

---

## Task 3: headless lib 重新导出 + 内部引用改路径

**Files:**
- Modify: `crates/ipkvm-headless/src/lib.rs` — 移除 `pub mod rfb_connection;` `pub mod rfb_input;`，改为 `pub use ipkvm_session::{rfb_connection, rfb_input};`，`HeadlessConfig` 迁入 session 或保留（**保留在 headless，不改现有测试 `headless_config_defaults_to_localhost_and_standard_http_port`**）
- Modify: `crates/ipkvm-headless/src/rfb_tcp/mod.rs` `server.rs` `transport.rs` — `crate::rfb_connection` → `ipkvm_session::rfb_connection`
- Modify: `crates/ipkvm-headless/src/rfb_ws/mod.rs` `service.rs` `transport.rs` — 同上
- Modify: `crates/ipkvm-headless/src/web/service.rs` — `crate::rfb_connection` → `ipkvm_session::rfb_connection`
- Modify: `crates/ipkvm-headless/src/main.rs` — `use ipkvm_headless::rfb_connection` → `use ipkvm_session::rfb_connection`
- Modify: `crates/ipkvm-headless/src/bin/ipkvm-demo.rs` `ipkvm-browser-fixture.rs` — 同上
- Test: 不改（`use ipkvm_headless::rfb_*` 由重新导出满足）

**Interfaces:**
- Consumes: `ipkvm_session::rfb_connection` / `ipkvm_session::rfb_input`（Task 1/2 产出）
- Produces: headless lib 对外 API 与搬迁前一致（重新导出）

- [ ] **Step 1: 修改 headless lib.rs**

```rust
// 移除
pub mod rfb_connection;
pub mod rfb_input;
// 改为
pub use ipkvm_session::rfb_connection;
pub use ipkvm_session::rfb_input;
```

- [ ] **Step 2: 批量替换内部引用**

```bash
# 在 headless src 内（排除已删除的 rfb_connection/rfb_input 目录）
grep -rl "crate::rfb_connection" crates/ipkvm-headless/src/rfb_tcp crates/ipkvm-headless/src/rfb_ws crates/ipkvm-headless/src/web
# 每个文件手动替换为 ipkvm_session::rfb_connection
```

- [ ] **Step 3: 修改 main.rs 与 bin**

```bash
# main.rs: use ipkvm_headless::rfb_connection::... → use ipkvm_session::rfb_connection::...
# bin/ipkvm-demo.rs: 同上
# bin/ipkvm-browser-fixture.rs: 同上
```

- [ ] **Step 4: 编译验证**

Run: `cargo check -p ipkvm-headless --all-features`
Expected: 编译通过。

- [ ] **Step 5: 提交**

```bash
git add crates/ipkvm-headless
git commit -m "refactor(headless): rfb_connection/rfb_input 改用 ipkvm_session 重新导出，保持 API 兼容"
```

---

## Task 4: 全量测试回归（行为不变验证）

**Files:**
- Test: 全部现有测试（不改）

**Interfaces:**
- Consumes: Task 1-3 的重新导出

- [ ] **Step 1: 运行全量测试**

Run: `cargo test --workspace --all-features`
Expected: 全部通过（headless 的 101 测试 + core 的 74 测试 + 其他 crate）。

- [ ] **Step 2: 运行 verify.ps1**

Run: `.\scripts\verify.ps1`
Expected: 通过（含真实浏览器闭环）。

- [ ] **Step 3: 提交（如有测试修复）**

```bash
git add .
git commit -m "test: 会话核心归位后全量回归通过"
```

---

## Task 5: session 新增 devices 设备枚举模块

**Files:**
- Create: `crates/ipkvm-session/src/devices.rs`
- Modify: `crates/ipkvm-session/src/lib.rs`（`pub mod devices;`）
- Modify: `crates/ipkvm-session/Cargo.toml`（serial feature 加 serialport）

**Interfaces:**
- Produces: `ipkvm_session::devices::{list_video_devices() -> Result<Vec<VideoDevice>, Error>, list_serial_devices() -> Result<Vec<SerialDevice>, Error>}`、`VideoDevice { id, display_name }`、`SerialDevice { path, port_type }`

- [ ] **Step 1: 写失败的测试**

```rust
// devices.rs 底部
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_video_devices_returns_list() {
        // mock/无相机下应返回 Ok（可能为空列表），不 panic。
        let devices = list_video_devices().unwrap();
        let _ = devices;
    }

    #[test]
    fn list_serial_devices_returns_list() {
        // 无串口下应返回 Ok（可能为空列表），不 panic。
        let devices = list_serial_devices().unwrap();
        let _ = devices;
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-session devices --features serial`
Expected: FAIL（函数未定义）。

- [ ] **Step 3: 实现**

```rust
//! 设备枚举：视频采集设备与串口设备。

use thiserror::Error;

/// 视频采集设备。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoDevice {
    pub id: String,
    pub display_name: String,
}

/// 串口设备。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SerialDevice {
    pub path: String,
    pub port_type: String,
}

#[derive(Debug, Error)]
pub enum DeviceListError {
    #[error("video device enumeration failed: {0}")]
    Video(String),
    #[error("serial device enumeration failed: {0}")]
    Serial(String),
}

/// 枚举视频采集设备（复用 ipkvm-video 的 camera::list_cameras）。
pub fn list_video_devices() -> Result<Vec<VideoDevice>, DeviceListError> {
    let cameras = ipkvm_video::camera::list_cameras()
        .map_err(|e| DeviceListError::Video(e.to_string()))?;
    Ok(cameras
        .iter()
        .map(|c| VideoDevice {
            id: c.id.clone(),
            display_name: c.display_name.clone(),
        })
        .collect())
}

/// 枚举串口设备（serialport::available_ports）。
///
/// 测试里无条件调用本函数，因此不做 `#[cfg(feature = "serial")]` 门控；
/// 依赖 serialport 为可选依赖时，串口枚举随 feature 联动（无 feature 时
/// 返回空列表，不 panic）。
#[cfg(feature = "serial")]
pub fn list_serial_devices() -> Result<Vec<SerialDevice>, DeviceListError> {
    let ports = serialport::available_ports()
        .map_err(|e| DeviceListError::Serial(e.to_string()))?;
    Ok(ports
        .iter()
        .map(|p| SerialDevice {
            path: p.port_name.clone(),
            port_type: format!("{:?}", p.port_type),
        })
        .collect())
}

// 非 serial feature 下返回空列表（避免编译错误，测试仍通过）。
#[cfg(not(feature = "serial"))]
pub fn list_serial_devices() -> Result<Vec<SerialDevice>, DeviceListError> {
    Ok(Vec::new())
}
```

> **注意**：本模块的 `VideoDevice` / `SerialDevice` 仅描述设备清单；`SessionManagerConfig` 只含 fps/baud（Task 6），真实设备选择字段由 `ConsoleSession` 组装时定义（后续 issue）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-session devices --features serial`
Expected: PASS（2 个测试全过）。

- [ ] **Step 5: 提交**

```bash
git add crates/ipkvm-session
git commit -m "feat(session): 新增设备枚举模块（视频 + 串口）"
```

---

## Task 6: ConsoleSession 真实组装（帧源 + 输入泵 + gate）

> **范围**：`ConsoleSession` 从空壳变为真实会话组装器。它持有帧源（`Arc<dyn FrameSource>`）、输入 sink（`S: InputSink`，headless 用 `Ch9329InputSink<SerialCommandQueue>` 或 `<FakeCommandQueue>`）、`RfbConnectionGate`（仲裁，与传输层共享）和事件出口（`mpsc::Sender<RfbServerEvent>`）。`start()` spawn 输入泵并收集通知；`stop()` 触发释放并停泵。

**Files:**
- Create: `crates/ipkvm-session/src/console_session.rs`
- Modify: `crates/ipkvm-session/src/lib.rs`（`pub mod console_session;`）
- Modify: `crates/ipkvm-session/Cargo.toml`（加 `ipkvm-rfb`、`tokio`、`thiserror`——如 T1 未加）

**Interfaces:**
- Produces:
  - `ConsoleSession<S: InputSink + Send + 'static>`
  - `ConsoleSession::new(frame_source, sink, gate, event_tx) -> Self`
  - `ConsoleSession::start(&mut self) -> Result<SessionHandle, SessionError>`（spawn 输入泵，返回可停止句柄）
  - `ConsoleSession::stop(&mut self) -> Result<(), SessionError>`（触发 release_all 并停泵）
  - `ConsoleSession::gate(&self) -> &RfbConnectionGate`
  - `ConsoleSession::event_tx(&self) -> &mpsc::Sender<RfbServerEvent>`（传输层 clone 使用）
  - `ConsoleSession::stats(&self) -> &SessionStats`
- Consumes: `RfbInputPump`（T2 搬入）、`RfbConnectionGate`（T1 搬入）、`ipkvm_core::InputSink`、`ipkvm_video::FrameSource`

- [ ] **Step 1: 写失败的测试**

```rust
// console_session.rs 底部
#[cfg(test)]
mod tests {
    use super::*;

    // 用记录型 sink 验证：start 后事件能到 pump，stop 后 release_all 被调用。
    // 帧源用 ipkvm-video mock（MockFrameSource），sink 用记录型 TestSink。
    #[test]
    fn start_returns_running_handle_and_stop_releases() {
        let mut session = console_session_fixture();
        let handle = session.start().unwrap();
        assert!(session.stats().is_running());
        session.stop().unwrap();
        assert!(!session.stats().is_running());
        let _ = handle;
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-session console_session --features mock`
Expected: FAIL（模块不存在）。

- [ ] **Step 3: 实现**

```rust
//! 控制台会话组装：把帧源、输入 sink、连接闸门和输入泵组装成可运行的会话。

use ipkvm_core::InputSink;
use ipkvm_video::FrameSource;
use tokio::sync::{mpsc, watch};
use thiserror::Error;

use crate::rfb_connection::{RfbConnectionGate, RfbServerEvent};
use crate::rfb_input::{RfbInputNotice, RfbInputPump, RfbInputRunError};

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session is already running")]
    AlreadyRunning,
    #[error("session is not running")]
    NotRunning,
    #[error("input pump failed: {0}")]
    Input(#[from] RfbInputRunError),
}

/// 会话统计：输入事件计数、最后输入时间、丢帧计数、串口统计。
#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    /// 累计输入事件数（键盘 + 指针）。
    pub input_events: u64,
    /// 最后输入事件的单调时间（纳秒）。
    pub last_input_ns: Option<u64>,
    /// 累计丢帧数（帧 seq 不连续）。
    pub dropped_frames: u64,
    /// 串口批次/帧统计（来自 CommandQueue::stats）。
    pub serial: Option<crate::serial_stats::SerialStats>,
}

/// 运行中会话句柄：可 Clone，调用方持有即可请求停止。
#[derive(Clone, Debug)]
pub struct SessionHandle;

/// 控制台会话：帧源 + 输入 sink + 连接闸门 + 输入泵的组装。
pub struct ConsoleSession<S: InputSink + Send + 'static> {
    frame_source: std::sync::Arc<dyn FrameSource>,
    sink: S,
    gate: RfbConnectionGate,
    event_tx: mpsc::Sender<RfbServerEvent>,
    pump_task: Option<tokio::task::JoinHandle<Result<(), RfbInputRunError>>>,
    stats: SessionStats,
}

impl<S: InputSink + Send + 'static> ConsoleSession<S> {
    pub fn new(
        frame_source: std::sync::Arc<dyn FrameSource>,
        sink: S,
        gate: RfbConnectionGate,
        event_tx: mpsc::Sender<RfbServerEvent>,
    ) -> Self {
        Self {
            frame_source,
            sink,
            gate,
            event_tx,
            pump_task: None,
            stats: SessionStats::default(),
        }
    }

    pub fn gate(&self) -> &RfbConnectionGate {
        &self.gate
    }

    pub fn event_tx(&self) -> &mpsc::Sender<RfbServerEvent> {
        &self.event_tx
    }

    pub fn stats(&self) -> &SessionStats {
        &self.stats
    }

    pub fn is_running(&self) -> bool {
        self.pump_task.is_some()
    }

    pub fn start(&mut self) -> Result<SessionHandle, SessionError> {
        if self.is_running() {
            return Err(SessionError::AlreadyRunning);
        }
        let (event_tx, event_rx) = mpsc::channel(64);
        self.event_tx = event_tx;
        let mut pump = RfbInputPump::new(self.sink.clone());
        let task = tokio::spawn(async move {
            let mut rx = event_rx;
            pump.run(&mut rx, |_notice: &RfbInputNotice| {}).await
        });
        self.pump_task = Some(task);
        Ok(SessionHandle)
    }

    pub fn stop(&mut self) -> Result<(), SessionError> {
        let task = self
            .pump_task
            .take()
            .ok_or(SessionError::NotRunning)?;
        // 关闭事件发送端 → pump 收到事件源关闭 → release_all()
        drop(&self.event_tx);
        task.abort(); // 超时兜底；正常路径 pump 因事件源关闭自然退出
        Ok(())
    }
}
```

> **说明**：`start()` 中 `self.sink` 需 `S: Clone`（pump 持有 sink 副本，调用方保留原 sink 便于统计/释放）；`self.event_tx` 在 `new()` 由调用方传入，`start()` 内重建 channel 使事件流向 pump。`RfbInputPump` 消费 `&mut mpsc::Receiver`（见 pump.rs:245），因此 `event_rx` 在闭包内声明为可变。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-session console_session --features mock`
Expected: PASS（1 个测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/ipkvm-session
git commit -m "feat(session): ConsoleSession 真实组装（帧源+输入泵+gate+事件出口）"
```

---

## Task 7: SessionManager 委托真实会话（不复制状态）

> **范围**：`SessionManager` 不再持有自己的 state 字段，改为持有 `Option<ConsoleSession>`，state 从实际会话推断。start/stop/restart 委托给 ConsoleSession 真实组装。`SessionManagerConfig` 含设备选择字段（video id、serial path、fps、baud）。

**Files:**
- Create: `crates/ipkvm-session/src/session_manager.rs`
- Modify: `crates/ipkvm-session/src/lib.rs`（`pub mod session_manager;`）

**Interfaces:**
- Produces:
  - `SessionManager<S: InputSink + Send + 'static>`
  - `SessionManager::new(config, frame_source, sink, gate) -> Self`（调用方构造帧源与 sink）
  - `SessionManager::state(&self) -> SessionState`（从 `is_running()` 推断）
  - `SessionManager::start(&mut self) -> Result<SessionHandle, SessionError>`（委托 ConsoleSession::start）
  - `SessionManager::stop(&mut self) -> Result<(), SessionError>`
  - `SessionManager::restart(&mut self) -> Result<(), SessionError>`
  - `SessionManager::session(&self) -> Option<&ConsoleSession<S>>`
- Consumes: `ConsoleSession`（T6）、`SessionState`、`SessionError`

- [ ] **Step 1: 写失败的测试**

```rust
// session_manager.rs 底部
#[cfg(test)]
mod tests {
    use super::*;

    // 用记录型 sink + mock 帧源构造 SessionManager，验证委托真实会话。
    #[test]
    fn manager_delegates_start_stop_to_console_session() {
        let mut manager = session_manager_fixture();
        assert_eq!(manager.state(), SessionState::Stopped);
        manager.start().unwrap();
        assert_eq!(manager.state(), SessionState::Running);
        manager.restart().unwrap();
        assert_eq!(manager.state(), SessionState::Running);
        manager.stop().unwrap();
        assert_eq!(manager.state(), SessionState::Stopped);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-session session_manager --features mock`
Expected: FAIL（模块不存在）。

- [ ] **Step 3: 实现**

```rust
//! 会话管理：创建、重启、停止控制台会话的生命周期。
//!
//! 委托 `ConsoleSession` 真实组装，不复制状态——state 从实际会话推断。

use ipkvm_core::InputSink;
use ipkvm_video::FrameSource;
use thiserror::Error;

use crate::console_session::{ConsoleSession, SessionError, SessionHandle};
use crate::rfb_connection::RfbConnectionGate;
use crate::rfb_input::RfbServerEvent;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct SessionManagerConfig {
    pub fps: u32,
    pub baud: u32,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self { fps: 10, baud: 9600 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Stopped,
    Running,
}

/// 会话管理器：委托 ConsoleSession 管理单个会话生命周期。
pub struct SessionManager<S: InputSink + Send + 'static> {
    config: SessionManagerConfig,
    session: Option<ConsoleSession<S>>,
}

impl<S: InputSink + Send + 'static> SessionManager<S> {
    pub fn new(
        config: SessionManagerConfig,
        frame_source: std::sync::Arc<dyn FrameSource>,
        sink: S,
        gate: RfbConnectionGate,
        event_tx: mpsc::Sender<RfbServerEvent>,
    ) -> Self {
        Self {
            config,
            session: Some(ConsoleSession::new(frame_source, sink, gate, event_tx)),
        }
    }

    pub fn state(&self) -> SessionState {
        if self.session.as_ref().is_some_and(|s| s.is_running()) {
            SessionState::Running
        } else {
            SessionState::Stopped
        }
    }

    pub fn session(&self) -> Option<&ConsoleSession<S>> {
        self.session.as_ref()
    }

    pub fn start(&mut self) -> Result<SessionHandle, SessionError> {
        let Some(session) = self.session.as_mut() else {
            return Err(SessionError::NotRunning);
        };
        session.start()
    }

    pub fn stop(&mut self) -> Result<(), SessionError> {
        let Some(session) = self.session.as_mut() else {
            return Err(SessionError::NotRunning);
        };
        session.stop()
    }

    pub fn restart(&mut self) -> Result<(), SessionError> {
        self.stop()?;
        self.start()?;
        Ok(())
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-session session_manager --features mock`
Expected: PASS（1 个测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/ipkvm-session
git commit -m "feat(session): SessionManager 委托真实 ConsoleSession，state 从会话推断"
```

---

## Task 8: 会话状态统计（输入统计、最后输入时间、丢帧、串口统计）

> **范围**：`ConsoleSession` 在输入泵 notice 回调中收集输入统计（键盘/指针计数 + 最后输入时间）；从帧源 `latest_frame().seq` 检测跳跃计数丢帧；串口统计从 sink 的 `CommandQueue::stats()` 读取。`SessionStats` 供后续 `/api/status` 扩展与桌面状态栏消费。

**Files:**
- Modify: `crates/ipkvm-session/src/console_session.rs`（`SessionStats` 字段填充 + notice 回调 + 帧 seq 检测）
- Create: `crates/ipkvm-session/src/serial_stats.rs`（`SerialStats` 类型，从 `CommandQueue::stats` 映射）

**Interfaces:**
- Produces:
  - `SessionStats { input_events, last_input_ns, dropped_frames, serial }`
  - `console_session::SerialStats { batches_accepted, frames_accepted }`
- Consumes: `ipkvm_core::CommandQueue::stats`、`ipkvm_video::FrameSource::latest_frame().seq`

- [ ] **Step 1: 写失败的测试**

```rust
// console_session.rs 底部
#[cfg(test)]
mod tests {
    use super::*;

    // 帧 seq 跳跃（1→3，缺 2）→ dropped_frames 计数 1。
    #[test]
    fn frame_seq_jump_counts_dropped_frame() {
        let mut stats = SessionStats::default();
        stats.observe_frame_seq(1);
        stats.observe_frame_seq(3);
        assert_eq!(stats.dropped_frames, 1);
    }

    // 输入事件计数与最后时间更新。
    #[test]
    fn input_notice_updates_stats() {
        let mut stats = SessionStats::default();
        stats.observe_input();
        stats.observe_input();
        assert_eq!(stats.input_events, 2);
        assert!(stats.last_input_ns.is_some());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-session console_session --features mock`
Expected: FAIL（`observe_frame_seq` / `observe_input` 未定义）。

- [ ] **Step 3: 实现**

```rust
// console_session.rs 内补充 SessionStats 方法：
impl SessionStats {
    /// 记录一次输入事件（键盘/指针）。
    pub fn observe_input(&mut self) {
        self.input_events = self.input_events.saturating_add(1);
        self.last_input_ns = Some(crate::now_ns());
    }

    /// 观察一帧：seq 跳跃即丢帧。
    pub fn observe_frame_seq(&mut self, seq: u64) {
        // 首帧初始化 last_seq；后续 seq 不连续则计数丢帧。
        match self.last_seq {
            Some(last) if seq > last + 1 => {
                self.dropped_frames =
                    self.dropped_frames.saturating_add(seq - last - 1);
            }
            _ => {}
        }
        self.last_seq = Some(seq);
    }
}

// serial_stats.rs:
//! 串口统计：从 `CommandQueue::stats()` 映射。

use ipkvm_core::QueueStats;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SerialStats {
    pub batches_accepted: u64,
    pub frames_accepted: u64,
}

impl From<QueueStats> for SerialStats {
    fn from(stats: QueueStats) -> Self {
        Self {
            batches_accepted: stats.batches_accepted,
            frames_accepted: stats.frames_accepted,
        }
    }
}
```

> **说明**：`ConsoleSession::start` 的 notice 回调改为 `observe` 闭包——键盘/指针 notice 时调用 `stats.observe_input()`；`stop` 后从 sink 的 `CommandQueue::stats()` 取串口统计填入 `SessionStats.serial`。帧 seq 检测在需要时由会话轮询帧源时调用（桌面渲染循环或 headless 快照前）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-session console_session --features mock`
Expected: PASS（3 个测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/ipkvm-session
git commit -m "feat(session): 会话状态统计（输入事件/最后输入/丢帧/串口统计）"
```

---

## Task 9: 全量验证收口（issue #30 关闭前）

**Files:**
- Test: 全部（不改）

- [ ] **Step 1: 全量测试 + fmt + clippy**

```bash
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: 全绿。

- [ ] **Step 2: verify.ps1**

Run: `.\scripts\verify.ps1`
Expected: 通过（含真实浏览器闭环）。

- [ ] **Step 3: 更新设计文档的「阶段状态」与 README**

- `docs/superpowers/specs/2026-08-02-product-apps-wiring-design.md`：把「阶段 1」标记为已实现，注明 issue #30。
- `README.md`：`ipkvm-session` 描述更新为真实会话核心（连接驱动、输入泵、设备枚举、会话管理）；`ipkvm-headless` 描述更新为传输适配 + Web 服务。

- [ ] **Step 4: 提交**

```bash
git add .
git commit -m "docs: session 归位完成，更新设计阶段状态与 README"
```

- [ ] **Step 5: 创建 PR 并关联 #30**

按 `.gitea/PULL_REQUEST_TEMPLATE.md`，`Closes #30`。

---
