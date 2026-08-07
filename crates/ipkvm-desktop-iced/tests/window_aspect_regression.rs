//! #48 回归测试：启动窗口视频区严格 16:9，不依赖 chrome 估算常量。
//!
//! 验证链路：measure_chrome_height（离屏实测）→ initial_window_size（实测 chrome）
//! → 视频区（窗口高 − chrome）与窗口宽严格满足 16:9。
//!
//! 另用 iced_test Simulator 独立渲染完整 menu+Fill(BoundsRecorder)+status 布局，
//! 断言 Simulator 实测的 chrome 与 measure_chrome_height 一致，且代入
//! initial_window_size 后视频区严格 16:9。这样无论字体/DPI/主题如何变化，
//! 启动窗口视频区都不会再出现比例失准导致的上下白条。

use iced::alignment::Vertical;
use iced::widget::{PickList, column, container, row, text};
use iced::{Element, Length, Size};
use iced_test::Simulator;
use ipkvm_desktop_iced::video_area::BoundsRecorder;
use std::cell::RefCell;
use std::rc::Rc;

const VIDEO_ASPECT: f32 = 16.0 / 9.0;

/// 复刻 App::view 顶/底结构：menu_bar + Fill 视频区(BoundsRecorder) + status_line 骨架。
fn app_like_view(cell: Rc<RefCell<Option<iced::Rectangle>>>) -> Element<'static, ()> {
    let menu: Element<'_, ipkvm_desktop_iced::menu::MenuAction> =
        ipkvm_desktop_iced::menu::menu_bar(
            &[],
            false,
            ipkvm_desktop_iced::locale::AppLanguage::System,
            true,
            true,
        );
    // Fill 视频区占位，包 BoundsRecorder。
    let video_inner: Element<'static, ()> = container(text(""))
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    let video: Element<'static, ()> = BoundsRecorder::new(cell, video_inner).into();
    // status_line 骨架：5 文本 + PickList(150)，container padding(6)。
    let options = vec!["profile".to_string()];
    let status_row = row![
        text("control"),
        text("keyboard"),
        text("pointer"),
        PickList::new(options.clone(), Some(options[0].clone()), |_| ())
            .width(Length::Fixed(150.0)),
        text("video"),
    ]
    .spacing(16)
    .align_y(Vertical::Center);
    let status: Element<'static, ()> = container(status_row).width(Length::Fill).padding(6).into();
    column![menu.map(|_| ()), video, status].into()
}

#[test]
fn measured_chrome_makes_initial_window_video_area_strictly_16_9() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let work_area = Size::new(1920.0, 1080.0);
    let font = ipkvm_desktop_iced::fonts::ui_font();
    let chrome = ipkvm_desktop_iced::measure_chrome_height(work_area.width, font);

    // chrome 必须落在合理区间（防止测量退化成 0 或异常值）。
    assert!(
        (20.0..200.0).contains(&chrome),
        "实测 chrome 应落在 20..200，实际 {chrome}"
    );

    let win = ipkvm_desktop_iced::initial_window_size(work_area, chrome);
    let video_h = win.height - chrome;
    let aspect = win.width / video_h;
    assert!(
        (aspect - VIDEO_ASPECT).abs() < 1e-3,
        "实测 chrome({chrome}) 代入后视频区宽高比 {aspect:.4} 必须严格 16:9({VIDEO_ASPECT:.4})"
    );
}

#[test]
fn simulator_independent_render_matches_measure_chrome_height() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    // 用 Simulator 独立渲染完整布局（与 measure_chrome_height 内部不同的渲染路径：
    // Simulator 用 fallback renderer，measure_chrome_height 用 iced_tiny_skia），
    // 验证两者实测的 chrome 一致。
    let cell: Rc<RefCell<Option<iced::Rectangle>>> = Rc::new(RefCell::new(None));
    let work_area_w = 1920.0_f32;
    // 窗口高度给足（视频区 Fill 撑开），宽度取工作区宽度。
    let window = Size::new(work_area_w, 900.0);
    let mut ui: Simulator<'static, ()> =
        Simulator::with_size(Default::default(), window, app_like_view(cell.clone()));
    let theme = iced::Theme::Light;
    let _ = ui.snapshot(&theme);

    let sim_chrome = match *cell.borrow() {
        Some(rect) => window.height - rect.height,
        None => panic!("Simulator snapshot 后 BoundsRecorder 必须记录视频区 bounds"),
    };

    let measured = ipkvm_desktop_iced::measure_chrome_height(
        work_area_w,
        ipkvm_desktop_iced::fonts::ui_font(),
    );
    // 两条独立渲染路径实测的 chrome 应一致（容许亚像素级差异）。
    assert!(
        (sim_chrome - measured).abs() < 1.0,
        "Simulator 实测 chrome({sim_chrome}) 与 measure_chrome_height({measured}) 偏差应 < 1px"
    );

    // 代入 initial_window_size：视频区严格 16:9。
    let win = ipkvm_desktop_iced::initial_window_size(Size::new(work_area_w, 1080.0), measured);
    let video_h = win.height - measured;
    let aspect = win.width / video_h;
    assert!(
        (aspect - VIDEO_ASPECT).abs() < 1e-3,
        "视频区宽高比 {aspect:.4} 必须严格 16:9"
    );
}
