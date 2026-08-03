//! 自定义菜单按钮：交互与 egui 内置 MenuButton/SubMenuButton 一致，但弹出层
//! 的 Area id 附带当前语言。
//!
//! 为什么不用 egui 内置按钮：内置菜单的弹出层 Area 缓存上一次打开的宽度，
//! 并按该宽度强制布局（justified）。语言切换后若缓存宽度小于新语言最长
//! 文本，长项会被挤成多行，且换行后的内容宽度不会超过缓存值，永远无法
//! 自动恢复（见 issue #69）。
//!
//! 修复：弹出层 id 绑定当前语言（`id.with(locale)`）。语言切换后 id 变化，
//! 旧宽度缓存自然失效，egui 走不可见的 sizing pass 按新文本重新测量；
//! 同一语言内 id 稳定，打开/关闭交互不受影响。

use eframe::egui::{
    self, Button, Frame, Popup, PopupCloseBehavior, RectAlign, Response, Ui, Vec2, WidgetText,
};
use egui::Widget as _;
use egui::containers::menu::{MenuConfig, MenuState, SubMenu};

/// 弹出层 id 附带当前语言：语言变化后强制重新测量菜单宽度。
fn localized_popup_id(base: egui::Id) -> egui::Id {
    base.with("locale").with(&*rust_i18n::locale())
}

/// 主菜单按钮：点击弹出菜单（宽度随语言自动重新评估）。
pub fn menu_button<R>(
    ui: &mut Ui,
    text: impl Into<WidgetText>,
    content: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<egui::InnerResponse<R>>) {
    let response = Button::new(text).ui(ui);
    let config = MenuConfig::find(ui);
    let inner = Popup::menu(&response)
        .id(localized_popup_id(response.id.with("popup")))
        .close_behavior(config.close_behavior)
        .style(config.style.clone())
        .show(content);
    (response, inner)
}

/// 子菜单按钮：hover/点击打开子菜单（宽度随语言自动重新评估）。
/// 实现与 egui `SubMenuButton::ui` / `SubMenu::show` 一致，仅弹出层 id 不同。
pub fn submenu_button<R>(
    ui: &mut Ui,
    text: impl Into<WidgetText>,
    content: impl FnOnce(&mut Ui) -> R,
) -> Response {
    let my_id = ui.next_auto_id();
    let open = MenuState::from_ui(ui, |state, _| {
        state.open_item == Some(SubMenu::id_from_widget_id(my_id))
    });
    let inactive = ui.style().visuals.widgets.inactive;
    if open {
        ui.style_mut().visuals.widgets.inactive = ui.style().visuals.widgets.open;
    }
    let response = Button::new(text).right_text("⏵").ui(ui);
    ui.style_mut().visuals.widgets.inactive = inactive;

    let frame = Frame::menu(ui.style());
    let id = SubMenu::id_from_widget_id(my_id);

    let (open_item, menu_id) = MenuState::from_ui(ui, |state, stack| (state.open_item, stack.id));
    let menu_config = MenuConfig::find(ui);

    let menu_root_response = ui.ctx().read_response(menu_id).unwrap();
    let hover_pos = ui.ctx().pointer_hover_pos();
    let menu_rect = menu_root_response.rect - frame.total_margin();
    let is_hovering_menu = hover_pos.is_some_and(|pos| {
        ui.ctx().layer_id_at(pos) == Some(menu_root_response.layer_id) && menu_rect.contains(pos)
    });

    let is_any_open = open_item.is_some();
    let mut is_open = open_item == Some(id);
    let mut set_open = None;
    let button_rect = response.rect.expand2(ui.style().spacing.item_spacing / 2.0);
    let is_hovered = hover_pos.is_some_and(|pos| button_rect.contains(pos));
    let should_open = ui.is_enabled() && (response.clicked() || (is_hovered && !is_any_open));
    if should_open {
        set_open = Some(true);
        is_open = true;
        MenuState::from_id(ui.ctx(), menu_id, |state| state.open_item = None);
    }

    let gap = frame.total_margin().sum().x / 2.0 + 2.0;
    let mut popup_response = response.clone();
    let expand = Vec2::new(0.0, frame.total_margin().sum().y / 2.0);
    popup_response.interact_rect = popup_response.interact_rect.expand2(expand);

    let popup_response = Popup::from_response(&popup_response)
        .id(localized_popup_id(id))
        .open(is_open)
        .align(RectAlign::RIGHT_START)
        .gap(gap)
        .style(menu_config.style.clone())
        .frame(frame)
        .close_behavior(PopupCloseBehavior::IgnoreClicks)
        .show(|ui| {
            if response.clicked() || response.is_pointer_button_down_on() {
                ui.ctx().move_to_top(ui.layer_id());
            }
            content(ui)
        });

    if let Some(popup_response) = &popup_response {
        // 若没有更深的子菜单打开，说明自己是最深一层，负责处理点击关闭。
        let is_deepest_submenu = MenuState::is_deepest_open_sub_menu(ui.ctx(), id);
        let clicked_outside = is_deepest_submenu
            && popup_response.response.clicked_elsewhere()
            && menu_root_response.clicked_elsewhere();
        let submenu_button_clicked = response.clicked();
        let clicked_inside = is_deepest_submenu
            && !submenu_button_clicked
            && response.ctx.input(|i| i.pointer.any_click())
            && hover_pos.is_some_and(|pos| popup_response.response.interact_rect.contains(pos));

        let click_close = match menu_config.close_behavior {
            PopupCloseBehavior::CloseOnClick => clicked_outside || clicked_inside,
            PopupCloseBehavior::CloseOnClickOutside => clicked_outside,
            PopupCloseBehavior::IgnoreClicks => false,
        };
        if click_close {
            set_open = Some(false);
            ui.close();
        }

        let is_moving_towards_rect = ui.input(|i| {
            i.pointer
                .is_moving_towards_rect(&popup_response.response.rect)
        });
        if is_moving_towards_rect {
            ui.ctx().request_repaint();
        }

        let hovering_other_menu_entry = is_open
            && !is_hovered
            && !popup_response.response.contains_pointer()
            && !is_moving_towards_rect
            && is_hovering_menu;
        if hovering_other_menu_entry {
            set_open = Some(false);
        }

        if popup_response.response.should_close() {
            ui.close();
        }
        if ui.will_parent_close() {
            ui.data_mut(|data| data.remove_by_type::<MenuState>());
        }
    }

    if let Some(set_open) = set_open {
        MenuState::from_id(ui.ctx(), menu_id, |state| {
            state.open_item = set_open.then_some(id);
        });
    }

    response
}
