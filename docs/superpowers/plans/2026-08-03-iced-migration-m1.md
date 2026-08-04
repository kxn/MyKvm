# M1 视频链路迁移实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 把已验证的 iced 视频链路（帧订阅 → Handle-in-state → 缩放三模式 → 状态栏骨架）从 spike crate 收编进正式 crate `ipkvm-desktop-iced`，并补齐订阅接线、状态流转与字节转换的新测试。

**Architecture:** 在 `ipkvm-desktop-iced` 内新建 `scale`（纯函数缩放数学）、`frames`（watch→Subscription Recipe）、`video`（BGRA→RGBA Handle 转换）、`status`（状态栏状态机）、`app`（状态/消息/视图/订阅）模块；`App` 通过 `DesktopSessionController`（复用 `ipkvm-desktop`）连接 mock 帧源，UI 只消费 `subscribe_frames()`。

**Tech Stack:** iced 0.14（tokio/image/advanced）、iced_test 0.14、tokio、`ipkvm-desktop`（controller）、`ipkvm-video`（FrameSource/MockFrameSource）。

## 执行记录（2026-08-03）

- 分支：`codex/issue75-migration-m1`；基线 `33192e3`。
- 提交链：`3995076`（scale）→ `b771e18`（frames）→ `f29a368`（video）→ `bbe5916`（status）→ `eda1055`（app）→ `1cc98fa`（rustfmt）→ `a59111b`（perf）→ 文档收口提交。
- 门禁：fmt / workspace tests / clippy `-D warnings` / rustdoc `-D warnings` 全部通过；`cargo check --target x86_64-apple-darwin` 因本机无 macOS C 交叉编译器（CC-MISSING）未完成，属交接文档已知预期。
- 冒烟证据：`cargo run -p ipkvm-desktop-iced --example video_1080p -- --duration 10` 生成 `video_1080p.stats.json`（debug 构建：source 77 / rendered 76；性能阈值面向 release，由 `scripts/perf-1080p.ps1` 执行）。
- 执行偏差（均已落码并有测试覆盖）：
  - `iced::window::Id::MAIN` 在 iced 0.14 不存在：App 改用 `window::open_events()` 捕获主窗口 Id（新增 `Message::WindowOpened`），`desired_window_size` 纯函数保持不变。
  - M0 占位 `App`/`Message` 随 app.rs 收编而移除（顺带修复 clippy `default-constructed-unit-structs` 门禁问题）。
  - Task 6 为保留 spike「渲染帧间隔」指标语义，App 增加可选 `stats: Option<Arc<FrameStats>>` 与 `with_stats()`（仅 perf 示例启用，常规运行不记录）。
  - 全量测试首次运行曾出现一次 `ipkvm-desktop-iced --lib` 偶发失败；复跑 3 次及最终全量门禁均通过，未见复现，已记入台账待观察。

## Global Constraints

- iced pin `0.14`，features `["tokio", "image", "advanced"]`（与 M0 一致）。
- 迁移阶段每单必须新增测试（先红后绿），见 #82 与迁移设计文档第 4 节；禁止只靠既有测试变绿。
- 核心 crate（core/session/rfb/video）零平台依赖；本单只改 `ipkvm-desktop-iced`，**不动 egui 端**。
- 布局对齐现有 egui 桌面版；单窗口；黑边颜色可配置。
- 跨平台：不引入 Windows 独占逻辑；尽量跑 `cargo check --target x86_64-apple-darwin`。
- 提交信息英文 conventional commit 并引用 `#75`。
- 全量门禁：fmt / workspace 测试 / clippy -D warnings / rustdoc -D warnings。

---

## 文件结构

- `crates/ipkvm-desktop-iced/src/scale.rs`：缩放纯函数（移植自 spike `src/scale.rs`，原样）。
- `crates/ipkvm-desktop-iced/src/frames.rs`：watch→Subscription Recipe（移植自 spike `src/frames.rs`，原样）。
- `crates/ipkvm-desktop-iced/src/video.rs`：BGRA→RGBA 字节转换 + `image::Handle` 包装（**加强**：字节转换独立纯函数并测试通道交换/stride，spike 只测不 panic）。
- `crates/ipkvm-desktop-iced/src/status.rs`：状态栏状态机（新）：`ConnectionStatus` 枚举 + 从 controller 派生 + 文案映射。
- `crates/ipkvm-desktop-iced/src/app.rs`：`App` 状态/消息/视图/订阅（controller、handle、frame_size、scale_mode、letterbox_color、status）。
- `crates/ipkvm-desktop-iced/src/perf.rs`：`FrameStats`（移植自 spike app.rs，供性能 example 用）。
- `crates/ipkvm-desktop-iced/examples/video_1080p.rs` + `scripts/perf-1080p.ps1`：真实窗口性能复测（移植自 spike）。
- `crates/ipkvm-desktop-iced/src/main.rs`：入口转发到 `app::run()`（现有 lib/bin 拆分不变）。

## 依赖与签名速查（移植时以 spike 源码为准）

- spike `scale.rs`：`ScaleMode`、`Rect::from_min_size(x,y,w,h)`、`FrameSize{width,height}`、`frame_rect(container, frame, mode) -> Rect`、`map_pointer(point, rect, frame) -> Option<(u16,u16)>`。
- spike `frames.rs`：`FrameUpdate::{Frame(SharedVideoFrame), Closed}`、`frame_subscription(id: u64, receiver: FrameReceiver) -> Subscription<FrameUpdate>`。
- `DesktopSessionController`（ipkvm-desktop）：`with_factory(factory)`、`connect(&ConnectRequest)`、`subscribe_frames() -> Option<FrameReceiver>`、`latest_frame()`、`is_control_online()`、`input_offline_reason() -> Option<String>`、`stop()`。
- `ConnectRequest { video_device_id, control_device_id, baud_rate, mouse_mode, preview_fps }`（mock 连接用 "mock"/9600/Absolute/30）。
- `ipkvm_video::VideoFrame { seq, width, height, stride, pixel_format, data }`（BGRA8888）。

---

## Task 1: 移植缩放纯函数 `scale.rs`

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/scale.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod scale;`）

**Interfaces:**
- Produces: `ScaleMode`, `Rect`, `FrameSize`, `frame_rect`, `map_pointer`（签名见速查）。

- [x] **Step 1: 把 `crates/ipkvm-desktop-iced-spike/src/scale.rs` 原样复制为目标文件**（含文件内全部 `#[cfg(test)]` 测试）。
- [x] **Step 2: 在 `lib.rs` 增加 `pub mod scale;`**
- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced scale::`
Expected: 8 passed（fit/actual/dpi250/zero/pointer 等，与 spike 相同用例）。

- [x] **Step 4: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/scale.rs crates/ipkvm-desktop-iced/src/lib.rs
git commit -m "feat(iced): port scale math for video modes (#75)"
```

## Task 2: 移植帧订阅 `frames.rs`

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/frames.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod frames;`）
- Modify: `Cargo.toml`（增加 async-stream / futures-util / iced_futures / ipkvm-video / tokio dev-deps）

**Interfaces:**
- Consumes: `ipkvm_video::{FrameReceiver, SharedVideoFrame}`。
- Produces: `FrameUpdate`, `frame_subscription`（签名见速查）。

- [x] **Step 1: 把 spike `src/frames.rs` 原样复制为目标文件**（含测试；`FrameRecipe` 保持私有）。
- [x] **Step 2: Cargo.toml 补依赖**（与 spike crate 一致）：

```toml
[dependencies]
async-stream = "0.3"
futures-util = { workspace = true }
iced = { version = "0.14", features = ["tokio", "image", "advanced"] }
iced_futures = "0.14"
ipkvm-video = { path = "../ipkvm-video", features = ["mock"] }

[dev-dependencies]
iced_test = "0.14"
ipkvm-video = { path = "../ipkvm-video", features = ["mock"] }
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

- [x] **Step 3: `lib.rs` 增加 `pub mod frames;`**
- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p ipkvm-desktop-iced frames::`
Expected: 1 passed（watch receiver 收到发布帧，seq=7）。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/frames.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): port frame subscription recipe (#75)"
```

## Task 3: BGRA→RGBA 转换与 Handle 包装 `video.rs`（加强版）

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/video.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod video;`）

**Interfaces:**
- Consumes: `ipkvm_video::VideoFrame`、`iced::widget::image::Handle`。
- Produces:
  - `pub fn bgra_to_rgba(frame: &VideoFrame) -> Result<Vec<u8>, String>`（纯字节转换，处理 stride 填充；通道交换 B↔R，A 保持）。
  - `pub fn handle_from_frame(frame: &VideoFrame) -> Handle`（内部调用 bgra_to_rgba，失败返回 1×1 透明 Handle 兜底）。

- [x] **Step 1: 写失败测试**（先红）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};

    fn frame(data: Vec<u8>, width: u32, height: u32, stride: u32) -> VideoFrame {
        VideoFrame::new(1, MonotonicTimestamp::from_nanos(1), width, height, stride,
            PixelFormat::Bgra8888, Arc::from(data.into_boxed_slice()))
    }

    #[test]
    fn bgra_to_rgba_swaps_channels_and_keeps_alpha() {
        let out = bgra_to_rgba(&frame(vec![10, 20, 30, 255], 1, 1, 4)).unwrap();
        assert_eq!(out, vec![30, 20, 10, 255]);
    }

    #[test]
    fn bgra_to_rgba_honors_stride_padding() {
        let out = bgra_to_rgba(&frame(
            vec![0, 1, 2, 255, 9, 9, 9, 9, 3, 4, 5, 255, 8, 8, 8, 8], 1, 2, 8,
        )).unwrap();
        assert_eq!(out, vec![2, 1, 0, 255, 5, 4, 3, 255]);
    }

    #[test]
    fn bgra_to_rgba_rejects_short_data() {
        assert!(bgra_to_rgba(&frame(vec![0, 0, 0], 1, 1, 4)).is_err());
    }

    #[test]
    fn handle_from_frame_never_panics_on_bad_frame() {
        let bad = frame(vec![0; 3], 1, 1, 4);
        let _ = handle_from_frame(&bad);
    }
}
```

- [x] **Step 2: 运行确认失败**

Run: `cargo test -p ipkvm-desktop-iced video::`
Expected: FAIL（`bgra_to_rgba` 未定义）。

- [x] **Step 3: 实现**

```rust
//! 视频帧字节转换：BGRA8888 → RGBA8888，处理 stride 填充，并包装成 iced Handle。

use iced::widget::image::Handle;
use ipkvm_video::VideoFrame;

pub fn bgra_to_rgba(frame: &VideoFrame) -> Result<Vec<u8>, String> {
    if frame.pixel_format != ipkvm_video::PixelFormat::Bgra8888 {
        return Err(format!("unsupported pixel format: {:?}", frame.pixel_format));
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let Some(required) = stride
        .checked_mul(height.saturating_sub(1))
        .and_then(|rows| rows.checked_add(width * 4))
    else {
        return Err("frame stride or size overflow".into());
    };
    if frame.data.len() < required {
        return Err(format!("frame data too short: need {required}, got {}", frame.data.len()));
    }
    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = &frame.data[y * stride..y * stride + width * 4];
        let dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
        for (rgba, bgra) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    Ok(pixels)
}

pub fn handle_from_frame(frame: &VideoFrame) -> Handle {
    match bgra_to_rgba(frame) {
        Ok(pixels) => Handle::from_rgba(frame.width, frame.height, pixels),
        Err(_) => Handle::from_rgba(1, 1, vec![0, 0, 0, 0]),
    }
}
```

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced video::`
Expected: 4 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/video.rs crates/ipkvm-desktop-iced/src/lib.rs
git commit -m "feat(iced): bgra to rgba conversion with stride tests (#75)"
```

## Task 4: 状态栏状态机 `status.rs`

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/status.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod status;`）

**Interfaces:**
- Produces:
  - `pub enum ConnectionStatus { Disconnected, Connecting, Connected, ControlOffline(String) }`
  - `pub fn derive_status(connected: bool, offline_reason: Option<String>) -> ConnectionStatus`
  - `impl ConnectionStatus { pub fn label(&self, zh: bool) -> String }`

- [x] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_when_not_connected() {
        assert_eq!(derive_status(false, None), ConnectionStatus::Disconnected);
    }

    #[test]
    fn connected_when_online_without_reason() {
        assert_eq!(derive_status(true, None), ConnectionStatus::Connected);
    }

    #[test]
    fn control_offline_carries_reason() {
        assert_eq!(
            derive_status(true, Some("serial write failed".into())),
            ConnectionStatus::ControlOffline("serial write failed".into())
        );
    }

    #[test]
    fn labels_are_nonempty_and_distinct() {
        for zh in [true, false] {
            for s in [
                ConnectionStatus::Disconnected,
                ConnectionStatus::Connecting,
                ConnectionStatus::Connected,
                ConnectionStatus::ControlOffline("x".into()),
            ] {
                assert!(!s.label(zh).is_empty());
            }
        }
        assert_ne!(ConnectionStatus::Connected.label(true), ConnectionStatus::Connected.label(false));
    }
}
```

- [x] **Step 2: 运行确认失败**（`derive_status` 未定义）
- [x] **Step 3: 实现**

```rust
//! 状态栏状态机（M1 骨架）：连接/在线/控制离线三态 + 文案。

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    ControlOffline(String),
}

pub fn derive_status(connected: bool, offline_reason: Option<String>) -> ConnectionStatus {
    match (connected, offline_reason) {
        (false, _) => ConnectionStatus::Disconnected,
        (true, Some(reason)) => ConnectionStatus::ControlOffline(reason),
        (true, None) => ConnectionStatus::Connected,
    }
}

impl ConnectionStatus {
    pub fn label(&self, zh: bool) -> String {
        match (self, zh) {
            (Self::Disconnected, true) => "未连接".into(),
            (Self::Disconnected, false) => "Disconnected".into(),
            (Self::Connecting, true) => "连接中".into(),
            (Self::Connecting, false) => "Connecting".into(),
            (Self::Connected, true) => "已连接".into(),
            (Self::Connected, false) => "Connected".into(),
            (Self::ControlOffline(reason), true) => format!("控制设备离线：{reason}"),
            (Self::ControlOffline(reason), false) => format!("Control offline: {reason}"),
        }
    }
}
```

- [x] **Step 4: 运行确认通过**（4 passed）
- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/status.rs crates/ipkvm-desktop-iced/src/lib.rs
git commit -m "feat(iced): status bar state machine skeleton (#75)"
```

## Task 5: App 状态/消息/订阅 `app.rs`

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/app.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod app;` + `pub use app::run;`）
- Modify: `crates/ipkvm-desktop-iced/src/main.rs`（run 转发到 app，入口不变）
- Modify: `Cargo.toml`（增加 ipkvm-core / ipkvm-desktop / ipkvm-session 依赖）

**Interfaces:**
- Consumes: `frames::frame_subscription`、`video::handle_from_frame`、`scale::{ScaleMode, FrameSize, Rect}`、`status::{derive_status, ConnectionStatus}`、`ipkvm_desktop::{ConnectRequest, DesktopSessionController, DesktopSessionError, SessionParts}`、`ipkvm_core::InputSink`。
- Produces:
  - `pub enum Message { FrameReady(VideoFrame), FrameClosed, SetScaleMode(ScaleMode), SetLetterboxColor(Color), ToggleLocale }`
  - `pub struct App`（`controller` 为 `pub(crate)`，测试可访问）；`pub fn run() -> iced::Result`。
  - `pub fn desired_window_size(frame: Option<FrameSize>, mode: ScaleMode) -> Option<Size>`
  - `impl App { update/view/subscription/new_mock/status/handle/frame_size/scale_mode/letterbox_color/subscribed/sync_status }`

- [x] **Step 1: 写失败测试**（控制器 + 状态 + 订阅标志 + 窗口尺寸决策）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use iced::widget::image::Handle;
    use ipkvm_core::{InputError, InputSink, KeyEvent, MouseMode, PointerEvent};
    use ipkvm_video::mock::MockFrameSource;
    use ipkvm_video::{FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame};
    use ipkvm_session::rfb_connection::RfbConnectionGate;

    #[derive(Clone, Debug, Default)]
    struct RecordingSink { pub key_batches: Arc<std::sync::Mutex<usize>> }
    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _m: MouseMode) -> Result<(), InputError> { Ok(()) }
        fn handle_key_batch(&mut self, _e: &[KeyEvent]) -> Result<(), InputError> {
            *self.key_batches.lock().unwrap() += 1; Ok(())
        }
        fn handle_pointer_batch(&mut self, _e: &[PointerEvent]) -> Result<(), InputError> { Ok(()) }
        fn release_all(&mut self) -> Result<(), InputError> { Ok(()) }
    }

    type MockFactory = Box<dyn FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>>;
    type MockController = DesktopSessionController<RecordingSink, MockFactory>;

    fn make_bgra_frame(seq: u64, w: u32, h: u32) -> VideoFrame {
        let mut data = vec![0u8; (w * h * 4) as usize];
        data[0] = 10; data[1] = 20; data[2] = 30; data[3] = 255;
        VideoFrame::new(seq, MonotonicTimestamp::from_nanos(seq), w, h, w * 4,
            PixelFormat::Bgra8888, Arc::from(data.into_boxed_slice()))
    }

    #[test]
    fn frame_ready_stores_handle_and_frame_size_and_status_connected() {
        let (mut app, _) = App::new_mock();
        let _ = app.update(Message::FrameReady(make_bgra_frame(1, 320, 240)));
        assert!(app.handle().is_some(), "FrameReady 后 Handle 必须存 state");
        assert_eq!(app.frame_size(), Some(FrameSize { width: 320, height: 240 }));
        assert_eq!(app.status(), &ConnectionStatus::Connected);
    }

    #[test]
    fn frame_closed_stops_subscription() {
        let (mut app, _) = App::new_mock();
        assert!(app.subscribed());
        let _ = app.update(Message::FrameClosed);
        assert!(!app.subscribed(), "FrameClosed 后订阅必须停");
        assert!(app.subscription().is_none() == false || !app.subscribed());
    }

    #[test]
    fn scale_mode_and_letterbox_transitions() {
        let (mut app, _) = App::new_mock();
        let _ = app.update(Message::SetScaleMode(ScaleMode::ActualSize));
        assert_eq!(app.scale_mode(), ScaleMode::ActualSize);
        let color = iced::Color::from_rgb(0.1, 0.2, 0.3);
        let _ = app.update(Message::SetLetterboxColor(color));
        assert_eq!(app.letterbox_color(), color);
    }

    #[test]
    fn desired_window_size_only_for_resize_mode() {
        let frame = Some(FrameSize { width: 1920, height: 1080 });
        assert_eq!(desired_window_size(frame, ScaleMode::ResizeWindowToVideo), Some(Size::new(1920.0, 1080.0)));
        assert_eq!(desired_window_size(frame, ScaleMode::FitWindow), None);
        assert_eq!(desired_window_size(None, ScaleMode::ResizeWindowToVideo), None);
    }

    #[test]
    fn stop_session_derives_disconnected_status() {
        let (mut app, _) = App::new_mock();
        app.controller.stop().unwrap();
        app.sync_status();
        assert_eq!(app.status(), &ConnectionStatus::Disconnected);
    }
}
```

- [x] **Step 2: 运行确认失败**（`App` 未定义）
- [x] **Step 3: 实现 app.rs**（工厂/连接样板与 spike `app.rs` 一致；核心代码）

```rust
pub struct App {
    pub(crate) controller: MockController,
    frame_source: Arc<MockFrameSource>,
    handle: Option<Handle>,
    frame_size: Option<FrameSize>,
    scale_mode: ScaleMode,
    letterbox_color: Color,
    status: ConnectionStatus,
    subscribed: bool,
    zh: bool,
}

impl App {
    pub fn new_mock() -> (Self, Task<Message>) {
        let frame_source = Arc::new(MockFrameSource::new());
        let fs = Arc::clone(&frame_source);
        let factory: MockFactory = Box::new(move |_req| {
            let src: Arc<dyn FrameSource> = fs.clone();
            Ok((src, RecordingSink::default(), RfbConnectionGate::new()))
        });
        let mut controller = DesktopSessionController::with_factory(factory);
        controller.connect(connect_request()).expect("mock connect");
        let status = derive_status(controller.is_control_online(), controller.input_offline_reason());
        (Self {
            controller, frame_source, handle: None, frame_size: None,
            scale_mode: ScaleMode::FitWindow,
            letterbox_color: Color::from_rgb(0.0, 0.0, 0.0),
            status, subscribed: true, zh: true,
        }, Task::none())
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FrameReady(frame) => {
                self.handle = Some(handle_from_frame(&frame));
                self.frame_size = Some(FrameSize { width: frame.width, height: frame.height });
                self.sync_status();
                if self.scale_mode == ScaleMode::ResizeWindowToVideo {
                    if let Some(size) = desired_window_size(self.frame_size, self.scale_mode) {
                        return iced::window::resize(iced::window::Id::MAIN, size);
                    }
                }
                Task::none()
            }
            Message::FrameClosed => { self.subscribed = false; Task::none() }
            Message::SetScaleMode(mode) => { self.scale_mode = mode; Task::none() }
            Message::SetLetterboxColor(color) => { self.letterbox_color = color; Task::none() }
            Message::ToggleLocale => { self.zh = !self.zh; Task::none() }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if !self.subscribed { return Subscription::none(); }
        self.controller.subscribe_frames()
            .map(|receiver| frame_subscription(0, receiver).map(|u| match u {
                FrameUpdate::Frame(f) => Message::FrameReady((*f).clone()),
                FrameUpdate::Closed => Message::FrameClosed,
            }))
            .unwrap_or_else(Subscription::none)
    }

    pub fn view(&self) -> Element<'_, Message> {
        use iced::widget::{column, container, image, text};
        let video = match self.handle.as_ref() {
            Some(handle) => image::Image::new(handle.clone()).content_fit(iced::ContentFit::Contain),
            None => text("等待帧…"),
        };
        let video_area = container(video)
            .width(Length::Fill).height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(self.letterbox_color.into()),
                ..Default::default()
            });
        let status_line = container(text(self.status.label(self.zh)))
            .width(Length::Fill).padding(6);
        column![video_area, status_line].into()
    }

    pub fn sync_status(&mut self) {
        self.status = derive_status(
            self.controller.is_control_online(),
            self.controller.input_offline_reason(),
        );
    }

    pub fn subscribed(&self) -> bool { self.subscribed }
    pub fn status(&self) -> &ConnectionStatus { &self.status }
    pub fn handle(&self) -> Option<&Handle> { self.handle.as_ref() }
    pub fn frame_size(&self) -> Option<FrameSize> { self.frame_size }
    pub fn scale_mode(&self) -> ScaleMode { self.scale_mode }
    pub fn letterbox_color(&self) -> Color { self.letterbox_color }
    pub fn frame_source(&self) -> &Arc<MockFrameSource> { &self.frame_source }
}

pub fn desired_window_size(frame: Option<FrameSize>, mode: ScaleMode) -> Option<Size> {
    match (mode, frame) {
        (ScaleMode::ResizeWindowToVideo, Some(f)) => {
            Some(Size::new(f.width as f32, f.height as f32))
        }
        _ => None,
    }
}

pub fn run() -> iced::Result {
    iced::application(App::new_mock, App::update, App::view)
        .subscription(App::subscription)
        .title(WINDOW_TITLE)
        .window_size(WINDOW_SIZE)
        .run()
}
```

> 注：`iced::window::Id::MAIN` 以 0.14 实际 API 为准（若为 `Id::MAIN` 不存在则用窗口 builder 的默认 id 或 `iced::window::Id::unique()` 变体，编译期修正并保持 `desired_window_size` 纯函数不变）。`frame_closed_stops_subscription` 中 `subscription().is_none()` 表达式仅为探针，若编译不过直接删除该行、保留 `subscribed()` 断言。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced app::`
Expected: 5 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/app.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/src/main.rs crates/ipkvm-desktop-iced/Cargo.toml Cargo.lock
git commit -m "feat(iced): app state, messages and frame subscription wiring (#75)"
```

## Task 6: 真实窗口性能示例与脚本

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/perf.rs`（`FrameStats`，移植自 spike app.rs）
- Create: `crates/ipkvm-desktop-iced/examples/video_1080p.rs`（移植自 spike 同名 example，改用 `App::new_mock`，保留 `--duration/--stats-file`）
- Create: `crates/ipkvm-desktop-iced/scripts/perf-1080p.ps1`（移植自 spike 脚本，crate 名替换）

**Interfaces:**
- Consumes: `App::frame_source()`、`FrameStats`。
- Produces: `FrameStats { new() -> Arc<Self>, record_at(Instant), summary() -> (u64, f64, f64) }`。

- [x] **Step 1: 写 FrameStats 失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn summary_computes_avg_and_p95() {
        let stats = FrameStats::new();
        let t0 = std::time::Instant::now();
        stats.record_at(t0);
        stats.record_at(t0 + Duration::from_millis(10));
        stats.record_at(t0 + Duration::from_millis(40));
        let (n, avg, p95) = stats.summary();
        assert_eq!(n, 3);
        assert!((avg - 20.0).abs() < 0.01);
        assert!((p95 - 30.0).abs() < 0.01);
    }

    #[test]
    fn empty_stats_returns_zero() {
        let stats = FrameStats::new();
        let (n, avg, p95) = stats.summary();
        assert_eq!((n, avg, p95), (0, 0.0, 0.0));
    }
}
```

- [x] **Step 2: 运行确认失败**（`FrameStats` 未定义）
- [x] **Step 3: 实现 FrameStats（算法同 spike）+ 移植 example 与脚本**
- [x] **Step 4: 运行快速冒烟**

Run: `cargo run -p ipkvm-desktop-iced --example video_1080p -- --duration 10`
Expected: 窗口显示 10 秒后退出，生成 `video_1080p.stats.json`。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/perf.rs crates/ipkvm-desktop-iced/examples crates/ipkvm-desktop-iced/scripts crates/ipkvm-desktop-iced/Cargo.toml
git commit -m "feat(iced): perf example and stats for video regression (#75)"
```

## Task 7: 门禁与验收

- [x] **Step 1: 全量门禁**

Run:
```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --all-features --no-deps
```
Expected: 全部通过，0 warning。

- [x] **Step 2: 验收核对（对应 #75 / #82）**
  - [x] spike 1 测试全部移植（scale 8 + frames 1 + app 5 + video 4 + status 4 + perf 2 = 24 项）
  - [x] 订阅接线测试存在（FrameReady→Handle、FrameClosed→停订阅）
  - [x] 状态栏状态流转测试存在（Connected/Disconnected/ControlOffline）
  - [x] 真实窗口 10s 冒烟有 stats.json 证据
  - [x] 回写 #75 验收结论
- [x] **Step 3: 提交文档更新并推送 PR**

```bash
git add docs/superpowers/plans/2026-08-03-iced-migration-m1.md
git commit -m "docs: record M1 plan and verification (#75)"
git push -u origin codex/issue75-migration-m1
```

- [x] **Step 4: PR → 自审 → 合并 → 关单**（`Closes #75`）
- [x] **Step 5: 同步 main 并继续 M2**

## Self-Review（计划自审）

- **Spec coverage**：对照 #75 与设计文档 3.4：帧订阅 ✅（Task 2/5）、Handle-in-state ✅（Task 3/5）、缩放三模式 ✅（Task 1/5）、黑边颜色可配置 ✅（Task 5）、状态栏骨架 ✅（Task 4/5）、spike 1 指标回归 ✅（Task 6/7）、#82 新增测试清单 ✅（Task 3/4/5/6）。未覆盖项：真实相机冒烟（属 M2 连接页范围）、perf 120s 全量（Task 6 提供 10s 快速版，脚本支持 120s）。
- **Placeholder scan**：无 TBD/占位；Task 5 的 `Id::MAIN` 说明是 API 兼容性提示，非占位。
- **Type consistency**：`FrameUpdate`/`frame_subscription`/`handle_from_frame`/`derive_status`/`desired_window_size` 跨任务签名一致；`FrameStats::record_at`/`summary` 在 Task 6 定义并被 example 使用。

