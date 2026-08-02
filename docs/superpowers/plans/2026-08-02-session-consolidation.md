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
- `crates/ipkvm-session/src/session_manager.rs` — 新增：会话创建/重启/停止
- `crates/ipkvm-session/Cargo.toml` — 新增 ipkvm-rfb、tokio、serialport（serial feature）

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

## Task 6: session 新增 SessionManager 会话生命周期管理

> **范围说明**：本任务只建 `SessionManager` 的状态机与生命周期骨架，**不组装真实帧源/串口/输入泵**（那属于后续 issue 的 `ConsoleSession` 真实组装）。SessionManager 面向 `/api/session`（headless）与桌面设备切换（desktop）共用，本期只交付可测试的状态迁移。`SessionHandle` 骨架可 Clone。

**Files:**
- Create: `crates/ipkvm-session/src/session_manager.rs`
- Modify: `crates/ipkvm-session/src/lib.rs`（`pub mod session_manager;`）

**Interfaces:**
- Produces: `ipkvm_session::session_manager::{SessionManager, SessionManagerConfig, SessionState, SessionHandle, SessionError}`
  - `SessionManager::new(config) -> Self`
  - `SessionManager::state(&self) -> SessionState`
  - `SessionManager::start(&mut self) -> Result<SessionHandle, SessionError>`
  - `SessionManager::stop(&mut self) -> Result<(), SessionError>`
  - `SessionManager::restart(&mut self) -> Result<(), SessionError>`
  - `SessionState::{Stopped, Running}`；`SessionHandle` 可 Clone；`SessionError::{AlreadyRunning, NotRunning}`

- [ ] **Step 1: 写失败的测试**

```rust
// session_manager.rs 底部
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_transitions_stopped_running_stopped() {
        let mut manager = SessionManager::new(SessionManagerConfig::default());
        assert_eq!(manager.state(), SessionState::Stopped);

        let handle = manager.start().unwrap();
        assert_eq!(manager.state(), SessionState::Running);
        // handle 可 Clone，供调用方共享
        let _handle2 = handle.clone();

        manager.stop().unwrap();
        assert_eq!(manager.state(), SessionState::Stopped);
    }

    #[test]
    fn restart_rebuilds_from_running() {
        let mut manager = SessionManager::new(SessionManagerConfig::default());
        manager.start().unwrap();
        manager.restart().unwrap();
        assert_eq!(manager.state(), SessionState::Running);
    }

    #[test]
    fn start_when_running_is_rejected() {
        let mut manager = SessionManager::new(SessionManagerConfig::default());
        manager.start().unwrap();
        assert_eq!(manager.start(), Err(SessionError::AlreadyRunning));
    }

    #[test]
    fn stop_when_stopped_is_rejected() {
        let mut manager = SessionManager::new(SessionManagerConfig::default());
        assert_eq!(manager.stop(), Err(SessionError::NotRunning));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-session session_manager`
Expected: FAIL（模块不存在，编译错误）。

- [ ] **Step 3: 实现（状态机骨架，不组装真实帧源/串口）**

```rust
//! 会话管理：创建、重启、停止控制台会话的生命周期。
//!
//! 本模块只负责会话生命周期状态机，不直接组装帧源/串口/输入泵；
//! 真实组装由 `ConsoleSession` 完成（后续 issue）。面向 /api/session
//! （headless）与桌面设备切换（desktop）共用同一状态模型。

use thiserror::Error;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum SessionError {
    #[error("session already running")]
    AlreadyRunning,
    #[error("session not running")]
    NotRunning,
}

/// 运行中会话的句柄：可 Clone，调用方持有即可停止。
#[derive(Clone, Debug)]
pub struct SessionHandle {
    state: SessionState,
}

impl SessionHandle {
    fn running() -> Self {
        Self { state: SessionState::Running }
    }
}

/// 会话管理器：管理单个控制台会话的生命周期状态机。
#[derive(Debug, Default)]
pub struct SessionManager {
    state: SessionState,
}

impl SessionManager {
    pub fn new(_config: SessionManagerConfig) -> Self {
        Self { state: SessionState::Stopped }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn start(&mut self) -> Result<SessionHandle, SessionError> {
        if self.state == SessionState::Running {
            return Err(SessionError::AlreadyRunning);
        }
        // TODO(#30): 真实组装帧源+串口+输入泵（ConsoleSession 组装，后续 issue）
        self.state = SessionState::Running;
        Ok(SessionHandle::running())
    }

    pub fn stop(&mut self) -> Result<(), SessionError> {
        if self.state == SessionState::Stopped {
            return Err(SessionError::NotRunning);
        }
        // TODO(#30): 真实释放帧源/串口/输入泵
        self.state = SessionState::Stopped;
        Ok(())
    }

    pub fn restart(&mut self) -> Result<(), SessionError> {
        self.stop()?;
        self.start()?;
        Ok(())
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-session session_manager`
Expected: PASS（4 个测试全过）。

- [ ] **Step 5: 提交**

```bash
git add crates/ipkvm-session
git commit -m "feat(session): 新增 SessionManager 会话生命周期状态机（骨架）"
```

---

## Task 7: 全量验证收口（issue #30 关闭前）

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
