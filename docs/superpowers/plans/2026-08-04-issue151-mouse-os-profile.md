# #151 鼠标 OS profile 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. 本计划作为五单连续实施的一部分执行，不在阶段之间暂停询问。

**Goal:** 在 Iced 和 headless Web 统一提供五个目标 OS profile、两个原始模式，并兼容旧 `mouse_mode` 配置/API。

**状态：** 实现与对应回归测试已完成，纳入联合 PR 收口。

**Architecture:** 在 `ipkvm-core` 增加无 serde 的 `MouseProfile` 稳定标识和 `resolve_mode`；桌面与 Web 各自做 serde/JSON 适配。profile 保留在桌面连接配置、WebSettings、session selection 和 `/api/status`，实际 sink 仍只接收 `MouseMode`。

**Tech Stack:** Rust 2024、iced 0.14、axum 0.8、原生 noVNC JavaScript、serde/toml。

## Global Constraints

- 保留 `MouseMode::Absolute` 和 `MouseMode::Relative`。
- 默认值为 `Raw(Absolute)`；旧 `mouse_mode` 只读迁移为对应 Raw profile。
- 不修改 egui、RFB 协议、CH9329 协议或 Pointer Lock 的用户手势约束。
- 任何实际模式变化只执行 `set_mouse_mode`，由 sink 释放旧鼠标按钮；失败回滚选择和本地状态。

### Task 1: Shared Profile Model

**Files:** Modify `crates/ipkvm-core/src/input.rs`, `crates/ipkvm-core/src/lib.rs`; tests in `input.rs`.

- [x] 为 `MouseProfile::{Windows,Linux,Bios,Android,MacOs,RawAbsolute,RawRelative}` 增加稳定标识、解析和 `MouseMode` 映射测试。
- [x] 实现 `as_str`, `parse`, `resolve_mode`，未知值返回明确错误；同模式 profile 比较时保留 identity。
- [x] 运行 `cargo test -p ipkvm-core`。

### Task 2: Desktop Configuration Migration

**Files:** `crates/ipkvm-desktop-core/src/config.rs`, `src/session.rs`, `crates/ipkvm-desktop-iced/src/profile.rs`.

- [x] 在 `ConnectionSettings` 增加 profile 字段并保留 `mouse_mode` 旧字段读取兼容。
- [x] 让 `ConnectRequest` 和 session factory 以 profile resolve 后的 mode 构造 sink。
- [x] 覆盖旧 TOML、新 TOML、缺失/未知 profile、保存/加载 profile 和手动快照。
- [x] 运行 `cargo test -p ipkvm-desktop-core -p ipkvm-desktop`。

### Task 3: Iced Controls and Mode Lifecycle

**Files:** `crates/ipkvm-desktop-iced/src/app.rs`, `src/modal.rs`, `src/profile.rs`, `src/lib.rs`, locales.

- [x] 将状态栏、连接设置、默认设置和保存/加载 profile 接入同一组选项。
- [x] 实际模式变化走 controller 的 release/set 顺序；同模式 profile 只更新 identity。
- [x] 无连接时只更新参数；连接中失败时恢复旧 profile、remote input 和 cursor 状态。
- [x] 添加应用级测试，覆盖五个 profile、原始模式、失败回滚和同模式不重复切换。

### Task 4: Headless Settings/API/Status

**Files:** `crates/ipkvm-headless/src/settings.rs`, `src/web/service.rs`, `src/web/recovery.rs`, `web/modules/{api,settings,connection,status,pointer}.js`, `web/index.html`.

- [x] `WebSettings` 增加 `mouse_profile`，旧 JSON/TOML 仅有 `mouse_mode` 时迁移为 Raw。
- [x] `POST /api/session` 接受 profile override；增加当前会话切换接口并返回最终 profile/mode。
- [x] `/api/status` 返回 profile、解析 mode 和实际 sink 模式；本地 capture 留在浏览器并串行化切换。
- [x] Web 连接页、设置和视频状态栏使用相同选项；相对选择不自动请求 Pointer Lock。
- [x] 添加 HTTP、Rust helper 和 browser fixture 测试，覆盖新旧字段、成功/失败、状态回读路径。

### Task 5: Documentation

**Files:** `docs/ipkvm-coarse-design.md`, `docs/superpowers/specs/2026-08-04-hid-os-profile-research.md`.

- [x] 记录 profile 映射、Raw 兼容、Pointer Lock 分层和人工验证矩阵。
- [x] 运行格式、core/desktop/headless 测试后纳入联合提交并使用 `Closes #151` 收口。
