//! #89 取证：PreloadedImage 必须真实画出图像像素。
//!
//! 主视频/预览若继续用普通 `image::Image`，每帧新 Handle 会走异步上传，
//! 上传完成前该层空白（露出 letterbox 背景）→ 闪烁。`PreloadedImage` 在
//! layout 阶段持锁预上传 Allocation，draw 时立即可见；本测试用 tiny_skia
//! 离屏渲染确认像素真实出现，修复前应没有此 widget（编译失败=红）。

use iced::advanced::clipboard::Null;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::{Color, Point, Rectangle, Size};
use iced_runtime::user_interface::{self, UserInterface};
use ipkvm_desktop_iced::preloaded::PreloadedImage;
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;

#[test]
fn preloaded_image_draws_visible_pixels() {
    let handle = iced::widget::image::Handle::from_rgba(4, 4, [255u8, 0, 0, 255].repeat(16));
    let view: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
        iced::widget::container(PreloadedImage::new(handle))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();

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
        Color::from_rgb(0.1, 0.1, 0.1),
    );

    let drawn = count_non_background_pixels(&pixmap);
    assert!(
        drawn > 0,
        "PreloadedImage 必须渲染出图像像素（实际 {drawn} 个）"
    );
}

fn count_non_background_pixels(pixmap: &tiny_skia::Pixmap) -> usize {
    let background = [26u8, 26, 26]; // 0.1 背景色的 8bit 近似
    pixmap
        .pixels()
        .iter()
        .filter(|px| {
            let close = |a: u8, b: u8| (a as i16 - b as i16).unsigned_abs() <= 8;
            !(close(px.red(), background[0])
                && close(px.green(), background[1])
                && close(px.blue(), background[2]))
        })
        .count()
}
