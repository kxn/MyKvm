# #156 键鼠移动调度与控制顺序实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. 设计依据为 `docs/superpowers/specs/2026-08-04-input-scheduler-order-design.md`。

**Goal:** 键盘立即、鼠标移动独立限频，按钮/滚轮先冲刷移动，并让 Iced 与 Web/noVNC 顺序一致。

**状态：** 实现与自动化回归已完成，纳入联合 PR 收口。

**Architecture:** Iced 扩展 `DeltaSampler` 提供周期采样和控制事件强制取出；Web/noVNC 增加相对累计量与独立 33ms 定时器。两端都在控制事件前 flush，键盘路径不经过鼠标调度器。

## Global Constraints

- 保留约 33ms 移动采样周期；绝对移动只保留最新坐标，相对移动累计不丢量。
- 快速 button down/up 必须是两个独立边沿。
- 不修改 RFB、CH9329、串口队列和 egui。

### Task 1: Iced Sampler Contract

**Files:** `crates/ipkvm-desktop-iced/src/relative.rs`.

- [x] 增加 `flush(now)` 返回当前整数增量、`reset()` 清理余数和周期。
- [x] 测试周期内累计、控制事件强制 flush、余数保留、reset 不泄漏。
- [x] 运行 `cargo test -p ipkvm-desktop-iced --lib relative`。

### Task 2: Iced Event Ordering

**Files:** `crates/ipkvm-desktop-iced/src/app.rs`, `src/input.rs`, tests in app.

- [x] 绝对/相对移动统一保留待发状态；button/wheel 先 flush 再提交控制事件。
- [x] 保持 key down/up 直接 `send_key`，不等待鼠标采样。
- [x] 退出远程输入、断开、模式切换清理 sampler、坐标和按钮同步状态。
- [x] 用记录 sink 测试移动->button、移动->wheel、快速 button down/up、键盘不延迟。

### Task 3: Web/noVNC Scheduler

**Files:** `third_party/novnc/1.7.0/core/rfb.js`, `crates/ipkvm-headless/src/web/assets.rs`, browser tests.

- [x] 相对 mousemove 累计灵敏度后的 dx/dy，首个有效移动立即发送，后续 33ms 合并。
- [x] button/wheel flush 相对累计量后独立发送控制边沿；退出、断开、setRelativeMode(false) 清理 timer/state。
- [x] 增加 headless asset/source assertions 和 Playwright/等价 browser test。

### Task 4: Verify and Document

- [x] 运行 Rust 全量、M5/crate gate、`node browser-tests/novnc-browser.mjs`。
- [x] 更新输入调度设计文档执行记录。
- [x] 纳入联合提交并使用 `Closes #156` 收口。
