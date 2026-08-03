//! #102 分步 1：BoundsRecorder 必须在 draw 时把视频区 bounds 写入共享 cell。
//!
//! iced update 阶段拿不到布局信息，只有 draw 阶段的 `Layout::bounds()`。
//! 本测试离屏渲染 320x180 画布，update + draw 后断言 cell 记录到视频区矩形；
//! 修复前 video_area 模块不存在（编译失败=红）。

use iced::advanced::clipboard::Null;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::{Point, Rectangle, Size};
use iced_runtime::user_interface::{self, UserInterface};
use ipkvm_desktop_iced::video_area::BoundsRecorder;
use std::cell::RefCell;
use std::rc::Rc;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

#[test]
fn bounds_recorder_records_video_area_bounds_on_draw() {
    let cell: Rc<RefCell<Option<Rectangle>>> = Rc::new(RefCell::new(None));
    let content: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        iced::widget::container(iced::widget::space())
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();
    let view: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        BoundsRecorder::new(cell.clone(), content).into();

    let mut renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), 16.0.into());
    let mut ui = UserInterface::build(
        view,
        Size::new(WIDTH as f32, HEIGHT as f32),
        user_interface::Cache::new(),
        &mut renderer,
    );

    let mut messages = Vec::new();
    let mut clipboard = Null;
    let _ = ui.update(
        &[],
        mouse::Cursor::Available(Point::ORIGIN),
        &mut renderer,
        &mut clipboard,
        &mut messages,
    );

    let theme = iced::Theme::Dark;
    let style = renderer::Style {
        text_color: theme.palette().text,
    };
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT).expect("pixmap");
    let mut mask = tiny_skia::Mask::new(WIDTH, HEIGHT).expect("mask");
    let viewport =
        iced::advanced::graphics::Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0);
    ui.draw(
        &mut renderer,
        &theme,
        &style,
        mouse::Cursor::Available(Point::ORIGIN),
    );
    renderer.draw(
        &mut pixmap.as_mut(),
        &mut mask,
        &viewport,
        &[Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32))],
        iced::Color::from_rgb(0.1, 0.1, 0.1),
    );

    assert_eq!(
        *cell.borrow(),
        Some(Rectangle::new(
            Point::ORIGIN,
            Size::new(WIDTH as f32, HEIGHT as f32)
        )),
        "draw 后 BoundsRecorder 必须把视频区 bounds 写入共享 cell"
    );
}
