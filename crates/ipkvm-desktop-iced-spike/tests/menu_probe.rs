//! Spike 2 前置 probe：验证 iced_aw MenuBar 在 iced_test headless 下
//! 能否响应注入的鼠标事件（CursorMoved / click）。
//!
//! 这是 #73 Spike 2 走廊 hover 验证的前提。能响应 → 继续用 Simulator
//! 脚本化 100 次穿越；不能 → 降级为开关状态机纯函数 + 人工。

use iced::Event;
use iced::mouse;
use iced::widget::{button, text};
use iced::{Element, Point};
use iced_aw::menu::{Item, Menu, MenuBar};
use iced_test::simulator::{self, click};

// 抑制未使用警告（click 通过 ui.click(selector) 间接使用，simulate 直接用 Event）。
#[allow(unused_imports)]
use click as _click;

#[derive(Clone, Debug, PartialEq)]
enum Message {
    FileClicked,
    OpenClicked,
}

fn view(_state: &State) -> Element<'_, Message> {
    // 一个最小 MenuBar：File 顶层（带子菜单 Open/Save）+ Edit 顶层（无子菜单）。
    let open_item = Item::new(button(text("Open")).on_press(Message::OpenClicked));
    let save_item = Item::new(text("Save"));
    let file_menu = Menu::new(vec![open_item, save_item]);
    let file_item = Item::with_menu(button(text("File")), file_menu);
    let edit_item = Item::new(text("Edit"));

    MenuBar::new(vec![file_item, edit_item]).into()
}

struct State;

#[test]
fn probe_menubar_responds_to_click() {
    // probe 目标：click("File") 能否打开子菜单（"Open" 变得可 find/visible）。
    // 这是 MenuBar 在 headless 是否响应事件的核心判据。
    let state = State;
    let mut ui = simulator::simulator(view(&state));

    // 菜单未展开时，Open 不应可见。
    let open_before = ui.find("Open");
    println!("Open before click: {:?}", open_before.is_ok());

    // 点击 File 顶层按钮（click 自动定位 + 产生 ButtonPressed/Released）。
    let file_click = ui.click("File");
    println!("click File result: {:?}", file_click.is_ok());

    // 菜单展开后，Open 应可 find。
    let open_after = ui.find("Open");
    println!("Open after click(File): {:?}", open_after.is_ok());

    // 收集产生的消息。
    let messages: Vec<Message> = ui.into_messages().collect();
    println!("probe messages: {:?}", messages);

    if open_after.is_ok() {
        println!("PROBE RESULT: MenuBar 在 headless 响应 click（菜单展开，Open 可见）");
    } else {
        println!("PROBE RESULT: 菜单未展开——MenuBar 在 headless 可能不响应 overlay 渲染");
    }
}

#[test]
fn probe_menubar_responds_to_cursor_moved() {
    // probe 目标：CursorMoved 注入能否驱动 MenuBar 的 hover/走廊逻辑。
    // 先 click 打开 File 菜单，再注入 CursorMoved 在菜单区域，看子菜单是否保持打开。
    let state = State;
    let mut ui = simulator::simulator(view(&state));

    let _ = ui.click("File");
    let open_after_click = ui.find("Open").is_ok();

    // 注入 CursorMoved 在菜单区域（File 下方）。
    ui.simulate([Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(30.0, 50.0),
    })]);

    let open_after_move = ui.find("Open").is_ok();
    println!(
        "cursor probe: open after click={}, after CursorMoved={}",
        open_after_click, open_after_move
    );

    if open_after_click && open_after_move {
        println!("PROBE RESULT: CursorMoved 不误关菜单（走廊逻辑在 headless 工作）");
    } else if open_after_click && !open_after_move {
        println!("PROBE RESULT: CursorMoved 误关菜单（走廊逻辑在 headless 异常）");
    } else {
        println!("PROBE RESULT: 菜单根本没展开，无法验证 CursorMoved");
    }
}
