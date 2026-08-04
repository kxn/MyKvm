//! 相机性能探测：测采集 CPU 占用与帧率。
//! --idle : 仅 open（启动采集线程）后纯睡眠，不读帧，验证采集线程空闲 CPU。
//! 默认   : 订阅统计帧率（订阅侧轮询开销另算）。
//! 用法：cargo run -p ipkvm-video --example camera_perf --features camera [--idle]

use ipkvm_video::FrameSource;
use std::time::Duration;

fn main() {
    let idle = std::env::args().any(|a| a == "--idle");
    let cams = ipkvm_video::camera::list_cameras().unwrap();
    let obs = cams
        .iter()
        .find(|c| c.display_name.contains("OBS"))
        .or_else(|| cams.first())
        .expect("no camera");
    println!(
        "opening {} (mode={})",
        obs.display_name,
        if idle { "idle" } else { "stream" }
    );
    let src = ipkvm_video::camera::CameraSource::open(&obs.id, 30).unwrap();
    println!("opened; sampling 5s...");

    // 进程内自测 CPU：用 thread CPU time（std 没有直接 API，改用墙钟 + 仅统计帧）。
    // CPU 由外部 PowerShell 测。这里只报帧率（stream 模式）或纯睡眠（idle 模式）。
    if idle {
        std::thread::sleep(Duration::from_secs(5));
        println!("idle done");
        return;
    }

    // stream 模式：等首帧后统计 5 秒帧率
    let t0 = std::time::Instant::now();
    while src.latest_frame().is_none() && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut rx = src.subscribe();
    let mut last_seq = 0u64;
    let mut frames = 0u64;
    let win = Duration::from_secs(5);
    let start = std::time::Instant::now();
    while start.elapsed() < win {
        if rx.has_changed().unwrap_or(false)
            && let Some(f) = rx.borrow_and_update().as_ref()
            && f.seq != last_seq
        {
            frames += 1;
            last_seq = f.seq;
        }
        // 用阻塞等待代替忙轮询，降低工具自身 CPU：用 recv_timeout 语义不可得（watch 无），
        // 改用较长 sleep 让采集线程主导。
        std::thread::sleep(Duration::from_millis(15));
    }
    println!(
        "=== {} frames in 5s => {:.1} fps ===",
        frames,
        frames as f64 / 5.0
    );
}
