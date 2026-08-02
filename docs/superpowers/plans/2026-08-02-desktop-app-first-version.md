# 桌面 app 第一版实施计划

> **给自动化协作者：** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 按任务实施。本计划步骤使用 checkbox（`- [ ]`）记录进度。

**目标：** 做出第一版可真实选择视频设备和 CH9329 控制设备、可预览、可连接、可发送输入、可重新选设备的本地桌面 KVM app。

**架构：** desktop 不依赖 headless，也不走本机 HTTP/RFB 回环；窗口事件直接转成 `RfbServerEvent` 喂给共享 `SessionManager`/`RfbInputPump`。第一版 UI 使用 `eframe/egui` 的 wgpu 后端，而不是手写 `pixels + winit` 控件，因为本轮有设备 dialog、菜单、状态栏、剪贴板、文件保存等典型 GUI 控件；视频帧作为 egui 纹理更新，输入坐标由独立渲染模型换算。

**技术栈：** Rust 1.89、eframe/egui 0.33.2、arboard 3.6.1、Windows gated rfd 0.15.4、tokio、serialport、ipkvm-core、ipkvm-video、ipkvm-session。

## 全局约束

- 仓库内自写文档使用中文；代码标识符、协议字段和第三方专有名词保留原文。
- 本计划围绕 Gitea issue #33；PR 描述必须链接 #33，并说明从原 issue 标题的 `wgpu+pixels` 调整为 `eframe/egui(wgpu)` 的原因。
- 不做自动恢复上次设备并连接；最多后续记住上次选择并预选，仍需用户点击连接。
- 不提供强制连接非法控制设备；串口必须通过 CH9329 `GetInfo` 探测后才能连接。
- 主流程只显示视频设备、控制设备、刷新检测、连接和高级入口；波特率、鼠标模式、视频格式策略放入高级。
- 连接按钮只有在视频预览可用且控制设备探测为合法 CH9329 时启用。
- 切换设备采用会话级停旧启新：释放输入、停止旧会话、释放旧硬件、创建新会话。
- 连接后画面优先；低频动作放入可呼出的控制菜单；底部状态栏显示控制设备、键盘输入、鼠标坐标和视频状态。
- desktop 第一版要处理自身视频断流、分辨率变化和 CH9329 掉线后的 UI/输入安全状态。
- headless 的视频断流与 CH9329 掉线自动恢复是产品要求，但不是本计划代码范围；本计划完成后必须拆独立 issue/计划，不能在 PR 中声明 headless 恢复已完成。
- 新增依赖必须通过 `.\scripts\test-license-policy.ps1` 和 `.\scripts\verify-licenses.ps1`；不通过时先收敛依赖，不用例外绕过。

---

## 文件结构

- 修改：`Cargo.toml`  
  增加 desktop 直接依赖的 workspace 版本：`eframe`、`arboard`。
- 修改：`crates/ipkvm-desktop/Cargo.toml`  
  接入 GUI、剪贴板、视频、串口、session、tokio runtime；Windows target 下接入 `rfd` 保存对话框。
- 修改：`crates/ipkvm-desktop/src/main.rs`  
  从 scaffold 改为调用 `ipkvm_desktop::run()`。
- 新建：`crates/ipkvm-desktop/src/lib.rs`  
  模块入口和 `run()`。
- 新建：`crates/ipkvm-desktop/src/state.rs`  
  设备选择状态、检测状态、高级配置、连接按钮 gating。
- 新建：`crates/ipkvm-desktop/src/probe.rs`  
  设备枚举、视频预览、CH9329 探测；生产 backend 与 fake backend 测试。
- 新建：`crates/ipkvm-desktop/src/frame.rs`  
  BGRA→RGBA 转换、视频尺寸、预览帧/渲染帧数据。
- 新建：`crates/ipkvm-desktop/src/render.rs`  
  画面缩放模式、aspect-fit 计算、窗口坐标到 framebuffer 坐标映射。
- 新建：`crates/ipkvm-desktop/src/session.rs`  
  桌面会话控制器：tokio runtime、`SessionManager`、本地 client 连接、重连、停止、notice 接收。
- 新建：`crates/ipkvm-desktop/src/input.rs`  
  egui 键鼠事件到 RFB keysym/pointer 事件的转换；特殊键序列。
- 新建：`crates/ipkvm-desktop/src/clipboard.rs`  
  文本粘贴、截图复制到剪贴板、JPEG 保存。
- 新建：`crates/ipkvm-desktop/src/app.rs`  
  `eframe::App` 实现：设备 dialog、控制台、菜单、状态栏、状态轮询。
- 修改：`crates/ipkvm-session/src/rfb_connection/mod.rs`  
  增加桌面本地控制器专用 `RfbClientId` 构造器。
- 修改：`crates/ipkvm-session/src/console_session.rs`  
  增加可选 notice mirror，供 desktop 知道文本粘贴完成、输入被拒绝和控制器释放。
- 修改：`crates/ipkvm-session/src/session_manager.rs`  
  保持 notice mirror 在 `create`/`replace_and_start` 后仍接到新会话。
- 修改：`README.md`  
  增加 desktop 第一版使用入口和真实硬件验证步骤。

---

### 任务 1：接入 GUI 依赖与 desktop 入口

**文件：**
- 修改：`Cargo.toml`
- 修改：`crates/ipkvm-desktop/Cargo.toml`
- 修改：`crates/ipkvm-desktop/src/main.rs`
- 新建：`crates/ipkvm-desktop/src/lib.rs`

**接口：**
- 产出：`pub fn run() -> Result<(), DesktopError>`
- 产出：`DesktopError::Gui(String)`

- [x] **步骤 1：修改 workspace 依赖**

在 `Cargo.toml` 的 `[workspace.dependencies]` 中加入：

```toml
arboard = "3.6.1"
eframe = { version = "0.33.2", default-features = false, features = ["default_fonts", "wgpu", "x11", "wayland"] }
```

- [x] **步骤 2：修改 desktop crate 依赖**

把 `crates/ipkvm-desktop/Cargo.toml` 的依赖改成：

```toml
[dependencies]
arboard.workspace = true
eframe.workspace = true
ipkvm-core = { path = "../ipkvm-core", features = ["serial"] }
ipkvm-session = { path = "../ipkvm-session", features = ["serial"] }
ipkvm-video = { path = "../ipkvm-video", features = ["mf"] }
jpeg-encoder.workspace = true
serialport.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["rt-multi-thread", "sync", "time"] }

[target.'cfg(windows)'.dependencies]
rfd = { version = "0.15.4", default-features = false, features = ["common-controls-v6"] }
```

- [x] **步骤 3：建立模块入口**

`crates/ipkvm-desktop/src/lib.rs` 写入：

```rust
mod app;
mod clipboard;
mod frame;
mod input;
mod probe;
mod render;
mod session;
mod state;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("desktop gui failed: {0}")]
    Gui(String),
}

pub fn run() -> Result<(), DesktopError> {
    app::run().map_err(|error| DesktopError::Gui(error.to_string()))
}
```

`crates/ipkvm-desktop/src/main.rs` 改为：

```rust
fn main() {
    if let Err(error) = ipkvm_desktop::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
```

- [x] **步骤 4：先放最小 app 壳**

`crates/ipkvm-desktop/src/app.rs` 先写最小可编译实现：

```rust
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "my_ipkvm",
        options,
        Box::new(|_cc| Ok(Box::<DesktopApp>::default())),
    )
}

#[derive(Default)]
struct DesktopApp;

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("my_ipkvm");
        });
    }
}
```

- [x] **步骤 5：验证依赖和入口**

Run:

```powershell
cargo check -p ipkvm-desktop --all-features
.\scripts\test-license-policy.ps1
.\scripts\verify-licenses.ps1
```

Expected: 全部 PASS。若许可证检查失败，先移除或替换新依赖，不进入后续任务。

- [x] **步骤 6：提交**

```powershell
git add Cargo.toml crates/ipkvm-desktop
git commit -m "feat: add desktop gui shell"
```

### 任务 2：设备选择状态机与连接 gating

**文件：**
- 新建：`crates/ipkvm-desktop/src/state.rs`
- 修改：`crates/ipkvm-desktop/src/lib.rs`

**接口：**
- 产出：`DeviceSelectionState::can_connect(&self) -> bool`
- 产出：`DeviceSelectionState::refresh_devices(&mut self, video: Vec<DeviceOption>, control: Vec<DeviceOption>)`
- 产出：`AdvancedSettings`

- [x] **步骤 1：编写状态模型红灯测试**

在 `state.rs` 中先写测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, label: &str) -> DeviceOption {
        DeviceOption {
            id: id.to_owned(),
            label: label.to_owned(),
        }
    }

    #[test]
    fn connect_requires_video_ready_and_control_ready() {
        let mut state = DeviceSelectionState::default();
        assert!(!state.can_connect());

        state.video_status = VideoProbeStatus::Ready(PreviewInfo {
            width: 1920,
            height: 1080,
            label: "capture".into(),
        });
        state.control_status = ControlProbeStatus::NoResponse;
        assert!(!state.can_connect());

        state.control_status = ControlProbeStatus::Ready(ControlInfo {
            version: 0x31,
            usb_enumerated: true,
        });
        assert!(state.can_connect());
    }

    #[test]
    fn refresh_marks_missing_selected_devices_disconnected() {
        let mut state = DeviceSelectionState::default();
        state.video_devices = vec![option("cam0", "Camera 0")];
        state.control_devices = vec![option("COM9", "COM9")];
        state.selected_video_id = Some("cam0".into());
        state.selected_control_id = Some("COM9".into());
        state.video_status = VideoProbeStatus::Ready(PreviewInfo {
            width: 640,
            height: 480,
            label: "Camera 0".into(),
        });
        state.control_status = ControlProbeStatus::Ready(ControlInfo {
            version: 0x31,
            usb_enumerated: true,
        });

        state.refresh_devices(Vec::new(), Vec::new());

        assert_eq!(state.video_status, VideoProbeStatus::Disconnected);
        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
        assert!(!state.can_connect());
    }
}
```

- [x] **步骤 2：实现状态模型**

在测试上方实现：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewInfo {
    pub width: u32,
    pub height: u32,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlInfo {
    pub version: u8,
    pub usb_enumerated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VideoProbeStatus {
    NotSelected,
    Checking,
    Ready(PreviewInfo),
    NoSignal,
    OpenFailed(String),
    Disconnected,
}

impl Default for VideoProbeStatus {
    fn default() -> Self {
        Self::NotSelected
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlProbeStatus {
    NotSelected,
    Checking,
    Ready(ControlInfo),
    NotCh9329(String),
    NoResponse,
    OpenFailed(String),
    Disconnected,
}

impl Default for ControlProbeStatus {
    fn default() -> Self {
        Self::NotSelected
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoScaleMode {
    FitWindow,
    ActualSize,
    ResizeWindowToVideo,
}

impl Default for VideoScaleMode {
    fn default() -> Self {
        Self::FitWindow
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvancedSettings {
    pub baud_rate: u32,
    pub mouse_mode: ipkvm_core::MouseMode,
    pub preview_fps: u64,
    pub scale_mode: VideoScaleMode,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            baud_rate: ipkvm_core::DEFAULT_BAUD_RATE,
            mouse_mode: ipkvm_core::MouseMode::Absolute,
            preview_fps: 30,
            scale_mode: VideoScaleMode::FitWindow,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DeviceSelectionState {
    pub video_devices: Vec<DeviceOption>,
    pub control_devices: Vec<DeviceOption>,
    pub selected_video_id: Option<String>,
    pub selected_control_id: Option<String>,
    pub video_status: VideoProbeStatus,
    pub control_status: ControlProbeStatus,
    pub advanced: AdvancedSettings,
}

impl DeviceSelectionState {
    pub fn can_connect(&self) -> bool {
        matches!(self.video_status, VideoProbeStatus::Ready(_))
            && matches!(self.control_status, ControlProbeStatus::Ready(_))
    }

    pub fn refresh_devices(&mut self, video: Vec<DeviceOption>, control: Vec<DeviceOption>) {
        let selected_video_missing = self
            .selected_video_id
            .as_ref()
            .is_some_and(|id| !video.iter().any(|device| device.id == *id));
        let selected_control_missing = self
            .selected_control_id
            .as_ref()
            .is_some_and(|id| !control.iter().any(|device| device.id == *id));

        self.video_devices = video;
        self.control_devices = control;

        if selected_video_missing {
            self.video_status = VideoProbeStatus::Disconnected;
        }
        if selected_control_missing {
            self.control_status = ControlProbeStatus::Disconnected;
        }
    }
}
```

- [x] **步骤 3：验证**

Run:

```powershell
cargo test -p ipkvm-desktop --all-features state::tests -- --nocapture
```

Expected: PASS。

- [x] **步骤 4：提交**

```powershell
git add crates/ipkvm-desktop/src/state.rs crates/ipkvm-desktop/src/lib.rs
git commit -m "feat: add desktop device selection state"
```

### 任务 3：设备枚举、视频预览与 CH9329 探测

**文件：**
- 新建：`crates/ipkvm-desktop/src/probe.rs`
- 新建：`crates/ipkvm-desktop/src/frame.rs`
- 修改：`crates/ipkvm-desktop/src/lib.rs`

**接口：**
- 产出：`ProbeBackend`
- 产出：`ProductionProbeBackend`
- 产出：`probe_ch9329(path: &str, baud_rate: u32, timeout: Duration) -> ControlProbeStatus`
- 产出：`capture_preview(device_id: &str, fps: u64, timeout: Duration) -> VideoPreviewResult`

- [x] **步骤 1：编写 fake backend 测试**

在 `probe.rs` 写测试，先锁定“刷新检测同时刷新两类设备并重探当前选择”：

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::state::{
        ControlInfo, ControlProbeStatus, DeviceSelectionState, PreviewInfo, VideoProbeStatus,
    };

    #[derive(Default)]
    struct FakeBackend {
        video_calls: usize,
        control_calls: usize,
    }

    impl ProbeBackend for FakeBackend {
        fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
            Ok(vec![DeviceOption {
                id: "cam0".into(),
                label: "Camera 0".into(),
            }])
        }

        fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
            Ok(vec![DeviceOption {
                id: "COM9".into(),
                label: "COM9".into(),
            }])
        }

        fn preview_video(
            &mut self,
            device_id: &str,
            _fps: u64,
            _timeout: Duration,
        ) -> VideoProbeStatus {
            self.video_calls += 1;
            assert_eq!(device_id, "cam0");
            VideoProbeStatus::Ready(PreviewInfo {
                width: 1920,
                height: 1080,
                label: "Camera 0".into(),
            })
        }

        fn probe_control(
            &mut self,
            device_id: &str,
            _baud_rate: u32,
            _timeout: Duration,
        ) -> ControlProbeStatus {
            self.control_calls += 1;
            assert_eq!(device_id, "COM9");
            ControlProbeStatus::Ready(ControlInfo {
                version: 0x31,
                usb_enumerated: true,
            })
        }
    }

    #[test]
    fn refresh_detection_lists_devices_and_rechecks_selected_devices() {
        let mut backend = FakeBackend::default();
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            ..DeviceSelectionState::default()
        };

        refresh_detection(&mut state, &mut backend, Duration::from_millis(10));

        assert_eq!(backend.video_calls, 1);
        assert_eq!(backend.control_calls, 1);
        assert!(matches!(state.video_status, VideoProbeStatus::Ready(_)));
        assert!(matches!(state.control_status, ControlProbeStatus::Ready(_)));
        assert!(state.can_connect());
    }
}
```

- [x] **步骤 2：实现 probe trait 与 refresh**

```rust
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use ipkvm_core::{Ch9329Command, Ch9329Decoder, Ch9329Response};

use crate::state::{
    ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState, PreviewInfo,
    VideoProbeStatus,
};

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("video device list failed: {0}")]
    VideoList(String),
    #[error("control device list failed: {0}")]
    ControlList(String),
}

pub trait ProbeBackend {
    fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError>;
    fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError>;
    fn preview_video(&mut self, device_id: &str, fps: u64, timeout: Duration)
        -> VideoProbeStatus;
    fn probe_control(
        &mut self,
        device_id: &str,
        baud_rate: u32,
        timeout: Duration,
    ) -> ControlProbeStatus;
}

pub fn refresh_detection(
    state: &mut DeviceSelectionState,
    backend: &mut impl ProbeBackend,
    timeout: Duration,
) {
    let video = backend.list_video_devices().unwrap_or_default();
    let control = backend.list_control_devices().unwrap_or_default();
    state.refresh_devices(video, control);

    if let Some(device_id) = state.selected_video_id.clone() {
        state.video_status =
            backend.preview_video(&device_id, state.advanced.preview_fps, timeout);
    }
    if let Some(device_id) = state.selected_control_id.clone() {
        state.control_status =
            backend.probe_control(&device_id, state.advanced.baud_rate, timeout);
    }
}
```

- [x] **步骤 3：实现生产 backend**

生产枚举映射使用现有共享库：

```rust
pub struct ProductionProbeBackend;

impl ProbeBackend for ProductionProbeBackend {
    fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
        ipkvm_session::devices::list_video_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|device| DeviceOption {
                        id: device.id,
                        label: device.display_name,
                    })
                    .collect()
            })
            .map_err(|error| ProbeError::VideoList(error.to_string()))
    }

    fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
        ipkvm_session::devices::list_serial_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|device| DeviceOption {
                        id: device.path.clone(),
                        label: format!("{} ({})", device.path, device.port_type),
                    })
                    .collect()
            })
            .map_err(|error| ProbeError::ControlList(error.to_string()))
    }

    fn preview_video(
        &mut self,
        device_id: &str,
        fps: u64,
        timeout: Duration,
    ) -> VideoProbeStatus {
        capture_preview(device_id, fps, timeout)
    }

    fn probe_control(
        &mut self,
        device_id: &str,
        baud_rate: u32,
        timeout: Duration,
    ) -> ControlProbeStatus {
        probe_ch9329(device_id, baud_rate, timeout)
    }
}
```

- [x] **步骤 4：实现 CH9329 探测**

```rust
pub fn probe_ch9329(path: &str, baud_rate: u32, timeout: Duration) -> ControlProbeStatus {
    let mut port = match serialport::new(path, baud_rate)
        .timeout(Duration::from_millis(50))
        .open()
    {
        Ok(port) => port,
        Err(error) => return ControlProbeStatus::OpenFailed(error.to_string()),
    };

    let frame = match Ch9329Command::GetInfo.to_frame(0) {
        Ok(frame) => frame,
        Err(error) => return ControlProbeStatus::NotCh9329(error.to_string()),
    };
    if let Err(error) = port.write_all(frame.as_bytes()) {
        return ControlProbeStatus::OpenFailed(error.to_string());
    }

    let deadline = Instant::now() + timeout;
    let mut decoder = Ch9329Decoder::new();
    let mut buf = [0_u8; 64];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(read) => {
                for event in decoder.push(&buf[..read]) {
                    let Ok(frame) = event else {
                        continue;
                    };
                    match Ch9329Response::parse(&frame) {
                        Ok(Ch9329Response::Info(info)) => {
                            return ControlProbeStatus::Ready(ControlInfo {
                                version: info.version,
                                usb_enumerated: info.usb_enumerated,
                            });
                        }
                        Ok(_) => return ControlProbeStatus::NotCh9329("unexpected acknowledgement".into()),
                        Err(error) => return ControlProbeStatus::NotCh9329(error.to_string()),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return ControlProbeStatus::OpenFailed(error.to_string()),
        }
    }
    ControlProbeStatus::NoResponse
}
```

- [x] **步骤 5：实现视频预览**

在 `frame.rs` 定义 `RgbaFrame` 和 BGRA 转换；`probe.rs` 的 `capture_preview` 打开 `CameraSource` 后等一帧，函数返回前 drop 预览源，避免正式连接被预览句柄占住。

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn bgra_to_rgba(frame: &ipkvm_video::VideoFrame) -> Result<RgbaFrame, String> {
    if frame.pixel_format != ipkvm_video::PixelFormat::Bgra8888 {
        return Err(format!("unsupported preview pixel format: {:?}", frame.pixel_format));
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let mut pixels = vec![0; width * height * 4];
    for y in 0..height {
        let src = &frame.data[y * stride..y * stride + width * 4];
        let dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
        for (rgba, bgra) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    Ok(RgbaFrame {
        width: frame.width,
        height: frame.height,
        pixels,
    })
}
```

- [x] **步骤 6：验证**

Run:

```powershell
cargo test -p ipkvm-desktop --all-features probe::tests frame::tests -- --nocapture
```

Expected: PASS。

- [x] **步骤 7：提交**

```powershell
git add crates/ipkvm-desktop/src/probe.rs crates/ipkvm-desktop/src/frame.rs crates/ipkvm-desktop/src/lib.rs
git commit -m "feat: add desktop device probing"
```

### 任务 4：渲染模型、缩放和坐标换算

**文件：**
- 新建：`crates/ipkvm-desktop/src/render.rs`

**接口：**
- 产出：`FrameSize`
- 产出：`VideoViewport::frame_rect(container: egui::Rect, frame: FrameSize, mode: VideoScaleMode) -> egui::Rect`
- 产出：`VideoViewport::map_pointer(point: egui::Pos2, rect: egui::Rect, frame: FrameSize) -> Option<(u16, u16)>`

- [x] **步骤 1：编写坐标和缩放测试**

```rust
#[cfg(test)]
mod tests {
    use eframe::egui::{pos2, Rect};

    use super::*;
    use crate::state::VideoScaleMode;

    #[test]
    fn fit_window_preserves_aspect_ratio() {
        let container = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(1000.0, 500.0));
        let rect = VideoViewport::frame_rect(
            container,
            FrameSize { width: 1920, height: 1080 },
            VideoScaleMode::FitWindow,
        );
        assert!((rect.width() - 888.8889).abs() < 0.01);
        assert_eq!(rect.height(), 500.0);
    }

    #[test]
    fn pointer_maps_back_to_framebuffer_coordinates() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), eframe::egui::vec2(200.0, 100.0));
        let frame = FrameSize { width: 400, height: 200 };

        assert_eq!(
            VideoViewport::map_pointer(pos2(110.0, 70.0), rect, frame),
            Some((200, 100))
        );
        assert_eq!(VideoViewport::map_pointer(pos2(9.0, 70.0), rect, frame), None);
    }
}
```

- [x] **步骤 2：实现渲染模型**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

pub struct VideoViewport;

impl VideoViewport {
    pub fn frame_rect(
        container: eframe::egui::Rect,
        frame: FrameSize,
        mode: crate::state::VideoScaleMode,
    ) -> eframe::egui::Rect {
        if frame.width == 0 || frame.height == 0 {
            return container;
        }
        let size = match mode {
            crate::state::VideoScaleMode::FitWindow
            | crate::state::VideoScaleMode::ResizeWindowToVideo => {
                let frame_aspect = frame.width as f32 / frame.height as f32;
                let container_aspect = container.width() / container.height();
                if container_aspect > frame_aspect {
                    eframe::egui::vec2(container.height() * frame_aspect, container.height())
                } else {
                    eframe::egui::vec2(container.width(), container.width() / frame_aspect)
                }
            }
            crate::state::VideoScaleMode::ActualSize => {
                eframe::egui::vec2(frame.width as f32, frame.height as f32)
            }
        };
        eframe::egui::Rect::from_center_size(container.center(), size)
    }

    pub fn map_pointer(
        point: eframe::egui::Pos2,
        rect: eframe::egui::Rect,
        frame: FrameSize,
    ) -> Option<(u16, u16)> {
        if !rect.contains(point) || frame.width == 0 || frame.height == 0 {
            return None;
        }
        let x = ((point.x - rect.left()) / rect.width() * frame.width as f32)
            .floor()
            .clamp(0.0, frame.width.saturating_sub(1) as f32) as u16;
        let y = ((point.y - rect.top()) / rect.height() * frame.height as f32)
            .floor()
            .clamp(0.0, frame.height.saturating_sub(1) as f32) as u16;
        Some((x, y))
    }
}
```

- [x] **步骤 3：验证并提交**

```powershell
cargo test -p ipkvm-desktop --all-features render::tests -- --nocapture
git add crates/ipkvm-desktop/src/render.rs
git commit -m "feat: add desktop video viewport mapping"
```

### 任务 5：共享 session 增加桌面本地 client 与 notice mirror

**文件：**
- 修改：`crates/ipkvm-session/src/rfb_connection/mod.rs`
- 修改：`crates/ipkvm-session/src/console_session.rs`
- 修改：`crates/ipkvm-session/src/session_manager.rs`

**接口：**
- 产出：`RfbClientId::local_desktop() -> RfbClientId`
- 产出：`ConsoleSession::set_notice_mirror(Option<mpsc::UnboundedSender<RfbInputNotice>>)`
- 产出：`SessionManager::set_notice_mirror(Option<mpsc::UnboundedSender<RfbInputNotice>>)`

- [x] **步骤 1：为本地桌面 client 写测试**

在 `rfb_connection/mod.rs` tests 中加入：

```rust
#[test]
fn local_desktop_client_id_is_reserved() {
    assert_eq!(RfbClientId::local_desktop().get(), u64::MAX);
}
```

实现：

```rust
pub fn local_desktop() -> Self {
    Self(u64::MAX)
}
```

- [x] **步骤 2：为 notice mirror 写 session 测试**

在 `console_session.rs` tests 中加入：启动会话、发送 `Connected` 和 `CutText`，从 mirror receiver 收到 `ControllerAcquired`、`TextDispatched`、`TextTyped`。

```rust
#[tokio::test]
async fn notice_mirror_receives_input_and_text_notices() {
    let (mut session, _sink) = console_session_fixture();
    let (notice_tx, mut notice_rx) = tokio::sync::mpsc::unbounded_channel();
    session.set_notice_mirror(Some(notice_tx));
    session.start().unwrap();

    let event_tx = session.event_tx().clone();
    let client_id = RfbClientId::for_test(1);
    let peer_addr = "127.0.0.1:5900".parse().unwrap();
    event_tx
        .send(RfbServerEvent::Connected {
            client_id,
            peer_addr,
            shared: true,
        })
        .await
        .unwrap();
    event_tx
        .send(RfbServerEvent::CutText {
            client_id,
            bytes: b"a".to_vec(),
        })
        .await
        .unwrap();

    let mut seen_text_typed = false;
    for _ in 0..8 {
        let notice = tokio::time::timeout(std::time::Duration::from_secs(1), notice_rx.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(notice, crate::rfb_input::RfbInputNotice::TextTyped { .. }) {
            seen_text_typed = true;
            break;
        }
    }
    assert!(seen_text_typed);

    drop(event_tx);
    drop(session.stop().unwrap());
}
```

- [x] **步骤 3：实现 `ConsoleSession` mirror**

给 `ConsoleSession` 增加字段：

```rust
notice_mirror: Option<tokio::sync::mpsc::UnboundedSender<RfbInputNotice>>,
```

增加方法：

```rust
pub fn set_notice_mirror(
    &mut self,
    notice_mirror: Option<tokio::sync::mpsc::UnboundedSender<RfbInputNotice>>,
) {
    self.notice_mirror = notice_mirror;
}
```

在 `start()` 里 clone 到 observe 闭包：

```rust
let notice_mirror = self.notice_mirror.clone();
let task = tokio::spawn(async move {
    let result = pump
        .run_until_stopped(&mut event_rx, stop_rx, |notice: &RfbInputNotice| {
            if matches!(
                notice,
                RfbInputNotice::Keyboard { .. } | RfbInputNotice::Pointer { .. }
            ) {
                stats.lock().unwrap().observe_input();
            }
            if let Some(tx) = &notice_mirror {
                let _ = tx.send(notice.clone());
            }
        })
        .await;
    running.store(false, Ordering::SeqCst);
    result
});
```

- [x] **步骤 4：让 `SessionManager` 保持 mirror**

给 `SessionManager` 增加同名字段和 setter；在 `new`/`create`/`replace_and_start` 组装新 `ConsoleSession` 后调用 `session.set_notice_mirror(self.notice_mirror.clone())`。

- [x] **步骤 5：验证并提交**

```powershell
cargo test -p ipkvm-session --all-features rfb_connection::tests::local_desktop_client_id_is_reserved console_session::tests::notice_mirror_receives_input_and_text_notices -- --nocapture
git add crates/ipkvm-session/src/rfb_connection/mod.rs crates/ipkvm-session/src/console_session.rs crates/ipkvm-session/src/session_manager.rs
git commit -m "feat: mirror session input notices"
```

### 任务 6：桌面会话控制器

**文件：**
- 新建：`crates/ipkvm-desktop/src/session.rs`

**接口：**
- 产出：`DesktopSessionController::connect(selection: ConnectRequest) -> Result<(), DesktopSessionError>`
- 产出：`DesktopSessionController::stop(&mut self) -> Result<(), DesktopSessionError>`
- 产出：`DesktopSessionController::send_key(&self, down: bool, keysym: u32)`
- 产出：`DesktopSessionController::send_pointer(&self, button_mask: u8, x: u16, y: u16, size: FrameSize)`
- 产出：`DesktopSessionController::paste_text(&self, text: String)`
- 产出：`DesktopSessionController::release_all(&self)`

- [x] **步骤 1：编写 controller 事件测试**

用 fake sink + `SessionManager` 覆盖本地 controller 会先发 `Connected`，再能发键盘/指针，`release_all` 通过 disconnect+reconnect 复位后仍能继续输入。

- [x] **步骤 2：实现生产连接**

生产 `connect` 需要：

```rust
let frame_source: std::sync::Arc<dyn ipkvm_video::FrameSource> =
    std::sync::Arc::new(ipkvm_video::camera::CameraSource::open(
        &request.video_device_id,
        request.preview_fps,
    )?);
let queue = ipkvm_core::SerialCommandQueue::open(&request.control_device_id, request.baud_rate)?;
let sink = ipkvm_core::Ch9329InputSink::new(queue, 0, request.mouse_mode);
```

`SessionManager::replace_and_start` 必须在 `runtime.block_on` 内执行；调用 `start`/`replace_and_start` 前进入 runtime，避免 `tokio::spawn` 找不到 runtime。

- [x] **步骤 3：建立本地 controller lifecycle**

连接成功后，拿 `manager.event_publisher().borrow().clone()` 当前 sender，并 `try_send`：

```rust
RfbServerEvent::Connected {
    client_id: RfbClientId::local_desktop(),
    peer_addr: "127.0.0.1:0".parse().unwrap(),
    shared: true,
}
```

`release_all` 发送：

```rust
RfbServerEvent::Disconnected {
    client_id: RfbClientId::local_desktop(),
    peer_addr: "127.0.0.1:0".parse().unwrap(),
    reason: RfbDisconnectReason::ClientClosed,
}
```

随后重新发送 `Connected`，让后续键鼠继续有 active controller。

- [x] **步骤 4：处理 notice**

`DesktopSessionController` 持有 `notice_rx`，暴露：

```rust
pub fn drain_notices(&mut self) -> Vec<ipkvm_session::rfb_input::RfbInputNotice>
```

app 每帧调用并更新粘贴、输入状态栏和错误提示。

- [x] **步骤 5：验证并提交**

```powershell
cargo test -p ipkvm-desktop --all-features session::tests -- --nocapture
git add crates/ipkvm-desktop/src/session.rs
git commit -m "feat: add desktop session controller"
```

### 任务 7：键盘、鼠标和特殊键适配

**文件：**
- 新建：`crates/ipkvm-desktop/src/input.rs`

**接口：**
- 产出：`egui_key_to_keysym(key: egui::Key, modifiers: egui::Modifiers) -> Option<u32>`
- 产出：`special_key_sequence(key: SpecialKey) -> Vec<KeyAction>`
- 产出：`modifier_diff(previous: egui::Modifiers, current: egui::Modifiers) -> Vec<KeyAction>`
- 产出：`pointer_button_mask(response: &egui::Response, previous_mask: u8) -> u8`

- [x] **步骤 1：编写特殊键测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_alt_del_sequence_presses_modifiers_before_delete_and_releases_reverse() {
        assert_eq!(
            special_key_sequence(SpecialKey::CtrlAltDel),
            vec![
                KeyAction::Down(XK_CONTROL_L),
                KeyAction::Down(XK_ALT_L),
                KeyAction::Down(XK_DELETE),
                KeyAction::Up(XK_DELETE),
                KeyAction::Up(XK_ALT_L),
                KeyAction::Up(XK_CONTROL_L),
            ]
        );
    }

    #[test]
    fn function_keys_use_x11_keysym_range() {
        assert_eq!(special_key_sequence(SpecialKey::F(1))[0], KeyAction::Down(0xffbe));
        assert_eq!(special_key_sequence(SpecialKey::F(12))[0], KeyAction::Down(0xffc9));
    }
}
```

- [x] **步骤 2：实现 keysym 常量和特殊键**

```rust
pub const XK_BACKSPACE: u32 = 0xff08;
pub const XK_TAB: u32 = 0xff09;
pub const XK_RETURN: u32 = 0xff0d;
pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_HOME: u32 = 0xff50;
pub const XK_LEFT: u32 = 0xff51;
pub const XK_UP: u32 = 0xff52;
pub const XK_RIGHT: u32 = 0xff53;
pub const XK_DOWN: u32 = 0xff54;
pub const XK_PAGE_UP: u32 = 0xff55;
pub const XK_PAGE_DOWN: u32 = 0xff56;
pub const XK_END: u32 = 0xff57;
pub const XK_INSERT: u32 = 0xff63;
pub const XK_DELETE: u32 = 0xffff;
pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ALT_L: u32 = 0xffe9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Down(u32),
    Up(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialKey {
    CtrlAltDel,
    Escape,
    F(u8),
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowUp,
    ArrowRight,
    ArrowDown,
}
```

- [x] **步骤 3：实现普通键映射**

字母按 `modifiers.shift` 决定大小写；数字按 Shift 映射 `!@#$%^&*()`；控制键走 X11 keysym。未覆盖的键返回 `None` 并在状态栏短暂显示“不支持的按键”。

- [x] **步骤 4：验证并提交**

```powershell
cargo test -p ipkvm-desktop --all-features input::tests -- --nocapture
git add crates/ipkvm-desktop/src/input.rs
git commit -m "feat: map desktop input events"
```

### 任务 8：剪贴板、截图和保存

**文件：**
- 新建：`crates/ipkvm-desktop/src/clipboard.rs`

**接口：**
- 产出：`ClipboardService::read_text() -> Result<String, ClipboardError>`
- 产出：`ClipboardService::copy_image(frame: &RgbaFrame) -> Result<(), ClipboardError>`
- 产出：`save_jpeg(path: &Path, frame: &RgbaFrame) -> Result<(), ClipboardError>`

- [x] **步骤 1：编写保存 JPEG 测试**

```rust
#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::frame::RgbaFrame;

    #[test]
    fn save_jpeg_writes_non_empty_file() {
        let path = std::env::temp_dir().join(format!(
            "my_ipkvm-test-{}.jpg",
            std::process::id()
        ));
        let frame = RgbaFrame {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, 255],
        };

        save_jpeg(&path, &frame).unwrap();

        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
        let _ = fs::remove_file(path);
    }
}
```

- [x] **步骤 2：实现剪贴板和 JPEG 保存**

`copy_image` 用 `arboard::ImageData`；`save_jpeg` 用已有 `jpeg-encoder`，避免再引入图片编码大依赖。

- [x] **步骤 3：Windows 保存对话框**

在 app 层用：

```rust
#[cfg(windows)]
fn choose_screenshot_path() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("JPEG image", &["jpg", "jpeg"])
        .set_file_name("my_ipkvm-screenshot.jpg")
        .save_file()
}
```

非 Windows 先不启用文件对话框，菜单项显示“当前平台暂不支持保存对话框”，但截图复制到剪贴板仍可用。

- [x] **步骤 4：验证并提交**

```powershell
cargo test -p ipkvm-desktop --all-features clipboard::tests -- --nocapture
git add crates/ipkvm-desktop/src/clipboard.rs
git commit -m "feat: add desktop clipboard actions"
```

### 任务 9：设备 dialog、控制台、菜单与状态栏

**文件：**
- 修改：`crates/ipkvm-desktop/src/app.rs`

**接口：**
- 消费：`DeviceSelectionState`、`ProductionProbeBackend`、`DesktopSessionController`、`VideoViewport`、`ClipboardService`
- 产出：启动先显示设备选择 dialog；连接后显示视频控制台；菜单可重新选择设备。

- [x] **步骤 1：实现 app 状态**

`DesktopApp` 至少持有：

```rust
struct DesktopApp {
    selection: crate::state::DeviceSelectionState,
    probe: crate::probe::ProductionProbeBackend,
    session: crate::session::DesktopSessionController,
    texture: Option<eframe::egui::TextureHandle>,
    latest_frame: Option<crate::frame::RgbaFrame>,
    pointer_mask: u8,
    paste_busy: bool,
    status_message: Option<String>,
    showing_device_dialog: bool,
}
```

- [x] **步骤 2：设备 dialog**

第一屏只画：

- 视频设备下拉。
- 视频预览区域。
- 控制设备下拉。
- CH9329 探测状态。
- “刷新检测”按钮。
- “高级”折叠区。
- “连接”按钮，`!selection.can_connect()` 时禁用。

设备选择变化时立即对选中项探测；刷新按钮调用任务 3 的 `refresh_detection`。

- [x] **步骤 3：连接行为**

点击连接：

1. 释放预览句柄。
2. 读取当前 `selected_video_id`、`selected_control_id` 和高级配置。
3. 调用 `session.connect(...)`。
4. 成功后关闭 dialog，进入控制台；失败时留在 dialog 并显示错误。

- [x] **步骤 4：控制台画面**

`CentralPanel` 显示视频纹理。无帧或连续 2 秒没有新 seq 时显示“无信号”，但 app 不退出。分辨率变化时更新 texture 尺寸、状态栏和坐标换算。

- [x] **步骤 5：输入只在视频区域聚焦时发送**

视频区域获得焦点时处理 egui key/pointer 事件；失焦、停止连接、重新选择设备、退出 app 前调用 `session.release_all()`。

- [x] **步骤 6：控制菜单**

菜单包含：

- 发送特殊键。
- 粘贴文本。
- 释放所有按键。
- 截图。
- 重新选择设备。
- 停止连接。
- 高级设置。

粘贴按钮在 `paste_busy` 为 true 时禁用；收到 `RfbInputNotice::TextTyped` 或 `TextInputFailed` 后解除。

- [x] **步骤 7：状态栏**

底部四段：

- 控制设备：合法、离线、重新探测中、错误。
- 键盘输入：聚焦可输入、失焦、粘贴中、错误。
- 鼠标坐标：当前 framebuffer 坐标或窗口外。
- 视频：分辨率、无信号、断流、预览/正式会话错误。

- [x] **步骤 8：验证并提交**

```powershell
cargo test -p ipkvm-desktop --all-features
cargo check -p ipkvm-desktop --all-features
git add crates/ipkvm-desktop/src/app.rs
git commit -m "feat: build desktop app flow"
```

### 任务 10：desktop 硬件异常处理

**文件：**
- 修改：`crates/ipkvm-desktop/src/app.rs`
- 修改：`crates/ipkvm-desktop/src/session.rs`
- 修改：`crates/ipkvm-desktop/src/state.rs`

**接口：**
- 产出：正式连接后视频无帧不崩溃，恢复后继续显示。
- 产出：视频分辨率变化更新坐标和 texture。
- 产出：输入泵因串口写失败停止时 UI 进入控制设备离线；刷新检测可重新探测并连接。

- [x] **步骤 1：视频断流状态**

app 每帧读取正式 `FrameSource::latest_frame()`。若没有帧或连续 2 秒没有新 seq，状态栏显示“无信号”；若重新出现新 seq，恢复“视频正常”。

- [x] **步骤 2：分辨率变化**

当最新帧的 `width/height` 与当前 `latest_frame` 不同：

1. 重建 egui texture。
2. 更新 `FrameSize`。
3. 下一次 pointer event 使用新尺寸。

- [x] **步骤 3：控制设备离线**

如果 session notice 或 state 表明输入泵退出、串口写失败或当前 event sender 不可用：

1. 状态栏显示“控制设备离线”。
2. 不再发送键鼠输入。
3. 用户点击“刷新检测”后重新执行 CH9329 探测。
4. 探测合法后用户点击连接，走会话级重连。

第一版不做后台自动重连；这避免在目标机反复上下电时反复抢串口。后续 headless/desktop 共享恢复计划再决定是否增加可配置自动重试。

- [x] **步骤 4：验证**

自动化覆盖状态机，真实硬件覆盖断流与掉电：

```powershell
cargo test -p ipkvm-desktop --all-features state::tests session::tests -- --nocapture
```

- [x] **步骤 5：提交**

```powershell
git add crates/ipkvm-desktop/src/app.rs crates/ipkvm-desktop/src/session.rs crates/ipkvm-desktop/src/state.rs
git commit -m "feat: handle desktop hardware interruptions"
```

### 任务 11：文档、真实硬件验证和最终验收

**文件：**
- 修改：`README.md`
- 修改：`docs/superpowers/specs/2026-08-02-desktop-app-product-design.md`

**接口：**
- 产出：README 中有 desktop 启动和硬件验证步骤。
- 产出：设计文档状态从“设计稿”更新为“第一版实施中/已实施”，并记录 headless 恢复拆分。

- [x] **步骤 1：更新 README**

增加命令和行为说明：

```powershell
cargo run -p ipkvm-desktop --all-features
```

说明：启动后选择视频设备和控制设备；控制设备必须探测为 CH9329；连接后菜单可发送特殊键、粘贴、截图和重新选设备。

- [x] **步骤 2：运行自动化验证**

```powershell
cargo fmt --all --check
cargo test -p ipkvm-session --all-features
cargo test -p ipkvm-desktop --all-features
cargo test --workspace --all-features
.\scripts\test-license-policy.ps1
.\scripts\verify-licenses.ps1
.\scripts\verify.ps1
```

Expected: 全部 PASS。

- [x] **步骤 3：真实硬件验证**

在 PR 描述写明以下人工验证证据：

- 启动 app，选择采集卡，能看到静图预览。
- 选择非 CH9329 串口，显示不是合法控制设备，连接按钮禁用。
- 选择 CH9329，探测成功后连接按钮启用。
- 连接后普通键鼠能控制目标机。
- 菜单能发送 Ctrl+Alt+Del、Esc、F1-F12、Insert/Delete/Home/End/PageUp/PageDown、方向键。
- 粘贴文本期间菜单项禁用，完成后恢复。
- 截图先复制到剪贴板，再可保存 JPEG。
- 重新选择设备不退出 app，并会停旧启新。
- 被控机重启导致视频短暂断流时 app 不崩溃，恢复后显示新画面。
- 视频分辨率变化后鼠标坐标仍对应目标画面。
- CH9329 掉电后输入停止，刷新检测恢复后可重新连接。

- [x] **步骤 4：拆 headless 恢复 issue**

如果本 PR 只实现 desktop，则创建独立 Gitea issue，标题：

```text
headless：视频断流与 CH9329 掉线恢复模型
```

正文包含 #33 产品要求、当前 desktop 第一版处理方式、headless 需要补的 `/api/status` 状态、输入离线、重新探测和恢复策略。

- [x] **步骤 5：提交**

```powershell
git add README.md docs/superpowers/specs/2026-08-02-desktop-app-product-design.md
git commit -m "docs: document desktop app verification"
```

## 自审

- 规格覆盖：启动设备选择、视频预览、CH9329 探测、连接 gating、统一刷新、控制菜单、特殊键、粘贴、截图、重新选择设备、状态栏、无信号、分辨率变化、CH9329 掉线后的输入安全均有任务覆盖。
- 范围拆分：headless 的自动恢复是产品要求，但独立于 desktop UI。计划明确要求本轮后拆独立 issue，避免把两个子系统塞进同一个 PR。
- 技术修正：不再强行使用 `pixels` 手写复杂 GUI；用 `eframe/egui` 的 wgpu 后端减少控件和菜单复杂度，同时保留本地 GPU 窗口。
- 依赖约束：`eframe 0.35` 要求 Rust 1.92，本机和 workspace 是 Rust 1.89；计划锁定 `eframe 0.33.2`，满足 rust-version 1.88。
- 占位扫描：计划未使用占位词；每个核心接口都有文件、函数名和测试入口。
- 类型一致性：`DeviceOption`、`VideoProbeStatus`、`ControlProbeStatus`、`FrameSize`、`DesktopSessionController`、`RfbClientId::local_desktop` 在后续任务中按首次定义使用。
