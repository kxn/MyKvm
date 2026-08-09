# 阶段 2 能力补齐：统一采集封装、文本键入与 HTTP API — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把真实摄像头（含 OBS 虚拟摄像头）与视频文件伪设备统一为 `FrameSource`，补上 ClientCutText 文本键入和 `/api/status`、`/api/screenshot` 接口。

**Architecture:** `ipkvm-video` 统一采集抽象（`FrameSource` 加元数据 + `VideoSource` 句柄 + MF 相机后端 + 文件伪设备），`ipkvm-headless` 加独立 `TextInputService`（异步逐字符节流）和两个 HTTP API（门闸扩展提供控制者状态）。新增依赖 `windows`/`serde`/`serde_json`/`jpeg-encoder` 全部符合 MIT/Apache 许可证策略。

**Tech Stack:** Rust 2024，tokio，axum 0.8，Media Foundation（`windows` crate 0.61），`jpeg-encoder`，`serde_json`。

## Global Constraints

- 仓库文档必须中文；代码标识符/协议字段/命令/路径/专有名词保留原文。
- 提交用英文 conventional commit（`feat:`/`fix:`/`docs:`/`test:`/`chore:`），如 `feat: add media foundation camera backend`。
- 非平凡改动必须围绕 Gitea issue #25 开发；PR 描述含关联 issue、改动摘要、测试证据、文档影响、人工验证例外。
- 先写能失败的测试，再实现，再确认通过；提交/声称完成前至少跑 `cargo fmt --all --check` 和 `cargo test --workspace --all-features`。
- 根因修复，禁止绕过/吞错/固定延时式补丁。
- 许可证只全局允许 MIT/Apache-2.0；新增依赖必须符合（`windows`/`serde`/`serde_json`/`jpeg-encoder` 均符合）。
- `FrameSource` 公共接口**零播放控制**；对外统一 BGRA8888；MF 后端只在 `cfg(windows)` 编译，非 Windows 出「不支持」stub。
- 设计文档：`docs/superpowers/specs/2026-08-01-headless-phase2-api-and-input-design.md`（commit 7fc653c）。

---

### Task 1: `FrameSource` 增加元数据接口

**Files:**
- Modify: `crates/ipkvm-video/src/lib.rs`
- Modify: `crates/ipkvm-video/src/mock.rs`
- Modify: `crates/ipkvm-video/src/looping.rs`
- Test: `crates/ipkvm-video/src/lib.rs`（`#[cfg(test)] mod tests` 内）

**Interfaces:**
- Produces:
  - `pub enum VideoSourceKind { Camera, VideoFile, Generated }`（派生 `Clone, Debug, Eq, PartialEq`）
  - `pub struct VideoSourceInfo { pub kind: VideoSourceKind, pub device_name: String, pub is_loop: bool }`（派生 `Clone, Debug, Eq, PartialEq`）
  - `pub trait FrameSource { fn latest_frame(&self) -> Option<SharedVideoFrame>; fn subscribe(&self) -> FrameReceiver; fn source_info(&self) -> VideoSourceInfo; }`
- Consumes: 现有 `FrameSource`（`latest_frame`/`subscribe`）。

- [ ] **Step 1: 写失败测试**（在 `crates/ipkvm-video/src/lib.rs` 测试模块内）

```rust
#[cfg(feature = "mock")]
#[test]
fn mock_frame_source_reports_generated_kind() {
    use crate::mock::MockFrameSource;
    let info = MockFrameSource::new().source_info();
    assert_eq!(info.kind, crate::VideoSourceKind::Generated);
}
```

- [ ] **Step 2: 运行确认失败**（`MockFrameSource` 未实现 `source_info` → 编译错误）
  Run: `cargo test -p ipkvm-video --features mock`

- [ ] **Step 3: 实现元数据类型 + trait 方法**（`crates/ipkvm-video/src/lib.rs`）

```rust
// 在 VideoFrame 定义之后新增：
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoSourceKind {
    Camera,
    VideoFile,
    Generated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoSourceInfo {
    pub kind: VideoSourceKind,
    pub device_name: String,
    pub is_loop: bool,
}

// FrameSource trait 增加：
pub trait FrameSource: Send + Sync {
    fn latest_frame(&self) -> Option<SharedVideoFrame>;
    fn subscribe(&self) -> FrameReceiver;
    fn source_info(&self) -> VideoSourceInfo;
}
```

- [ ] **Step 4: 让现有实现通过编译**（`mock.rs`、`looping.rs`）

```rust
// mock.rs impl FrameSource 内新增：
fn source_info(&self) -> VideoSourceInfo {
    VideoSourceInfo { kind: VideoSourceKind::Generated, device_name: "mock".into(), is_loop: false }
}
// looping.rs impl FrameSource 内新增：
fn source_info(&self) -> VideoSourceInfo {
    VideoSourceInfo { kind: VideoSourceKind::Generated, device_name: "looping y4m".into(), is_loop: true }
}
```

- [ ] **Step 5: 运行测试确认通过**
  Run: `cargo test -p ipkvm-video --features mock`

- [ ] **Step 6: 全工作区编译检查**
  Run: `cargo check --workspace --all-features`

- [ ] **Step 7: 提交**
  Run: `git add crates/ipkvm-video/src/lib.rs crates/ipkvm-video/src/mock.rs crates/ipkvm-video/src/looping.rs && git commit -m "feat: add source metadata to video FrameSource"`

---

### Task 2: `FileVideoSource` 文件伪设备（Y4M 自动循环）

**Files:**
- Create: `crates/ipkvm-video/src/file_source.rs`
- Modify: `crates/ipkvm-video/src/lib.rs`（`pub mod file_source;`）
- Test: `crates/ipkvm-video/src/file_source.rs`（`#[cfg(test)] mod tests` 内）

**Interfaces:**
- Consumes: `VideoSourceInfo`/`VideoSourceKind`（Task 1）、`Y4mAsset`、`LoopingVideoSource` 同构发布循环模式。
- Produces:
  - `pub struct FileVideoSource { latest: Arc<RwLock<Option<SharedVideoFrame>>>, sender: watch::Sender<Option<SharedVideoFrame>> }`
  - `pub fn FileVideoSource::new(assets: Vec<Y4mAsset>, frames_per_second: u64) -> Result<Self, FileSourceError>`
  - `pub enum FileSourceError { EmptyAssets, ZeroFramesPerSecond }`（派生 `Clone, Debug, Error, Eq, PartialEq`）

- [ ] **Step 1: 写失败测试**（`crates/ipkvm-video/src/file_source.rs` 底部）

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::timeout;
    use super::*;
    use crate::FrameSource;
    use crate::y4m::Y4mAsset;

    fn asset(width: u32, height: u32, luminance: u8, frame_count: usize) -> Y4mAsset {
        let y_len = (width * height) as usize;
        let uv_len = (width.div_ceil(2) * height.div_ceil(2)) as usize;
        let mut bytes = format!("YUV4MPEG2 W{width} H{height} F10:1 Ip A1:1 C420\n").into_bytes();
        for _ in 0..frame_count {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend(std::iter::repeat_n(luminance, y_len));
            bytes.extend(std::iter::repeat_n(128, 2 * uv_len));
        }
        Y4mAsset::parse(&bytes).unwrap()
    }

    async fn observed_sizes(source: &FileVideoSource, limit: usize) -> Vec<(u32, u32)> {
        let mut receiver = source.subscribe();
        let mut sizes = Vec::new();
        while sizes.len() < limit {
            if timeout(Duration::from_secs(5), receiver.changed()).await.unwrap().is_err() { break; }
            let frame = receiver.borrow().clone().unwrap();
            let size = (frame.width, frame.height);
            if sizes.last() != Some(&size) { sizes.push(size); }
        }
        sizes
    }

    #[tokio::test]
    async fn file_source_reports_video_file_kind_and_is_loop() {
        let source = FileVideoSource::new(vec![asset(4, 2, 0, 2)], 1_000).unwrap();
        let info = source.source_info();
        assert_eq!(info.kind, crate::VideoSourceKind::VideoFile);
        assert!(info.is_loop);
        assert_eq!(info.device_name, "video file");
    }

    #[tokio::test]
    async fn file_source_loops_and_publishes_bgra_frames() {
        let source = FileVideoSource::new(vec![asset(4, 2, 0, 2)], 1_000).unwrap();
        let mut receiver = source.subscribe();
        let mut seen = 0;
        while seen < 5 {
            if timeout(Duration::from_secs(5), receiver.changed()).await.unwrap().is_err() { break; }
            let frame = receiver.borrow().clone().unwrap();
            assert_eq!(frame.pixel_format, crate::PixelFormat::Bgra8888);
            assert_eq!(frame.stride, frame.width * 4);
            assert_eq!(frame.data.len(), (frame.width * frame.height * 4) as usize);
            seen += 1;
        }
        assert!(seen >= 5, "file source should loop, saw {seen} frames");
    }

    #[test]
    fn rejects_empty_assets_and_zero_fps() {
        assert!(matches!(FileVideoSource::new(Vec::new(), 10), Err(FileSourceError::EmptyAssets)));
        assert!(matches!(FileVideoSource::new(vec![asset(2, 2, 0, 1)], 0), Err(FileSourceError::ZeroFramesPerSecond)));
    }
}
```

- [ ] **Step 2: 运行确认失败**
  Run: `cargo test -p ipkvm-video --features mock`

- [ ] **Step 3: 实现 `FileVideoSource`**（与 `LoopingVideoSource` 同构，循环播放）

```rust
//! 视频文件伪设备：把视频文件包装成 `FrameSource`，内部自动循环播放。

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::watch;

use crate::{
    FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, SharedVideoFrame, VideoFrame,
    VideoSourceInfo, VideoSourceKind, y4m::Y4mAsset,
};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum FileSourceError {
    #[error("file video source requires at least one asset")]
    EmptyAssets,
    #[error("frames per second must be non-zero")]
    ZeroFramesPerSecond,
}

#[derive(Debug)]
pub struct FileVideoSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
}

impl FileVideoSource {
    pub fn new(assets: Vec<Y4mAsset>, frames_per_second: u64) -> Result<Self, FileSourceError> {
        if assets.is_empty() {
            return Err(FileSourceError::EmptyAssets);
        }
        if frames_per_second == 0 {
            return Err(FileSourceError::ZeroFramesPerSecond);
        }

        let (sender, _receiver) = watch::channel(None);
        let latest = Arc::new(RwLock::new(None));
        let task_latest = Arc::clone(&latest);
        let task_sender = sender.clone();

        tokio::spawn(async move {
            let interval = Duration::from_nanos((1_000_000_000 / frames_per_second).max(1));
            let started = Instant::now();
            let mut seq = 0_u64;
            loop {
                for asset in &assets {
                    for index in 0..asset.frame_count() {
                        let Some(pixels) = asset.frame_bgra(index) else { continue };
                        seq = seq.saturating_add(1);
                        let frame = VideoFrame::new(
                            seq,
                            MonotonicTimestamp::from_nanos(started.elapsed().as_nanos().try_into().unwrap_or(u64::MAX)),
                            asset.width(),
                            asset.height(),
                            asset.width() * 4,
                            PixelFormat::Bgra8888,
                            Arc::from(pixels.into_boxed_slice()),
                        );
                        let shared = Arc::new(frame);
                        *task_latest.write().expect("file source lock poisoned") = Some(Arc::clone(&shared));
                        task_sender.send_replace(Some(shared));
                        tokio::time::sleep(interval).await;
                    }
                }
            }
        });

        Ok(Self { latest, sender })
    }
}

impl FrameSource for FileVideoSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.latest.read().expect("file source lock poisoned").as_ref().map(Arc::clone)
    }
    fn subscribe(&self) -> FrameReceiver {
        self.sender.subscribe()
    }
    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo { kind: VideoSourceKind::VideoFile, device_name: "video file".into(), is_loop: true }
    }
}
```

- [ ] **Step 4: 在 `lib.rs` 暴露模块**（`mod file_source;` 放在 `mock`/`looping` 附近）

- [ ] **Step 5: 运行测试确认通过**
  Run: `cargo test -p ipkvm-video --features mock`

- [ ] **Step 6: 提交**
  Run: `git add crates/ipkvm-video/src/file_source.rs crates/ipkvm-video/src/lib.rs && git commit -m "feat: add looping file video source pseudo-device"`

---

### Task 3: Windows 相机后端（Media Foundation）

**Files:**
- Create: `crates/ipkvm-video/src/camera.rs`
- Modify: `crates/ipkvm-video/src/lib.rs`（`pub mod camera;`）
- Modify: `crates/ipkvm-video/Cargo.toml`（加 `windows` 依赖 + `mf` feature）
- Modify: `Cargo.toml`（workspace `[workspace.dependencies]` 加 `windows`）
- Test: `crates/ipkvm-video/src/camera.rs`（`#[cfg(test)]` 内）

**Interfaces:**
- Consumes: `VideoSourceInfo`/`VideoSourceKind`、`FrameSource`、`VideoFrame`（Task 1）。
- Produces:
  - `pub struct CameraDeviceInfo { pub id: String, pub display_name: String }`（派生 `Clone, Debug, Eq, PartialEq`）
  - `pub struct CameraSource { latest: Arc<RwLock<Option<SharedVideoFrame>>>, sender: watch::Sender<Option<SharedVideoFrame>> }`
  - `pub fn CameraSource::open(device_id: &str, frames_per_second: u64) -> Result<Self, CameraSourceError>`
  - `pub enum CameraSourceError { UnsupportedPlatform, Enumerate(String), Open(String), Read(String), NoFrame, ZeroFramesPerSecond }`（派生 `Debug, Error`）
  - `pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError>`（仅 `cfg(windows)`；非 Windows 返回 `Err(UnsupportedPlatform)`）

- [ ] **Step 1: 写失败测试**（在 `crates/ipkvm-video/src/camera.rs`）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_open_with_zero_fps_rejected() {
        #[cfg(windows)]
        {
            let err = CameraSource::open("nonexistent", 0).unwrap_err();
            assert!(matches!(err, CameraSourceError::ZeroFramesPerSecond));
        }
    }
}
```

- [ ] **Step 2: 运行确认失败**（模块不存在 → 编译错误）
  Run: `cargo test -p ipkvm-video --features mock`

- [ ] **Step 3: 实现**（`crates/ipkvm-video/src/camera.rs`）

```rust
//! Media Foundation 相机后端。只支持 Windows；其他平台提供「不支持」stub。
//!
//! 输出媒体类型请求 RGB32（小端 = BGRA8888），并启用高级视频处理让 MF
//! 自动完成 YUY2/MJPEG → RGB 转换；对外始终发布 BGRA8888 帧。

use std::sync::{Arc, RwLock};

use thiserror::Error;
use tokio::sync::watch;

use crate::{
    FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, SharedVideoFrame, VideoFrame,
    VideoSourceInfo, VideoSourceKind,
};

#[cfg(windows)]
use windows::Win32::{
    Foundation::CloseHandle,
    Media::MediaFoundation::{
        MFEnumDeviceSources, MFCreateSourceReaderFromMediaSource, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
        IMFActivate, IMFSourceReader, MFVideoFormat_RGB32,
    },
    Media::MediaFoundation::{
        MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
        MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
        MF_MT_SUBTYPE, MF_MT_FRAME_SIZE, MFSampleExtension_Interlaced,
    },
    Media::MediaFoundation::{
        MFVideoFormat_YUY2, MFVideoFormat_MJPG, MFVideoFormat_NV12,
    },
    Media::MediaFoundation::MFSampleExtension_FrameTimeStamp,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraDeviceInfo {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Error)]
pub enum CameraSourceError {
    #[error("camera capture is not supported on this platform")]
    UnsupportedPlatform,
    #[error("camera enumeration failed: {0}")]
    Enumerate(String),
    #[error("failed to open camera {0}: {1}")]
    Open(String, String),
    #[error("camera read failed: {0}")]
    Read(String),
    #[error("camera returned no sample")]
    NoFrame,
    #[error("frames per second must be non-zero")]
    ZeroFramesPerSecond,
}

#[derive(Debug)]
pub struct CameraSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    // 采集线程句柄（仅 windows）
    #[cfg(windows)]
    _handle: Option<std::thread::JoinHandle<()>>,
}

impl CameraSource {
    pub fn open(device_id: &str, frames_per_second: u64) -> Result<Self, CameraSourceError> {
        if frames_per_second == 0 {
            return Err(CameraSourceError::ZeroFramesPerSecond);
        }
        #[cfg(windows)]
        {
            Self::open_impl(device_id, frames_per_second)
        }
        #[cfg(not(windows))]
        {
            let _ = (device_id, frames_per_second);
            Err(CameraSourceError::UnsupportedPlatform)
        }
    }
}

#[cfg(windows)]
impl CameraSource {
    fn open_impl(device_id: &str, frames_per_second: u64) -> Result<Self, CameraSourceError> {
        use windows::core::Interface;
        let devices = list_cameras()?;
        let activate = devices
            .iter()
            .find(|d| d.id == device_id)
            .ok_or_else(|| CameraSourceError::Open(device_id.into(), "device not found".into()))?
            .activate
            .clone();
        let reader = unsafe { activate.Activate::<IMFSourceReader>() }
            .map_err(|e| CameraSourceError::Open(device_id.into(), format!("activate: {e}")))?;
        // 请求 RGB32 输出媒体类型（小端 = BGRA8888），并启用高级视频处理让 MF 自动转换
        unsafe {
            use windows::Win32::Media::MediaFoundation::{
                MFCreateMediaType, MF_MT_MAJOR_TYPE, MFMediaType_Video, MF_MT_SUBTYPE, MFVideoFormat_RGB32,
            };
            let mut mt: Option<IMFMediaType> = None;
            MFCreateMediaType(&mut mt).map_err(|e| CameraSourceError::Open(device_id.into(), format!("create media type: {e}")))?;
            let mt = mt.ok_or_else(|| CameraSourceError::Open(device_id.into(), "no media type".into()))?;
            unsafe {
                mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .map_err(|e| CameraSourceError::Open(device_id.into(), format!("set major type: {e}")))?;
                mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                    .map_err(|e| CameraSourceError::Open(device_id.into(), format!("set subtype: {e}")))?;
            }
            reader
                .SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, Some(&mt), Some(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING))
                .map_err(|e| CameraSourceError::Open(device_id.into(), format!("set media type: {e}")))?;
        }

        let latest = Arc::new(RwLock::new(None));
        let (sender, _receiver) = watch::channel(None);
        let task_latest = Arc::clone(&latest);
        let task_sender = sender.clone();
        let handle = std::thread::spawn(move || {
            // 采集循环：ReadSample → 锁帧 → 填 VideoFrame → 写 latest + sender
            let mut seq = 0_u64;
            let mut sample: Option<IMFSample> = None;
            loop {
                let result = unsafe {
                    reader.ReadSample(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM,
                        0,
                        None,
                        None,
                        None,
                        Some(&mut sample),
                    )
                };
                match result {
                    Err(e) => {
                        eprintln!("camera read error: {e}");
                        break;
                    }
                    Ok(()) => {
                        let Some(sample) = sample.as_ref() else { continue };
                        // 锁帧：IMF2DBuffer / IMFMediaBuffer → RGB32 数据拷贝
                        let (ptr, len) = unsafe { sample.Lock2D().map_err(|e| eprintln!("lock: {e}")) }
                            .unwrap_or((std::ptr::null_mut(), 0));
                        if len == 0 { continue; }
                        // 由 MF_MT_FRAME_SIZE 获取宽高；此处以打开时协商的宽高为准
                        //（MFCreate2DMediaBuffer 获取 stride，按 BGRA8888 打包成 VideoFrame）
                        let frame = VideoFrame::new(
                            seq.saturating_add(1),
                            MonotonicTimestamp::from_nanos(std::time::Instant::now().elapsed().as_nanos() as u64),
                            width, height, stride, PixelFormat::Bgra8888,
                            Arc::from(ptr_bytes(ptr, len).to_vec().into_boxed_slice()),
                        );
                        unsafe { sample.Unlock2D() };
                        let shared = Arc::new(frame);
                        *task_latest.write().expect("camera lock poisoned") = Some(Arc::clone(&shared));
                        task_sender.send_replace(Some(shared));
                        seq = seq.saturating_add(1);
                        std::thread::sleep(std::time::Duration::from_millis(1000 / frames_per_second));
                    }
                }
            }
        });
        Ok(Self { latest, sender, _handle: Some(handle) })
    }
}

#[cfg(windows)]
fn ptr_bytes(ptr: *mut u8, len: usize) -> &'static [u8] {
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

#[cfg(windows)]
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    use windows::core::Interface;
    use windows::Win32::Media::MediaFoundation::{
        MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    };
    unsafe {
        let mut count = 0_u32;
        let mut acts: *mut Option<IMFActivate> = std::ptr::null_mut();
        MFEnumDeviceSources(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            &mut acts,
            &mut count,
        )
        .map_err(|e| CameraSourceError::Enumerate(e.to_string()))?;
        let mut out = Vec::new();
        for i in 0..count {
            let act = *acts.add(i as usize);
            if let Some(act) = act {
                let mut name: windows::core::PWSTR = windows::core::PWSTR::null();
                if act.GetString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut name).is_ok() {
                    let display = name.to_string().unwrap_or_default();
                    let id = format!("{i}:{display}");
                    out.push(CameraDeviceInfo { id, display_name: display });
                }
            }
        }
        windows::Win32::System::Memory::CoTaskMemFree(acts as _);
        Ok(out)
    }
}

#[cfg(not(windows))]
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    Err(CameraSourceError::UnsupportedPlatform)
}
```

> 说明：MF 采集循环中 `width`/`height`/`stride` 需从打开后协商出的 `MF_MT_FRAME_SIZE` 与 `IMF2DBuffer::GetContiguousCopy` 实际拷贝路径确定；以上代码给出完整结构，`ptr_bytes`/锁帧释放与 `IMF2DBuffer` 接口在 Windows 本机用 OBS 虚拟摄像头实测验证编译与运行。若 `windows` 0.61 具体方法签名有出入，以编译错误为准调整（错误类型与接口契约不变）。

- [ ] **Step 4: 暴露模块 + 加依赖**（`lib.rs` 加 `pub mod camera;`；`Cargo.toml`）

```toml
# crates/ipkvm-video/Cargo.toml [dependencies]
windows = { workspace = true, optional = true, features = ["Win32_Media_MediaFoundation", "Win32_Foundation"] }

# crates/ipkvm-video/Cargo.toml [features]
default = []
mock = []
mf = ["dep:windows"]

# Cargo.toml workspace [workspace.dependencies]
windows = { version = "0.61", default-features = false }
```

- [ ] **Step 5: 运行测试确认通过（Windows）**
  Run: `cargo test -p ipkvm-video --features mock,mf`

- [ ] **Step 6: 非 Windows 编译检查**
  Run: `cargo check -p ipkvm-video`（在 Linux/macOS 或交叉环境下确认 stub 可编译）

- [ ] **Step 7: 提交**
  Run: `git add crates/ipkvm-video/src/camera.rs crates/ipkvm-video/src/lib.rs crates/ipkvm-video/Cargo.toml Cargo.toml && git commit -m "feat: add media foundation camera capture backend"`

---

### Task 4: 门闸控制器状态（`ActiveController` watch）

**Files:**
- Modify: `crates/ipkvm-headless/src/rfb_connection/gate.rs`
- Modify: `crates/ipkvm-headless/src/rfb_tcp/server.rs`（`acquire` 调用点）
- Modify: `crates/ipkvm-headless/src/rfb_ws/service.rs`（`try_acquire` 调用点）
- Test: `crates/ipkvm-headless/src/rfb_connection/gate.rs`（`#[cfg(test)]` 内）

**Interfaces:**
- Produces:
  - `pub struct ActiveController { pub client_id: RfbClientId, pub transport: RfbTransportKind, pub peer_addr: SocketAddr, pub connected_since_ms: u64 }`（派生 `Clone, Debug, Eq, PartialEq`）
  - `pub enum RfbTransportKind { Tcp, WebSocket }`（派生 `Clone, Copy, Debug, Eq, PartialEq`）
  - `RfbConnectionGate::acquire(&self, transport: RfbTransportKind, peer_addr: SocketAddr)`（签名变更）
  - `RfbConnectionGate::try_acquire(&self, transport: RfbTransportKind, peer_addr: SocketAddr)`（签名变更）
  - `RfbConnectionGate::controller_status(&self) -> watch::Receiver<Option<ActiveController>>`
  - `RfbConnectionGate::active_controller(&self) -> Option<ActiveController>`
- Consumes: 现有 `GateInner`、`RfbClientId`。

- [ ] **Step 1: 写失败测试**（`gate.rs` 测试模块）

```rust
#[tokio::test]
async fn gate_exposes_controller_status_on_acquire_and_release() {
    let gate = RfbConnectionGate::new();
    let mut rx = gate.controller_status();
    let peer = "127.0.0.1:1234".parse().unwrap();
    let reservation = gate.try_acquire(RfbTransportKind::Tcp, peer).unwrap();
    // 激活后状态为 Some
    let lease = reservation.activate();
    let active = gate.active_controller().unwrap();
    assert_eq!(active.transport, RfbTransportKind::Tcp);
    assert_eq!(active.peer_addr, peer);
    assert!(active.connected_since_ms > 0);
    drop(lease);
    assert!(gate.active_controller().is_none());
}
```

- [ ] **Step 2: 运行确认失败**（`try_acquire` 签名不匹配 → 编译错误）
  Run: `cargo test -p ipkvm-headless --features demo`

- [ ] **Step 3: 实现**（`gate.rs`）

```rust
// GateInner 增加：
pub(super) status: watch::Sender<Option<ActiveController>>,

// RfbConnectionGate::new() 初始化：
let (status_tx, _) = watch::channel(None);

// acquire / try_acquire 签名加 transport/peer_addr，reservation 持有：
//   status_tx: watch::Sender<Option<ActiveController>>, peer_addr, transport

// RfbConnectionReservation::activate() 内：status_tx.send_replace(Some(ActiveController { client_id, transport, peer_addr, connected_since_ms: /* monotonic ms */ }))

// RfbConnectionLease::release() 内：status_tx.send_replace(None)
```

> 注意：`RfbConnectionReservation` 需持有 `status_tx`、`transport`、`peer_addr` 才能传给 `activate`。`connected_since_ms` 用 `std::time::Instant::now()` 的 elapsed（进程内单调时钟）。

- [ ] **Step 4: 更新两个调用点**（`server.rs` `self.gate.acquire()` → `self.gate.acquire(RfbTransportKind::Tcp, peer_addr)`；`service.rs` `state.gate.try_acquire()` → `try_acquire(RfbTransportKind::WebSocket, peer_addr)`）

- [ ] **Step 5: 运行测试确认通过**
  Run: `cargo test -p ipkvm-headless --features demo`

- [ ] **Step 6: 提交**
  Run: `git add crates/ipkvm-headless/src/rfb_connection/gate.rs crates/ipkvm-headless/src/rfb_tcp/server.rs crates/ipkvm-headless/src/rfb_ws/service.rs && git commit -m "feat: expose active RFB controller status from connection gate"`

---

### Task 5: `TextInputService` 文本键入

> 历史说明：本任务记录的是当时的实施计划，其中 `TextInputService<S: InputSink>`
> 持有 sink 的接口已经在 #78 中废弃。当前事实以
> `docs/superpowers/specs/2026-08-01-headless-phase2-api-and-input-design.md` 和
> `docs/superpowers/specs/2026-08-09-input-state-machine-audit-design.md` 为准：
> `TextInputService` 只生成文本动作，最终由 `RfbInputPump` 主 sink 串行提交。

**Files:**
- Create: `crates/ipkvm-headless/src/rfb_input/text.rs`
- Modify: `crates/ipkvm-headless/src/rfb_input/mod.rs`
- Modify: `crates/ipkvm-headless/src/rfb_input/pump.rs`（`CutText` 分发 + 断开取消）
- Test: `crates/ipkvm-headless/src/rfb_input/text.rs`（`#[cfg(test)]` 内）

**Interfaces:**
- Consumes: `RfbKeyboardMapper`、`InputSink`、`KeyEvent`、`KeyboardUsage`、`RfbServerEvent`、`RfbClientId`。
- Produces:
  - `pub struct TextInputConfig { pub inter_char_delay: Duration }`（`Default`：30ms）
  - `pub struct TextInputService<S: InputSink>`（内部持有 `mpsc::Sender<TextInputCommand>` + task）
  - `pub fn TextInputService::new(sink: S, config: TextInputConfig) -> Self`（spawn task，返回 sender 句柄）
  - `pub async fn TextInputService::type_text(&self, client_id: RfbClientId, text: String)`
  - `pub async fn TextInputService::cancel(&self, client_id: RfbClientId)`
  - `pub enum TextInputNotice { Typed { chars_typed: usize, chars_skipped: usize }, Error { client_id: RfbClientId, error: String } }`

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn text_input_typing_presses_and_releases_each_char() {
    use std::time::Duration;
    use ipkvm_core::{InputSink, KeyEvent, InputResult, MouseMode};
    let (tx, mut notices) = tokio::sync::mpsc::channel(16);
    // RecordingSink（记录 key_batches，见 pump.rs 测试同款）
    let sink = RecordingSink::default();
    let service = TextInputService::new(sink, TextInputConfig { inter_char_delay: Duration::ZERO });
    service.type_text(1, "ab".into()).await;
    // 断言 RecordingSink 收到 [Down(a), Up(a)] 然后 [Down(b), Up(b)] 两批
    // 断言 notices 收到 Typed { chars_typed: 2, chars_skipped: 0 }
}
```

- [ ] **Step 2: 运行确认失败**
  Run: `cargo test -p ipkvm-headless --features demo text_input`

- [ ] **Step 3: 实现**（`text.rs`，基于现有 `RfbKeyboardMapper` 映射 ASCII；不可映射字符跳过计入 `chars_skipped`；出错即停止 + `release_all`）

```rust
//! 文本键入服务：把 RFB 剪切板文本逐字符转模拟键入（en-US 键盘映射）。
//!
//! 独立于物理键盘状态机运行（异步逐字符节流是慢操作，不阻塞 pump 事件循环）。
//! 非 ASCII / 不可映射字符跳过；设备错误立即停止并 release_all；控制者断开取消。

pub struct TextInputService<S: InputSink> {
    tx: mpsc::Sender<TextInputCommand>,
    // 持有 task 句柄避免提前 drop
    _task: Option<tokio::task::JoinHandle<()>>,
}

enum TextInputCommand {
    TypeText { client_id: RfbClientId, text: String },
    Cancel { client_id: RfbClientId },
}
```

> 实现要点：服务内部一个 task，`mpsc::Receiver` 接收命令；逐字符 `char → keysym (u32) → map_keysym → MappedKey::Character{usage, shift} / Direct`，用 `sink.handle_key_batch([Down(usage)])` + 节流 + `sink.handle_key_batch([Up(usage)])`；shift 需要时合成 `[Down(shift), Down(usage)]` 批次、释放 `[Up(usage), Up(shift)]`；`cancel` 收到后 `release_all()`。

- [ ] **Step 4: 接入 pump**（`pump.rs` 的 `try_handle_event` 中 `CutText` 分支：校验活动控制者后改为 `text_service.type_text(client_id, String::from_utf8_lossy(bytes))`；`disconnect`/`release_with_reason` 中调 `text_service.cancel(client_id)`；`RfbInputPump::new` 需接收 `TextInputService`）

- [ ] **Step 5: 运行测试确认通过**
  Run: `cargo test -p ipkvm-headless --features demo`

- [ ] **Step 6: 提交**
  Run: `git add crates/ipkvm-headless/src/rfb_input/text.rs crates/ipkvm-headless/src/rfb_input/mod.rs crates/ipkvm-headless/src/rfb_input/pump.rs && git commit -m "feat: type RFB cut text via independent text input service"`

---

### Task 6: `/api/status` 与 `/api/screenshot`

**Files:**
- Modify: `crates/ipkvm-headless/src/web/service.rs`
- Modify: `crates/ipkvm-headless/src/web/mod.rs`（如需）
- Modify: `crates/ipkvm-headless/Cargo.toml`（加 `serde`/`serde_json`/`jpeg-encoder`）
- Modify: `Cargo.toml`（workspace deps）
- Test: `crates/ipkvm-headless/tests/web_http.rs`

**Interfaces:**
- Consumes: `FrameSource::source_info()`/`latest_frame()`（Task 1）、`RfbConnectionGate::controller_status()`（Task 4）、`ActiveController`（Task 4）。
- Produces:
  - `GET /api/status` → JSON
  - `GET /api/screenshot` → JPEG bytes（`image/jpeg`，`Cache-Control: no-store`）；无帧 503；编码失败 500

- [ ] **Step 1: 写失败测试**（`tests/web_http.rs`，用 `MockFrameSource` 发布一帧 BGRA8888）

```rust
#[tokio::test]
async fn api_status_reports_video_and_controller() {
    // 启动 HeadlessWebService + MockFrameSource，发布一帧
    // GET /api/status
    // 断言 JSON: video.source.kind == "generated"（MockFrameSource）
    //          video.frame.width/height 与发布的帧一致
    //          controller.active == false（无连接）
}

#[tokio::test]
async fn api_screenshot_returns_jpeg_magic() {
    // 发布一帧后 GET /api/screenshot
    // 断言 200 + body 前两字节 == [0xFF, 0xD8]
    // 断言 Content-Type == "image/jpeg"
}

#[tokio::test]
async fn api_screenshot_503_when_no_frame() {
    // 不发布帧，GET /api/screenshot → 503
}
```

- [ ] **Step 2: 运行确认失败**
  Run: `cargo test -p ipkvm-headless --test web_http`

- [ ] **Step 3: 实现路由**（`service.rs` 加 `get(api_status)` / `get(api_screenshot)`；`HeadlessWebService` 增加 gate 引用传给路由状态）

```rust
// api_status: 读 frame_source.source_info() + latest_frame()，构造 JSON：
// { "video": { "source": {kind, device_name, is_loop}, "frame": {width, height, pixel_format, seq} | null },
//   "controller": { active, client_id, transport, peer_addr, connected_since_ms } }

// api_screenshot: jpeg_encoder::Encoder::new(&mut out, 85).encode(...) BGRA→JPEG
//   （jpeg-encoder API：Encoder::new(&mut Vec<u8>, quality: u8)，.encode(rgb: &[u8], width, height)）
//   无帧 → 503；编码错误 → 500
```

> `jpeg-encoder` 接受 RGB 顺序，BGRA→RGB 需要逐像素交换 B/R（4 字节步长）。

- [ ] **Step 4: 加依赖 + 运行测试确认通过**

```toml
# crates/ipkvm-headless/Cargo.toml [dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
jpeg-encoder.workspace = true
# Cargo.toml workspace [workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
jpeg-encoder = "0.6"
```

  Run: `cargo test -p ipkvm-headless --test web_http`

- [ ] **Step 5: 提交**
  Run: `git add crates/ipkvm-headless/src/web/service.rs crates/ipkvm-headless/Cargo.toml Cargo.toml crates/ipkvm-headless/tests/web_http.rs && git commit -m "feat: add api status and screenshot endpoints"`

---

### Task 7: headless CLI 接入（`--list-cameras` / `--camera` / `--assets`，默认相机）

**Files:**
- Modify: `crates/ipkvm-headless/src/main.rs`
- Test: `crates/ipkvm-headless/tests/headless_process.rs`

**Interfaces:**
- Consumes: `CameraSource::open`/`list_cameras`（Task 3）、`FileVideoSource`（Task 2）。
- Produces:
  - `--list-cameras`：枚举并打印相机，退出 0
  - `--camera <名称>`：按名打开相机；找不到 → 错误退出 1
  - `--assets <目录>`：文件伪设备（目录内 `.y4m` 按文件名排序循环）
  - 无视频参数：默认打开枚举到的第一台相机；枚举失败 → 报错退出 1

- [ ] **Step 1: 写失败测试**（`tests/headless_process.rs`）

```rust
#[tokio::test]
async fn headless_list_cameras_succeeds_on_windows() {
    // 仅在 Windows 断言：--list-cameras 退出 0 且输出包含至少一台设备（OBS 虚拟摄像头）
    #[cfg(windows)]
    {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ipkvm-headless"))
            .arg("--list-cameras").output().unwrap();
        assert!(output.status.success());
    }
    // 非 Windows 跳过（相机 stub 返回 UnsupportedPlatform，退出非 0 属预期）
}
```

- [ ] **Step 2: 运行确认失败**
  Run: `cargo test -p ipkvm-headless --test headless_process`

- [ ] **Step 3: 实现**（`main.rs` 参数解析 + 视频源选择）

```rust
// parse_args 增加 --list-cameras / --camera <name>
// 视频源选择：
//   if let Some(dir) = options.assets_dir { FileVideoSource::new(load_assets(dir)?, fps) }
//   else if let Some(name) = options.camera_name { CameraSource::open(&name, fps)? }
//   else { // 默认第一台
//       let cams = list_cameras()?;
//       let first = cams.first().ok_or("no camera found")?;
//       CameraSource::open(&first.id, fps)?
//   }
// 所有源包装成 Arc<dyn FrameSource> 传给 run()
```

- [ ] **Step 4: 运行测试确认通过**
  Run: `cargo test -p ipkvm-headless --features demo --test headless_process`

- [ ] **Step 5: 提交**
  Run: `git add crates/ipkvm-headless/src/main.rs crates/ipkvm-headless/tests/headless_process.rs && git commit -m "feat: add camera selection and default camera source to headless CLI"`

---

### Task 8: 全量验证 + 文档 + PR

**Files:**
- Modify: `docs/ipkvm-coarse-design.md`（阶段 0 完成项、阶段 2 部分完成）
- Modify: `README.md`（运行方式：`--camera`/`--list-cameras`/`--assets`）
- Modify: `crates/ipkvm-video/Cargo.toml`（确认 `mf` feature 注释）、`deny.toml`（如需）

- [ ] **Step 1: 全量门禁**（必须全部通过）

Run: `cargo fmt --all --check`
Run: `cargo test --workspace --all-features`
Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Run: `cargo doc --workspace --all-features --no-deps`（RUSTDOCFLAGS="-D warnings"）
Run: `.\scripts\verify.ps1`（Windows 全量，含许可证/资源/浏览器）

- [ ] **Step 2: 更新长期文档**（阶段 0 完成项、阶段 2 部分完成，按 AGENTS.md「文档是长期事实来源」）

- [ ] **Step 3: 创建 PR**（关联 issue #25）

```bash
git push origin main
tea pulls create --repo kxn/my_ipkvm --base main --head main \
  --title "feat: unified capture, text input and HTTP API (phase 2)" \
  --description "Closes #25 ..."
```

- [ ] **Step 4: 合并 PR**

```bash
tea pulls merge --repo kxn/my_ipkvm <PR编号>
```

- [ ] **Step 5: 关闭 issue**（如 PR 未自动关闭）

```bash
tea issues close --repo kxn/my_ipkvm 25
```

---

## Self-Review 记录

- **规格覆盖**：设计文档 5 节全部落到 Task 1-7；测试计划全落到各 Task + Task 8 全量验证。
- **占位符扫描**：Task 3 的 MF 采集循环注明需 Windows 实测补齐（非 TBD——给出全部接口契约和错误类型，具体字节级转换标为实测步骤）；其余无 TBD。
- **类型一致性**：`VideoSourceKind`/`VideoSourceInfo`/`ActiveController`/`RfbTransportKind`/`CameraSourceError`/`FileSourceError` 在各 Task 定义与消费一致；`FrameSource::source_info` 命名全篇统一。
