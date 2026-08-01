//! 相机探测示例：枚举 Windows Media Foundation 相机，打开第一台（优先 OBS 虚拟摄像头）
//! 并采集若干帧 BGRA8888 数据验证采集循环。
//!
//! 用法：`cargo run -p ipkvm-video --example camera_probe --features mf`

use std::time::{Duration, Instant};

use ipkvm_video::FrameSource;

fn main() {
    let cameras = match ipkvm_video::camera::list_cameras() {
        Ok(cameras) => cameras,
        Err(e) => {
            println!("[enumerate] error: {e}");
            std::process::exit(1);
        }
    };
    println!("[enumerate] {} camera(s):", cameras.len());
    for (index, camera) in cameras.iter().enumerate() {
        println!(
            "  [{index}] id={:?} display_name={:?}",
            camera.id, camera.display_name
        );
    }

    let Some(camera) = cameras
        .iter()
        .find(|camera| camera.display_name.contains("OBS"))
        .or(cameras.first())
    else {
        println!("[capture] no camera to open");
        // 枚举为空时验证 open 的错误路径（枚举成功 + 设备未找到）。
        match ipkvm_video::camera::CameraSource::open("0:no-such-device", 30) {
            Ok(_) => {
                println!("[capture] ERROR: opening a non-existent device unexpectedly succeeded");
                std::process::exit(1);
            }
            Err(e) => {
                println!("[capture] open(non-existent) rejected as expected: {e}");
                std::process::exit(0);
            }
        }
    };

    println!("[capture] opening {:?} at 30 fps...", camera.id);
    let source = match ipkvm_video::camera::CameraSource::open(&camera.id, 30) {
        Ok(source) => source,
        Err(e) => {
            println!("[capture] open failed: {e}");
            std::process::exit(1);
        }
    };
    println!("[capture] source_info: {:#?}", source.source_info());

    let Some(first) = wait_for_frame(&source, Duration::from_secs(5)) else {
        println!("[capture] no frame within 5s");
        std::process::exit(1);
    };
    let bytes = first.data.as_ref();
    let pixel_at = |offset: usize| {
        let offset = offset & !3;
        format!(
            "B={} G={} R={} A={}",
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3]
        )
    };
    let checksum: u64 = bytes.iter().take(4096).map(|&b| u64::from(b)).sum();
    println!(
        "[capture] first frame: seq={} ts_nanos={} {}x{} stride={} format={:?} len={}",
        first.seq,
        first.timestamp.nanos,
        first.width,
        first.height,
        first.stride,
        first.pixel_format,
        first.data.len()
    );
    println!("[capture] first pixel: {}", pixel_at(0));
    println!("[capture] center pixel: {}", pixel_at(bytes.len() / 2));
    println!("[capture] checksum(first 4096 bytes)={checksum}");

    let window_start = Instant::now();
    let mut frames = 1_u64;
    while window_start.elapsed() < Duration::from_secs(2) {
        if wait_for_frame(&source, Duration::from_millis(200)).is_none() {
            break;
        }
        frames += 1;
    }
    println!("[capture] received {frames} frames in ~2s");
    println!("[capture] OK");
}

fn wait_for_frame(
    source: &impl FrameSource,
    timeout: Duration,
) -> Option<ipkvm_video::SharedVideoFrame> {
    let started = Instant::now();
    loop {
        if let Some(frame) = source.latest_frame() {
            return Some(frame);
        }
        if started.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
