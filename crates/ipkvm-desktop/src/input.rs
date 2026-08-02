//! egui 键鼠事件 → RFB keysym/pointer 事件的适配。

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
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ALT_L: u32 = 0xffe9;

const XK_SPACE: u32 = 0x20;
const XK_F1: u32 = 0xffbe;

/// Shift + 数字键的符号（0-9 顺序：`) ! @ # $ % ^ & * (`）。
const SHIFTED_DIGITS: [u32; 10] = [0x29, 0x21, 0x40, 0x23, 0x24, 0x25, 0x5e, 0x26, 0x2a, 0x28];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Down(u32),
    Up(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialKey {
    CtrlAltDel,
    Escape,
    F(u8),
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowUp,
    ArrowRight,
    ArrowDown,
}

fn press(keysym: u32) -> Vec<KeyAction> {
    vec![KeyAction::Down(keysym), KeyAction::Up(keysym)]
}

/// 特殊键序列：Ctrl+Alt+Del 按下修饰键后按 Delete、再逆序释放；其余单键按下+释放。
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
        SpecialKey::Escape => press(XK_ESCAPE),
        SpecialKey::F(n) => press(XK_F1 + n.saturating_sub(1) as u32),
        SpecialKey::Insert => press(XK_INSERT),
        SpecialKey::Delete => press(XK_DELETE),
        SpecialKey::Home => press(XK_HOME),
        SpecialKey::End => press(XK_END),
        SpecialKey::PageUp => press(XK_PAGE_UP),
        SpecialKey::PageDown => press(XK_PAGE_DOWN),
        SpecialKey::ArrowLeft => press(XK_LEFT),
        SpecialKey::ArrowUp => press(XK_UP),
        SpecialKey::ArrowRight => press(XK_RIGHT),
        SpecialKey::ArrowDown => press(XK_DOWN),
    }
}

/// egui 键 → X11 keysym。字母按 Shift 决定大小写，数字按 Shift 映射符号，
/// 控制键走 X11 专用 keysym；未覆盖键返回 None（状态栏提示“不支持的按键”）。
pub fn egui_key_to_keysym(
    key: eframe::egui::Key,
    modifiers: eframe::egui::Modifiers,
) -> Option<u32> {
    use eframe::egui::Key::*;
    match key {
        Backspace => Some(XK_BACKSPACE),
        Enter => Some(XK_RETURN),
        Space => Some(XK_SPACE),
        Tab => Some(XK_TAB),
        Escape => Some(XK_ESCAPE),
        Delete => Some(XK_DELETE),
        Insert => Some(XK_INSERT),
        Home => Some(XK_HOME),
        End => Some(XK_END),
        PageUp => Some(XK_PAGE_UP),
        PageDown => Some(XK_PAGE_DOWN),
        ArrowLeft => Some(XK_LEFT),
        ArrowRight => Some(XK_RIGHT),
        ArrowUp => Some(XK_UP),
        ArrowDown => Some(XK_DOWN),
        k if (eframe::egui::Key::A as u32..=eframe::egui::Key::Z as u32).contains(&(k as u32)) => {
            let index = k as u32 - eframe::egui::Key::A as u32;
            Some(if modifiers.shift {
                'A' as u32 + index
            } else {
                'a' as u32 + index
            })
        }
        k if (eframe::egui::Key::Num0 as u32..=eframe::egui::Key::Num9 as u32)
            .contains(&(k as u32)) =>
        {
            let index = (k as u32 - eframe::egui::Key::Num0 as u32) as usize;
            Some(if modifiers.shift {
                SHIFTED_DIGITS[index]
            } else {
                '0' as u32 + index as u32
            })
        }
        k if (eframe::egui::Key::F1 as u32..=eframe::egui::Key::F20 as u32)
            .contains(&(k as u32)) =>
        {
            Some(XK_F1 + (k as u32 - eframe::egui::Key::F1 as u32))
        }
        _ => None,
    }
}

/// 计算两个帧间修饰键变化：false→true 发 Down，true→false 发 Up；顺序固定
/// Shift → Ctrl → Alt，保证上下对称。
pub fn modifier_diff(
    previous: eframe::egui::Modifiers,
    current: eframe::egui::Modifiers,
) -> Vec<KeyAction> {
    let mut actions = Vec::new();
    diff_modifier(previous.shift, current.shift, XK_SHIFT_L, &mut actions);
    diff_modifier(previous.ctrl, current.ctrl, XK_CONTROL_L, &mut actions);
    diff_modifier(previous.alt, current.alt, XK_ALT_L, &mut actions);
    actions
}

fn diff_modifier(previous: bool, current: bool, keysym: u32, actions: &mut Vec<KeyAction>) {
    match (previous, current) {
        (false, true) => actions.push(KeyAction::Down(keysym)),
        (true, false) => actions.push(KeyAction::Up(keysym)),
        _ => {}
    }
}

/// 当前指针按键掩码：Primary=1、Secondary=2、Middle=4。
///
/// 指针仍按住但按钮状态事件短暂中断时，沿用上一帧掩码，避免误发抬键。
pub fn pointer_button_mask(response: &eframe::egui::Response, previous_mask: u8) -> u8 {
    let mut mask = 0;
    if response
        .ctx
        .input(|i| i.pointer.button_down(eframe::egui::PointerButton::Primary))
    {
        mask |= 0b001;
    }
    if response.ctx.input(|i| {
        i.pointer
            .button_down(eframe::egui::PointerButton::Secondary)
    }) {
        mask |= 0b010;
    }
    if response
        .ctx
        .input(|i| i.pointer.button_down(eframe::egui::PointerButton::Middle))
    {
        mask |= 0b100;
    }
    if mask == 0 && previous_mask != 0 && response.ctx.input(|i| i.pointer.any_down()) {
        mask = previous_mask;
    }
    mask
}

/// 指针输入是否活跃：视频区聚焦即活跃；未聚焦时只有按住（拖出窗口或
/// 松开在窗口外）才继续发送，避免悬停误动目标机鼠标。
pub fn pointer_active(focused: bool, mask: u8, previous_mask: u8) -> bool {
    focused || mask != 0 || previous_mask != 0
}

/// 远程输入模式下的 egui 焦点锁：Tab/方向键/Esc 都留在视频面板，
/// 不参与 egui 焦点导航（防止方向键把焦点移到菜单栏导致输入中断）。
pub fn remote_focus_filter() -> eframe::egui::EventFilter {
    eframe::egui::EventFilter {
        tab: true,
        horizontal_arrows: true,
        vertical_arrows: true,
        escape: true,
    }
}

/// Ctrl+Alt+K：本地退出远程输入模式的组合键（本地拦截，不转发远端）。
pub fn is_remote_exit_combo(event: &eframe::egui::Event) -> bool {
    matches!(
        event,
        eframe::egui::Event::Key {
            key: eframe::egui::Key::K,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if modifiers.ctrl && modifiers.alt
    )
}

/// Ctrl+Alt+M：本地切换绝对/相对鼠标模式（本地拦截，不转发远端）。
pub fn is_mode_toggle_combo(event: &eframe::egui::Event) -> bool {
    matches!(
        event,
        eframe::egui::Event::Key {
            key: eframe::egui::Key::M,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if modifiers.ctrl && modifiers.alt
    )
}

/// 把浮点增量累积到余数并返回可发送的整数增量（避免亚像素漂移）。
pub fn accumulate_delta(remainder: &mut (f32, f32), dx: f32, dy: f32) -> (i16, i16) {
    remainder.0 += dx;
    remainder.1 += dy;
    let ix = remainder.0.trunc().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    let iy = remainder.1.trunc().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    remainder.0 -= ix as f32;
    remainder.1 -= iy as f32;
    (ix, iy)
}

/// 把 egui 滚轮增量换算成滚轮步数（Line/Page 直接取整，Point 按 50 点一步）。
pub fn wheel_steps(unit: eframe::egui::MouseWheelUnit, delta_y: f32) -> i8 {
    let steps = match unit {
        eframe::egui::MouseWheelUnit::Line | eframe::egui::MouseWheelUnit::Page => delta_y,
        eframe::egui::MouseWheelUnit::Point => delta_y / 50.0,
    };
    steps.round().clamp(i8::MIN as f32, i8::MAX as f32) as i8
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use super::*;

    #[test]
    fn ctrl_alt_del_sequence_presses_modifiers_before_delete_and_releases_reverse() {
        assert_eq!(
            special_key_sequence(SpecialKey::CtrlAltDel),
            vec![
                KeyAction::Down(XK_CONTROL_L),
                KeyAction::Down(XK_ALT_L),
                KeyAction::Down(XK_DELETE),
                KeyAction::Up(XK_DELETE),
                KeyAction::Up(XK_ALT_L),
                KeyAction::Up(XK_CONTROL_L),
            ]
        );
    }

    #[test]
    fn function_keys_use_x11_keysym_range() {
        assert_eq!(
            special_key_sequence(SpecialKey::F(1))[0],
            KeyAction::Down(0xffbe)
        );
        assert_eq!(
            special_key_sequence(SpecialKey::F(12))[0],
            KeyAction::Down(0xffc9)
        );
    }

    #[test]
    fn letters_respect_shift_for_case() {
        assert_eq!(
            egui_key_to_keysym(egui::Key::A, egui::Modifiers::NONE),
            Some(0x61)
        );
        assert_eq!(
            egui_key_to_keysym(
                egui::Key::A,
                egui::Modifiers {
                    shift: true,
                    ..Default::default()
                }
            ),
            Some(0x41)
        );
    }

    #[test]
    fn digits_map_shifted_symbols() {
        assert_eq!(
            egui_key_to_keysym(egui::Key::Num1, egui::Modifiers::NONE),
            Some(0x31)
        );
        assert_eq!(
            egui_key_to_keysym(
                egui::Key::Num1,
                egui::Modifiers {
                    shift: true,
                    ..Default::default()
                }
            ),
            Some(0x21)
        );
    }

    #[test]
    fn unsupported_keys_return_none() {
        assert_eq!(
            egui_key_to_keysym(egui::Key::Minus, egui::Modifiers::NONE),
            None
        );
    }

    #[test]
    fn modifier_diff_emits_down_then_up_in_stable_order() {
        let none = egui::Modifiers::NONE;
        let pressed = egui::Modifiers {
            shift: true,
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            modifier_diff(none, pressed),
            vec![KeyAction::Down(XK_SHIFT_L), KeyAction::Down(XK_CONTROL_L)]
        );
        assert_eq!(
            modifier_diff(pressed, none),
            vec![KeyAction::Up(XK_SHIFT_L), KeyAction::Up(XK_CONTROL_L)]
        );
    }

    #[test]
    fn pointer_button_mask_tracks_held_buttons() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));

        let mut pressed = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        pressed.events.push(egui::Event::PointerButton {
            pos: egui::pos2(100.0, 100.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let mut response = None;
        let _ = ctx.run(pressed, |ctx| {
            response = Some(
                egui::CentralPanel::default()
                    .show(ctx, |ui| {
                        ui.allocate_response(ui.available_size(), egui::Sense::click_and_drag())
                    })
                    .inner,
            );
        });
        let response = response.unwrap();
        assert_eq!(pointer_button_mask(&response, 0), 1);

        let mut released = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        released.events.push(egui::Event::PointerButton {
            pos: egui::pos2(100.0, 100.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(released, |_| {});
        assert_eq!(pointer_button_mask(&response, 1), 0);
    }

    #[test]
    fn pointer_active_requires_focus_or_held_button() {
        assert!(!pointer_active(false, 0, 0));
        assert!(pointer_active(true, 0, 0));
        assert!(pointer_active(false, 1, 0));
        assert!(pointer_active(false, 0, 1));
    }

    #[test]
    fn remote_focus_filter_keeps_navigation_keys_in_remote_mode() {
        let filter = remote_focus_filter();
        for key in [
            egui::Key::Tab,
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::Escape,
        ] {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            assert!(
                filter.matches(&event),
                "navigation key {key:?} must stay in remote mode"
            );
        }
    }

    #[test]
    fn remote_exit_combo_requires_ctrl_alt_k_pressed_once() {
        let combo = |key: egui::Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers| {
            is_remote_exit_combo(&egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat,
                modifiers,
            })
        };
        let ctrl_alt = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert!(combo(egui::Key::K, true, false, ctrl_alt));
        assert!(!combo(egui::Key::K, false, false, ctrl_alt));
        assert!(!combo(egui::Key::K, true, true, ctrl_alt));
        assert!(!combo(
            egui::Key::K,
            true,
            false,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
        assert!(!combo(
            egui::Key::K,
            true,
            false,
            egui::Modifiers {
                alt: true,
                ..Default::default()
            }
        ));
        assert!(!combo(egui::Key::A, true, false, ctrl_alt));
    }

    #[test]
    fn mode_toggle_combo_requires_ctrl_alt_m_pressed_once() {
        let combo = |key: egui::Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers| {
            is_mode_toggle_combo(&egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat,
                modifiers,
            })
        };
        let ctrl_alt = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert!(combo(egui::Key::M, true, false, ctrl_alt));
        assert!(!combo(egui::Key::M, false, false, ctrl_alt));
        assert!(!combo(egui::Key::M, true, true, ctrl_alt));
        assert!(!combo(
            egui::Key::M,
            true,
            false,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
        assert!(!combo(egui::Key::K, true, false, ctrl_alt));
    }

    #[test]
    fn accumulate_delta_sends_integer_parts_and_keeps_remainder() {
        let mut remainder = (0.0, 0.0);
        assert_eq!(accumulate_delta(&mut remainder, 1.6, 2.4), (1, 2));
        assert!((remainder.0 - 0.6).abs() < 1e-6);
        assert!((remainder.1 - 0.4).abs() < 1e-6);
        assert_eq!(accumulate_delta(&mut remainder, 0.4, 0.6), (1, 1));
        assert!((remainder.0 - 0.0).abs() < 1e-6);
        assert!((remainder.1 - 0.0).abs() < 1e-6);
    }

    #[test]
    fn wheel_steps_converts_lines_and_points() {
        assert_eq!(wheel_steps(egui::MouseWheelUnit::Line, 2.0), 2);
        assert_eq!(wheel_steps(egui::MouseWheelUnit::Point, -100.0), -2);
        assert_eq!(wheel_steps(egui::MouseWheelUnit::Page, 1.0), 1);
    }
}
