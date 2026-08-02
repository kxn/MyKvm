# Issue #40 鼠标相对模式实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 让桌面 app 在 BIOS/启动菜单阶段可用相对鼠标（不依赖目标机分辨率），并保留系统内绝对模式；模式可一键切换。

**Architecture:** 新增桌面专用事件 `RfbServerEvent::PointerRelative`（携带按钮掩码、dx/dy、滚轮步数），经输入泵路由到 `RfbPointerMapper::handle_relative_pointer` 输出 `PointerEvent::RelativeMove`/`Wheel`；桌面端相对模式锁定并隐藏本地光标，用 egui 指针增量换算成帧像素发送；Ctrl+Alt+M 本地切换模式并通过重连应用。RFB 网络客户端仍走绝对事件，协议不变。

**Tech Stack:** Rust workspace；egui/eframe 0.33（`CursorGrab::Locked`、`ViewportCommand::CursorGrab/CursorVisible`、`pointer.delta()`、`Event::MouseWheel`）；tokio mpsc；`tea`（Gitea）。

## Global Constraints

- 仓库文档一律中文；提交信息用英文 conventional commit。
- 围绕 Gitea issue #40；TDD：先失败测试再实现。
- 提交前必须通过：`cargo fmt --all --check`、`cargo test --workspace --all-features`。
- 不改 RFB 线协议；`RfbServerEvent::PointerRelative` 仅供本地桌面控制器产生，传输层不产生。
- 绝对模式现有行为不变。
- 灵敏度默认 1.0，可在高级设置调整（0.1–5.0）。

---

## 文件结构

- `crates/ipkvm-session/src/rfb_connection/mod.rs`：`RfbServerEvent` 新增 `PointerRelative` 变体。
- `crates/ipkvm-session/src/rfb_input/pump.rs`：`RfbInputEventKind::PointerRelative`、路由与 `handle_pointer_relative`。
- `crates/ipkvm-session/src/rfb_input/pointer.rs`：`handle_relative_pointer` + 共用按键事件辅助函数。
- `crates/ipkvm-desktop/src/session.rs`：`send_pointer_relative`。
- `crates/ipkvm-desktop/src/input.rs`：`is_mode_toggle_combo`、`accumulate_delta`、`wheel_steps`。
- `crates/ipkvm-desktop/src/app.rs`：相对模式输入路径、光标锁定/隐藏、Ctrl+Alt+M 切换重连、状态栏模式提示。
- `crates/ipkvm-desktop/src/state.rs`：`AdvancedSettings::relative_sensitivity`。
- `docs/ipkvm-coarse-design.md`：补充相对鼠标小节。

---

### Task 1: 事件变体与泵路由

**Files:**
- Modify: `crates/ipkvm-session/src/rfb_connection/mod.rs`
- Modify: `crates/ipkvm-session/src/rfb_input/pump.rs`

**Interfaces:**
- Produces: `RfbServerEvent::PointerRelative { client_id, button_mask: u8, dx: i16, dy: i16, wheel: i8 }`；`RfbInputEventKind::PointerRelative`；`RfbInputPump::handle_pointer_relative`。

- [x] **Step 1: 写失败测试**（追加到 `pump.rs` 测试模块）

```rust
    #[tokio::test]
    async fn routes_relative_pointer_events_after_connect() {
        let client_id = client(20);
        let peer_addr = peer(5920);
        let mut pump = RfbInputPump::new(RecordingSink::default());
        pump.handle_event(connected(client_id, peer_addr)).unwrap();

        assert_eq!(
            pump.handle_event(RfbServerEvent::PointerRelative {
                client_id,
                button_mask: 1,
                dx: 12,
                dy: -4,
                wheel: 0,
            }),
            Ok(RfbInputNotice::Pointer {
                client_id,
                outcome: RfbPointerOutcome::Applied,
            })
        );
        assert_eq!(
            pump.sink().pointer_batches,
            vec![vec![
                PointerEvent::RelativeMove { dx: 12, dy: -4 },
                PointerEvent::Button {
                    button: PointerButton::Left,
                    down: true,
                },
            ]]
        );
    }

    #[tokio::test]
    async fn relative_pointer_without_active_controller_is_rejected() {
        let client_id = client(21);
        let error = pump
            .handle_event(RfbServerEvent::PointerRelative {
                client_id,
                button_mask: 0,
                dx: 1,
                dy: 0,
                wheel: 0,
            })
            .unwrap_err();
        assert!(matches!(
            error.error(),
            RfbInputError::Lifecycle(RfbInputLifecycleError::NoActiveController {
                event_kind: RfbInputEventKind::PointerRelative,
                ..
            })
        ));
    }
```

（第二个测试需要 `let mut pump = RfbInputPump::new(RecordingSink::default());`。）

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-session rfb_input::pump::tests::routes_relative_pointer`
Expected: 编译失败，`RfbServerEvent::PointerRelative` 不存在。

- [x] **Step 3: 实现**

`mod.rs` 的 `RfbServerEvent` 枚举中 `Pointer` 变体之后追加：

```rust
    PointerRelative {
        client_id: RfbClientId,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    },
```

`pump.rs` 的 `RfbInputEventKind` 追加：

```rust
    PointerRelative,
```

`try_handle_event` 的 `Pointer` 分支之后追加：

```rust
            RfbServerEvent::PointerRelative {
                client_id,
                button_mask,
                dx,
                dy,
                wheel,
            } => self.handle_pointer_relative(*client_id, *button_mask, *dx, *dy, *wheel),
```

`handle_pointer` 之后追加：

```rust
    fn handle_pointer_relative(
        &mut self,
        client_id: RfbClientId,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    ) -> Result<RfbInputNotice, RfbInputError> {
        self.require_active(client_id, RfbInputEventKind::PointerRelative)?;
        match self
            .pointer
            .handle_relative_pointer(&mut self.sink, button_mask, dx, dy, wheel)
        {
            Ok(outcome) => Ok(RfbInputNotice::Pointer { client_id, outcome }),
            Err(RfbPointerError::Input(source)) => Err(RfbInputError::Sink {
                client_id,
                operation: RfbInputOperation::Pointer,
                source,
            }),
        }
    }
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-session rfb_input::pump::tests::`
Expected: 新增 2 个测试 PASS（Task 2 实现前 `handle_relative_pointer` 不存在，会编译失败——因此 Task 1 与 Task 2 合并验证）。

---

### Task 2: 相对指针 mapper

**Files:**
- Modify: `crates/ipkvm-session/src/rfb_input/pointer.rs`

**Interfaces:**
- Produces: `pub fn handle_relative_pointer(&mut self, sink: &mut impl InputSink, button_mask: u8, dx: i16, dy: i16, wheel: i8) -> Result<RfbPointerOutcome, RfbPointerError>`

- [x] **Step 1: 写失败测试**（追加到 `pointer.rs` 测试模块）

```rust
    #[test]
    fn relative_move_emits_delta_and_preserves_buttons() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_relative_pointer(&mut sink, 0x01, 12, -4, 0)
            .unwrap();

        assert_eq!(
            sink.batches,
            vec![vec![
                PointerEvent::RelativeMove { dx: 12, dy: -4 },
                button(PointerButton::Left, true),
            ]]
        );
    }

    #[test]
    fn relative_wheel_emits_wheel_and_keeps_button_state() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();
        mapper
            .handle_relative_pointer(&mut sink, 0x01, 0, 0, 0)
            .unwrap();

        mapper
            .handle_relative_pointer(&mut sink, 0x01, 0, 0, 3)
            .unwrap();

        assert_eq!(sink.batches[1], vec![PointerEvent::Wheel { delta: 3 }]);
    }

    #[test]
    fn relative_zero_delta_skips_move_but_still_sends_buttons() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_relative_pointer(&mut sink, 0x01, 0, 0, 0)
            .unwrap();

        assert_eq!(
            sink.batches[0],
            vec![button(PointerButton::Left, true)]
        );
    }
```

（`pointer.rs` 测试模块已有 `button(PointerButton, bool)` 辅助函数与 `RecordingSink`。）

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-session rfb_input::pointer::tests::relative_`
Expected: 编译失败，`handle_relative_pointer` 不存在。

- [x] **Step 3: 实现**

在 `handle_pointer` 之后追加：

```rust
    pub fn handle_relative_pointer(
        &mut self,
        sink: &mut impl InputSink,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    ) -> Result<RfbPointerOutcome, RfbPointerError> {
        let mut events = Vec::new();
        if dx != 0 || dy != 0 {
            events.push(PointerEvent::RelativeMove { dx, dy });
        }
        events.extend(button_events(self.committed_button_mask, button_mask));
        if wheel != 0 {
            events.push(PointerEvent::Wheel {
                delta: i16::from(wheel),
            });
        }

        sink.handle_pointer_batch(&events)?;
        self.committed_button_mask = button_mask;
        let ignored = button_mask & UNSUPPORTED_BUTTON_MASK;
        if ignored == 0 {
            Ok(RfbPointerOutcome::Applied)
        } else {
            Ok(RfbPointerOutcome::AppliedIgnoringButtons {
                button_mask: ignored,
            })
        }
    }
```

提取共用按键事件辅助（模块级，`handle_pointer` 也改为调用它）：

```rust
fn button_events(committed: u8, new_mask: u8) -> Vec<PointerEvent> {
    let mut events = Vec::new();
    for (mask, button) in PERSISTENT_BUTTONS {
        if committed & mask != 0 && new_mask & mask == 0 {
            events.push(PointerEvent::Button { button, down: false });
        }
    }
    for (mask, button) in PERSISTENT_BUTTONS {
        if committed & mask == 0 && new_mask & mask != 0 {
            events.push(PointerEvent::Button { button, down: true });
        }
    }
    events
}
```

`handle_pointer` 中原来的两个按键循环替换为 `events.extend(button_events(self.committed_button_mask, button_mask));`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-session rfb_input::pointer::tests::` 与 `cargo test -p ipkvm-session rfb_input::pump::tests::`
Expected: 全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-session/src/rfb_connection/mod.rs crates/ipkvm-session/src/rfb_input/pump.rs crates/ipkvm-session/src/rfb_input/pointer.rs
git commit -m "feat: route relative pointer events through the input pump"
```

---

### Task 3: 桌面控制器发送相对指针

**Files:**
- Modify: `crates/ipkvm-desktop/src/session.rs`

**Interfaces:**
- Produces: `pub fn send_pointer_relative(&self, button_mask: u8, dx: i16, dy: i16, wheel: i8) -> Result<(), DesktopSessionError>`

- [x] **Step 1: 写失败测试**（追加到 `session.rs` 测试模块）

```rust
    #[test]
    fn connect_then_relative_pointer_reaches_sink() {
        let (mut controller, sink) = controller_with_sink();
        controller.connect(request()).unwrap();

        controller
            .send_pointer_relative(1, 10, -3, 2)
            .unwrap();

        wait_until(
            || sink.recorded.lock().unwrap().pointer_batches == 1,
            "相对指针事件未到达记录型 sink",
        );
        controller.stop().unwrap();
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop session::tests::connect_then_relative_pointer_reaches_sink`
Expected: 编译失败，`send_pointer_relative` 不存在。

- [x] **Step 3: 实现**（`send_pointer` 之后追加）

```rust
    /// 发送相对指针事件（桌面相对鼠标模式；dx/dy 为帧像素增量）。
    pub fn send_pointer_relative(
        &self,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    ) -> Result<(), DesktopSessionError> {
        self.send_event(RfbServerEvent::PointerRelative {
            client_id: RfbClientId::local_desktop(),
            button_mask,
            dx,
            dy,
            wheel,
        })
    }
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop session::tests::`
Expected: 新增测试 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/session.rs
git commit -m "feat: send relative pointer events from the desktop controller"
```

---

### Task 4: 输入辅助函数（模式切换热键、增量累积、滚轮步数）

**Files:**
- Modify: `crates/ipkvm-desktop/src/input.rs`

**Interfaces:**
- Produces: `pub fn is_mode_toggle_combo(event: &eframe::egui::Event) -> bool`；`pub fn accumulate_delta(remainder: &mut (f32, f32), dx: f32, dy: f32) -> (i16, i16)`；`pub fn wheel_steps(unit: eframe::egui::MouseWheelUnit, delta_y: f32) -> i8`

- [x] **Step 1: 写失败测试**（追加到 `input.rs` 测试模块）

```rust
    #[test]
    fn mode_toggle_combo_requires_ctrl_alt_m_pressed_once() {
        let combo = |key: egui::Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers| {
            is_mode_toggle_combo(&egui::Event::Key {
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
        assert!(combo(egui::Key::M, true, false, ctrl_alt));
        assert!(!combo(egui::Key::M, false, false, ctrl_alt));
        assert!(!combo(egui::Key::M, true, true, ctrl_alt));
        assert!(!combo(egui::Key::M, true, false, egui::Modifiers {
            ctrl: true,
            ..Default::default()
        }));
        assert!(!combo(egui::Key::K, true, false, ctrl_alt));
    }

    #[test]
    fn accumulate_delta_sends_integer_parts_and_keeps_remainder() {
        let mut remainder = (0.0, 0.0);
        assert_eq!(accumulate_delta(&mut remainder, 1.6, 2.4), (1, 2));
        assert_eq!(remainder, (0.6, 0.4));
        assert_eq!(accumulate_delta(&mut remainder, 0.4, 0.6), (1, 1));
        assert_eq!(remainder, (0.0, 0.0));
    }

    #[test]
    fn wheel_steps_converts_lines_and_points() {
        assert_eq!(
            wheel_steps(egui::MouseWheelUnit::Line, 2.0),
            2
        );
        assert_eq!(
            wheel_steps(egui::MouseWheelUnit::Point, -100.0),
            -2
        );
        assert_eq!(wheel_steps(egui::MouseWheelUnit::Page, 1.0), 1);
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop input::tests::mode_toggle_combo_requires`
Expected: 编译失败，函数未定义。

- [x] **Step 3: 实现**（`is_remote_exit_combo` 之后追加）

```rust
/// Ctrl+Alt+M：本地切换绝对/相对鼠标模式（本地拦截，不转发远端）。
pub fn is_mode_toggle_combo(event: &eframe::egui::Event) -> bool {
    matches!(
        event,
        eframe::egui::Event::Key {
            key: eframe::egui::Key::M,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if modifiers.ctrl && modifiers.alt
    )
}

/// 把浮点增量累积到余数并返回可发送的整数增量（避免亚像素漂移）。
pub fn accumulate_delta(remainder: &mut (f32, f32), dx: f32, dy: f32) -> (i16, i16) {
    remainder.0 += dx;
    remainder.1 += dy;
    let ix = remainder
        .0
        .trunc()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    let iy = remainder
        .1
        .trunc()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    remainder.0 -= ix as f32;
    remainder.1 -= iy as f32;
    (ix, iy)
}

/// 把 egui 滚轮增量换算成滚轮步数（Line/Page 直接取整，Point 按 50 点一步）。
pub fn wheel_steps(unit: eframe::egui::MouseWheelUnit, delta_y: f32) -> i8 {
    let steps = match unit {
        eframe::egui::MouseWheelUnit::Line | eframe::egui::MouseWheelUnit::Page => delta_y,
        eframe::egui::MouseWheelUnit::Point => delta_y / 50.0,
    };
    steps.round().clamp(i8::MIN as f32, i8::MAX as f32) as i8
}
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop input::tests::`
Expected: 全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/input.rs
git commit -m "test: cover relative mouse input helpers"
```

---

### Task 5: 桌面相对模式（光标锁定、增量发送、模式切换、状态栏）

**Files:**
- Modify: `crates/ipkvm-desktop/src/app.rs`
- Modify: `crates/ipkvm-desktop/src/state.rs`

**Interfaces:**
- Consumes: `is_mode_toggle_combo`、`accumulate_delta`、`wheel_steps`、`send_pointer_relative`。
- Produces: `DesktopApp::connect_request()`（从当前选择构造请求）；`DesktopApp::toggle_mouse_mode()`。

- [x] **Step 1: 写失败测试**（追加到 `app.rs` 测试模块；`state.rs` 追加默认值测试）

```rust
    #[test]
    fn status_texts_show_relative_mode_hint_when_video_focused() {
        let mut app = DesktopApp::empty();
        app.showing_device_dialog = false;
        app.video_focused = true;
        app.selection.advanced.mouse_mode = MouseMode::Relative;

        let texts = app.status_bar_texts();

        assert_eq!(texts.pointer, "相对模式");
    }
```

`state.rs` 追加：

```rust
    #[test]
    fn advanced_defaults_use_absolute_mouse_and_unity_sensitivity() {
        let advanced = AdvancedSettings::default();
        assert_eq!(advanced.mouse_mode, MouseMode::Absolute);
        assert_eq!(advanced.relative_sensitivity, 1.0);
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop app::tests::status_texts_show_relative_mode_hint`、`cargo test -p ipkvm-desktop state::tests::advanced_defaults_use_absolute_mouse`
Expected: 编译失败，`relative_sensitivity` 字段不存在；状态栏断言失败。

- [x] **Step 3: 实现**

`state.rs` 的 `AdvancedSettings` 追加字段：

```rust
    pub relative_sensitivity: f32,
```

`Default` 初始化追加：

```rust
            relative_sensitivity: 1.0,
```

`app.rs` 的 `DesktopApp` 结构体追加字段：

```rust
    cursor_grabbed: bool,
    relative_remainder: (f32, f32),
```

`empty()` 初始化追加：

```rust
            cursor_grabbed: false,
            relative_remainder: (0.0, 0.0),
```

`stop_session` 与 `sync_control_state` 的复位块追加：

```rust
            self.cursor_grabbed = false;
            self.relative_remainder = (0.0, 0.0);
```

`connect()` 中请求构造提取为：

```rust
    fn connect_request(&self) -> Option<ConnectRequest> {
        Some(ConnectRequest {
            video_device_id: self.selection.selected_video_id.clone()?,
            control_device_id: self.selection.selected_control_id.clone()?,
            baud_rate: self.selection.advanced.baud_rate,
            mouse_mode: self.selection.advanced.mouse_mode,
            preview_fps: self.selection.advanced.preview_fps,
        })
    }
```

`connect()` 改用 `let Some(request) = self.connect_request() else { return; };`。

新增模式切换：

```rust
    fn toggle_mouse_mode(&mut self) {
        self.selection.advanced.mouse_mode = match self.selection.advanced.mouse_mode {
            MouseMode::Absolute => MouseMode::Relative,
            MouseMode::Relative => MouseMode::Absolute,
        };
        let Some(request) = self.connect_request() else {
            return;
        };
        if let Err(error) = self.session.release_all() {
            self.status_message = Some(format!("释放输入失败：{error}"));
        }
        match self.connect(request) {
            Ok(()) => {
                self.status_message = Some(format!("已切换为{}鼠标", mouse_mode_label(self.selection.advanced.mouse_mode)));
            }
            Err(error) => {
                self.status_message = Some(format!("切换鼠标模式失败：{error}"));
            }
        }
    }
```

`handle_input` 的 Ctrl+Alt+K 拦截块之后追加模式切换拦截：

```rust
        if remote_active {
            let toggle_requested = response
                .ctx
                .input(|input| input.events.iter().any(crate::input::is_mode_toggle_combo));
            if toggle_requested {
                self.toggle_mouse_mode();
                return;
            }
        }
```

光标锁定/隐藏（`self.video_focused = remote_active;` 之后）：

```rust
        let relative_mode = self.selection.advanced.mouse_mode == MouseMode::Relative;
        if remote_active && relative_mode {
            if !self.cursor_grabbed {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked));
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                self.cursor_grabbed = true;
            }
        } else if self.cursor_grabbed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            self.cursor_grabbed = false;
        }
```

指针发送分支改为按模式分流（替换现有指针块）：

```rust
        // 指针：相对模式用增量（本地光标已锁定），绝对模式用窗口坐标。
        let mask = pointer_button_mask(response, self.pointer_mask);
        if remote_active && relative_mode {
            // 光标锁定后位置不变，位置增量恒为 0；必须用原始鼠标运动事件
            // （eframe 从 DeviceEvent::MouseMotion 转发，物理像素）。
            let (raw_dx, raw_dy) = response.ctx.input(|input| {
                input.events.iter().fold((0.0f32, 0.0f32), |acc, event| {
                    if let egui::Event::MouseMoved(delta) = event {
                        (acc.0 + delta.x, acc.1 + delta.y)
                    } else {
                        acc
                    }
                })
            });
            let sensitivity = self.selection.advanced.relative_sensitivity;
            let pixels_per_point = response.ctx.pixels_per_point();
            let dx_points = raw_dx / pixels_per_point * sensitivity;
            let dy_points = raw_dy / pixels_per_point * sensitivity;
            let dx = dx_points * (frame.width as f32 / video_rect.width());
            let dy = dy_points * (frame.height as f32 / video_rect.height());
            let (dx, dy) = crate::input::accumulate_delta(&mut self.relative_remainder, dx, dy);
            let wheel = response.ctx.input(|input| {
                let total: i32 = input
                    .events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::MouseWheel { unit, delta, .. } = event {
                            Some(i32::from(crate::input::wheel_steps(*unit, delta.y)))
                        } else {
                            None
                        }
                    })
                    .sum();
                total.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8
            });
            if dx != 0 || dy != 0 || wheel != 0 || mask != self.pointer_mask {
                if let Err(error) = self.session.send_pointer_relative(mask, dx, dy, wheel) {
                    self.status_message = Some(format!("指针发送失败：{error}"));
                }
            }
            self.pointer_mask = mask;
        } else if pointer_active(remote_active, mask, self.pointer_mask)
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
```

`status_bar_texts` 的指针文本改为：

```rust
        let pointer = if self.selection.advanced.mouse_mode == MouseMode::Relative
            && self.video_focused
        {
            "相对模式".to_owned()
        } else {
            self.last_pointer
                .map(|(x, y)| format!("({x}, {y})"))
                .unwrap_or_else(|| "窗口外".into())
        };
```

`advanced_ui` 的鼠标模式 ComboBox 之后追加：

```rust
        ui.horizontal(|ui| {
            ui.label("相对灵敏度");
            ui.add(
                egui::DragValue::new(&mut self.selection.advanced.relative_sensitivity)
                    .range(0.1..=5.0)
                    .speed(0.05),
            );
        });
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop`
Expected: 全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/app.rs crates/ipkvm-desktop/src/state.rs
git commit -m "feat: relative mouse mode with cursor grab and ctrl-alt-m toggle"
```

---

### Task 6: 文档与收口

**Files:**
- Modify: `docs/ipkvm-coarse-design.md`

- [x] **Step 1: 更新设计文档**

在“桌面远程输入模式”小节追加：

```markdown
- 鼠标支持绝对/相对两种模式：相对模式锁定并隐藏本地光标，按帧像素增量发送，
  不依赖目标机分辨率（BIOS/启动菜单用）；Ctrl+Alt+M 切换并通过重连应用；
  灵敏度可在高级设置调整（默认 1.0）。
- 滚轮通过相对事件通道发送，绝对与相对模式均可用。
```

- [x] **Step 2: 提交**

```bash
git add docs/ipkvm-coarse-design.md
git commit -m "docs: document relative mouse mode"
```

- [x] **Step 3: 全量验证**

Run: `cargo fmt --all --check`、`cargo test --workspace --all-features`
Expected: 全部通过。

- [x] **Step 4: 自审**：检查实现与计划是否一致（模式切换需重连、绝对模式行为不变、无协议改动、光标状态在退出/断开时复位）。

- [x] **Step 5: 推送、PR、合并、关闭 issue #40**（按用户既定流程）。

---

## Self-Review

- **Spec 覆盖**：mapper 相对输出（Task 2）、桌面增量换算（Task 5）、光标锁定/隐藏（Task 5）、灵敏度（Task 4/5）、切换入口（Task 5）、滚轮（Task 4/5）、绝对模式不变（Task 5 分流）、测试覆盖（各 Task）全部有对应任务。
- **类型一致性**：`PointerRelative` 字段在 Task 1/2/3/5 一致（`button_mask: u8, dx: i16, dy: i16, wheel: i8`）；`handle_relative_pointer` 签名在 Task 1 与 Task 2 一致；`accumulate_delta`/`wheel_steps` 在 Task 4 与 Task 5 一致。
