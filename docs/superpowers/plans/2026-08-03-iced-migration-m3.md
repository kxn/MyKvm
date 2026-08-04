# M3 输入接线迁移实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 把 spike 3 已验证的输入链路收编进 `ipkvm-desktop-iced`：物理键 → keysym → `controller.send_key`、修饰键状态层、Ctrl+Alt+K/M 本地拦截、相对鼠标（Raw Input + 采样器）接入、`flush_pending` 定时器、特殊键菜单与粘贴文本。

**Architecture:** `keymap.rs`/`relative.rs`/`platform/` 按 spike 原样移植；纯输入逻辑（特殊键序列、组合键、滚轮步数、限频）收口到 `input.rs`（iced 类型适配）；`App` 增加 `keyboard::listen`/`event::listen`/16ms `UiTick` 订阅，键盘与相对鼠标事件经消息进入 update 后调用 controller 输入 API；mock 控制器暴露 `RecordingSink` 供端到端断言。

**Tech Stack:** iced 0.14（tokio/image/advanced）、iced_test 0.14、`ipkvm-desktop`（controller）、`ipkvm-core`（serial）、windows crate（Raw Input，target 限定）、arboard（剪贴板）。

## 执行记录（2026-08-03）

- 分支：`codex/issue77-migration-m3`；基线 `be4bc09`（main）。
- 提交链：`7f97b5a`（keymap）→ `f89b353`（relative）→ `97e863a`（platform）→ `6285b1d`（input 逻辑）→ `e2d799f`（clipboard）→ `1e1065c`（app 输入接线）→ `76a0870`（rustfmt）→ 文档收口提交。
- 门禁：fmt / workspace tests / clippy `-D warnings` / rustdoc `-D warnings` 全部通过。
- 执行偏差（均已落码并有测试覆盖）：
  - iced 0.14 `keyboard::Key` 无 `Code` 变体：事件只消费 `physical_key: Physical::Code`，测试用 `Key::Unidentified` 占位。
  - iced `Modifiers` 常量名为 `CTRL/ALT/SHIFT/LOGO`（非 CONTROL）。
  - `RecordingSink` 增加 `key_events`/`pointer_batches` 记录，供端到端断言（M3 测试辅助，非生产路径）。
  - 相对鼠标测试工厂 `ChannelRelativeSource` 用内部可变性，不实现 Clone（Arc 持有）。
  - 突发补送测试必须持续触发 `UiTick`（真实 16ms 定时器语义），单次 tick 只能补送通道容量内的事件。
  - 特殊键 HID usage 断言按真实映射修正（Delete=0x4C，非 0x4A）。
  - Raw Input 冒烟首次运行偶发失败，复跑 3 次全部通过（物理鼠标事件穿插，已知环境敏感项）。
  - 绝对鼠标接线不在 #77 范围（M4/M5 收口）；相对模式未做光标锁定/隐藏（M4）。

## Global Constraints

- iced pin `0.14`，features `["tokio", "image", "advanced"]`（与 M0–M2 一致）。
- 迁移阶段每单必须新增测试（先红后绿），见 #82 与迁移设计文档第 4 节；禁止只靠既有测试变绿。
- 本单主体改 `ipkvm-desktop-iced`；`ipkvm-desktop` 不新增改动（M2 已导出所需共享逻辑）。
- 跨平台：相对鼠标平台差异收口 `RelativePointerSource` trait；macOS/其它平台保留 stub（`cargo check --target x86_64-apple-darwin` 因本机 CC-MISSING 属预期）。
- 提交信息英文 conventional commit 并引用 `#77`。
- 全量门禁：fmt / workspace 测试 / clippy -D warnings / rustdoc -D warnings。

---

## 文件结构

- `crates/ipkvm-desktop-iced/src/keymap.rs`：物理键 → keysym（spike 原样）。
- `crates/ipkvm-desktop-iced/src/relative.rs`：`RelativePointerSource` + `DeltaSampler`（spike 原样）。
- `crates/ipkvm-desktop-iced/src/platform/{mod,windows,stub}.rs`：Raw Input / stub（spike 原样）。
- `crates/ipkvm-desktop-iced/src/input.rs`：特殊键序列、组合键、修饰键 diff、滚轮步数、限频（iced 适配）。
- `crates/ipkvm-desktop-iced/src/clipboard.rs`：`ClipboardReader` trait + arboard 实现。
- `crates/ipkvm-desktop-iced/src/app.rs`：输入接线（键盘/相对鼠标/flush/粘贴/特殊键）。
- `crates/ipkvm-desktop-iced/tests/`：keymap_table/relative_pointer 移植。
- `crates/ipkvm-desktop-iced/locales/`：新增输入相关文案。

---

## Task 1: keymap 移植

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/keymap.rs`（复制 spike 同名文件）
- Create: `crates/ipkvm-desktop-iced/tests/keymap_table.rs`（复制 spike 同名测试，替换 crate 名）
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod keymap;`）

**Interfaces:**
- Produces: `physical_code_to_keysym(code: iced::keyboard::key::Code) -> Option<u32>`、`XK_*` 常量。

- [x] **Step 1: 复制文件并替换 crate 名**

```powershell
Copy-Item crates/ipkvm-desktop-iced-spike/src/keymap.rs crates/ipkvm-desktop-iced/src/keymap.rs
Copy-Item crates/ipkvm-desktop-iced-spike/tests/keymap_table.rs crates/ipkvm-desktop-iced/tests/keymap_table.rs
```

`tests/keymap_table.rs` 中 `ipkvm_desktop_iced_spike` → `ipkvm_desktop_iced`。

- [x] **Step 2: `lib.rs` 增加 `pub mod keymap;`**
- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced keymap:: --test keymap_table`
Expected: 2 + 3 = 5 passed。

- [x] **Step 4: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/keymap.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/tests/keymap_table.rs
git commit -m "feat(iced): port physical key to keysym mapping (#77)"
```

## Task 2: 相对鼠标 trait 与采样器移植

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/relative.rs`（复制 spike 同名文件）
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod relative;`）

**Interfaces:**
- Produces: `type DeltaReceiver = Receiver<(i16,i16)>`、`trait RelativePointerSource: Send { receiver() -> Result<DeltaReceiver,String>; stop() }`、`DeltaSampler::new(interval)`、`DeltaSampler::feed(dx, dy, now) -> Option<(i16,i16)>`。

- [x] **Step 1: 复制 spike 文件**（`Copy-Item .../relative.rs`）
- [x] **Step 2: `lib.rs` 增加 `pub mod relative;`**
- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced relative::`
Expected: 5 passed。

- [x] **Step 4: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/relative.rs crates/ipkvm-desktop-iced/src/lib.rs
git commit -m "feat(iced): port relative pointer trait and delta sampler (#77)"
```

## Task 3: 平台模块移植（Windows Raw Input / stub）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/platform/mod.rs`、`platform/windows.rs`、`platform/stub.rs`（复制 spike）
- Create: `crates/ipkvm-desktop-iced/tests/relative_pointer.rs`（复制 spike 同名测试，替换 crate 名）
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod platform;`）
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`（windows target 依赖，与 spike 一致）

**Interfaces:**
- Produces: `platform::create() -> Result<Box<dyn RelativePointerSource>, String>`（Windows Raw Input / 其它平台 stub）。

- [x] **Step 1: 复制 spike 文件并替换 crate 名**

```powershell
Copy-Item crates/ipkvm-desktop-iced-spike/src/platform crates/ipkvm-desktop-iced/src/platform -Recurse
Copy-Item crates/ipkvm-desktop-iced-spike/tests/relative_pointer.rs crates/ipkvm-desktop-iced/tests/relative_pointer.rs
```

`tests/relative_pointer.rs` 中 crate 名替换。

- [x] **Step 2: Cargo.toml 增加 target 依赖**

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.61", default-features = false, features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_System_LibraryLoader",
    "Win32_UI_Input",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [x] **Step 3: `lib.rs` 增加 `pub mod platform;`**
- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced --test relative_pointer`
Expected: Windows 1 passed（Raw Input 注入冒烟）；非 Windows 1 passed（stub 未实现）。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/platform crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/tests/relative_pointer.rs crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): port raw input platform module and smoke test (#77)"
```

## Task 4: 输入纯逻辑（input.rs）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/input.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod input;`）

**Interfaces:**
- Consumes: `iced::keyboard::{key::Code, Modifiers}`、`iced::mouse::{Button, ScrollDelta}`。
- Produces:
  - `pub enum SpecialKey { CtrlAltDel, Win, PrintScreen, AltTab }`
  - `pub enum KeyAction { Down(u32), Up(u32) }`
  - `pub fn special_key_sequence(key: SpecialKey) -> Vec<KeyAction>`
  - `pub fn special_key_from_menu(name: &str) -> Option<SpecialKey>`（"CtrlAltDel"/"Win"/"PrintScreen"/"AltTab"）
  - `pub fn is_remote_exit_combo(code: Code, modifiers: Modifiers, repeat: bool) -> bool`（KeyK + Ctrl+Alt + 非 repeat）
  - `pub fn is_mode_toggle_combo(code: Code, modifiers: Modifiers, repeat: bool) -> bool`（KeyM + Ctrl+Alt + 非 repeat）
  - `pub fn modifier_diff(previous: Modifiers, current: Modifiers) -> Vec<KeyAction>`（Shift→Ctrl→Alt→Logo 顺序，上下对称）
  - `pub fn wheel_steps(delta: ScrollDelta) -> i8`（Lines 取整；Pixels 按 50 点一步）
  - `pub fn pointer_changed(current: (u8,u16,u16), last: Option<(u8,u16,u16)>) -> bool`
  - `pub fn throttle_elapsed(now: Instant, last: Option<Instant>, interval: Duration) -> bool`

- [x] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::Code;
    use iced::keyboard::Modifiers;
    use iced::mouse::ScrollDelta;
    use std::time::{Duration, Instant};

    fn mods(ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        let mut m = Modifiers::empty();
        m.set(Modifiers::CONTROL, ctrl);
        m.set(Modifiers::ALT, alt);
        m.set(Modifiers::SHIFT, shift);
        m
    }

    #[test]
    fn ctrl_alt_del_sequence_presses_modifiers_before_delete_and_releases_reverse() {
        assert_eq!(
            special_key_sequence(SpecialKey::CtrlAltDel),
            vec![
                KeyAction::Down(0xffe3),
                KeyAction::Down(0xffe9),
                KeyAction::Down(0xffff),
                KeyAction::Up(0xffff),
                KeyAction::Up(0xffe9),
                KeyAction::Up(0xffe3),
            ]
        );
    }

    #[test]
    fn win_and_print_screen_are_single_press_release() {
        assert_eq!(
            special_key_sequence(SpecialKey::Win),
            vec![KeyAction::Down(0xffeb), KeyAction::Up(0xffeb)]
        );
        assert_eq!(
            special_key_sequence(SpecialKey::PrintScreen),
            vec![KeyAction::Down(0xff61), KeyAction::Up(0xff61)]
        );
    }

    #[test]
    fn alt_tab_holds_alt_while_tapping_tab() {
        assert_eq!(
            special_key_sequence(SpecialKey::AltTab),
            vec![
                KeyAction::Down(0xffe9),
                KeyAction::Down(0xff09),
                KeyAction::Up(0xff09),
                KeyAction::Up(0xffe9),
            ]
        );
    }

    #[test]
    fn special_key_from_menu_maps_names() {
        assert_eq!(special_key_from_menu("CtrlAltDel"), Some(SpecialKey::CtrlAltDel));
        assert_eq!(special_key_from_menu("Win"), Some(SpecialKey::Win));
        assert_eq!(special_key_from_menu("PrintScreen"), Some(SpecialKey::PrintScreen));
        assert_eq!(special_key_from_menu("AltTab"), Some(SpecialKey::AltTab));
        assert_eq!(special_key_from_menu("unknown"), None);
    }

    #[test]
    fn remote_exit_combo_requires_ctrl_alt_k_pressed_once() {
        assert!(is_remote_exit_combo(Code::KeyK, mods(true, true, false), false));
        assert!(!is_remote_exit_combo(Code::KeyK, mods(true, true, false), true));
        assert!(!is_remote_exit_combo(Code::KeyK, mods(true, false, false), false));
        assert!(!is_remote_exit_combo(Code::KeyK, mods(false, true, false), false));
        assert!(!is_remote_exit_combo(Code::KeyA, mods(true, true, false), false));
    }

    #[test]
    fn mode_toggle_combo_requires_ctrl_alt_m_pressed_once() {
        assert!(is_mode_toggle_combo(Code::KeyM, mods(true, true, false), false));
        assert!(!is_mode_toggle_combo(Code::KeyM, mods(true, true, false), true));
        assert!(!is_mode_toggle_combo(Code::KeyM, mods(true, false, false), false));
        assert!(!is_mode_toggle_combo(Code::KeyK, mods(true, true, false), false));
    }

    #[test]
    fn modifier_diff_emits_down_then_up_in_stable_order() {
        let none = Modifiers::empty();
        let pressed = mods(true, true, true);
        assert_eq!(
            modifier_diff(none, pressed),
            vec![
                KeyAction::Down(0xffe1),
                KeyAction::Down(0xffe3),
                KeyAction::Down(0xffe9),
            ]
        );
        assert_eq!(
            modifier_diff(pressed, none),
            vec![
                KeyAction::Up(0xffe1),
                KeyAction::Up(0xffe3),
                KeyAction::Up(0xffe9),
            ]
        );
    }

    #[test]
    fn wheel_steps_converts_lines_and_pixels() {
        assert_eq!(wheel_steps(ScrollDelta::Lines { x: 0.0, y: 2.0 }), 2);
        assert_eq!(wheel_steps(ScrollDelta::Lines { x: 0.0, y: -1.0 }), -1);
        assert_eq!(wheel_steps(ScrollDelta::Pixels { x: 0.0, y: -100.0 }), -2);
        assert_eq!(wheel_steps(ScrollDelta::Pixels { x: 0.0, y: 25.0 }), 0);
    }

    #[test]
    fn pointer_changed_detects_position_or_mask_changes() {
        let last = Some((0, 100, 100));
        assert!(!pointer_changed((0, 100, 100), last));
        assert!(pointer_changed((1, 100, 100), last));
        assert!(pointer_changed((0, 101, 100), last));
        assert!(pointer_changed((0, 100, 100), None));
    }

    #[test]
    fn throttle_elapsed_requires_interval_to_pass() {
        let start = Instant::now();
        assert!(throttle_elapsed(start, None, Duration::from_millis(33)));
        assert!(throttle_elapsed(
            start + Duration::from_millis(34),
            Some(start),
            Duration::from_millis(33)
        ));
        assert!(!throttle_elapsed(
            start + Duration::from_millis(32),
            Some(start),
            Duration::from_millis(33)
        ));
    }
}
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced input::`
Expected: FAIL（函数未定义）。

- [x] **Step 3: 实现 input.rs**

```rust
//! 键鼠事件 → RFB keysym/pointer 事件的纯适配逻辑（iced 版，移植自 egui input.rs）。

use std::time::{Duration, Instant};

use iced::keyboard::key::Code;
use iced::keyboard::Modifiers;
use iced::mouse::ScrollDelta;

pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ALT_L: u32 = 0xffe9;
pub const XK_SUPER_L: u32 = 0xffeb;
pub const XK_TAB: u32 = 0xff09;
pub const XK_PRINT: u32 = 0xff61;
pub const XK_DELETE: u32 = 0xffff;

/// 键盘动作：按下/抬起某个 keysym。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Down(u32),
    Up(u32),
}

/// 本地 OS 会拦截、无法从键盘直发的组合键。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialKey {
    CtrlAltDel,
    Win,
    PrintScreen,
    AltTab,
}

/// 特殊键菜单名 → 枚举（spike 菜单发布的是名字字符串）。
pub fn special_key_from_menu(name: &str) -> Option<SpecialKey> {
    match name {
        "CtrlAltDel" => Some(SpecialKey::CtrlAltDel),
        "Win" => Some(SpecialKey::Win),
        "PrintScreen" => Some(SpecialKey::PrintScreen),
        "AltTab" => Some(SpecialKey::AltTab),
        _ => None,
    }
}

fn press(keysym: u32) -> Vec<KeyAction> {
    vec![KeyAction::Down(keysym), KeyAction::Up(keysym)]
}

/// 特殊键序列：Ctrl+Alt+Del 按下修饰键后按 Delete 再逆序释放；Win/PrintScreen
/// 单键；Alt+Tab 按下 Alt 后按 Tab 再逆序释放。
pub fn special_key_sequence(key: SpecialKey) -> Vec<KeyAction> {
    match key {
        SpecialKey::CtrlAltDel => vec![
            KeyAction::Down(XK_CONTROL_L),
            KeyAction::Down(XK_ALT_L),
            KeyAction::Down(XK_DELETE),
            KeyAction::Up(XK_DELETE),
            KeyAction::Up(XK_ALT_L),
            KeyAction::Up(XK_CONTROL_L),
        ],
        SpecialKey::Win => press(XK_SUPER_L),
        SpecialKey::PrintScreen => press(XK_PRINT),
        SpecialKey::AltTab => vec![
            KeyAction::Down(XK_ALT_L),
            KeyAction::Down(XK_TAB),
            KeyAction::Up(XK_TAB),
            KeyAction::Up(XK_ALT_L),
        ],
    }
}

/// Ctrl+Alt+K：本地退出远程输入模式（本地拦截，不转发远端）。
pub fn is_remote_exit_combo(code: Code, modifiers: Modifiers, repeat: bool) -> bool {
    code == Code::KeyK && !repeat && modifiers.control() && modifiers.alt()
}

/// Ctrl+Alt+M：本地切换绝对/相对鼠标模式（本地拦截，不转发远端）。
pub fn is_mode_toggle_combo(code: Code, modifiers: Modifiers, repeat: bool) -> bool {
    code == Code::KeyM && !repeat && modifiers.control() && modifiers.alt()
}

/// 修饰键状态变化：false→true 发 Down，true→false 发 Up；顺序固定
/// Shift → Ctrl → Alt → Logo，保证上下对称。
pub fn modifier_diff(previous: Modifiers, current: Modifiers) -> Vec<KeyAction> {
    let mut actions = Vec::new();
    diff_modifier(previous.shift(), current.shift(), XK_SHIFT_L, &mut actions);
    diff_modifier(
        previous.control(),
        current.control(),
        XK_CONTROL_L,
        &mut actions,
    );
    diff_modifier(previous.alt(), current.alt(), XK_ALT_L, &mut actions);
    diff_modifier(previous.logo(), current.logo(), XK_SUPER_L, &mut actions);
    actions
}

fn diff_modifier(previous: bool, current: bool, keysym: u32, actions: &mut Vec<KeyAction>) {
    match (previous, current) {
        (false, true) => actions.push(KeyAction::Down(keysym)),
        (true, false) => actions.push(KeyAction::Up(keysym)),
        _ => {}
    }
}

/// 滚轮增量换算成滚轮步数（Lines 直接取整，Pixels 按 50 点一步）。
pub fn wheel_steps(delta: ScrollDelta) -> i8 {
    let steps = match delta {
        ScrollDelta::Lines { y, .. } => y,
        ScrollDelta::Pixels { y, .. } => y / 50.0,
    };
    steps.round().clamp(i8::MIN as f32, i8::MAX as f32) as i8
}

/// 指针位置或按钮掩码是否变化。
pub fn pointer_changed(current: (u8, u16, u16), last: Option<(u8, u16, u16)>) -> bool {
    last != Some(current)
}

/// 距上次发送是否已超过最小间隔（限频用；从未发送过且有待发数据时立即发送）。
pub fn throttle_elapsed(
    now: Instant,
    last: Option<Instant>,
    interval: Duration,
) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= interval)
}
```

`lib.rs` 增加 `pub mod input;`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced input::`
Expected: 9 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/input.rs crates/ipkvm-desktop-iced/src/lib.rs
git commit -m "feat(iced): input logic for special keys, combos and throttling (#77)"
```

## Task 5: 剪贴板读取薄层

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/clipboard.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod clipboard;`）
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`（`arboard.workspace = true`）

**Interfaces:**
- Produces: `trait ClipboardReader: Send + Sync { fn read_text(&self) -> Result<String, String> }`、`pub struct SystemClipboard;`（arboard 实现）、`pub fn read_clipboard_text(reader: &dyn ClipboardReader) -> Result<String, String>`（空文本视为 Ok("")，由调用方提示）。

- [x] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeClipboard;

    impl ClipboardReader for FakeClipboard {
        fn read_text(&self) -> Result<String, String> {
            Ok("hello".into())
        }
    }

    struct FailingClipboard;

    impl ClipboardReader for FailingClipboard {
        fn read_text(&self) -> Result<String, String> {
            Err("clipboard locked".into())
        }
    }

    #[test]
    fn reader_trait_returns_text() {
        assert_eq!(read_clipboard_text(&FakeClipboard), Ok("hello".into()));
    }

    #[test]
    fn reader_error_propagates() {
        assert_eq!(
            read_clipboard_text(&FailingClipboard),
            Err("clipboard locked".into())
        );
    }
}
```

- [x] **Step 2: 运行确认失败**
- [x] **Step 3: 实现 clipboard.rs**

```rust
//! 剪贴板读取薄层（粘贴文本用；系统实现走 arboard）。

/// 剪贴板读取接口：生产用系统剪贴板，测试注入 fake。
pub trait ClipboardReader: Send + Sync {
    fn read_text(&self) -> Result<String, String>;
}

/// 系统剪贴板（arboard）。
pub struct SystemClipboard;

impl ClipboardReader for SystemClipboard {
    fn read_text(&self) -> Result<String, String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|error| error.to_string())
    }
}

/// 读取剪贴板文本（空文本返回 Ok("")，调用方决定提示文案）。
pub fn read_clipboard_text(reader: &dyn ClipboardReader) -> Result<String, String> {
    reader.read_text()
}
```

- [x] **Step 4: 运行确认通过**（2 passed）
- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/clipboard.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): clipboard reader abstraction for paste (#77)"
```

## Task 6: App 输入接线（键盘/相对鼠标/flush/特殊键/粘贴）

**Files:**
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`
- Modify: `crates/ipkvm-desktop-iced/src/relative.rs`（增加 `RelativeSourceFactory`）
- Modify: `crates/ipkvm-desktop-iced/src/platform/mod.rs`（`PlatformRelativeSourceFactory`）
- Modify: `crates/ipkvm-desktop-iced/locales/{en,zh-CN}.yml`、`src/lib.rs`（I18N_KEYS/translate_key 增补）
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`（无新增，arboard 已加）

**Interfaces:**
- Consumes: `keymap::physical_code_to_keysym`、`input::*`、`relative::{DeltaReceiver, DeltaSampler, RelativePointerSource}`、`platform::create`、`clipboard::*`、`controller::{send_key, send_pointer_relative, set_mouse_mode, paste_text, release_all, flush_pending, drain_notices}`。
- Produces: `Message::{Keyboard, IcedEvent, UiTick}`、`App::recording_sink() -> Option<&RecordingSink>`（mock 端到端断言）、`RelativeSourceFactory`。

- [x] **Step 1: 增补文案（locales + I18N_KEYS + translate_key）**

en.yml / zh-CN.yml 增加：

```yaml
message:
  unsupported_key: "Unsupported key"            # 中文：不支持的按键
  clipboard_empty: "Clipboard is empty"         # 中文：剪贴板为空
  clipboard_read_failed: "Failed to read clipboard: %{error}"  # 中文：读取剪贴板失败：%{error}
  keyboard_send_failed: "Keyboard send failed: %{error}"      # 中文：键盘发送失败：%{error}
  pointer_send_failed: "Pointer send failed: %{error}"        # 中文：指针发送失败：%{error}

status:
  pasting: "Pasting"                             # 中文：粘贴中
  remote_input: "Remote input · Ctrl+Alt+K to exit"  # 中文：远程输入中 · Ctrl+Alt+K 退出
  keyboard_lost: "No focus"                      # 中文：失焦
  relative_mode: "Relative mode"                 # 中文：相对模式
```

`I18N_KEYS` 与 `translate_key` 同步增加上述 key。

- [x] **Step 2: 写失败测试（app 级，先红）**

在 `src/app.rs` 测试模块追加（并给 `RecordingSink` 增加 `pointer_batches` 计数）：

```rust
fn press_key(code: iced::keyboard::key::Code) -> Message {
    Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Code(code),
        modified_key: iced::keyboard::Key::Code(code),
        physical_key: iced::keyboard::key::Physical::Code(code),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    })
}

fn release_key(code: iced::keyboard::key::Code) -> Message {
    Message::Keyboard(iced::keyboard::Event::KeyReleased {
        key: iced::keyboard::Key::Code(code),
        modified_key: iced::keyboard::Key::Code(code),
        physical_key: iced::keyboard::key::Physical::Code(code),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::empty(),
    })
}

fn click_video(app: &mut MockApp) {
    let _ = app.update(Message::IcedEvent(iced::Event::Mouse(
        iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left),
    )));
}

fn enter_remote(app: &mut MockApp) {
    let _ = app.update(Message::Disconnect);
    let _ = app.update(Message::RefreshDevices);
    let _ = app.update(Message::SelectVideo("Camera 0".into()));
    let _ = app.update(Message::PreviewTick);
    let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
    let _ = app.update(Message::Connect);
    click_video(app);
}

fn wait_sink(app: &MockApp, count: usize) -> Vec<(bool, u8)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(sink) = app.recording_sink() {
            let recorded = sink.key_events.lock().unwrap();
            if recorded.len() >= count {
                return recorded.clone();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sink 事件未达 {count}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn keyboard_press_release_reaches_sink_in_order() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
    let _ = app.update(release_key(iced::keyboard::key::Code::KeyA));
    let recorded = wait_sink(&app, 2);
    assert_eq!(recorded[0], (true, 0x04));
    assert_eq!(recorded[1], (false, 0x04));
}

#[test]
fn five_hundred_mixed_keys_reach_sink_in_order() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    for i in 0..500 {
        let code = match i % 3 {
            0 => iced::keyboard::key::Code::KeyA,
            1 => iced::keyboard::key::Code::ArrowUp,
            _ => iced::keyboard::key::Code::F1,
        };
        let _ = app.update(press_key(code));
        let _ = app.update(release_key(code));
        let _ = app.update(Message::UiTick);
    }
    let recorded = wait_sink(&app, 1000);
    for (i, (down, _usage)) in recorded.iter().enumerate() {
        assert_eq!(*down, i % 2 == 0);
    }
}

#[test]
fn first_key_is_not_swallowed() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
    let recorded = wait_sink(&app, 1);
    assert_eq!(recorded[0], (true, 0x04));
}

#[test]
fn flush_tick_drains_burst_without_further_input() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    for _ in 0..50 {
        let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
    }
    let _ = app.update(Message::UiTick);
    let recorded = wait_sink(&app, 50);
    assert!(recorded.iter().all(|(down, _)| *down));
}

#[test]
fn ctrl_alt_k_exits_remote_input_without_forwarding() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    let mut modifiers = iced::keyboard::Modifiers::empty();
    modifiers.set(iced::keyboard::Modifiers::CONTROL, true);
    modifiers.set(iced::keyboard::Modifiers::ALT, true);
    let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Code(iced::keyboard::key::Code::KeyK),
        modified_key: iced::keyboard::Key::Code(iced::keyboard::key::Code::KeyK),
        physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyK),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    }));
    assert!(!app.remote_input(), "Ctrl+Alt+K 必须退出远程输入");
    let _ = app.update(press_key(iced::keyboard::key::Code::KeyA));
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(sink) = app.recording_sink() {
        assert_eq!(sink.key_events.lock().unwrap().len(), 0, "退出后不得转发");
    }
}

#[test]
fn ctrl_alt_m_toggles_mouse_mode() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    let before = app.connection.mouse_mode;
    let mut modifiers = iced::keyboard::Modifiers::empty();
    modifiers.set(iced::keyboard::Modifiers::CONTROL, true);
    modifiers.set(iced::keyboard::Modifiers::ALT, true);
    let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Code(iced::keyboard::key::Code::KeyM),
        modified_key: iced::keyboard::Key::Code(iced::keyboard::key::Code::KeyM),
        physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyM),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    }));
    assert_ne!(app.connection.mouse_mode, before, "Ctrl+Alt+M 必须切换鼠标模式");
}

#[test]
fn special_key_menu_sends_sequence() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    let _ = app.update(Message::Menu(MenuAction::SpecialKey("CtrlAltDel".into())));
    let recorded = wait_sink(&app, 6);
    assert_eq!(recorded[0], (true, 0xe0));
    assert_eq!(recorded[1], (true, 0xe2));
    assert_eq!(recorded[2], (true, 0x4a));
    assert_eq!(recorded[3], (false, 0x4a));
    assert_eq!(recorded[4], (false, 0xe2));
    assert_eq!(recorded[5], (false, 0xe0));
}

#[test]
fn paste_uses_clipboard_and_sets_busy() {
    let (mut app, _) = MockApp::new_mock();
    app.clipboard = Arc::new(crate::clipboard::SystemClipboard);
    // 用 fake 剪贴板替换（空文本也会进入 paste_text，由 controller 处理）。
    struct EmptyClipboard;
    impl crate::clipboard::ClipboardReader for EmptyClipboard {
        fn read_text(&self) -> Result<String, String> {
            Ok("hello".into())
        }
    }
    app.clipboard = Arc::new(EmptyClipboard);
    enter_remote(&mut app);
    let _ = app.update(Message::Menu(MenuAction::Simple("paste")));
    assert!(app.paste_busy(), "paste_text 成功后必须置 paste_busy");
}

#[test]
fn relative_pointer_delta_reaches_sink() {
    let (mut app, _) = MockApp::new_mock();
    enter_remote(&mut app);
    let _ = app.update(Message::SetMouseMode(MouseMode::Relative));
    // 注入相对源：测试工厂返回 channel，推入 (5, -3)。
    app.relative_factory = Arc::new(ChannelRelativeFactory::new());
    let _ = app.update(Message::UiTick); // 启动相对源
    app.push_relative_delta(5, -3);
    let _ = app.update(Message::UiTick); // 采样并发送
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(sink) = app.recording_sink() {
            if *sink.pointer_batches.lock().unwrap() > 0 {
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
```

> 注：`recording_sink()`、`remote_input()`、`paste_busy()`、`relative_factory`、`push_relative_delta` 为测试辅助 API；`special_key_menu_sends_sequence` 的 HID usage 断言以 session 键盘映射为准，若与预期不符在实现时按真实 usage 修正。

- [x] **Step 3: 运行确认失败**（`Message::Keyboard` 等未定义）
- [x] **Step 4: 实现 app.rs 输入接线**

新增字段：

```rust
remote_input: bool,
last_modifiers: iced::keyboard::Modifiers,
relative_source: Option<Box<dyn RelativePointerSource>>,
relative_rx: Option<DeltaReceiver>,
relative_sampler: DeltaSampler,
relative_wheel: i8,
pointer_mask: u8,
paste_busy: bool,
recording: Option<RecordingSink>,
clipboard: Arc<dyn crate::clipboard::ClipboardReader>,
relative_factory: Arc<dyn RelativeSourceFactory>,
```

`RecordingSink` 增加 `pub pointer_batches: Arc<Mutex<usize>>`，`handle_pointer_batch` 递增。

`relative.rs` 增加：

```rust
/// 相对鼠标源工厂：生产用平台实现，测试注入 channel。
pub trait RelativeSourceFactory: Send + Sync {
    fn create(&self) -> Result<Box<dyn RelativePointerSource>, String>;
}
```

`platform/mod.rs` 增加：

```rust
/// 平台默认相对鼠标源工厂。
pub struct PlatformRelativeSourceFactory;

impl crate::relative::RelativeSourceFactory for PlatformRelativeSourceFactory {
    fn create(&self) -> Result<Box<dyn crate::relative::RelativePointerSource>, String> {
        create()
    }
}
```

消息增加：

```rust
Keyboard(iced::keyboard::Event),
IcedEvent(iced::Event),
UiTick,
```

`subscription()` 增加：

```rust
let keyboard = iced::keyboard::listen().map(Message::Keyboard);
let events = iced::event::listen().map(Message::IcedEvent);
let ui_tick = iced::time::every(Duration::from_millis(16)).map(|_| Message::UiTick);
```

并合并进 `Subscription::batch`（保留 frames/window_events/preview_timer）。

`update` 增加分支（核心逻辑）：

```rust
Message::Keyboard(event) => {
    self.handle_keyboard_event(event);
    Task::none()
}
Message::IcedEvent(event) => {
    self.handle_iced_event(event);
    Task::none()
}
Message::UiTick => {
    let _ = self.controller.flush_pending();
    self.drain_notices();
    self.poll_relative();
    Task::none()
}
```

辅助方法：

```rust
fn handle_keyboard_event(&mut self, event: iced::keyboard::Event) {
    if !self.remote_input {
        return;
    }
    match event {
        iced::keyboard::Event::KeyPressed {
            physical_key,
            modifiers,
            repeat,
            ..
        } => {
            let iced::keyboard::key::Physical::Code(code) = physical_key else {
                return;
            };
            if is_remote_exit_combo(code, modifiers, repeat) {
                self.remote_input = false;
                self.last_modifiers = iced::keyboard::Modifiers::empty();
                return;
            }
            if is_mode_toggle_combo(code, modifiers, repeat) {
                self.toggle_mouse_mode();
                return;
            }
            if repeat {
                return;
            }
            let Some(keysym) = physical_code_to_keysym(code) else {
                self.status_message =
                    Some(t!("message.unsupported_key").to_string());
                return;
            };
            if let Err(error) = self.controller.send_key(true, keysym) {
                self.status_message = Some(
                    t!("message.keyboard_send_failed", error = error.to_string()).to_string(),
                );
            }
        }
        iced::keyboard::Event::KeyReleased { physical_key, .. } => {
            let iced::keyboard::key::Physical::Code(code) = physical_key else {
                return;
            };
            if let Some(keysym) = physical_code_to_keysym(code)
                && let Err(error) = self.controller.send_key(false, keysym)
            {
                self.status_message = Some(
                    t!("message.keyboard_send_failed", error = error.to_string()).to_string(),
                );
            }
        }
        iced::keyboard::Event::ModifiersChanged(modifiers) => {
            for action in modifier_diff(self.last_modifiers, modifiers) {
                self.send_key_action(action);
            }
            self.last_modifiers = modifiers;
        }
    }
}

fn handle_iced_event(&mut self, event: iced::Event) {
    if !self.controller.is_control_online() {
        return;
    }
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button)) => {
            self.pointer_mask |= mouse_button_bit(button);
            if !self.remote_input {
                self.remote_input = true;
            }
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(button)) => {
            self.pointer_mask &= !mouse_button_bit(button);
        }
        iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) => {
            self.relative_wheel = self.relative_wheel.saturating_add(wheel_steps(delta));
        }
        _ => {}
    }
}

fn send_key_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::Down(keysym) => {
                let _ = self.controller.send_key(true, keysym);
            }
            KeyAction::Up(keysym) => {
                let _ = self.controller.send_key(false, keysym);
            }
        }
    }

fn send_special(&mut self, key: SpecialKey) {
    for action in special_key_sequence(key) {
        self.send_key_action(action);
    }
}

fn drain_notices(&mut self) {
    for notice in self.controller.drain_notices() {
        match notice {
            ipkvm_session::rfb_input::RfbInputNotice::TextTyped { .. }
            | ipkvm_session::rfb_input::RfbInputNotice::TextInputFailed { .. } => {
                self.paste_busy = false;
            }
            _ => {}
        }
    }
}

fn poll_relative(&mut self) {
    if !self.remote_input
        || !self.controller.is_control_online()
        || self.connection.mouse_mode != MouseMode::Relative
    {
        return;
    }
    if self.relative_rx.is_none() {
        match self.relative_factory.create() {
            Ok(mut source) => match source.receiver() {
                Ok(rx) => {
                    self.relative_source = Some(source);
                    self.relative_rx = Some(rx);
                }
                Err(error) => {
                    self.status_message = Some(format!("relative capture: {error}"));
                }
            },
            Err(error) => {
                self.status_message = Some(format!("relative capture: {error}"));
            }
        }
    }
    let Some(rx) = &self.relative_rx else {
        return;
    };
    let mut acc = (0.0f32, 0.0f32);
    while let Ok((dx, dy)) = rx.try_recv() {
        acc.0 += f32::from(dx);
        acc.1 += f32::from(dy);
    }
    let now = Instant::now();
    if let Some((dx, dy)) = self.relative_sampler.feed(acc.0, acc.1, now) {
        let wheel = self.relative_wheel;
        if dx != 0 || dy != 0 || wheel != 0 || self.pointer_mask != 0 {
            if let Err(error) =
                self.controller.send_pointer_relative(self.pointer_mask, dx, dy, wheel)
            {
                self.status_message = Some(
                    t!("message.pointer_send_failed", error = error.to_string()).to_string(),
                );
            }
            self.relative_wheel = 0;
        }
    }
}

fn toggle_mouse_mode(&mut self) {
    let next = match self.connection.mouse_mode {
        MouseMode::Absolute => MouseMode::Relative,
        MouseMode::Relative => MouseMode::Absolute,
    };
    match self.controller.set_mouse_mode(next) {
        Ok(()) => {
            self.connection.mouse_mode = next;
            if next != MouseMode::Relative {
                self.stop_relative_source();
            }
        }
        Err(_) => {}
    }
}

fn stop_relative_source(&mut self) {
    if let Some(mut source) = self.relative_source.take() {
        source.stop();
    }
    self.relative_rx = None;
}
```

`handle_menu_action` 增加：

```rust
MenuAction::SpecialKey(name) => {
    if let Some(key) = special_key_from_menu(&name) {
        self.send_special(key);
    }
}
MenuAction::Simple("paste") => self.paste(),
MenuAction::Simple("release_all") => {
    let _ = self.controller.release_all();
}
MenuAction::Simple(_) => {}
```

`paste()`：

```rust
fn paste(&mut self) {
    match crate::clipboard::read_clipboard_text(self.clipboard.as_ref()) {
        Ok(text) if !text.is_empty() => {
            if self.controller.paste_text(text).is_ok() {
                self.paste_busy = true;
            }
        }
        Ok(_) => self.status_message = Some(t!("message.clipboard_empty").to_string()),
        Err(error) => {
            self.status_message =
                Some(t!("message.clipboard_read_failed", error = error).to_string());
        }
    }
}
```

`Disconnect`/`Connect` 分支补充：断开时 `self.remote_input = false; self.stop_relative_source();`；连接成功且相对模式时保持 `remote_input` 由点击进入（相对源在 poll_relative 中按需启动）。

`new_mock` 初始化新字段：`remote_input: false`、`last_modifiers: Modifiers::empty()`、`relative_source/rx: None`、`relative_sampler: DeltaSampler::new(Duration::from_millis(33))`、`relative_wheel: 0`、`pointer_mask: 0`、`paste_busy: false`、`recording: Some(RecordingSink::default())`（工厂闭包改用 `self.recording` 的 clone）、`clipboard: Arc::new(SystemClipboard)`、`relative_factory: Arc::new(ChannelRelativeFactory)`（测试）或平台工厂。

测试辅助 API：

```rust
pub fn recording_sink(&self) -> Option<&RecordingSink> { self.recording.as_ref() }
pub fn remote_input(&self) -> bool { self.remote_input }
pub fn paste_busy(&self) -> bool { self.paste_busy }
```

`relative.rs` 测试模块增加 `ChannelRelativeSource`/`ChannelRelativeFactory`（`pub(crate)`）：

```rust
pub struct ChannelRelativeSource {
    tx: std::sync::mpsc::Sender<(i16, i16)>,
    rx: Option<DeltaReceiver>,
    started: bool,
}

impl ChannelRelativeSource {
    pub fn new() -> (Arc<Self>, std::sync::mpsc::Sender<(i16, i16)>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            Arc::new(Self { tx, rx: Some(rx), started: false }),
            tx,
        )
    }

    pub fn push(&self, dx: i16, dy: i16) {
        let _ = self.tx.send((dx, dy));
    }
}

impl RelativePointerSource for ChannelRelativeSource {
    fn receiver(&mut self) -> Result<DeltaReceiver, String> {
        if self.started {
            return Err("already started".into());
        }
        self.started = true;
        Ok(self.rx.take().expect("receiver available"))
    }

    fn stop(&mut self) {
        self.started = false;
    }
}
```

App 测试用工厂与注入点：

```rust
#[derive(Default)]
pub struct ChannelRelativeFactory {
    source: std::sync::Mutex<Option<Arc<ChannelRelativeSource>>>,
}

impl ChannelRelativeFactory {
    pub fn push(&self, dx: i16, dy: i16) {
        if let Some(source) = &*self.source.lock().unwrap() {
            source.push(dx, dy);
        }
    }
}

impl RelativeSourceFactory for ChannelRelativeFactory {
    fn create(&self) -> Result<Box<dyn RelativePointerSource>, String> {
        let (source, _tx) = ChannelRelativeSource::new();
        *self.source.lock().unwrap() = Some(Arc::clone(&source));
        Ok(Box::new((*source).clone()) as Box<dyn RelativePointerSource>)
    }
}
```

> ChannelRelativeSource 需实现 Clone（Arc 包装）。App 增加 `pub fn push_relative_delta(&self, dx: i16, dy: i16)` 测试辅助（转发给 ChannelRelativeFactory）。

- [x] **Step 5: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced app::`
Expected: M2 13 + M3 新增（keyboard 4 + combo 2 + special 1 + paste 1 + relative 1 = 9）≈ 22 passed；再跑 `cargo test -p ipkvm-desktop-iced`（含全部输入模块）。

- [x] **Step 6: fmt/clippy 修复并跑全量**

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy -p ipkvm-desktop-iced --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check -p ipkvm-desktop-iced --example video_1080p
```

- [x] **Step 7: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/app.rs crates/ipkvm-desktop-iced/src/relative.rs crates/ipkvm-desktop-iced/src/platform crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/locales crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): wire keyboard, relative mouse, flush timer, special keys and paste (#77)"
```

## Task 7: 门禁与验收

- [x] **Step 1: 全量门禁**

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --all-features --no-deps
```

- [x] **Step 2: 验收核对（对应 #77 / #82）**
  - [x] spike 3 测试全部移植（keymap 5 + relative 5 + raw input 1 + 相对鼠标 stub 1）
  - [x] 键盘端到端：500 键顺序/不吞首键/flush 补送（app 级）
  - [x] Ctrl+Alt+K 退出远程输入、Ctrl+Alt+M 切换鼠标模式（不转发远端）
  - [x] 特殊键序列与粘贴文本接线测试
  - [x] 相对鼠标增量 → sampler → sink 测试
  - [x] 回写 #77 验收结论；真实硬件冒烟（BIOS 方向键/相对鼠标/特殊键/粘贴）列为人工项
- [x] **Step 3: 提交文档更新并推送 PR**

```bash
git add docs/superpowers/plans/2026-08-03-iced-migration-m3.md HANDOFF.md
git commit -m "docs: record M3 plan and verification (#77)"
git push -u origin codex/issue77-migration-m3
```

- [x] **Step 4: PR → 自审 → 合并 → 关单**（`Closes #77`）
- [x] **Step 5: 同步 main 并继续 M4**

## Self-Review（计划自审）

- **Spec coverage**：对照 #77 与设计文档 3.5/3.6：键盘接入 ✅（Task 1/6）、修饰键状态层 ✅（Task 4/6）、Ctrl+Alt+K/M 拦截 ✅（Task 4/6）、相对鼠标 trait 接 UI ✅（Task 2/3/6）、flush_pending 定时器 ✅（Task 6）、特殊键菜单 ✅（Task 4/6）、粘贴 ✅（Task 5/6）。未覆盖项：绝对鼠标接线（不在 #77 范围，M4/M5 收口）、光标锁定/隐藏（M4 主题与观感）、macOS 相对鼠标实现（stub 留口）。
- **Placeholder scan**：无 TBD；`special_key_menu_sends_sequence` 的 HID usage 注明按真实映射修正。
- **Type consistency**：`physical_code_to_keysym`/`DeltaSampler::feed`/`special_key_sequence`/`RelativeSourceFactory::create` 跨任务签名一致；`Message::{Keyboard,IcedEvent,UiTick}` 在 Task 6 定义并被测试使用。

