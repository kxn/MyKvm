//! iced 物理键 → X11 keysym 映射。
//!
//! winit/iced 的 `keyboard::key::Code` 是跨平台统一的物理键码（USB HID 风格），
//! 因此这张表一次实现、Windows/macOS 通用；keysym→HID usage 由 session 输入泵
//! 内部完成，本表只负责物理键码到 keysym。

use iced::keyboard::key::Code;

// X11 keysym（与 desktop/src/input.rs 一致）。
pub const XK_BACKSPACE: u32 = 0xff08;
pub const XK_TAB: u32 = 0xff09;
pub const XK_RETURN: u32 = 0xff0d;
pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_HOME: u32 = 0xff50;
pub const XK_LEFT: u32 = 0xff51;
pub const XK_UP: u32 = 0xff52;
pub const XK_RIGHT: u32 = 0xff53;
pub const XK_DOWN: u32 = 0xff54;
pub const XK_PAGE_UP: u32 = 0xff55;
pub const XK_PAGE_DOWN: u32 = 0xff56;
pub const XK_END: u32 = 0xff57;
pub const XK_INSERT: u32 = 0xff63;
pub const XK_DELETE: u32 = 0xffff;
pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_SHIFT_R: u32 = 0xffe2;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_CONTROL_R: u32 = 0xffe4;
pub const XK_CAPS_LOCK: u32 = 0xffe5;
pub const XK_ALT_L: u32 = 0xffe9;
pub const XK_ALT_R: u32 = 0xffea;
pub const XK_SUPER_L: u32 = 0xffeb;
pub const XK_SUPER_R: u32 = 0xffec;
pub const XK_PRINT: u32 = 0xff61;
pub const XK_PAUSE: u32 = 0xff13;
pub const XK_SCROLL_LOCK: u32 = 0xff14;
pub const XK_NUM_LOCK: u32 = 0xff7f;
pub const XK_MENU: u32 = 0xff67;
pub const XK_F1: u32 = 0xffbe;

/// 物理键码 → 基础 keysym（未按 Shift 变换大小写/符号；大小写由修饰键状态层处理，
/// 与旧桌面端的键码映射语义一致）。
pub fn physical_code_to_keysym(code: Code) -> Option<u32> {
    // winit 的 Code 是无字段枚举且字母/数字/F 键区间连续；用序数偏移映射。
    let ordinal = code as u32;
    if (Code::KeyA as u32..=Code::KeyZ as u32).contains(&ordinal) {
        return Some('a' as u32 + (ordinal - Code::KeyA as u32));
    }
    if (Code::Digit0 as u32..=Code::Digit9 as u32).contains(&ordinal) {
        return Some('0' as u32 + (ordinal - Code::Digit0 as u32));
    }
    if (Code::F1 as u32..=Code::F20 as u32).contains(&ordinal) {
        return Some(XK_F1 + (ordinal - Code::F1 as u32));
    }
    let keysym = match code {
        Code::Space => 0x20,
        Code::Minus => '-' as u32,
        Code::Equal => '=' as u32,
        Code::BracketLeft => '[' as u32,
        Code::BracketRight => ']' as u32,
        Code::Backslash => '\\' as u32,
        Code::Semicolon => ';' as u32,
        Code::Quote => '\'' as u32,
        Code::Backquote => '`' as u32,
        Code::Comma => ',' as u32,
        Code::Period => '.' as u32,
        Code::Slash => '/' as u32,
        Code::Backspace => XK_BACKSPACE,
        Code::Tab => XK_TAB,
        Code::Enter => XK_RETURN,
        Code::Escape => XK_ESCAPE,
        Code::Home => XK_HOME,
        Code::End => XK_END,
        Code::PageUp => XK_PAGE_UP,
        Code::PageDown => XK_PAGE_DOWN,
        Code::Insert => XK_INSERT,
        Code::Delete => XK_DELETE,
        Code::ArrowLeft => XK_LEFT,
        Code::ArrowUp => XK_UP,
        Code::ArrowRight => XK_RIGHT,
        Code::ArrowDown => XK_DOWN,
        Code::ShiftLeft => XK_SHIFT_L,
        Code::ShiftRight => XK_SHIFT_R,
        Code::ControlLeft => XK_CONTROL_L,
        Code::ControlRight => XK_CONTROL_R,
        Code::AltLeft => XK_ALT_L,
        Code::AltRight => XK_ALT_R,
        Code::SuperLeft => XK_SUPER_L,
        Code::SuperRight => XK_SUPER_R,
        Code::PrintScreen => XK_PRINT,
        Code::CapsLock => XK_CAPS_LOCK,
        Code::NumLock => XK_NUM_LOCK,
        Code::ScrollLock => XK_SCROLL_LOCK,
        Code::Pause => XK_PAUSE,
        Code::ContextMenu => XK_MENU,
        _ => return None,
    };
    Some(keysym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_representative_keys() {
        assert_eq!(physical_code_to_keysym(Code::KeyA), Some(0x61));
        assert_eq!(physical_code_to_keysym(Code::KeyZ), Some(0x7a));
        assert_eq!(physical_code_to_keysym(Code::Digit0), Some(0x30));
        assert_eq!(physical_code_to_keysym(Code::Digit9), Some(0x39));
        assert_eq!(physical_code_to_keysym(Code::F1), Some(XK_F1));
        assert_eq!(physical_code_to_keysym(Code::F12), Some(XK_F1 + 11));
        assert_eq!(physical_code_to_keysym(Code::ArrowUp), Some(XK_UP));
        assert_eq!(physical_code_to_keysym(Code::ArrowDown), Some(XK_DOWN));
        assert_eq!(physical_code_to_keysym(Code::ArrowLeft), Some(XK_LEFT));
        assert_eq!(physical_code_to_keysym(Code::ArrowRight), Some(XK_RIGHT));
        assert_eq!(physical_code_to_keysym(Code::Enter), Some(XK_RETURN));
        assert_eq!(physical_code_to_keysym(Code::Escape), Some(XK_ESCAPE));
        assert_eq!(physical_code_to_keysym(Code::Backspace), Some(XK_BACKSPACE));
        assert_eq!(physical_code_to_keysym(Code::Delete), Some(XK_DELETE));
        assert_eq!(physical_code_to_keysym(Code::Tab), Some(XK_TAB));
        assert_eq!(
            physical_code_to_keysym(Code::ControlLeft),
            Some(XK_CONTROL_L)
        );
        assert_eq!(physical_code_to_keysym(Code::AltRight), Some(XK_ALT_R));
        assert_eq!(physical_code_to_keysym(Code::Space), Some(0x20));
        assert_eq!(physical_code_to_keysym(Code::Comma), Some(',' as u32));
        assert_eq!(physical_code_to_keysym(Code::Slash), Some('/' as u32));
        // 未覆盖键返回 None（状态栏提示“不支持的按键”）。
        assert_eq!(physical_code_to_keysym(Code::KanaMode), None);
    }

    #[test]
    fn covers_at_least_60_keys() {
        let cases: [(Code, Option<u32>); 96] = [
            // 字母 26
            (Code::KeyA, Some(0x61)),
            (Code::KeyB, Some(0x62)),
            (Code::KeyC, Some(0x63)),
            (Code::KeyD, Some(0x64)),
            (Code::KeyE, Some(0x65)),
            (Code::KeyF, Some(0x66)),
            (Code::KeyG, Some(0x67)),
            (Code::KeyH, Some(0x68)),
            (Code::KeyI, Some(0x69)),
            (Code::KeyJ, Some(0x6a)),
            (Code::KeyK, Some(0x6b)),
            (Code::KeyL, Some(0x6c)),
            (Code::KeyM, Some(0x6d)),
            (Code::KeyN, Some(0x6e)),
            (Code::KeyO, Some(0x6f)),
            (Code::KeyP, Some(0x70)),
            (Code::KeyQ, Some(0x71)),
            (Code::KeyR, Some(0x72)),
            (Code::KeyS, Some(0x73)),
            (Code::KeyT, Some(0x74)),
            (Code::KeyU, Some(0x75)),
            (Code::KeyV, Some(0x76)),
            (Code::KeyW, Some(0x77)),
            (Code::KeyX, Some(0x78)),
            (Code::KeyY, Some(0x79)),
            (Code::KeyZ, Some(0x7a)),
            // 数字 10
            (Code::Digit0, Some(0x30)),
            (Code::Digit1, Some(0x31)),
            (Code::Digit2, Some(0x32)),
            (Code::Digit3, Some(0x33)),
            (Code::Digit4, Some(0x34)),
            (Code::Digit5, Some(0x35)),
            (Code::Digit6, Some(0x36)),
            (Code::Digit7, Some(0x37)),
            (Code::Digit8, Some(0x38)),
            (Code::Digit9, Some(0x39)),
            // F1..F20
            (Code::F1, Some(0xffbe)),
            (Code::F2, Some(0xffbf)),
            (Code::F3, Some(0xffc0)),
            (Code::F4, Some(0xffc1)),
            (Code::F5, Some(0xffc2)),
            (Code::F6, Some(0xffc3)),
            (Code::F7, Some(0xffc4)),
            (Code::F8, Some(0xffc5)),
            (Code::F9, Some(0xffc6)),
            (Code::F10, Some(0xffc7)),
            (Code::F11, Some(0xffc8)),
            (Code::F12, Some(0xffc9)),
            (Code::F13, Some(0xffca)),
            (Code::F14, Some(0xffcb)),
            (Code::F15, Some(0xffcc)),
            (Code::F16, Some(0xffcd)),
            (Code::F17, Some(0xffce)),
            (Code::F18, Some(0xffcf)),
            (Code::F19, Some(0xffd0)),
            (Code::F20, Some(0xffd1)),
            // 标点/空格
            (Code::Space, Some(0x20)),
            (Code::Minus, Some('-' as u32)),
            (Code::Equal, Some('=' as u32)),
            (Code::BracketLeft, Some('[' as u32)),
            (Code::BracketRight, Some(']' as u32)),
            (Code::Backslash, Some('\\' as u32)),
            (Code::Semicolon, Some(';' as u32)),
            (Code::Quote, Some('\'' as u32)),
            (Code::Backquote, Some('`' as u32)),
            (Code::Comma, Some(',' as u32)),
            (Code::Period, Some('.' as u32)),
            (Code::Slash, Some('/' as u32)),
            // 编辑/导航
            (Code::Backspace, Some(0xff08)),
            (Code::Tab, Some(0xff09)),
            (Code::Enter, Some(0xff0d)),
            (Code::Escape, Some(0xff1b)),
            (Code::Home, Some(0xff50)),
            (Code::End, Some(0xff57)),
            (Code::PageUp, Some(0xff55)),
            (Code::PageDown, Some(0xff56)),
            (Code::Insert, Some(0xff63)),
            (Code::Delete, Some(0xffff)),
            // 方向键
            (Code::ArrowLeft, Some(0xff51)),
            (Code::ArrowUp, Some(0xff52)),
            (Code::ArrowRight, Some(0xff53)),
            (Code::ArrowDown, Some(0xff54)),
            // 修饰键
            (Code::ShiftLeft, Some(0xffe1)),
            (Code::ShiftRight, Some(0xffe2)),
            (Code::ControlLeft, Some(0xffe3)),
            (Code::ControlRight, Some(0xffe4)),
            (Code::AltLeft, Some(0xffe9)),
            (Code::AltRight, Some(0xffea)),
            (Code::SuperLeft, Some(0xffeb)),
            (Code::SuperRight, Some(0xffec)),
            // 其它
            (Code::PrintScreen, Some(0xff61)),
            (Code::CapsLock, Some(0xffe5)),
            (Code::NumLock, Some(0xff7f)),
            (Code::ScrollLock, Some(0xff14)),
            (Code::Pause, Some(0xff13)),
            (Code::ContextMenu, Some(0xff67)),
        ];
        assert!(cases.len() >= 60, "映射表至少 60 键");
        for (code, expected) in cases {
            assert_eq!(physical_code_to_keysym(code), expected, "code {code:?}");
        }
    }
}
