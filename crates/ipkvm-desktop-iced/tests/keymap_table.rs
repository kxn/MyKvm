//! M3 键盘映射表验证：≥60 个代表键全部可映射（100% 通过）。

use iced::keyboard::key::Code;
use ipkvm_desktop_iced::keymap::{
    XK_ALT_L, XK_BACKSPACE, XK_CONTROL_L, XK_DELETE, XK_DOWN, XK_ESCAPE, XK_LEFT, XK_RETURN,
    XK_RIGHT, XK_SHIFT_L, XK_TAB, XK_UP, physical_code_to_keysym,
};

const COVERED: &[Code] = &[
    // 字母 26
    Code::KeyA,
    Code::KeyB,
    Code::KeyC,
    Code::KeyD,
    Code::KeyE,
    Code::KeyF,
    Code::KeyG,
    Code::KeyH,
    Code::KeyI,
    Code::KeyJ,
    Code::KeyK,
    Code::KeyL,
    Code::KeyM,
    Code::KeyN,
    Code::KeyO,
    Code::KeyP,
    Code::KeyQ,
    Code::KeyR,
    Code::KeyS,
    Code::KeyT,
    Code::KeyU,
    Code::KeyV,
    Code::KeyW,
    Code::KeyX,
    Code::KeyY,
    Code::KeyZ,
    // 数字 10
    Code::Digit0,
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
    // F1..F20
    Code::F1,
    Code::F2,
    Code::F3,
    Code::F4,
    Code::F5,
    Code::F6,
    Code::F7,
    Code::F8,
    Code::F9,
    Code::F10,
    Code::F11,
    Code::F12,
    Code::F13,
    Code::F14,
    Code::F15,
    Code::F16,
    Code::F17,
    Code::F18,
    Code::F19,
    Code::F20,
    // 标点/空格
    Code::Space,
    Code::Minus,
    Code::Equal,
    Code::BracketLeft,
    Code::BracketRight,
    Code::Backslash,
    Code::Semicolon,
    Code::Quote,
    Code::Backquote,
    Code::Comma,
    Code::Period,
    Code::Slash,
    // 编辑/导航
    Code::Backspace,
    Code::Tab,
    Code::Enter,
    Code::Escape,
    Code::Home,
    Code::End,
    Code::PageUp,
    Code::PageDown,
    Code::Insert,
    Code::Delete,
    // 方向键
    Code::ArrowLeft,
    Code::ArrowUp,
    Code::ArrowRight,
    Code::ArrowDown,
    // 修饰键
    Code::ShiftLeft,
    Code::ShiftRight,
    Code::ControlLeft,
    Code::ControlRight,
    Code::AltLeft,
    Code::AltRight,
    Code::SuperLeft,
    Code::SuperRight,
    // 其它
    Code::PrintScreen,
    Code::CapsLock,
    Code::NumLock,
    Code::ScrollLock,
    Code::Pause,
    Code::ContextMenu,
];

#[test]
fn table_covers_at_least_60_keys_with_no_holes() {
    assert!(
        COVERED.len() >= 60,
        "映射表至少 60 键（实际 {}）",
        COVERED.len()
    );
    for code in COVERED {
        assert!(
            physical_code_to_keysym(*code).is_some(),
            "{code:?} 必须可映射"
        );
    }
}

#[test]
fn representative_keys_map_to_expected_keysyms() {
    let cases = [
        (Code::KeyA, 0x61),
        (Code::KeyZ, 0x7a),
        (Code::Digit0, 0x30),
        (Code::Digit9, 0x39),
        (Code::F1, 0xffbe),
        (Code::F12, 0xffc9),
        (Code::Space, 0x20),
        (Code::Comma, ',' as u32),
        (Code::Backspace, XK_BACKSPACE),
        (Code::Tab, XK_TAB),
        (Code::Enter, XK_RETURN),
        (Code::Escape, XK_ESCAPE),
        (Code::Delete, XK_DELETE),
        (Code::ArrowLeft, XK_LEFT),
        (Code::ArrowUp, XK_UP),
        (Code::ArrowRight, XK_RIGHT),
        (Code::ArrowDown, XK_DOWN),
        (Code::ShiftLeft, XK_SHIFT_L),
        (Code::ControlLeft, XK_CONTROL_L),
        (Code::AltLeft, XK_ALT_L),
    ];
    for (code, expected) in cases {
        assert_eq!(physical_code_to_keysym(code), Some(expected), "{code:?}");
    }
}

#[test]
fn uncovered_keys_return_none() {
    assert_eq!(physical_code_to_keysym(Code::KanaMode), None);
    assert_eq!(physical_code_to_keysym(Code::Lang1), None);
}
