//! Windows 相机后端（DirectShow）。只支持 Windows；其他平台提供「不支持」stub。
//!
//! 用 DirectShow 系统设备枚举器（`ICreateDevEnum` + `CLSID_VideoInputDeviceCategory`）
//! 枚举——能看到 OBS 虚拟摄像头等 DirectShow 过滤器（MF 的 `MFEnumDeviceSources` 看不到）。
//!
//! 打开用 Capture Graph Builder + 自研纯 sink filter（`dshow_sink::SinkFilter`），
//! `RenderStream(NULL, NULL, device, NULL, sink)` 把 capture pin 直连到 sink 的输入 pin；
//! sink 在 `IMemInputPin::Receive`（流线程回调）里拷帧到共享槽。这是唯一被验证能从 OBS
//! 虚拟摄像头持续收帧的方式（系统 Sample Grabber 与 OBS 不兼容，回调只触发 1 帧）。
//! 像素格式按 `ReceiveConnection` 协商到的真实 subtype 转换（NV12/YUY2/RGB24/ARGB32），
//! 统一输出 BGRA8888。详见 `dshow_sink` 模块文档。
//!
//! COM 初始化用 STA（`COINIT_APARTMENTTHREADED`，DirectShow 官方推荐），并保证
//! 先释放所有 COM 对象再 `CoUninitialize`（MTA + 对象存活时反初始化会访问违例崩溃）。
//!
//! `CameraSource` 的采集线程独占 OS 线程（DirectShow 要求单线程串行 + COM 初始化）。
//! 采集是**事件驱动**：采集线程在 `SinkFrameSlot::next_frame` 上**阻塞等待**
//! （`Condvar`），DirectShow 流线程每来一帧（`Receive` 回调）写入并唤醒它；
//! 被唤醒后做像素转换并发布到 `watch`。无帧时采集线程彻底睡眠，CPU 占用趋近于零。

use std::sync::{Arc, RwLock};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use tokio::sync::watch;

use crate::{FrameReceiver, FrameSource, SharedVideoFrame, VideoSourceInfo, VideoSourceKind};

#[cfg(windows)]
use crate::{MonotonicTimestamp, PixelFormat, VideoFrame};

#[cfg(windows)]
use crate::dshow_sink::{SinkFilter, SinkFrameSlot, convert_to_bgra};
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

/// DirectShow 相机帧源。`open` 成功后由后台采集线程持续发布帧。
///
/// 停止：`CameraSource` 被 drop 时调 `SinkFrameSlot::stop`（置位 + notify_all），
/// 唤醒正阻塞在 `next_frame` 的采集线程使其立即返回 `Stopped` 并退出；线程退出前
/// `control.Stop()` 停止图并 drop 所有 DirectShow 对象。
#[derive(Debug)]
pub struct CameraSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    name: String,
    /// 帧槽：drop 时 stop() 唤醒采集线程；事件驱动核心。
    #[cfg(windows)]
    slot: Option<SinkFrameSlot>,
    /// 兼容停止信号（control.Stop 之前让采集循环退出）。
    #[cfg(windows)]
    stop: Arc<AtomicBool>,
    /// 采集线程句柄。
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

/// 初始化结果：CameraSource + 采集循环所需变量（slot/control/stop/started/latest/sender）。
#[cfg(windows)]
type InitResult = (
    CameraSource,
    SinkFrameSlot,
    windows::Win32::Media::DirectShow::IMediaControl,
    Arc<AtomicBool>,
    std::time::Instant,
    Arc<RwLock<Option<SharedVideoFrame>>>,
    watch::Sender<Option<SharedVideoFrame>>,
);

#[cfg(windows)]
impl CameraSource {
    fn open_impl(device_id: &str) -> Result<Self, CameraSourceError> {
        use windows::Win32::Media::DirectShow::{
            IBaseFilter, ICaptureGraphBuilder2, IGraphBuilder, IMediaControl,
        };
        use windows::Win32::Media::MediaFoundation::{
            CLSID_CaptureGraphBuilder2, CLSID_FilterGraph,
        };
        use windows::Win32::System::Com::CLSCTX;

        // 所有 DirectShow 对象必须在同一 COM 初始化线程上创建/使用。整个枚举 + 打开 +
        // 采集循环都在采集线程（detached thread::spawn）里做，主线程不接触任何 COM 对象
        // ——避免跨线程传 moniker（STA 下跨线程用 COM 对象需要封送，DirectShow moniker 不支持）。
        // 用 sync_channel 传初始化结果（open 等待初始化完成即返回，不 join 采集循环）。
        let device_id = device_id.to_owned();
        let device_id_for_error = device_id.clone();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel(1);
        let _handle = std::thread::Builder::new()
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
                    // RenderStream(NULL, NULL, source, NULL, sink) —— FFmpeg 同款：
                    // 从 capture filter 的输出 pin 直接连到我们的 sink（无中间 filter）。
                    // sink 的 ReceiveConnection 会校验并接受协商到的视频媒体类型。
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
                        CameraSourceError::Open(device_id.to_owned(), format!("render stream: {e}"))
                    })?;
                    let control: IMediaControl = graph.cast().map_err(|e| {
                        CameraSourceError::Open(device_id.to_owned(), format!("cast control: {e}"))
                    })?;
                    let latest = Arc::new(RwLock::new(None));
                    let (sender, _receiver) = watch::channel(None);
                    let started = std::time::Instant::now();
                    let stop = Arc::new(AtomicBool::new(false));
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
                        },
                        slot,
                        control,
                        stop,
                        started,
                        latest,
                        sender,
                    ))
                })();
                // 初始化完成：把 CameraSource 发给主线程（open 返回），然后本线程跑采集循环。
                let (source, slot, control, stop, started, task_latest, task_sender) = match init {
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
                loop {
                    // 等下一帧；长超时只是兜底（防极端情况下永远等不到帧的死锁），
                    // 正常路径由 Receive 的 notify_one 立即唤醒，drop 时 stop() 返回 Stopped。
                    match slot.next_frame_into(std::time::Duration::from_secs(1), &mut raw_buf) {
                        crate::dshow_sink::NextFrame::Frame {
                            len,
                            media_type: mt,
                        } => {
                            // 按协商到的真实 subtype 转 BGRA8888（NV12/YUY2/RGB24/ARGB32）。
                            let Some((bgra, w, h)) = convert_to_bgra(&raw_buf[..len], &mt) else {
                                continue;
                            };
                            seq = seq.saturating_add(1);
                            let frame = VideoFrame::new(
                                seq,
                                MonotonicTimestamp::from_nanos(
                                    started.elapsed().as_nanos().try_into().unwrap_or(u64::MAX),
                                ),
                                w,
                                h,
                                w * 4,
                                PixelFormat::Bgra8888,
                                Arc::from(bgra),
                            );
                            let shared = Arc::new(frame);
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
                let _ = source;
                Ok(())
            })
            .map_err(|e| {
                CameraSourceError::Open(device_id_for_error.clone(), format!("spawn: {e}"))
            })?;
        // open 等待初始化完成（最多 5 秒），不 join 采集循环。
        match init_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(CameraSourceError::Open(
                device_id_for_error,
                "camera initialization timed out".to_owned(),
            )),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(CameraSourceError::Open(
                device_id_for_error,
                "camera thread exited during init".to_owned(),
            )),
        }
    }
}

impl Drop for CameraSource {
    fn drop(&mut self) {
        // 双路径停止：置位 stop 标志（Timeout 分支兜底退出）+ slot.stop() 唤醒
        // 阻塞在 next_frame 的采集线程，使其立即返回 Stopped 并退出。
        #[cfg(windows)]
        {
            self.stop.store(true, Ordering::Release);
            if let Some(slot) = &self.slot {
                slot.stop();
            }
        }
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

#[cfg(not(windows))]
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
        #[cfg(windows)]
        {
            let err = CameraSource::open("nonexistent", 0).unwrap_err();
            assert!(matches!(err, CameraSourceError::ZeroFramesPerSecond));
        }
    }
}
