//! 自绘模态 overlay（spike 2）：遮罩 + 事件拦截 + 三关闭路径。
//!
//! 复刻 egui 端的 settings / connection / save profile / about 模态。
//! spike 验证：打开后背景事件被拦截；Esc/关闭按钮/点遮罩三关闭路径有效。

use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse, overlay, renderer};
use iced::border::Border;
use iced::keyboard;
use iced::widget::PickList;
use iced::widget::{button, button::Status, column, container, mouse_area, space, stack, text};
use iced::{Color, Element, Event, Length, Rectangle, Shadow, Size, Vector};
use ipkvm_core::MouseMode;
use rust_i18n::t;

/// 四种模态（对应 egui 端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalKind {
    Settings,
    Connection,
    SaveProfile,
    About,
}

/// 模态状态：当前打开的模态（或 None）+ save profile 的输入文本 + 设置同步值。
#[derive(Clone, Debug)]
pub struct ModalState {
    pub open: Option<ModalKind>,
    pub save_name: String,
    /// 覆盖保存确认（同名时第一次 Save 进入确认态）。
    pub confirm_overwrite: bool,
    /// 连接设置模态显示用的连接参数（app 打开前同步）。
    pub baud_rate: u32,
    pub preview_fps: u64,
    pub auto_baud: bool,
    pub mouse_mode: MouseMode,
    pub relative_sensitivity: f32,
    /// 数字输入编辑缓冲（TextInput 受控文本，改动实时解析并 clamp）。
    pub baud_text: String,
    pub fps_text: String,
    pub sensitivity_text: String,
    /// 设置模态显示用的缩放模式（app 打开前同步）。
    pub scale_mode: crate::scale::ScaleMode,
}

impl Default for ModalState {
    fn default() -> Self {
        Self {
            open: None,
            save_name: String::new(),
            confirm_overwrite: false,
            baud_rate: ipkvm_core::DEFAULT_BAUD_RATE,
            preview_fps: 30,
            auto_baud: true,
            mouse_mode: MouseMode::Absolute,
            relative_sensitivity: 1.0,
            baud_text: ipkvm_core::DEFAULT_BAUD_RATE.to_string(),
            fps_text: "30".to_string(),
            sensitivity_text: "1".to_string(),
            scale_mode: crate::scale::ScaleMode::FitWindow,
        }
    }
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
    /// 覆盖确认：取消覆盖。
    CancelOverwrite,
    /// 恢复默认连接参数（连接设置模态）。
    RestoreDefaults,
    /// 连接设置：波特率。
    SetBaudRate(u32),
    /// 连接设置：预览帧率。
    SetPreviewFps(u64),
    /// 连接设置：自动波特率。
    SetAutoBaud(bool),
    /// 连接设置：鼠标模式。
    SetMouseMode(MouseMode),
    /// 连接设置：相对灵敏度。
    SetRelativeSensitivity(f32),
    /// 数字输入缓冲变化（波特率，解析成功后同时路由到目标连接参数）。
    BaudRateTextChanged(String),
    /// 数字输入缓冲变化（预览帧率）。
    PreviewFpsTextChanged(String),
    /// 数字输入缓冲变化（相对灵敏度）。
    RelativeSensitivityTextChanged(String),
    /// 设置模态：缩放模式。
    SetScaleMode(crate::scale::ScaleMode),
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
        use iced::widget::{PickList, column};

        let scale_labels = vec![
            t!("scale_mode.fit_window").to_string(),
            t!("scale_mode.actual_size").to_string(),
            t!("scale_mode.resize_to_video").to_string(),
        ];
        let selected_scale = match self.scale_mode {
            crate::scale::ScaleMode::FitWindow => t!("scale_mode.fit_window").to_string(),
            crate::scale::ScaleMode::ActualSize => t!("scale_mode.actual_size").to_string(),
            crate::scale::ScaleMode::ResizeWindowToVideo => {
                t!("scale_mode.resize_to_video").to_string()
            }
        };
        let scale = PickList::new(scale_labels, Some(selected_scale), |label: String| {
            ModalAction::SetScaleMode(if label == t!("scale_mode.fit_window") {
                crate::scale::ScaleMode::FitWindow
            } else if label == t!("scale_mode.actual_size") {
                crate::scale::ScaleMode::ActualSize
            } else {
                crate::scale::ScaleMode::ResizeWindowToVideo
            })
        });

        column![
            self.connection_fields(),
            label(t!("settings.scale_mode")),
            scale,
            close_button(),
        ]
        .spacing(8)
        .into()
    }

    fn connection_content(&self) -> Element<'_, ModalAction> {
        use iced::widget::{button, column};
        column![
            self.connection_fields(),
            button(label(t!("profile.restore_defaults"))).on_press(ModalAction::RestoreDefaults),
            close_button(),
        ]
        .spacing(8)
        .into()
    }

    fn save_profile_content(&self) -> Element<'_, ModalAction> {
        use iced::widget::{button, text_input};
        let input = text_input("name", &self.save_name).on_input(ModalAction::SaveNameChanged);
        let name_ok = !self.save_name.trim().is_empty();
        let save_button = if name_ok {
            button(label(t!("modal.save"))).on_press(ModalAction::Save)
        } else {
            button(label(t!("modal.save")))
        };
        let mut content = iced::widget::Column::new().spacing(8);
        content = content.push(input);
        if self.confirm_overwrite {
            content = content.push(label(t!(
                "profile.overwrite_body",
                name = self.save_name.trim()
            )));
            content = content
                .push(button(label(t!("profile.overwrite_confirm"))).on_press(ModalAction::Save));
            content = content
                .push(button(label(t!("common.cancel"))).on_press(ModalAction::CancelOverwrite));
        } else {
            content = content.push(save_button);
            content = content.push(close_button());
        }
        content.into()
    }

    fn about_content(&self) -> Element<'_, ModalAction> {
        use iced::widget::column;
        column![
            label(t!("about.title")),
            label(t!("about.version", commit = env!("GIT_COMMIT"))),
            label(t!("about.license")),
            label(t!("about.project_url", url = crate::app::PROJECT_URL)),
            close_button(),
        ]
        .spacing(8)
        .into()
    }

    /// 连接参数表单（设置默认值对话框与连接设置对话框共用，复刻 egui
    /// `connection_fields_ui`）：波特率/预览 FPS/鼠标模式/相对灵敏度/自动波特率。
    ///
    /// iced_aw 的 number_input feature 在本仓库 vendored 版本上无法编译
    /// （icon 字体 proc macro 读不到 font.ttf），退化为 TextInput + parse/clamp。
    fn connection_fields(&self) -> iced::widget::Column<'_, ModalAction> {
        use iced::widget::{Checkbox, column, text_input};

        let baud =
            text_input("1200..115200", &self.baud_text).on_input(ModalAction::BaudRateTextChanged);
        let fps = text_input("1..60", &self.fps_text).on_input(ModalAction::PreviewFpsTextChanged);
        let sensitivity = text_input("0.1..5.0", &self.sensitivity_text)
            .on_input(ModalAction::RelativeSensitivityTextChanged);
        let auto_baud = Checkbox::new(self.auto_baud)
            .label(t!("settings.auto_baud"))
            .on_toggle(ModalAction::SetAutoBaud);

        let mouse_labels = vec![
            t!("mouse_mode.absolute").to_string(),
            t!("mouse_mode.relative").to_string(),
        ];
        let selected_mouse = if self.mouse_mode == MouseMode::Absolute {
            t!("mouse_mode.absolute").to_string()
        } else {
            t!("mouse_mode.relative").to_string()
        };
        let mouse = PickList::new(mouse_labels, Some(selected_mouse), |label: String| {
            ModalAction::SetMouseMode(if label == t!("mouse_mode.absolute") {
                MouseMode::Absolute
            } else {
                MouseMode::Relative
            })
        });

        column![
            label(t!("settings.baud_rate")),
            baud,
            auto_baud,
            label(t!("settings.preview_fps")),
            fps,
            label(t!("settings.mouse_mode")),
            mouse,
            label(t!("settings.relative_sensitivity")),
            sensitivity,
        ]
        .spacing(8)
    }
}

fn close_button<'a>() -> iced::widget::Button<'a, ModalAction> {
    button(label(t!("modal.close").to_string())).on_press(ModalAction::Close)
}

fn title_for(kind: ModalKind) -> String {
    match kind {
        ModalKind::Settings => t!("modal.settings_title").to_string(),
        ModalKind::Connection => t!("modal.connection_title").to_string(),
        ModalKind::SaveProfile => t!("modal.save_title").to_string(),
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
    let card = column![label(title).size(20), content]
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

/// 模态内文本：按当前语言选择字体（语言切换后重绘时生效）。
fn label<'a>(s: impl Into<String>) -> iced::widget::Text<'a> {
    text(s.into()).font(crate::fonts::ui_font())
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
