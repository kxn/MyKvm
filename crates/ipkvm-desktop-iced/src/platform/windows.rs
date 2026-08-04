//! Windows Raw Input 相对鼠标源。
//!
//! 隐藏消息窗口 + `RegisterRawInputDevices(RIDEV_INPUTSINK)`：无论窗口是否前台，
//! 都能收到原始鼠标移动增量（`WM_INPUT` → `RAWMOUSE.lLastX/lLastY`），
//! 这正是 winit `DeviceEvent::MouseMotion` 在 Windows 上的底层实现。
//! 线程退出：`stop()` 投递自定义消息 → WNDPROC `PostQuitMessage`。
//!
//! 单实例限制：WNDPROC 是进程级静态回调，通过全局 Sender 转发增量
//! （当前实现满足单实例需求；如需多实例再改为按窗口参数寻址）。

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, SyncSender, channel, sync_channel};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_RELATIVE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RAWMOUSE, RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassExW, TranslateMessage, UnregisterClassW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_INPUT, WNDCLASSEXW,
};
use windows::core::{PCWSTR, w};

use crate::relative::{DeltaReceiver, RelativePointerSource};

const WM_APP_EXIT: u32 = 0x8000 + 1;

static TX: Mutex<Option<Sender<(i16, i16)>>> = Mutex::new(None);

pub struct WindowsRawInput {
    started: bool,
    /// 本实例消息窗口句柄（HWND 非 Send/Sync，这里存 isize）。
    hwnd: Option<isize>,
    handle: Option<JoinHandle<()>>,
}

impl WindowsRawInput {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            started: false,
            hwnd: None,
            handle: None,
        })
    }
}

impl RelativePointerSource for WindowsRawInput {
    fn receiver(&mut self) -> Result<DeltaReceiver, String> {
        if self.started {
            return Err("raw input source already started".into());
        }
        let mut guard = TX.lock().map_err(|_| "raw input TX poisoned".to_string())?;
        if guard.is_some() {
            return Err("another raw input source is active".into());
        }
        let (tx, rx) = channel();
        *guard = Some(tx);
        drop(guard);

        let (init_tx, init_rx) = sync_channel::<Result<isize, String>>(1);
        let handle = thread::Builder::new()
            .name("raw-input".into())
            .spawn(move || raw_input_thread(init_tx))
            .map_err(|e| format!("spawn raw input thread: {e}"))?;

        let hwnd = init_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| format!("raw input init timeout: {e}"))??;

        self.started = true;
        self.hwnd = Some(hwnd);
        self.handle = Some(handle);
        Ok(rx)
    }

    fn stop(&mut self) {
        if let Some(raw) = self.hwnd.take() {
            let hwnd = HWND(raw as *mut c_void);
            let _ = unsafe { PostMessageW(Some(hwnd), WM_APP_EXIT, WPARAM(0), LPARAM(0)) };
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.started = false;
        *TX.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

impl Drop for WindowsRawInput {
    fn drop(&mut self) {
        self.stop();
    }
}

fn raw_input_thread(init_tx: SyncSender<Result<isize, String>>) {
    let result = (|| -> Result<isize, String> {
        let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
            .map_err(|e| format!("GetModuleHandleW: {e}"))?
            .into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: Default::default(),
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: w!("IpkvmRawInputClass"),
            hIconSm: Default::default(),
        };
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err("RegisterClassExW failed".into());
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("IpkvmRawInputClass"),
                w!("ipkvm raw input"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance),
                None,
            )
        }
        .map_err(|e| format!("CreateWindowExW: {e}"))?;

        let device = RAWINPUTDEVICE {
            usUsagePage: 0x01, // HID_USAGE_PAGE_GENERIC
            usUsage: 0x02,     // HID_USAGE_GENERIC_MOUSE
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32) }
            .map_err(|e| format!("RegisterRawInputDevices: {e}"))?;

        Ok(hwnd.0 as isize)
    })();

    let hwnd_raw = match result {
        Ok(raw) => {
            let _ = init_tx.send(Ok(raw));
            raw
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    unsafe {
        let _ = DestroyWindow(HWND(hwnd_raw as *mut c_void));
        let hinstance: Option<HINSTANCE> = GetModuleHandleW(None).ok().map(Into::into);
        if let Some(hinstance) = hinstance {
            let _ = UnregisterClassW(w!("IpkvmRawInputClass"), Some(hinstance));
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_INPUT => {
            if let Some(tx) = TX.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
                && let Some((dx, dy)) = unsafe { read_mouse_delta(lparam) }
            {
                let _ = tx.send((dx, dy));
            }
            LRESULT(0)
        }
        WM_APP_EXIT => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 解析 WM_INPUT：相对移动的鼠标原始增量。
unsafe fn read_mouse_delta(lparam: LPARAM) -> Option<(i16, i16)> {
    let handle = HRAWINPUT(lparam.0 as *mut c_void);
    let mut size: u32 = 0;
    unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            None,
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        );
    }
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let written = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if written == 0 || written == u32::MAX {
        return None;
    }
    let raw = unsafe { &*(buf.as_ptr() as *const RAWINPUT) };
    if raw.header.dwType != RIM_TYPEMOUSE.0 {
        return None;
    }
    let mouse: RAWMOUSE = unsafe { raw.data.mouse };
    if mouse.usFlags != MOUSE_MOVE_RELATIVE {
        return None;
    }
    let (dx, dy) = (mouse.lLastX, mouse.lLastY);
    if dx == 0 && dy == 0 {
        return None;
    }
    Some((dx as i16, dy as i16))
}
