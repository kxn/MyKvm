# #153 iced 视觉语言与布局层级实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. 本计划最后执行，消费 #151/#154 已稳定的控件和状态。

**Goal:** 只调整 iced 的外观、尺寸层级和布局，使其对齐 headless 浅色视觉语言，不改变消息、状态机和行为。

**状态：** 实现、回归测试和结构性视觉 QA 已完成，纳入联合 PR 收口。

**Architecture:** 在 `theme.rs` 建立 headless-derived design tokens 和控件 style helper；`modal.rs` 使用有最大宽度的 centered panel 和 label/control 两列 form；`app.rs/menu.rs` 统一使用 helper，窄窗口通过 responsive row/column 退化。

## Global Constraints

- 页面背景、面板、边框、强调色、弱化文字与 headless 浅色基准一致。
- 常用控件高度 30–38px，控件圆角约 4px，面板圆角约 8–10px，弹窗最大宽度约 460px。
- 不修改行为、消息路由、配置语义、输入逻辑和 headless Web 样式。

### Task 1: Visual Regression Contracts

**Files:** `crates/ipkvm-desktop-iced/src/theme.rs`, `src/modal.rs`, `src/app.rs`, iced tests.

- [x] 为 panel/button/input/pick-list/checkbox/status/menu 定义 token 和 style helper。
- [x] 增加结构/尺寸测试：modal max width、settings two-column row、控件高度和长文本不越界。
- [x] 运行现有 iced interaction/pixel tests，记录并消化布局基线。

### Task 2: Modal and Form Layout

**Files:** `crates/ipkvm-desktop-iced/src/modal.rs`, `src/app.rs`.

- [x] 设置、连接设置、保存 profile、关于弹窗统一 panel 宽度、padding、border/shadow。
- [x] 标签与控件默认同一行，窄宽度用 responsive fallback 纵向排列；不让父级 Fill 横向撑开。
- [x] 将 #151 的 profile selector 和 #154 capture/status 文案纳入同一控件样式。

### Task 3: Page/Menu/Control Styles

**Files:** `crates/ipkvm-desktop-iced/src/theme.rs`, `crates/ipkvm-desktop-iced/src/menu.rs`, `crates/ipkvm-desktop-iced/src/status.rs`, `crates/ipkvm-desktop-iced/src/app.rs`, `crates/ipkvm-desktop-iced/locales/en.yml`, `crates/ipkvm-desktop-iced/locales/zh-CN.yml`.

- [x] 统一普通/主操作/禁用/悬停/按下态，连接页、视频页、菜单、状态栏不再落到 iced 默认样式。
- [x] 保持中英文文本单行/换行约束，检查 1280x800 与窄窗口的结构约束。
- [x] 不改变 `Message`、点击命中、键盘快捷键和 modal blocking。

### Task 4: Visual QA and Documentation

- [x] 运行 fmt、workspace tests、clippy、doc、release build。
- [x] 记录 1280x800 和窄窗口连接页/设置/菜单/视频状态栏的结构性 QA；真实硬件不作为视觉验收前提。
- [x] 更新 iced 设计文档并纳入联合提交，使用 `Closes #153` 收口。
