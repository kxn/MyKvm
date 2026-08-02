# Issue #39 键盘方向键/组合错乱实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 修复桌面 app 方向键被 egui 焦点导航截胡、失焦误触发全量释放、事件通道满时丢事件导致的组合键错乱与粘键。

**Architecture:** 三层根因各归其位：(1) 在 `ipkvm-desktop/src/input.rs` 提供 egui 焦点锁过滤器与 Ctrl+Alt+K 退出组合键判定；(2) 在 `ipkvm-desktop/src/session.rs` 给桌面控制器加“待发送队列 + 通道满时保留重试”的无损事件提交，不动共享的 64 容量会话通道（避免波及 RFB 传输层背压语义）；(3) 在 `ipkvm-desktop/src/app.rs` 把远程输入模式定义为“视频面板聚焦 && 窗口前台”，锁定焦点导航、拦截退出热键、失焦即释放并复位。

**Tech Stack:** Rust workspace；egui/eframe 0.33（`Memory::set_focus_lock_filter`、`EventFilter`、`InputState::focused`）；tokio mpsc；`tea` 客户端（Gitea）。

## Global Constraints

- 仓库文档一律中文；代码标识符/协议字段保留原文。
- 非平凡改动围绕 Gitea issue #39；提交信息用英文 conventional commit。
- TDD：每个任务先写失败测试，确认失败后再实现。
- 提交前必须通过：`cargo fmt --all --check`、`cargo test --workspace --all-features`。
- Windows PowerShell 写中文到 Gitea 前设置 UTF-8 编码；写入后读回确认。
- 不改动 `ConsoleSession`/传输层共享事件通道的有界语义（保持 `mpsc::channel(64)`）。
- 不改动键盘 HID 映射（keymap.rs 已确认与 kvm-serial 一致）。

---

## 文件结构

- `crates/ipkvm-desktop/src/input.rs`：新增 `remote_focus_filter()`、`is_remote_exit_combo()` 两个纯函数及测试。
- `crates/ipkvm-desktop/src/session.rs`：新增 `pending_events` 字段、`flush_pending_events()` 函数；`send_event`/`release_all` 改为无损提交；`rollback`/`stop` 清空待发送队列；补纯函数测试。
- `crates/ipkvm-desktop/src/app.rs`：`handle_input` 改为远程模式门控（聚焦 + 窗口前台）、焦点锁、Ctrl+Alt+K 拦截、发送错误可见；状态栏提示“远程输入中 · Ctrl+Alt+K 退出”。
- `docs/ipkvm-coarse-design.md`：补充“桌面远程输入模式”小节。

---

### Task 1: 输入辅助函数（焦点锁过滤器 + Ctrl+Alt+K 判定）

**Files:**
- Modify: `crates/ipkvm-desktop/src/input.rs`
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub fn remote_focus_filter() -> eframe::egui::EventFilter`；`pub fn is_remote_exit_combo(event: &eframe::egui::Event) -> bool`

- [x] **Step 1: 写失败测试**（追加到 `input.rs` 测试模块末尾）

```rust
    #[test]
    fn remote_focus_filter_keeps_navigation_keys_in_remote_mode() {
        let filter = remote_focus_filter();
        for key in [
            egui::Key::Tab,
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::Escape,
        ] {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            assert!(
                filter.matches(&event),
                "navigation key {key:?} must stay in remote mode"
            );
        }
    }

    #[test]
    fn remote_exit_combo_requires_ctrl_alt_k_pressed_once() {
        let combo = |key: egui::Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers| {
            is_remote_exit_combo(&egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat,
                modifiers,
            })
        };
        let ctrl_alt = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert!(combo(egui::Key::K, true, false, ctrl_alt));
        assert!(!combo(egui::Key::K, false, false, ctrl_alt));
        assert!(!combo(egui::Key::K, true, true, ctrl_alt));
        assert!(!combo(
            egui::Key::K,
            true,
            false,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
        assert!(!combo(
            egui::Key::K,
            true,
            false,
            egui::Modifiers {
                alt: true,
                ..Default::default()
            }
        ));
        assert!(!combo(egui::Key::A, true, false, ctrl_alt));
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop input::tests::remote_`
Expected: 编译失败，`remote_focus_filter` / `is_remote_exit_combo` 未定义。

- [x] **Step 3: 实现**（追加到 `modifier_diff` 之后、测试模块之前）

```rust
/// 远程输入模式下的 egui 焦点锁：Tab/方向键/Esc 都留在视频面板，
/// 不参与 egui 焦点导航（防止方向键把焦点移到菜单栏导致输入中断）。
pub fn remote_focus_filter() -> eframe::egui::EventFilter {
    eframe::egui::EventFilter {
        tab: true,
        horizontal_arrows: true,
        vertical_arrows: true,
        escape: true,
    }
}

/// Ctrl+Alt+K：本地退出远程输入模式的组合键（本地拦截，不转发远端）。
pub fn is_remote_exit_combo(event: &eframe::egui::Event) -> bool {
    matches!(
        event,
        eframe::egui::Event::Key {
            key: eframe::egui::Key::K,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if modifiers.ctrl && modifiers.alt
    )
}
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop input::tests::remote_`
Expected: 两个测试 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/input.rs
git commit -m "test: cover remote focus filter and exit combo"
```

---

### Task 2: 桌面控制器无损事件提交（待发送队列）

**Files:**
- Modify: `crates/ipkvm-desktop/src/session.rs`
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `RfbServerEvent`（Clone），`DesktopSessionError`。
- Produces: `fn flush_pending_events(pending: &mut VecDeque<RfbServerEvent>, try_send: impl FnMut(RfbServerEvent) -> Result<(), TrySendError<RfbServerEvent>>) -> Result<(), DesktopSessionError>`；`DesktopSessionController::send_event` 改为无损提交。

- [x] **Step 1: 写失败测试**（追加到 `session.rs` 测试模块，测试内 `use tokio::sync::mpsc::error::TrySendError;`）

```rust
    fn key_event(tag: u8) -> RfbServerEvent {
        RfbServerEvent::Key {
            client_id: RfbClientId::local_desktop(),
            down: true,
            keysym: u32::from(tag),
        }
    }

    fn keysym_of(event: &RfbServerEvent) -> u32 {
        match event {
            RfbServerEvent::Key { keysym, .. } => *keysym,
            _ => 0,
        }
    }

    #[test]
    fn flush_pending_events_drains_in_fifo_order() {
        let mut pending = std::collections::VecDeque::new();
        let mut delivered = Vec::new();
        pending.push_back(key_event(1));
        pending.push_back(key_event(2));

        let result = flush_pending_events(&mut pending, |next| {
            delivered.push(next);
            Ok(())
        });

        assert!(result.is_ok());
        assert!(pending.is_empty());
        assert_eq!(
            delivered.iter().map(keysym_of).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn flush_pending_events_keeps_remainder_when_full_then_resumes_in_order() {
        let mut pending = std::collections::VecDeque::new();
        let mut delivered = Vec::new();
        pending.push_back(key_event(1));
        pending.push_back(key_event(2));
        pending.push_back(key_event(3));

        let mut accepts = 0;
        let result = flush_pending_events(&mut pending, |next| {
            if accepts < 2 {
                accepts += 1;
                delivered.push(next);
                Ok(())
            } else {
                Err(TrySendError::Full(next))
            }
        });

        assert!(result.is_ok());
        assert_eq!(pending.len(), 1);
        assert_eq!(delivered.iter().map(keysym_of).collect::<Vec<_>>(), vec![1, 2]);

        let result = flush_pending_events(&mut pending, |next| {
            delivered.push(next);
            Ok(())
        });

        assert!(result.is_ok());
        assert!(pending.is_empty());
        assert_eq!(
            delivered.iter().map(keysym_of).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn flush_pending_events_clears_on_closed_channel() {
        let mut pending = std::collections::VecDeque::new();
        pending.push_back(key_event(1));

        let result = flush_pending_events(&mut pending, |next| Err(TrySendError::Closed(next)));

        assert!(result.is_err());
        assert!(pending.is_empty());
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop session::tests::flush_pending_`
Expected: 编译失败，`flush_pending_events` 未定义。

- [x] **Step 3: 实现**

顶部导入追加：

```rust
use std::collections::VecDeque;
```

`DesktopSessionController` 结构体追加字段：

```rust
    /// 未送出的事件（通道满时暂存），保证桌面输入不丢事件。
    pending_events: std::sync::Mutex<VecDeque<RfbServerEvent>>,
```

`with_factory` 初始化追加：

```rust
            pending_events: std::sync::Mutex::new(VecDeque::new()),
```

`impl` 块内（`release_all` 之前）新增：

```rust
    /// 尽力把暂存队列按 FIFO 顺序送入事件通道；通道满时保留剩余事件等待下次
    /// 提交，通道关闭时清空并返回错误。返回 Ok 不保证队列已清空（Full 是
    /// 正常暂存而非失败）。
    fn flush_pending_events(
        pending: &mut VecDeque<RfbServerEvent>,
        mut try_send: impl FnMut(
            RfbServerEvent,
        )
            -> Result<(), tokio::sync::mpsc::error::TrySendError<RfbServerEvent>>,
    ) -> Result<(), DesktopSessionError> {
        while let Some(next) = pending.front().cloned() {
            match try_send(next) {
                Ok(()) => {
                    pending.pop_front();
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => break,
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    pending.clear();
                    return Err(DesktopSessionError::Input("event channel closed".into()));
                }
            }
        }
        Ok(())
    }
```

`send_event` 替换为：

```rust
    fn send_event(&self, event: RfbServerEvent) -> Result<(), DesktopSessionError> {
        let mut pending = self
            .pending_events
            .lock()
            .map_err(|_| DesktopSessionError::Input("pending event queue poisoned".into()))?;
        pending.push_back(event);
        let Some(tx) = &self.event_tx else {
            pending.clear();
            return Err(DesktopSessionError::NoEventSender);
        };
        Self::flush_pending_events(&mut pending, |next| tx.try_send(next))
    }
```

`release_all` 替换为：

```rust
    pub fn release_all(&self) -> Result<(), DesktopSessionError> {
        self.send_event(RfbServerEvent::Disconnected {
            client_id: RfbClientId::local_desktop(),
            peer_addr: LOCAL_PEER,
            reason: RfbDisconnectReason::ClientClosed,
        })?;
        self.send_event(RfbServerEvent::Connected {
            client_id: RfbClientId::local_desktop(),
            peer_addr: LOCAL_PEER,
            shared: true,
        })
    }
```

`rollback` 与 `stop` 各追加一行清空待发送队列：

```rust
        self.pending_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop`
Expected: 新增 3 个测试及原有 session 测试全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/session.rs
git commit -m "fix: make desktop event submission lossless with pending retry queue"
```

---

### Task 3: 桌面远程输入模式（焦点锁 + Ctrl+Alt+K + 窗口失焦 + 错误可见）

**Files:**
- Modify: `crates/ipkvm-desktop/src/app.rs`
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::input::remote_focus_filter()`、`crate::input::is_remote_exit_combo()`、`crate::input::KeyAction`。
- Produces: 远程模式门控逻辑与状态栏提示文案。

- [x] **Step 1: 写失败测试**（追加到 `app.rs` 测试模块）

```rust
    #[test]
    fn status_texts_show_remote_input_hint_when_video_focused() {
        let mut app = DesktopApp::empty();
        app.showing_device_dialog = false;
        app.video_focused = true;

        let texts = app.status_bar_texts();

        assert_eq!(texts.keyboard, "远程输入中 · Ctrl+Alt+K 退出");
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop app::tests::status_texts_show_remote_input_hint_when_video_focused`
Expected: FAIL，实际为 `"聚焦可输入"`。

- [x] **Step 3: 实现**

`status_bar_texts` 中键盘分支改为：

```rust
        let keyboard = if self.paste_busy {
            "粘贴中".to_owned()
        } else if self.video_focused {
            "远程输入中 · Ctrl+Alt+K 退出".to_owned()
        } else {
            "失焦".to_owned()
        };
```

`handle_input` 整体替换为：

```rust
    fn handle_input(
        &mut self,
        response: &egui::Response,
        video_rect: egui::Rect,
        frame: FrameSize,
    ) {
        if !self.session.is_control_online() {
            self.pointer_mask = 0;
            self.last_pointer = None;
            return;
        }
        // 远程输入模式 = 视频面板持有 egui 焦点 且 窗口处于前台。
        // 窗口失焦（Alt+Tab 切走）也视为退出远程模式，防止远端粘键。
        let focused = response.has_focus();
        let window_focused = response.ctx.input(|input| input.focused);
        let remote_active = focused && window_focused;

        // Ctrl+Alt+K：本地退出热键。先于指针/键盘转发处理，拦截后不发送远端。
        if remote_active {
            let exit_requested = response
                .ctx
                .input(|input| input.events.iter().any(crate::input::is_remote_exit_combo));
            if exit_requested {
                let _ = self.session.release_all();
                response
                    .ctx
                    .memory_mut(|memory| memory.surrender_focus(response.id));
                self.video_focused = false;
                self.pointer_mask = 0;
                self.last_pointer = None;
                self.last_modifiers = response.ctx.input(|input| input.modifiers);
                return;
            }
        }

        if remote_active {
            // 锁住焦点导航：Tab/方向键/Esc 都转发远端，不让 egui 拿去移动焦点。
            response.ctx.memory_mut(|memory| {
                memory.set_focus_lock_filter(response.id, crate::input::remote_focus_filter());
            });
        }

        if remote_active && !self.video_focused {
            // 刚获得焦点：以当前修饰键为基线，避免把历史按住状态当新按下。
            self.last_modifiers = response.ctx.input(|input| input.modifiers);
        }
        if !remote_active && self.video_focused {
            // 退出远程模式（点击本地 UI / 窗口失焦 / Ctrl+Alt+K）：
            // 释放所有按键并复位本地状态。
            let _ = self.session.release_all();
            self.pointer_mask = 0;
            self.last_pointer = None;
        }
        self.video_focused = remote_active;

        // 指针：悬停或拖动中发送坐标；点击视频区会先获得焦点。
        let mask = pointer_button_mask(response, self.pointer_mask);
        if pointer_active(remote_active, mask, self.pointer_mask)
            && let Some(position) = response.ctx.input(|input| input.pointer.latest_pos())
            && let Some((x, y)) = VideoViewport::map_pointer(position, video_rect, frame)
        {
            if let Err(error) = self.session.send_pointer(mask, x, y, frame) {
                self.status_message = Some(format!("指针发送失败：{error}"));
            }
            self.last_pointer = Some((x, y));
            self.pointer_mask = mask;
        }
        if mask == 0 {
            self.pointer_mask = 0;
        }

        // 键盘：仅远程模式时发送。
        if remote_active {
            let modifiers = response.ctx.input(|input| input.modifiers);
            for action in modifier_diff(self.last_modifiers, modifiers) {
                self.send_key_action(action);
            }
            self.last_modifiers = modifiers;

            let events = response.ctx.input(|input| input.events.clone());
            for event in events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = event
                {
                    if repeat {
                        continue;
                    }
                    match egui_key_to_keysym(key, modifiers) {
                        Some(keysym) => {
                            if let Err(error) = self.session.send_key(pressed, keysym) {
                                self.status_message = Some(format!("键盘发送失败：{error}"));
                            }
                        }
                        None => self.status_message = Some("不支持的按键".into()),
                    }
                }
            }
        }
    }
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop`
Expected: 新增测试及全部桌面测试 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/app.rs
git commit -m "feat: lock remote input focus with ctrl-alt-k exit and window blur release"
```

---

### Task 4: 文档与 issue 收口

**Files:**
- Modify: `docs/ipkvm-coarse-design.md`（若无桌面小节则在文档末尾追加）

- [x] **Step 1: 更新设计文档**

在 `docs/ipkvm-coarse-design.md` 追加：

```markdown
## 桌面远程输入模式

- 视频面板聚焦且窗口在前台时进入远程输入模式：Tab/方向键/Esc 全部转发远端，
  不参与 egui 焦点导航（`Memory::set_focus_lock_filter`）。
- 退出远程模式：点击视频外本地 UI、按 Ctrl+Alt+K（本地拦截，不转发远端）、
  或窗口失焦（Alt+Tab 切走）。退出时执行 release_all 并复位本地输入状态。
- 桌面控制器事件提交无损：事件通道满时暂存待发送队列，通道恢复后按 FIFO
  补发，不丢按键 down/up；发送失败在状态栏可见。
```

- [x] **Step 2: 提交**

```bash
git add docs/ipkvm-coarse-design.md
git commit -m "docs: document desktop remote input mode and lossless event submission"
```

- [x] **Step 3: 全量验证**

Run: `cargo fmt --all --check`、`cargo test --workspace --all-features`
Expected: 全部通过。

- [x] **Step 4: 在 issue #39 评论实施记录与人工验证清单**

```powershell
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
tea issues comment 39 --repo kxn/my_ipkvm --message "已实现：焦点锁 + Ctrl+Alt+K + 窗口失焦释放 + 无损事件队列。待硬件验证：方向键连打、Shift/方向键组合、Ctrl+Alt+K 退出、Alt+Tab 往返、快速打字无粘键。"
```

Expected: 评论创建成功，读回确认中文无乱码。

---

## Self-Review

- **Spec 覆盖**：方向键截胡（Task 3 焦点锁）、失焦误释放（Task 3 远程模式门控）、事件通道丢事件（Task 2 无损队列）、Ctrl+Alt+K（Task 1+3）、状态栏提示（Task 3）、错误可见（Task 3）全部有对应任务。
- **占位符扫描**：所有代码步骤均给出完整可编译代码；命令给出预期输出。
- **类型一致性**：`flush_pending_events` 签名在 Task 2 测试与实现一致；`remote_focus_filter`/`is_remote_exit_combo` 在 Task 1 与 Task 3 引用一致；`DesktopSessionError::Input`/`NoEventSender` 复用现有变体。
