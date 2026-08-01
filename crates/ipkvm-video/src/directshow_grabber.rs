//! DirectShow Sample Grabber 的手写 COM 绑定与封装。
//!
//! qedit.h 自 Windows 7 SDK 起被移除，windows crate 不包含 `ISampleGrabber`。
//! 这里用 `windows-interface` 手工声明 `ISampleGrabber`（vtable 顺序与 Wine qedit.idl /
//! 微软 ABI 对齐）、`windows-implement` 实现 `ISampleGrabberCB` 回调。
//! 封装成 `SampleGrabberFilter`：建图用 `ICaptureGraphBuilder2` 直连
//! 捕获 filter → Sample Grabber → Null Renderer，读帧用**回调模式**
//! （`SetCallback(1)` = BufferCB，回调在流线程触发、拷贝帧到共享槽，
//! 采集线程读共享槽——避免缓冲模式 `GetCurrentBuffer` 可能阻塞的问题）。
//!
//! COM 方法名按惯例保持 PascalCase，抑制 snake_case 警告。

#![cfg(windows)]
#![allow(non_snake_case)]

use std::sync::{Arc, Mutex};

use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, MEDIASUBTYPE_NV12, MEDIATYPE_Video, VIDEOINFOHEADER,
};
use windows::core::Interface;

/// ISampleGrabber IID：{6B652FFF-11FE-4FCE-92AD-0266B5D7C78F}
#[windows_interface::interface("6B652FFF-11FE-4FCE-92AD-0266B5D7C78F")]
unsafe trait ISampleGrabber: windows_core::IUnknown {
    unsafe fn SetOneShot(&self, one_shot: i32) -> windows::core::Result<()>;
    unsafe fn SetMediaType(&self, media_type: *const AM_MEDIA_TYPE) -> windows::core::Result<()>;
    unsafe fn GetConnectedMediaType(
        &self,
        media_type: *mut AM_MEDIA_TYPE,
    ) -> windows::core::Result<()>;
    unsafe fn SetBufferSamples(&self, buffer: i32) -> windows::core::Result<()>;
    unsafe fn GetCurrentBuffer(
        &self,
        size: *mut i32,
        buffer: *mut i32,
    ) -> windows::core::Result<()>;
    unsafe fn GetCurrentSample(
        &self,
        sample: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()>;
    unsafe fn SetCallback(
        &self,
        callback: *mut core::ffi::c_void,
        method: i32,
    ) -> windows::core::Result<()>;
}

/// ISampleGrabberCB IID：{0579154A-2B53-4994-B0D0-E773148EFF85}
#[windows_interface::interface("0579154A-2B53-4994-B0D0-E773148EFF85")]
unsafe trait ISampleGrabberCB: windows_core::IUnknown {
    unsafe fn SampleCB(
        &self,
        sample_time: f64,
        sample: *mut core::ffi::c_void,
    ) -> windows::core::Result<()>;
    unsafe fn BufferCB(
        &self,
        sample_time: f64,
        buffer: *mut u8,
        buffer_len: i32,
    ) -> windows::core::Result<()>;
}

/// CLSID_SampleGrabber：{C1F400A0-3F08-11D3-9F0B-006008039E37}
#[allow(non_upper_case_globals)]
pub const CLSID_SampleGrabber: windows::core::GUID =
    windows::core::GUID::from_u128(0xC1F400A0_3F08_11D3_9F0B_006008039E37);
/// CLSID_NullRenderer：{C1F400A4-3F08-11D3-9F0B-006008039E37}
#[allow(non_upper_case_globals)]
pub const CLSID_NullRenderer: windows::core::GUID =
    windows::core::GUID::from_u128(0xC1F400A4_3F08_11D3_9F0B_006008039E37);

/// BufferCB 回调实现：流线程上被调用，立即拷贝帧到共享槽（不阻塞、不持有指针）。
#[windows_implement::implement(ISampleGrabberCB)]
struct SampleGrabberCallback {
    latest: Arc<Mutex<Option<Vec<u8>>>>,
}

impl ISampleGrabberCB_Impl for SampleGrabberCallback_Impl {
    unsafe fn SampleCB(
        &self,
        _sample_time: f64,
        _sample: *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        // 未使用（我们用 BufferCB）
        Ok(())
    }

    unsafe fn BufferCB(
        &self,
        _sample_time: f64,
        buffer: *mut u8,
        buffer_len: i32,
    ) -> windows::core::Result<()> {
        if buffer.is_null() || buffer_len <= 0 {
            return Ok(());
        }
        // 立即拷贝（pBuffer 是原始数据指针，回调返回后失效）
        let bytes = unsafe { std::slice::from_raw_parts(buffer, buffer_len as usize) };
        *self.latest.lock().expect("grabber callback lock poisoned") = Some(bytes.to_vec());
        Ok(())
    }
}

/// Sample Grabber 封装：持有 filter、grabber、回调，暴露建图所需 + 读帧所需。
pub struct SampleGrabberFilter {
    filter: windows::Win32::Media::DirectShow::IBaseFilter,
    grabber: ISampleGrabber,
    /// 回调对象（ComObject 计数指针，须保持存活，否则流线程调用悬垂指针崩溃）。
    _callback: windows::core::ComObject<SampleGrabberCallback>,
    /// 回调写入的最新帧。
    latest: Arc<Mutex<Option<Vec<u8>>>>,
    /// 协商后的帧尺寸（来自 GetConnectedMediaType 的 VIDEOINFOHEADER.bmiHeader）。
    size: Option<(u32, u32)>,
}

impl SampleGrabberFilter {
    pub fn new() -> windows::core::Result<Self> {
        use windows::Win32::System::Com::{CLSCTX, CoCreateInstance};
        let filter: windows::Win32::Media::DirectShow::IBaseFilter =
            unsafe { CoCreateInstance(&CLSID_SampleGrabber, None, CLSCTX(1)) }?;
        let grabber: ISampleGrabber = filter.cast()?;
        let latest = Arc::new(Mutex::new(None));
        let callback = windows::core::ComObject::new(SampleGrabberCallback {
            latest: Arc::clone(&latest),
        });
        Ok(Self {
            filter,
            grabber,
            _callback: callback,
            latest,
            size: None,
        })
    }

    pub fn as_filter(&self) -> &windows::Win32::Media::DirectShow::IBaseFilter {
        &self.filter
    }

    /// 注册 BufferCB 回调（方法 1）。在连接成功后、Run 之前调用。
    pub fn set_callback(&self) -> windows::core::Result<()> {
        // ComObject::into_interface 消费 self，转成 ISampleGrabberCB 拿原始指针。
        let cb: ISampleGrabberCB = self._callback.clone().into_interface();
        unsafe { self.grabber.SetCallback(cb.as_raw() as *mut _, 1) }
    }

    /// 限定协商到 NV12（OBS 虚拟摄像头默认偏好 NV12 输出，Sample Grabber 需匹配）。
    pub fn request_nv12(&self) -> windows::core::Result<()> {
        let mt = AM_MEDIA_TYPE {
            majortype: MEDIATYPE_Video,
            subtype: MEDIASUBTYPE_NV12,
            formattype: windows::core::GUID::zeroed(),
            ..Default::default()
        };
        unsafe { self.grabber.SetMediaType(&mt) }
    }

    /// 建图完成后读取协商的帧尺寸（VIDEOINFOHEADER.bmiHeader.biWidth/biHeight）。
    pub fn negotiated_size(&mut self) -> Option<(u32, u32)> {
        if let Some(size) = self.size {
            return Some(size);
        }
        let mut mt = AM_MEDIA_TYPE::default();
        if unsafe { self.grabber.GetConnectedMediaType(&mut mt) }.is_err() {
            return None;
        }
        // GetConnectedMediaType 返回的格式块需要释放（pbFormat 由方法分配）。
        let size = if mt.formattype != windows::core::GUID::zeroed() && !mt.pbFormat.is_null() {
            unsafe {
                let vih = &*(mt.pbFormat as *const VIDEOINFOHEADER);
                let w = vih.bmiHeader.biWidth.max(0) as u32;
                let h = vih.bmiHeader.biHeight.max(0) as u32;
                Some((w, h))
            }
        } else {
            None
        };
        // 释放：CoTaskMemFree(pbFormat) + CoTaskMemFree(struct)
        if !mt.pbFormat.is_null() {
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(mt.pbFormat as *const _)) };
        }
        self.size = size;
        size
    }

    /// 读最新一帧 NV12 数据（回调模式，非阻塞）。返回 None 表示暂无新帧。
    pub fn current_buffer(&self) -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = self.size?;
        let latest = self.latest.lock().expect("grabber lock poisoned").clone()?;
        Some((latest, w, h))
    }
}

/// NV12 (4:2:0, 12bpp) -> BGRA8888。src 布局：Y 平面（width*height 字节）+
/// 交错 UV 平面（width*height/2 字节，每 2 字节 [U V] 覆盖 2x2 像素块）。
/// full_range: false = 有限范围(16-235, OBS 默认), true = 全范围(0-255, 常见于 UVC)。
/// 用微软官方 BT.601 整数公式：C=Y-16, D=U-128, E=V-128,
/// R=clip((298C+409E+128)>>8), G=clip((298C-100D-208E+128)>>8), B=clip((298C+516D+128)>>8)。
pub fn nv12_to_bgra(src: &[u8], width: usize, height: usize, full_range: bool, out: &mut [u8]) {
    debug_assert!(src.len() >= width * height * 3 / 2);
    debug_assert!(out.len() >= width * height * 4);
    let y_plane = &src[..width * height];
    let uv_plane = &src[width * height..];
    for y in 0..height {
        for x in 0..width {
            let y_val = y_plane[y * width + x] as i32;
            // UV 交错：每 2x2 像素块共享一对 U/V，UV 行 = y/2
            let uv_index = (y / 2) * width + (x / 2) * 2;
            let u = uv_plane[uv_index] as i32;
            let v = uv_plane[uv_index + 1] as i32;
            let (d, e) = (u - 128, v - 128);
            let c = if full_range { y_val } else { y_val - 16 };
            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            let o = y * width * 4 + x * 4;
            out[o..o + 4].copy_from_slice(&[b, g, r, 255]);
        }
    }
}
