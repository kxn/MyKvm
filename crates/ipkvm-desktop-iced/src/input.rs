//! 键鼠事件 → RFB keysym/pointer 事件的纯适配逻辑（iced 版，移植自 egui input.rs）。

use std::time::{Duration, Instant};

use iced::keyboard::Modifiers;
use iced::keyboard::key::Code;
use iced::mouse::ScrollDelta;

pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ALT_L: u32 = 0xffe9;
pub const XK_SUPER_L: u32 = 0xffeb;
pub const XK_TAB: u32 = 0xff09;
pub const XK_PRINT: u32 = 0xff61;
pub const XK_DELETE: u32 = 0xffff;

/// 键盘动作：按下/抬起某个 keysym。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Down(u32),
    Up(u32),
}

/// 本地 OS 会拦截、无法从键盘直发的组合键。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialKey {
    CtrlAltDel,
    Win,
    PrintScreen,
    AltTab,
}

/// 特殊键菜单名 → 枚举（spike 菜单发布的是名字字符串）。
pub fn special_key_from_menu(name: &str) -> Option<SpecialKey> {
    match name {
        "CtrlAltDel" => Some(SpecialKey::CtrlAltDel),
        "Win" => Some(SpecialKey::Win),
        "PrintScreen" => Some(SpecialKey::PrintScreen),
        "AltTab" => Some(SpecialKey::AltTab),
        _ => None,
    }
}

fn press(keysym: u32) -> Vec<KeyAction> {
    vec![KeyAction::Down(keysym), KeyAction::Up(keysym)]
}

/// 特殊键序列：Ctrl+Alt+Del 按下修饰键后按 Delete 再逆序释放；Win/PrintScreen
/// 单键；Alt+Tab 按下 Alt 后按 Tab 再逆序释放。
pub fn special_key_sequence(key: SpecialKey) -> Vec<KeyAction> {
    match key {
        SpecialKey::CtrlAltDel => vec![
            KeyAction::Down(XK_CONTROL_L),
            KeyAction::Down(XK_ALT_L),
            KeyAction::Down(XK_DELETE),
            KeyAction::Up(XK_DELETE),
            KeyAction::Up(XK_ALT_L),
            KeyAction::Up(XK_CONTROL_L),
        ],
        SpecialKey::Win => press(XK_SUPER_L),
        SpecialKey::PrintScreen => press(XK_PRINT),
        SpecialKey::AltTab => vec![
            KeyAction::Down(XK_ALT_L),
            KeyAction::Down(XK_TAB),
            KeyAction::Up(XK_TAB),
            KeyAction::Up(XK_ALT_L),
        ],
    }
}

/// Ctrl+Alt+K：本地退出远程输入模式（本地拦截，不转发远端）。
pub fn is_remote_exit_combo(code: Code, modifiers: Modifiers, repeat: bool) -> bool {
    code == Code::KeyK && !repeat && modifiers.control() && modifiers.alt()
}

/// Ctrl+Alt+M：本地切换绝对/相对鼠标模式（本地拦截，不转发远端）。
pub fn is_mode_toggle_combo(code: Code, modifiers: Modifiers, repeat: bool) -> bool {
    code == Code::KeyM && !repeat && modifiers.control() && modifiers.alt()
}

/// 修饰键状态变化：false→true 发 Down，true→false 发 Up；顺序固定
/// Shift → Ctrl → Alt → Logo，保证上下对称。
pub fn modifier_diff(previous: Modifiers, current: Modifiers) -> Vec<KeyAction> {
    let mut actions = Vec::new();
    diff_modifier(previous.shift(), current.shift(), XK_SHIFT_L, &mut actions);
    diff_modifier(
        previous.control(),
        current.control(),
        XK_CONTROL_L,
        &mut actions,
    );
    diff_modifier(previous.alt(), current.alt(), XK_ALT_L, &mut actions);
    diff_modifier(previous.logo(), current.logo(), XK_SUPER_L, &mut actions);
    actions
}

fn diff_modifier(previous: bool, current: bool, keysym: u32, actions: &mut Vec<KeyAction>) {
    match (previous, current) {
        (false, true) => actions.push(KeyAction::Down(keysym)),
        (true, false) => actions.push(KeyAction::Up(keysym)),
        _ => {}
    }
}

/// 滚轮增量换算成滚轮步数（Lines 直接取整，Pixels 按 50 点一步）。
pub fn wheel_steps(delta: ScrollDelta) -> i8 {
    let steps = match delta {
        ScrollDelta::Lines { y, .. } => y,
        ScrollDelta::Pixels { y, .. } => y / 50.0,
    };
    steps.round().clamp(i8::MIN as f32, i8::MAX as f32) as i8
}

/// 指针位置或按钮掩码是否变化。
pub fn pointer_changed(current: (u8, u16, u16), last: Option<(u8, u16, u16)>) -> bool {
    last != Some(current)
}

/// 距上次发送是否已超过最小间隔（限频用；从未发送过且有待发数据时立即发送）。
pub fn throttle_elapsed(now: Instant, last: Option<Instant>, interval: Duration) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= interval)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::Modifiers;
    use iced::keyboard::key::Code;
    use iced::mouse::ScrollDelta;
    use std::time::{Duration, Instant};

    fn mods(ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        let mut m = Modifiers::empty();
        m.set(Modifiers::CTRL, ctrl);
        m.set(Modifiers::ALT, alt);
        m.set(Modifiers::SHIFT, shift);
        m
    }

    #[test]
    fn ctrl_alt_del_sequence_presses_modifiers_before_delete_and_releases_reverse() {
        assert_eq!(
            special_key_sequence(SpecialKey::CtrlAltDel),
            vec![
                KeyAction::Down(0xffe3),
                KeyAction::Down(0xffe9),
                KeyAction::Down(0xffff),
                KeyAction::Up(0xffff),
                KeyAction::Up(0xffe9),
                KeyAction::Up(0xffe3),
            ]
        );
    }

    #[test]
    fn win_and_print_screen_are_single_press_release() {
        assert_eq!(
            special_key_sequence(SpecialKey::Win),
            vec![KeyAction::Down(0xffeb), KeyAction::Up(0xffeb)]
        );
        assert_eq!(
            special_key_sequence(SpecialKey::PrintScreen),
            vec![KeyAction::Down(0xff61), KeyAction::Up(0xff61)]
        );
    }

    #[test]
    fn alt_tab_holds_alt_while_tapping_tab() {
        assert_eq!(
            special_key_sequence(SpecialKey::AltTab),
            vec![
                KeyAction::Down(0xffe9),
                KeyAction::Down(0xff09),
                KeyAction::Up(0xff09),
                KeyAction::Up(0xffe9),
            ]
        );
    }

    #[test]
    fn special_key_from_menu_maps_names() {
        assert_eq!(
            special_key_from_menu("CtrlAltDel"),
            Some(SpecialKey::CtrlAltDel)
        );
        assert_eq!(special_key_from_menu("Win"), Some(SpecialKey::Win));
        assert_eq!(
            special_key_from_menu("PrintScreen"),
            Some(SpecialKey::PrintScreen)
        );
        assert_eq!(special_key_from_menu("AltTab"), Some(SpecialKey::AltTab));
        assert_eq!(special_key_from_menu("unknown"), None);
    }

    #[test]
    fn remote_exit_combo_requires_ctrl_alt_k_pressed_once() {
        assert!(is_remote_exit_combo(
            Code::KeyK,
            mods(true, true, false),
            false
        ));
        assert!(!is_remote_exit_combo(
            Code::KeyK,
            mods(true, true, false),
            true
        ));
        assert!(!is_remote_exit_combo(
            Code::KeyK,
            mods(true, false, false),
            false
        ));
        assert!(!is_remote_exit_combo(
            Code::KeyK,
            mods(false, true, false),
            false
        ));
        assert!(!is_remote_exit_combo(
            Code::KeyA,
            mods(true, true, false),
            false
        ));
    }

    #[test]
    fn mode_toggle_combo_requires_ctrl_alt_m_pressed_once() {
        assert!(is_mode_toggle_combo(
            Code::KeyM,
            mods(true, true, false),
            false
        ));
        assert!(!is_mode_toggle_combo(
            Code::KeyM,
            mods(true, true, false),
            true
        ));
        assert!(!is_mode_toggle_combo(
            Code::KeyM,
            mods(true, false, false),
            false
        ));
        assert!(!is_mode_toggle_combo(
            Code::KeyK,
            mods(true, true, false),
            false
        ));
    }

    #[test]
    fn modifier_diff_emits_down_then_up_in_stable_order() {
        let none = Modifiers::empty();
        let pressed = mods(true, true, true);
        assert_eq!(
            modifier_diff(none, pressed),
            vec![
                KeyAction::Down(0xffe1),
                KeyAction::Down(0xffe3),
                KeyAction::Down(0xffe9),
            ]
        );
        assert_eq!(
            modifier_diff(pressed, none),
            vec![
                KeyAction::Up(0xffe1),
                KeyAction::Up(0xffe3),
                KeyAction::Up(0xffe9),
            ]
        );
    }

    #[test]
    fn wheel_steps_converts_lines_and_pixels() {
        assert_eq!(wheel_steps(ScrollDelta::Lines { x: 0.0, y: 2.0 }), 2);
        assert_eq!(wheel_steps(ScrollDelta::Lines { x: 0.0, y: -1.0 }), -1);
        assert_eq!(wheel_steps(ScrollDelta::Pixels { x: 0.0, y: -100.0 }), -2);
        assert_eq!(wheel_steps(ScrollDelta::Pixels { x: 0.0, y: 25.0 }), 1);
    }

    #[test]
    fn pointer_changed_detects_position_or_mask_changes() {
        let last = Some((0, 100, 100));
        assert!(!pointer_changed((0, 100, 100), last));
        assert!(pointer_changed((1, 100, 100), last));
        assert!(pointer_changed((0, 101, 100), last));
        assert!(pointer_changed((0, 100, 100), None));
    }

    #[test]
    fn throttle_elapsed_requires_interval_to_pass() {
        let start = Instant::now();
        assert!(throttle_elapsed(start, None, Duration::from_millis(33)));
        assert!(throttle_elapsed(
            start + Duration::from_millis(34),
            Some(start),
            Duration::from_millis(33)
        ));
        assert!(!throttle_elapsed(
            start + Duration::from_millis(32),
            Some(start),
            Duration::from_millis(33)
        ));
    }
}
