# M2 菜单/模态/连接页/profile UI 迁移实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 把 spike 2 已验证的自绘菜单与模态收编进 `ipkvm-desktop-iced`，并实现连接页（设备下拉/预览/刷新/连接/断开）与 profile 保存/加载/最近使用，布局对齐现有 egui 桌面版。

**Architecture:** 复用 `ipkvm-desktop` 的共享逻辑（`state`/`probe`/`config` 模块仅增量导出、不动 egui UI）；菜单/模态按 spike 原样移植（状态机在 app 侧）；连接页状态机为纯函数 + `PreviewSourceFactory` trait（生产用真实相机、测试注入 mock），profile 应用/固化为纯函数；`App` 泛型化为 mock/生产两种 controller。

**Tech Stack:** iced 0.14（tokio/image/advanced）、iced_test 0.14、rust-i18n 4.2.1、sys-locale 0.3.2、`ipkvm-desktop`（controller/probe/config/state）、`ipkvm-video`（mock + mf 相机）、`ipkvm-core`（serial，类型命名用）。

## 执行记录（2026-08-03）

- 分支：`codex/issue76-migration-m2`；基线 `0302a9d`（main）。
- 提交链：`88f9367`（i18n）→ `b1be49b`（modal）→ `a5ec15e`（menu）→ `da2ad29`（desktop 共享导出）→ `e17a961`（connect）→ `df87443`（profile）→ `5067328`（app 集成）→ `dad8a28`（rustfmt）→ 文档收口提交。
- 门禁：fmt / workspace tests / clippy `-D warnings` / rustdoc `-D warnings` 全部通过。
- 观感截图：`docs/superpowers/artifacts/m2-screenshots/m2-connection-page.png`（自动化截图存档；真实相机预览为硬件项，视觉评审待用户确认）。
- 执行偏差（均已落码并有测试覆盖）：
  - `refresh_detection` 参数由 `&mut impl ProbeBackend` 改为 `&mut dyn ProbeBackend`：App 以 `Box<dyn ProbeBackend>` 持有探测后端，trait 对象不能匹配 `impl Trait`；egui 调用方经自动强转仍可编译。
  - rust-i18n `t!` 返回 `Cow<str>`：所有赋给 `String`/`Option<String>` 处显式 `.to_string()`。
  - `MockApp::new_mock()` 保留 M1「构造即连接」语义；连接页流程测试先发 `Disconnect` 进入断开态再走 选设备→预览→连接。
  - profile 流程测试先选控制设备再保存（原计划测试漏选，profile 不含 control 导致加载断言失败）。
  - mock 帧源永久保留最后一帧，无法模拟停帧；停帧用例改用 `OneShotSource` 测试替身（出帧后返回 None）。
  - 模态 LoadProfile 用 `Column::push` 构建列表（`column!` 宏不接受 `Vec<Button>`）。
  - 未跑 `cargo check --target x86_64-apple-darwin`（沿用 M1 结论：本机 CC-MISSING，属预期）。

## Global Constraints

- iced pin `0.14`，features `["tokio", "image", "advanced"]`（与 M0/M1 一致）。
- 迁移阶段每单必须新增测试（先红后绿），见 #82 与迁移设计文档第 4 节；禁止只靠既有测试变绿。
- 本单主体改 `ipkvm-desktop-iced`；`ipkvm-desktop` 只允许**增量导出**共享模块（`pub mod config/probe/state` + session 类型导出），不改 egui UI 逻辑。
- 布局对齐现有 egui 桌面版（菜单栏/连接页/视频区/状态栏）；单窗口。
- 跨平台：不引入 Windows 独占逻辑；M2 的 profile 加载/保存走应用内模态（不引 rfd 文件对话框，文件对话框留 M5）；尽量跑 `cargo check --target x86_64-apple-darwin`（CC-MISSING 属预期）。
- 提交信息英文 conventional commit 并引用 `#76`。
- 全量门禁：fmt / workspace 测试 / clippy -D warnings / rustdoc -D warnings。

---

## 文件结构

- `crates/ipkvm-desktop-iced/locales/{en,zh-CN}.yml`：i18n 文案（spike + 连接页/状态栏 key）。
- `crates/ipkvm-desktop-iced/src/locale.rs`：`AppLanguage`（System/Chinese/English，移植 egui desktop locale.rs）。
- `crates/ipkvm-desktop-iced/src/modal.rs`：自绘模态（spike 原样 + M2 扩展 LoadProfile）。
- `crates/ipkvm-desktop-iced/src/menu.rs`：自绘菜单（spike 原样）。
- `crates/ipkvm-desktop-iced/src/connect.rs`：连接页状态机薄层（预览工厂/超时/刷新决策）。
- `crates/ipkvm-desktop-iced/src/profile.rs`：profile 应用/固化纯函数。
- `crates/ipkvm-desktop-iced/src/app.rs`：App 泛型化 + 菜单/模态/连接页/状态栏集成。
- `crates/ipkvm-desktop-iced/tests/`：spike 测试移植（modal_blocking/menu_interact/corridor_hover/i18n_switch + common harness）。
- `crates/ipkvm-desktop/src/lib.rs`：增量导出（Task 4）。

---

## Task 1: i18n 基座与语言切换

**Files:**
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`
- Create: `crates/ipkvm-desktop-iced/locales/en.yml`
- Create: `crates/ipkvm-desktop-iced/locales/zh-CN.yml`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`
- Create: `crates/ipkvm-desktop-iced/src/locale.rs`

**Interfaces:**
- Produces: `crate::I18N_TEST_LOCK: Mutex<()>`、`crate::translate_key(key: &str) -> String`、`crate::locale::AppLanguage { System, Chinese, English }`（`label() -> String`、`apply()`）。
- Consumes: rust-i18n `t!` 宏、`rust_i18n::set_locale`。

- [x] **Step 1: 写失败测试**（`locale.rs` 测试 + lib.rs 文案测试）

在 `src/locale.rs` 写入：

```rust
//! 界面语言选择：跟随系统，或显式指定中文/英文（移植 egui desktop locale.rs）。

use rust_i18n::t;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppLanguage {
    System,
    Chinese,
    English,
}

impl AppLanguage {
    pub const ALL: [AppLanguage; 3] = [
        AppLanguage::System,
        AppLanguage::Chinese,
        AppLanguage::English,
    ];

    pub fn label(self) -> String {
        match self {
            AppLanguage::System => t!("language.system").to_string(),
            AppLanguage::Chinese => t!("language.chinese").to_string(),
            AppLanguage::English => t!("language.english").to_string(),
        }
    }

    pub fn apply(self) {
        rust_i18n::set_locale(match self {
            AppLanguage::System => detect_system_locale(),
            AppLanguage::Chinese => "zh-CN",
            AppLanguage::English => "en",
        });
    }
}

fn detect_system_locale() -> &'static str {
    map_system_locale(sys_locale::get_locale().as_deref())
}

fn map_system_locale(locale: Option<&str>) -> &'static str {
    match locale {
        Some(locale) if locale.starts_with("zh") => "zh-CN",
        Some(_) => "en",
        None => "zh-CN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_system_locales_map_to_zh_cn() {
        for locale in ["zh-CN", "zh-Hans-CN", "zh-TW", "zh-SG"] {
            assert_eq!(map_system_locale(Some(locale)), "zh-CN");
        }
    }

    #[test]
    fn non_chinese_system_locales_map_to_en() {
        for locale in ["en-US", "en-SG", "ja-JP", "de-DE"] {
            assert_eq!(map_system_locale(Some(locale)), "en");
        }
    }

    #[test]
    fn undetectable_system_locale_falls_back_to_zh_cn() {
        assert_eq!(map_system_locale(None), "zh-CN");
    }

    #[test]
    fn explicit_languages_apply_matching_locales() {
        let _guard = crate::I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rust_i18n::set_locale("en");
        AppLanguage::Chinese.apply();
        assert_eq!(&*rust_i18n::locale(), "zh-CN");
        AppLanguage::English.apply();
        assert_eq!(&*rust_i18n::locale(), "en");
        rust_i18n::set_locale("zh-CN");
    }
}
```

在 `src/lib.rs` 的测试模块写入（Step 1 先只写测试，Step 3 才加实现）：

```rust
#[test]
fn labels_are_single_line_nonempty_and_not_keys() {
    let _guard = I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    for locale in ["en", "zh-CN"] {
        rust_i18n::set_locale(locale);
        for key in I18N_KEYS {
            let label = translate_key(key);
            assert!(!label.contains('\n'), "[{locale}] {key} 译文不得含换行");
            assert!(!label.is_empty() && label != key, "[{locale}] {key} 译文不得为空或等于 key");
        }
    }
}
```

`I18N_KEYS` 常量（lib.rs）：

```rust
/// M2 全部 i18n key（labels 测试遍历）。
pub const I18N_KEYS: &[&str] = &[
    "menu.file", "menu.edit", "menu.send", "menu.about",
    "menu.reselect_device", "menu.stop_connection",
    "file.load_profile", "file.recent", "file.recent_more", "file.exit",
    "edit.copy_screenshot", "edit.language", "edit.settings",
    "send.paste_text", "send.release_all", "send.special_keys",
    "special_keys.ctrl_alt_del", "special_keys.win",
    "special_keys.print_screen", "special_keys.alt_tab",
    "language.system", "language.chinese", "language.english",
    "modal.settings_title", "modal.connection_title", "modal.close",
    "modal.about_title", "modal.save_title", "modal.load_title",
    "modal.name_label", "modal.save",
    "device.title", "device.video", "device.control", "device.refresh",
    "device.connect", "device.preview", "device.no_preview",
    "preview.no_signal", "preview.open_failed",
    "profile.save", "profile.saved", "profile.save_failed",
    "profile.load_failed", "profile.device_missing",
    "profile.control_missing", "profile.no_recent",
    "connection_settings.title",
    "settings.title", "settings.baud_rate", "settings.auto_baud",
    "settings.preview_fps", "settings.mouse_mode",
    "mouse_mode.absolute", "mouse_mode.relative",
    "status.control_device", "status.keyboard", "status.pointer",
    "status.video", "status.message", "status.offline",
    "status.video_no_signal", "status.video_stalled",
    "control_status.not_selected", "control_status.checking",
    "control_status.ready", "control_status.not_ch9329",
    "control_status.no_response", "control_status.open_failed",
    "control_status.offline",
    "video_status.not_selected", "video_status.checking",
    "video_status.ready", "video_status.no_signal",
    "video_status.open_failed", "video_status.disconnected",
    "control_status_label.not_selected", "control_status_label.checking",
    "control_status_label.ready", "control_status_label.not_ch9329",
    "control_status_label.no_response", "control_status_label.open_failed",
    "control_status_label.disconnected",
    "message.enumeration_failed", "message.connect_failed",
    "message.baud_selected", "message.offline_with_reason",
    "message.offline_reconnect", "message.input_rejected",
    "common.not_selected",
];
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced locale:: labels_are_single_line_nonempty_and_not_keys`
Expected: FAIL（`translate_key`/`I18N_KEYS`/`locale` 模块未定义）。

- [x] **Step 3: 实现**

`Cargo.toml` 增加：

```toml
rust-i18n = "4.2.1"
sys-locale = "0.3.2"
```

创建 `locales/en.yml`：

```yaml
_version: 1

menu:
  file: "File"
  edit: "Edit"
  send: "Send"
  about: "About"
  reselect_device: "Reselect device…"
  stop_connection: "Stop connection"

file:
  load_profile: "Load connection profile…"
  recent: "Recent"
  recent_more: "More…"
  exit: "Exit"

edit:
  copy_screenshot: "Copy screenshot"
  language: "Language"
  settings: "Settings…"

send:
  paste_text: "Paste text"
  release_all: "Release all keys/mouse"
  special_keys: "Send special keys"

special_keys:
  ctrl_alt_del: "Ctrl+Alt+Del"
  win: "Win"
  print_screen: "PrintScreen"
  alt_tab: "Alt+Tab"

language:
  system: "System"
  chinese: "Chinese"
  english: "English"

modal:
  settings_title: "Settings"
  connection_title: "Connection settings"
  close: "Close"
  about_title: "About"
  save_title: "Save profile"
  load_title: "Load profile"
  name_label: "Name:"
  save: "Save"

device:
  title: "Select device"
  video: "Video device"
  control: "Control device (CH9329)"
  refresh: "Refresh detection"
  connect: "Connect"
  preview: "Video preview"
  no_preview: "No preview"

preview:
  no_signal: "No signal"
  open_failed: "Open failed"

profile:
  save: "Save current options…"
  saved: "Profile \"%{name}\" saved"
  save_failed: "Failed to save profile: %{error}"
  load_failed: "Failed to load profile: %{error}"
  device_missing: "Video device not found"
  control_missing: "Control device not found"
  no_recent: "None yet"

connection_settings:
  title: "Connection settings"

settings:
  title: "Settings"
  baud_rate: "Baud rate"
  auto_baud: "Auto-detect baud rate on connect"
  preview_fps: "Preview FPS"
  mouse_mode: "Mouse mode"

mouse_mode:
  absolute: "Absolute"
  relative: "Relative"

status:
  control_device: "Control device: %{value}"
  keyboard: "Keyboard: %{value}"
  pointer: "Mouse: %{value}"
  video: "Video: %{value}"
  message: "Status: %{message}"
  offline: "Offline"
  video_no_signal: "No signal"
  video_stalled: "Stalled / no signal"

control_status:
  not_selected: "Not selected"
  checking: "Rechecking"
  ready: "CH9329(%{port})"
  not_ch9329: "Not a CH9329 (%{reason})"
  no_response: "No response"
  open_failed: "Open failed (%{error})"
  offline: "Offline"

video_status:
  not_selected: "Video: not selected"
  checking: "Video: checking…"
  ready: "Video: preview available %{width}×%{height} (%{label})"
  no_signal: "Video: no signal"
  open_failed: "Video: open failed %{error}"
  disconnected: "Video: device disconnected"

control_status_label:
  not_selected: "Control: not selected"
  checking: "Control: probing…"
  ready: "Control: CH9329(%{port})"
  not_ch9329: "Control: not a valid CH9329 (%{reason})"
  no_response: "Control: no response"
  open_failed: "Control: open failed %{error}"
  disconnected: "Control: device disconnected"

message:
  offline_with_reason: "Control device offline: %{reason}"
  offline_reconnect: "Control device offline; refresh detection and reconnect"
  input_rejected: "Input rejected"
  enumeration_failed: "Device enumeration failed: %{error}"
  connect_failed: "Connection failed: %{error}"
  baud_selected: "Auto-selected baud rate %{baud}"

common:
  not_selected: "Not selected"
```

创建 `locales/zh-CN.yml`（同一 key 集，中文译文）：

```yaml
_version: 1

menu:
  file: "文件"
  edit: "编辑"
  send: "发送"
  about: "关于"
  reselect_device: "重新选择设备…"
  stop_connection: "停止连接"

file:
  load_profile: "加载连接 profile…"
  recent: "最近使用"
  recent_more: "更多…"
  exit: "退出"

edit:
  copy_screenshot: "复制截图"
  language: "Language"
  settings: "设置…"

send:
  paste_text: "粘贴文本"
  release_all: "释放全部按键/鼠标"
  special_keys: "发送特殊键"

special_keys:
  ctrl_alt_del: "Ctrl+Alt+Del"
  win: "Win"
  print_screen: "PrintScreen"
  alt_tab: "Alt+Tab"

language:
  system: "跟随系统"
  chinese: "中文"
  english: "English"

modal:
  settings_title: "设置"
  connection_title: "连接设置"
  close: "关闭"
  about_title: "关于"
  save_title: "保存 profile"
  load_title: "加载 profile"
  name_label: "名称："
  save: "保存"

device:
  title: "选择设备"
  video: "视频设备"
  control: "控制设备（CH9329）"
  refresh: "刷新检测"
  connect: "连接"
  preview: "视频预览"
  no_preview: "无预览"

preview:
  no_signal: "无信号"
  open_failed: "打开失败"

profile:
  save: "保存当前选项…"
  saved: "已保存 profile“%{name}”"
  save_failed: "保存 profile 失败：%{error}"
  load_failed: "加载 profile 失败：%{error}"
  device_missing: "视频设备未找到"
  control_missing: "控制设备未找到"
  no_recent: "暂无"

connection_settings:
  title: "连接设置"

settings:
  title: "设置"
  baud_rate: "波特率"
  auto_baud: "连接时自动检测波特率"
  preview_fps: "预览帧率"
  mouse_mode: "鼠标模式"

mouse_mode:
  absolute: "绝对坐标"
  relative: "相对坐标"

status:
  control_device: "控制设备：%{value}"
  keyboard: "键盘：%{value}"
  pointer: "鼠标：%{value}"
  video: "视频：%{value}"
  message: "状态：%{message}"
  offline: "离线"
  video_no_signal: "无信号"
  video_stalled: "断流/无信号"

control_status:
  not_selected: "未选择"
  checking: "重新探测中"
  ready: "CH9329(%{port})"
  not_ch9329: "非 CH9329（%{reason}）"
  no_response: "无应答"
  open_failed: "打开失败（%{error}）"
  offline: "离线"

video_status:
  not_selected: "视频：未选择"
  checking: "视频：检测中…"
  ready: "视频：预览可用 %{width}×%{height}（%{label}）"
  no_signal: "视频：无信号"
  open_failed: "视频：打开失败 %{error}"
  disconnected: "视频：设备已断开"

control_status_label:
  not_selected: "控制：未选择"
  checking: "控制：探测中…"
  ready: "控制：CH9329(%{port})"
  not_ch9329: "控制：不是合法 CH9329（%{reason}）"
  no_response: "控制：无应答"
  open_failed: "控制：打开失败 %{error}"
  disconnected: "控制：设备已断开"

message:
  offline_with_reason: "控制设备离线：%{reason}"
  offline_reconnect: "控制设备离线，请刷新检测后重连"
  input_rejected: "输入被拒绝"
  enumeration_failed: "设备枚举失败：%{error}"
  connect_failed: "连接失败：%{error}"
  baud_selected: "已自动选择波特率 %{baud}"

common:
  not_selected: "未选择"
```

`lib.rs` 增加（`mod locale;` + i18n 基座 + 常量 + 测试模块）：

```rust
pub mod locale;

rust_i18n::i18n!("locales", fallback = "en");
use rust_i18n::t;

/// i18n 全局 locale 是进程级状态，涉及它的测试串行执行。
pub static I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 运行时翻译（集成测试无法直接调 t! 宏，收口成函数）。
pub fn translate_key(key: &str) -> String {
    match key {
        "menu.file" => t!("menu.file").to_string(),
        "menu.edit" => t!("menu.edit").to_string(),
        "menu.send" => t!("menu.send").to_string(),
        "menu.about" => t!("menu.about").to_string(),
        "menu.reselect_device" => t!("menu.reselect_device").to_string(),
        "menu.stop_connection" => t!("menu.stop_connection").to_string(),
        "file.load_profile" => t!("file.load_profile").to_string(),
        "file.recent" => t!("file.recent").to_string(),
        "file.recent_more" => t!("file.recent_more").to_string(),
        "file.exit" => t!("file.exit").to_string(),
        "edit.copy_screenshot" => t!("edit.copy_screenshot").to_string(),
        "edit.language" => t!("edit.language").to_string(),
        "edit.settings" => t!("edit.settings").to_string(),
        "send.paste_text" => t!("send.paste_text").to_string(),
        "send.release_all" => t!("send.release_all").to_string(),
        "send.special_keys" => t!("send.special_keys").to_string(),
        "special_keys.ctrl_alt_del" => t!("special_keys.ctrl_alt_del").to_string(),
        "special_keys.win" => t!("special_keys.win").to_string(),
        "special_keys.print_screen" => t!("special_keys.print_screen").to_string(),
        "special_keys.alt_tab" => t!("special_keys.alt_tab").to_string(),
        "language.system" => t!("language.system").to_string(),
        "language.chinese" => t!("language.chinese").to_string(),
        "language.english" => t!("language.english").to_string(),
        "modal.settings_title" => t!("modal.settings_title").to_string(),
        "modal.connection_title" => t!("modal.connection_title").to_string(),
        "modal.close" => t!("modal.close").to_string(),
        "modal.about_title" => t!("modal.about_title").to_string(),
        "modal.save_title" => t!("modal.save_title").to_string(),
        "modal.load_title" => t!("modal.load_title").to_string(),
        "modal.name_label" => t!("modal.name_label").to_string(),
        "modal.save" => t!("modal.save").to_string(),
        "device.title" => t!("device.title").to_string(),
        "device.video" => t!("device.video").to_string(),
        "device.control" => t!("device.control").to_string(),
        "device.refresh" => t!("device.refresh").to_string(),
        "device.connect" => t!("device.connect").to_string(),
        "device.preview" => t!("device.preview").to_string(),
        "device.no_preview" => t!("device.no_preview").to_string(),
        "preview.no_signal" => t!("preview.no_signal").to_string(),
        "preview.open_failed" => t!("preview.open_failed").to_string(),
        "profile.save" => t!("profile.save").to_string(),
        "profile.saved" => t!("profile.saved", name = "x").to_string(),
        "profile.save_failed" => t!("profile.save_failed", error = "x").to_string(),
        "profile.load_failed" => t!("profile.load_failed", error = "x").to_string(),
        "profile.device_missing" => t!("profile.device_missing").to_string(),
        "profile.control_missing" => t!("profile.control_missing").to_string(),
        "profile.no_recent" => t!("profile.no_recent").to_string(),
        "connection_settings.title" => t!("connection_settings.title").to_string(),
        "settings.title" => t!("settings.title").to_string(),
        "settings.baud_rate" => t!("settings.baud_rate").to_string(),
        "settings.auto_baud" => t!("settings.auto_baud").to_string(),
        "settings.preview_fps" => t!("settings.preview_fps").to_string(),
        "settings.mouse_mode" => t!("settings.mouse_mode").to_string(),
        "mouse_mode.absolute" => t!("mouse_mode.absolute").to_string(),
        "mouse_mode.relative" => t!("mouse_mode.relative").to_string(),
        "status.control_device" => t!("status.control_device", value = "x").to_string(),
        "status.keyboard" => t!("status.keyboard", value = "x").to_string(),
        "status.pointer" => t!("status.pointer", value = "x").to_string(),
        "status.video" => t!("status.video", value = "x").to_string(),
        "status.message" => t!("status.message", message = "x").to_string(),
        "status.offline" => t!("status.offline").to_string(),
        "status.video_no_signal" => t!("status.video_no_signal").to_string(),
        "status.video_stalled" => t!("status.video_stalled").to_string(),
        "control_status.not_selected" => t!("control_status.not_selected").to_string(),
        "control_status.checking" => t!("control_status.checking").to_string(),
        "control_status.ready" => t!("control_status.ready", port = "x").to_string(),
        "control_status.not_ch9329" => t!("control_status.not_ch9329", reason = "x").to_string(),
        "control_status.no_response" => t!("control_status.no_response").to_string(),
        "control_status.open_failed" => t!("control_status.open_failed", error = "x").to_string(),
        "control_status.offline" => t!("control_status.offline").to_string(),
        "video_status.not_selected" => t!("video_status.not_selected").to_string(),
        "video_status.checking" => t!("video_status.checking").to_string(),
        "video_status.ready" => t!("video_status.ready", width = 1, height = 1, label = "x").to_string(),
        "video_status.no_signal" => t!("video_status.no_signal").to_string(),
        "video_status.open_failed" => t!("video_status.open_failed", error = "x").to_string(),
        "video_status.disconnected" => t!("video_status.disconnected").to_string(),
        "control_status_label.not_selected" => t!("control_status_label.not_selected").to_string(),
        "control_status_label.checking" => t!("control_status_label.checking").to_string(),
        "control_status_label.ready" => t!("control_status_label.ready", port = "x").to_string(),
        "control_status_label.not_ch9329" => t!("control_status_label.not_ch9329", reason = "x").to_string(),
        "control_status_label.no_response" => t!("control_status_label.no_response").to_string(),
        "control_status_label.open_failed" => t!("control_status_label.open_failed", error = "x").to_string(),
        "control_status_label.disconnected" => t!("control_status_label.disconnected").to_string(),
        "message.enumeration_failed" => t!("message.enumeration_failed", error = "x").to_string(),
        "message.connect_failed" => t!("message.connect_failed", error = "x").to_string(),
        "message.baud_selected" => t!("message.baud_selected", baud = 9600).to_string(),
        "message.offline_with_reason" => t!("message.offline_with_reason", reason = "x").to_string(),
        "message.offline_reconnect" => t!("message.offline_reconnect").to_string(),
        "message.input_rejected" => t!("message.input_rejected").to_string(),
        "common.not_selected" => t!("common.not_selected").to_string(),
        _ => key.to_string(),
    }
}
```

`lib.rs` 增加 `pub mod locale;` 与 `I18N_KEYS`、`translate_key`、`I18N_TEST_LOCK`、测试（Step 1 的测试代码并入 lib.rs `mod tests`）。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced locale:: labels_are_single_line_nonempty_and_not_keys`
Expected: 4 + 1 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/Cargo.toml crates/ipkvm-desktop-iced/Cargo.lock crates/ipkvm-desktop-iced/locales crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/src/locale.rs
git commit -m "feat(iced): i18n scaffolding and language switching for M2 (#76)"
```

## Task 2: 自绘模态移植（modal.rs + 测试）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/modal.rs`（复制 spike 同名文件）
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod modal;`）
- Create: `crates/ipkvm-desktop-iced/tests/modal_blocking.rs`（复制 spike 同名测试，替换 crate 名）

**Interfaces:**
- Consumes: `rust_i18n::t`、iced widget（button/column/container/mouse_area/space/stack/text/text_input）。
- Produces: `ModalKind { Settings, Connection, SaveProfile, About }`、`ModalState { open, save_name }`、`ModalAction { Close, SaveNameChanged, Save, Noop }`、`modal::overlay(Element<ModalAction>) -> Element<ModalAction>`、`ModalState::{open, close, view, overlay}`。

- [x] **Step 1: 复制 spike 文件**

```powershell
Copy-Item crates/ipkvm-desktop-iced-spike/src/modal.rs crates/ipkvm-desktop-iced/src/modal.rs
Copy-Item crates/ipkvm-desktop-iced-spike/tests/modal_blocking.rs crates/ipkvm-desktop-iced/tests/modal_blocking.rs
```

把 `tests/modal_blocking.rs` 中的 `ipkvm_desktop_iced_spike` 全部替换为 `ipkvm_desktop_iced`。

- [x] **Step 2: `lib.rs` 增加 `pub mod modal;`**
- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced --test modal_blocking`
Expected: 4 passed（背景拦截/关闭按钮/Esc/关闭后恢复）。

- [x] **Step 4: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/modal.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/tests/modal_blocking.rs
git commit -m "feat(iced): port self-drawn modal overlay and tests (#76)"
```

## Task 3: 自绘菜单移植（menu.rs + 测试）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/menu.rs`（复制 spike 同名文件）
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod menu;`）
- Create: `crates/ipkvm-desktop-iced/tests/common/mod.rs`（复制 spike harness）
- Create: `crates/ipkvm-desktop-iced/tests/menu_interact.rs`
- Create: `crates/ipkvm-desktop-iced/tests/corridor_hover.rs`
- Create: `crates/ipkvm-desktop-iced/tests/i18n_switch.rs`

**Interfaces:**
- Consumes: `crate::modal::ModalKind`、`rust_i18n::t`。
- Produces: `MenuAction`、`LanguageChoice`、`MenuState::{apply}`、`MenuItem/MenuRoot`、`menu_bar(state: &MenuState, recent_profiles: &[&str]) -> Element<'a, MenuAction>`。

- [x] **Step 1: 复制 spike 文件并替换 crate 名**

```powershell
Copy-Item crates/ipkvm-desktop-iced-spike/src/menu.rs crates/ipkvm-desktop-iced/src/menu.rs
Copy-Item crates/ipkvm-desktop-iced-spike/tests/common crates/ipkvm-desktop-iced/tests/common -Recurse
Copy-Item crates/ipkvm-desktop-iced-spike/tests/menu_interact.rs crates/ipkvm-desktop-iced/tests/menu_interact.rs
Copy-Item crates/ipkvm-desktop-iced-spike/tests/corridor_hover.rs crates/ipkvm-desktop-iced/tests/corridor_hover.rs
Copy-Item crates/ipkvm-desktop-iced-spike/tests/i18n_switch.rs crates/ipkvm-desktop-iced/tests/i18n_switch.rs
```

把 4 个测试文件中的 `ipkvm_desktop_iced_spike` 全部替换为 `ipkvm_desktop_iced`。

- [x] **Step 2: `lib.rs` 增加 `pub mod menu;`**
- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced --test menu_interact --test corridor_hover --test i18n_switch`
Expected: 8 + 1 + 4 = 13 passed。

- [x] **Step 4: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/menu.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/tests
git commit -m "feat(iced): port self-drawn menu bar and corridor hover tests (#76)"
```

## Task 4: ipkvm-desktop 共享逻辑增量导出

**Files:**
- Modify: `crates/ipkvm-desktop/src/lib.rs`

**Interfaces:**
- Produces（后续任务消费）: `ipkvm_desktop::state::{DeviceOption, PreviewInfo, ControlInfo, VideoProbeStatus, ControlProbeStatus, DeviceSelectionState}`、`ipkvm_desktop::probe::{ProbeBackend, ProbeError, ProductionProbeBackend, refresh_detection, detect_baud_rate, BAUD_CANDIDATES}`、`ipkvm_desktop::config::{ProfileStore, Profile, DeviceRef, ConnectionSettings, ManualSnapshot, RECENT_LIMIT}`、`ipkvm_desktop::ProductionDesktopSessionController`、`ipkvm_desktop::ProductionSessionFactory`。

- [x] **Step 1: 改 lib.rs**

```rust
pub mod config;
pub mod probe;
pub mod state;
```

并把 session 导出改为：

```rust
pub use session::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, ProductionDesktopSessionController,
    ProductionSessionFactory, SessionParts,
};
```

其余（`mod app;` 等）不动。

- [x] **Step 2: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop`
Expected: 全部通过（既有 state/probe/config/session 测试覆盖）。

- [x] **Step 3: Commit**

```bash
git add crates/ipkvm-desktop/src/lib.rs
git commit -m "refactor(desktop): export shared state, probe and config modules for iced (#76)"
```

## Task 5: 连接页状态机与预览驱动（connect.rs）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/connect.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod connect;`）
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`（`ipkvm-video` 增加 `mf` feature）

**Interfaces:**
- Consumes: `ipkvm_desktop::{state, probe, config}`（Task 4）、`ipkvm_video::camera::CameraSource`、`ipkvm_video::FrameSource`。
- Produces: 重导出 `DeviceOption/DeviceSelectionState/VideoProbeStatus/ControlProbeStatus/PreviewInfo/ControlInfo/ConnectionSettings/Profile/ProfileStore/DeviceRef/ProbeBackend/ProbeError/ProductionProbeBackend/refresh_detection/detect_baud_rate`；`PreviewSourceFactory`、`CameraPreviewFactory`、`PreviewRefreshAction`、`preview_refresh_action`、`elapsed_since`、`PreviewRuntime::{reset, source, tick}`、常量 `PROBE_TIMEOUT`/`NO_SIGNAL_TIMEOUT`。

- [x] **Step 1: 写失败测试**（Step 3 前只建文件骨架并放入下面测试）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use ipkvm_video::mock::MockFrameSource;
    use ipkvm_video::{FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame};

    fn option(id: &str, label: &str) -> DeviceOption {
        DeviceOption { id: id.into(), label: label.into() }
    }

    fn ready_state() -> DeviceSelectionState {
        DeviceSelectionState {
            video_devices: vec![option("cam0", "Camera 0")],
            control_devices: vec![option("COM9", "COM9")],
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            video_status: VideoProbeStatus::Ready(PreviewInfo { width: 640, height: 480, label: "Camera 0".into() }),
            control_status: ControlProbeStatus::Ready(ControlInfo { version: 0x31, usb_enumerated: true }),
        }
    }

    fn make_frame(seq: u64, w: u32, h: u32) -> Arc<VideoFrame> {
        let mut data = vec![0u8; (w * h * 4) as usize];
        data[0] = 10; data[1] = 20; data[2] = 30; data[3] = 255;
        Arc::new(VideoFrame::new(
            seq, MonotonicTimestamp::from_nanos(seq), w, h, w * 4,
            PixelFormat::Bgra8888, Arc::from(data.into_boxed_slice()),
        ))
    }

    /// mock 预览源：open 即返回已有一帧 64×48 的 MockFrameSource。
    #[derive(Default)]
    struct MockPreviewFactory;

    impl PreviewSourceFactory for MockPreviewFactory {
        fn open(&self, device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            assert_eq!(device_id, "cam0");
            let mock = Arc::new(MockFrameSource::new());
            mock.publish_frame(make_frame(1, 64, 48));
            Ok(mock as Arc<dyn FrameSource>)
        }
    }

    #[derive(Default)]
    struct FailingPreviewFactory;

    impl PreviewSourceFactory for FailingPreviewFactory {
        fn open(&self, _device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            Err("boom".into())
        }
    }

    #[derive(Default)]
    struct EmptyPreviewFactory;

    impl PreviewSourceFactory for EmptyPreviewFactory {
        fn open(&self, device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
            assert_eq!(device_id, "cam0");
            Ok(Arc::new(MockFrameSource::new()) as Arc<dyn FrameSource>)
        }
    }

    #[test]
    fn connect_requires_video_ready_and_control_ready() {
        let mut state = DeviceSelectionState {
            video_status: VideoProbeStatus::Ready(PreviewInfo { width: 1920, height: 1080, label: "capture".into() }),
            control_status: ControlProbeStatus::NoResponse,
            ..DeviceSelectionState::default()
        };
        assert!(!state.can_connect());
        state.control_status = ControlProbeStatus::Ready(ControlInfo { version: 0x31, usb_enumerated: true });
        assert!(state.can_connect());
    }

    #[test]
    fn refresh_marks_missing_selected_devices_disconnected() {
        let mut state = ready_state();
        state.refresh_devices(Vec::new(), Vec::new());
        assert_eq!(state.video_status, VideoProbeStatus::Disconnected);
        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
        assert!(!state.can_connect());
    }

    #[test]
    fn mark_control_offline_sets_disconnected_status() {
        let mut state = ready_state();
        state.mark_control_offline();
        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
    }

    #[test]
    fn preview_refresh_skips_when_ready_or_checking_or_not_selected() {
        let cases = [
            (VideoProbeStatus::Ready(PreviewInfo { width: 1, height: 1, label: "x".into() }), true),
            (VideoProbeStatus::Checking, false),
            (VideoProbeStatus::NotSelected, true),
        ];
        for (status, present) in cases {
            assert_eq!(preview_refresh_action(&status, present), PreviewRefreshAction::Skip);
        }
    }

    #[test]
    fn preview_refresh_reopens_on_failure_or_no_signal() {
        for status in [
            VideoProbeStatus::OpenFailed("x".into()),
            VideoProbeStatus::NoSignal,
        ] {
            assert_eq!(preview_refresh_action(&status, true), PreviewRefreshAction::Reopen);
        }
    }

    #[test]
    fn preview_refresh_keeps_disconnected_when_device_gone_and_reopens_when_back() {
        assert_eq!(preview_refresh_action(&VideoProbeStatus::Disconnected, false), PreviewRefreshAction::KeepDisconnected);
        assert_eq!(preview_refresh_action(&VideoProbeStatus::Disconnected, true), PreviewRefreshAction::Reopen);
    }

    #[test]
    fn preview_timeout_only_moves_checking_to_no_signal() {
        let t0 = Instant::now();
        assert!(!elapsed_since(None, Duration::from_secs(1), t0 + Duration::from_secs(2)));
        assert!(elapsed_since(Some(t0), Duration::from_secs(1), t0 + Duration::from_secs(2)));
    }

    #[test]
    fn preview_tick_reaches_ready_with_frame() {
        let mut state = DeviceSelectionState { selected_video_id: Some("cam0".into()), ..DeviceSelectionState::default() };
        let mut preview = PreviewRuntime::default();
        let got = preview.tick(&mut state, &MockPreviewFactory, 30, Instant::now());
        assert!(got);
        assert!(matches!(state.video_status, VideoProbeStatus::Ready(info) if info.width == 64 && info.height == 48));
        assert!(preview.source().is_some());
    }

    #[test]
    fn preview_tick_open_failure_sets_open_failed() {
        let mut state = DeviceSelectionState { selected_video_id: Some("cam0".into()), ..DeviceSelectionState::default() };
        let mut preview = PreviewRuntime::default();
        assert!(!preview.tick(&mut state, &FailingPreviewFactory, 30, Instant::now()));
        assert_eq!(state.video_status, VideoProbeStatus::OpenFailed("boom".into()));
    }

    #[test]
    fn preview_tick_no_frame_times_out_to_no_signal() {
        let mut state = DeviceSelectionState { selected_video_id: Some("cam0".into()), ..DeviceSelectionState::default() };
        let mut preview = PreviewRuntime::default();
        let t0 = Instant::now();
        assert!(!preview.tick(&mut state, &EmptyPreviewFactory, 30, t0));
        assert_eq!(state.video_status, VideoProbeStatus::Checking);
        assert!(!preview.tick(&mut state, &EmptyPreviewFactory, 30, t0 + PROBE_TIMEOUT));
        assert_eq!(state.video_status, VideoProbeStatus::NoSignal);
    }

    #[test]
    fn preview_tick_stall_after_ready_moves_to_no_signal() {
        let mut state = DeviceSelectionState { selected_video_id: Some("cam0".into()), ..DeviceSelectionState::default() };
        let mut preview = PreviewRuntime::default();
        let t0 = Instant::now();
        assert!(preview.tick(&mut state, &MockPreviewFactory, 30, t0));
        assert!(matches!(state.video_status, VideoProbeStatus::Ready(_)));
        // 同一 factory 返回的 mock 只有一帧：停帧超时 → NoSignal。
        assert!(!preview.tick(&mut state, &MockPreviewFactory, 30, t0 + NO_SIGNAL_TIMEOUT));
        assert_eq!(state.video_status, VideoProbeStatus::NoSignal);
    }

    #[test]
    fn preview_tick_disconnected_never_reopens() {
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            video_status: VideoProbeStatus::Disconnected,
            ..DeviceSelectionState::default()
        };
        let mut preview = PreviewRuntime::default();
        assert!(!preview.tick(&mut state, &MockPreviewFactory, 30, Instant::now()));
        assert_eq!(state.video_status, VideoProbeStatus::Disconnected);
        assert!(preview.source().is_none());
    }
}
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced connect::`
Expected: FAIL（`PreviewRuntime`/`preview_refresh_action` 未定义）。

- [x] **Step 3: 实现 connect.rs**

```rust
//! 连接页状态机与预览驱动（M2）。
//!
//! 复用 ipkvm-desktop 的共享逻辑（state/probe/config，Task 4 导出），
//! 这里只做 iced 侧薄层：预览源工厂、预览超时/状态推进、刷新决策。

pub use ipkvm_desktop::config::{
    ConnectionSettings, DeviceRef, ManualSnapshot, Profile, ProfileStore,
};
pub use ipkvm_desktop::probe::{
    ProbeBackend, ProbeError, ProductionProbeBackend, detect_baud_rate, refresh_detection,
};
pub use ipkvm_desktop::state::{
    ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState, PreviewInfo,
    VideoProbeStatus,
};

use std::sync::Arc;
use std::time::{Duration, Instant};

use ipkvm_video::FrameSource;

/// 控制设备探测超时（与 egui 端一致）。
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
/// 预览出帧后停帧视为无信号的超时。
pub const NO_SIGNAL_TIMEOUT: Duration = Duration::from_secs(3);

/// 预览源工厂：生产用真实相机，测试注入 mock。
pub trait PreviewSourceFactory {
    fn open(&self, device_id: &str, fps: u64) -> Result<Arc<dyn FrameSource>, String>;
}

/// 生产预览源：ipkvm-video 相机。
#[derive(Default)]
pub struct CameraPreviewFactory;

impl PreviewSourceFactory for CameraPreviewFactory {
    fn open(&self, device_id: &str, fps: u64) -> Result<Arc<dyn FrameSource>, String> {
        ipkvm_video::camera::CameraSource::open(device_id, fps)
            .map(|source| Arc::new(source) as Arc<dyn FrameSource>)
            .map_err(|error| error.to_string())
    }
}

/// 刷新枚举后视频预览的处理决策（复刻 egui app.rs 纯函数）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewRefreshAction {
    Skip,
    Reopen,
    KeepDisconnected,
}

pub fn preview_refresh_action(
    status: &VideoProbeStatus,
    device_present: bool,
) -> PreviewRefreshAction {
    match status {
        VideoProbeStatus::Ready(_)
        | VideoProbeStatus::Checking
        | VideoProbeStatus::NotSelected => PreviewRefreshAction::Skip,
        VideoProbeStatus::OpenFailed(_) | VideoProbeStatus::NoSignal => {
            PreviewRefreshAction::Reopen
        }
        VideoProbeStatus::Disconnected if device_present => PreviewRefreshAction::Reopen,
        VideoProbeStatus::Disconnected => PreviewRefreshAction::KeepDisconnected,
    }
}

/// 超时判定（复刻 egui app.rs）。
pub fn elapsed_since(since: Option<Instant>, timeout: Duration, now: Instant) -> bool {
    since.is_some_and(|at| now.duration_since(at) >= timeout)
}

/// 预览运行时：持有预览帧源与时间戳，按 tick 推进 video_status。
#[derive(Default)]
pub struct PreviewRuntime {
    source: Option<Arc<dyn FrameSource>>,
    device_id: Option<String>,
    opened_at: Option<Instant>,
    last_frame_at: Option<Instant>,
}

impl PreviewRuntime {
    pub fn reset(&mut self) {
        self.source = None;
        self.device_id = None;
        self.opened_at = None;
        self.last_frame_at = None;
    }

    /// 当前预览源（app 用它取最新帧构建 Handle）。
    pub fn source(&self) -> Option<&Arc<dyn FrameSource>> {
        self.source.as_ref()
    }

    /// 推进一帧：按 egui update_preview 语义打开/换源/超时判定。
    /// 返回 true 表示本 tick 收到了新帧（调用方应刷新预览 Handle）。
    pub fn tick(
        &mut self,
        selection: &mut DeviceSelectionState,
        factory: &dyn PreviewSourceFactory,
        fps: u64,
        now: Instant,
    ) -> bool {
        if selection.video_status == VideoProbeStatus::Disconnected {
            return false;
        }
        let video_id = selection.selected_video_id.clone();
        if self.device_id.as_deref() != video_id.as_deref() {
            self.reset();
            self.device_id = video_id.clone();
            match video_id {
                Some(id) => match factory.open(&id, fps) {
                    Ok(source) => {
                        self.source = Some(source);
                        self.opened_at = Some(now);
                        selection.video_status = VideoProbeStatus::Checking;
                    }
                    Err(error) => {
                        selection.video_status = VideoProbeStatus::OpenFailed(error);
                        return false;
                    }
                },
                None => {
                    selection.video_status = VideoProbeStatus::NotSelected;
                    return false;
                }
            }
        }
        let Some(source) = &self.source else {
            return false;
        };
        let Some(frame) = source.latest_frame() else {
            let stalled = match selection.video_status {
                VideoProbeStatus::Checking => {
                    elapsed_since(self.opened_at, PROBE_TIMEOUT, now)
                }
                VideoProbeStatus::Ready(_) => {
                    elapsed_since(self.last_frame_at, NO_SIGNAL_TIMEOUT, now)
                }
                _ => false,
            };
            if stalled {
                selection.video_status = VideoProbeStatus::NoSignal;
            }
            return false;
        };
        self.last_frame_at = Some(now);
        if !matches!(selection.video_status, VideoProbeStatus::Ready(_)) {
            selection.video_status = VideoProbeStatus::Ready(PreviewInfo {
                width: frame.width,
                height: frame.height,
                label: source.source_info().device_name,
            });
        }
        true
    }
}
```

`lib.rs` 增加 `pub mod connect;`；`Cargo.toml` 中 `ipkvm-video` 改为 `features = ["mock", "mf"]`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced connect::`
Expected: 11 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/connect.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): connection page state machine and preview driver (#76)"
```

## Task 6: profile 应用与固化（profile.rs）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/profile.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod profile;`）

**Interfaces:**
- Consumes: `ipkvm_desktop::config::{Profile, DeviceRef, ConnectionSettings}`、`crate::connect::{DeviceOption, DeviceSelectionState, ControlProbeStatus, VideoProbeStatus}`。
- Produces: `MissingDevices { video, control }`、`selected_device_ref`、`apply_profile_to_selection(selection, profile) -> MissingDevices`、`build_profile(name, selection, connection) -> Profile`。

- [x] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_desktop::config::{ConnectionSettings, DeviceRef, Profile};
    use crate::connect::{ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState, PreviewInfo, VideoProbeStatus};

    fn option(id: &str, label: &str) -> DeviceOption {
        DeviceOption { id: id.into(), label: label.into() }
    }

    fn selection() -> DeviceSelectionState {
        DeviceSelectionState {
            video_devices: vec![option("cam0", "Camera 0")],
            control_devices: vec![option("COM9", "CH9329 (COM9)")],
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            video_status: VideoProbeStatus::Ready(PreviewInfo { width: 640, height: 480, label: "Camera 0".into() }),
            control_status: ControlProbeStatus::Ready(ControlInfo { version: 0x31, usb_enumerated: true }),
        }
    }

    fn profile() -> Profile {
        Profile {
            name: "办公室".into(),
            video_device: Some(DeviceRef { id: "cam0".into(), label: "Camera 0".into() }),
            control_device: Some(DeviceRef { id: "COM9".into(), label: "CH9329 (COM9)".into() }),
            connection: ConnectionSettings::default(),
        }
    }

    #[test]
    fn apply_profile_selects_matching_devices() {
        let mut state = DeviceSelectionState::default();
        state.video_devices = vec![option("cam0", "Camera 0")];
        state.control_devices = vec![option("COM9", "CH9329 (COM9)")];
        let missing = apply_profile_to_selection(&mut state, &profile());
        assert_eq!(missing, MissingDevices::default());
        assert_eq!(state.selected_video_id.as_deref(), Some("cam0"));
        assert_eq!(state.selected_control_id.as_deref(), Some("COM9"));
        assert_eq!(state.video_status, VideoProbeStatus::Checking);
        assert_eq!(state.control_status, ControlProbeStatus::Checking);
    }

    #[test]
    fn apply_profile_clears_missing_devices_and_reports() {
        let mut state = DeviceSelectionState {
            video_devices: vec![option("other", "Other")],
            control_devices: vec![option("COM9", "CH9329 (COM9)")],
            selected_video_id: Some("other".into()),
            selected_control_id: Some("COM9".into()),
            ..DeviceSelectionState::default()
        };
        let missing = apply_profile_to_selection(&mut state, &profile());
        assert!(missing.video && !missing.control);
        assert_eq!(state.selected_video_id, None);
        assert_eq!(state.video_status, VideoProbeStatus::NotSelected);
        assert_eq!(state.selected_control_id.as_deref(), Some("COM9"));
    }

    #[test]
    fn selected_device_ref_falls_back_to_id_when_missing() {
        let devices = vec![option("cam0", "Camera 0")];
        assert_eq!(
            selected_device_ref(&devices, Some("cam0")),
            Some(DeviceRef { id: "cam0".into(), label: "Camera 0".into() })
        );
        assert_eq!(
            selected_device_ref(&devices, Some("gone")),
            Some(DeviceRef { id: "gone".into(), label: "gone".into() })
        );
        assert_eq!(selected_device_ref(&devices, None), None);
    }

    #[test]
    fn build_profile_captures_selection_and_connection() {
        let state = selection();
        let profile = build_profile("办公室".into(), &state, &ConnectionSettings::default());
        assert_eq!(profile.name, "办公室");
        assert_eq!(profile.video_device.as_ref().map(|d| d.id.as_str()), Some("cam0"));
        assert_eq!(profile.control_device.as_ref().map(|d| d.label.as_str()), Some("CH9329 (COM9)"));
    }
}
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced profile::`
Expected: FAIL（`apply_profile_to_selection` 等未定义）。

- [x] **Step 3: 实现 profile.rs**

```rust
//! profile 应用与固化（M2）：把 profile 应用到连接选择、把当前选择固化为 profile。

use ipkvm_desktop::config::{ConnectionSettings, DeviceRef, Profile};

use crate::connect::{
    ControlProbeStatus, DeviceOption, DeviceSelectionState, VideoProbeStatus,
};

/// 应用 profile 后缺失设备标记。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MissingDevices {
    pub video: bool,
    pub control: bool,
}

/// 把当前选中设备固化为 DeviceRef（label 兜底用 id，复刻 egui app.rs）。
pub fn selected_device_ref(
    devices: &[DeviceOption],
    selected_id: Option<&str>,
) -> Option<DeviceRef> {
    let id = selected_id?.to_string();
    let label = devices
        .iter()
        .find(|device| device.id == id)
        .map(|device| device.label.clone())
        .unwrap_or_else(|| id.clone());
    Some(DeviceRef { id, label })
}

/// 应用 profile 到当前选择：按 id 匹配设备；匹配不到清空该侧并标记缺失。
/// 视频选中后状态置 Checking（预览由 PreviewRuntime::tick 驱动）；
/// 控制选中后状态置 Checking（调用方随后同步探测）。
pub fn apply_profile_to_selection(
    selection: &mut DeviceSelectionState,
    profile: &Profile,
) -> MissingDevices {
    let mut missing = MissingDevices::default();
    if apply_device_ref(selection, profile.video_device.clone(), true) {
        missing.video = true;
    }
    if apply_device_ref(selection, profile.control_device.clone(), false) {
        missing.control = true;
    }
    missing
}

fn apply_device_ref(
    selection: &mut DeviceSelectionState,
    device: Option<DeviceRef>,
    is_video: bool,
) -> bool {
    let Some(device) = device else {
        clear_device_selection(selection, is_video);
        return false;
    };
    let matched = if is_video {
        selection
            .video_devices
            .iter()
            .find(|candidate| candidate.id == device.id)
            .map(|candidate| candidate.id.clone())
    } else {
        selection
            .control_devices
            .iter()
            .find(|candidate| candidate.id == device.id)
            .map(|candidate| candidate.id.clone())
    };
    match matched {
        Some(id) => {
            if is_video {
                selection.selected_video_id = Some(id);
                selection.video_status = VideoProbeStatus::Checking;
            } else {
                selection.selected_control_id = Some(id);
                selection.control_status = ControlProbeStatus::Checking;
            }
            false
        }
        None => {
            clear_device_selection(selection, is_video);
            true
        }
    }
}

fn clear_device_selection(selection: &mut DeviceSelectionState, is_video: bool) {
    if is_video {
        selection.selected_video_id = None;
        selection.video_status = VideoProbeStatus::NotSelected;
    } else {
        selection.selected_control_id = None;
        selection.control_status = ControlProbeStatus::NotSelected;
    }
}

/// 把当前选择与连接参数固化为 Profile（复刻 egui do_save_profile）。
pub fn build_profile(
    name: String,
    selection: &DeviceSelectionState,
    connection: &ConnectionSettings,
) -> Profile {
    Profile {
        name,
        video_device: selected_device_ref(
            &selection.video_devices,
            selection.selected_video_id.as_deref(),
        ),
        control_device: selected_device_ref(
            &selection.control_devices,
            selection.selected_control_id.as_deref(),
        ),
        connection: connection.clone(),
    }
}
```

`lib.rs` 增加 `pub mod profile;`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced profile::`
Expected: 4 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/profile.rs crates/ipkvm-desktop-iced/src/lib.rs
git commit -m "feat(iced): profile apply and build helpers (#76)"
```

## Task 7: App 集成（泛型化 + 菜单/模态/连接页/状态栏）

**Files:**
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`（整体重写）
- Modify: `crates/ipkvm-desktop-iced/src/modal.rs`（扩展 LoadProfile）
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub use app::{App, MockApp};`）
- Modify: `crates/ipkvm-desktop-iced/examples/video_1080p.rs`（适配 MockApp）
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`（增加 `ipkvm-core = { path = "../ipkvm-core", features = ["serial"] }`）

**Interfaces:**
- Consumes: `crate::{menu, modal, connect, profile, locale, status, scale, video, frames, perf}`、`ipkvm_desktop::{ConnectRequest, DesktopSessionController, DesktopSessionError, ProductionDesktopSessionController, SessionParts}`、`ipkvm_core::{Ch9329InputSink, InputSink, MouseMode, SerialCommandQueue}`。
- Produces: `MockApp = App<RecordingSink, MockFactory>`、`App::new_mock() -> (MockApp, Task<Message>)`、`App::production() -> (App<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory>, Task<Message>)`、共享 `update/view/subscription`、`desired_window_size`。

- [x] **Step 1: 扩展 modal.rs（先加失败测试）**

在 `tests/modal_blocking.rs` 追加：

```rust
#[test]
fn load_profile_modal_lists_names_and_picks() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut app = TestApp::default();
    app.modal.load_names = vec!["a".into(), "b".into()];
    app.open(ModalKind::LoadProfile);

    let mut ui = simulator::simulator(app.view());
    assert!(ui.click("a").is_ok(), "profile 名必须可点击");
    let msgs = messages_of(ui);
    assert!(
        msgs.contains(&Msg::Modal(ModalAction::LoadPicked("a".into()))),
        "点击 profile 名必须产生 LoadPicked（实际消息: {msgs:?}）"
    );
}
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced --test modal_blocking load_profile_modal_lists_names_and_picks`
Expected: FAIL（`ModalKind::LoadProfile`/`ModalAction::LoadPicked` 不存在）。

- [x] **Step 3: 扩展 modal.rs**

```rust
pub enum ModalKind {
    Settings,
    Connection,
    SaveProfile,
    LoadProfile,
    About,
}

#[derive(Clone, Debug, Default)]
pub struct ModalState {
    pub open: Option<ModalKind>,
    pub save_name: String,
    /// 加载 profile 模态的候选名（app 打开前填充）。
    pub load_names: Vec<String>,
}

pub enum ModalAction {
    Close,
    SaveNameChanged(String),
    Save,
    /// 点击某个候选 profile 名。
    LoadPicked(String),
    Noop,
}
```

`view()` 增加分支：

```rust
ModalKind::LoadProfile => self.load_profile_content(),
```

```rust
fn load_profile_content(&self) -> Element<'_, ModalAction> {
    let mut items: Vec<iced::widget::Button<'_, ModalAction>> = if self.load_names.is_empty() {
        vec![button(text(t!("profile.no_recent").to_string()))]
    } else {
        self.load_names
            .iter()
            .map(|name| {
                button(text(name.clone()))
                    .on_press(ModalAction::LoadPicked(name.clone()))
            })
            .collect()
    };
    items.push(close_button());
    column(items).spacing(8).into()
}
```

`title_for` 增加 `ModalKind::LoadProfile => t!("modal.load_title").to_string()`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced --test modal_blocking`
Expected: 5 passed。

- [x] **Step 5: 重写 app.rs（完整文件）**

```rust
//! 应用状态/消息/视图/订阅（M2）：菜单/模态/连接页/profile + M1 视频链路。

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::image::Handle;
use iced::{Color, Element, Length, Size, Subscription, Task};
use ipkvm_core::{
    Ch9329InputSink, InputError, InputSink, KeyEvent, MouseMode, PointerEvent,
    SerialCommandQueue,
};
use ipkvm_desktop::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, ProductionDesktopSessionController,
    ProductionSessionFactory, SessionParts,
};
use ipkvm_session::rfb_connection::RfbConnectionGate;
use ipkvm_video::mock::MockFrameSource;
use ipkvm_video::{FrameSource, VideoFrame};
use rust_i18n::t;

use crate::connect::{
    CameraPreviewFactory, ConnectionSettings, ControlProbeStatus, DeviceSelectionState,
    PreviewRefreshAction, PreviewRuntime, PreviewSourceFactory, ProductionProbeBackend,
    VideoProbeStatus, detect_baud_rate, preview_refresh_action, refresh_detection, PROBE_TIMEOUT,
};
use crate::frames::{FrameUpdate, frame_subscription};
use crate::locale::AppLanguage;
use crate::menu::{MenuAction, MenuState, menu_bar};
use crate::modal::{ModalAction, ModalKind, ModalState};
use crate::perf::FrameStats;
use crate::profile::{apply_profile_to_selection, build_profile};
use crate::scale::{FrameSize, ScaleMode};
use crate::status::{ConnectionStatus, derive_status};
use crate::video::handle_from_frame;
use crate::{WINDOW_SIZE, WINDOW_TITLE};

/// 记录型 sink：测试与 mock 连接用。
#[derive(Clone, Debug, Default)]
pub struct RecordingSink {
    pub key_batches: Arc<std::sync::Mutex<usize>>,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> Result<(), InputError> {
        Ok(())
    }
    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> Result<(), InputError> {
        *self.key_batches.lock().unwrap() += 1;
        Ok(())
    }
    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> Result<(), InputError> {
        Ok(())
    }
    fn release_all(&mut self) -> Result<(), InputError> {
        Ok(())
    }
}

pub type MockFactory =
    Box<dyn FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>>;
type MockController = DesktopSessionController<RecordingSink, MockFactory>;

/// 测试/示例用 App 类型。
pub type MockApp = App<RecordingSink, MockFactory>;
/// 生产 App 类型（真实相机 + CH9329 串口）。
pub type ProductionApp =
    App<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory>;

/// 测试用探测后端：控制设备永远 Ready。
#[derive(Default)]
struct FakeProbeBackend;

impl ipkvm_desktop::probe::ProbeBackend for FakeProbeBackend {
    fn list_video_devices(
        &mut self,
    ) -> Result<Vec<crate::connect::DeviceOption>, ipkvm_desktop::probe::ProbeError> {
        Ok(vec![crate::connect::DeviceOption {
            id: "cam0".into(),
            label: "Camera 0".into(),
        }])
    }

    fn list_control_devices(
        &mut self,
    ) -> Result<Vec<crate::connect::DeviceOption>, ipkvm_desktop::probe::ProbeError> {
        Ok(vec![crate::connect::DeviceOption {
            id: "COM9".into(),
            label: "CH9329 (COM9)".into(),
        }])
    }

    fn probe_control(
        &mut self,
        _device_id: &str,
        _baud_rate: u32,
        _timeout: Duration,
    ) -> ControlProbeStatus {
        ControlProbeStatus::Ready(crate::connect::ControlInfo {
            version: 0x31,
            usb_enumerated: true,
        })
    }
}

/// 应用消息。
#[derive(Clone, Debug)]
pub enum Message {
    FrameReady(VideoFrame),
    FrameClosed,
    SetScaleMode(ScaleMode),
    SetLetterboxColor(Color),
    ToggleLocale,
    WindowOpened(iced::window::Id),
    Menu(MenuAction),
    Modal(ModalAction),
    OpenModal(ModalKind),
    SelectVideo(String),
    SelectControl(String),
    RefreshDevices,
    Connect,
    Disconnect,
    PreviewTick,
    SetBaudRate(u32),
    SetAutoBaud(bool),
    SetPreviewFps(u64),
    SetMouseMode(MouseMode),
    LoadProfile(String),
}

/// 应用状态：controller + 连接页 + 菜单/模态 + 视频。
pub struct App<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: FnMut(&ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError>,
{
    pub(crate) controller: DesktopSessionController<S, F>,
    frame_source: Option<Arc<MockFrameSource>>,
    handle: Option<Handle>,
    frame_size: Option<FrameSize>,
    scale_mode: ScaleMode,
    letterbox_color: Color,
    status: ConnectionStatus,
    subscribed: bool,
    zh: bool,
    window_id: Option<iced::window::Id>,
    pending_resize: Option<Size>,
    stats: Option<Arc<FrameStats>>,
    menu: MenuState,
    modal: ModalState,
    selection: DeviceSelectionState,
    connection: ConnectionSettings,
    probe: Box<dyn ipkvm_desktop::probe::ProbeBackend>,
    preview: PreviewRuntime,
    preview_factory: Arc<dyn PreviewSourceFactory>,
    preview_handle: Option<Handle>,
    store: ipkvm_desktop::config::ProfileStore,
    active_profile: Option<String>,
    status_message: Option<String>,
}

impl App<RecordingSink, MockFactory> {
    /// 构造并连接 mock 会话（测试/示例用）。
    pub fn new_mock() -> (Self, Task<Message>) {
        let frame_source = Arc::new(MockFrameSource::new());
        let fs = Arc::clone(&frame_source);
        let factory: MockFactory = Box::new(move |_req| {
            let src: Arc<dyn FrameSource> = fs.clone();
            Ok((src, RecordingSink::default(), RfbConnectionGate::new()))
        });
        let mut controller = DesktopSessionController::with_factory(factory);
        controller.connect(connect_request()).expect("mock connect");
        let status =
            derive_status(controller.is_control_online(), controller.input_offline_reason());
        (
            Self {
                controller,
                frame_source: Some(frame_source),
                handle: None,
                frame_size: None,
                scale_mode: ScaleMode::FitWindow,
                letterbox_color: Color::from_rgb(0.0, 0.0, 0.0),
                status,
                subscribed: true,
                zh: true,
                window_id: None,
                pending_resize: None,
                stats: None,
                menu: MenuState::default(),
                modal: ModalState::default(),
                selection: DeviceSelectionState::default(),
                connection: ConnectionSettings::default(),
                probe: Box::new(FakeProbeBackend),
                preview: PreviewRuntime::default(),
                preview_factory: Arc::new(MockPreviewFactory),
                preview_handle: None,
                store: ipkvm_desktop::config::ProfileStore::new(
                    std::env::temp_dir().join(format!(
                        "my-ipkvm-iced-mock-{}",
                        std::process::id()
                    )),
                ),
                active_profile: None,
                status_message: None,
            },
            Task::none(),
        )
    }
}

impl App<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory> {
    /// 构造生产应用（真实 controller + 设备探测 + 相机预览 + 磁盘 profile）。
    pub fn production() -> (Self, Task<Message>) {
        let controller = ProductionDesktopSessionController::production();
        let store = ipkvm_desktop::config::ProfileStore::production();
        (
            Self {
                controller,
                frame_source: None,
                handle: None,
                frame_size: None,
                scale_mode: ScaleMode::FitWindow,
                letterbox_color: Color::from_rgb(0.0, 0.0, 0.0),
                status: ConnectionStatus::Disconnected,
                subscribed: true,
                zh: true,
                window_id: None,
                pending_resize: None,
                stats: None,
                menu: MenuState::default(),
                modal: ModalState::default(),
                selection: DeviceSelectionState::default(),
                connection: ConnectionSettings::default(),
                probe: Box::new(ProductionProbeBackend),
                preview: PreviewRuntime::default(),
                preview_factory: Arc::new(CameraPreviewFactory),
                preview_handle: None,
                store,
                active_profile: None,
                status_message: None,
            },
            Task::none(),
        )
    }
}

impl<S, F> App<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: FnMut(&ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError>,
{
    pub fn with_stats(mut self, stats: Arc<FrameStats>) -> Self {
        self.stats = Some(stats);
        self
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FrameReady(frame) => {
                self.handle = Some(handle_from_frame(&frame));
                if let Some(stats) = &self.stats {
                    stats.record_at(Instant::now());
                }
                self.frame_size = Some(FrameSize {
                    width: frame.width,
                    height: frame.height,
                });
                self.sync_status();
                if self.scale_mode == ScaleMode::ResizeWindowToVideo
                    && let Some(size) = desired_window_size(self.frame_size, self.scale_mode)
                {
                    if let Some(id) = self.window_id {
                        return iced::window::resize(id, size);
                    }
                    self.pending_resize = Some(size);
                }
                Task::none()
            }
            Message::FrameClosed => {
                self.subscribed = false;
                Task::none()
            }
            Message::SetScaleMode(mode) => {
                self.scale_mode = mode;
                Task::none()
            }
            Message::SetLetterboxColor(color) => {
                self.letterbox_color = color;
                Task::none()
            }
            Message::ToggleLocale => {
                self.zh = !self.zh;
                Task::none()
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                if let Some(size) = self.pending_resize.take() {
                    return iced::window::resize(id, size);
                }
                Task::none()
            }
            Message::Menu(action) => {
                if let Some(business) = self.menu.apply(action) {
                    self.handle_menu_action(business);
                }
                Task::none()
            }
            Message::Modal(action) => {
                self.handle_modal_action(action);
                Task::none()
            }
            Message::OpenModal(kind) => {
                if kind == ModalKind::LoadProfile {
                    self.modal.load_names = self.store.list_profiles();
                }
                self.modal.open(kind);
                Task::none()
            }
            Message::SelectVideo(label) => {
                if let Some(device) = self
                    .selection
                    .video_devices
                    .iter()
                    .find(|device| device.label == label)
                {
                    self.selection.selected_video_id = Some(device.id.clone());
                    self.selection.video_status = VideoProbeStatus::Checking;
                    self.preview.reset();
                    self.preview_handle = None;
                    self.active_profile = None;
                }
                Task::none()
            }
            Message::SelectControl(label) => {
                if let Some(device) = self
                    .selection
                    .control_devices
                    .iter()
                    .find(|device| device.label == label)
                {
                    self.selection.selected_control_id = Some(device.id.clone());
                    self.selection.control_status =
                        self.probe
                            .probe_control(&device.id, self.connection.baud_rate, PROBE_TIMEOUT);
                    self.active_profile = None;
                }
                Task::none()
            }
            Message::RefreshDevices => {
                let mut selection = self.selection.clone();
                match refresh_detection(
                    &mut selection,
                    self.probe.as_mut(),
                    self.connection.baud_rate,
                    PROBE_TIMEOUT,
                ) {
                    Ok(()) => {
                        self.selection = selection;
                        self.status_message = None;
                        let selected_present = self
                            .selection
                            .selected_video_id
                            .as_deref()
                            .is_some_and(|id| {
                                self.selection
                                    .video_devices
                                    .iter()
                                    .any(|device| device.id == id)
                            });
                        match preview_refresh_action(
                            &self.selection.video_status,
                            selected_present,
                        ) {
                            PreviewRefreshAction::Skip => {}
                            PreviewRefreshAction::Reopen => {
                                self.preview.reset();
                                self.preview_handle = None;
                                self.selection.video_status = VideoProbeStatus::Checking;
                            }
                            PreviewRefreshAction::KeepDisconnected => {
                                self.preview.reset();
                                self.preview_handle = None;
                            }
                        }
                    }
                    Err(error) => {
                        self.status_message =
                            Some(t!("message.enumeration_failed", error = error.to_string()));
                    }
                }
                Task::none()
            }
            Message::Connect => {
                self.preview.reset();
                self.preview_handle = None;
                if self.connection.auto_baud
                    && let Some(control_id) = self.selection.selected_control_id.clone()
                    && let Some(baud) = detect_baud_rate(&control_id, PROBE_TIMEOUT)
                {
                    self.connection.baud_rate = baud;
                    self.status_message = Some(t!("message.baud_selected", baud = baud));
                }
                let Some(request) = self.connect_request() else {
                    return Task::none();
                };
                match self.controller.connect(request) {
                    Ok(()) => {
                        self.status_message = None;
                        self.sync_status();
                        if let Some(name) = self.active_profile.clone() {
                            if let Err(error) = self.store.add_recent_profile(&name) {
                                self.status_message = Some(t!(
                                    "profile.save_failed",
                                    error = error.to_string()
                                ));
                            }
                        } else {
                            let snapshot = ipkvm_desktop::config::ManualSnapshot {
                                video_device: crate::profile::selected_device_ref(
                                    &self.selection.video_devices,
                                    self.selection.selected_video_id.as_deref(),
                                ),
                                control_device: crate::profile::selected_device_ref(
                                    &self.selection.control_devices,
                                    self.selection.selected_control_id.as_deref(),
                                ),
                                connection: self.connection.clone(),
                            };
                            if let Err(error) = self.store.set_last_manual(&snapshot) {
                                self.status_message = Some(t!(
                                    "profile.save_failed",
                                    error = error.to_string()
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        self.status_message =
                            Some(t!("message.connect_failed", error = error.to_string()));
                    }
                }
                Task::none()
            }
            Message::Disconnect => {
                let _ = self.controller.stop();
                self.sync_status();
                self.selection.control_status = ControlProbeStatus::Disconnected;
                self.selection.video_status = VideoProbeStatus::NotSelected;
                self.preview.reset();
                self.preview_handle = None;
                Task::none()
            }
            Message::PreviewTick => {
                if self.controller.is_control_online() {
                    return Task::none();
                }
                if self.preview.tick(
                    &mut self.selection,
                    self.preview_factory.as_ref(),
                    self.connection.preview_fps,
                    Instant::now(),
                ) && let Some(source) = self.preview.source()
                    && let Some(frame) = source.latest_frame()
                {
                    self.preview_handle = Some(handle_from_frame(&frame));
                }
                Task::none()
            }
            Message::SetBaudRate(baud) => {
                self.connection.baud_rate = baud;
                self.active_profile = None;
                Task::none()
            }
            Message::SetAutoBaud(enabled) => {
                self.connection.auto_baud = enabled;
                self.active_profile = None;
                Task::none()
            }
            Message::SetPreviewFps(fps) => {
                self.connection.preview_fps = fps;
                self.active_profile = None;
                Task::none()
            }
            Message::SetMouseMode(mode) => {
                self.connection.mouse_mode = mode;
                self.active_profile = None;
                Task::none()
            }
            Message::LoadProfile(name) => self.load_profile(&name),
        }
    }

    fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::OpenModal(kind) => {
                if kind == ModalKind::LoadProfile {
                    self.modal.load_names = self.store.list_profiles();
                }
                self.modal.open(kind);
            }
            MenuAction::SetLanguage(choice) => {
                let language = match choice {
                    crate::menu::LanguageChoice::System => AppLanguage::System,
                    crate::menu::LanguageChoice::Chinese => AppLanguage::Chinese,
                    crate::menu::LanguageChoice::English => AppLanguage::English,
                };
                language.apply();
                self.zh = rust_i18n::locale().starts_with("zh");
            }
            MenuAction::LoadRecent(name) => self.load_profile(&name),
            MenuAction::SpecialKey(_) | MenuAction::Simple(_) => {}
        }
    }

    fn handle_modal_action(&mut self, action: ModalAction) {
        match action {
            ModalAction::Close => self.modal.close(),
            ModalAction::SaveNameChanged(name) => self.modal.save_name = name,
            ModalAction::Save => self.save_profile(),
            ModalAction::LoadPicked(name) => {
                self.modal.close();
                self.load_profile(&name);
            }
            ModalAction::Noop => {}
        }
    }

    fn load_profile(&mut self, name: &str) {
        match self.store.load_profile(name) {
            Ok(profile) => {
                let missing =
                    apply_profile_to_selection(&mut self.selection, &profile);
                self.connection = profile.connection;
                self.active_profile = Some(profile.name);
                self.preview.reset();
                self.preview_handle = None;
                let mut notes = Vec::new();
                if missing.video {
                    notes.push(t!("profile.device_missing"));
                }
                if missing.control {
                    notes.push(t!("profile.control_missing"));
                }
                self.status_message = if notes.is_empty() {
                    None
                } else {
                    Some(notes.join("；"))
                };
            }
            Err(error) => {
                self.status_message =
                    Some(t!("profile.load_failed", error = error.to_string()));
            }
        }
    }

    fn save_profile(&mut self) {
        let name = self.modal.save_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let profile = build_profile(name.clone(), &self.selection, &self.connection);
        match self.store.save_profile(&profile) {
            Ok(()) => {
                self.status_message = Some(t!("profile.saved", name = name));
                self.modal.close();
            }
            Err(error) => {
                self.status_message =
                    Some(t!("profile.save_failed", error = error.to_string()));
            }
        }
    }

    fn connect_request(&self) -> Option<ConnectRequest> {
        Some(ConnectRequest {
            video_device_id: self.selection.selected_video_id.clone()?,
            control_device_id: self.selection.selected_control_id.clone()?,
            baud_rate: self.connection.baud_rate,
            mouse_mode: self.connection.mouse_mode,
            preview_fps: self.connection.preview_fps,
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let window_events = iced::window::open_events().map(Message::WindowOpened);
        let preview_timer =
            iced::time::every(Duration::from_millis(100)).map(|_| Message::PreviewTick);
        if !self.subscribed {
            return Subscription::batch([window_events, preview_timer]);
        }
        let frames = self
            .controller
            .subscribe_frames()
            .map(|receiver| {
                frame_subscription(0, receiver).map(|update| match update {
                    FrameUpdate::Frame(frame) => Message::FrameReady((*frame).clone()),
                    FrameUpdate::Closed => Message::FrameClosed,
                })
            })
            .unwrap_or_else(Subscription::none);
        Subscription::batch([frames, window_events, preview_timer])
    }

    pub fn view(&self) -> Element<'_, Message> {
        use iced::widget::{column, stack};
        let page = column![self.menu_view(), self.main_view(), self.status_line()];
        let page: Element<'_, Message> = page.into();
        match self.modal.view() {
            Some(modal) => stack![page, crate::modal::overlay(modal).map(Message::Modal)].into(),
            None => page,
        }
    }

    fn menu_view(&self) -> Element<'_, Message> {
        let recent: Vec<String> = self.store.recent_profiles();
        let recent_refs: Vec<&str> = recent.iter().map(String::as_str).collect();
        menu_bar(&self.menu, &recent_refs).map(Message::Menu)
    }

    fn main_view(&self) -> Element<'_, Message> {
        if self.controller.is_control_online() {
            self.video_view()
        } else {
            self.connection_view()
        }
    }

    fn video_view(&self) -> Element<'_, Message> {
        use iced::widget::{column, container, image, text};
        let video: Element<'_, Message> = match self.handle.as_ref() {
            Some(handle) => image::Image::<Handle>::new(handle.clone())
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => text("等待帧…").into(),
        };
        let video_area = container(video)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(self.letterbox_color.into()),
                ..Default::default()
            });
        column![video_area].into()
    }

    fn connection_view(&self) -> Element<'_, Message> {
        use iced::widget::{
            button, checkbox, column, container, image, pick_list, row, text, Checkbox, PickList,
        };
        let video_pick = PickList::new(
            self.video_labels(),
            self.selected_video_label(),
            Message::SelectVideo,
        )
        .placeholder(t!("common.not_selected"));
        let control_pick = PickList::new(
            self.control_labels(),
            self.selected_control_label(),
            Message::SelectControl,
        )
        .placeholder(t!("common.not_selected"));
        let baud_pick = PickList::new(
            vec![9600u32, 19200, 38400, 57600, 115200],
            Some(self.connection.baud_rate),
            Message::SetBaudRate,
        );
        let fps_pick = PickList::new(
            vec![10u64, 15, 30, 60],
            Some(self.connection.preview_fps),
            Message::SetPreviewFps,
        );
        let auto_baud = Checkbox::new(self.connection.auto_baud)
            .label(t!("settings.auto_baud"))
            .on_toggle(Message::SetAutoBaud);
        let relative = Checkbox::new(self.connection.mouse_mode == MouseMode::Relative)
            .label(t!("mouse_mode.relative"))
            .on_toggle(|on| {
                Message::SetMouseMode(if on {
                    MouseMode::Relative
                } else {
                    MouseMode::Absolute
                })
            });
        let connect = button(text(t!("device.connect")))
            .on_press_maybe(self.selection.can_connect().then_some(Message::Connect));
        let refresh = button(text(t!("device.refresh"))).on_press(Message::RefreshDevices);
        let save_profile =
            button(text(t!("profile.save"))).on_press(Message::OpenModal(ModalKind::SaveProfile));
        let load_profile = button(text(t!("file.load_profile")))
            .on_press(Message::OpenModal(ModalKind::LoadProfile));
        let preview: Element<'_, Message> = match &self.preview_handle {
            Some(handle) => image::Image::<Handle>::new(handle.clone())
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => text(self.preview_placeholder()).into(),
        };
        let preview_area = container(preview)
            .width(Length::Fill)
            .height(Length::Fixed(180.0))
            .style(|_theme| container::Style {
                background: Some(Color::from_rgb(0.08, 0.08, 0.08).into()),
                ..Default::default()
            });
        column![
            text(t!("device.title")).size(18),
            text(t!("device.video")),
            video_pick,
            self.video_status_text(),
            text(t!("device.control")),
            control_pick,
            self.control_status_text(),
            row![baud_pick, fps_pick].spacing(8),
            auto_baud,
            relative,
            row![refresh, connect, save_profile, load_profile].spacing(8),
            preview_area,
            self.status_message_view(),
        ]
        .spacing(8)
        .padding(12)
        .into()
    }

    fn status_line(&self) -> Element<'_, Message> {
        use iced::widget::{container, text};
        container(text(self.status.label(self.zh)))
            .width(Length::Fill)
            .padding(6)
            .into()
    }

    fn status_message_view(&self) -> Element<'_, Message> {
        use iced::widget::{container, text};
        match &self.status_message {
            Some(message) => container(text(message.clone())).padding(4).into(),
            None => container(text("")).into(),
        }
    }

    fn preview_placeholder(&self) -> String {
        match &self.selection.video_status {
            VideoProbeStatus::NoSignal => t!("preview.no_signal"),
            VideoProbeStatus::OpenFailed(_) => t!("preview.open_failed"),
            _ => t!("device.no_preview"),
        }
    }

    fn video_status_text(&self) -> Element<'_, Message> {
        use iced::widget::text;
        let label = match &self.selection.video_status {
            VideoProbeStatus::NotSelected => t!("video_status.not_selected"),
            VideoProbeStatus::Checking => t!("video_status.checking"),
            VideoProbeStatus::Ready(info) => t!(
                "video_status.ready",
                width = info.width,
                height = info.height,
                label = info.label
            ),
            VideoProbeStatus::NoSignal => t!("video_status.no_signal"),
            VideoProbeStatus::OpenFailed(error) => {
                t!("video_status.open_failed", error = error)
            }
            VideoProbeStatus::Disconnected => t!("video_status.disconnected"),
        };
        text(label).into()
    }

    fn control_status_text(&self) -> Element<'_, Message> {
        use iced::widget::text;
        let label = match &self.selection.control_status {
            ControlProbeStatus::NotSelected => t!("control_status_label.not_selected"),
            ControlProbeStatus::Checking => t!("control_status_label.checking"),
            ControlProbeStatus::Ready(_) => t!(
                "control_status_label.ready",
                port = self.selection.selected_control_id.as_deref().unwrap_or("?")
            ),
            ControlProbeStatus::NotCh9329(reason) => {
                t!("control_status_label.not_ch9329", reason = reason)
            }
            ControlProbeStatus::NoResponse => t!("control_status_label.no_response"),
            ControlProbeStatus::OpenFailed(error) => {
                t!("control_status_label.open_failed", error = error)
            }
            ControlProbeStatus::Disconnected => t!("control_status_label.disconnected"),
        };
        text(label).into()
    }

    fn video_labels(&self) -> Vec<String> {
        self.selection
            .video_devices
            .iter()
            .map(|device| device.label.clone())
            .collect()
    }

    fn control_labels(&self) -> Vec<String> {
        self.selection
            .control_devices
            .iter()
            .map(|device| device.label.clone())
            .collect()
    }

    fn selected_video_label(&self) -> Option<String> {
        let id = self.selection.selected_video_id.as_deref()?;
        Some(
            self.selection
                .video_devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.label.clone())
                .unwrap_or_default(),
        )
    }

    fn selected_control_label(&self) -> Option<String> {
        let id = self.selection.selected_control_id.as_deref()?;
        Some(
            self.selection
                .control_devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.label.clone())
                .unwrap_or_default(),
        )
    }

    pub fn sync_status(&mut self) {
        self.status = derive_status(
            self.controller.is_control_online(),
            self.controller.input_offline_reason(),
        );
    }

    pub fn subscribed(&self) -> bool {
        self.subscribed
    }

    pub fn status(&self) -> &ConnectionStatus {
        &self.status
    }

    pub fn handle(&self) -> Option<&Handle> {
        self.handle.as_ref()
    }

    pub fn frame_size(&self) -> Option<FrameSize> {
        self.frame_size
    }

    pub fn scale_mode(&self) -> ScaleMode {
        self.scale_mode
    }

    pub fn letterbox_color(&self) -> Color {
        self.letterbox_color
    }

    pub fn frame_source(&self) -> Option<&Arc<MockFrameSource>> {
        self.frame_source.as_ref()
    }
}

/// mock 预览源：open 即返回已有一帧 64×48 的 MockFrameSource。
#[derive(Default)]
struct MockPreviewFactory;

impl PreviewSourceFactory for MockPreviewFactory {
    fn open(&self, _device_id: &str, _fps: u64) -> Result<Arc<dyn FrameSource>, String> {
        let mock = Arc::new(MockFrameSource::new());
        let mut data = vec![0u8; 64 * 48 * 4];
        data[0] = 10;
        data[1] = 20;
        data[2] = 30;
        data[3] = 255;
        let frame = VideoFrame::new(
            1,
            ipkvm_video::MonotonicTimestamp::from_nanos(1),
            64,
            48,
            256,
            ipkvm_video::PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        );
        mock.publish_frame(Arc::new(frame));
        Ok(mock as Arc<dyn FrameSource>)
    }
}

/// ResizeWindowToVideo 模式的期望窗口尺寸；其余模式返回 None。
pub fn desired_window_size(frame: Option<FrameSize>, mode: ScaleMode) -> Option<Size> {
    match (mode, frame) {
        (ScaleMode::ResizeWindowToVideo, Some(f)) => {
            Some(Size::new(f.width as f32, f.height as f32))
        }
        _ => None,
    }
}

fn connect_request() -> ConnectRequest {
    ConnectRequest {
        video_device_id: "mock".into(),
        control_device_id: "mock".into(),
        baud_rate: 9_600,
        mouse_mode: MouseMode::Absolute,
        preview_fps: 30,
    }
}

/// 启动生产 iced 应用（bin 入口调用；测试不启动真实窗口）。
pub fn run() -> iced::Result {
    iced::application(App::production, App::update, App::view)
        .subscription(App::subscription)
        .title(WINDOW_TITLE)
        .window_size(WINDOW_SIZE)
        .run()
}
```

`lib.rs` 导出改为 `pub use app::{App, MockApp, run};`。

`examples/video_1080p.rs` 适配：

```rust
use ipkvm_desktop_iced::{FrameStats, MockApp};
```

并把 `let (app, initial_task) = App::new_mock();` 改为 `MockApp::new_mock()`、`app.frame_source()` 改为 `app.frame_source().expect("mock app").clone()`、`iced::application(move || ..., App::update, App::view)` 改为 `iced::application(move || ..., MockApp::update, MockApp::view)`、`App::subscription` 改为 `MockApp::subscription`、title 闭包参数类型改为 `&MockApp`。

`Cargo.toml` 增加：

```toml
ipkvm-core = { path = "../ipkvm-core", features = ["serial"] }
```

- [x] **Step 6: 追加 app 级测试（先红）**

在 `src/app.rs` 测试模块追加：

```rust
#[test]
fn select_video_then_preview_tick_reaches_ready() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::RefreshDevices);
    let _ = app.update(Message::SelectVideo("Camera 0".into()));
    let _ = app.update(Message::PreviewTick);
    assert!(matches!(
        app.selection.video_status,
        VideoProbeStatus::Ready(info) if info.width == 64 && info.height == 48
    ));
    assert!(app.preview_handle.is_some(), "预览 tick 后必须有预览 Handle");
}

#[test]
fn select_control_reaches_ready_and_can_connect() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::RefreshDevices);
    let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
    assert!(matches!(app.selection.control_status, ControlProbeStatus::Ready(_)));
}

#[test]
fn connect_then_disconnect_transitions() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::RefreshDevices);
    let _ = app.update(Message::SelectVideo("Camera 0".into()));
    let _ = app.update(Message::PreviewTick);
    let _ = app.update(Message::SelectControl("CH9329 (COM9)".into()));
    let _ = app.update(Message::Connect);
    assert_eq!(app.status(), &ConnectionStatus::Connected);
    let _ = app.update(Message::Disconnect);
    assert_eq!(app.status(), &ConnectionStatus::Disconnected);
    assert_eq!(app.selection.control_status, ControlProbeStatus::Disconnected);
}

#[test]
fn save_profile_flow_writes_store() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
    let _ = app.update(Message::Modal(ModalAction::SaveNameChanged("办公室".into())));
    let _ = app.update(Message::Modal(ModalAction::Save));
    assert!(app.store.profile_exists("办公室"));
    assert!(app.modal.open.is_none(), "保存成功后模态必须关闭");
}

#[test]
fn load_profile_applies_selection() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::RefreshDevices);
    let _ = app.update(Message::SelectVideo("Camera 0".into()));
    let _ = app.update(Message::OpenModal(ModalKind::SaveProfile));
    let _ = app.update(Message::Modal(ModalAction::SaveNameChanged("办公室".into())));
    let _ = app.update(Message::Modal(ModalAction::Save));
    // 清空选择再加载。
    app.selection.selected_video_id = None;
    app.selection.selected_control_id = None;
    let _ = app.update(Message::LoadProfile("办公室".into()));
    assert_eq!(app.selection.selected_video_id.as_deref(), Some("cam0"));
    assert_eq!(app.selection.selected_control_id.as_deref(), Some("COM9"));
}

#[test]
fn menu_action_opens_and_closes_modal() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::Menu(MenuAction::OpenModal(ModalKind::Settings)));
    assert_eq!(app.modal.open, Some(ModalKind::Settings));
    let _ = app.update(Message::Modal(ModalAction::Close));
    assert!(app.modal.open.is_none());
}

#[test]
fn locale_switch_updates_zh_flag() {
    let _guard = crate::I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::Menu(MenuAction::SetLanguage(
        crate::menu::LanguageChoice::Chinese,
    )));
    assert!(app.zh);
    let _ = app.update(Message::Menu(MenuAction::SetLanguage(
        crate::menu::LanguageChoice::English,
    )));
    assert!(!app.zh);
}

#[test]
fn settings_fields_update_connection() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::SetBaudRate(115200));
    let _ = app.update(Message::SetAutoBaud(false));
    let _ = app.update(Message::SetPreviewFps(15));
    let _ = app.update(Message::SetMouseMode(MouseMode::Relative));
    assert_eq!(app.connection.baud_rate, 115200);
    assert!(!app.connection.auto_baud);
    assert_eq!(app.connection.preview_fps, 15);
    assert_eq!(app.connection.mouse_mode, MouseMode::Relative);
}
```

把既有 M1 app 测试里的 `App::new_mock()` 改为 `MockApp::new_mock()`。

- [x] **Step 7: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced app::`
Expected: FAIL（`MockApp`/新消息未定义或编译错误）。

- [x] **Step 8: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced app::`
Expected: 13 passed（M1 5 + M2 8）。

- [x] **Step 9: fmt/clippy 修复并跑全量**

Run:
```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy -p ipkvm-desktop-iced --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```
Expected: 全部通过；示例编译（`cargo check -p ipkvm-desktop-iced --example video_1080p`）。

- [x] **Step 10: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/app.rs crates/ipkvm-desktop-iced/src/modal.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/examples/video_1080p.rs crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): integrate menu, modal, connection page and profiles (#76)"
```

## Task 8: 门禁与验收

- [x] **Step 1: 全量门禁**

Run:
```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --all-features --no-deps
```
Expected: 全部通过，0 warning。

- [x] **Step 2: 验收核对（对应 #76 / #82）**
  - [x] spike 2 测试全部移植（modal 4 + menu_interact 8 + corridor 1 + i18n 4 = 17 项，加 modal LoadProfile 1 项）
  - [x] 连接页状态机测试存在（选设备→预览→连接→断开，Task 5/7）
  - [x] profile 保存/加载/最近使用接线测试存在（Task 6/7）
  - [x] 人工观感截图：运行 `cargo run -p ipkvm-desktop-iced`，截图存档到 `docs/superpowers/artifacts/m2-screenshots/`（真实相机预览为硬件项，若无相机记录例外）
  - [x] 回写 #76 验收结论
- [x] **Step 3: 提交文档更新并推送 PR**

```bash
git add docs/superpowers/plans/2026-08-03-iced-migration-m2.md HANDOFF.md
git commit -m "docs: record M2 plan and verification (#76)"
git push -u origin codex/issue76-migration-m2
```

- [x] **Step 4: PR → 自审 → 合并 → 关单**（`Closes #76`）
- [x] **Step 5: 同步 main 并继续 M3**

## Self-Review（计划自审）

- **Spec coverage**：对照 #76 与设计文档 3.2/3.3：自绘菜单 ✅（Task 3）、自绘模态 ✅（Task 2）、连接页（设备下拉/预览/刷新/连接）✅（Task 5/7）、profile 保存/加载/最近使用 ✅（Task 6/7）、i18n ✅（Task 1）、布局对齐（菜单栏/连接页/视频区/状态栏）✅（Task 7）。未覆盖项：真实相机预览冒烟（硬件项，验收列为人工/例外）、rfd 文件对话框（M5）、设置模态完整表单（M2 提供连接页内嵌表单，设置模态保留占位）。
- **Placeholder scan**：无 TBD/占位；`on_press_maybe`、`MockPreviewFactory` 等均为具体代码。
- **Type consistency**：`PreviewRuntime::tick` 返回 `bool`、`apply_profile_to_selection` 返回 `MissingDevices`、`build_profile` 返回 `Profile`、`MockApp`/`ProductionApp` 类型别名跨任务一致；`menu_bar` 签名与 spike 一致；`ModalState::load_names` 由 Task 7 使用。

