# Issue #41 输入延迟实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 降低桌面 app 输入延迟：指针去重/限频降低串口占用，波特率连接时自动扫描（115200 优先），视频新帧触发重绘，并顺手修正窗口跟随视频模式的高 DPI 尺寸换算。

**Architecture:** 三处独立改动：(1) `input.rs` 提供指针去重与限频纯函数，`app.rs` 用“待发指针 + 最小发送间隔（33ms ≈ 30Hz）+ 位置/掩码去重”控制串口流量；(2) `probe.rs` 新增 `detect_baud_rate` 按候选表扫描 GetInfo 应答，`app.rs` 连接时自动选择并写回设置；(3) `refresh_video` 发现新帧时 `ctx.request_repaint()`，`ResizeWindowToVideo` 的窗口尺寸除以 `pixels_per_point`。

**Tech Stack:** Rust workspace；egui/eframe 0.33；serialport；`tea`（Gitea）。

## Global Constraints

- 仓库文档一律中文；提交信息用英文 conventional commit。
- 围绕 Gitea issue #41；TDD：先失败测试再实现。
- 提交前必须通过：`cargo fmt --all --check`、`cargo test --workspace --all-features`。
- 波特率候选顺序固定：115200 → 57600 → 38400 → 19200 → 9600；扫描失败回退用户配置值。
- 指针限频只作用于移动事件，按键状态变化必须立即发送（不延迟点击）。

---

## 文件结构

- `crates/ipkvm-desktop/src/input.rs`：`pointer_changed`、`throttle_elapsed` 纯函数及测试。
- `crates/ipkvm-desktop/src/app.rs`：待发指针与限频、相对模式批量 flush、`refresh_video(ctx)` 重绘、ResizeWindowToVideo 高 DPI 修正、连接时自动扫描波特率、高级设置复选框。
- `crates/ipkvm-desktop/src/probe.rs`：`BAUD_CANDIDATES`、`detect_baud_rate` 及候选顺序测试。
- `crates/ipkvm-desktop/src/state.rs`：`AdvancedSettings::auto_baud`（默认 true）及测试。
- `docs/ipkvm-coarse-design.md`：补充自动波特率与指针限频说明。

---

### Task 1: 指针去重与限频纯函数

**Files:**
- Modify: `crates/ipkvm-desktop/src/input.rs`

**Interfaces:**
- Produces: `pub fn pointer_changed(current: (u8, u16, u16), last: Option<(u8, u16, u16)>) -> bool`；`pub fn throttle_elapsed(now: std::time::Instant, last: Option<std::time::Instant>, interval: std::time::Duration) -> bool`

- [x] **Step 1: 写失败测试**（追加到 `input.rs` 测试模块）

```rust
    #[test]
    fn pointer_changed_detects_position_or_mask_changes() {
        let last = Some((0, 100, 100));
        assert!(!pointer_changed((0, 100, 100), last));
        assert!(pointer_changed((1, 100, 100), last));
        assert!(pointer_changed((0, 101, 100), last));
        assert!(pointer_changed((0, 100, 101), last));
        assert!(pointer_changed((0, 100, 100), None));
    }

    #[test]
    fn throttle_elapsed_requires_interval_to_pass() {
        let start = std::time::Instant::now();
        assert!(!throttle_elapsed(start, None, std::time::Duration::from_millis(33)));
        assert!(throttle_elapsed(
            start + std::time::Duration::from_millis(34),
            Some(start),
            std::time::Duration::from_millis(33)
        ));
        assert!(!throttle_elapsed(
            start + std::time::Duration::from_millis(32),
            Some(start),
            std::time::Duration::from_millis(33)
        ));
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop input::tests::pointer_changed_detects`
Expected: 编译失败，函数未定义。

- [x] **Step 3: 实现**（`wheel_steps` 之后追加）

```rust
/// 指针位置或按钮掩码是否变化（位置未变且掩码未变时不需重发）。
pub fn pointer_changed(current: (u8, u16, u16), last: Option<(u8, u16, u16)>) -> bool {
    last != Some(current)
}

/// 距上次发送是否已超过最小间隔（限频用；从未发送过且有待发数据时立即发送）。
pub fn throttle_elapsed(
    now: std::time::Instant,
    last: Option<std::time::Instant>,
    interval: std::time::Duration,
) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= interval)
}
```

（`is_none_or` 需 Rust 1.82+；若工具链较旧改用 `last.map_or(true, |last| ...)`。）

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop input::tests::`
Expected: 全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/input.rs
git commit -m "test: cover pointer dedupe and throttle helpers"
```

---

### Task 2: 桌面指针限频与重绘、窗口尺寸修正

**Files:**
- Modify: `crates/ipkvm-desktop/src/app.rs`

**Interfaces:**
- Consumes: `pointer_changed`、`throttle_elapsed`。
- Produces: `DesktopApp::refresh_video(&mut self, ctx: &egui::Context)`；`POINTER_MIN_INTERVAL` 常量。

- [x] **Step 1: 写失败测试**（`app.rs` 测试模块）

```rust
    #[test]
    fn desired_window_inner_size_scales_physical_pixels_to_points() {
        let size = desired_window_inner_size(
            FrameSize {
                width: 1920,
                height: 1080,
            },
            48.0,
            2.5,
        );
        assert!((size.x - 768.0).abs() < 0.01);
        assert!((size.y - 480.0).abs() < 0.01);
    }
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop app::tests::desired_window_inner_size_scales_physical_pixels_to_points`
Expected: 编译失败，函数签名不匹配。

- [x] **Step 3: 实现**

新增常量与字段：

```rust
/// 指针最小发送间隔（约 30Hz 限频），按键状态变化不受此限制。
const POINTER_MIN_INTERVAL: Duration = Duration::from_millis(33);
```

`DesktopApp` 追加字段：

```rust
    pending_pointer: Option<(u8, u16, u16)>,
    pending_relative: (i16, i16, i8),
    last_pointer_sent: Option<(u8, u16, u16)>,
    last_pointer_sent_at: Option<Instant>,
```

`empty()` 初始化：

```rust
            pending_pointer: None,
            pending_relative: (0, 0, 0),
            last_pointer_sent: None,
            last_pointer_sent_at: None,
```

`sync_control_state`、`stop_session`、`connect()` 成功分支各追加复位：

```rust
            self.pending_pointer = None;
            self.pending_relative = (0, 0, 0);
            self.last_pointer_sent = None;
            self.last_pointer_sent_at = None;
```

`update_impl` 中 `self.refresh_video();` 改为 `self.refresh_video(ctx);`；`refresh_video` 签名改为：

```rust
    fn refresh_video(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.session.latest_frame() else {
            return;
        };
        let now = Instant::now();
        if self.last_frame_seq != Some(frame.seq) {
            self.last_frame_seq = Some(frame.seq);
            self.last_frame_at = Some(now);
            // eframe 事件驱动重绘：新帧到达必须请求重绘，否则空闲时画面停滞。
            ctx.request_repaint();
        }
        if let Ok(rgba) = bgra_to_rgba(&frame) {
            ...
        }
    }
```

`handle_input` 指针分流：绝对模式分支替换为“待发 + 限频 + 去重”：

```rust
        } else if pointer_active(remote_active, mask, self.pointer_mask)
            && let Some(position) = response.ctx.input(|input| input.pointer.latest_pos())
            && let Some((x, y)) = VideoViewport::map_pointer(position, video_rect, frame)
        {
            self.pending_pointer = Some((mask, x, y));
            let now = Instant::now();
            let mask_changed = self.last_pointer_sent.is_some_and(|(last_mask, _, _)| {
                last_mask != mask
            });
            if mask_changed
                || crate::input::throttle_elapsed(
                    now,
                    self.last_pointer_sent_at,
                    POINTER_MIN_INTERVAL,
                )
            {
                if let Some((send_mask, send_x, send_y)) = self.pending_pointer
                    && crate::input::pointer_changed(
                        (send_mask, send_x, send_y),
                        self.last_pointer_sent,
                    )
                {
                    if let Err(error) = self.session.send_pointer(send_mask, send_x, send_y, frame)
                    {
                        self.status_message = Some(format!("指针发送失败：{error}"));
                    }
                    self.last_pointer_sent = Some((send_mask, send_x, send_y));
                    self.last_pointer_sent_at = Some(now);
                }
                self.pending_pointer = None;
            }
            self.last_pointer = Some((x, y));
            self.pointer_mask = mask;
        }
```

相对模式分支改为“累积 + 限频 flush”：

```rust
            let (dx, dy) = crate::input::accumulate_delta(&mut self.relative_remainder, dx, dy);
            let wheel = self.wheel_steps_from_events(response);
            self.pending_relative.0 = self.pending_relative.0.saturating_add(dx);
            self.pending_relative.1 = self.pending_relative.1.saturating_add(dy);
            self.pending_relative.2 = self.pending_relative.2.saturating_add(wheel);
            let now = Instant::now();
            let mask_changed = self.last_pointer_sent.is_some_and(|(last_mask, _, _)| {
                last_mask != mask
            });
            let (pending_dx, pending_dy, pending_wheel) = self.pending_relative;
            if mask_changed
                || crate::input::throttle_elapsed(
                    now,
                    self.last_pointer_sent_at,
                    POINTER_MIN_INTERVAL,
                )
            {
                if pending_dx != 0 || pending_dy != 0 || pending_wheel != 0 || mask_changed {
                    if let Err(error) =
                        self.session
                            .send_pointer_relative(mask, pending_dx, pending_dy, pending_wheel)
                    {
                        self.status_message = Some(format!("指针发送失败：{error}"));
                    }
                    self.last_pointer_sent = Some((mask, u16::MAX, u16::MAX));
                    self.last_pointer_sent_at = Some(now);
                }
                self.pending_relative = (0, 0, 0);
            }
            self.pointer_mask = mask;
```

（相对模式用 `(mask, u16::MAX, u16::MAX)` 占位去重，仅用于掩码变化检测。）

`desired_window_inner_size` 增加 `pixels_per_point: f32` 参数并改为：

```rust
fn desired_window_inner_size(frame: FrameSize, chrome: f32, pixels_per_point: f32) -> egui::Vec2 {
    let ppp = pixels_per_point.max(1.0);
    egui::vec2(
        frame.width as f32 / ppp,
        frame.height as f32 / ppp + chrome,
    )
}
```

调用点改为：

```rust
                let size = desired_window_inner_size(actual, FOLLOW_CHROME, ctx.pixels_per_point());
```

现有 `desired_window_inner_size_adds_chrome` 测试改为传 `1.0`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop`
Expected: 全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/app.rs crates/ipkvm-desktop/src/input.rs
git commit -m "perf: throttle and dedupe pointer sends with repaint on new frames"
```

---

### Task 3: 波特率自动扫描

**Files:**
- Modify: `crates/ipkvm-desktop/src/probe.rs`
- Modify: `crates/ipkvm-desktop/src/state.rs`
- Modify: `crates/ipkvm-desktop/src/app.rs`

**Interfaces:**
- Produces: `pub const BAUD_CANDIDATES: [u32; 5]`；`pub fn detect_baud_rate(path: &str, timeout: Duration) -> Option<u32>`；`AdvancedSettings::auto_baud: bool`。

- [x] **Step 1: 写失败测试**

`probe.rs` 测试模块追加：

```rust
    #[test]
    fn baud_candidates_prefer_115200_and_fall_back_to_9600() {
        assert_eq!(BAUD_CANDIDATES[0], 115200);
        assert_eq!(BAUD_CANDIDATES[4], 9600);
        assert_eq!(BAUD_CANDIDATES.len(), 5);
    }
```

`state.rs` 测试模块的默认值测试追加：

```rust
        assert!(advanced.auto_baud);
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop probe::tests::baud_candidates_prefer`、`cargo test -p ipkvm-desktop state::tests::advanced_defaults_use_absolute_mouse`
Expected: 编译失败，`BAUD_CANDIDATES`/`auto_baud` 不存在。

- [x] **Step 3: 实现**

`probe.rs` 顶部追加：

```rust
/// 自动扫描候选波特率（成品线电平未知：115200 可用则最快，失败逐档降级）。
pub const BAUD_CANDIDATES: [u32; 5] = [115200, 57600, 38400, 19200, 9600];
```

`probe_ch9329` 之后追加：

```rust
/// 扫描候选波特率，返回第一个 GetInfo 有合法应答的档位。
pub fn detect_baud_rate(path: &str, timeout: Duration) -> Option<u32> {
    BAUD_CANDIDATES.into_iter().find(|baud| {
        matches!(
            probe_ch9329(path, *baud, timeout),
            ControlProbeStatus::Ready(_)
        )
    })
}
```

`state.rs` 的 `AdvancedSettings` 追加：

```rust
    pub auto_baud: bool,
```

`Default` 追加：

```rust
            auto_baud: true,
```

`app.rs` 的 `connect()` 在 `connect_request()` 之后、`session.connect` 之前追加：

```rust
        if self.selection.advanced.auto_baud
            && let Some(control_id) = self.selection.selected_control_id.clone()
            && let Some(baud) = crate::probe::detect_baud_rate(&control_id, PROBE_TIMEOUT)
        {
            self.selection.advanced.baud_rate = baud;
            self.status_message = Some(format!("已自动选择波特率 {baud}"));
        }
```

`advanced_ui` 波特率行之后追加：

```rust
        ui.checkbox(&mut self.selection.advanced.auto_baud, "连接时自动检测波特率");
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop`
Expected: 全部 PASS。

- [x] **Step 5: 提交**

```bash
git add crates/ipkvm-desktop/src/probe.rs crates/ipkvm-desktop/src/state.rs crates/ipkvm-desktop/src/app.rs
git commit -m "feat: auto-detect CH9329 baud rate on connect"
```

---

### Task 4: 文档与收口

**Files:**
- Modify: `docs/ipkvm-coarse-design.md`

- [x] **Step 1: 更新设计文档**（“默认决策”与“桌面远程输入模式”相关处追加）

```markdown
- 串口波特率连接时自动扫描（115200→57600→38400→19200→9600，GetInfo 应答即用），
  可关闭；扫描失败回退手动配置值。
- 桌面指针发送限频约 30Hz 并做位置/掩码去重；按键状态变化立即发送。
- 视频新帧到达时请求重绘，空闲无输入时画面保持刷新。
```

- [x] **Step 2: 提交**

```bash
git add docs/ipkvm-coarse-design.md
git commit -m "docs: document baud auto-detect and pointer throttling"
```

- [x] **Step 3: 全量验证**

Run: `cargo fmt --all --check`、`cargo test --workspace --all-features`
Expected: 全部通过。

- [x] **Step 4: 自审**：指针按键不延迟、限频不影响点击、自动扫描失败回退、重绘不空转（新帧才请求）。

- [x] **Step 5: 推送、PR、合并、关闭 issue #41**（按用户既定流程）。

---

## Self-Review

- **Spec 覆盖**：指针去重/限频（Task 1/2）、波特率自动扫描（Task 3）、重绘（Task 2）、高 DPI 窗口尺寸（Task 2）、测试（各 Task）全部有对应任务。
- **类型一致性**：`desired_window_inner_size` 三参签名在 Task 2 测试与实现一致；`detect_baud_rate` 返回 `Option<u32>`；`pointer_changed`/`throttle_elapsed` 在 Task 1 与 Task 2 引用一致。
