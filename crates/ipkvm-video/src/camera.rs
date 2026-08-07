//! 相机后端公共入口与共享类型。
//!
//! 平台后端：
//! - **Windows**：自研纯 DirectShow sink filter（本文件 `CameraSource` 实现 + `dshow_sink`）。
//!   系统 Sample Grabber 与 OBS 虚拟摄像头不兼容；DirectShow 能枚举到 OBS 等设备。
//! - **Linux/macOS**：nokhwa（见 `camera_nokhwa`），Linux 走 V4L2、macOS 走 AVFoundation，
//!   这两个平台能原生枚举到虚拟摄像头。本文件下方的 `CameraSource` 仅 Windows 编译；
//!   非 Windows 平台由 `camera_nokhwa` 提供 `CameraSource` 并在此 `pub use`。
//!
//! 共享类型（所有平台）：`CameraDeviceInfo`、`CameraSourceError`。
//!
//! Windows DirectShow 实现要点：用 Capture Graph Builder + 自研 sink filter，
//! `RenderStream(NULL, NULL, device, NULL, sink)` 直连；sink 在 `Receive`（流线程）拷帧到
//! 共享槽，采集线程事件驱动（Condvar）转换发布。COM 初始化用 STA，同一线程串行使用。
//! 像素格式按协商到的真实 subtype 转换（NV12/YUY2/RGB24/ARGB32），统一输出 BGRA8888。

// 非 Windows 平台：CameraSource / list_cameras 由 nokhwa 后端提供，在此 re-export，
// 使 `ipkvm_video::camera::CameraSource` 路径在所有平台一致（headless 无需改 import）。
#[cfg(all(unix, feature = "camera"))]
pub use crate::camera_nokhwa::{CameraSource, list_cameras};

// Windows 实现和「无相机后端」stub 都用到这些；Linux/macOS（nokhwa 后端接管）不需要。
// 守卫条件：Windows 或未启用 camera（stub 路径）。
#[cfg(any(windows, not(feature = "camera")))]
use std::sync::{Arc, RwLock};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;

#[cfg(any(windows, not(feature = "camera")))]
use tokio::sync::watch;

#[cfg(any(windows, not(feature = "camera")))]
use crate::{FrameReceiver, FrameSource, SharedVideoFrame, VideoSourceInfo, VideoSourceKind};

#[cfg(windows)]
use crate::{MonotonicTimestamp, PixelFormat, VideoFrame};

#[cfg(windows)]
use crate::dshow_sink::{SinkFilter, SinkFrameSlot, convert_to_bgra_into};
#[cfg(windows)]
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize, IEnumMoniker, IMoniker,
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

// ============================================================================
// Windows DirectShow 实现（CameraSource + list_cameras）。仅 Windows 编译。
// 非 Windows + 无 mf feature 时返回 UnsupportedPlatform stub。
// ============================================================================

#[cfg(windows)]
#[derive(Debug)]
pub struct CameraSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    name: String,
    /// 帧槽：drop 时 stop() 唤醒采集线程；事件驱动核心。
    slot: Option<SinkFrameSlot>,
    /// 兼容停止信号（control.Stop 之前让采集循环退出）。
    stop: Arc<AtomicBool>,
    /// 采集线程句柄（Drop 时 join，同步等待 control.Stop() 完成后再释放设备）。
    _handle: Option<std::thread::JoinHandle<Result<(), CameraSourceError>>>,
    /// 源侧采集/转换统计（采集线程写，/api/status 读）。
    stats: Arc<crate::SourceStats>,
}

#[cfg(windows)]
impl CameraSource {
    pub fn open(device_id: &str, frames_per_second: u64) -> Result<Self, CameraSourceError> {
        if frames_per_second == 0 {
            return Err(CameraSourceError::ZeroFramesPerSecond);
        }
        Self::open_impl(device_id, frames_per_second)
    }
}

/// 初始化结果：CameraSource + 采集循环所需变量（slot/control/stop/latest/sender/stats）。
#[cfg(windows)]
type InitResult = (
    CameraSource,
    SinkFrameSlot,
    windows::Win32::Media::DirectShow::IMediaControl,
    Arc<AtomicBool>,
    Arc<RwLock<Option<SharedVideoFrame>>>,
    watch::Sender<Option<SharedVideoFrame>>,
    Arc<crate::SourceStats>,
);

#[cfg(windows)]
impl CameraSource {
    fn open_impl(device_id: &str, frames_per_second: u64) -> Result<Self, CameraSourceError> {
        use windows::Win32::Media::DirectShow::{
            IBaseFilter, ICaptureGraphBuilder2, IGraphBuilder, IMediaControl,
        };
        use windows::Win32::Media::MediaFoundation::{
            CLSID_CaptureGraphBuilder2, CLSID_FilterGraph, FORMAT_VideoInfo, VIDEOINFOHEADER,
        };
        use windows::Win32::System::Com::CLSCTX;

        // 所有 DirectShow 对象必须在同一 COM 初始化线程上创建/使用。整个枚举 + 打开 +
        // 采集循环都在采集线程（detached thread::spawn）里做，主线程不接触任何 COM 对象
        // ——避免跨线程传 moniker（STA 下跨线程用 COM 对象需要封送，DirectShow moniker 不支持）。
        // 用 sync_channel 传初始化结果（open 等待初始化完成即返回，不 join 采集循环）。
        let device_id = device_id.to_owned();
        let device_id_for_error = device_id.clone();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel(1);
        // stop 标志在闭包外创建，clone 进采集线程；open 超时失败路径需要置位它唤醒线程退出。
        let stop = Arc::new(AtomicBool::new(false));
        let task_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("camera-directshow".into())
            .spawn(move || -> Result<(), CameraSourceError> {
                // DirectShow 必须 STA + 同一线程；线程结束时先 drop 所有 COM 对象
                // 再 CoUninitialize（ComInit 引用计数保证顺序）。
                let _com = ComInit::init();
                // 初始化：建图 + 启动，返回 CameraSource + 采集循环所需变量。
                let init: Result<InitResult, CameraSourceError> = (|| {
                    // 设备匹配：优先精确匹配 id，其次匹配显示名。
                    let (_, display_name, moniker) = enumerate_devices()?
                        .into_iter()
                        .find(|(id, display_name, _)| {
                            id == &device_id || display_name == &device_id
                        })
                        .ok_or_else(|| {
                            CameraSourceError::Open(
                                device_id.clone(),
                                "device not found".to_owned(),
                            )
                        })?;
                    // moniker → IBaseFilter（捕获 filter）
                    let source: IBaseFilter =
                        unsafe { moniker.BindToObject(None, None) }.map_err(|e| {
                            CameraSourceError::Open(
                                device_id.to_owned(),
                                format!("bind to object: {e}"),
                            )
                        })?;
                    // 图 + 捕获图构建器
                    let graph: IGraphBuilder = unsafe {
                        windows::Win32::System::Com::CoCreateInstance(
                            &CLSID_FilterGraph,
                            None,
                            CLSCTX(1),
                        )
                    }
                    .map_err(|e| {
                        CameraSourceError::Open(device_id.to_owned(), format!("create graph: {e}"))
                    })?;
                    let builder: ICaptureGraphBuilder2 = unsafe {
                        windows::Win32::System::Com::CoCreateInstance(
                            &CLSID_CaptureGraphBuilder2,
                            None,
                            CLSCTX(1),
                        )
                    }
                    .map_err(|e| {
                        CameraSourceError::Open(
                            device_id.to_owned(),
                            format!("create graph builder: {e}"),
                        )
                    })?;
                    unsafe { builder.SetFiltergraph(&graph) }.map_err(|e| {
                        CameraSourceError::Open(device_id.to_owned(), format!("set graph: {e}"))
                    })?;
                    unsafe { graph.AddFilter(&source, windows::core::w!("Capture")) }.map_err(
                        |e| {
                            CameraSourceError::Open(
                                device_id.to_owned(),
                                format!("add filter: {e}"),
                            )
                        },
                    )?;
                    // Best-effort 帧率协商（issue #20）：在连接前找 capture pin 的
                    // IAMStreamConfig，设 AvgTimePerFrame = 10_000_000 / fps。
                    // 全程忽略错误——失败则用设备默认帧率继续。
                    let _ = (|| -> windows::core::Result<()> {
                        use windows::Win32::Media::DirectShow::IAMStreamConfig;
                        let mut pins = [None; 1];
                        unsafe { source.EnumPins()?.Next(&mut pins, None) }.ok()?;
                        let Some(pin) = pins[0].as_ref() else {
                            return Ok(());
                        };
                        let config: IAMStreamConfig = pin.cast()?;
                        let mt_ptr = unsafe { config.GetFormat()? };
                        if !mt_ptr.is_null() {
                            let mt = unsafe { &mut *mt_ptr };
                            if mt.formattype == FORMAT_VideoInfo && !mt.pbFormat.is_null() {
                                let vih = unsafe { &mut *(mt.pbFormat as *mut VIDEOINFOHEADER) };
                                vih.AvgTimePerFrame =
                                    (10_000_000_i64 / frames_per_second.max(1) as i64).max(1);
                                let _ = unsafe { config.SetFormat(mt) };
                            }
                            unsafe {
                                windows::Win32::System::Com::CoTaskMemFree(Some(mt_ptr as *const _))
                            };
                        }
                        Ok(())
                    })();
                    // 纯 sink filter（FFmpeg 同款）：capture pin → sink filter 输入 pin。
                    let slot = SinkFrameSlot::new();
                    let sink_filter = windows::core::ComObject::new(SinkFilter::new(slot.clone()));
                    let sink: IBaseFilter = sink_filter.clone().into_interface();
                    unsafe { graph.AddFilter(&sink, windows::core::w!("Sink")) }.map_err(|e| {
                        CameraSourceError::Open(device_id.to_owned(), format!("add sink: {e}"))
                    })?;
                    // 把 filter 引用注入 pin：图管理器连接协商时调 pin.QueryPinInfo，
                    // 期望拿到所属 filter 的 AddRef 副本；返回 NULL 会导致崩溃。
                    sink_filter.get().attach_filter_reference(sink.clone());
                    // 两轮连接（issue #20）：
                    // 第一轮：sink 只认 MJPEG + BGRA，让 DirectShow 插转换 filter 帮转 YUV
                    //         （设备出 MJPEG→直通；出 YUY2→DS 转 BGRA）。
                    // 第二轮兜底：第一轮失败→sink 认所有格式→设备原生格式直连→自己 convert。
                    let render_first = unsafe {
                        builder.RenderStream(
                            None,
                            std::ptr::null(),
                            &source,
                            None::<&windows::Win32::Media::DirectShow::IBaseFilter>,
                            &sink,
                        )
                    };
                    if render_first.is_err() {
                        sink_filter.get().accept_all_formats();
                        unsafe {
                            builder.RenderStream(
                                None,
                                std::ptr::null(),
                                &source,
                                None::<&windows::Win32::Media::DirectShow::IBaseFilter>,
                                &sink,
                            )
                        }
                        .map_err(|e| {
                            CameraSourceError::Open(
                                device_id.to_owned(),
                                format!("render stream (fallback): {e}"),
                            )
                        })?;
                    }
                    let control: IMediaControl = graph.cast().map_err(|e| {
                        CameraSourceError::Open(device_id.to_owned(), format!("cast control: {e}"))
                    })?;
                    let latest = Arc::new(RwLock::new(None));
                    let (sender, _receiver) = watch::channel(None);
                    let stats = crate::SourceStats::new();
                    // stop 由 open_impl 闭包外创建并 clone 进来（task_stop）。
                    let stop = Arc::clone(&task_stop);
                    // 启动图（采集设备在 paused 时不出帧，必须 Run）
                    unsafe { control.Run() }.map_err(|e| {
                        CameraSourceError::Open(device_id.to_owned(), format!("run graph: {e}"))
                    })?;
                    let name = if display_name.is_empty() {
                        device_id.to_owned()
                    } else {
                        display_name
                    };
                    // CameraSource 持 slot 的 clone（drop 时 stop() 唤醒采集线程）。
                    Ok((
                        CameraSource {
                            latest: Arc::clone(&latest),
                            sender: sender.clone(),
                            name,
                            slot: Some(slot.clone()),
                            stop: Arc::clone(&stop),
                            _handle: None,
                            stats: Arc::clone(&stats),
                        },
                        slot,
                        control,
                        stop,
                        latest,
                        sender,
                        stats,
                    ))
                })();
                // 初始化完成：把 CameraSource 发给主线程（open 返回），然后本线程跑采集循环。
                let (source, slot, control, stop, task_latest, task_sender, task_stats) = match init
                {
                    Ok(v) => v,
                    Err(e) => {
                        // 错误路径：把错误转成字符串经 channel 送出（CameraSourceError 不 Clone）。
                        let message = e.to_string();
                        let _ =
                            init_tx.send(Err(CameraSourceError::Open(device_id.clone(), message)));
                        return Err(e);
                    }
                };
                let _ = init_tx.send(Ok(source));
                // 事件驱动采集循环：阻塞等待 next_frame_into，被 Receive 唤醒后处理一帧。
                // 无帧时彻底睡眠（Condvar::wait），CPU 趋近于零；来一帧处理一帧，无需去重。
                // raw_buf 复用：next_frame_into 把帧数据拷进它（复用容量，稳定后零分配）。
                let mut seq = 0_u64;
                let mut raw_buf: Vec<u8> = Vec::new();
                // BGRA 输出 buffer 复用：消除每帧 vec![0u8; w*h*4] 分配（调研阶段 1.2，#19）。
                let mut bgra_buf: Vec<u8> = Vec::new();
                loop {
                    // 等下一帧；长超时只是兜底（防极端情况下永远等不到帧的死锁），
                    // 正常路径由 Receive 的 notify_one 立即唤醒，drop 时 stop() 返回 Stopped。
                    let wait_start = std::time::Instant::now();
                    match slot.next_frame_into(std::time::Duration::from_secs(1), &mut raw_buf) {
                        crate::dshow_sink::NextFrame::Frame {
                            len,
                            media_type: mt,
                        } => {
                            task_stats.record_capture_wait(wait_start.elapsed());
                            // 按协商到的真实格式转换：MJPEG 直通（原始字节），其余转 BGRA8888。
                            let convert_start = std::time::Instant::now();
                            let Some((w, h)) =
                                convert_to_bgra_into(&raw_buf[..len], &mt, &mut bgra_buf)
                            else {
                                continue;
                            };
                            task_stats.record_convert(convert_start.elapsed());
                            // mem::take 把 bgra_buf 整体取出转 Arc（数据不拷贝，只加 refcount 头），
                            // bgra_buf 变空 Vec 下一帧重新填充——消除每帧 vec![0u8; N] 分配。
                            let data: Arc<[u8]> =
                                Arc::from(std::mem::take(&mut bgra_buf).into_boxed_slice());
                            let capture_ns = crate::now_ns();
                            seq = seq.saturating_add(1);
                            // MJPEG 直通时 pixel_format 标 Mjpeg（stride 无意义）；其余 RGB。
                            let (pixel_format, stride) = match mt.format {
                                crate::dshow_sink::NegotiatedFormat::Mjpeg => {
                                    (PixelFormat::Mjpeg, 0)
                                }
                                _ => (PixelFormat::Rgb888, w * 3),
                            };
                            let frame = VideoFrame::new(
                                seq,
                                MonotonicTimestamp::from_nanos(capture_ns),
                                w,
                                h,
                                stride,
                                pixel_format,
                                data,
                            );
                            let shared = Arc::new(frame);
                            task_stats.record_publish(seq, capture_ns);
                            *task_latest.write().expect("camera lock poisoned") =
                                Some(Arc::clone(&shared));
                            task_sender.send_replace(Some(shared));
                        }
                        crate::dshow_sink::NextFrame::Timeout => {
                            // 兜底超时：检查是否应退出（停止主要由 slot.stop() 触发 Stopped）。
                            if stop.load(Ordering::Relaxed) {
                                break;
                            }
                        }
                        crate::dshow_sink::NextFrame::Stopped => break,
                    }
                }
                // 停止图：同步等待流线程退出，之后才能释放对象。
                unsafe {
                    let _ = control.Stop();
                };
                Ok(())
            })
            .map_err(|e| {
                CameraSourceError::Open(device_id_for_error.clone(), format!("spawn: {e}"))
            })?;
        // open 等待初始化完成（最多 5 秒），不 join 采集循环（循环会一直跑到 Drop）。
        // 成功时把采集线程句柄塞回 CameraSource，供 Drop 同步等待 control.Stop() 完成；
        // 失败时 join 句柄让线程干净退出（避免设备句柄在线程未结束时被丢弃）。
        match init_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(mut source)) => {
                source._handle = Some(handle);
                Ok(source)
            }
            Ok(Err(error)) => {
                // 闭包内初始化失败，线程已 return；join 让它彻底退出。
                let _ = handle.join();
                Err(error)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                stop.store(true, Ordering::Release);
                let _ = handle.join();
                Err(CameraSourceError::Open(
                    device_id_for_error,
                    "camera initialization timed out".to_owned(),
                ))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = handle.join();
                Err(CameraSourceError::Open(
                    device_id_for_error,
                    "camera thread exited during init".to_owned(),
                ))
            }
        }
    }
}

#[cfg(windows)]
impl Drop for CameraSource {
    fn drop(&mut self) {
        // 双路径停止：置位 stop 标志（Timeout 分支兜底退出）+ slot.stop() 唤醒
        // 阻塞在 next_frame 的采集线程，使其立即返回 Stopped 并退出。
        self.stop.store(true, Ordering::Release);
        if let Some(slot) = &self.slot {
            slot.stop();
        }
        // 同步等待采集线程退出：线程退出循环末尾会执行 control.Stop()（停止 DirectShow
        // graph 并释放摄像头设备句柄）。join 返回即证明 graph 已停止，之后可安全重开同一
        // 设备（#46：此前句柄被丢弃无法 join，Drop 返回后 graph 可能还在 Running、独占设备，
        // 导致快速重开报「系统资源不足」）。采集循环有 1 秒兜底超时，正常 1-2 秒内退出。
        if let Some(handle) = self._handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(windows)]
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

    fn source_stats(&self) -> Option<crate::SourceStatsSnapshot> {
        Some(self.stats.snapshot())
    }
}

/// 枚举视频采集设备，返回 (id, display_name, moniker)。
///
/// 用 DirectShow 系统设备枚举器（`ICreateDevEnum` + `CLSID_VideoInputDeviceCategory`）
/// 枚举，能看到 OBS 虚拟摄像头等 DirectShow 过滤器（MF 的 `MFEnumDeviceSources` 看不到）。
/// id 采用 `{index}:{display_name}` 格式；`open` 时重新枚举以取得对应 moniker。
#[cfg(windows)]
fn enumerate_devices() -> Result<Vec<(String, String, IMoniker)>, CameraSourceError> {
    use windows::Win32::Media::DirectShow::ICreateDevEnum;
    use windows::Win32::Media::MediaFoundation::{
        CLSID_SystemDeviceEnum, CLSID_VideoInputDeviceCategory,
    };
    use windows::Win32::System::Com::{CLSCTX, CoCreateInstance};

    // 枚举器本身需要 COM；调用方（list_cameras / open_impl）都应已初始化 COM，
    // 这里再初始化一次是幂等的（CoInitializeEx 同线程同模型返回 S_OK/S_FALSE）。
    let _com = ComInit::init();
    let enumerator: ICreateDevEnum =
        unsafe { CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX(1)) }
            .map_err(|e| CameraSourceError::Enumerate(format!("create device enum: {e}")))?;
    // 空类别时官方约定返回 S_FALSE 且 ppenummoniker 置 NULL；windows-rs 的 ok() 把
    // S_FALSE 当成功，所以必须检查出参是否为 None。
    let mut moniker_enum: Option<IEnumMoniker> = None;
    unsafe {
        enumerator
            .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut moniker_enum, 0)
            .map_err(|e| CameraSourceError::Enumerate(format!("create class enumerator: {e}")))?;
    }
    let Some(moniker_enum) = moniker_enum else {
        return Ok(Vec::new());
    };

    let mut devices = Vec::new();
    loop {
        // Next 返回裸 HRESULT（非 Result），S_OK = 取到，S_FALSE = 取完。
        let mut slot = [None::<IMoniker>];
        let result = unsafe { moniker_enum.Next(&mut slot, None) };
        if result != windows::core::HRESULT(0) {
            break;
        }
        if let Some(moniker) = slot[0].take() {
            let display_name = friendly_name(&moniker);
            devices.push((
                format!("{}:{display_name}", devices.len()),
                display_name,
                moniker,
            ));
        }
    }
    // 先释放枚举器（moniker 已取出），再释放 system device enum，最后才 CoUninitialize。
    drop(moniker_enum);
    drop(enumerator);
    Ok(devices)
}

/// 读取设备的友好名称；属性缺失或非字符串时返回空字符串。
#[cfg(windows)]
fn friendly_name(moniker: &IMoniker) -> String {
    use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
    use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VariantClear};

    let Ok(bag) = (unsafe { moniker.BindToStorage::<_, _, IPropertyBag>(None, None) }) else {
        return String::new();
    };
    let mut variant = VARIANT::default();
    if unsafe { bag.Read(windows::core::w!("FriendlyName"), &mut variant, None) }.is_err() {
        return String::new();
    }
    // FriendlyName 是 VT_BSTR；读出后立即释放 VARIANT（它持有 BSTR）。
    let name = unsafe {
        let inner = &variant.Anonymous.Anonymous;
        if inner.vt == VT_BSTR {
            let bstr: windows_core::BSTR = (*inner.Anonymous.bstrVal).clone();
            bstr.to_string()
        } else {
            String::new()
        }
    };
    unsafe {
        let _ = VariantClear(&mut variant);
    };
    // bag 在此 drop（先于 CoUninitialize）。
    drop(bag);
    name
}

#[cfg(windows)]
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    let _com = ComInit::init();
    Ok(enumerate_devices()?
        .into_iter()
        .map(|(id, display_name, _)| CameraDeviceInfo { id, display_name })
        .collect())
}

// 非 Windows 且未启用 camera feature 的 stub（保持 API 完整性）。
#[cfg(not(any(windows, all(unix, feature = "camera"))))]
impl CameraSource {
    pub fn open(_device_id: &str, _frames_per_second: u64) -> Result<Self, CameraSourceError> {
        Err(CameraSourceError::UnsupportedPlatform)
    }
}

#[cfg(not(any(windows, all(unix, feature = "camera"))))]
#[derive(Debug)]
pub struct CameraSource {
    _private: (),
}

#[cfg(not(any(windows, all(unix, feature = "camera"))))]
impl FrameSource for CameraSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        None
    }
    fn subscribe(&self) -> FrameReceiver {
        unreachable!("camera not supported on this platform/configuration")
    }
    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::Camera,
            device_name: String::new(),
            is_loop: false,
        }
    }
}

#[cfg(not(any(windows, all(unix, feature = "camera"))))]
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    Err(CameraSourceError::UnsupportedPlatform)
}

/// 线程级 COM 初始化守卫（引用计数 + STA）。
///
/// 每线程首次 `init()` 时 `CoInitializeEx(COINIT_APARTMENTTHREADED)`（S_OK）并递增计数；
/// 后续 `init()` 只递增计数。`drop` 递减计数，归零时才 `CoUninitialize`。
/// 这样多次枚举/打开不会反复初始化/反初始化 COM（`CoUninitialize` 后 COM 对象
/// 若仍存活会访问违例崩溃）。`RPC_E_CHANGED_MODE`（已有不匹配 COM 模型）时不初始化。
#[cfg(windows)]
struct ComInit;

#[cfg(windows)]
thread_local! {
    static COM_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(windows)]
impl ComInit {
    fn init() -> Option<Self> {
        if COM_DEPTH.with(|depth| depth.get() > 0) {
            COM_DEPTH.with(|depth| depth.set(depth.get() + 1));
            return Some(Self);
        }
        // 只有 S_OK（本线程首次初始化）才创建守卫；S_FALSE（已初始化）不计数不反初始化。
        if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) } == windows::core::HRESULT(0) {
            COM_DEPTH.with(|depth| depth.set(1));
            Some(Self)
        } else {
            None
        }
    }
}

#[cfg(windows)]
impl Drop for ComInit {
    fn drop(&mut self) {
        let depth = COM_DEPTH.with(|depth| {
            let current = depth.get();
            if current > 0 {
                depth.set(current - 1);
            }
            current
        });
        if depth == 1 {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_open_with_zero_fps_rejected() {
        #[cfg(any(windows, all(unix, feature = "camera")))]
        {
            let err = CameraSource::open("nonexistent", 0).unwrap_err();
            assert!(matches!(err, CameraSourceError::ZeroFramesPerSecond));
        }
    }
}
