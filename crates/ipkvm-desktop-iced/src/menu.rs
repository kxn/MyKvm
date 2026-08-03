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
    /// 断开当前连接并回到连接页。
    Disconnect,
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

impl LanguageChoice {
    pub fn label(self) -> String {
        match self {
            LanguageChoice::System => t!("language.system").to_string(),
            LanguageChoice::Chinese => t!("language.chinese").to_string(),
            LanguageChoice::English => t!("language.english").to_string(),
        }
    }
}

impl From<LanguageChoice> for crate::locale::AppLanguage {
    fn from(choice: LanguageChoice) -> Self {
        match choice {
            LanguageChoice::System => crate::locale::AppLanguage::System,
            LanguageChoice::Chinese => crate::locale::AppLanguage::Chinese,
            LanguageChoice::English => crate::locale::AppLanguage::English,
        }
    }
}

/// 构建完整菜单栏 Element（顶层 4 菜单 + 嵌套子菜单）。
pub fn menu_bar<'a, R>(
    recent_profiles: &[&str],
    paste_busy: bool,
    language: crate::locale::AppLanguage,
    online: bool,
    has_frame: bool,
) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    menu_bar_macro!(
        (
            root_label(t!("menu.file")),
            file_menu(recent_profiles, online)
        ),
        (root_label(t!("menu.edit")), edit_menu(language, has_frame)),
        (root_label(t!("menu.send")), send_menu(paste_busy)),
        (root_label(t!("menu.about")), about_menu()),
    )
    .close_on_item_click_global(true)
    .style(iced_aw::style::menu_bar::primary)
    .into()
}

// ---------------------------------------------------------------------------
// 菜单数据（复刻 egui 端结构）
// ---------------------------------------------------------------------------

fn file_menu<'a, R>(recent_profiles: &[&str], online: bool) -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    Menu::new(menu_items_macro!(
        action_item(t!("menu.disconnect"), MenuAction::Disconnect, online),
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
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    let items: Vec<Item<'_, MenuAction, iced::Theme, R>> = if recent_profiles.is_empty() {
        vec![Item::new(
            text(t!("profile.no_recent")).font(crate::fonts::ui_font()),
        )]
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

fn edit_menu<'a, R>(
    language: crate::locale::AppLanguage,
    has_frame: bool,
) -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    Menu::new(menu_items_macro!(
        action_item(
            t!("edit.copy_screenshot"),
            MenuAction::Simple("copy_screenshot"),
            has_frame
        ),
        save_screenshot_item(has_frame),
        (separator()),
        language_item(language),
        (item_button(
            t!("edit.settings"),
            MenuAction::OpenModal(ModalKind::Settings)
        )),
    ))
    .width(240.0)
}

fn save_screenshot_item<'a, R>(has_frame: bool) -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    #[cfg(windows)]
    {
        action_item(
            t!("edit.save_screenshot"),
            MenuAction::Simple("save_screenshot"),
            has_frame,
        )
    }
    #[cfg(not(windows))]
    {
        Item::new(
            button(text(t!("edit.save_screenshot_unsupported")).font(crate::fonts::ui_font()))
                .width(Length::Fill)
                .padding([4, 10])
                .style(menu_item_style),
        )
    }
}

fn language_item<'a, R>(current: crate::locale::AppLanguage) -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    Item::with_menu(
        submenu_label(t!("edit.language")),
        Menu::new(menu_items_macro!(
            (language_option(LanguageChoice::System, current)),
            (language_option(LanguageChoice::Chinese, current)),
            (language_option(LanguageChoice::English, current)),
        ))
        .width(240.0),
    )
}

fn language_option<'a, R>(
    option: LanguageChoice,
    current: crate::locale::AppLanguage,
) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    let selected = crate::locale::AppLanguage::from(option) == current;
    let label = if selected {
        format!("✓ {}", option.label())
    } else {
        option.label()
    };
    item_button(label, MenuAction::SetLanguage(option))
}

fn send_menu<'a, R>(paste_busy: bool) -> Menu<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    Menu::new(menu_items_macro!(
        action_item(
            t!("send.paste_text"),
            MenuAction::Simple("paste"),
            !paste_busy
        ),
        (item_button(t!("send.release_all"), MenuAction::Simple("release_all"))),
        (separator()),
        special_keys_item(),
    ))
    .width(240.0)
}

fn special_keys_item<'a, R>() -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
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
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
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
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    button(text(label.into()).font(crate::fonts::ui_font()))
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
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    button(
        text(label.into())
            .font(crate::fonts::ui_font())
            .width(Length::Fill)
            .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .padding([4, 10])
    .style(menu_item_style)
    .on_press(action)
    .into()
}

/// 菜单项：可用时发布动作，禁用时无 `on_press`（iced 渲染为禁用态）+ 置灰。
fn action_item<'a, R>(
    label: impl Into<String>,
    action: MenuAction,
    enabled: bool,
) -> Item<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    let label = label.into();
    if enabled {
        Item::new(item_button(label, action))
    } else {
        Item::new(
            button(
                text(label)
                    .font(crate::fonts::ui_font())
                    .width(Length::Fill)
                    .align_y(alignment::Vertical::Center),
            )
            .width(Length::Fill)
            .padding([4, 10])
            .style(menu_item_style),
        )
    }
}

/// 子菜单父项：标签 + 右箭头（文本 "›"，不依赖 iced_aw 图标字体）。
fn submenu_label<'a, R>(label: impl Into<String>) -> iced::Element<'a, MenuAction, iced::Theme, R>
where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    row![
        text(label.into())
            .font(crate::fonts::ui_font())
            .width(Length::Fill),
        text("›").font(crate::fonts::ui_font())
    ]
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
    let text_color = if matches!(status, button::Status::Disabled) {
        iced::Color {
            a: palette.text.a * 0.45,
            ..palette.text
        }
    } else {
        palette.text
    };
    iced::widget::button::Style {
        background,
        text_color,
        border: Default::default(),
        shadow: Default::default(),
        snap: false,
    }
}
