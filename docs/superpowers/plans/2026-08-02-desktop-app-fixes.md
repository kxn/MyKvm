# 桌面 app 第一版缺陷修复计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 按任务实施。步骤使用 checkbox（`- [ ]`）语法记录进度。

**Goal:** 修复桌面 app 第一版审计发现的 8 个缺陷（1 个重要 + 4 个明显 + 3 个轻微），全部从根因修复并补回归测试。

**Architecture:** 每项修复先写红灯测试再实现；涉及 UI 状态的可测试逻辑抽成纯函数/纯数据结构；会话控制器 `connect` 改为事务语义（失败回滚）；字体改为「系统字体优先 + 内置 Apache-2.0 兜底字体」保证任何环境至少有一个字体。

**Tech Stack:** Rust 1.89、eframe/egui 0.33、tokio、现有 ipkvm-desktop 模块。

## Global Constraints

- 仓库内自写文档使用中文；代码标识符、协议字段和第三方专有名词保留原文。
- 禁止补丁式修复：每项必须给出根因、失败路径、修复点与回归测试。
- 新增测试先红灯后绿灯；验证命令以 `.\scripts\verify.ps1` 全绿为准。
- 新增资产（内置字体）必须带许可证文本；Apache-2.0 已在全局允许列表。
- 提交使用英文 conventional commit；PR 关联 #33。
- 本计划不扩大桌面 app 功能范围，不修改 headless 行为。

---

### 任务 1：字体兜底——任何环境都保证至少一个字体（P1）

**文件：**
- 新建：`crates/ipkvm-desktop/assets/Roboto-Regular.ttf`
- 新建：`crates/ipkvm-desktop/assets/ROBOTO-LICENSE.txt`
- 修改：`crates/ipkvm-desktop/src/fonts.rs`

**根因：** 去掉 `default_fonts` 后，`install()` 找不到系统字体时只打印告警，egui 拿到空的 `FontDefinitions`；渲染任意文本时 epaint 内部 `panic!("FontFamily::{family:?} is not bound to any fonts")`（epaint-0.33.3/src/text/fonts.rs:808），应用启动即崩。

**修复设计：** `fonts.rs` 拆出纯函数 `resolve_font_bytes(candidates) -> Option<Vec<u8>>`（按候选顺序读第一个可读字体）；找不到时返回内置 `fallback_font_bytes()`（Roboto-Regular，Apache-2.0，随二进制分发）。`install()` 永远至少装一个字体。

- [x] **步骤 1：准备内置字体资产**

```powershell
Invoke-WebRequest -Uri "https://github.com/google/fonts/raw/main/apache/roboto/static/Roboto-Regular.ttf" -OutFile "crates\ipkvm-desktop\assets\Roboto-Regular.ttf"
Invoke-WebRequest -Uri "https://www.apache.org/licenses/LICENSE-2.0.txt" -OutFile "crates\ipkvm-desktop\assets\ROBOTO-LICENSE.txt"
```

校验：文件头 4 字节为 `\x00\x01\x00\x00`（TrueType）或 `OTTO`（CFF），大小 > 100KB。

- [x] **步骤 2：写红灯测试**

在 `fonts.rs` tests 加入：

```rust
#[test]
fn fallback_font_is_embedded_and_parseable() {
    let bytes = fallback_font_bytes();
    assert!(bytes.len() > 100_000);
    assert_eq!(&bytes[..4], &[0x00, 0x01, 0x00, 0x00]);
}

#[test]
fn resolve_font_bytes_prefers_first_readable_candidate() {
    let bytes = fallback_font_bytes().to_vec();
    let dir = std::env::temp_dir().join(format!("my-ipkvm-font-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("first.ttf");
    let second = dir.join("second.ttf");
    std::fs::write(&first, &bytes).unwrap();
    std::fs::write(&second, b"not a font").unwrap();
    let resolved = resolve_font_bytes(vec![first.clone(), second]).unwrap();
    assert_eq!(resolved, bytes);
    let _ = std::fs::remove_dir_all(dir);
}
```

- [x] **步骤 3：运行测试确认红灯**

```powershell
cargo test -p ipkvm-desktop --all-features fonts::tests -- --nocapture
```

Expected: `fallback_font_bytes`/`resolve_font_bytes` 未定义，编译失败。

- [x] **步骤 4：实现**

```rust
pub fn fallback_font_bytes() -> &'static [u8] {
    include_bytes!("../assets/Roboto-Regular.ttf")
}

pub fn resolve_font_bytes(candidates: Vec<PathBuf>) -> Option<Vec<u8>> {
    candidates
        .into_iter()
        .find_map(|path| std::fs::read(&path).ok())
}

pub fn install(ctx: &eframe::egui::Context) {
    let bytes = resolve_font_bytes(system_font_candidates())
        .or_else(|| Some(fallback_font_bytes().to_vec()))
        .expect("embedded fallback font must exist");
    // 其余逻辑不变：插入 "system" 字体并设置两个 family。
}
```

- [x] **步骤 5：绿灯 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features fonts::tests -- --nocapture
git add crates/ipkvm-desktop/src/fonts.rs crates/ipkvm-desktop/assets
git commit -m "fix: guarantee a bundled fallback font"
```

### 任务 2：控制台错误可见 + 状态栏单一错误出口（P2）

**文件：**
- 修改：`crates/ipkvm-desktop/src/app.rs`

**根因：** `status_message` 只在设备 dialog 渲染（`device_dialog` 内），控制台没有任何渲染点；"不支持的按键/输入被拒绝/粘贴失败/截图失败"等消息被静默丢弃，用户无反馈。

**修复设计：** 状态栏作为唯一错误出口：`status_bar` 增加第五段"状态"，渲染 `status_message`（着色）；把状态栏文案抽成可测纯函数 `status_bar_texts()`。

- [x] **步骤 1：红灯测试**

```rust
#[test]
fn status_texts_include_message_and_offline_state() {
    let mut app = DesktopApp::test_instance();
    app.showing_device_dialog = false;
    app.paste_busy = true;
    app.video_focused = true;
    app.status_message = Some("粘贴失败".into());
    app.selection.mark_control_offline();

    let texts = app.status_bar_texts();
    assert_eq!(texts.control, "离线");
    assert_eq!(texts.keyboard, "粘贴中");
    assert_eq!(texts.message, Some("粘贴失败".into()));
}
```

`DesktopApp::test_instance()` 为 `#[cfg(test)]` 构造器（跳过启动设备枚举）。

- [x] **步骤 2：实现**

`status_bar` 渲染改为从 `status_bar_texts()` 取字符串；状态段顺序：控制设备 / 键盘 / 鼠标 / 视频 / 状态（message 为 `None` 时省略该段）。

- [x] **步骤 3：验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features app::tests
git commit -m "fix: surface desktop status messages in status bar"
```

### 任务 3：指针输入改为聚焦门控（P2，计划偏移）

**文件：**
- 修改：`crates/ipkvm-desktop/src/input.rs`
- 修改：`crates/ipkvm-desktop/src/app.rs`

**根因：** 计划要求"输入只在视频区域聚焦时发送"，实现却用 `hovered()` 作为指针发送条件；未点击聚焦时悬停也会移动目标机鼠标，可能误操作。

**修复设计：** 抽出 `pointer_active(focused, mask, previous_mask) -> bool`：聚焦即活跃；未聚焦时只有按住（mask/previous_mask 非零，拖出窗口或松开在窗口外）才继续发送。

- [x] **步骤 1：红灯测试**（`input.rs`）

```rust
#[test]
fn pointer_active_requires_focus_or_held_button() {
    assert!(!pointer_active(false, 0, 0));
    assert!(pointer_active(true, 0, 0));
    assert!(pointer_active(false, 1, 0));
    assert!(pointer_active(false, 0, 1));
}
```

- [x] **步骤 2：实现 + 接入**

`handle_input` 中指针分支条件由 `response.hovered() || ...` 改为 `pointer_active(focused, mask, self.pointer_mask)`；点击获得焦点后当帧 `has_focus()` 为 true，按下/抬起仍会发送。

- [x] **步骤 3：验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features input::tests
git commit -m "fix: gate desktop pointer input on video focus"
```

### 任务 4：`connect()` 事务化，失败回滚（P2）

**文件：**
- 修改：`crates/ipkvm-desktop/src/session.rs`

**根因：** `connect()` 在 `replace_and_start` 成功前就写入 `self.frame_source` 并设置 notice mirror；失败时残留新相机句柄（相机被占）与旧 `event_tx`，状态不一致。

**修复设计：** 连接视为事务：工厂成功后先不落状态；`replace_and_start` 与 `Connected` 发送任一失败即调用 `rollback()`（`stop_and_destroy` 释放已组装会话 + 清空 `event_tx/frame_source` + 换新 `notice_rx`），再返回错误。

- [x] **步骤 1：红灯测试**

```rust
#[test]
fn failed_connect_rolls_back_controller_state() {
    let (mut controller, _sink) = controller_with_sink();
    controller.connect(request()).unwrap();
    controller.stop().unwrap();

    let mut controller = controller_with_failing_factory(); // 工厂返回 Err(Build)
    assert!(controller.connect(request()).is_err());
    assert!(!controller.is_control_online());
    assert!(controller.latest_frame().is_none());
}
```

（`controller_with_failing_factory` 用返回 `Err(DesktopSessionError::Build("boom".into()))` 的工厂。）

- [x] **步骤 2：实现**

`connect()` 改为：工厂 → 建 notice 通道并设 mirror → `block_on(replace_and_start)`，失败走 `rollback` → 成功后写 `frame_source/notice_rx` → 取 sender 并 `try_send(Connected)`，失败同样 `rollback`。

- [x] **步骤 3：验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features session::tests
git commit -m "fix: roll back desktop session state on connect failure"
```

### 任务 5：输入/粘贴 UI 状态随生命周期复位（P2/P3）

**文件：**
- 修改：`crates/ipkvm-desktop/src/app.rs`

**根因：** `paste_busy` 只靠 notice 解除，`video_focused` 只在 `handle_input` 维护；停止连接、控制设备离线后两者都不会复位，导致"粘贴菜单永久禁用""键盘状态误显示聚焦"。

**修复设计：** 抽 `sync_control_state()`：`!showing_device_dialog && !is_control_online()` 时标记控制离线并复位 `paste_busy/video_focused/pointer_mask/last_pointer/last_modifiers`；`stop_session()` 同样复位。`update_impl` 每帧调用。

- [x] **步骤 1：红灯测试**

```rust
#[test]
fn offline_sync_resets_paste_and_focus_state() {
    let mut app = DesktopApp::test_instance();
    app.showing_device_dialog = false;
    app.paste_busy = true;
    app.video_focused = true;
    app.pointer_mask = 1;

    app.sync_control_state();

    assert!(!app.paste_busy);
    assert!(!app.video_focused);
    assert_eq!(app.pointer_mask, 0);
    assert_eq!(app.selection.control_status, ControlProbeStatus::Disconnected);
}
```

- [x] **步骤 2：实现 + 验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features app::tests
git commit -m "fix: reset desktop input state on session lifecycle changes"
```

### 任务 6：`bgra_to_rgba` 长度校验（P3）

**文件：**
- 修改：`crates/ipkvm-desktop/src/frame.rs`

**根因：** 直接切片 `data[y*stride .. y*stride+width*4]`，数据短于预期时 slice panic，畸形帧会拖垮 GUI 线程。

**修复设计：** 转换前校验 `data.len() >= stride*(height-1) + width*4`（`height == 0` 时按 0 处理），不足返回 `Err`。

- [x] **步骤 1：红灯测试**

```rust
#[test]
fn bgra_to_rgba_rejects_truncated_data() {
    let frame = bgra_frame(2, 2, 8, vec![0; 4]); // 需要 16 字节，只有 4
    assert!(bgra_to_rgba(&frame).is_err());
}
```

- [x] **步骤 2：实现 + 验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features frame::tests
git commit -m "fix: validate frame length before pixel conversion"
```

### 任务 7：`refresh_detection` 错误传播（P3）

**文件：**
- 修改：`crates/ipkvm-desktop/src/probe.rs`
- 修改：`crates/ipkvm-desktop/src/app.rs`

**根因：** `unwrap_or_default()` 吞掉枚举错误并把列表替换为空，选中设备被误标"断开"，用户无提示。

**修复设计：** `refresh_detection` 返回 `Result<(), ProbeError>`：任一列表枚举失败即返回错误且**不替换**旧列表、不重探；app 把错误写入 `status_message`（dialog 与状态栏均可见）。启动时首轮枚举失败只告警不阻塞。

- [x] **步骤 1：红灯测试**（`probe.rs`）

```rust
#[test]
fn refresh_detection_propagates_list_errors_without_replacing_state() {
    let mut backend = FailingListBackend; // list_video_devices 返回 Err
    let mut state = DeviceSelectionState {
        video_devices: vec![option("cam0", "Camera 0")],
        ..Default::default()
    };
    assert!(refresh_detection(&mut state, &mut backend, Duration::from_millis(10)).is_err());
    assert_eq!(state.video_devices.len(), 1);
}
```

- [x] **步骤 2：实现 + 接入 + 验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features probe::tests
git commit -m "fix: surface desktop device enumeration errors"
```

### 任务 8：`ResizeWindowToVideo` 真实跟随窗口（P3，设计偏差）

**文件：**
- 修改：`crates/ipkvm-desktop/src/app.rs`

**根因：** 该模式在 UI 中可选、渲染时却与 `FitWindow` 合并，窗口从不真正调整到视频尺寸，与设计文档"调整窗口到视频尺寸"不符。

**修复设计：** 连接控制台且缩放模式为 `ResizeWindowToVideo` 时，分辨率变化后向视口发送 `ViewportCommand::InnerSize(视频尺寸 + 菜单/状态栏高度)`；渲染逻辑保持 fit（窗口尺寸跟随视频后 fit≈1:1）。抽 `desired_window_inner_size(frame, chrome)` 纯函数并测试。

- [x] **步骤 1：红灯测试**

```rust
#[test]
fn desired_window_inner_size_adds_chrome() {
    let size = desired_window_inner_size(FrameSize { width: 1280, height: 720 }, 48.0);
    assert_eq!(size, egui::vec2(1280.0, 768.0));
}
```

- [x] **步骤 2：实现 + 验证 + 提交**

```powershell
cargo test -p ipkvm-desktop --all-features app::tests
git commit -m "fix: resize desktop window to follow video mode"
```

## 最终验收

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
.\scripts\verify.ps1
```

Expected: 全部 PASS；`verify.ps1` 的 Clippy `-D warnings` 无新告警。

## 自审

- 规格覆盖：8 个审计项各自有任务、根因、测试与提交点。
- 占位扫描：无 TBD/TODO；每步含具体测试代码或实现要点。
- 类型一致性：`pointer_active`、`sync_control_state`、`status_bar_texts`、`desired_window_inner_size`、`resolve_font_bytes` 在本计划内首次定义并被后续步骤使用。
