//! 菜单：基于 iced_aw 0.14.1（vendored + 本地最小补丁）。
//!
//! 背景：#95。自绘菜单（#88 起）反复出现渲染/交互问题，改用现成实现。
//! iced_aw 0.14.1 有嵌套子菜单树状态 bug（operate 遍历越界 panic），
//! 本仓库通过 `[patch.crates-io]` 引入 vendored 副本并 backport 上游 main
//! 修复，详见 `third_party/iced_aw/PATCHES.md`。
//!
//! 打开/关闭状态由 iced_aw 在 widget 树内管理，本模块只负责：
//! - 菜单结构与 i18n 文案（view 构建时用 `rust_i18n::t!` 生成）；
//! - 菜单项动作映射（`MenuAction` 业务动作）；
//! - 菜单项观感（透明底按钮 + 悬停高亮、子菜单箭头、分隔线）。

use iced::widget::{button, row, text};
use iced::{Length, alignment};
use iced_aw::menu::{Item, Menu};
use iced_aw::menu_bar as menu_bar_macro;
use iced_aw::menu_items as menu_items_macro;
use rust_i18n::t;

use crate::modal::ModalKind;

/// 菜单业务动作（打开/关闭状态由 iced_aw 内部管理，不再有 Open*/Close* 消息）。
#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    /// 打开指定模态。
    OpenModal(ModalKind),
    /// 语言切换。
    SetLanguage(LanguageChoice),
    /// 最近使用 profile。
    LoadRecent(String),
    /// 特殊键发送。
    SpecialKey(String),
    /// 其它简单动作（退出/复制截图等）。
    Simple(&'static str),
}

/// 语言菜单选项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LanguageChoice {
    System,
    Chinese,
    English,
}

/// 构建完整菜单栏 Element（顶层 4 菜单 + 嵌套子菜单）。
pub fn menu_bar<'a, R>(recent_profiles: &[&str]) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    menu_bar_macro!(
        (root_label(t!("menu.file")), file_menu(recent_profiles)),
        (root_label(t!("menu.edit")), edit_menu()),
        (root_label(t!("menu.send")), send_menu()),
        (root_label(t!("menu.about")), about_menu()),
    )
    .close_on_item_click_global(true)
    .style(iced_aw::style::menu_bar::primary)
    .into()
}

// ---------------------------------------------------------------------------
// 菜单数据（复刻 egui 端结构）
// ---------------------------------------------------------------------------

fn file_menu<'a, R>(recent_profiles: &[&str]) -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    Menu::new(menu_items_macro!(
        (item_button(t!("menu.reselect_device"), MenuAction::Simple("reselect"))),
        (item_button(t!("menu.stop_connection"), MenuAction::Simple("stop"))),
        (separator()),
        (item_button(t!("file.load_profile"), MenuAction::Simple("load_profile"))),
        recent_item(recent_profiles),
        (separator()),
        (item_button(t!("file.exit"), MenuAction::Simple("exit"))),
    ))
    .width(240.0)
}

fn recent_item<'a, R>(recent_profiles: &[&str]) -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    let items: Vec<Item<'_, MenuAction, iced::Theme, R>> = if recent_profiles.is_empty() {
        vec![Item::new(text("(none)"))]
    } else {
        let mut items: Vec<Item<'_, MenuAction, iced::Theme, R>> = recent_profiles
            .iter()
            .take(3)
            .map(|name| {
                Item::new(item_button(
                    (*name).to_string(),
                    MenuAction::LoadRecent((*name).to_string()),
                ))
            })
            .collect();
        if recent_profiles.len() > 3 {
            let more_items: Vec<Item<'_, MenuAction, iced::Theme, R>> = recent_profiles
                .iter()
                .skip(3)
                .map(|name| {
                    Item::new(item_button(
                        (*name).to_string(),
                        MenuAction::LoadRecent((*name).to_string()),
                    ))
                })
                .collect();
            items.push(Item::with_menu(
                submenu_label(t!("file.recent_more")),
                Menu::new(more_items).width(240.0),
            ));
        }
        items
    };
    Item::with_menu(
        submenu_label(t!("file.recent")),
        Menu::new(items).width(240.0),
    )
}

fn edit_menu<'a, R>() -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    Menu::new(menu_items_macro!(
        (item_button(
            t!("edit.copy_screenshot"),
            MenuAction::Simple("copy_screenshot")
        )),
        (separator()),
        language_item(),
        (item_button(
            t!("edit.settings"),
            MenuAction::OpenModal(ModalKind::Settings)
        )),
    ))
    .width(240.0)
}

fn language_item<'a, R>() -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    Item::with_menu(
        submenu_label(t!("edit.language")),
        Menu::new(menu_items_macro!(
            (item_button(
                t!("language.system"),
                MenuAction::SetLanguage(LanguageChoice::System)
            )),
            (item_button(
                t!("language.chinese"),
                MenuAction::SetLanguage(LanguageChoice::Chinese)
            )),
            (item_button(
                t!("language.english"),
                MenuAction::SetLanguage(LanguageChoice::English)
            )),
        ))
        .width(240.0),
    )
}

fn send_menu<'a, R>() -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    Menu::new(menu_items_macro!(
        (item_button(t!("send.paste_text"), MenuAction::Simple("paste"))),
        (item_button(t!("send.release_all"), MenuAction::Simple("release_all"))),
        (separator()),
        special_keys_item(),
    ))
    .width(240.0)
}

fn special_keys_item<'a, R>() -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    Item::with_menu(
        submenu_label(t!("send.special_keys")),
        Menu::new(menu_items_macro!(
            (item_button(
                t!("special_keys.ctrl_alt_del"),
                MenuAction::SpecialKey("CtrlAltDel".into())
            )),
            (item_button(t!("special_keys.win"), MenuAction::SpecialKey("Win".into()))),
            (item_button(
                t!("special_keys.print_screen"),
                MenuAction::SpecialKey("PrintScreen".into())
            )),
            (item_button(
                t!("special_keys.alt_tab"),
                MenuAction::SpecialKey("AltTab".into())
            )),
        ))
        .width(240.0),
    )
}

fn about_menu<'a, R>() -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    Menu::new(menu_items_macro!(
        (item_button(
            t!("modal.about_title"),
            MenuAction::OpenModal(ModalKind::About)
        )),
        (item_button(t!("about.project_home"), MenuAction::Simple("project_home"))),
    ))
    .width(240.0)
}

// ---------------------------------------------------------------------------
// 菜单项 widget
// ---------------------------------------------------------------------------

/// 顶层根按钮：透明底，提供舒适的点击区域；打开状态高亮由 iced_aw 绘制。
fn root_label<'a, R>(label: impl Into<String>) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    button(text(label.into()))
        .padding([4, 8])
        .style(menu_item_style)
        .into()
}

/// 叶子菜单项：透明底按钮，点击发布业务动作；悬停高亮用主题主色半透明。
fn item_button<'a, R>(
    label: impl Into<String>,
    action: MenuAction,
) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    button(
        text(label.into())
            .width(Length::Fill)
            .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .padding([4, 10])
    .style(menu_item_style)
    .on_press(action)
    .into()
}

/// 子菜单父项：标签 + 右箭头（文本 "›"，不依赖 iced_aw 图标字体）。
fn submenu_label<'a, R>(label: impl Into<String>) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer + 'a,
{
    row![text(label.into()).width(Length::Fill), text("›")]
        .width(Length::Fill)
        .padding([4, 10])
        .align_y(alignment::Vertical::Center)
        .into()
}

/// 分隔线：1px 水平线。
fn separator<'a, R>() -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + 'a,
{
    iced::widget::rule::horizontal(1.0).into()
}

/// 菜单项统一样式：透明底 + 主题文字色；悬停/按下用主题主色半透明高亮。
fn menu_item_style(theme: &iced::Theme, status: button::Status) -> iced::widget::button::Style {
    use iced::Background;

    let palette = theme.palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => {
            Some(Background::Color(crate::theme::hover(palette)))
        }
        _ => Some(Background::Color(iced::Color::TRANSPARENT)),
    };
    iced::widget::button::Style {
        background,
        text_color: palette.text,
        border: Default::default(),
        shadow: Default::default(),
        snap: false,
    }
}
