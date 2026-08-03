# M4 主题与观感迁移实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 给 `ipkvm-desktop-iced` 加自定义主题与观感：菜单选中高亮/箭头/分隔线/弹出面板、模态卡片样式、状态栏与连接页对齐 egui 观感、暗色模式适配、黑边颜色设置入口（设置对话框）。

**Architecture:** 新增 `theme.rs` 定义亮/暗两套 `Palette` 与派生色纯函数（surface/hover/border，测试可断言）；菜单/模态的自绘 draw 层用 `theme.palette()` 派生颜色绘制高亮与面板；`App` 增加 `dark` 字段与 `theme()`，经 iced application builder 接入；设置模态扩展黑边色预设与暗色开关。

**Tech Stack:** iced 0.14（tokio/image/advanced）、iced_test 0.14、iced `advanced::Renderer`（fill_quad 绘制菜单面板/高亮）。

## 执行记录（2026-08-03）

- 分支：`codex/issue78-migration-m4`；基线 `03d2cc3`（main）。
- 提交链：`691bac2`（theme）→ `59f838f`（菜单主题化）→ `f225a0c`（模态/状态栏/连接页样式）→ `e6147a3`（设置模态）→ 文档收口提交。
- 门禁：fmt / workspace tests / clippy `-D warnings` / rustdoc `-D warnings` 全部通过。
- 观感截图：`docs/superpowers/artifacts/m4-screenshots/m4-themed-connection-page.png`（自动截图存档；视觉评审人工项）。
- 执行偏差（均已落码并有测试覆盖）：
  - 子菜单箭头改为独立 text 并入 Row（`item_has_arrow`），不并入标签文本：iced_test 的 find/click 按精确文本匹配，并入会破坏 M2 菜单回归（corridor/menu_interact）。
  - `MenuItem` 增加 `separator` 标志替代原「──────────────」文本分隔线；分隔线不参与命中/悬停，draw 画 1px 横线。
  - `MenuBar::draw`/`MenuPopup::draw` 使用 `advanced::Renderer::fill_quad` 绘制打开根高亮、弹出面板（surface + 圆角边框 + 阴影）与悬停高亮。
  - 设置模态的「Black/White/Gray/Blue」色板按钮带文本标签，便于 headless 点击断言。
  - 相对模式光标锁定/隐藏：iced 0.14 无光标 grab API（winit 未暴露），记录为版本限制，M5 复查。
  - 绝对鼠标接线不在 #78 范围（egui 对齐项，M5 收口）。

## Global Constraints

- iced pin `0.14`，features `["tokio", "image", "advanced"]`（与 M0–M3 一致）。
- 迁移阶段每单必须新增测试（先红后绿），见 #82 与迁移设计文档第 4 节；禁止只靠既有测试变绿。
- 本单主体改 `ipkvm-desktop-iced`；不动 `ipkvm-desktop`。
- 交互回归：M2/M3 全部 headless 测试必须保持通过（菜单走廊/模态三关闭路径等）。
- 跨平台：样式不引入平台独占逻辑；darwin check CC-MISSING 属预期。
- 提交信息英文 conventional commit 并引用 `#78`；全量门禁 fmt / tests / clippy -D warnings / rustdoc -D warnings。

---

## 文件结构

- `crates/ipkvm-desktop-iced/src/theme.rs`：亮/暗 Palette、`app_theme(dark)`、派生色纯函数。
- `crates/ipkvm-desktop-iced/src/menu.rs`：菜单主题化（面板/高亮/分隔线/箭头）。
- `crates/ipkvm-desktop-iced/src/modal.rs`：模态卡片主题化 + 设置内容（黑边色/暗色）。
- `crates/ipkvm-desktop-iced/src/app.rs`：`dark` 字段、`theme()`、设置消息接线、状态栏/连接页样式。
- `crates/ipkvm-desktop-iced/locales/`：设置相关文案。

---

## Task 1: 主题模块（theme.rs）与 App 接线

**Files:**
- Create: `crates/ipkvm-desktop-iced/src/theme.rs`
- Modify: `crates/ipkvm-desktop-iced/src/lib.rs`（`pub mod theme;`）
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`（`dark` 字段 + `theme()` + run() 接入）

**Interfaces:**
- Produces: `pub const LIGHT: Palette`、`pub const DARK: Palette`、`pub fn app_theme(dark: bool) -> iced::Theme`、`pub fn surface(palette: Palette) -> Color`、`pub fn hover(palette: Palette) -> Color`、`pub fn border_color(palette: Palette) -> Color`。

- [x] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color) -> f32 {
        // 相对亮度（sRGB 线性化后加权）。
        fn lin(v: f32) -> f32 {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn light_and_dark_palettes_differ() {
        assert_ne!(LIGHT, DARK);
        assert_ne!(LIGHT.background, DARK.background);
        assert_ne!(LIGHT.text, DARK.text);
    }

    #[test]
    fn text_background_contrast_is_readable_in_both_modes() {
        for palette in [LIGHT, DARK] {
            assert!(
                contrast(palette.text, palette.background) >= 4.5,
                "文本/背景对比度必须 >= 4.5"
            );
        }
    }

    #[test]
    fn app_theme_returns_custom_palette_for_mode() {
        assert_eq!(app_theme(true).palette(), DARK);
        assert_eq!(app_theme(false).palette(), LIGHT);
    }

    #[test]
    fn derived_colors_are_distinct_and_opaque() {
        for palette in [LIGHT, DARK] {
            let s = surface(palette);
            let h = hover(palette);
            let b = border_color(palette);
            assert_ne!(s, palette.background, "surface 不得等于背景");
            assert!(h.a > 0.0 && h.a < 1.0, "hover 必须是半透明高亮");
            assert!(b.a > 0.0, "边框色必须可见");
        }
    }
}
```

- [x] **Step 2: 运行确认失败**（`theme` 模块未定义）
- [x] **Step 3: 实现 theme.rs**

```rust
//! my_ipkvm 自定义主题：亮/暗 Palette 与派生色纯函数。

use iced::Color;
use iced::theme::{Palette, Theme};

/// 亮色主题（对齐 egui 端亮色观感）。
pub const LIGHT: Palette = Palette {
    background: Color::from_rgb(0.96, 0.96, 0.97),
    text: Color::from_rgb(0.12, 0.12, 0.14),
    primary: Color::from_rgb(0.20, 0.42, 0.82),
    success: Color::from_rgb(0.12, 0.55, 0.35),
    warning: Color::from_rgb(0.78, 0.55, 0.12),
    danger: Color::from_rgb(0.76, 0.20, 0.20),
};

/// 暗色主题（默认：KVM 监控类工具深色观感，对齐 egui 端暗色）。
pub const DARK: Palette = Palette {
    background: Color::from_rgb(0.13, 0.14, 0.17),
    text: Color::from_rgb(0.90, 0.90, 0.92),
    primary: Color::from_rgb(0.34, 0.56, 0.95),
    success: Color::from_rgb(0.30, 0.72, 0.52),
    warning: Color::from_rgb(0.92, 0.66, 0.28),
    danger: Color::from_rgb(0.90, 0.34, 0.32),
};

/// 按模式返回应用主题（iced application builder 使用）。
pub fn app_theme(dark: bool) -> Theme {
    Theme::custom(
        "my_ipkvm",
        if dark { DARK } else { LIGHT },
    )
}

/// 弹出面板/卡片表面色：背景与文本按 6% 混合，亮色模式更亮、暗色模式更暗。
pub fn surface(palette: Palette) -> Color {
    mix(palette.background, palette.text, 0.06)
}

/// 选中/悬停高亮：主题主色半透明。
pub fn hover(palette: Palette) -> Color {
    Color::from_rgba(palette.primary.r, palette.primary.g, palette.primary.b, 0.18)
}

/// 面板边框：文本色低透明度。
pub fn border_color(palette: Palette) -> Color {
    Color::from_rgba(palette.text.r, palette.text.g, palette.text.b, 0.16)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}
```

`lib.rs` 增加 `pub mod theme;`。

`app.rs`：
- App 增加字段 `dark: bool`（new_mock/production 初始 `true`）。
- 增加方法：

```rust
/// 应用主题（iced builder 的 theme 回调）。
pub fn theme(&self) -> iced::Theme {
    crate::theme::app_theme(self.dark)
}
```

- `run()` 增加 `.theme(App::theme)`。
- 设置模态打开时同步 `self.modal.dark = self.dark`（`Message::OpenModal(ModalKind::Settings)` 分支）。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced theme::`
Expected: 4 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/theme.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/src/app.rs
git commit -m "feat(iced): custom theme palettes and dark mode wiring (#78)"
```

## Task 2: 菜单主题化

**Files:**
- Modify: `crates/ipkvm-desktop-iced/src/menu.rs`

**Interfaces:**
- Produces: `MenuItem::separator` 标志、`pub fn item_label(item: &MenuItem) -> String`（子菜单项带 " ›" 箭头，分隔线为空串）。

- [x] **Step 1: 写失败测试**（menu.rs 测试模块追加）

```rust
#[test]
fn item_label_shows_arrow_for_submenu_and_empty_for_separator() {
    let sub = MenuItem::with_submenu("Recent", Vec::new());
    assert!(item_label(&sub).contains('›'));
    let sep = MenuItem::separator();
    assert!(sep.separator, "separator() 必须标记分隔线");
    assert_eq!(item_label(&sep), "");
    let plain = MenuItem::action("Paste text", MenuAction::Simple("paste"));
    assert_eq!(item_label(&plain), "Paste text");
}
```

- [x] **Step 2: 运行确认失败**（`item_label`/`separator` 字段不存在）
- [x] **Step 3: 实现**

`MenuItem` 增加字段：

```rust
pub struct MenuItem {
    pub label: String,
    pub action: Option<MenuAction>,
    pub submenu: Vec<MenuItem>,
    /// true = 分隔线（不参与命中/悬停，draw 画横线）。
    pub separator: bool,
}
```

构造函数同步补 `separator: false`；`separator()` 改为：

```rust
pub fn separator() -> Self {
    Self {
        label: String::new(),
        action: None,
        submenu: Vec::new(),
        separator: true,
    }
}
```

新增：

```rust
/// 菜单项显示文本：子菜单项追加箭头，分隔线为空。
pub fn item_label(item: &MenuItem) -> String {
    if item.separator {
        return String::new();
    }
    if item.submenu.is_empty() {
        item.label.clone()
    } else {
        format!("{}  ›", item.label)
    }
}
```

`label_element` 改为按 item 生成：

```rust
fn label_element<'a>(item: &MenuItem) -> MenuElement<'a> {
    text(item_label(item)).into()
}
```

调用点同步改为传 `item`（`MenuBar::overlay` 的 `root.items.iter().map(|i| label_element(i))`、`MenuPopup` 构造两处、`popup_tree` 的 `Tree::new(label_element(&item))`）。

`MenuPopup::update` 的 `hover_item`/`activate_item` 开头跳过分隔线：

```rust
fn activate_item(&mut self, i: usize, shell: &mut Shell<'_, MenuAction>) {
    let Some(item) = self.items.get(i) else { return; };
    if item.separator { return; }
    ...
}

fn hover_item(&mut self, i: usize, shell: &mut Shell<'_, MenuAction>) {
    let Some(item) = self.items.get(i) else { return; };
    if item.separator { return; }
    ...
}
```

`MenuBar::draw` 增加打开根按钮高亮（在文字绘制前）：

```rust
use iced::advanced::Renderer as _;
use iced::advanced::renderer::Quad;
use iced::border;

// MenuBar::draw 开头：
let palette = theme.palette();
if let Some(open) = self.open_root {
    if let Some(rect) = self.root_label_rect(layout, open).into_bounds() {
        let quad = Quad {
            bounds: rect,
            border: border::rounded(4),
            ..Quad::default()
        };
        renderer.fill_quad(quad, crate::theme::hover(palette));
    }
}
```

> `root_label_rect` 当前返回 `Rectangle`；`Rectangle` 有 `.into_bounds()` 吗？没有——直接用 `Rectangle`（iced::Rectangle 即 Bounds 别名）。上述 `into_bounds()` 改为直接使用 rect：

```rust
if let Some(open) = self.open_root {
    let rect = self.root_label_rect(layout, open);
    if rect.width > 0.0 && rect.height > 0.0 {
        let quad = Quad {
            bounds: rect,
            border: border::rounded(4),
            ..Quad::default()
        };
        renderer.fill_quad(quad, crate::theme::hover(palette));
    }
}
```

`MenuPopup::draw` 改为：先画面板背景/边框/分隔线/悬停高亮，再画文字：

```rust
fn draw(&self, renderer, theme, style, layout, cursor) {
    let palette = theme.palette();
    let panel = layout.bounds();
    if panel.width > 0.0 && panel.height > 0.0 {
        let quad = Quad {
            bounds: panel,
            border: Border::default()
                .rounded(6)
                .width(1.0)
                .color(crate::theme::border_color(palette)),
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 12.0,
            },
            snap: false,
        };
        renderer.fill_quad(quad, crate::theme::surface(palette));
    }
    let item_rects: Vec<Rectangle> = layout.children().map(|child| child.bounds()).collect();
    let hovered = cursor
        .position()
        .and_then(|pos| hit_rects(pos, &item_rects))
        .filter(|i| !self.items.get(*i).is_some_and(|item| item.separator));
    if let Some(i) = hovered {
        let rect = item_rects[i];
        let quad = Quad {
            bounds: rect,
            border: border::rounded(4),
            ..Quad::default()
        };
        renderer.fill_quad(quad, crate::theme::hover(palette));
    }
    for (i, (widget, child)) in self.widgets.iter().zip(layout.children()).enumerate() {
        if self.items.get(i).is_some_and(|item| item.separator) {
            // 分隔线：1px 横线。
            let rect = child.bounds();
            let line = Rectangle {
                x: rect.x + 8.0,
                y: rect.y + rect.height / 2.0,
                width: rect.width - 16.0,
                height: 1.0,
            };
            renderer.fill_quad(
                Quad { bounds: line, ..Quad::default() },
                crate::theme::border_color(palette),
            );
            continue;
        }
        widget.as_widget().draw(
            &self.tree.children[i],
            renderer,
            theme,
            style,
            child,
            cursor,
            &panel,
        );
    }
}
```

`menu.rs` 增加导入：`use iced::advanced::renderer::Quad;`、`use iced::advanced::Renderer as _;`、`use iced::border;`、`use iced::{Color, Shadow, Vector};`（Color/Shadow/Vector 若未导入）。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced menu::` 与 `cargo test -p ipkvm-desktop-iced --test menu_interact --test corridor_hover --test i18n_switch`
Expected: 3（新）+ 13（回归）passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/menu.rs
git commit -m "feat(iced): themed menu popup with highlight, separators and arrows (#78)"
```

## Task 3: 模态卡片/状态栏/连接页样式

**Files:**
- Modify: `crates/ipkvm-desktop-iced/src/modal.rs`
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`

**Interfaces:**
- Consumes: `crate::theme::{surface, border_color}`。

- [x] **Step 1: 写失败测试（headless 渲染冒烟）**

`src/app.rs` 测试模块追加：

```rust
#[test]
fn connection_page_view_renders_after_theme_wiring() {
    let _guard = crate::I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::Disconnect);
    let _ = app.update(Message::RefreshDevices);
    let mut ui = iced_test::simulator::simulator(app.view());
    assert!(ui.find("Select device").is_ok(), "连接页标题必须渲染");
    assert!(ui.find("Refresh detection").is_ok(), "刷新按钮必须渲染");
}
```

- [x] **Step 2: 运行确认失败**（若已通过则改为先实现后验证；此处主要防回归）
- [x] **Step 3: 实现**

`modal.rs`：

- `modal_card` 的白色背景改为主题化（style 闭包接收 theme）：

```rust
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
```

- 遮罩透明度从 0.45 提到 0.55（模态层级更清晰），颜色不变。

`app.rs`：

- `status_line` 样式化：

```rust
fn status_line(&self) -> Element<'_, Message> {
    use iced::widget::{container, text};
    container(text(self.status.label(self.zh)))
        .width(Length::Fill)
        .padding(6)
        .style(|theme| container::Style {
            background: Some(crate::theme::surface(theme.palette()).into()),
            border: Border::default().width(0.0),
            ..Default::default()
        })
        .into()
}
```

- `connection_view` 外层内容包一个面板容器（surface 背景 + 圆角边框），并给按钮加主色风格（连接按钮 `button::primary` 风格）。

```rust
// connection_view 的 column 包进：
let panel = container(column![...]).width(Length::Fill).padding(16).style(|theme| container::Style {
    background: Some(crate::theme::surface(theme.palette()).into()),
    border: Border::default().rounded(10).width(1.0).color(crate::theme::border_color(theme.palette())),
    ..Default::default()
});
```

连接按钮：

```rust
let connect = button(text(t!("device.connect")))
    .on_press_maybe(self.selection.can_connect().then_some(Message::Connect))
    .style(iced::widget::button::primary);
```

`app.rs` 顶部 `use iced::border; use iced::Shadow; use iced::Vector;`（Vector 仅 Shadow offset 用）。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced --test modal_blocking app::connection_page_view_renders_after_theme_wiring`
Expected: 5 + 1 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/modal.rs crates/ipkvm-desktop-iced/src/app.rs
git commit -m "feat(iced): themed modal card, status bar and connection page (#78)"
```

## Task 4: 设置模态（黑边色预设 + 暗色开关）

**Files:**
- Modify: `crates/ipkvm-desktop-iced/src/modal.rs`
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`
- Modify: `crates/ipkvm-desktop-iced/locales/{en,zh-CN}.yml`、`src/lib.rs`（I18N_KEYS/translate_key）

**Interfaces:**
- Produces: `ModalAction::{SetLetterboxColor(Color), SetDarkMode(bool)}`、`ModalState.dark: bool`、设置模态内容（黑边色 swatch + 暗色 Checkbox）。

- [x] **Step 1: 写失败测试**

`tests/modal_blocking.rs` 追加：

```rust
#[test]
fn settings_modal_emits_letterbox_and_dark_messages() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut app = TestApp::default();
    app.open(ModalKind::Settings);
    let mut ui = simulator::simulator(app.view());
    assert!(ui.click("Black").is_ok(), "黑边色预设必须可点击");
    let msgs = messages_of(ui);
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Msg::Modal(ModalAction::SetLetterboxColor(c)) if c == &iced::Color::BLACK
        )),
        "点击 Black 必须产生 SetLetterboxColor(black)（实际 {msgs:?}）"
    );
}
```

`src/app.rs` 测试模块追加：

```rust
#[test]
fn settings_modal_updates_letterbox_and_dark_mode() {
    let (mut app, _) = MockApp::new_mock();
    let _ = app.update(Message::OpenModal(ModalKind::Settings));
    let _ = app.update(Message::Modal(ModalAction::SetLetterboxColor(
        iced::Color::WHITE,
    )));
    assert_eq!(app.letterbox_color(), iced::Color::WHITE);
    let _ = app.update(Message::Modal(ModalAction::SetDarkMode(false)));
    assert!(!app.dark);
}
```

- [x] **Step 2: 运行确认失败**（`SetLetterboxColor` 不存在）
- [x] **Step 3: 实现**

`modal.rs`：

```rust
pub enum ModalAction {
    Close,
    SaveNameChanged(String),
    Save,
    LoadPicked(String),
    SetLetterboxColor(Color),
    SetDarkMode(bool),
    Noop,
}

pub struct ModalState {
    pub open: Option<ModalKind>,
    pub save_name: String,
    pub load_names: Vec<String>,
    /// 设置模态显示用的暗色开关（app 打开前同步）。
    pub dark: bool,
}
```

`settings_content` 实现：

```rust
fn settings_content(&self) -> Element<'_, ModalAction> {
    use iced::widget::{button, button::Style as ButtonStyle, checkbox, column, text, Checkbox};
    let swatches = [
        ("Black", Color::BLACK),
        ("White", Color::WHITE),
        ("Gray", Color::from_rgb(0.35, 0.35, 0.35)),
        ("Blue", Color::from_rgb(0.2, 0.4, 0.8)),
    ];
    let mut rows: Vec<Element<'_, ModalAction>> = Vec::new();
    for (label, color) in swatches {
        let style = move |_theme: &iced::Theme, _status: Status| ButtonStyle {
            background: Some(color.into()),
            text_color: Color::WHITE,
            border: Border::default().rounded(6),
            ..Default::default()
        };
        rows.push(
            button(text(label))
                .on_press(ModalAction::SetLetterboxColor(color))
                .style(style)
                .into(),
        );
    }
    let dark_toggle = Checkbox::new(self.dark)
        .label(t!("settings.dark_mode"))
        .on_toggle(ModalAction::SetDarkMode);
    let mut content = iced::widget::Column::new().spacing(8);
    content = content.push(text(t!("settings.letterbox_color")));
    for row in rows {
        content = content.push(row);
    }
    content.push(dark_toggle).push(close_button()).into()
}
```

`app.rs`：

- `handle_modal_action` 增加：

```rust
ModalAction::SetLetterboxColor(color) => self.letterbox_color = color,
ModalAction::SetDarkMode(dark) => self.dark = dark,
```

- `Message::OpenModal`/菜单打开 Settings 时同步：`if kind == ModalKind::Settings { self.modal.dark = self.dark; }`。
- `tests/modal_blocking.rs` 的 `TestApp::update` match 增加两个新变体。

`locales`：

```yaml
settings:
  letterbox_color: "Letterbox color"      # 中文：黑边颜色
  dark_mode: "Dark mode"                  # 中文：暗色模式
```

`I18N_KEYS` 与 `translate_key` 同步增加 `settings.letterbox_color`、`settings.dark_mode`。

- [x] **Step 4: 运行确认通过**

Run: `cargo test -p ipkvm-desktop-iced --test modal_blocking app::settings_modal_updates_letterbox_and_dark_mode`
Expected: 6 + 1 passed。

- [x] **Step 5: Commit**

```bash
git add crates/ipkvm-desktop-iced/src/modal.rs crates/ipkvm-desktop-iced/src/app.rs crates/ipkvm-desktop-iced/src/lib.rs crates/ipkvm-desktop-iced/locales crates/ipkvm-desktop-iced/tests/modal_blocking.rs
git commit -m "feat(iced): settings modal for letterbox color and dark mode (#78)"
```

## Task 5: 门禁与验收

- [x] **Step 1: 全量门禁**

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --all-features --no-deps
```

- [x] **Step 2: 验收核对（对应 #78 / #82）**
  - [x] 主题模块测试（亮/暗 palette、对比度、派生色）
  - [x] 菜单主题化测试（箭头/分隔线标记）+ M2 菜单 13 项回归
  - [x] 模态/状态栏/连接页样式 + modal 5 项回归
  - [x] 设置模态黑边色/暗色开关测试
  - [x] 截图存档：`docs/superpowers/artifacts/m4-screenshots/`（自动截图，视觉评审人工项）
  - [x] 回写 #78 验收结论
- [x] **Step 3: 提交文档更新并推送 PR**

```bash
git add docs/superpowers/plans/2026-08-03-iced-migration-m4.md HANDOFF.md
git commit -m "docs: record M4 plan and verification (#78)"
git push -u origin codex/issue78-migration-m4
```

- [x] **Step 4: PR → 自审 → 合并 → 关单**（`Closes #78`）
- [x] **Step 5: 同步 main 并继续 M5**

## Self-Review（计划自审）

- **Spec coverage**：对照 #78 与设计文档 3.3/3.7：自定义 Theme/appearance ✅（Task 1/2/3）、菜单选中高亮/箭头/分隔线/圆角阴影 ✅（Task 2）、模态卡片样式 ✅（Task 3）、状态栏/连接页观感 + 暗色适配 ✅（Task 1/3/4）、黑边颜色设置入口 ✅（Task 4）、截图评审 ✅（Task 5 人工项）、交互回归 ✅（Task 5 全量）。未覆盖项：相对模式光标锁定/隐藏——iced 0.14 无光标 grab API（winit 未暴露），记录为版本限制，M5 复查；绝对鼠标接线不在 #78 范围（egui 对齐项，M5 收口）。
- **Placeholder scan**：无 TBD；Task 2 的 `into_bounds()` 笔误已在计划内修正说明。
- **Type consistency**：`app_theme(dark) -> Theme`、`surface/hover/border_color(Palette) -> Color`、`ModalAction::{SetLetterboxColor(Color), SetDarkMode(bool)}`、`MenuItem.separator: bool`、`item_label(&MenuItem) -> String` 跨任务一致。

