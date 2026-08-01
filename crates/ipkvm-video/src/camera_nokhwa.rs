//! Linux/macOS 相机后端（nokhwa）。Windows 不编译（Windows 用 DirectShow sink filter）。
//!
//! 用 nokhwa 0.10 的顶层 `Camera` 抽象：Linux 走 V4L2、macOS 走 AVFoundation。
//! 这两个平台的虚拟摄像头（如 OBS、v4l2loopback）能被原生枚举到，不像 Windows 的
//! Media Foundation 列不出 OBS 虚拟摄像头。
//!
//! `Camera::frame()` 在驱动有帧时返回（阻塞语义，天然事件驱动，无轮询、无 CPU 空转）。
//! 读到的 `Buffer` 用 `RgbAFormat::decode_image_to_buffer` 转成 RGBA，再重排成 BGRA8888
//! 对外发布（与 Windows 后端输出格式一致）。

#![cfg(all(unix, feature = "mf"))]

use std::sync::{Arc, RwLock};

use nokhwa::{
    Camera,
    pixel_format::RgbAFormat,
    query,
    utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType},
};
use tokio::sync::watch;

use crate::camera::{CameraDeviceInfo, CameraSourceError};
use crate::{
    FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, SharedVideoFrame, VideoFrame,
};

/// nokhwa 相机帧源。`open` 成功后由后台采集线程持续发布帧。
///
/// 停止：drop 时置位停止标志，采集线程在下次 `frame()` 返回（或被停止唤醒）后退出并
/// drop `Camera`（释放 V4L2/AVFoundation 设备句柄）。
#[derive(Debug)]
pub struct CameraSource {
    latest: Arc<RwLock<Option<SharedVideoFrame>>>,
    sender: watch::Sender<Option<SharedVideoFrame>>,
    name: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    _handle: Option<std::thread::JoinHandle<()>>,
}

impl CameraSource {
    pub fn open(device_id: &str, frames_per_second: u64) -> Result<Self, CameraSourceError> {
        if frames_per_second == 0 {
            return Err(CameraSourceError::ZeroFramesPerSecond);
        }
        // 设备匹配：优先精确匹配 id，其次匹配显示名（与 Windows 后端一致）。
        let devices = list_cameras()?;
        let chosen = devices
            .iter()
            .find(|d| d.id == device_id || d.display_name == device_id)
            .ok_or_else(|| {
                CameraSourceError::Open(device_id.to_owned(), "device not found".into())
            })?;
        let name = chosen.display_name.clone();
        let cam_index = parse_camera_index(&chosen.id);

        let latest = Arc::new(RwLock::new(None));
        let (sender, _receiver) = watch::channel(None);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // nokhwa 的 Camera 内部持 Box<dyn CaptureBackendTrait>（非 Send），不能跨线程 move，
        // 所以在采集线程内创建 + open_stream + 持续 frame()。用 sync_channel 把初始化结果
        // 送回 open（失败则透传错误），open 返回后线程继续跑采集循环。
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
        let task_latest = Arc::clone(&latest);
        let task_sender = sender.clone();
        let task_stop = Arc::clone(&stop);
        let init_name = name.clone();
        let handle = std::thread::Builder::new()
            .name("camera-nokhwa".into())
            .spawn(move || {
                // 线程内创建相机：RequestedFormat::None 让驱动自选格式；RgbAFormat 仅用于
                // 后续 decode 输出，不影响捕获协商。
                let req = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None);
                let mut camera = match Camera::new(cam_index, req) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = init_tx.send(Err(format!("nokhwa open: {e}")));
                        return;
                    }
                };
                if let Err(e) = camera.open_stream() {
                    let _ = init_tx.send(Err(format!("open_stream: {e}")));
                    return;
                }
                // 初始化成功：通知 open 可以返回了。
                let _ = init_tx.send(Ok(()));
                let started = std::time::Instant::now();
                let mut seq = 0_u64;
                let mut rgba_buf: Vec<u8> = Vec::new();
                loop {
                    if task_stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let frame = match camera.frame() {
                        Ok(f) => f,
                        Err(_) => {
                            // 读帧错误（设备断开等）：短暂退避后重试，避免忙转；
                            // 持续失败由上层「长时间无新帧」感知。
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            continue;
                        }
                    };
                    let res = frame.resolution();
                    // 解码到复用缓冲（容量稳定后零分配）。
                    rgba_buf.clear();
                    rgba_buf.resize((res.width_x as usize) * (res.height_y as usize) * 4, 0);
                    if frame
                        .decode_image_to_buffer::<RgbAFormat>(&mut rgba_buf)
                        .is_err()
                    {
                        // 解码失败（罕见，如不支持的源格式）：丢弃这一帧。
                        continue;
                    }
                    // RGBA -> BGRA8888（R↔B 交换），alpha 保持 255。
                    for px in rgba_buf.chunks_exact_mut(4) {
                        px.swap(0, 2);
                        px[3] = 255;
                    }
                    seq = seq.saturating_add(1);
                    let video_frame = VideoFrame::new(
                        seq,
                        MonotonicTimestamp::from_nanos(
                            started.elapsed().as_nanos().try_into().unwrap_or(u64::MAX),
                        ),
                        res.width_x,
                        res.height_y,
                        res.width_x * 4,
                        PixelFormat::Bgra8888,
                        Arc::from(rgba_buf.as_slice()),
                    );
                    let shared = Arc::new(video_frame);
                    *task_latest.write().expect("camera lock poisoned") = Some(Arc::clone(&shared));
                    task_sender.send_replace(Some(shared));
                }
            })
            .map_err(|e| CameraSourceError::Open(name.clone(), format!("spawn: {e}")))?;

        // 等待线程内相机初始化完成（最多 5 秒）。
        match init_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                // 让线程退出（stop 置位 + 等它结束）。
                stop.store(true, std::sync::atomic::Ordering::Release);
                let _ = handle.join();
                return Err(CameraSourceError::Open(init_name, msg));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                stop.store(true, std::sync::atomic::Ordering::Release);
                let _ = handle.join();
                return Err(CameraSourceError::Open(
                    init_name,
                    "camera initialization timed out".into(),
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = handle.join();
                return Err(CameraSourceError::Open(
                    init_name,
                    "camera thread exited during init".into(),
                ));
            }
        }

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
        // 置位停止标志；采集线程在 frame() 返回后检测到并退出，释放设备句柄。
        self.stop.store(true, std::sync::atomic::Ordering::Release);
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

    fn source_info(&self) -> crate::VideoSourceInfo {
        crate::VideoSourceInfo {
            kind: crate::VideoSourceKind::Camera,
            device_name: self.name.clone(),
            is_loop: false,
        }
    }
}

/// id 形如 "{index}:{display_name}"；取冒号前的数字作 CameraIndex::Index，
/// 解析失败或不含冒号时按字符串路径处理（V4L2 也接受 /dev/videoN 路径）。
fn parse_camera_index(id: &str) -> CameraIndex {
    match id.split_once(':').and_then(|(n, _)| n.parse::<u32>().ok()) {
        Some(idx) => CameraIndex::Index(idx),
        None => CameraIndex::String(id.to_owned()),
    }
}

/// 枚举视频采集设备。用 nokhwa 的 query(ApiBackend::Auto) 让库按平台选后端
/// （Linux=V4L2，macOS=AVFoundation）。
pub fn list_cameras() -> Result<Vec<CameraDeviceInfo>, CameraSourceError> {
    let devices = query(ApiBackend::Auto)
        .map_err(|e| CameraSourceError::Enumerate(format!("nokhwa query: {e}")))?;
    Ok(devices
        .into_iter()
        .enumerate()
        .map(|(i, info)| CameraDeviceInfo {
            // id 复用 Windows 后端的 "{index}:{name}" 约定，便于 open 时回查。
            id: format!("{i}:{}", info.human_name()),
            display_name: info.human_name(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_open_with_zero_fps_rejected() {
        let err = CameraSource::open("nonexistent", 0).unwrap_err();
        assert!(matches!(err, CameraSourceError::ZeroFramesPerSecond));
    }

    #[test]
    fn parse_camera_index_handles_indexed_id() {
        let CameraIndex::Index(n) = parse_camera_index("0:OBS") else {
            panic!("expected Index");
        };
        assert_eq!(n, 0);
    }

    #[test]
    fn parse_camera_index_falls_back_to_string() {
        let CameraIndex::String(s) = parse_camera_index("not-a-number") else {
            panic!("expected String");
        };
        assert_eq!(s, "not-a-number");
    }
}
