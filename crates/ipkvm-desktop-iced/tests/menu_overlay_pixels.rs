//! #88 回归保护：菜单弹出层必须真的画出文字像素。
//!
//! 自绘菜单时期 #88 是弹出层文字整体不渲染/右偏；改用 iced_aw 后文字走
//! 普通 widget 绘制管线，这里保留一个像素级冒烟：真实点击打开 File 菜单，
//! 离屏渲染后统计弹出区域内接近主题文字色的像素数必须大于 0。

use iced::advanced::clipboard::Null;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::{Color, Point, Rectangle, Size};
use iced_runtime::user_interface::{self, UserInterface};
use ipkvm_desktop_iced::menu::{self, MenuAction};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

#[test]
fn menu_popup_draws_text_pixels() {
    rust_i18n::set_locale("en");

    let mut renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), 16.0.into());
    let root: iced::Element<'static, MenuAction, iced::Theme, iced_tiny_skia::Renderer> =
        menu::menu_bar(&[]);

    let mut ui = UserInterface::build(
        root,
        Size::new(WIDTH as f32, HEIGHT as f32),
        user_interface::Cache::new(),
        &mut renderer,
    );

    let mut messages = Vec::new();
    let mut clipboard = Null;
    // 点击顶部第一个根按钮（File）打开菜单。MenuBar padding 为 0，
    // 根按钮从 (0,0) 开始，File 按钮中心约在 (18, 12)。
    let click_at = Point::new(18.0, 12.0);
    let _ = ui.update(
        &[
            iced::Event::Mouse(mouse::Event::CursorMoved { position: click_at }),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ],
        mouse::Cursor::Available(click_at),
        &mut renderer,
        &mut clipboard,
        &mut messages,
    );

    let theme = iced::Theme::Light;
    let text_color = theme.palette().text;
    let style = renderer::Style {
        text_color,
        ..renderer::Style::default()
    };
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT).expect("pixmap");
    let mut mask = tiny_skia::Mask::new(WIDTH, HEIGHT).expect("mask");
    let viewport =
        iced::advanced::graphics::Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0);
    ui.draw(
        &mut renderer,
        &theme,
        &style,
        mouse::Cursor::Available(click_at),
    );
    renderer.draw(
        &mut pixmap.as_mut(),
        &mut mask,
        &viewport,
        &[Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32))],
        Color::from_rgb(0.1, 0.1, 0.1),
    );

    // 菜单栏高度约 24px；只统计 y >= 30 的区域，避免把顶部根按钮文字算进来。
    let text_pixels = count_text_pixels(&pixmap, text_color, 30);
    let root_pixels = count_text_pixels(&pixmap, text_color, 0) - text_pixels;
    assert!(
        root_pixels > 0,
        "对照失败：顶部根菜单也必须渲染出文字像素（实际 {root_pixels} 个），检查测试环境字体"
    );
    assert!(
        text_pixels > 0,
        "菜单弹出层必须渲染出文字像素（实际 {text_pixels} 个）"
    );
}

fn count_text_pixels(pixmap: &tiny_skia::Pixmap, text_color: Color, min_y: u32) -> usize {
    let target = [
        (text_color.r * 255.0) as u8,
        (text_color.g * 255.0) as u8,
        (text_color.b * 255.0) as u8,
    ];
    let tolerance = 32u16;
    pixmap
        .pixels()
        .iter()
        .enumerate()
        .filter(|(i, px)| {
            let y = (*i as u32) / WIDTH;
            if y < min_y {
                return false;
            }
            let close = |a: u8, b: u8| (a as i16 - b as i16).unsigned_abs() <= tolerance;
            close(px.red(), target[0])
                && close(px.green(), target[1])
                && close(px.blue(), target[2])
        })
        .count()
}
