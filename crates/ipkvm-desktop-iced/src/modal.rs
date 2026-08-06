//! 自绘模态 overlay：遮罩 + 事件拦截 + 三关闭路径。
//!
//! 设置、连接、保存 profile 和关于模态。
//! 打开后背景事件被拦截；Esc/关闭按钮/点遮罩三关闭路径有效。

use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse, overlay, renderer};
use iced::border::Border;
use iced::keyboard;
use iced::widget::PickList;
use iced::widget::{
    button, button::Status, column, container, mouse_area, row, space, stack, text,
};
use iced::{Color, Element, Event, Length, Rectangle, Shadow, Size, Vector};
use ipkvm_core::{MouseMode, MouseProfile};
use rust_i18n::t;

/// 四种应用模态。
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
    pub mouse_profile: MouseProfile,
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
            mouse_profile: MouseProfile::RawAbsolute,
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
    /// 目标端鼠标兼容 profile。
    SetMouseProfile(MouseProfile),
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
        let (content, footer) = match kind {
            ModalKind::Settings => self.settings_content(),
            ModalKind::Connection => self.connection_content(),
            ModalKind::SaveProfile => self.save_profile_content(),
            ModalKind::About => self.about_content(),
        };
        Some(modal_card(title_for(kind), content, footer))
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

    fn settings_content(&self) -> (Element<'_, ModalAction>, Element<'_, ModalAction>) {
        use iced::widget::PickList;

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
        })
        .width(Length::Fill)
        .style(crate::theme::pick_list_style);

        let body = column![
            section_title(t!("modal.section_connection")),
            self.connection_fields(),
            section_title(t!("modal.section_display")),
            field_row(t!("settings.scale_mode"), scale),
        ]
        .spacing(12);
        (body.into(), footer_row(vec![close_button().into()]))
    }

    fn connection_content(&self) -> (Element<'_, ModalAction>, Element<'_, ModalAction>) {
        use iced::widget::{button, column};
        let body = column![
            section_title(t!("modal.section_connection")),
            self.connection_fields(),
        ]
        .spacing(12);
        let footer = footer_row(vec![
            button(button_text(t!("profile.restore_defaults")))
                .on_press(ModalAction::RestoreDefaults)
                .style(crate::theme::secondary_button)
                .into(),
            close_button().into(),
        ]);
        (body.into(), footer)
    }

    fn save_profile_content(&self) -> (Element<'_, ModalAction>, Element<'_, ModalAction>) {
        use iced::widget::{button, text_input};
        let input = text_input(t!("modal.name_placeholder").as_ref(), &self.save_name)
            .on_input(ModalAction::SaveNameChanged)
            .width(Length::Fill)
            .style(crate::theme::text_input_style);
        let name_ok = !self.save_name.trim().is_empty();

        let body = if self.confirm_overwrite {
            column![
                field_row(t!("modal.name_label"), input),
                overwrite_warning(t!("profile.overwrite_body", name = self.save_name.trim())),
            ]
            .spacing(12)
        } else {
            column![field_row(t!("modal.name_label"), input)].spacing(12)
        };

        let footer = if self.confirm_overwrite {
            footer_row(vec![
                // 覆盖确认态的取消回到正常态（不改名不关闭），与旧行为一致。
                button(button_text(t!("common.cancel")))
                    .on_press(ModalAction::CancelOverwrite)
                    .style(crate::theme::secondary_button)
                    .into(),
                button(button_text(t!("profile.overwrite_confirm")))
                    .on_press(ModalAction::Save)
                    .style(crate::theme::danger_button)
                    .into(),
            ])
        } else {
            footer_row(vec![
                cancel_button().into(),
                if name_ok {
                    button(button_text(t!("modal.save")))
                        .on_press(ModalAction::Save)
                        .style(crate::theme::primary_button)
                } else {
                    button(button_text(t!("modal.save"))).style(crate::theme::primary_button)
                }
                .into(),
            ])
        };
        (body.into(), footer)
    }

    fn about_content(&self) -> (Element<'_, ModalAction>, Element<'_, ModalAction>) {
        use iced::widget::column;
        let body = column![
            text(t!("about.title"))
                .size(16)
                .font(crate::fonts::ui_font()),
            info_line(t!("about.version", commit = env!("GIT_COMMIT"))),
            info_line(t!("about.license")),
            info_line(t!("about.project_url", url = crate::app::PROJECT_URL)),
        ]
        .spacing(8);
        (body.into(), footer_row(vec![close_button().into()]))
    }

    /// 连接参数表单（设置默认值对话框与连接设置对话框共用，
    /// `connection_fields_ui`）：波特率/预览 FPS/鼠标模式/相对灵敏度/自动波特率。
    ///
    /// iced_aw 的 number_input feature 在本仓库 vendored 版本上无法编译
    /// （icon 字体 proc macro 读不到 font.ttf），退化为 TextInput + parse/clamp。
    fn connection_fields(&self) -> iced::widget::Column<'_, ModalAction> {
        use iced::widget::{Checkbox, column, text_input};

        let baud = text_input("1200..115200", &self.baud_text)
            .on_input(ModalAction::BaudRateTextChanged)
            .width(Length::Fill)
            .style(crate::theme::text_input_style);
        let fps = text_input("1..60", &self.fps_text)
            .on_input(ModalAction::PreviewFpsTextChanged)
            .width(Length::Fill)
            .style(crate::theme::text_input_style);
        let sensitivity = text_input("0.1..5.0", &self.sensitivity_text)
            .on_input(ModalAction::RelativeSensitivityTextChanged)
            .width(Length::Fill)
            .style(crate::theme::text_input_style);
        let auto_baud = Checkbox::new(self.auto_baud)
            .label(t!("settings.auto_baud"))
            .on_toggle(ModalAction::SetAutoBaud)
            .text_size(13.0);

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
        })
        .width(Length::Fill)
        .style(crate::theme::pick_list_style);

        let profile_labels: Vec<String> =
            MouseProfile::ALL.into_iter().map(profile_label).collect();
        let selected_profile = profile_label(self.mouse_profile);
        let profile = PickList::new(profile_labels, Some(selected_profile), |label: String| {
            ModalAction::SetMouseProfile(profile_from_label(&label))
        })
        .width(Length::Fill)
        .style(crate::theme::pick_list_style);

        column![
            field_row(t!("settings.baud_rate"), baud),
            row![container(space()).width(Length::Fixed(150.0)), auto_baud,]
                .spacing(12)
                .padding([3, 0])
                .align_y(iced::alignment::Vertical::Center),
            field_row(t!("settings.preview_fps"), fps),
            field_row(t!("settings.mouse_mode"), mouse),
            field_row(t!("settings.mouse_profile"), profile),
            field_row(t!("settings.relative_sensitivity"), sensitivity),
        ]
        .spacing(10)
    }
}

fn profile_label(profile: MouseProfile) -> String {
    match profile {
        MouseProfile::Windows => t!("mouse_profile.windows").to_string(),
        MouseProfile::Linux => t!("mouse_profile.linux").to_string(),
        MouseProfile::Bios => t!("mouse_profile.bios").to_string(),
        MouseProfile::Android => t!("mouse_profile.android").to_string(),
        MouseProfile::MacOs => t!("mouse_profile.macos").to_string(),
        MouseProfile::RawAbsolute => t!("mouse_profile.raw_absolute").to_string(),
        MouseProfile::RawRelative => t!("mouse_profile.raw_relative").to_string(),
    }
}

fn profile_from_label(label: &str) -> MouseProfile {
    MouseProfile::ALL
        .into_iter()
        .find(|profile| profile_label(*profile) == label)
        .unwrap_or(MouseProfile::RawAbsolute)
}

fn close_button<'a>() -> iced::widget::Button<'a, ModalAction> {
    button(button_text(t!("modal.close")))
        .on_press(ModalAction::Close)
        .style(crate::theme::secondary_button)
}

/// footer 取消按钮：关闭模态。
fn cancel_button<'a>() -> iced::widget::Button<'a, ModalAction> {
    button(button_text(t!("common.cancel")))
        .on_press(ModalAction::Close)
        .style(crate::theme::secondary_button)
}

/// 标题栏右上角 × 关闭按钮：透明底，悬停变红。
fn close_x_button<'a>() -> iced::widget::Button<'a, ModalAction> {
    button(text("\u{00d7}").size(16).font(crate::fonts::ui_font()))
        .on_press(ModalAction::Close)
        .style(crate::theme::close_button_style)
        .padding(3)
}

/// 按钮内文本：13px 统一字号。
fn button_text<'a>(s: impl Into<String>) -> iced::widget::Text<'a> {
    text(s.into()).size(13).font(crate::fonts::ui_font())
}

/// 表单字段标签：13px。
fn field_label<'a>(s: impl Into<String>) -> iced::widget::Text<'a> {
    text(s.into()).size(13).font(crate::fonts::ui_font())
}

/// 表单行：固定 150px 标签列 + 控件（控件 Fill 撑满剩余宽度）。
fn field_row<'a>(
    label_text: impl Into<String>,
    control: impl Into<Element<'a, ModalAction>>,
) -> Element<'a, ModalAction> {
    let mut row = iced::widget::Row::new()
        .spacing(12)
        .align_y(iced::alignment::Vertical::Center);
    row = row.push(
        container(field_label(label_text))
            .width(Length::Fixed(150.0))
            .align_y(iced::alignment::Vertical::Center),
    );
    row = row.push(control);
    row.into()
}

/// 节标题：主色竖条 + 小字，用于表单分组。
fn section_title<'a>(s: impl Into<String>) -> Element<'a, ModalAction> {
    row![
        container(space())
            .width(Length::Fixed(3.0))
            .height(Length::Fixed(13.0))
            .style(|theme: &iced::Theme| container::Style {
                background: Some(theme.palette().primary.into()),
                ..Default::default()
            }),
        text(s.into()).size(12).font(crate::fonts::ui_font()),
    ]
    .spacing(6)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

/// footer 按钮行：右侧对齐。
fn footer_row<'a>(buttons: Vec<Element<'a, ModalAction>>) -> Element<'a, ModalAction> {
    let mut row = iced::widget::Row::new()
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center);
    row = row.push(space().width(Length::Fill));
    for button in buttons {
        row = row.push(button);
    }
    row.into()
}

/// 覆盖保存警示条：danger 弱底 + 边框。
fn overwrite_warning<'a>(message: impl Into<String>) -> Element<'a, ModalAction> {
    container(
        text(message.into())
            .size(13)
            .font(crate::fonts::ui_font())
            .width(Length::Fill),
    )
    .padding([10, 12])
    .width(Length::Fill)
    .style(|theme: &iced::Theme| {
        let danger = theme.palette().danger;
        container::Style {
            background: Some(Color::from_rgba(danger.r, danger.g, danger.b, 0.10).into()),
            border: Border::default()
                .rounded(crate::theme::CONTROL_RADIUS)
                .width(1.0)
                .color(Color::from_rgba(danger.r, danger.g, danger.b, 0.35)),
            ..Default::default()
        }
    })
    .into()
}

/// 关于信息行：弱色前缀 + 值。
fn info_line<'a>(s: impl Into<String>) -> Element<'a, ModalAction> {
    text(s.into()).size(13).font(crate::fonts::ui_font()).into()
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

/// 模态卡片：标题栏（标题 + × 关闭）+ 分隔线 + 内容 + footer 按钮行。
/// 固定宽度居中（不再横向填充），圆角加大、阴影加深。
fn modal_card<'a>(
    title: String,
    content: Element<'a, ModalAction>,
    footer: Element<'a, ModalAction>,
) -> Element<'a, ModalAction> {
    let header = row![
        label(title).size(16),
        space().width(Length::Fill),
        close_x_button(),
    ]
    .align_y(iced::alignment::Vertical::Center);
    let divider = container(space())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|theme: &iced::Theme| {
            // 分隔线比普通边框略深，保证标题区与内容区界限清晰。
            let text = theme.palette().text;
            container::Style {
                background: Some(Color::from_rgba(text.r, text.g, text.b, 0.22).into()),
                ..Default::default()
            }
        });
    let card = column![
        header,
        divider,
        content,
        space().height(Length::Fixed(4.0)),
        footer
    ]
    .spacing(12)
    .padding(20);
    container(card)
        .width(Length::Fixed(crate::theme::PANEL_WIDTH))
        .style(|theme| container::Style {
            background: Some(crate::theme::surface(theme.palette()).into()),
            border: Border::default()
                .rounded(crate::theme::PANEL_RADIUS)
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
