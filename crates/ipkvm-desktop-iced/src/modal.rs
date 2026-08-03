//! 自绘模态 overlay（spike 2）：遮罩 + 事件拦截 + 三关闭路径。
//!
//! 复刻 egui 端的 settings / connection / save profile / about 模态。
//! spike 验证：打开后背景事件被拦截；Esc/关闭按钮/点遮罩三关闭路径有效。

use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse, overlay, renderer};
use iced::border::Border;
use iced::keyboard;
use iced::widget::{
    button, button::Status, column, container, mouse_area, space, stack, text, text_input,
};
use iced::{Color, Element, Event, Length, Rectangle, Shadow, Size, Vector};
use rust_i18n::t;

/// 四种模态（对应 egui 端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalKind {
    Settings,
    Connection,
    SaveProfile,
    LoadProfile,
    About,
}

/// 模态状态：当前打开的模态（或 None）+ save profile 的输入文本。
#[derive(Clone, Debug, Default)]
pub struct ModalState {
    pub open: Option<ModalKind>,
    pub save_name: String,
    /// 加载 profile 模态的候选名（app 打开前填充）。
    pub load_names: Vec<String>,
}

/// 模态产生的动作。
#[derive(Clone, Debug, PartialEq)]
pub enum ModalAction {
    /// 请求关闭（Esc / 关闭按钮 / 点遮罩 三条路径都发这个）。
    Close,
    /// save profile 名字输入变化。
    SaveNameChanged(String),
    /// 保存 profile。
    Save,
    /// 点击某个候选 profile 名。
    LoadPicked(String),
    /// 点击卡片空白区域：吞掉事件但不关闭（避免落到下层遮罩）。
    Noop,
}

impl ModalState {
    pub fn open(&mut self, kind: ModalKind) {
        self.open = Some(kind);
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// 渲染模态内容（若打开）。返回 None 表示无模态。
    /// overlay 拦截背景事件由调用方在 iced overlay 层处理；这里只产生内容。
    pub fn view(&self) -> Option<Element<'_, ModalAction>> {
        let kind = self.open?;
        let content = match kind {
            ModalKind::Settings => self.settings_content(),
            ModalKind::Connection => self.connection_content(),
            ModalKind::SaveProfile => self.save_profile_content(),
            ModalKind::LoadProfile => self.load_profile_content(),
            ModalKind::About => self.about_content(),
        };
        Some(modal_card(title_for(kind), content))
    }

    /// 模态 overlay：遮罩（点击关闭）+ 居中卡片（点击卡片不关闭）+ Esc 关闭。
    ///
    /// 层序（自下而上）：背景 → 全屏遮罩 catcher → 居中卡片。
    /// - 卡片外的鼠标按下：catcher 发布 `Close` 并捕获事件（背景收不到）。
    /// - 卡片内按下：卡片 mouse_area 发布 `Noop` 并捕获（不落到 catcher）。
    /// - Esc：`EscClose` 包装层发布 `Close` 并捕获。
    pub fn overlay(&self) -> Option<Element<'_, ModalAction>> {
        let content = self.view()?;
        Some(overlay(content))
    }

    fn settings_content(&self) -> Element<'_, ModalAction> {
        column![close_button()].into()
    }

    fn connection_content(&self) -> Element<'_, ModalAction> {
        column![close_button()].into()
    }

    fn save_profile_content(&self) -> Element<'_, ModalAction> {
        let input = text_input("name", &self.save_name).on_input(ModalAction::SaveNameChanged);
        column![
            input,
            button(text("Save")).on_press(ModalAction::Save),
            close_button(),
        ]
        .spacing(8)
        .into()
    }

    fn load_profile_content(&self) -> Element<'_, ModalAction> {
        let mut content = iced::widget::Column::new().spacing(8);
        if self.load_names.is_empty() {
            content = content.push(button(text(t!("profile.no_recent").to_string())));
        } else {
            for name in &self.load_names {
                content = content.push(
                    button(text(name.clone())).on_press(ModalAction::LoadPicked(name.clone())),
                );
            }
        }
        content.push(close_button()).into()
    }

    fn about_content(&self) -> Element<'_, ModalAction> {
        column![text("my_ipkvm iced spike"), close_button()]
            .spacing(8)
            .into()
    }
}

fn close_button<'a>() -> iced::widget::Button<'a, ModalAction> {
    button(text(t!("modal.close").to_string())).on_press(ModalAction::Close)
}

fn title_for(kind: ModalKind) -> String {
    match kind {
        ModalKind::Settings => t!("modal.settings_title").to_string(),
        ModalKind::Connection => t!("modal.connection_title").to_string(),
        ModalKind::SaveProfile => t!("modal.save_title").to_string(),
        ModalKind::LoadProfile => t!("modal.load_title").to_string(),
        ModalKind::About => t!("modal.about_title").to_string(),
    }
}

/// 模态 overlay 组合（供 app 与测试复用）：遮罩 + 卡片 + Esc 关闭。
pub fn overlay<'a>(content: Element<'a, ModalAction>) -> Element<'a, ModalAction> {
    let dim = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()),
            ..Default::default()
        });
    let catcher = mouse_area(dim).on_press(ModalAction::Close);
    // 卡片用透明 Button 包一层：命中区域=卡片本身，点击卡片空白吞掉（Noop）
    // 并捕获事件；点击卡片外则落到下层遮罩（Close）。stack 不居中子元素，
    // 所以外层用 Fill 容器把卡片居中。
    let card: iced::widget::Button<'_, ModalAction, iced::Theme, iced::Renderer> = button(content)
        .on_press(ModalAction::Noop)
        .style(transparent_button)
        .padding(0);
    let centered = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill);
    EscClose::new(stack![catcher, centered].into(), ModalAction::Close).into()
}

fn transparent_button(theme: &iced::Theme, status: Status) -> button::Style {
    let mut style = button::text(theme, status);
    style.background = None;
    style
}

/// 模态卡片：标题 + 内容，居中白色背景。
fn modal_card<'a>(title: String, content: Element<'a, ModalAction>) -> Element<'a, ModalAction> {
    let card = column![text(title).size(20), content]
        .spacing(12)
        .padding(20);
    container(card)
        .style(|theme| container::Style {
            background: Some(theme.palette().background.into()),
            border: Border::default()
                .rounded(10)
                .width(1.0)
                .color(crate::theme::border_color(theme.palette())),
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        })
        .into()
}

/// 包装层：把 Esc 按键转成消息并捕获（模态打开时背景收不到键盘事件）。
struct EscClose<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_escape: Message,
}

impl<'a, Message, Theme, Renderer> EscClose<'a, Message, Theme, Renderer> {
    fn new(content: Element<'a, Message, Theme, Renderer>, on_escape: Message) -> Self {
        Self { content, on_escape }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for EscClose<'_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<()>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        if shell.is_event_captured() {
            return;
        }

        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) = event
        {
            shell.publish(self.on_escape.clone());
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: layout::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<EscClose<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(widget: EscClose<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}
