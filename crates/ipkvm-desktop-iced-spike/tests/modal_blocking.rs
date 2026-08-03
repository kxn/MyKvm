//! Spike 2 模态验证：背景事件拦截 + 三条关闭路径（关闭按钮/点遮罩/Esc）。

use iced::mouse;
use iced::widget::{button, column, container, stack, text};
use iced::{Element, Event, Point};
use iced_test::simulator::{self, Simulator};
use ipkvm_desktop_iced_spike::modal::{self, ModalAction, ModalKind, ModalState};

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Modal(ModalAction),
    BackgroundPressed,
}

#[derive(Default)]
struct TestApp {
    modal: ModalState,
    bg_hits: u32,
}

impl TestApp {
    fn open(&mut self, kind: ModalKind) {
        self.modal.open(kind);
    }

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Modal(action) => match action {
                ModalAction::Close => self.modal.close(),
                ModalAction::SaveNameChanged(name) => self.modal.save_name = name,
                ModalAction::Save | ModalAction::Noop => {}
            },
            Msg::BackgroundPressed => self.bg_hits += 1,
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        // 背景放在左上角，避免与居中的模态卡片重叠。
        let background = container(column![
            button(text("Hit me")).on_press(Msg::BackgroundPressed),
            text(format!("hits: {}", self.bg_hits)),
        ])
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .align_x(iced::alignment::Horizontal::Left)
        .align_y(iced::alignment::Vertical::Top)
        .into();
        match self.modal.view() {
            Some(content) => stack![background, modal::overlay(content).map(Msg::Modal)].into(),
            None => background,
        }
    }
}

fn messages_of(ui: Simulator<'_, Msg>) -> Vec<Msg> {
    ui.into_messages().collect()
}

#[test]
fn modal_blocks_background_click_and_overlay_click_closes() {
    let _lock = ipkvm_desktop_iced_spike::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut app = TestApp::default();
    app.open(ModalKind::Settings);

    let mut ui = simulator::simulator(app.view());
    // 模态打开时点击背景按钮位置：必须被遮罩拦截，不能触发下层背景消息。
    assert!(ui.click("Hit me").is_ok(), "背景按钮必须可定位");
    let msgs = messages_of(ui);
    assert!(
        !msgs.contains(&Msg::BackgroundPressed),
        "模态打开时背景按钮不得收到点击（实际消息: {msgs:?}）"
    );
    assert!(
        msgs.contains(&Msg::Modal(ModalAction::Close)),
        "点遮罩必须产生 Close（实际消息: {msgs:?}）"
    );
    for msg in msgs {
        app.update(msg);
    }
    assert!(app.modal.open.is_none(), "点遮罩关闭后模态必须关闭");
}

#[test]
fn close_button_closes_modal() {
    let _lock = ipkvm_desktop_iced_spike::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut app = TestApp::default();
    app.open(ModalKind::About);

    let mut ui = simulator::simulator(app.view());
    assert!(ui.click("Close").is_ok(), "关闭按钮必须可定位");
    let msgs = messages_of(ui);
    assert!(
        msgs.contains(&Msg::Modal(ModalAction::Close)),
        "关闭按钮必须产生 Close（实际消息: {msgs:?}）"
    );
    assert!(!msgs.contains(&Msg::BackgroundPressed));
    for msg in msgs {
        app.update(msg);
    }
    assert!(app.modal.open.is_none(), "关闭按钮后模态必须关闭");
}

#[test]
fn escape_key_closes_modal() {
    let _lock = ipkvm_desktop_iced_spike::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut app = TestApp::default();
    app.open(ModalKind::SaveProfile);

    let mut ui = simulator::simulator(app.view());
    ui.tap_key(iced::keyboard::Key::Named(
        iced::keyboard::key::Named::Escape,
    ));
    let msgs = messages_of(ui);
    assert!(
        msgs.contains(&Msg::Modal(ModalAction::Close)),
        "Esc 必须产生 Close（实际消息: {msgs:?}）"
    );
    for msg in msgs {
        app.update(msg);
    }
    assert!(app.modal.open.is_none(), "Esc 后模态必须关闭");
}

#[test]
fn background_click_works_after_modal_closes() {
    let _lock = ipkvm_desktop_iced_spike::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut app = TestApp::default();
    app.open(ModalKind::Settings);
    // 先通过点遮罩关闭。
    {
        let mut ui = simulator::simulator(app.view());
        // 左上角远离卡片的位置 = 遮罩区域。
        ui.point_at(Point::new(4.0, 4.0));
        ui.simulate([
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        for msg in ui.into_messages() {
            app.update(msg);
        }
    }
    assert!(app.modal.open.is_none(), "遮罩点击必须先关闭模态");

    // 关闭后背景按钮恢复可交互。
    let mut ui = simulator::simulator(app.view());
    assert!(ui.click("Hit me").is_ok());
    let msgs = messages_of(ui);
    assert!(
        msgs.contains(&Msg::BackgroundPressed),
        "模态关闭后点击背景按钮必须恢复响应（实际消息: {msgs:?}）"
    );
}
