//! #88 取证：菜单弹出层必须真的画出文字像素。
//!
//! iced 的 `UserInterface` 在 update 阶段用 overlay 实例 A 计算布局，在 draw 阶段
//! 用新建的实例 B 重画（只复用 A 的 layout 节点）。如果弹出层把自己的子控件树做成
//! “每次新建的临时 Tree”，B 里的 text 段落从未被 layout 填充，draw 时就只有空段落：
//! 面板/悬停块正常，文字一个像素都画不出来。
//!
//! 本测试用 tiny_skia 离屏渲染真实 `menu_bar`，统计弹出区域里“接近主题文字色”的像素数。
//! 修复前应为 0（复现 #88），修复后应大于 0（回归保护）。

use iced::advanced::clipboard::Null;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::{Color, Point, Rectangle, Size};
use iced_runtime::user_interface::{self, UserInterface};
use ipkvm_desktop_iced::menu::{MenuState, menu_bar};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

#[test]
fn menu_popup_draws_text_pixels() {
    rust_i18n::set_locale("en");

    let mut renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), 16.0.into());
    let state = MenuState {
        open_root: Some(0),
        open_path: Vec::new(),
    };
    let root = menu_bar::<iced_tiny_skia::Renderer>(&state, &[]);

    let mut ui = UserInterface::build(
        root,
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
        mouse::Cursor::Available(Point::ORIGIN),
    );
    renderer.draw(
        &mut pixmap.as_mut(),
        &mut mask,
        &viewport,
        &[Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32))],
        Color::from_rgb(0.1, 0.1, 0.1),
    );

    // 弹出菜单在菜单栏下方；只统计 y >= 40 的区域，避免把顶部根按钮文字算进来。
    let text_pixels = count_text_pixels(&pixmap, text_color, 40);
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
