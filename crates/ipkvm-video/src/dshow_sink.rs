//! 纯 sink filter（DirectShow），对照 FFmpeg 的 `libdshow_filter.c` / `libdshow_pin.c` 移植。
//!
//! # 为什么不用 Sample Grabber
//! 实测 Sample Grabber 与 OBS 虚拟摄像头不兼容（回调模式只触发 1 帧、缓冲模式连不上）。
//! FFmpeg 用**自己的纯 sink filter**（`IBaseFilter` + `IPin` + `IMemInputPin`，输入 pin 的
//! `Receive` 里拷数据）持续收帧，这是被验证能持续收 OBS 帧的方式。本实现完全照搬其逻辑。
//!
//! # 崩溃根因（前一版踩的坑）
//! DirectShow 图管理器在 `RenderStream` 期间会回调我们 sink 的若干 COM 方法并接管返回的
//! 出参（如 `QueryId` 返回的 `PWSTR`、`QueryFilterInfo` 的 `FILTER_INFO`）。windows-rs 的
//! vtable shim 在 `Err` 分支**不写出参**，调用方拿到栈上垃圾指针 `CoTaskMemFree` 即段错误。
//! 因此每个返回出参的方法都必须成功写出有效内存；同理不能返回 `E_NOTIMPL` 的枚举器，
//! 否则图管理器在协商时读到未初始化结构。
//!
//! # 拓扑
//! capture filter 的 capture pin → 本 filter 的输入 pin（id="In"）。
//! 无输出 pin（纯 sink），`RenderStream(NULL, NULL, device, NULL, sink)` 连接。

#![cfg(windows)]
#![allow(non_snake_case)]

use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, S_FALSE};
use windows::Win32::Media::DirectShow::{
    IBaseFilter, IBaseFilter_Impl, IEnumMediaTypes, IEnumMediaTypes_Impl, IEnumPins,
    IEnumPins_Impl, IFilterGraph, IMediaSample, IMemAllocator, IMemInputPin, IMemInputPin_Impl,
    IPin, IPin_Impl, PIN_DIRECTION, PINDIR_INPUT,
};
use windows::Win32::Media::IReferenceClock;
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, FORMAT_VideoInfo, MEDIASUBTYPE_ARGB32, MEDIASUBTYPE_NV12, MEDIASUBTYPE_RGB24,
    MEDIASUBTYPE_RGB32, MEDIASUBTYPE_YUY2, MEDIATYPE_Video, VIDEOINFOHEADER,
};
use windows::core::{PCWSTR, PWSTR, Ref};

/// 协商到的像素格式（按 DirectShow subtype 识别）。BGRA8888 是对外发布格式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiatedFormat {
    Nv12,
    Yuy2,
    /// BGR 字节序，每像素 3 字节。
    Rgb24,
    /// 32bpp，按 subtype 区分 ARGB（有 alpha）/ RGB32（不透明，alpha=255）。
    Argb32,
}

impl NegotiatedFormat {
    pub fn from_subtype(subtype: windows::core::GUID) -> Option<Self> {
        if subtype == MEDIASUBTYPE_NV12 {
            Some(Self::Nv12)
        } else if subtype == MEDIASUBTYPE_YUY2 {
            Some(Self::Yuy2)
        } else if subtype == MEDIASUBTYPE_RGB24 {
            Some(Self::Rgb24)
        } else if subtype == MEDIASUBTYPE_RGB32 || subtype == MEDIASUBTYPE_ARGB32 {
            Some(Self::Argb32)
        } else {
            None
        }
    }

    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgb24 => 3,
            Self::Argb32 => 4,
            // NV12/YUY2 是打包格式，不按每像素字节数计；上层按平面/打包布局单独处理。
            Self::Nv12 | Self::Yuy2 => 0,
        }
    }
}

/// 已连接媒体类型的快照：尺寸 + 格式。`Receive` 拷帧后连同格式一起对外发布。
#[derive(Clone, Debug)]
pub struct NegotiatedMediaType {
    pub width: u32,
    pub height: u32,
    /// biHeight 为负表示自顶向下；DirectShow 约定视频通常自底向上（正 biHeight）。
    pub top_down: bool,
    pub format: NegotiatedFormat,
}

/// 帧槽：事件驱动的单生产者（`Receive`，流线程）单消费者（采集线程）缓冲。
///
/// `Receive` 每来一帧就写入并 `notify_one`；采集线程在 `next_frame` 里**阻塞等待**
/// （`Condvar::wait`），被唤醒后取出最新帧做像素转换并发布。无帧时采集线程彻底睡眠，
/// CPU 占用趋近于零——避免轮询架构每 16ms 空转检查 + 反复拷贝转换的开销。
///
/// 取出即「消费」：`next_frame` 把 data 置 None，下次必须等 `Receive` 写入新帧才再次返回，
/// 因此天然「来一帧处理一帧」，无需上层做指针去重。
#[derive(Clone)]
pub struct SinkFrameSlot {
    inner: Arc<SinkFrameSlotInner>,
}

struct SinkFrameSlotInner {
    /// 一次性锁住帧数据与媒体类型，避免两次加锁。
    state: Mutex<SlotState>,
    /// 新帧到达通知：`store_frame` 写入后 notify_one，唤醒 `next_frame` 的 wait。
    cond: std::sync::Condvar,
    /// 停止标志：置位后 `next_frame` 立即返回 Stopped，避免永久阻塞。
    stop: std::sync::atomic::AtomicBool,
}

struct SlotState {
    /// 复用缓冲：`store_frame` 在此写入（`clear`+`extend_from_slice`，容量稳定后零分配）。
    /// `has_new` 为 false 时表示缓冲内容已被消费，可被下一帧覆盖。
    data: Vec<u8>,
    /// 是否有待消费的新帧。`store_frame` 置 true + notify；`next_frame_into` 取走后置 false。
    has_new: bool,
    /// `ReceiveConnection` 协商到的媒体类型（连接成功后恒定，连接前为 None）。
    media_type: Option<NegotiatedMediaType>,
}

impl std::fmt::Debug for SinkFrameSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SinkFrameSlot").finish_non_exhaustive()
    }
}

/// `next_frame_into` 的结果：取到帧 / 超时 / 已停止。
#[derive(Debug)]
pub enum NextFrame {
    /// 取到帧。`len` 是写入 `buf` 的字节数（buf 可能容量更大）。
    Frame {
        len: usize,
        media_type: NegotiatedMediaType,
    },
    Timeout,
    Stopped,
}

impl SinkFrameSlot {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SinkFrameSlotInner {
                state: Mutex::new(SlotState {
                    data: Vec::new(),
                    has_new: false,
                    media_type: None,
                }),
                cond: std::sync::Condvar::new(),
                stop: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// 停止：置位后唤醒所有等待者，`next_frame_into` 返回 Stopped。可重复调用、幂等。
    pub fn stop(&self) {
        self.inner
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        self.inner.cond.notify_all();
    }

    /// 阻塞等待下一帧，把数据拷进调用方提供的 `buf`（复用其容量，零分配）。
    /// 被唤醒后将 slot 内缓冲内容拷进 buf 并置 `has_new=false`（消费语义），返回
    /// `Frame { len, media_type }`；超时 `Timeout`；已停止 `Stopped`。
    ///
    /// 拷贝而非移动：因为 slot 内缓冲是复用的（流线程下一帧要写它），不能 move 出去。
    /// 调用方 buf 同样复用，所以稳定状态下无堆分配。
    pub fn next_frame_into(&self, timeout: std::time::Duration, buf: &mut Vec<u8>) -> NextFrame {
        let mut guard = match self.inner.state.lock() {
            Ok(g) => g,
            Err(_) => return NextFrame::Stopped,
        };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if guard.has_new {
                if let Some(mt) = guard.media_type.clone() {
                    // 复用 buf 容量：clear 保留分配，extend 拷贝（容量足够时零分配）。
                    buf.clear();
                    buf.extend_from_slice(&guard.data);
                    guard.has_new = false;
                    return NextFrame::Frame {
                        len: buf.len(),
                        media_type: mt,
                    };
                }
                // 媒体类型尚未协商（理论上 data 不会先于 media_type 到达）：保留 has_new 等下轮。
            }
            if self.inner.stop.load(std::sync::atomic::Ordering::Acquire) {
                return NextFrame::Stopped;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return NextFrame::Timeout;
            }
            let (new_guard, wait_result) = match self.inner.cond.wait_timeout(guard, deadline - now)
            {
                Ok(v) => v,
                Err(_) => return NextFrame::Stopped,
            };
            guard = new_guard;
            let _ = wait_result;
        }
    }

    fn store_media_type(&self, mt: NegotiatedMediaType) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.media_type = Some(mt);
        }
        // 媒体类型变化不单独通知（data 到达时才通知）。
    }

    /// 写入新帧：复用 slot 内缓冲（`clear`+`extend_from_slice`，容量稳定后零分配），
    /// 置 `has_new=true` 并 notify_one 唤醒采集线程。这是性能关键路径——
    /// 实测每帧 `to_vec` 分配会让流线程吃满一个核，复用缓冲后降到个位数百分比。
    fn store_frame(&self, bytes: &[u8]) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.data.clear();
            state.data.extend_from_slice(bytes);
            state.has_new = true;
        }
        // 唤醒等待的采集线程：来一帧处理一帧。
        self.inner.cond.notify_one();
    }
}

/// 从 `AM_MEDIA_TYPE` 解析尺寸/格式。只认 `FORMAT_VideoInfo` 的 `VIDEOINFOHEADER`。
/// 返回 None 表示不认识，连接会被拒绝。
fn parse_video_info(mt: &AM_MEDIA_TYPE) -> Option<NegotiatedMediaType> {
    if mt.majortype != MEDIATYPE_Video {
        return None;
    }
    if mt.formattype != FORMAT_VideoInfo || mt.pbFormat.is_null() || mt.cbFormat == 0 {
        return None;
    }
    let vih = unsafe { &*(mt.pbFormat as *const VIDEOINFOHEADER) };
    let width = vih.bmiHeader.biWidth.max(0) as u32;
    // biHeight 为负 = 自顶向下；为正 = 自底向上（DirectShow 视频默认）。
    let height = vih.bmiHeader.biHeight.abs() as u32;
    let top_down = vih.bmiHeader.biHeight < 0;
    let format = NegotiatedFormat::from_subtype(mt.subtype)?;
    if width == 0 || height == 0 {
        return None;
    }
    Some(NegotiatedMediaType {
        width,
        height,
        top_down,
        format,
    })
}

/// 分配一个 NUL 结尾的 UTF-16 字符串给 COM 调用方，调用方负责 `CoTaskMemFree`。
/// 用于 `QueryId` / `QueryFilterInfo.achName` 等返回字符串的 COM 约定。
fn alloc_co_pwstr(s: &str) -> windows::core::Result<PWSTR> {
    use windows::Win32::System::Com::CoTaskMemAlloc;
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0u16)).collect();
    let byte_len = wide.len() * 2;
    // SAFETY: CoTaskMemAlloc 分配对齐、可写的 COM 内存。
    let ptr = unsafe { CoTaskMemAlloc(byte_len) };
    if ptr.is_null() {
        return Err(E_OUTOFMEMORY.into());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr as *mut u8, byte_len);
    }
    Ok(PWSTR(ptr as *mut u16))
}

// ============================================================================
// 空但有效的 IEnumMediaTypes：图管理器协商时会枚举，必须返回有效枚举器而非 E_NOTIMPL。
// FFmpeg 的 dshow 用 ff_dshow_enummediatypes_Create(NULL)：Next 永远返回 S_FALSE（空），
// 表示「我不挑格式，按上游的来」，触发上游提供媒体类型并最终走 ReceiveConnection。
// ============================================================================

#[windows_implement::implement(IEnumMediaTypes)]
struct EmptyEnumMediaTypes;

impl IEnumMediaTypes_Impl for EmptyEnumMediaTypes_Impl {
    fn Next(
        &self,
        _cmediatypes: u32,
        _ppmediatypes: *mut *mut AM_MEDIA_TYPE,
        pcfetched: *mut u32,
    ) -> windows::core::HRESULT {
        if !pcfetched.is_null() {
            unsafe {
                pcfetched.write(0);
            }
        }
        S_FALSE // 没有更多元素
    }
    fn Skip(&self, _cmediatypes: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn Reset(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn Clone(&self) -> windows::core::Result<IEnumMediaTypes> {
        Ok(windows::core::ComObject::new(EmptyEnumMediaTypes).into_interface())
    }
}

// ============================================================================
// IEnumPins：只含单个输入 pin 的枚举器（FFmpeg ff_dshow_enumpins_Create 同款）。
// 注意 windows-rs 的 vtable shim 对 Next 直接用 trait 返回的 HRESULT，不接管写出参，
// 所以我们必须自己正确写 pppins / pcfetched。
// ============================================================================

#[windows_implement::implement(IEnumPins)]
struct SinkEnumPins {
    pin: windows::core::ComObject<SinkPin>,
    /// 0=未返回，1=已返回唯一 pin，2=遍历完。
    position: Mutex<u32>,
}

impl IEnumPins_Impl for SinkEnumPins_Impl {
    fn Next(
        &self,
        cpins: u32,
        pppins: *mut Option<IPin>,
        pcfetched: *mut u32,
    ) -> windows::core::HRESULT {
        let mut pos = self.position.lock().expect("enumpins lock poisoned");
        let mut fetched = 0_u32;
        if *pos == 0 && cpins >= 1 {
            // 拷贝一份 IPin 引用（AddRef 由 into_interface 负责）给调用方。
            unsafe {
                pppins.write(Some(self.pin.clone().into_interface()));
            }
            *pos = 1;
            fetched = 1;
        } else {
            *pos = 2;
        }
        if !pcfetched.is_null() {
            unsafe {
                pcfetched.write(fetched);
            }
        }
        if fetched == 0 {
            S_FALSE
        } else {
            windows::core::HRESULT(0)
        }
    }
    fn Skip(&self, cpins: u32) -> windows::core::Result<()> {
        let mut pos = self.position.lock().expect("enumpins lock poisoned");
        *pos = (*pos + cpins).min(2);
        Ok(())
    }
    fn Reset(&self) -> windows::core::Result<()> {
        *self.position.lock().expect("enumpins lock poisoned") = 0;
        Ok(())
    }
    fn Clone(&self) -> windows::core::Result<IEnumPins> {
        let cloned = SinkEnumPins {
            pin: self.pin.clone(),
            position: Mutex::new(*self.position.lock().expect("enumpins lock poisoned")),
        };
        Ok(windows::core::ComObject::new(cloned).into_interface())
    }
}

// ============================================================================
// SinkFilter：实现 IPersist + IMediaFilter + IBaseFilter。
// 注意：windows-rs 的 #[implement(IBaseFilter)] 已自动让 QI(IPersist)/QI(IMediaFilter)
// 成功（IBaseFilter 的 vtable matches 硬编码包含继承链）。这里 impl 继承链上所有 trait
// 是因为编译要求：IBaseFilter_Impl: IMediaFilter_Impl: IPersist_Impl。
// ============================================================================

#[windows_implement::implement(IBaseFilter)]
pub struct SinkFilter {
    pin: windows::core::ComObject<SinkPin>,
    state: Mutex<i32>, // FILTER_STATE: 0=Stopped,1=Paused,2=Running
    /// JoinFilterGraph 回填，供 QueryFilterInfo 返回（图管理器期望拿到它 AddRef 过的副本）。
    graph: Mutex<Option<IFilterGraph>>,
}

impl SinkFilter {
    pub fn new(slot: SinkFrameSlot) -> Self {
        let pin = windows::core::ComObject::new(SinkPin::new(slot));
        Self {
            pin,
            state: Mutex::new(0),
            graph: Mutex::new(None),
        }
    }

    const FILTER_NAME: &'static str = "ipkvm Sink";

    /// 把自身的 IBaseFilter 引用注入内部 pin，供 pin 的 QueryPinInfo 返回。
    ///
    /// 图管理器在连接协商时调用 pin.QueryPinInfo，期望拿到所属 filter 的 AddRef 副本；
    /// 若返回 NULL 会在后续处理时崩溃。必须在 AddFilter 后、RenderStream 前调用。
    /// 这会让 pin 持有 filter 的引用，形成 filter↔pin 循环持有，导致 sink 在进程
    /// 生命周期内不释放（仅泄漏，不崩溃），可接受。
    pub fn attach_filter_reference(&self, filter: IBaseFilter) {
        self.pin.get().attach_filter(filter);
    }
}

impl windows::Win32::System::Com::IPersist_Impl for SinkFilter_Impl {
    fn GetClassID(&self) -> windows::core::Result<windows::core::GUID> {
        // FFmpeg 同款：sink filter 无 CLSID，返回 E_NOTIMPL（出参为 GUID，shim 不写也安全）。
        Err(E_NOTIMPL.into())
    }
}

impl windows::Win32::Media::DirectShow::IMediaFilter_Impl for SinkFilter_Impl {
    fn Stop(&self) -> windows::core::Result<()> {
        *self.state.lock().expect("filter state lock poisoned") = 0;
        Ok(())
    }
    fn Pause(&self) -> windows::core::Result<()> {
        *self.state.lock().expect("filter state lock poisoned") = 1;
        Ok(())
    }
    fn Run(&self, _tstart: i64) -> windows::core::Result<()> {
        *self.state.lock().expect("filter state lock poisoned") = 2;
        Ok(())
    }
    fn GetState(
        &self,
        _mstimeout: u32,
    ) -> windows::core::Result<windows::Win32::Media::DirectShow::FILTER_STATE> {
        Ok(windows::Win32::Media::DirectShow::FILTER_STATE(
            *self.state.lock().expect("filter state lock poisoned"),
        ))
    }
    fn SetSyncSource(&self, _pclock: Ref<'_, IReferenceClock>) -> windows::core::Result<()> {
        // 不需要参考时钟（纯 sink，立即消费）。FFmpeg 同款忽略。
        Ok(())
    }
    fn GetSyncSource(&self) -> windows::core::Result<IReferenceClock> {
        // 没有时钟：返回 E_NOTIMPL（出参是指针，shim Err 分支不写，调用方期望 NULL/失败）。
        Err(E_NOTIMPL.into())
    }
}

impl IBaseFilter_Impl for SinkFilter_Impl {
    fn EnumPins(&self) -> windows::core::Result<IEnumPins> {
        let enumerator = SinkEnumPins {
            pin: self.pin.clone(),
            position: Mutex::new(0),
        };
        Ok(windows::core::ComObject::new(enumerator).into_interface())
    }
    fn FindPin(&self, id: &PCWSTR) -> windows::core::Result<IPin> {
        let id_str = unsafe { id.to_string() }.unwrap_or_default();
        if id_str.eq_ignore_ascii_case("in") {
            Ok(self.pin.clone().into_interface())
        } else {
            // VFW_E_NOT_FOUND。出参是指针，Err 安全。
            Err(windows::Win32::Media::DirectShow::VFW_E_NOT_FOUND.into())
        }
    }
    fn QueryFilterInfo(
        &self,
        pinfo: *mut windows::Win32::Media::DirectShow::FILTER_INFO,
    ) -> windows::core::Result<()> {
        // 关键：必须写整个 FILTER_INFO。achName 是 WCHAR[128]，垃圾内容会被图管理器使用 → 崩。
        unsafe {
            (*pinfo).achName = [0u16; 128];
            let name: Vec<u16> = SinkFilter::FILTER_NAME
                .encode_utf16()
                .take(127)
                .chain(std::iter::once(0))
                .collect();
            let n = name.len().min(128);
            (&mut (*pinfo).achName)[..n].copy_from_slice(&name[..n]);
            // pGraph 期望拿到 AddRef 过的副本；调用方负责 Release（ManuallyDrop 由 shim 处理释放）。
            let graph_clone = self.graph.lock().expect("graph lock poisoned").clone();
            (*pinfo).pGraph = core::mem::ManuallyDrop::new(graph_clone);
        }
        Ok(())
    }
    fn JoinFilterGraph(
        &self,
        pgraph: Ref<'_, IFilterGraph>,
        _pname: &PCWSTR,
    ) -> windows::core::Result<()> {
        // 缓存 graph 引用（cloned 会 AddRef），供 QueryFilterInfo 回填。
        let graph = pgraph.cloned();
        *self.graph.lock().expect("graph lock poisoned") = graph;
        Ok(())
    }
    fn QueryVendorInfo(&self) -> windows::core::Result<PWSTR> {
        // FFmpeg 同款 E_NOTIMPL（出参 PWSTR，shim Err 分支不写，安全）。
        Err(E_NOTIMPL.into())
    }
}

// ============================================================================
// SinkPin：实现 IPin + IMemInputPin。
// 这是图管理器连接协商的核心：QueryAccept/EnumMediaTypes/ReceiveConnection 决定能否连上，
// 连上后 Receive 在流线程持续收帧。
// ============================================================================

#[windows_implement::implement(IPin, IMemInputPin)]
struct SinkPin {
    slot: SinkFrameSlot,
    /// 已连接的对端 pin（ReceiveConnection 存，ConnectedTo 返回其克隆）。
    connected_to: Mutex<Option<IPin>>,
    /// 协商到的媒体类型深拷贝（ReceiveConnection 存，ConnectionMediaType 回填给调用方）。
    /// 用 Vec<u8> 持有 pbFormat 的独立拷贝，避免引用上游缓冲（上游断连即 UAF）。
    connected_mt: Mutex<OwnedMediaType>,
    /// 所属 filter 的弱引用（QueryPinInfo 回填）。filter 持有 pin，故用克隆 AddRef。
    filter: Mutex<Option<IBaseFilter>>,
}

impl SinkPin {
    fn new(slot: SinkFrameSlot) -> Self {
        Self {
            slot,
            connected_to: Mutex::new(None),
            connected_mt: Mutex::new(OwnedMediaType::empty()),
            filter: Mutex::new(None),
        }
    }

    /// 供 SinkFilter 注入所属 filter 引用，QueryPinInfo 时回填（AddRef 副本）。
    pub fn attach_filter(&self, filter: IBaseFilter) {
        *self.filter.lock().expect("pin filter lock poisoned") = Some(filter);
    }

    const PIN_NAME: &'static str = "In";
}

impl IPin_Impl for SinkPin_Impl {
    fn Connect(
        &self,
        _preceivepin: Ref<'_, IPin>,
        _pmt: *const AM_MEDIA_TYPE,
    ) -> windows::core::Result<()> {
        // 输入 pin 不主动连接：返回 E_NOTIMPL（出参无，安全）。
        Err(E_NOTIMPL.into())
    }
    fn ReceiveConnection(
        &self,
        pconnector: Ref<'_, IPin>,
        pmt: *const AM_MEDIA_TYPE,
    ) -> windows::core::Result<()> {
        let Some(connector) = pconnector.as_ref() else {
            return Err(E_POINTER.into());
        };
        if pmt.is_null() {
            return Err(windows::Win32::Media::DirectShow::VFW_E_TYPE_NOT_ACCEPTED.into());
        }
        let mt = unsafe { &*pmt };
        let Some(parsed) = parse_video_info(mt) else {
            // 不认识的格式（如 FORMAT_VideoInfo2 或非视频）：拒绝连接，让图管理器
            // 尝试其它格式或失败（明确返回错误，而非崩溃）。
            return Err(windows::Win32::Media::DirectShow::VFW_E_TYPE_NOT_ACCEPTED.into());
        };
        // 深拷贝媒体类型（pbFormat 走 CoTaskMemAlloc），独立于上游缓冲。
        let owned = OwnedMediaType::copy_from(mt);
        // 缓存对端 pin 与媒体类型，供 ConnectedTo / ConnectionMediaType 返回。
        *self.connected_to.lock().expect("pin conn lock poisoned") = Some(connector.clone());
        *self.connected_mt.lock().expect("pin mt lock poisoned") = owned;
        self.slot.store_media_type(parsed);
        Ok(())
    }
    fn Disconnect(&self) -> windows::core::Result<()> {
        *self.connected_to.lock().expect("pin conn lock poisoned") = None;
        *self.connected_mt.lock().expect("pin mt lock poisoned") = OwnedMediaType::empty();
        Ok(())
    }
    fn ConnectedTo(&self) -> windows::core::Result<IPin> {
        self.connected_to
            .lock()
            .expect("pin conn lock poisoned")
            .clone()
            .ok_or_else(|| windows::Win32::Media::DirectShow::VFW_E_NOT_CONNECTED.into())
    }
    fn ConnectionMediaType(&self, pmt: *mut AM_MEDIA_TYPE) -> windows::core::Result<()> {
        // 关键：必须写出参。未连接时返回 VFW_E_NOT_CONNECTED（shim Err 分支不写，但此时
        // 调用方按约定不应读 pmt，安全）。已连接时把缓存的深拷贝再深拷一份给调用方
        //（pbFormat 由调用方 CoTaskMemFree）。
        let owned = self.connected_mt.lock().expect("pin mt lock poisoned");
        if !owned.is_connected() {
            return Err(windows::Win32::Media::DirectShow::VFW_E_NOT_CONNECTED.into());
        }
        unsafe {
            // 先清零调用方的结构，避免残留指针。
            pmt.write(AM_MEDIA_TYPE::default());
            owned.write_into(&mut *pmt);
        }
        Ok(())
    }
    fn QueryPinInfo(
        &self,
        pinfo: *mut windows::Win32::Media::DirectShow::PIN_INFO,
    ) -> windows::core::Result<()> {
        // 关键：必须写整个 PIN_INFO，特别是 pFilter（AddRef 副本）与 achName。
        unsafe {
            (*pinfo).achName = [0u16; 128];
            let name: Vec<u16> = SinkPin::PIN_NAME
                .encode_utf16()
                .take(127)
                .chain(std::iter::once(0))
                .collect();
            let n = name.len().min(128);
            (&mut (*pinfo).achName)[..n].copy_from_slice(&name[..n]);
            (*pinfo).dir = PIN_DIRECTION(PINDIR_INPUT.0);
            (*pinfo).pFilter = core::mem::ManuallyDrop::new(
                self.filter
                    .lock()
                    .expect("pin filter lock poisoned")
                    .clone(),
            );
        }
        Ok(())
    }
    fn QueryDirection(&self) -> windows::core::Result<PIN_DIRECTION> {
        Ok(PIN_DIRECTION(PINDIR_INPUT.0))
    }
    fn QueryId(&self) -> windows::core::Result<PWSTR> {
        // 关键：必须返回 CoTaskMemAlloc 的 UTF-16 "In"。返回 Err 时 shim 不写出参，
        // 图管理器拿到栈垃圾 CoTaskMemFree → 段错误（前一版的崩溃根因之一）。
        alloc_co_pwstr(SinkPin::PIN_NAME)
    }
    fn QueryAccept(&self, _pmt: *const AM_MEDIA_TYPE) -> windows::core::HRESULT {
        // FFmpeg 同款：不预先筛选，让协商走 ReceiveConnection（那里做真实检查）。
        // 返回 S_OK 表示「可能接受」，触发后续 ReceiveConnection 提供具体格式。
        windows::core::HRESULT(0)
    }
    fn EnumMediaTypes(&self) -> windows::core::Result<IEnumMediaTypes> {
        // 关键：必须返回有效枚举器，不能 E_NOTIMPL。空枚举器表示「按上游的来」。
        Ok(windows::core::ComObject::new(EmptyEnumMediaTypes).into_interface())
    }
    fn QueryInternalConnections(
        &self,
        _appin: windows::core::OutRef<'_, IPin>,
        _npin: *mut u32,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EndOfStream(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn BeginFlush(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn EndFlush(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn NewSegment(&self, _tstart: i64, _tstop: i64, _drate: f64) -> windows::core::Result<()> {
        Ok(())
    }
}

impl IMemInputPin_Impl for SinkPin_Impl {
    fn GetAllocator(&self) -> windows::core::Result<IMemAllocator> {
        // VFW_E_NO_ALLOCATOR：让上游用默认 allocator（FFmpeg 同款）。
        Err(windows::Win32::Media::DirectShow::VFW_E_NO_ALLOCATOR.into())
    }
    fn NotifyAllocator(
        &self,
        _pallocator: Ref<'_, IMemAllocator>,
        _breadonly: windows::core::BOOL,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn GetAllocatorRequirements(
        &self,
    ) -> windows::core::Result<windows::Win32::Media::DirectShow::ALLOCATOR_PROPERTIES> {
        Err(E_NOTIMPL.into())
    }
    fn Receive(&self, psample: Ref<'_, IMediaSample>) -> windows::core::Result<()> {
        // 关键路径：流线程持续调用，拷贝帧数据到共享槽。
        let Some(sample) = psample.as_ref() else {
            return Ok(());
        };
        let len = unsafe { sample.GetActualDataLength() };
        if len <= 0 {
            return Ok(());
        }
        let ptr = match unsafe { sample.GetPointer() } {
            Ok(p) if !p.is_null() => p,
            _ => return Ok(()),
        };
        // 立即拷贝：IMediaSample 缓冲由 allocator 管理，Receive 返回后可能被复用。
        // store_frame 内部复用预分配缓冲，避免每帧 8MB 堆分配（实测：每帧 to_vec 会让
        // 流线程吃满一个核；复用缓冲后采集 CPU 降到个位数百分比）。
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        self.slot.store_frame(bytes);
        Ok(())
    }
    fn ReceiveMultiple(
        &self,
        psamples: *const Option<IMediaSample>,
        nsamples: i32,
    ) -> windows::core::Result<i32> {
        // 逐个拷数据到共享槽（与 Receive 同逻辑，FFmpeg 同款循环）。
        if psamples.is_null() || nsamples <= 0 {
            return Ok(0);
        }
        let mut processed = 0_i32;
        for i in 0..nsamples {
            // SAFETY: 调用方保证 psamples[..nsamples] 有效。
            if let Some(sample) = unsafe { &*psamples.offset(i as isize) } {
                let len = unsafe { sample.GetActualDataLength() };
                if len > 0 {
                    if let Ok(ptr) = unsafe { sample.GetPointer() } {
                        if !ptr.is_null() {
                            let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
                            self.slot.store_frame(bytes);
                        }
                    }
                }
                processed += 1;
            }
        }
        Ok(processed)
    }
    fn ReceiveCanBlock(&self) -> windows::core::Result<()> {
        // S_OK：可以阻塞（FFmpeg 同款）。返回 Ok 即 S_OK。
        Ok(())
    }
}

// ============================================================================
// OwnedMediaType：AM_MEDIA_TYPE 的深拷贝所有者。pbFormat 走 CoTaskMemAlloc 独立持有，
// 析构时 CoTaskMemFree。用于 ReceiveConnection 缓存、ConnectionMediaType 回填。
// pUnk 在 sink 场景恒为 NULL（视频媒体类型不带 IUnknown），复制时也置 NULL。
// ============================================================================

struct OwnedMediaType {
    majortype: windows::core::GUID,
    subtype: windows::core::GUID,
    bFixedSizeSamples: bool,
    bTemporalCompression: bool,
    lSampleSize: u32,
    formattype: windows::core::GUID,
    cbFormat: u32,
    /// CoTaskMemAlloc 分配的 cbFormat 字节；None 表示空（未连接）。
    pbFormat: Option<*mut u8>,
}

// pbFormat 是裸指针但由内部 Mutex 保护、单线程访问；Send/Sync 安全声明。
unsafe impl Send for OwnedMediaType {}
unsafe impl Sync for OwnedMediaType {}

impl OwnedMediaType {
    const fn empty() -> Self {
        Self {
            majortype: windows::core::GUID::zeroed(),
            subtype: windows::core::GUID::zeroed(),
            bFixedSizeSamples: false,
            bTemporalCompression: false,
            lSampleSize: 0,
            formattype: windows::core::GUID::zeroed(),
            cbFormat: 0,
            pbFormat: None,
        }
    }

    fn is_connected(&self) -> bool {
        self.pbFormat.is_some()
    }

    /// 深拷贝一个 `AM_MEDIA_TYPE`。pbFormat 走新的 CoTaskMemAlloc 分配。
    fn copy_from(src: &AM_MEDIA_TYPE) -> Self {
        use windows::Win32::System::Com::CoTaskMemAlloc;
        let pbFormat = if !src.pbFormat.is_null() && src.cbFormat > 0 {
            let p = unsafe { CoTaskMemAlloc(src.cbFormat as usize) };
            if !p.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.pbFormat,
                        p as *mut u8,
                        src.cbFormat as usize,
                    );
                }
                Some(p as *mut u8)
            } else {
                None
            }
        } else {
            None
        };
        Self {
            majortype: src.majortype,
            subtype: src.subtype,
            bFixedSizeSamples: src.bFixedSizeSamples.as_bool(),
            bTemporalCompression: src.bTemporalCompression.as_bool(),
            lSampleSize: src.lSampleSize,
            formattype: src.formattype,
            cbFormat: src.cbFormat,
            pbFormat,
        }
    }

    /// 把缓存的内容写进调用方的 `AM_MEDIA_TYPE`（pbFormat 用新的 CoTaskMemAlloc 副本，
    /// 调用方负责 CoTaskMemFree）。pUnk 置 NULL。
    fn write_into(&self, dst: &mut AM_MEDIA_TYPE) {
        use windows::Win32::System::Com::CoTaskMemAlloc;
        dst.majortype = self.majortype;
        dst.subtype = self.subtype;
        dst.bFixedSizeSamples = self.bFixedSizeSamples.into();
        dst.bTemporalCompression = self.bTemporalCompression.into();
        dst.lSampleSize = self.lSampleSize;
        dst.formattype = self.formattype;
        dst.pUnk = core::mem::ManuallyDrop::new(None);
        if let Some(src) = self.pbFormat {
            let p = unsafe { CoTaskMemAlloc(self.cbFormat as usize) };
            if !p.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(src, p as *mut u8, self.cbFormat as usize);
                }
                dst.cbFormat = self.cbFormat;
                dst.pbFormat = p as *mut u8;
            } else {
                dst.cbFormat = 0;
                dst.pbFormat = std::ptr::null_mut();
            }
        } else {
            dst.cbFormat = 0;
            dst.pbFormat = std::ptr::null_mut();
        }
    }
}

impl Drop for OwnedMediaType {
    fn drop(&mut self) {
        if let Some(p) = self.pbFormat.take() {
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p as *const _)) };
        }
    }
}

impl Default for OwnedMediaType {
    fn default() -> Self {
        Self::empty()
    }
}

// ============================================================================
// 像素格式转换：协商到的源格式 → BGRA8888（对外统一发布格式）。
// BT.601 整数公式（微软官方），有限范围 16-235（OBS 虚拟摄像头默认）。
// ============================================================================

/// 把协商到的源帧转成 BGRA8888。返回 (bgra 数据, 宽, 高)。None 表示格式不支持。
pub fn convert_to_bgra(src: &[u8], mt: &NegotiatedMediaType) -> Option<(Vec<u8>, u32, u32)> {
    let w = mt.width as usize;
    let h = mt.height as usize;
    let out_size = w * h * 4;
    let mut bgra = vec![0u8; out_size];
    match mt.format {
        NegotiatedFormat::Nv12 => {
            if src.len() < w * h * 3 / 2 {
                return None;
            }
            // NV12 是打包 YUV，数据始终 top-to-bottom 线性排列；biHeight 正负号对它
            // 无意义（很多驱动对 YUV 仍填正 biHeight）。绝不按 biHeight 翻转，否则画面头朝下。
            nv12_to_bgra_inner(src, w, h, true, &mut bgra);
        }
        NegotiatedFormat::Yuy2 => {
            // YUY2: packed 4:2:2，每 4 字节 [Y0 U Y1 V] = 2 像素。stride 行 = w*2。
            // 同 NV12，YUV 打包格式数据 top-to-bottom，不翻转。
            if src.len() < w * h * 2 {
                return None;
            }
            yuy2_to_bgra_inner(src, w, h, true, &mut bgra);
        }
        NegotiatedFormat::Rgb24 => {
            // BGR 字节序（DirectShow RGB24 = BGR 内存序）。未压缩 RGB 遵守 biHeight：
            // 正=bottom-up（需翻转），负=top-down。
            if src.len() < w * h * 3 {
                return None;
            }
            rgb24_to_bgra_inner(src, w, h, mt.top_down, &mut bgra);
        }
        NegotiatedFormat::Argb32 => {
            // 32bpp 未压缩 RGB：源是 BGRA/ARGB 字节序，遵守 biHeight 行序。
            if src.len() < w * h * 4 {
                return None;
            }
            copy_bgra_with_flip(src, w, h, mt.top_down, &mut bgra);
        }
    }
    Some((bgra, mt.width, mt.height))
}

/// NV12 (4:2:0, 12bpp) -> BGRA8888。src 布局：Y 平面（w*h）+ 交错 UV 平面（w*h/2）。
/// full_range: false=有限范围(16-235, OBS 默认)。top_down 决定输出行序。
fn nv12_to_bgra_inner(src: &[u8], w: usize, h: usize, top_down: bool, out: &mut [u8]) {
    let y_plane = &src[..w * h];
    let uv_plane = &src[w * h..];
    for row in 0..h {
        // 自底向上：源第 row 行写到输出第 (h-1-row) 行。
        let dst_row = if top_down { row } else { h - 1 - row };
        for x in 0..w {
            let y_val = y_plane[row * w + x] as i32;
            let uv_index = (row / 2) * w + (x / 2) * 2;
            let u = uv_plane[uv_index] as i32;
            let v = uv_plane[uv_index + 1] as i32;
            let (d, e) = (u - 128, v - 128);
            let c = y_val - 16;
            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            let o = dst_row * w * 4 + x * 4;
            out[o..o + 4].copy_from_slice(&[b, g, r, 255]);
        }
    }
}

/// YUY2 (packed 4:2:2) -> BGRA8888。每 4 字节 [Y0 U Y1 V] 产出 2 个像素。
fn yuy2_to_bgra_inner(src: &[u8], w: usize, h: usize, top_down: bool, out: &mut [u8]) {
    let stride = w * 2;
    for row in 0..h {
        let dst_row = if top_down { row } else { h - 1 - row };
        let line = &src[row * stride..][..stride];
        let mut x = 0;
        let mut i = 0;
        while x + 1 < w {
            let y0 = line[i] as i32;
            let u = line[i + 1] as i32;
            let y1 = line[i + 2] as i32;
            let v = line[i + 3] as i32;
            i += 4;
            let o0 = dst_row * w * 4 + x * 4;
            let o1 = o0 + 4;
            let (d, e) = (u - 128, v - 128);
            let c0 = y0 - 16;
            let c1 = y1 - 16;
            let r0 = ((298 * c0 + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g0 = ((298 * c0 - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b0 = ((298 * c0 + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            let r1 = ((298 * c1 + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g1 = ((298 * c1 - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b1 = ((298 * c1 + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            out[o0..o0 + 4].copy_from_slice(&[b0, g0, r0, 255]);
            out[o1..o1 + 4].copy_from_slice(&[b1, g1, r1, 255]);
            x += 2;
        }
        // 奇数宽度：最后一列复用前一对 UV。
        if x < w {
            let y0 = line[i.min(stride - 1)] as i32;
            let u = line[i.saturating_sub(3).min(stride - 3)] as i32;
            let v = line[i.saturating_sub(1).min(stride - 1)] as i32;
            let (d, e) = (u - 128, v - 128);
            let c0 = y0 - 16;
            let r0 = ((298 * c0 + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g0 = ((298 * c0 - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b0 = ((298 * c0 + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            let o0 = dst_row * w * 4 + x * 4;
            out[o0..o0 + 4].copy_from_slice(&[b0, g0, r0, 255]);
        }
    }
}

/// RGB24 (BGR 内存序) -> BGRA8888，alpha=255。
fn rgb24_to_bgra_inner(src: &[u8], w: usize, h: usize, top_down: bool, out: &mut [u8]) {
    let src_stride = w * 3;
    for row in 0..h {
        let dst_row = if top_down { row } else { h - 1 - row };
        let line = &src[row * src_stride..][..src_stride];
        for x in 0..w {
            let si = x * 3;
            let di = dst_row * w * 4 + x * 4;
            // DirectShow RGB24 内存序为 B,G,R。
            out[di] = line[si];
            out[di + 1] = line[si + 1];
            out[di + 2] = line[si + 2];
            out[di + 3] = 255;
        }
    }
}

/// 32bpp BGRA/ARGB 直接拷贝（仅处理行序翻转）。
fn copy_bgra_with_flip(src: &[u8], w: usize, h: usize, top_down: bool, out: &mut [u8]) {
    let stride = w * 4;
    if top_down {
        out[..w * h * 4].copy_from_slice(&src[..w * h * 4]);
    } else {
        for row in 0..h {
            let dst_row = h - 1 - row;
            out[dst_row * stride..dst_row * stride + stride]
                .copy_from_slice(&src[row * stride..row * stride + stride]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个 4×2 的 NV12 帧：第 0 行 Y=235（亮），第 1 行 Y=16（暗），UV 全 128（无色度）。
    /// 用于验证 YUV 打包格式转换后输出顶部仍是亮行（绝不被 biHeight 翻转）。
    fn nv12_top_bright_4x2() -> Vec<u8> {
        // 4×2 NV12：Y 平面 8 字节（第 0 行=235 亮，第 1 行=16 暗）+ UV 平面 4 字节（全 128）。
        let mut buf = vec![0u8; 4 * 2 * 3 / 2];
        buf[0..4].fill(235); // Y 第 0 行（源顶）：亮
        buf[4..8].fill(16); //  Y 第 1 行（源底）：暗
        buf[8..12].fill(128); // UV 全中性（无色度）
        buf
    }

    /// 取 BGRA 输出中某像素的亮度（R+G+B 之和，越大越亮）。
    fn bgra_luminance(out: &[u8], w: usize, row: usize, x: usize) -> u32 {
        let o = (row * w + x) * 4;
        u32::from(out[o]) + u32::from(out[o + 1]) + u32::from(out[o + 2])
    }

    #[test]
    fn nv12_ignores_top_down_flag_always_top_first() {
        // 回归：YUV 打包格式数据始终 top-to-bottom，绝不能按 biHeight 翻转，
        // 否则 OBS 虚拟摄像头画面头朝下。
        let src = nv12_top_bright_4x2();
        let mt_bright_top = NegotiatedMediaType {
            width: 4,
            height: 2,
            top_down: false, // biHeight 正（曾被误判为 bottom-up）
            format: NegotiatedFormat::Nv12,
        };
        let mt_explicit_top = NegotiatedMediaType {
            width: 4,
            height: 2,
            top_down: true,
            format: NegotiatedFormat::Nv12,
        };
        let (out_flag, _, _) = convert_to_bgra(&src, &mt_bright_top).expect("convert");
        let (out_top, _, _) = convert_to_bgra(&src, &mt_explicit_top).expect("convert");
        // 两种 top_down 值下，输出第 0 行都必须是亮行（不被翻转）。
        assert!(
            bgra_luminance(&out_flag, 4, 0, 0) > bgra_luminance(&out_flag, 4, 1, 0),
            "top_down=false 时第 0 行应更亮（NV12 不翻转）"
        );
        assert!(
            bgra_luminance(&out_top, 4, 0, 0) > bgra_luminance(&out_top, 4, 1, 0),
            "top_down=true 时第 0 行应更亮"
        );
        // 两者结果必须一致（top_down 对 NV12 无影响）。
        assert_eq!(out_flag, out_top, "NV12 转换不应受 top_down 影响");
    }

    #[test]
    fn rgb24_respects_top_down_flag_for_uncompressed_rgb() {
        // 对照：未压缩 RGB24 遵守 biHeight，top_down=false（正 biHeight）时需翻转行序。
        // 构造 2×2 RGB24：内存里第 0 行（源顶）亮、第 1 行（源底）暗。
        let mut src = vec![0u8; 2 * 2 * 3];
        // 源第 0 行（前 6 字节，2 像素 BGR）= 亮白
        src[0..6].fill(255);
        // 源第 1 行（后 6 字节）= 暗
        src[6..12].fill(0);
        let mt_bottom_up = NegotiatedMediaType {
            width: 2,
            height: 2,
            top_down: false, // bottom-up：输出应把源第 0 行翻到底部
            format: NegotiatedFormat::Rgb24,
        };
        let (out, _, _) = convert_to_bgra(&src, &mt_bottom_up).expect("convert");
        // bottom-up：输出第 1 行应是源第 0 行（亮），第 0 行是源第 1 行（暗）。
        assert!(
            bgra_luminance(&out, 2, 1, 0) > bgra_luminance(&out, 2, 0, 0),
            "RGB24 bottom-up 应翻转：输出第 1 行更亮"
        );
    }

    #[test]
    fn alloc_co_pwstr_roundtrips_in() {
        // QueryId 必须返回可被 CoTaskMemFree 的 UTF-16 "In"。
        let pw = alloc_co_pwstr("In").expect("alloc");
        unsafe {
            let s = pw.to_string().expect("utf16");
            assert_eq!(s, "In");
            windows::Win32::System::Com::CoTaskMemFree(Some(pw.as_ptr() as *const _));
        }
    }

    #[test]
    fn empty_enum_media_types_next_returns_s_false() {
        let e: IEnumMediaTypes =
            windows::core::ComObject::new(EmptyEnumMediaTypes).into_interface();
        let mut fetched = 1_u32;
        let mut out: *mut AM_MEDIA_TYPE = std::ptr::null_mut();
        // IEnumMediaTypes::Next 的接口签名：(&mut [*mut AM_MEDIA_TYPE], Option<*mut u32>)。
        let hr = unsafe {
            e.Next(
                std::slice::from_mut(&mut out),
                Some(&mut fetched as *mut u32),
            )
        };
        assert_eq!(hr, S_FALSE);
        assert_eq!(fetched, 0);
    }

    #[test]
    fn slot_stores_and_reads_frame() {
        let slot = SinkFrameSlot::new();
        let mut buf = Vec::new();
        // 无帧时阻塞等待应超时（而非立即返回）。
        assert!(matches!(
            slot.next_frame_into(std::time::Duration::from_millis(20), &mut buf),
            NextFrame::Timeout
        ));
        slot.store_media_type(NegotiatedMediaType {
            width: 2,
            height: 2,
            top_down: false,
            format: NegotiatedFormat::Rgb24,
        });
        slot.store_frame(&[1, 2, 3]);
        // store_frame 立即唤醒，应直接取到帧。
        match slot.next_frame_into(std::time::Duration::from_millis(20), &mut buf) {
            NextFrame::Frame {
                len,
                media_type: mt,
            } => {
                assert_eq!(len, 3);
                assert_eq!(&buf[..len], &[1, 2, 3]);
                assert_eq!(mt.width, 2);
                assert_eq!(mt.format, NegotiatedFormat::Rgb24);
            }
            other => panic!("expected Frame, got {other:?}"),
        }
        // 消费语义：取出后再次等待应超时（无新帧）。
        assert!(matches!(
            slot.next_frame_into(std::time::Duration::from_millis(20), &mut buf),
            NextFrame::Timeout
        ));
    }

    #[test]
    fn slot_stop_unblocks_next_frame() {
        let slot = SinkFrameSlot::new();
        let slot_clone = slot.clone();
        let handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            // 长超时阻塞等待。
            slot_clone.next_frame_into(std::time::Duration::from_secs(10), &mut buf)
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        slot.stop();
        // stop 后阻塞的 next_frame_into 应迅速返回 Stopped（被 notify_all 唤醒）。
        let result = handle.join().expect("thread");
        assert!(matches!(result, NextFrame::Stopped));
    }

    #[test]
    fn parse_video_info_rejects_non_video() {
        let mt = AM_MEDIA_TYPE {
            majortype: windows::core::GUID::zeroed(),
            ..Default::default()
        };
        assert!(parse_video_info(&mt).is_none());
    }
}
