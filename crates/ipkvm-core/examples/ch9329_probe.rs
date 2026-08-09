//! CH9329 串口探测工具：直接向 CH9329 发送键盘/鼠标命令，用于硬件闭环验证。
//!
//! 用法：
//!   ch9329_probe <串口路径> [波特率=9600] <子命令> [参数]
//!
//! 子命令：
//!   key <usage_hex>              按下并释放一个 HID usage（如 0x04 = a）
//!   text <字符串>                依次「按下并释放」每个 ASCII 字符（模拟打字）
//!   info                         发送 GetInfo 并等待响应；无响应时自动尝试恢复
//!   mouse-abs <x> <y>            绝对鼠标移动到 (x,y)，坐标按 0-4095 原始范围
//!   mouse-rel <dx> <dy>          相对鼠标移动 (dx,dy)
//!   click <left|middle|right>    用绝对帧在中心位置按下并释放一次按键
//!   click-rel <left|middle|right>
//!                                用相对帧 0x05 按下并释放一次按键，用于验证目标 OS
//!                                是否接受零位移相对按钮报告
//!   mouse-abs-btn <x> <y> <btn>  直接发送绝对帧 0x04 带按钮位（绕过 InputSink 的按键路由），
//!                                用于实测「绝对帧按键位是否被目标机识别」；btn 为 0-7 掩码
//!                                （bit0=左 bit1=右 bit2=中），按下 60ms 后原位释放
//!   btn <left|middle|right> <down|up>
//!                                发送一个相对按钮边沿（走相对帧 0x05）；进程收尾仍会
//!                                release_all，持续按住请使用专门的单进程测试命令
//!   drag <x1> <y1> <x2> <y2>     拖拽测试：绝对移动到 A → 左键 down → 绝对移动到 B → 左键 up，
//!                                全程在一个进程内完成（避免命令收尾 release_all 干扰）；
//!                                用于验证绝对移动帧期间按钮状态是否保持
//!
//! 验证方法（本机 COM9 → ARM 10.10.10.21）：
//!   1. ARM 上 `cat /dev/tty1` 或登录控制台，运行 `ch9329_probe COM9 text hello`
//!      → 应在 ARM 屏幕看到 "hello" 字符出现。
//!   2. `ch9329_probe COM9 mouse-rel 100 0` → ARM 上 `evtest /dev/input/event6`
//!      应看到鼠标 X 方向事件。

use std::time::Duration;

use ipkvm_core::{
    AbsoluteMouseReport, Ch9329Command, Ch9329InputSink, CommandBatch, CommandQueue,
    DEFAULT_BAUD_RATE, KeyEvent, KeyboardUsage, MouseMode, PointerButton, PointerEvent,
    SerialCommandQueue,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (serial_path, baud, subcommand, rest) = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("参数错误：{e}");
            eprintln!(
                "用法：ch9329_probe <串口路径> [波特率=9600] <key|text|info|mouse-abs|mouse-rel|click|click-rel|mouse-abs-btn> [参数]"
            );
            std::process::exit(2);
        }
    };

    let queue = match SerialCommandQueue::open(&serial_path, baud) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("打开串口 {serial_path}@{baud} 失败：{e}");
            std::process::exit(1);
        }
    };
    eprintln!("[probe] 已打开 {serial_path}@{baud}（8N1 无流控）");

    // 键盘测试用 Absolute 鼠标模式（CH9329 同时支持键鼠；鼠标模式仅影响鼠标命令）。
    // raw_queue 与 sink 共享同一个线程安全发送队列，供需要绕过 InputSink
    // 直接构造帧的子命令（mouse-abs-btn）使用。
    let raw_queue = queue.clone();
    let mut sink = Ch9329InputSink::new(queue, 0x00, MouseMode::Absolute);

    let result = match subcommand.as_str() {
        "key" => cmd_key(&mut sink, &rest),
        "text" => cmd_text(&mut sink, &rest),
        "info" => {
            eprintln!("[probe] 等待 GetInfo/恢复结果（最多 5 秒）");
            match raw_queue.wait_until_ready(Duration::from_secs(5)) {
                Some(health) => {
                    eprintln!(
                        "[probe] Ready: pending={} timeouts={} protocol_errors={} device_errors={} resets={} reopens={}",
                        health.pending_frames,
                        health.timeouts,
                        health.protocol_errors,
                        health.device_errors,
                        health.resets,
                        health.reopens,
                    );
                    Ok(())
                }
                None => {
                    let health = raw_queue.health();
                    Err(format!(
                        "CH9329 未恢复到 Ready: state={:?}, timeouts={}, protocol_errors={}, device_errors={}, resets={}, reopens={}",
                        health.state,
                        health.timeouts,
                        health.protocol_errors,
                        health.device_errors,
                        health.resets,
                        health.reopens,
                    ))
                }
            }
        }
        "mouse-abs" => cmd_mouse_abs(&mut sink, &rest),
        "mouse-rel" => {
            // 相对鼠标需切到 Relative 模式。
            sink.set_mouse_mode(MouseMode::Relative).unwrap();
            cmd_mouse_rel(&mut sink, &rest)
        }
        "click" => cmd_click(&mut sink, &rest),
        "click-rel" => cmd_click_rel(&mut sink, &rest),
        "mouse-abs-btn" => cmd_mouse_abs_btn(&raw_queue, &rest),
        "btn" => cmd_btn(&mut sink, &rest),
        "drag" => cmd_drag(&mut sink, &rest),
        other => Err(format!("未知子命令：{other}")),
    };

    match result {
        Ok(()) => {
            if let Err(error) = wait_for_idle(&raw_queue) {
                eprintln!("[probe] 失败：{error}");
                std::process::exit(1);
            }
            // 释放所有键鼠状态（避免按键卡住）。
            let _ = sink.release_all();
            if let Err(error) = wait_for_idle(&raw_queue) {
                eprintln!("[probe] 释放状态未完成：{error}");
                std::process::exit(1);
            }
            eprintln!("[probe] 完成");
        }
        Err(e) => {
            eprintln!("[probe] 失败：{e}");
            let _ = sink.release_all();
            let _ = wait_for_idle(&raw_queue);
            std::process::exit(1);
        }
    }
}

fn wait_for_idle(queue: &SerialCommandQueue) -> Result<(), String> {
    queue
        .wait_until_idle(Duration::from_secs(5))
        .map(|_| ())
        .ok_or_else(|| {
            let health = queue.health();
            format!(
                "CH9329 未在限定时间内完成发送: state={:?}, pending={}, queued_batches={}, timeouts={}, protocol_errors={}, device_errors={}, resets={}, reopens={}",
                health.state,
                health.pending_frames,
                health.queued_batches,
                health.timeouts,
                health.protocol_errors,
                health.device_errors,
                health.resets,
                health.reopens,
            )
        })
}

/// 解析：<串口路径> [波特率] <子命令> [参数...]。波特率可选，缺省 9600。
fn parse_args(args: &[String]) -> Result<(String, u32, String, Vec<String>), String> {
    if args.len() < 2 {
        return Err("至少需要串口路径和子命令".into());
    }
    let serial_path = args[0].clone();
    // 第二个参数若以数字开头且是合法波特率，则当作波特率。
    let (baud, sub_idx) = if args.len() >= 3 && args[1].parse::<u32>().is_ok() {
        (args[1].parse::<u32>().unwrap(), 2)
    } else {
        (DEFAULT_BAUD_RATE, 1)
    };
    if args.len() <= sub_idx {
        return Err("缺少子命令".into());
    }
    let subcommand = args[sub_idx].clone();
    let rest = args[sub_idx + 1..].to_vec();
    Ok((serial_path, baud, subcommand, rest))
}

fn cmd_key<Q: CommandQueue>(sink: &mut Ch9329InputSink<Q>, rest: &[String]) -> Result<(), String> {
    let usage_str = rest.first().ok_or("key 需要一个 usage 参数（如 0x04）")?;
    let usage_val = parse_u8(usage_str)?;
    let key = KeyboardUsage::new(usage_val).map_err(|e| format!("无效 usage：{e}"))?;
    eprintln!("[probe] 按下 usage={usage_val:#04x}");
    sink.handle_key(KeyEvent::Down { usage: key })
        .map_err(|e| format!("key down: {e}"))?;
    std::thread::sleep(Duration::from_millis(40));
    sink.handle_key(KeyEvent::Up { usage: key })
        .map_err(|e| format!("key up: {e}"))?;
    eprintln!("[probe] 释放 usage={usage_val:#04x}");
    Ok(())
}

fn cmd_text<Q: CommandQueue>(sink: &mut Ch9329InputSink<Q>, rest: &[String]) -> Result<(), String> {
    // rest 各参数用空格拼接为待输入文本。
    let text = rest.join(" ");
    if text.is_empty() {
        return Err("text 需要文本参数".into());
    }
    eprintln!("[probe] 输入文本：{text:?}");
    for ch in text.chars() {
        let usage =
            ascii_to_usage(ch).ok_or_else(|| format!("无法映射字符 {ch:?} 到 HID usage"))?;
        sink.handle_key(KeyEvent::Down { usage })
            .map_err(|e| format!("key down {ch}: {e}"))?;
        std::thread::sleep(Duration::from_millis(20));
        sink.handle_key(KeyEvent::Up { usage })
            .map_err(|e| format!("key up {ch}: {e}"))?;
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

fn cmd_mouse_abs<Q: CommandQueue>(
    sink: &mut Ch9329InputSink<Q>,
    rest: &[String],
) -> Result<(), String> {
    let x = rest
        .first()
        .ok_or("mouse-abs 需要 x 参数")?
        .parse::<u32>()
        .map_err(|e| format!("无效 x：{e}"))?;
    let y = rest
        .get(1)
        .ok_or("mouse-abs 需要 y 参数")?
        .parse::<u32>()
        .map_err(|e| format!("无效 y：{e}"))?;
    if x > 4095 || y > 4095 {
        return Err(format!("绝对坐标超出 0-4095：({x},{y})"));
    }
    eprintln!("[probe] 绝对鼠标移动到 ({x},{y})");
    // 用 4096x4096 的虚拟帧缓冲，使传入的原始坐标直通（map_framebuffer_axis 在
    // 像素坐标 == 范围时 1:1 映射）。
    sink.handle_pointer(PointerEvent::AbsoluteMove {
        x,
        y,
        framebuffer_size: ipkvm_core::FramebufferSize {
            width: 4096,
            height: 4096,
        },
    })
    .map_err(|e| format!("mouse-abs: {e}"))?;
    Ok(())
}

fn cmd_mouse_rel<Q: CommandQueue>(
    sink: &mut Ch9329InputSink<Q>,
    rest: &[String],
) -> Result<(), String> {
    let dx = rest
        .first()
        .ok_or("mouse-rel 需要 dx 参数")?
        .parse::<i16>()
        .map_err(|e| format!("无效 dx：{e}"))?;
    let dy = rest
        .get(1)
        .ok_or("mouse-rel 需要 dy 参数")?
        .parse::<i16>()
        .map_err(|e| format!("无效 dy：{e}"))?;
    eprintln!("[probe] 相对鼠标移动 ({dx},{dy})");
    sink.handle_pointer(PointerEvent::RelativeMove { dx, dy })
        .map_err(|e| format!("mouse-rel: {e}"))?;
    Ok(())
}

/// 直接发送绝对帧 0x04 带按钮位，用于实测「绝对帧的按键位是否被目标机识别」。
///
/// 与 `click` 不同，这里绕过 `Ch9329InputSink` 的严格模式路由，用底层
/// `CommandQueue` 直接构造 `Ch9329Command::MouseAbsolute`：
/// 按下（绝对帧 buttons=btn，坐标 (x,y)）→ 60ms 后释放（绝对帧 buttons=0，原位）。
///
/// 对比观测：在目标机（如 ARM 10.10.10.21）上 `evtest /dev/input/event6`，
/// 若绝对帧的按钮位有效，应看到 BTN_LEFT/BTN_RIGHT/BTN_MIDDLE 的 press/release；
/// 若无效（仅坐标被识别），则只有 ABS_X/ABS_Y 而没有任何按钮事件。
fn cmd_mouse_abs_btn<Q: CommandQueue>(queue: &Q, rest: &[String]) -> Result<(), String> {
    let x = rest
        .first()
        .ok_or("mouse-abs-btn 需要 x 参数")?
        .parse::<u32>()
        .map_err(|e| format!("无效 x：{e}"))?;
    let y = rest
        .get(1)
        .ok_or("mouse-abs-btn 需要 y 参数")?
        .parse::<u32>()
        .map_err(|e| format!("无效 y：{e}"))?;
    let buttons = rest
        .get(2)
        .ok_or("mouse-abs-btn 需要 btn 参数（0-7，bit0=左 bit1=右 bit2=中）")?;
    let buttons = parse_u8(buttons)?;
    if x > 4095 || y > 4095 {
        return Err(format!("绝对坐标超出 0-4095：({x},{y})"));
    }
    if buttons > 0x07 {
        return Err(format!("按钮掩码超出 0-7：{buttons:#04x}"));
    }

    let frame = |buttons: u8| {
        let report = AbsoluteMouseReport::new(buttons, x as u16, y as u16, 0)
            .map_err(|e| format!("构造绝对报告失败：{e}"))?;
        Ch9329Command::MouseAbsolute(report)
            .to_frame(0x00)
            .map_err(|e| format!("构造绝对帧失败：{e}"))
    };
    let down_batch =
        CommandBatch::new(vec![frame(buttons)?]).map_err(|e| format!("构造 batch 失败：{e}"))?;
    let up_batch =
        CommandBatch::new(vec![frame(0)?]).map_err(|e| format!("构造 batch 失败：{e}"))?;

    eprintln!("[probe] 绝对帧 0x04 buttons={buttons:#04x} 按下 @({x},{y})，60ms 后释放");
    queue
        .enqueue_batch(down_batch)
        .map_err(|e| format!("发送按下帧失败：{e}"))?;
    std::thread::sleep(Duration::from_millis(60));
    queue
        .enqueue_batch(up_batch)
        .map_err(|e| format!("发送释放帧失败：{e}"))?;
    Ok(())
}

/// 发送一个相对鼠标按钮边沿（走相对帧 0x05）。
///
/// 注意：主程序成功后会执行 release_all，所以 `down` 子命令不会跨进程保持按住；
/// 需要一次完整相对点击时使用 `click-rel`。
fn cmd_btn<Q: CommandQueue>(sink: &mut Ch9329InputSink<Q>, rest: &[String]) -> Result<(), String> {
    let button = parse_button(rest.first(), "btn")?;
    let down = match rest.get(1).map(|s| s.as_str()) {
        Some("down") => true,
        Some("up") => false,
        Some(other) => return Err(format!("未知动作：{other}（可选 down/up）")),
        None => return Err("btn 需要 down/up 参数".into()),
    };
    sink.set_mouse_mode(MouseMode::Relative)
        .map_err(|e| format!("切换相对鼠标模式: {e}"))?;
    sink.handle_pointer(PointerEvent::Button { button, down })
        .map_err(|e| format!("btn {button:?} {down}: {e}"))?;
    eprintln!("[probe] {button:?} {down}");
    Ok(())
}

/// 拖拽测试：绝对移动 A → 左键 down → 绝对移动 B → 左键 up，全程一个进程。
/// 在目标机上观察：若绝对移动期间按钮状态被芯片清掉，会在中间看到 BTN_LEFT 意外 up；
/// 若保持，则事件序列为 ABS(A) → BTN_LEFT down → ABS(B) → BTN_LEFT up。
fn cmd_drag<Q: CommandQueue>(sink: &mut Ch9329InputSink<Q>, rest: &[String]) -> Result<(), String> {
    let coords: Vec<u32> = rest
        .iter()
        .map(|s| s.parse::<u32>().map_err(|e| format!("无效坐标 {s:?}：{e}")))
        .collect::<Result<_, _>>()?;
    if coords.len() != 4 {
        return Err("drag 需要 4 个参数：x1 y1 x2 y2".into());
    }
    if coords.iter().any(|c| *c > 4095) {
        return Err("坐标超出 0-4095".into());
    }
    let [x1, y1, x2, y2] = coords[..] else {
        unreachable!()
    };
    let move_to = |x: u32, y: u32| PointerEvent::AbsoluteMove {
        x,
        y,
        framebuffer_size: ipkvm_core::FramebufferSize {
            width: 4096,
            height: 4096,
        },
    };

    eprintln!("[probe] drag ({x1},{y1}) → ({x2},{y2})，左键拖拽");
    sink.handle_pointer(move_to(x1, y1))
        .map_err(|e| format!("move A: {e}"))?;
    std::thread::sleep(Duration::from_millis(30));
    sink.handle_pointer(PointerEvent::Button {
        button: PointerButton::Left,
        down: true,
    })
    .map_err(|e| format!("btn down: {e}"))?;
    std::thread::sleep(Duration::from_millis(80));
    sink.handle_pointer(move_to(x2, y2))
        .map_err(|e| format!("move B: {e}"))?;
    std::thread::sleep(Duration::from_millis(80));
    sink.handle_pointer(PointerEvent::Button {
        button: PointerButton::Left,
        down: false,
    })
    .map_err(|e| format!("btn up: {e}"))?;
    Ok(())
}

fn cmd_click<Q: CommandQueue>(
    sink: &mut Ch9329InputSink<Q>,
    rest: &[String],
) -> Result<(), String> {
    let button = parse_button(rest.first(), "click")?;
    // 先移到屏幕中心（绝对 2048,2048），再点击。
    sink.handle_pointer(PointerEvent::AbsoluteMove {
        x: 2048,
        y: 2048,
        framebuffer_size: ipkvm_core::FramebufferSize {
            width: 4096,
            height: 4096,
        },
    })
    .map_err(|e| format!("move to center: {e}"))?;
    std::thread::sleep(Duration::from_millis(30));
    sink.handle_pointer(PointerEvent::Button { button, down: true })
        .map_err(|e| format!("button down: {e}"))?;
    std::thread::sleep(Duration::from_millis(40));
    sink.handle_pointer(PointerEvent::Button {
        button,
        down: false,
    })
    .map_err(|e| format!("button up: {e}"))?;
    eprintln!("[probe] 点击 {button:?}（中心位置）");
    Ok(())
}

fn cmd_click_rel<Q: CommandQueue>(
    sink: &mut Ch9329InputSink<Q>,
    rest: &[String],
) -> Result<(), String> {
    cmd_click_rel_with_hold(sink, rest, Duration::from_millis(60))
}

fn cmd_click_rel_with_hold<Q: CommandQueue>(
    sink: &mut Ch9329InputSink<Q>,
    rest: &[String],
    hold: Duration,
) -> Result<(), String> {
    let button = parse_button(rest.first(), "click-rel")?;
    sink.set_mouse_mode(MouseMode::Relative)
        .map_err(|e| format!("切换相对鼠标模式: {e}"))?;
    sink.handle_pointer(PointerEvent::Button { button, down: true })
        .map_err(|e| format!("button down: {e}"))?;
    std::thread::sleep(hold);
    sink.handle_pointer(PointerEvent::Button {
        button,
        down: false,
    })
    .map_err(|e| format!("button up: {e}"))?;
    eprintln!("[probe] 相对点击 {button:?}");
    Ok(())
}

fn parse_button(value: Option<&String>, command: &str) -> Result<PointerButton, String> {
    match value.map(|s| s.as_str()) {
        Some("left") => Ok(PointerButton::Left),
        Some("middle") => Ok(PointerButton::Middle),
        Some("right") => Ok(PointerButton::Right),
        Some(other) => Err(format!("未知按键：{other}（可选 left/middle/right）")),
        None => Err(format!("{command} 需要 left/middle/right 参数")),
    }
}

fn parse_u8(s: &str) -> Result<u8, String> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).map_err(|e| format!("无效十六进制 {s:?}：{e}"))
    } else {
        s.parse::<u8>().map_err(|e| format!("无效数字 {s:?}：{e}"))
    }
}

/// ASCII 字符 → HID Keyboard usage（USB HID Usage Tables，Keyboard/Keypad page）。
/// 仅覆盖可打印 ASCII；大小写不加 Shift（输出小写形式），Shift 由调用方按需。
fn ascii_to_usage(ch: char) -> Option<KeyboardUsage> {
    let usage: u8 = match ch {
        'a'..='z' => 0x04 + (ch as u8 - b'a'),
        'A'..='Z' => 0x04 + (ch as u8 - b'A'),
        '1'..='9' => 0x1e + (ch as u8 - b'1'),
        '0' => 0x27,
        ' ' => 0x2c,
        '\n' | '\r' => 0x28,
        '-' => 0x2d,
        '=' => 0x2e,
        '[' => 0x2f,
        ']' => 0x30,
        '\\' => 0x31,
        ';' => 0x33,
        '\'' => 0x34,
        '`' => 0x35,
        ',' => 0x36,
        '.' => 0x37,
        '/' => 0x38,
        _ => return None,
    };
    KeyboardUsage::new(usage).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_core::fake_serial::FakeCommandQueue;

    #[test]
    fn btn_command_uses_relative_frame_without_absolute_position() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);

        cmd_btn(&mut sink, &["left".into(), "down".into()]).unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        let frame = &batches[0].frames()[0];
        assert_eq!(
            frame.as_bytes(),
            &[0x57, 0xab, 0, 0x05, 0x05, 0x01, 0x01, 0, 0, 0, 0x0e]
        );
    }

    #[test]
    fn click_rel_command_sends_relative_down_then_up_frames() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);

        cmd_click_rel_with_hold(&mut sink, &["left".into()], Duration::ZERO).unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches[0].frames()[0].as_bytes(),
            &[0x57, 0xab, 0, 0x05, 0x05, 0x01, 0x01, 0, 0, 0, 0x0e]
        );
        assert_eq!(
            batches[1].frames()[0].as_bytes(),
            &[0x57, 0xab, 0, 0x05, 0x05, 0x01, 0, 0, 0, 0, 0x0d]
        );
    }
}

// CommandQueue 在 example 里需要泛型约束可见。
#[allow(dead_code)]
fn _assert_command_queue<Q: CommandQueue>(_: &Q) {}
