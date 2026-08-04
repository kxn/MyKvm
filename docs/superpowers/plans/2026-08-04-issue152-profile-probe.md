# #152 加载 profile 后控制探测实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. 本计划依赖 #151 桌面配置字段稳定后执行。

**Goal:** 加载最近 profile、文件 profile 和连接页 profile 后，匹配的控制设备自动完成一次探测，不再卡在 `Checking`。

**状态：** 实现与对应回归测试已完成，纳入联合 PR 收口。

**Architecture:** 把“应用选择”和“完成控制探测”明确分成两个同步步骤；应用 profile 后只对存在且匹配的 control device 调用一次现有 `probe_control`，视频仍由现有 PreviewTick 负责。

## Global Constraints

- 不对缺失的串口调用探测。
- 探测失败必须保留具体 `NoResponse`/`NotCh9329`/`OpenFailed` 等状态。
- 三个加载入口必须收敛到同一 helper，不复制状态逻辑。

### Task 1: Regression Tests

**Files:** `crates/ipkvm-desktop-iced/src/app.rs`, `src/profile.rs`.

- [x] 增加成功场景：枚举视频/控制设备，清空选择，加载 profile，推进消息/任务，断言 control 为 Ready 且 `can_connect()` 为 true。
- [x] 增加探测失败场景，断言最终状态是具体失败而非 Checking。
- [x] 分别覆盖最近 profile、文件 profile、连接页选择的共同入口。
- [x] 先运行 `cargo test -p ipkvm-desktop-iced --lib profile`，确认新增回归测试暴露并修复 Checking 状态。

### Task 2: Shared Apply Helper

**Files:** `crates/ipkvm-desktop-iced/src/app.rs`, optionally `src/profile.rs`.

- [x] 新增 `apply_loaded_profile_and_probe`：调用 `apply_profile_to_selection` 后检查 `selected_control_id`。
- [x] 仅当控制设备存在且 selection 状态为 Checking 时调用 `probe_control`，写回验证 baud 和具体失败状态。
- [x] 让 `Message::LoadRecentProfile`、文件加载和连接页加载全部调用 helper。

### Task 3: Verify and Document

- [x] 运行 `cargo test -p ipkvm-desktop-iced --lib --all-features` 与 workspace 测试。
- [x] 在 iced 长期设计文档记录 profile 应用必须收口控制探测。
- [x] 纳入联合提交并使用 `Closes #152` 收口。
