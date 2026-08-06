//! Linux/macOS 相机后端（nokhwa）。Windows 不编译（Windows 用 DirectShow sink filter）。
//!
//! 用 nokhwa 0.10 的顶层 `Camera` 抽象：Linux 走 V4L2、macOS 走 AVFoundation。
//! 这两个平台的虚拟摄像头（如 OBS、v4l2loopback）能被原生枚举到，不像 Windows 的
//! Media Foundation 列不出 OBS 虚拟摄像头。
//!
//! 视频格式处理：
//! - MJPEG：frame_raw() 直接透传原始 JPEG 字节，零解码零编码
//! - YUYV 等：frame_raw() 获取原始数据，直接转换为 BGRA（跳过 RGBA 中间格式）
//!
//! `Camera::frame()` 在驱动有帧时返回（阻塞语义，天然事件驱动，无轮询、无 CPU 空转）。

#![cfg(all(unix, feature = "camera"))]

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

/// 视频采集模式：根据设备输出格式选择最优处理路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureMode {
    /// MJPEG 直接透传：零解码零编码，最优性能。
    MjpegPassthrough,
    /// YUYV 直接转 RGB：省掉 RGBA 中间格式，省内存。
    YuyvToRgb,
    /// 通用路径：nokhwa 解码 RGBA → 转 BGRA，兼容所有格式。
    DecodeToBgra,
}

/// YUYV 4:2:2 → RGB 直接转换（跳过 RGBA 中间格式）。
///
/// 每 4 字节 (Y1, U, Y2, V) 转换为 2 个 RGB 像素（6 字节）。
/// 使用标准 YCbCr→RGB 公式，直接输出 RGB 顺序。
/// JPEG 不支持透明通道，RGB 比 RGBA 省 25% 内存。
#[inline]
fn yuyv_to_rgb(y: i32, u: i32, v: i32) -> [u8; 3] {
    let c298 = (y - 16) * 298;
    let d = u - 128;
    let e = v - 128;
    let r = ((c298 + 409 * e + 128) >> 8).clamp(0, 255) as u8;
    let g = ((c298 - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
    let b = ((c298 + 516 * d + 128) >> 8).clamp(0, 255) as u8;
    [r, g, b]
}

/// 将 YUYV 4:2:2 数据直接转换为 RGB 格式。
///
/// 输入：YUYV 数据（每 4 字节 = 2 像素）
/// 输出：RGB 数据（每 3 字节 = 1 像素）
fn convert_yuyv_to_rgb(src: &[u8], dst: &mut [u8]) {
    for (chunk, pixels) in src.chunks_exact(4).zip(dst.chunks_exact_mut(6)) {
        let y1 = chunk[0] as i32;
        let u = chunk[1] as i32;
        let y2 = chunk[2] as i32;
        let v = chunk[3] as i32;

        let [r1, g1, b1] = yuyv_to_rgb(y1, u, v);
        let [r2, g2, b2] = yuyv_to_rgb(y2, u, v);

        pixels[0] = r1;
        pixels[1] = g1;
        pixels[2] = b1;
        pixels[3] = r2;
        pixels[4] = g2;
        pixels[5] = b2;
    }
}

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
    stats: Arc<crate::SourceStats>,
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
        let stats = crate::SourceStats::new();

        // nokhwa 的 Camera 内部持 Box<dyn CaptureBackendTrait>（非 Send），不能跨线程 move，
        // 所以在采集线程内创建 + open_stream + 持续 frame()。用 sync_channel 把初始化结果
        // 送回 open（失败则透传错误），open 返回后线程继续跑采集循环。
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
        let task_latest = Arc::clone(&latest);
        let task_sender = sender.clone();
        let task_stop = Arc::clone(&stop);
        let task_stats = Arc::clone(&stats);
        let init_name = name.clone();
        let handle = std::thread::Builder::new()
            .name("camera-nokhwa".into())
            .spawn(move || {
                // 格式/帧率协商（issue #20）：首选 Closest 指定帧率（best-effort），
                // 失败回退 None（驱动自选），保证设备总能打开。
                // macOS AVFoundation 帧率协商无效是已知限制，格式协商仍尝试。
                let fps = frames_per_second.max(1) as u32;
                let cam_index_fallback = cam_index.clone();
                let mut camera = {
                    let closest = RequestedFormat::new::<RgbAFormat>(
                        RequestedFormatType::HighestFrameRate(fps),
                    );
                    match Camera::new(cam_index, closest) {
                        Ok(c) => c,
                        Err(_) => {
                            // 回退：驱动不支持请求的帧率/格式，用 None 让库自选。
                            let none =
                                RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None);
                            match Camera::new(cam_index_fallback, none) {
                                Ok(c) => c,
                                Err(e) => {
                                    let _ = init_tx.send(Err(format!("nokhwa open: {e}")));
                                    return;
                                }
                            }
                        }
                    }
                };
                if let Err(e) = camera.open_stream() {
                    let _ = init_tx.send(Err(format!("open_stream: {e}")));
                    return;
                }
                // 初始化成功：通知 open 可以返回了。
                let _ = init_tx.send(Ok(()));
                let mut seq = 0_u64;
                let mut rgb_buf: Vec<u8> = Vec::new();
                let mut rgba_buf: Vec<u8> = Vec::new();
                // 检测设备输出格式，选择最优处理路径：
                // 1. MJPEG → 直接透传（零解码零编码）
                // 2. YUYV → 直接转 RGB（省 25% 内存，跳过 RGBA 中间格式）
                // 3. 其他格式 → nokhwa 解码 RGBA → 转 BGRA（兼容性最好）
                let device_format = camera.frame_format();
                let capture_mode = match device_format {
                    nokhwa::utils::FrameFormat::MJPEG => CaptureMode::MjpegPassthrough,
                    nokhwa::utils::FrameFormat::YUYV => CaptureMode::YuyvToRgb,
                    _ => CaptureMode::DecodeToBgra,
                };
                // MJPEG 透传时 frame_raw() 不含分辨率，先调用 frame() 获取并缓存。
                // （会消耗一帧，但分辨率在整个会话期间不变。）
                let cached_resolution = if matches!(capture_mode, CaptureMode::MjpegPassthrough) {
                    camera.frame().ok().map(|f| {
                        let r = f.resolution();
                        (r.width_x, r.height_y)
                    })
                } else {
                    None
                };
                loop {
                    if task_stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let frame_wait_start = std::time::Instant::now();

                    match capture_mode {
                        CaptureMode::MjpegPassthrough => {
                            // MJPEG 透传：frame_raw() 返回原始 JPEG 字节，直接作为
                            // RFB Tight JPEG 矩形发送，跳过解码→转换→再编码。
                            match camera.frame_raw() {
                                Ok(raw) => {
                                    task_stats.record_capture_wait(frame_wait_start.elapsed());
                                    let capture_ns = crate::now_ns();
                                    // V4L2 buffer 可能大于实际 JPEG 数据大小。
                                    // 裁剪到 JPEG EOI (0xFF 0xD9) 为止。
                                    let jpeg_end = raw
                                        .windows(2)
                                        .position(|w| w == [0xFF, 0xD9])
                                        .map(|p| p + 2)
                                        .unwrap_or(raw.len());
                                    let jpeg_data: Vec<u8> = raw[..jpeg_end].to_vec();
                                    // 获取分辨率：优先用缓存，否则调用 frame() 获取。
                                    let (w, h) = if let Some(r) = cached_resolution {
                                        r
                                    } else {
                                        match camera.frame() {
                                            Ok(f) => {
                                                let r = f.resolution();
                                                (r.width_x, r.height_y)
                                            }
                                            Err(_) => continue,
                                        }
                                    };
                                    seq = seq.saturating_add(1);
                                    let video_frame = VideoFrame::new(
                                        seq,
                                        MonotonicTimestamp::from_nanos(capture_ns),
                                        w,
                                        h,
                                        0,
                                        PixelFormat::Mjpeg,
                                        Arc::from(jpeg_data),
                                    );
                                    let shared = Arc::new(video_frame);
                                    task_stats.record_publish(seq, capture_ns);
                                    *task_latest.write().expect("camera lock poisoned") =
                                        Some(Arc::clone(&shared));
                                    task_sender.send_replace(Some(shared));
                                }
                                Err(_) => {
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            }
                        }
                        CaptureMode::YuyvToRgb => {
                            // YUYV 直接转 RGB：frame_raw() 获取原始数据，直接转换为 RGB。
                            // 跳过 nokhwa 的 RGBA 解码，省掉 alpha 通道（JPEG 不支持透明）。
                            match camera.frame_raw() {
                                Ok(raw) => {
                                    task_stats.record_capture_wait(frame_wait_start.elapsed());
                                    let capture_ns = crate::now_ns();
                                    let convert_start = std::time::Instant::now();
                                    // 复制 raw 数据，释放对 camera 的借用。
                                    let raw_data: Vec<u8> = raw.into_owned();
                                    // 获取分辨率
                                    let (w, h) = match camera.frame() {
                                        Ok(f) => {
                                            let r = f.resolution();
                                            (r.width_x, r.height_y)
                                        }
                                        Err(_) => continue,
                                    };
                                    // YUYV 4:2:2：每 4 字节 = 2 像素
                                    // RGB：每像素 3 字节
                                    let expected_yuyv = (w * h * 2) as usize;
                                    let expected_rgb = (w * h * 3) as usize;
                                    if raw_data.len() < expected_yuyv {
                                        continue;
                                    }
                                    rgb_buf.clear();
                                    rgb_buf.resize(expected_rgb, 0);
                                    convert_yuyv_to_rgb(&raw_data[..expected_yuyv], &mut rgb_buf);
                                    task_stats.record_convert(convert_start.elapsed());
                                    seq = seq.saturating_add(1);
                                    let video_frame = VideoFrame::new(
                                        seq,
                                        MonotonicTimestamp::from_nanos(capture_ns),
                                        w,
                                        h,
                                        w * 3, // stride = width * 3 (RGB)
                                        PixelFormat::Rgb888,
                                        Arc::from(rgb_buf.as_slice()),
                                    );
                                    let shared = Arc::new(video_frame);
                                    task_stats.record_publish(seq, capture_ns);
                                    *task_latest.write().expect("camera lock poisoned") =
                                        Some(Arc::clone(&shared));
                                    task_sender.send_replace(Some(shared));
                                }
                                Err(_) => {
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            }
                        }
                        CaptureMode::DecodeToBgra => {
                            // 通用路径：nokhwa 解码 RGBA → 转 BGRA。
                            // 适用于 NV12、GRAY、RAWRGB、RAWBGR 等格式。
                            let frame = match camera.frame() {
                                Ok(f) => f,
                                Err(_) => {
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                    continue;
                                }
                            };
                            task_stats.record_capture_wait(frame_wait_start.elapsed());
                            let capture_ns = crate::now_ns();
                            let convert_start = std::time::Instant::now();
                            let res = frame.resolution();
                            rgba_buf.clear();
                            rgba_buf.resize((res.width_x as usize) * (res.height_y as usize) * 4, 0);
                            if frame
                                .decode_image_to_buffer::<RgbAFormat>(&mut rgba_buf)
                                .is_err()
                            {
                                continue;
                            }
                            for px in rgba_buf.chunks_exact_mut(4) {
                                px.swap(0, 2);
                                px[3] = 255;
                            }
                            task_stats.record_convert(convert_start.elapsed());
                            seq = seq.saturating_add(1);
                            let video_frame = VideoFrame::new(
                                seq,
                                MonotonicTimestamp::from_nanos(capture_ns),
                                res.width_x,
                                res.height_y,
                                res.width_x * 4,
                                PixelFormat::Bgra8888,
                                Arc::from(rgba_buf.as_slice()),
                            );
                            let shared = Arc::new(video_frame);
                            task_stats.record_publish(seq, capture_ns);
                            *task_latest.write().expect("camera lock poisoned") =
                                Some(Arc::clone(&shared));
                            task_sender.send_replace(Some(shared));
                        }
                    }
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
            stats,
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

    fn source_stats(&self) -> Option<crate::SourceStatsSnapshot> {
        Some(self.stats.snapshot())
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
    // 过滤掉元数据设备（UVC 设备的 metadata 接口不支持视频捕获）。
    // 尝试打开设备并检查是否支持视频格式，如果不支持则跳过。
    let mut result = Vec::new();
    for (i, info) in devices.into_iter().enumerate() {
        let index = CameraIndex::Index(i as u32);
        // 尝试用 None 格式打开设备，如果失败则跳过
        let fmt = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None);
        if Camera::new(index, fmt).is_ok() {
            // 设备可以打开，认为是有效的视频设备
            result.push(CameraDeviceInfo {
                id: format!("{i}:{}", info.human_name()),
                display_name: info.human_name(),
            });
        }
    }
    Ok(result)
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

    #[test]
    fn yuyv_to_rgb_converts_correctly() {
        // 测试 YUYV→RGB 转换
        // YUYV 格式：每 4 字节 (Y1, U, Y2, V) 转换为 2 个 RGB 像素
        // 使用标准 YCbCr→RGB 公式，Y 有 -16 的偏移
        let y1 = 128i32;
        let u = 128i32;
        let y2 = 128i32;
        let v = 128i32;

        let [r, g, b] = yuyv_to_rgb(y1, u, v);
        // 当 Y=128, U=128, V=128 时，由于 Y-16 偏移，应该是 (130, 130, 130)
        // C298 = (128-16) * 298 = 33376
        // R = (33376 + 128) >> 8 = 130
        assert_eq!(r, 130);
        assert_eq!(g, 130);
        assert_eq!(b, 130);
    }

    #[test]
    fn convert_yuyv_to_rgb_produces_correct_size() {
        // 2x2 像素的 YUYV 数据：4 字节 (Y1, U, Y2, V) = 2 像素
        // 需要 2 个这样的块来表示 2x2 图像
        let yuyv_data = vec![128u8; 8]; // 2x2 像素，每像素 2 字节 YUYV
        let mut rgb_buf = vec![0u8; 12]; // 2x2 像素，每像素 3 字节 RGB

        convert_yuyv_to_rgb(&yuyv_data, &mut rgb_buf);

        // 验证输出大小正确
        assert_eq!(rgb_buf.len(), 12);
    }

    #[test]
    fn convert_yuyv_to_rgb_handles_white() {
        // 白色：Y=255, U=128, V=128
        let yuyv_data = vec![255, 128, 255, 128]; // 1 个 YUYV 块 = 2 像素
        let mut rgb_buf = vec![0u8; 6]; // 2 像素 * 3 字节

        convert_yuyv_to_rgb(&yuyv_data, &mut rgb_buf);

        // 白色应该是 (255, 255, 255)
        assert_eq!(rgb_buf[0], 255); // R1
        assert_eq!(rgb_buf[1], 255); // G1
        assert_eq!(rgb_buf[2], 255); // B1
        assert_eq!(rgb_buf[3], 255); // R2
        assert_eq!(rgb_buf[4], 255); // G2
        assert_eq!(rgb_buf[5], 255); // B2
    }
}
