//! Spike 1 本机性能基准入口。
//!
//! 用 MockFrameSource 以 30fps 推 1080p 帧（后台线程），iced 应用订阅帧并渲染。
//! 内置源帧/渲染帧计数（窗口标题实时显示），供 perf-1080p 脚本采样。
//!
//! `--duration N`：运行 N 秒后自动退出并打印 JSON 总结（帧数/平均/p95 帧间隔）。
//!
//! 运行：`cargo run -p ipkvm-desktop-iced-spike --example video_1080p_spike --release -- --duration 120`

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ipkvm_desktop_iced_spike::{FrameStats, SpikeApp};
use ipkvm_video::mock::MockFrameSource;
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};

/// 帧统计 JSON 写入的文件名（perf 脚本读这个文件，而非 stdout，避免
/// process::exit 跳过 stdout 管道 flush 导致数据丢失）。
/// 可用 `--stats-file <path>` 覆盖（perf 脚本传绝对路径，绕开 CWD 不确定）。
fn stats_file_path() -> String {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--stats-file"
            && let Some(v) = args.get(i + 1)
        {
            return v.clone();
        }
        i += 1;
    }
    "video_1080p.stats.json".to_string()
}

fn main() -> iced::Result {
    // 解析 --duration N（默认 0 = 不自动退出，人工关闭窗口）。
    let duration_sec = parse_duration_arg();

    let frame_source = Arc::new(MockFrameSource::new());
    let stats = FrameStats::new();
    let stats_for_exit = Arc::clone(&stats);
    let fps_publisher = Arc::clone(&frame_source);
    let source_count = Arc::new(AtomicU64::new(0));
    let source_count_for_exit = Arc::clone(&source_count);

    // 后台线程以 30fps 推 1920×1080 BGRA 帧（模拟视频流）。
    std::thread::spawn(move || {
        let interval = Duration::from_secs_f32(1.0 / 30.0);
        let mut seq: u64 = 0;
        loop {
            seq += 1;
            source_count.store(seq, Ordering::Relaxed);
            // 1080p BGRA 帧：width=1920, height=1080, stride=7680。
            // 用简单图案（每帧整体亮度随 seq 变化）确保帧内容确实在变。
            // 用 cycle().take() 一次性填充，避免逐像素循环（否则 200 万次 extend
            // 会让推帧本身 >33ms，无法稳定 30fps）。
            let brightness = ((seq % 60) as u8).saturating_mul(4);
            let pixel_count = 1920 * 1080;
            let row: Vec<u8> = [20u8, 40, brightness, 255]
                .iter()
                .cycle()
                .take(pixel_count * 4)
                .copied()
                .collect();
            let frame = VideoFrame::new(
                seq,
                MonotonicTimestamp::from_nanos(seq),
                1920,
                1080,
                7680,
                PixelFormat::Bgra8888,
                Arc::from(row.into_boxed_slice()),
            );
            fps_publisher.publish_frame(Arc::new(frame));
            std::thread::sleep(interval);
        }
    });

    // 到时长后写 JSON 统计文件并退出。
    // 写独立文件（而非 stdout）：process::exit 会跳过 stdout 管道 flush，导致
    // 重定向文件丢数据；直接写文件 + flush 更可靠。
    if duration_sec > 0 {
        let stats_path = stats_file_path();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(duration_sec));
            let (rendered, avg, p95) = stats_for_exit.summary();
            let source = source_count_for_exit.load(Ordering::Relaxed);
            let json = format!(
                "{{\"source_frames\": {source}, \"rendered_frames\": {rendered}, \"avg_interval_ms\": {avg:.2}, \"p95_interval_ms\": {p95:.2}}}"
            );
            if let Ok(mut f) = std::fs::File::create(&stats_path) {
                let _ = f.write_all(json.as_bytes());
                let _ = f.flush();
                let _ = f.sync_all();
            }
            // 直接退出（spike 可接受跳过优雅停；controller runtime 随进程结束释放）。
            std::process::exit(0);
        });
    }

    iced::application(
        move || SpikeApp::new(frame_source.clone(), stats.clone()),
        SpikeApp::update,
        SpikeApp::view,
    )
    .subscription(SpikeApp::subscription)
    .title(move |app: &SpikeApp| format!("iced spike | 已渲染 {} 帧", app.rendered_frames()))
    .run()
}

/// 解析 `--duration N` 参数（N 为秒；缺失或 0 表示不自动退出）。
fn parse_duration_arg() -> u64 {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--duration"
            && let Some(v) = args.get(i + 1)
            && let Ok(n) = v.parse::<u64>()
        {
            return n;
        }
        i += 1;
    }
    0
}
