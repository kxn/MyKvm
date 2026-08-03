//! 自绘菜单（spike 2）。
//!
//! iced_aw 0.14.1 的嵌套子菜单存在真实树状态 bug（`Item::children()` 把菜单子树
//! 注释掉，打开嵌套子菜单后 operate/update 会 `tree.children[1]` 越界 panic；
//! 修复只在 0.15-dev 分支、未发版），故本 spike 改用自绘：
//! - 顶层 `MenuBar` 是一个普通 Widget，点击根按钮打开菜单；
//! - 打开的菜单/子菜单用 `Overlay` 实现（绝对定位、可嵌套）；
//! - hover 走廊（父项 → 子菜单之间的空隙）由子菜单自身持父项矩形计算连通区域，
//!   光标离开连通区域才关闭——不依赖 iced_aw 的私有逻辑。
//!
//! 状态机（open_root/open_path）在 app 侧持有，widget 是纯展示 + 事件转发；
//! 这样 headless 测试可以逐条驱动：点击 → 收集消息 → 更新状态 → 重建 view。

use iced::advanced::layout;
use iced::advanced::overlay::{self, Overlay};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Layout, Shell, mouse, renderer};
use iced::keyboard;
use iced::widget::text;
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};
use rust_i18n::t;

use crate::modal::ModalKind;

type MenuElement<'a> = Element<'a, MenuAction, iced::Theme, iced::Renderer>;

/// 菜单动作：既有业务动作，也有自绘菜单的内部状态消息（Open* / Close*）。
#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    /// 打开（或切换）顶层菜单。
    OpenRoot(usize),
    /// 打开指定路径的子菜单（路径为各层菜单内的 item 下标）。
    OpenSubmenu(Vec<usize>),
    /// 关闭最深一层子菜单。
    CloseSubmenus,
    /// 关闭全部菜单。
    CloseMenus,
    /// 打开指定模态。
    OpenModal(ModalKind),
    /// 语言切换。
    SetLanguage(LanguageChoice),
    /// 最近使用 profile（spike 用固定列表模拟）。
    LoadRecent(String),
    /// 特殊键发送（spike 不接真实链路，仅记录）。
    SpecialKey(String),
    /// 其它简单动作（退出/复制截图等，spike 不实现真实逻辑）。
    Simple(&'static str),
}

/// 语言菜单选项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LanguageChoice {
    System,
    Chinese,
    English,
}

/// 菜单状态（app 侧持有）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuState {
    pub open_root: Option<usize>,
    /// 各层菜单内已展开子菜单的 item 下标路径（空 = 只开顶层菜单）。
    pub open_path: Vec<usize>,
}

impl MenuState {
    /// 应用一条菜单消息；业务动作（非 Open*/Close*）原样返回由调用方处理。
    pub fn apply(&mut self, action: MenuAction) -> Option<MenuAction> {
        match action {
            MenuAction::OpenRoot(i) => {
                if self.open_root == Some(i) {
                    self.open_root = None;
                } else {
                    self.open_root = Some(i);
                }
                self.open_path.clear();
                None
            }
            MenuAction::OpenSubmenu(path) => {
                if self.open_root.is_some() {
                    self.open_path = path;
                }
                None
            }
            MenuAction::CloseSubmenus => {
                self.open_path.pop();
                None
            }
            MenuAction::CloseMenus => {
                self.open_root = None;
                self.open_path.clear();
                None
            }
            other => {
                self.open_root = None;
                self.open_path.clear();
                Some(other)
            }
        }
    }
}

/// 菜单项数据。
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub action: Option<MenuAction>,
    pub submenu: Vec<MenuItem>,
}

impl MenuItem {
    pub fn action(label: impl Into<String>, action: MenuAction) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
            submenu: Vec::new(),
        }
    }

    pub fn with_submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            action: None,
            submenu: items,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: "──────────────".into(),
            action: None,
            submenu: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MenuRoot {
    pub label: String,
    pub items: Vec<MenuItem>,
}

/// 构建完整菜单栏 Element（顶层为一行根按钮，弹出菜单走 overlay）。
pub fn menu_bar<'a>(state: &MenuState, recent_profiles: &[&str]) -> MenuElement<'a> {
    let roots = vec![
        file_menu(recent_profiles),
        edit_menu(),
        send_menu(),
        about_menu(),
    ];
    let labels = roots.iter().map(|r| label_element(&r.label)).collect();
    MenuBar {
        roots,
        open_root: state.open_root,
        open_path: state.open_path.clone(),
        labels,
    }
    .into()
}

// ---------------------------------------------------------------------------
// 菜单数据（复刻 egui 端结构）
// ---------------------------------------------------------------------------

fn file_menu(recent_profiles: &[&str]) -> MenuRoot {
    let items = vec![
        MenuItem::action(t!("menu.reselect_device"), MenuAction::Simple("reselect")),
        MenuItem::action(t!("menu.stop_connection"), MenuAction::Simple("stop")),
        MenuItem::separator(),
        MenuItem::action(t!("file.load_profile"), MenuAction::Simple("load_profile")),
        recent_menu(recent_profiles),
        MenuItem::separator(),
        MenuItem::action(t!("file.exit"), MenuAction::Simple("exit")),
    ];
    MenuRoot {
        label: t!("menu.file").to_string(),
        items,
    }
}

fn recent_menu(recent_profiles: &[&str]) -> MenuItem {
    let mut items: Vec<MenuItem> = Vec::new();
    if recent_profiles.is_empty() {
        items.push(MenuItem {
            label: "(none)".into(),
            action: None,
            submenu: Vec::new(),
        });
    } else {
        for name in recent_profiles.iter().take(3) {
            items.push(MenuItem::action(
                (*name).to_string(),
                MenuAction::LoadRecent((*name).to_string()),
            ));
        }
        if recent_profiles.len() > 3 {
            let more_items: Vec<MenuItem> = recent_profiles
                .iter()
                .skip(3)
                .map(|name| {
                    MenuItem::action(
                        (*name).to_string(),
                        MenuAction::LoadRecent((*name).to_string()),
                    )
                })
                .collect();
            items.push(MenuItem::with_submenu(t!("file.recent_more"), more_items));
        }
    }
    MenuItem::with_submenu(t!("file.recent"), items)
}

fn edit_menu() -> MenuRoot {
    let items = vec![
        MenuItem::action(
            t!("edit.copy_screenshot"),
            MenuAction::Simple("copy_screenshot"),
        ),
        MenuItem::separator(),
        language_menu(),
        MenuItem::action(
            t!("edit.settings"),
            MenuAction::OpenModal(ModalKind::Settings),
        ),
    ];
    MenuRoot {
        label: t!("menu.edit").to_string(),
        items,
    }
}

fn language_menu() -> MenuItem {
    let items = vec![
        MenuItem::action(
            t!("language.system"),
            MenuAction::SetLanguage(LanguageChoice::System),
        ),
        MenuItem::action(
            t!("language.chinese"),
            MenuAction::SetLanguage(LanguageChoice::Chinese),
        ),
        MenuItem::action(
            t!("language.english"),
            MenuAction::SetLanguage(LanguageChoice::English),
        ),
    ];
    MenuItem::with_submenu(t!("edit.language"), items)
}

fn send_menu() -> MenuRoot {
    let items = vec![
        MenuItem::action(t!("send.paste_text"), MenuAction::Simple("paste")),
        MenuItem::action(t!("send.release_all"), MenuAction::Simple("release_all")),
        MenuItem::separator(),
        special_keys_menu(),
    ];
    MenuRoot {
        label: t!("menu.send").to_string(),
        items,
    }
}

fn special_keys_menu() -> MenuItem {
    let items = vec![
        MenuItem::action(
            t!("special_keys.ctrl_alt_del"),
            MenuAction::SpecialKey("CtrlAltDel".into()),
        ),
        MenuItem::action(t!("special_keys.win"), MenuAction::SpecialKey("Win".into())),
        MenuItem::action(
            t!("special_keys.print_screen"),
            MenuAction::SpecialKey("PrintScreen".into()),
        ),
        MenuItem::action(
            t!("special_keys.alt_tab"),
            MenuAction::SpecialKey("AltTab".into()),
        ),
    ];
    MenuItem::with_submenu(t!("send.special_keys"), items)
}

fn about_menu() -> MenuRoot {
    let items = vec![
        MenuItem::action(
            t!("modal.about_title"),
            MenuAction::OpenModal(ModalKind::About),
        ),
        MenuItem::action(t!("about.project_home"), MenuAction::Simple("project_home")),
    ];
    MenuRoot {
        label: t!("menu.about").to_string(),
        items,
    }
}

// ---------------------------------------------------------------------------
// 顶层菜单栏 Widget
// ---------------------------------------------------------------------------

const ITEM_SPACING: f32 = 4.0;
const SUBMENU_GAP: f32 = 2.0;

struct MenuBar<'a> {
    roots: Vec<MenuRoot>,
    open_root: Option<usize>,
    open_path: Vec<usize>,
    labels: Vec<MenuElement<'a>>,
}

impl<'a> From<MenuBar<'a>> for MenuElement<'a> {
    fn from(bar: MenuBar<'a>) -> Self {
        Element::new(bar)
    }
}

impl MenuBar<'_> {
    fn root_label_rect(&self, layout: Layout<'_>, index: usize) -> Rectangle {
        layout
            .children()
            .nth(index)
            .map(|node| node.bounds())
            .unwrap_or_default()
    }

    fn root_hit(&self, pos: Point, layout: Layout<'_>) -> Option<usize> {
        self.roots
            .iter()
            .enumerate()
            .find(|(i, _)| self.root_label_rect(layout, *i).contains(pos))
            .map(|(i, _)| i)
    }
}

impl Widget<MenuAction, iced::Theme, iced::Renderer> for MenuBar<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<()>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(())
    }

    fn children(&self) -> Vec<Tree> {
        self.labels.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.labels);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut x: f32 = 0.0;
        let mut height: f32 = 0.0;
        let mut nodes = Vec::with_capacity(self.labels.len());
        for (i, label) in self.labels.iter_mut().enumerate() {
            let mut node = label.as_widget_mut().layout(
                &mut tree.children[i],
                renderer,
                &layout::Limits::NONE,
            );
            node.move_to_mut(Point::new(x, 0.0));
            height = height.max(node.size().height);
            x += node.size().width + 12.0;
            nodes.push(node);
        }
        layout::Node::with_children(Size::new(limits.max().width, height), nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for (i, (label, child)) in self.labels.iter_mut().zip(layout.children()).enumerate() {
                label
                    .as_widget_mut()
                    .operate(&mut tree.children[i], child, renderer, operation);
            }
        });
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, MenuAction>,
        _viewport: &Rectangle,
    ) {
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) = event
        {
            if self.open_root.is_some() {
                shell.publish(MenuAction::CloseMenus);
                shell.capture_event();
            }
            return;
        }

        let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event else {
            return;
        };
        let Some(pos) = cursor.position() else {
            return;
        };

        if self.open_root.is_some() {
            // 菜单已打开：点根按钮切换，点其它任何地方关闭（不穿透到下层）。
            if let Some(i) = self.root_hit(pos, layout) {
                shell.publish(MenuAction::OpenRoot(i));
            } else {
                shell.publish(MenuAction::CloseMenus);
            }
            shell.capture_event();
            return;
        }

        if let Some(i) = self.root_hit(pos, layout) {
            shell.publish(MenuAction::OpenRoot(i));
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        // 只有悬停在某个根按钮文字上才声明 Pointer；整行声明会把下层
        // 同高度控件的光标置为 Unavailable，导致其收不到点击。
        let over_root = cursor.position().is_some_and(|pos| {
            self.roots
                .iter()
                .enumerate()
                .any(|(i, _)| self.root_label_rect(layout, i).contains(pos))
        });
        if over_root {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for (i, (label, child)) in self.labels.iter().zip(layout.children()).enumerate() {
            label.as_widget().draw(
                &tree.children[i],
                renderer,
                theme,
                style,
                child,
                cursor,
                viewport,
            );
        }
    }

    fn overlay<'b>(
        &'b mut self,
        _tree: &'b mut Tree,
        layout: Layout<'b>,
        _renderer: &iced::Renderer,
        _viewport: &Rectangle,
        _translation: Vector,
    ) -> Option<overlay::Element<'b, MenuAction, iced::Theme, iced::Renderer>> {
        let index = self.open_root?;
        let root = self.roots.get(index)?;
        let anchor = self.root_label_rect(layout, index);
        let popup = MenuPopup {
            items: root.items.clone(),
            widgets: root.items.iter().map(|i| label_element(&i.label)).collect(),
            tree: popup_tree(&root.items),
            prefix: Vec::new(),
            open_path: self.open_path.clone(),
            parent_rect: Some(anchor),
            parent_bounds: None,
            is_submenu: false,
        };
        Some(overlay::Element::new(Box::new(popup)))
    }
}

// ---------------------------------------------------------------------------
// 菜单弹出层 Overlay（可嵌套）
// ---------------------------------------------------------------------------

struct MenuPopup<'a> {
    items: Vec<MenuItem>,
    widgets: Vec<MenuElement<'a>>,
    /// 子控件树（overlay 没有框架传入的 tree，这里自持）。
    tree: Tree,
    /// 本弹出层在根菜单中的 item 路径前缀（发布 OpenSubmenu 时用绝对路径）。
    prefix: Vec<usize>,
    /// 相对本弹出层的已展开子菜单路径（首元素 = 本层哪个 item 展开了子菜单）。
    open_path: Vec<usize>,
    /// 触发本弹出层的锚点矩形：顶层菜单 = 根按钮矩形；子菜单 = 父项矩形。
    parent_rect: Option<Rectangle>,
    /// 父菜单整体 bounds（走廊关闭边界；顶层菜单为 None）。
    parent_bounds: Option<Rectangle>,
    /// true = 本层是子菜单（锚点放在父项右侧）；false = 顶层菜单（锚点放在根按钮下方）。
    is_submenu: bool,
}

impl Overlay<MenuAction, iced::Theme, iced::Renderer> for MenuPopup<'_> {
    fn layout(&mut self, renderer: &iced::Renderer, _bounds: Size) -> layout::Node {
        let mut y: f32 = 0.0;
        let mut width: f32 = 0.0;
        let mut nodes = Vec::with_capacity(self.widgets.len());
        let anchor = self.anchor();
        for (i, widget) in self.widgets.iter_mut().enumerate() {
            let mut node = widget.as_widget_mut().layout(
                &mut self.tree.children[i],
                renderer,
                &layout::Limits::NONE,
            );
            node.move_to_mut(Point::new(anchor.x, anchor.y + y));
            width = width.max(node.size().width);
            y += node.size().height + ITEM_SPACING;
            nodes.push(node);
        }
        layout::Node::with_children(Size::new(width, y - ITEM_SPACING), nodes).move_to(anchor)
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for (i, (widget, child)) in self.widgets.iter_mut().zip(layout.children()).enumerate() {
                widget.as_widget_mut().operate(
                    &mut self.tree.children[i],
                    child,
                    renderer,
                    operation,
                );
            }
        });
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, MenuAction>,
    ) {
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) = event
        {
            shell.publish(MenuAction::CloseMenus);
            shell.capture_event();
            return;
        }

        let Some(pos) = cursor.position() else {
            return;
        };
        let item_rects: Vec<Rectangle> = layout.children().map(|child| child.bounds()).collect();

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(i) = hit_rects(pos, &item_rects) {
                    self.activate_item(i, shell);
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // 子菜单走廊：光标离开“父菜单 ∪ 本层 ∪ 走廊”才关闭，穿越缝隙不关。
                if let (Some(parent), Some(parent_bounds)) = (self.parent_rect, self.parent_bounds)
                {
                    let union = corridor_union(parent, layout.bounds(), parent_bounds);
                    if !union.contains(pos) {
                        shell.publish(MenuAction::CloseSubmenus);
                        shell.capture_event();
                        return;
                    }
                }
                if let Some(i) = hit_rects(pos, &item_rects) {
                    self.hover_item(i, shell);
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let viewport = layout.bounds();
        for (i, (widget, child)) in self.widgets.iter().zip(layout.children()).enumerate() {
            widget.as_widget().draw(
                &self.tree.children[i],
                renderer,
                theme,
                style,
                child,
                cursor,
                &viewport,
            );
        }
    }

    fn overlay<'b>(
        &'b mut self,
        layout: Layout<'b>,
        _renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'b, MenuAction, iced::Theme, iced::Renderer>> {
        let i = *self.open_path.first()?;
        let item = self.items.get(i)?;
        if item.submenu.is_empty() {
            return None;
        }
        let parent_rect = layout.children().nth(i)?.bounds();
        let mut prefix = self.prefix.clone();
        prefix.push(i);
        let child = MenuPopup {
            items: item.submenu.clone(),
            widgets: item
                .submenu
                .iter()
                .map(|child| label_element(&child.label))
                .collect(),
            tree: popup_tree(&item.submenu),
            prefix,
            open_path: self.open_path[1..].to_vec(),
            parent_rect: Some(parent_rect),
            parent_bounds: Some(layout.bounds()),
            is_submenu: true,
        };
        Some(overlay::Element::new(Box::new(child)))
    }
}

impl MenuPopup<'_> {
    fn anchor(&self) -> Point {
        match (self.parent_rect, self.is_submenu) {
            (Some(rect), true) => Point::new(rect.x + rect.width + SUBMENU_GAP, rect.y),
            (Some(rect), false) => Point::new(rect.x, rect.y + rect.height),
            (None, _) => Point::ORIGIN,
        }
    }

    fn activate_item(&mut self, i: usize, shell: &mut Shell<'_, MenuAction>) {
        let Some(item) = self.items.get(i) else {
            return;
        };
        if !item.submenu.is_empty() {
            let mut path = self.prefix.clone();
            path.push(i);
            if self.open_path.first() == Some(&i) {
                shell.publish(MenuAction::CloseSubmenus);
            } else {
                shell.publish(MenuAction::OpenSubmenu(path));
            }
            return;
        }
        if let Some(action) = item.action.clone() {
            shell.publish(action);
            shell.publish(MenuAction::CloseMenus);
        }
    }

    fn hover_item(&mut self, i: usize, shell: &mut Shell<'_, MenuAction>) {
        let Some(item) = self.items.get(i) else {
            return;
        };
        if self.open_path.first() == Some(&i) {
            return;
        }
        if !item.submenu.is_empty() {
            let mut path = self.prefix.clone();
            path.push(i);
            shell.publish(MenuAction::OpenSubmenu(path));
        } else if !self.open_path.is_empty() {
            shell.publish(MenuAction::CloseSubmenus);
        }
    }
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

fn label_element<'a>(label: &str) -> MenuElement<'a> {
    text(label.to_string()).into()
}

/// 为弹出层构造子控件树：每个 item 一个 text 子树。
fn popup_tree(items: &[MenuItem]) -> Tree {
    Tree {
        tag: tree::Tag::stateless(),
        state: tree::State::None,
        children: items
            .iter()
            .map(|item| Tree::new(label_element(&item.label)))
            .collect(),
    }
}

fn hit_rects(pos: Point, rects: &[Rectangle]) -> Option<usize> {
    rects.iter().position(|rect| rect.contains(pos))
}

/// 走廊连通区域：父菜单 bounds ∪ 子菜单矩形 ∪ 二者之间的水平走廊。
/// 子菜单固定向右展开（本 spike 约定）。
fn corridor_union(parent: Rectangle, child: Rectangle, parent_bounds: Rectangle) -> Rectangle {
    let top = parent.y.min(child.y);
    let bottom = (parent.y + parent.height).max(child.y + child.height);
    let corridor = Rectangle::new(
        Point::new(parent.x + parent.width, top),
        Size::new((child.x - (parent.x + parent.width)).max(0.0), bottom - top),
    );
    parent_bounds.union(&parent).union(&child).union(&corridor)
}
