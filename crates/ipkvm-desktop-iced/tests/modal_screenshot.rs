//! 手动截图工具：渲染各模态并输出 PNG（改版式后核对视觉用）。
//!
//! 运行：cargo test -p ipkvm-desktop-iced --test modal_screenshot -- --ignored
//! 输出：仓库根 artifacts/ 目录下 modal-*.png（首次运行生成，之后断言一致）。

use iced_test::simulator;
use ipkvm_desktop_iced::modal::{ModalKind, ModalState};
use ipkvm_desktop_iced::theme::LIGHT;

fn render(kind: ModalKind, path: &str) {
    let mut state = ModalState::default();
    state.open(kind);
    // 用 overlay()：真实 app 渲染的是完整 overlay（遮罩 + 居中卡片）。
    let element = state.overlay().expect("modal should be open");
    let mut ui = simulator(element);
    let theme = iced::Theme::custom("my_ipkvm", LIGHT);
    let snapshot = ui.snapshot(&theme).expect("snapshot should render");
    snapshot
        .matches_image(path)
        .expect("screenshot should be written");
}

#[test]
#[ignore = "手动截图生成"]
fn generate_settings_screenshot() {
    render(ModalKind::Settings, "../../artifacts/modal-settings.png");
}

#[test]
#[ignore = "手动截图生成"]
fn generate_connection_screenshot() {
    render(
        ModalKind::Connection,
        "../../artifacts/modal-connection.png",
    );
}

#[test]
#[ignore = "手动截图生成"]
fn generate_save_screenshot() {
    render(ModalKind::SaveProfile, "../../artifacts/modal-save.png");
}

#[test]
#[ignore = "手动截图生成"]
fn generate_save_overwrite_screenshot() {
    let mut state = ModalState::default();
    state.open(ModalKind::SaveProfile);
    state.save_name = "办公室".into();
    state.confirm_overwrite = true;
    let element = state.overlay().expect("modal should be open");
    let mut ui = simulator(element);
    let theme = iced::Theme::custom("my_ipkvm", LIGHT);
    let snapshot = ui.snapshot(&theme).expect("snapshot should render");
    snapshot
        .matches_image("../../artifacts/modal-save-overwrite.png")
        .expect("screenshot should be written");
}

#[test]
#[ignore = "手动截图生成"]
fn generate_about_screenshot() {
    render(ModalKind::About, "../../artifacts/modal-about.png");
}
