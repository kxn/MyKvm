//! 预上传图像 widget（#89）：消除视频帧异步上传导致的空白闪烁。
//!
//! iced 0.14 对未加载的 `image::Handle` 走异步解码/上传，上传完成前 draw
//! 什么都不画（露出 letterbox 背景）。`Allocation` 文档保证：只要持有
//! Allocation，同一 Handle 的绘制立即可见。本 widget 在 layout 阶段
//! （`&mut Tree` + `&Renderer` 同时可用）为新 Handle 阻塞预上传并持有
//! Allocation；旧帧 Allocation 一直保留到新帧上传成功才替换，因此不会出现
//! 「新帧未就绪 → 空白」的闪烁窗口。

use iced::advanced::image::{self, Allocation, FilterMethod, Image, Renderer as ImageRenderer};
use iced::advanced::layout::{self, Layout};
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget, tree};
use iced::{ContentFit, Element, Length, Point, Rectangle, Size};

/// 一个在 layout 阶段预上传、draw 阶段立即可见的图像 widget。
///
/// 用法与 `iced::widget::image::Image` 一致（`new(handle)` +
/// `content_fit(...)` 等），区别是内部持有 [`Allocation`]。
pub struct PreloadedImage<Handle = image::Handle> {
    handle: Handle,
    width: Length,
    height: Length,
    content_fit: ContentFit,
    filter_method: FilterMethod,
}

impl<Handle> PreloadedImage<Handle> {
    /// Creates a new [`PreloadedImage`] with the given handle.
    pub fn new(handle: impl Into<Handle>) -> Self {
        Self {
            handle: handle.into(),
            width: Length::Shrink,
            height: Length::Shrink,
            content_fit: ContentFit::default(),
            filter_method: FilterMethod::default(),
        }
    }

    /// Sets the width of the [`PreloadedImage`] boundaries.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`PreloadedImage`] boundaries.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the [`ContentFit`] of the [`PreloadedImage`].
    pub fn content_fit(mut self, content_fit: ContentFit) -> Self {
        self.content_fit = content_fit;
        self
    }

    /// Sets the [`FilterMethod`] of the [`PreloadedImage`].
    pub fn filter_method(mut self, filter_method: FilterMethod) -> Self {
        self.filter_method = filter_method;
        self
    }
}

/// Widget 持久状态：已上传的 Allocation 及其 Handle。
struct State<Handle> {
    allocation: Option<Allocation>,
    handle: Option<Handle>,
}

impl<Handle> Default for State<Handle> {
    fn default() -> Self {
        Self {
            allocation: None,
            handle: None,
        }
    }
}

impl<Message, Theme, Renderer, Handle> Widget<Message, Theme, Renderer> for PreloadedImage<Handle>
where
    Renderer: ImageRenderer<Handle = Handle>,
    Handle: Clone + PartialEq + 'static,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Handle>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new::<State<Handle>>(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Handle>>();
        // 新 Handle（新帧）：阻塞预上传并持有 Allocation。旧帧的 Allocation
        // 直到新帧上传成功才被替换，期间 draw 仍是旧帧，无空白窗口。
        if state.handle.as_ref() != Some(&self.handle) {
            state.allocation = renderer.load_image(&self.handle).ok();
            state.handle = Some(self.handle.clone());
        }

        // 布局语义与 iced_widget::image 一致（无 crop/rotation/expand）。
        let image_size = renderer
            .measure_image(&self.handle)
            .map(|size| Size::new(size.width as f32, size.height as f32))
            .unwrap_or_default();
        let bounds = limits.resolve(self.width, self.height, image_size);
        let full_size = self.content_fit.fit(image_size, bounds);
        let final_size = Size {
            width: match self.width {
                Length::Shrink => f32::min(bounds.width, full_size.width),
                _ => bounds.width,
            },
            height: match self.height {
                Length::Shrink => f32::min(bounds.height, full_size.height),
                _ => bounds.height,
            },
        };
        layout::Node::new(final_size)
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let original = renderer
            .measure_image(&self.handle)
            .map(|size| Size::new(size.width as f32, size.height as f32))
            .unwrap_or_default();
        let adjusted = self.content_fit.fit(original, bounds.size());
        let drawing = Rectangle::new(
            Point::new(
                bounds.center_x() - adjusted.width / 2.0,
                bounds.center_y() - adjusted.height / 2.0,
            ),
            adjusted,
        );
        renderer.draw_image(
            Image {
                handle: self.handle.clone(),
                filter_method: self.filter_method,
                rotation: iced::Radians(0.0),
                border_radius: iced::border::Radius::default(),
                opacity: 1.0,
                snap: true,
            },
            drawing,
            bounds,
        );
    }
}

impl<'a, Message, Theme, Renderer, Handle> From<PreloadedImage<Handle>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: ImageRenderer<Handle = Handle> + 'a,
    Handle: Clone + PartialEq + 'static,
{
    fn from(widget: PreloadedImage<Handle>) -> Self {
        Self::new(widget)
    }
}
