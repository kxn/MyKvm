//! Windows Media Foundation 相机后端。只支持 Windows；其他平台提供「不支持」stub。
//!
//! 输出媒体类型请求 RGB32（小端 = BGRA8888），并启用高级视频处理（AVP）让 MF 的
//! Video Processor MFT 自动完成 YUY2/MJPEG → RGB 转换；对外始终发布 BGRA8888 帧。
//!
//! `CameraSource` 的采集线程持有 `IMFSourceReader`，采集循环
//! `ReadSample → 锁帧（IMF2DBuffer）→ 填 VideoFrame → RwLock + watch`
//! 与 `LoopingVideoSource` 同构：`latest_frame()` 返回最新帧，`subscribe()` 订阅帧流。

use std::sync::{Arc, RwLock};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use tokio::sync::watch;

use crate::{FrameReceiver, FrameSource, SharedVideoFrame, VideoSourceInfo, VideoSourceKind};

#[cfg(windows)]
use crate::{MonotonicTimestamp, PixelFormat, VideoFrame};

#[cfg(windows)]
use windows::Win32::{
    Media::MediaFoundation::{
        IMF2DBuffer, IMFActivate, IMFAttributes, IMFMediaSource, IMFSample, IMFSourceReader,
        MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
        MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
        MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MFCreateAttributes,
        MFCreateMediaType, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources,
        MFMediaType_Video, MFVideoFormat_RGB32,
    },
    System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
};
#[cfg(windows)]
use windows::core::Interface;

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

/// 设备被移除/变更信号。mfreadwrite.h 中 MF_SOURCE_READERF_STREAMTOREADER = 0x1000，
/// windows crate 0.61 未导出该常量。
#[cfg(windows)]
const MF_SOURCE_READERF_STREAMTOREADER: u32 = 0x0000_1000;

/// `IMFSourceReader` 在 windows crate 0.61 中不是 `Send`（接口基于 `NonNull<c_void>`）。
/// reader 在打开线程创建、完成媒体类型协商后，仅由采集线程独占调用
/// （MF 要求 source reader 单线程使用），因此可安全地移动到采集线程。
#[cfg(windows)]
struct SourceReader(IMFSourceReader);

#[cfg(windows)]
unsafe impl Send for SourceReader {}

/// MF 相机帧源。`open` 成功后由后台采集线程持续发布帧。
///
/// 停止：`CameraSource` 被 drop 时置位共享停止标志，采集线程在下一帧读取返回后
/// 退出（延迟 ≤ 一帧周期），reader 随线程内变量 drop，MF 资源随之释放。
/// `JoinHandle` 仅用于配合停止机制（`JoinHandle<()>` 是 `Send + Sync`，
/// 不违反 `FrameSource` 的 `Send + Sync` 约束）。
#[derive(Debug)]
pub struct CameraSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    name: String,
    /// 停止标志：drop 时置位，采集线程检测到后退出。
    #[cfg(windows)]
    stop: Arc<AtomicBool>,
    /// 采集线程句柄。drop 置位停止标志后线程自行退出，句柄随之释放。
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
            let _ = frames_per_second;
            Self::open_impl(device_id)
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
    fn open_impl(device_id: &str) -> Result<Self, CameraSourceError> {
        // 设备匹配：优先精确匹配 id，其次匹配显示名（headless CLI 的 --camera <名称> 用显示名打开）。
        let (_, display_name, activate) = enumerate_devices()?
            .into_iter()
            .find(|(id, display_name, _)| id == device_id || display_name == device_id)
            .ok_or_else(|| {
                CameraSourceError::Open(device_id.to_owned(), "device not found".to_owned())
            })?;

        // 设备激活对象产出 IMFMediaSource，再包一层 IMFSourceReader 用于帧读取。
        let media_source = unsafe { activate.ActivateObject::<IMFMediaSource>() }
            .map_err(|e| CameraSourceError::Open(device_id.to_owned(), format!("activate: {e}")))?;
        drop(activate);

        // 启用高级视频处理：让 Video Processor MFT 自动把 YUY2/MJPEG 转成 RGB32。
        let mut reader_attributes: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut reader_attributes, 1) }.map_err(|e| {
            CameraSourceError::Open(device_id.to_owned(), format!("create attributes: {e}"))
        })?;
        let reader_attributes = reader_attributes.ok_or_else(|| {
            CameraSourceError::Open(device_id.to_owned(), "no attributes store".to_owned())
        })?;
        unsafe {
            reader_attributes
                .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
                .map_err(|e| {
                    CameraSourceError::Open(
                        device_id.to_owned(),
                        format!("enable video processing: {e}"),
                    )
                })?;
        }
        let reader =
            unsafe { MFCreateSourceReaderFromMediaSource(&media_source, &reader_attributes) }
                .map_err(|e| {
                    CameraSourceError::Open(device_id.to_owned(), format!("create reader: {e}"))
                })?;
        drop(media_source);
        let reader = SourceReader(reader);

        // 请求 RGB32 输出媒体类型（小端 = BGRA8888）。
        let media_type = unsafe { MFCreateMediaType() }.map_err(|e| {
            CameraSourceError::Open(device_id.to_owned(), format!("create media type: {e}"))
        })?;
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|e| {
                    CameraSourceError::Open(device_id.to_owned(), format!("set major type: {e}"))
                })?;
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(|e| {
                    CameraSourceError::Open(device_id.to_owned(), format!("set subtype: {e}"))
                })?;
        }
        unsafe {
            reader
                .0
                .SetCurrentMediaType(
                    MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                    None,
                    &media_type,
                )
                .map_err(|e| {
                    CameraSourceError::Open(device_id.to_owned(), format!("set media type: {e}"))
                })?;
        }

        // 读取协商后的帧尺寸（MF_MT_FRAME_SIZE：高 32 位宽、低 32 位高）。
        let negotiated = unsafe {
            reader
                .0
                .GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)
        }
        .map_err(|e| {
            CameraSourceError::Open(device_id.to_owned(), format!("get media type: {e}"))
        })?;
        let frame_size = unsafe { negotiated.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(|e| {
            CameraSourceError::Open(device_id.to_owned(), format!("get frame size: {e}"))
        })?;
        let width = (frame_size >> 32) as u32;
        let height = frame_size as u32;

        let latest = Arc::new(RwLock::new(None));
        let (sender, _receiver) = watch::channel(None);
        let task_latest = Arc::clone(&latest);
        let task_sender = sender.clone();
        let started = std::time::Instant::now();
        let stop = Arc::new(AtomicBool::new(false));
        let task_stop = Arc::clone(&stop);

        // 采集线程。ReadSample 在实时源上会阻塞到下一帧到达，天然按设备帧率驱动，
        // 因此不再额外 sleep（额外 sleep 会让实际帧率降到请求值的一半以下）。
        let handle = std::thread::spawn(move || {
            // 整体捕获 SourceReader（Send 包装）而不是字段 reader.0：Rust 的
            // disjoint closure capture 会只捕获 `.0` 字段（IMFSourceReader，!Send）。
            let reader = reader;
            let _com = ComInit::init();
            let mut seq = 0_u64;
            loop {
                // 停止检查：drop 置位后尽快退出，让 reader 随本线程 drop、MF 资源释放。
                // 检查延迟 ≤ 一帧周期（ReadSample 阻塞时长）。
                if task_stop.load(Ordering::Relaxed) {
                    break;
                }
                // MF 文档要求每次 ReadSample 前把 *pSample 置 NULL；每轮新建即可。
                let mut sample: Option<IMFSample> = None;
                let mut stream_flags = 0_u32;
                let result = unsafe {
                    reader.0.ReadSample(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                        0,
                        None,
                        Some(&mut stream_flags),
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
                        if stream_flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0
                            || stream_flags & MF_SOURCE_READERF_STREAMTOREADER != 0
                        {
                            eprintln!("camera stream ended or device removed");
                            break;
                        }
                        let Some(sample) = sample.as_ref() else {
                            continue;
                        };
                        let Some(pixels) = lock_sample_pixels(sample, width, height) else {
                            continue;
                        };
                        seq = seq.saturating_add(1);
                        let frame = VideoFrame::new(
                            seq,
                            MonotonicTimestamp::from_nanos(
                                started.elapsed().as_nanos().try_into().unwrap_or(u64::MAX),
                            ),
                            width,
                            height,
                            width * 4,
                            PixelFormat::Bgra8888,
                            Arc::from(pixels),
                        );
                        let shared = Arc::new(frame);
                        *task_latest.write().expect("camera lock poisoned") =
                            Some(Arc::clone(&shared));
                        task_sender.send_replace(Some(shared));
                    }
                }
            }
        });

        let name = if display_name.is_empty() {
            device_id.to_owned()
        } else {
            display_name
        };
        Ok(Self {
            latest,
            sender,
            name,
            stop,
            _handle: Some(handle),
        })
    }
}

impl Drop for CameraSource {
    fn drop(&mut self) {
        // 置位停止标志；采集线程在下一帧读取返回后退出并释放 MF 资源。
        // 不 join：ReadSample 最长阻塞一帧周期，join 会在异常设备上无限等待。
        #[cfg(windows)]
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl FrameSource for CameraSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.latest
            .read()
            .expect("camera lock poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    fn subscribe(&self) -> FrameReceiver {
        self.sender.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::Camera,
            device_name: self.name.clone(),
            is_loop: false,
        }
    }
}

/// 从 sample 中取出一帧 BGRA8888 像素数据（按行间距拷贝，兼容行填充）。
/// 优先走 IMF2DBuffer::Lock2D；若缓冲不支持 2D 接口，退化为 IMFMediaBuffer::Lock 连续拷贝。
#[cfg(windows)]
fn lock_sample_pixels(sample: &IMFSample, width: u32, height: u32) -> Option<Box<[u8]>> {
    let buffer = unsafe { sample.GetBufferByIndex(0) }.ok()?;
    let row_bytes = (width * 4) as usize;
    let frame_bytes = row_bytes * height as usize;
    if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
        let mut scanline: *mut u8 = std::ptr::null_mut();
        let mut pitch: i32 = 0;
        if unsafe { two_d.Lock2D(&mut scanline, &mut pitch) }.is_err() {
            return None;
        }
        let pitch = pitch as isize;
        let mut out = Vec::with_capacity(frame_bytes);
        unsafe {
            for row in 0..height as isize {
                let source = if pitch >= 0 {
                    scanline.offset(row * pitch)
                } else {
                    scanline.offset((height as isize - 1 - row) * pitch)
                };
                out.extend_from_slice(std::slice::from_raw_parts(source, row_bytes));
            }
            let _ = two_d.Unlock2D();
        }
        Some(out.into_boxed_slice())
    } else {
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut max_len: u32 = 0;
        let mut current_len: u32 = 0;
        if unsafe { buffer.Lock(&mut ptr, Some(&mut max_len), Some(&mut current_len)) }.is_err() {
            return None;
        }
        let copy_len = (current_len as usize).min(frame_bytes);
        let out = unsafe {
            let bytes = std::slice::from_raw_parts(ptr, copy_len);
            bytes.to_vec().into_boxed_slice()
        };
        let _ = unsafe { buffer.Unlock() };
        Some(out)
    }
}

/// 枚举视频采集设备，返回 (id, display_name, 激活对象)。
///
/// id 采用 `{index}:{display_name}` 格式（与简报一致）；`open` 时重新枚举以取得
/// 对应的激活对象，`CameraDeviceInfo` 保持只含 `id`/`display_name` 的公开契约。
#[cfg(windows)]
fn enumerate_devices() -> Result<Vec<(String, String, IMFActivate)>, CameraSourceError> {
    let mut attributes: Option<IMFAttributes> = None;
    unsafe { MFCreateAttributes(&mut attributes, 1) }
        .map_err(|e| CameraSourceError::Enumerate(format!("create attributes: {e}")))?;
    let attributes =
        attributes.ok_or_else(|| CameraSourceError::Enumerate("no attributes store".to_owned()))?;
    unsafe {
        attributes
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .map_err(|e| CameraSourceError::Enumerate(format!("set device source type: {e}")))?;
    }
    let mut count = 0_u32;
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    unsafe {
        MFEnumDeviceSources(&attributes, &mut activates, &mut count)
            .map_err(|e| CameraSourceError::Enumerate(e.to_string()))?;
    }
    let mut devices = Vec::new();
    unsafe {
        for index in 0..count as usize {
            let activate = activates.add(index).read();
            let Some(activate) = activate else {
                continue;
            };
            let display_name = friendly_name(&activate);
            devices.push((format!("{index}:{display_name}"), display_name, activate));
        }
        CoTaskMemFree(Some(activates as *const _));
    }
    Ok(devices)
}

/// 读取设备的友好名称；属性缺失时返回空字符串。
#[cfg(windows)]
fn friendly_name(activate: &IMFActivate) -> String {
    let length = match unsafe { activate.GetStringLength(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME) } {
        Ok(length) => length,
        Err(_) => return String::new(),
    };
    let mut buffer = vec![0_u16; length as usize + 1];
    if unsafe { activate.GetString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut buffer, None) }
        .is_err()
    {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

#[cfg(windows)]
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    Ok(enumerate_devices()?
        .into_iter()
        .map(|(id, display_name, _)| CameraDeviceInfo { id, display_name })
        .collect())
}

#[cfg(not(windows))]
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    Err(CameraSourceError::UnsupportedPlatform)
}

/// 线程级 COM 初始化守卫：构造时 CoInitializeEx，析构时 CoUninitialize。
/// 若线程已有不匹配的 COM 模型（RPC_E_CHANGED_MODE），跳过初始化也不做反初始化。
#[cfg(windows)]
struct ComInit;

#[cfg(windows)]
impl ComInit {
    fn init() -> Option<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        result.is_ok().then_some(Self)
    }
}

#[cfg(windows)]
impl Drop for ComInit {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

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

    /// 编译期回归：`CameraSource` 含 `JoinHandle<()>` 与停止标志后仍满足
    /// `FrameSource` 的 `Send + Sync` 约束（`JoinHandle<T: Send>` 是 Send + Sync）。
    #[test]
    fn camera_source_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CameraSource>();
        assert_send_sync::<std::thread::JoinHandle<()>>();
    }
}
