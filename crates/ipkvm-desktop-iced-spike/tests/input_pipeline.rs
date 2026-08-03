//! Spike 3 输入管道验证：物理键 → keysym → 会话泵 → mock sink，
//! 500 次混合按键（1000 个事件）顺序一致、0 丢失/重复、不吞首键。

use std::sync::Arc;
use std::time::Duration;

use iced::keyboard::key::Code;
use ipkvm_core::MouseMode;
use ipkvm_desktop::{ConnectRequest, DesktopSessionController, DesktopSessionError, SessionParts};
use ipkvm_desktop_iced_spike::app::RecordingSink;
use ipkvm_desktop_iced_spike::keymap::physical_code_to_keysym;
use ipkvm_session::rfb_connection::RfbConnectionGate;
use ipkvm_video::FrameSource;
use ipkvm_video::mock::MockFrameSource;

fn request() -> ConnectRequest {
    ConnectRequest {
        video_device_id: "mock".into(),
        control_device_id: "mock".into(),
        baud_rate: 9_600,
        mouse_mode: MouseMode::Absolute,
        preview_fps: 30,
    }
}

type TestFactory =
    Box<dyn FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>>;

#[test]
fn five_hundred_mixed_keys_reach_sink_in_order() {
    let sink = RecordingSink::default();
    let sink_for_factory = sink.clone();
    let factory: TestFactory = Box::new(move |_request| {
        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        Ok((
            frame_source,
            sink_for_factory.clone(),
            RfbConnectionGate::new(),
        ))
    });
    let mut controller = DesktopSessionController::with_factory(factory);
    controller
        .connect(request())
        .expect("spike controller connect");

    // 生成 500 个物理键 + 期望的 HID usage。
    let mut codes = Vec::with_capacity(500);
    let mut expected_usages = Vec::with_capacity(1000);
    for i in 0..500 {
        let code = match i % 3 {
            0 => Code::KeyA,
            1 => Code::ArrowUp,
            _ => Code::F1,
        };
        codes.push(code);
        let usage = match code {
            Code::KeyA => 0x04,
            Code::ArrowUp => 0x52,
            Code::F1 => 0x3a,
            _ => unreachable!(),
        };
        expected_usages.push(usage);
        expected_usages.push(usage);
    }

    for code in &codes {
        let keysym = physical_code_to_keysym(*code).expect("code mapped");
        controller.send_key(true, keysym).expect("send down");
        controller.send_key(false, keysym).expect("send up");
    }

    // 控制器通道满时残余事件滞留 pending，UI 每帧应显式补送（见 flush_pending）。
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while sink.key_events.lock().unwrap().len() < 1000 {
        controller.flush_pending().expect("flush pending");
        assert!(
            std::time::Instant::now() < deadline,
            "flush_pending 补送未完成"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    // 等待全部 1000 个事件到达（2s 超时）。
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let len = sink.key_events.lock().unwrap().len();
        if len >= 1000 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "事件未全部到达：{len}/1000"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let recorded = sink.key_events.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1000, "0 丢失 / 0 重复");
    for (i, ((down, usage), expected)) in recorded.iter().zip(expected_usages).enumerate() {
        assert_eq!(
            *down,
            i % 2 == 0,
            "第 {i} 个事件 down/up 顺序不符（期望 down,up 交替）"
        );
        assert_eq!(*usage, expected, "第 {i} 个事件 usage 不匹配");
    }

    controller.stop().expect("stop");
}

#[test]
fn first_key_is_not_swallowed() {
    let sink = RecordingSink::default();
    let sink_for_factory = sink.clone();
    let factory: TestFactory = Box::new(move |_request| {
        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        Ok((
            frame_source,
            sink_for_factory.clone(),
            RfbConnectionGate::new(),
        ))
    });
    let mut controller = DesktopSessionController::with_factory(factory);
    controller.connect(request()).expect("connect");

    let keysym = physical_code_to_keysym(Code::KeyA).expect("KeyA");
    let start = std::time::Instant::now();
    controller.send_key(true, keysym).expect("send");

    let deadline = start + Duration::from_secs(1);
    loop {
        let recorded = sink.key_events.lock().unwrap();
        if !recorded.is_empty() {
            assert_eq!(recorded[0], (true, 0x04), "首键必须是 KeyA down");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "首键被吞");
        drop(recorded);
        std::thread::sleep(Duration::from_millis(5));
    }
    controller.stop().expect("stop");
}
