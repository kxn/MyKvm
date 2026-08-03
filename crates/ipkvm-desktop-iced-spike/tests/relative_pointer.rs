//! Spike 3 相对鼠标验证：平台源 + 采样器集成。
//!
//! - Windows：SendInput 注入真实鼠标移动 → Raw Input → receiver，测 1:1 增量与
//!   p95 < 16ms 延迟（smoke；需要无其它鼠标干扰的环境）。
//! - 非 Windows：stub 返回“未实现”，保证 trait 形状可编译（macOS 留口）。

use std::time::{Duration, Instant};

#[cfg(windows)]
mod windows_smoke {
    use std::mem::size_of;

    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
    };

    use super::*;
    use ipkvm_desktop_iced_spike::platform;

    fn send_mouse_move(dx: i32, dy: i32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
        assert_eq!(sent, 1, "SendInput 必须成功");
    }

    #[test]
    fn windows_raw_input_roundtrip_and_lifecycle() {
        // 启动 → 注入 → 校验增量与延迟；重复启动拒绝；stop 后可重新启动。
        let mut source = platform::create().expect("windows raw input source");
        let rx = source.receiver().expect("start capture");
        assert!(source.receiver().is_err(), "重复启动必须被拒绝");

        let mut latencies = Vec::new();
        let mut received = 0u32;
        for _ in 0..20 {
            // 排空可能的外部鼠标移动，避免干扰判定。
            while rx.recv_timeout(Duration::from_millis(1)).is_ok() {}
            // 留出间隔避免 SendInput 合并事件。
            std::thread::sleep(Duration::from_millis(10));

            let (want_x, want_y) = if received % 2 == 0 {
                (3i16, 0i16)
            } else {
                (0i16, 3i16)
            };
            let start = Instant::now();
            send_mouse_move(i32::from(want_x), i32::from(want_y));
            if let Ok((dx, dy)) = rx.recv_timeout(Duration::from_millis(1000)) {
                latencies.push(start.elapsed());
                assert_eq!((dx, dy), (want_x, want_y), "注入增量必须 1:1 到达");
                received += 1;
            }
        }
        source.stop();

        assert!(
            received >= 18,
            "至少 90% 注入事件被接收（实际 {received}/20）"
        );
        latencies.sort_unstable();
        let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
        assert!(
            p95 < Duration::from_millis(16),
            "Raw Input → receiver p95 延迟 {p95:?} 必须 < 16ms"
        );

        // stop 清空全局状态后可重新启动。
        let _rx = source.receiver().expect("stop 后可重新启动");
        source.stop();
    }
}

#[cfg(not(windows))]
#[test]
fn stub_reports_not_implemented() {
    use ipkvm_desktop_iced_spike::platform;

    let mut source = platform::create().expect("stub source factory");
    assert!(
        source.receiver().is_err(),
        "macOS/linux stub 必须报告未实现（留口，不假装可用）"
    );
    source.stop();
}
