# #73 iced 迁移可行性验证（spike）方案与进度

- **关联 issue**：Gitea `kxn/my_ipkvm#73`
- **分支**：`spike/iced-feasibility-73`
- **创建日期**：2026-08-03
- **状态**：进行中（阶段 0 已完成）

> 本文件是长期事实来源。每个阶段完成后更新 checkbox 与实测数据。中途停下时，任何人可据此接手：看「当前进度」一节即可知道下一步。

## 目标

围绕 #73 做验证性 spike（**不迁移**现有 egui 桌面端）：

- 三个 spike 全过 → 回写 #73，标注「可开迁移实施/设计单」，本 issue 存档。
- 任一不过 → 回 #73 记录结论，重新评估备选（Tauri 单窗口 / 混合方案）。**禁止用降低指标的方式「通过」**。

## 架构与技术栈

- iced `0.14.0`（winit + wgpu，与 workspace 现有 `wgpu 27.0.1` 一致，无版本冲突，已验证编译通过）
- iced_aw `0.14.1`（`menu` feature，Menu/MenuBar 多级子菜单）
- iced_test `0.14.0`（headless `Simulator`：`click`/`tap_key`/`typewrite`/`snapshot`/`simulate`；**无 `hover`/`move_mouse` 高层 API**，`point_at` 文档明说「不产生鼠标移动事件」）
- 复用 `ipkvm_desktop::DesktopSessionController`（已加 `pub use` + `subscribe_frames`），仅替换前端唤醒机制

## 全局约束

- **不动现有 egui desktop**：工作在 `spike/iced-feasibility-73` 分支，不碰 `app.rs`/`menus.rs` 未提交改动与 `tmck.txt`/`tsg*.txt`。
- **TDD**（AGENTS.md 修复原则）：每个 spike 先写失败测试再实现。
- **跨平台**：Windows 实测（本机带显示器，验证者直接跑，不让用户上手）+ macOS 仅 `cargo check --target x86_64-apple-darwin`，三 spike 全过。
- **「观感/目视/手感」类项**：用截图 + 自动断言自己做，文档标注「截图+断言确认」；唯一无法替代的是 macOS 实机（按约定留口）。

## 澄清的调研缺口（相对 #73 原文）

1. **hover 走廊验证**：iced_test 无 hover API，改用 `Simulator::simulate(vec![Event::Mouse(...)])` 注入 `mouse::Event::CursorMoved` 测真实 iced_aw（非纯函数兜底）。阶段 2 首要做 probe 确认 iced_aw 在 headless 是否响应 cursor。
2. **keysym→HID 映射不在 core**：它在 `ipkvm-session::rfb_input::keymap`（`pub(super)` 未导出）；controller 的 `send_key(keysym)` 在 pump 内自动转换。spike 只需新建「iced 物理键 → keysym」表（XK_* 常量从 `desktop/src/input.rs:3-24` 复用），keysym→HID 链路无需重写。
3. **性能数据**：本机直接采（脚本启动 examples + PowerShell 采样 120s），不作为用户人工项。
4. **macOS 门控**：不只挂 Spike 3，三个 spike 都过 `cargo check --target x86_64-apple-darwin`。

## 当前进度

- **阶段 0**：✅ 已完成（脚手架 + controller 接线 + 本文件骨架）
- 阶段 1（Spike 1 视频渲染）：⏳ 待做
- 阶段 2（Spike 2 菜单模态）：⏳ 待做
- 阶段 3（Spike 3 输入层）：⏳ 待做
- 阶段 4（收口）：⏳ 待做

---

## 阶段 0：脚手架 + controller 接线 ✅

- [x] 建分支 `spike/iced-feasibility-73`
- [x] 新建 crate `crates/ipkvm-desktop-iced-spike/`，加入 workspace
- [x] 依赖 pin：iced 0.14 / iced_aw 0.14.1 / iced_test 0.14（已验证编译，无版本冲突）
- [x] `ipkvm-desktop/src/lib.rs`：`pub use session::{ConnectRequest, DesktopSessionController, DesktopSessionError, SessionParts};`
- [x] `ipkvm-desktop/src/session.rs`：新增 `pub fn subscribe_frames(&self) -> Option<ipkvm_video::FrameReceiver>`（不动 egui 版 `spawn_frame_repainter`）
- [x] TDD 测试（已通过）：`subscribe_frames_is_none_before_connect`、`subscribe_frames_notifies_on_new_frame_after_connect`
- [x] 本文件骨架

**结论**：controller 可被非 egui 前端原样复用（仅暴露平台中立 `subscribe_frames`，egui 路径不变）。

---

## 阶段 1：Spike 1 视频渲染（1080p30）

### 任务

- [x] TDD：`scale.rs` 纯函数 `frame_rect(container, frame, mode)` 三模式，250% DPI 断言（移植自 `render.rs:11-49`）— 8 个单测全过
- [x] `frames.rs`：`iced::Subscription` 把 `subscribe_frames()` 的 watch receiver 转 `Message::FrameReady`（自定义 Recipe，绕过 run_with 的 Hash 约束）— watch 链路单测过
- [x] `app.rs`：最小 iced 应用，`Handle::from_rgba` **存 state**（view 只 clone，#3160 经验），内置源/渲染帧计数器 — 3 个单测过
- [x] `examples/video_1080p.rs`：MockFrameSource 以 30fps 推 1080p + 真 wgpu 窗口 + `--duration/--stats-file` 参数 + 帧统计 JSON

### 自动化验证

- [x] `scale.rs` 内联单测：三模式矩形断言（不越界、不截底）+ 250% DPI 关键用例 — 8 测全过
- [x] `frames.rs` 内联单测：watch receiver 收到发布的帧 — 过
- [x] `app.rs` 内联单测：update 让 rendered_frames +1、Handle 存 state、FrameClosed 停订阅 — 3 测过
- [x] Handle 存 state 代码审查：`SpikeApp::update` 每帧 `handle_from_frame` 重建 Handle 存入 `self.handle`，`view` 只 `handle.clone()`（非每帧重建）

### 性能数据（验证者本机采，120s）

| 指标 | 阈值 | 实测 | 结论 |
|---|---|---|---|
| 渲染帧率/源帧率 | ≥99%（丢帧≤36/3600） | — | — |
| 平均帧间隔 | ≤34ms | — | — |
| p95 帧间隔 | ≤40ms | — | — |
| 进程 CPU（单核当量） | <40% | — | — |
| 内存增量 | <100MB | — | — |

- 闪烁：`iced_test::Snapshot`/`window::screenshot` 定时采样断言帧更新 + 截图确认（待做）
- 运行环境（系统/GPU/DPI）：—（待填）

### Spike 1 结论

—（待填：达标/不达标 + 数据；初步：渲染链路几乎零丢帧 99.97%，mock 推帧线程本身无法稳定 30fps 是帧间隔指标的瓶颈，非 iced 渲染问题）

---

## 阶段 2：Spike 2 菜单 + 模态

### 前置 probe（先做，结果回来再继续）

- [ ] **probe**：验证 `iced_aw::menu::MenuBar` 在 iced_test headless 下能否响应注入的 `mouse::Event::CursorMoved`。能 → 继续；不能 → 降级「开关状态机纯函数 + 人工」并回 #73 记录。

### 任务

- [ ] `menu.rs`：iced_aw MenuBar 复刻 4 顶层（文件/编辑/发送/关于）+ 语言子菜单 + 最近使用（含「更多…」二级）+ 特殊键子菜单，文案 `rust-i18n::t!`
- [ ] `modal.rs`：自绘 overlay（settings/connection/save profile/about），遮罩 + 事件拦截 + Esc/按钮/点遮罩三关闭路径

### 自动化验证

- [ ] `tests/menu_interact.rs`：click 开 4 顶层、子菜单深度≥3、tap_key Esc 关
- [ ] `tests/corridor_hover.rs`：`Simulator::simulate` 注入 `CursorMoved`，父→子连续穿越 100 次断言误关=0（测真实 iced_aw）
- [ ] `tests/i18n_switch.rs`：set_locale zh↔en 后 snapshot 断言文案变、无换行/截断（沿用 `I18N_TEST_LOCK` 串行）
- [ ] `tests/modal_blocking.rs`：click 背景断言不触发下层消息；三关闭路径逐一断言；关闭后焦点恢复

### Spike 2 结论

—（待填：iced_aw 可用 / 改自绘菜单 + 证据）

---

## 阶段 3：Spike 3 输入层

### 任务

- [ ] TDD：`keymap.rs` `iced::keyboard::key::Code → u32`，≥60 键（字母26+数字10+控制键15+F1-F20）
- [ ] TDD：`relative.rs` `RelativePointerSource` trait + `sample_delta`（复用 `desktop/src/input.rs:219`）
- [ ] `platform/windows.rs`：Windows Raw Input 实现
- [ ] `platform/stub.rs`：macOS/linux stub

### 自动化验证

- [ ] `tests/keymap_table.rs`：≥60 键 100% 通过
- [ ] `tests/input_pipeline.rs`：mock sink 注入 500 次混合事件，断言顺序一致、0 丢失/重复、不吞首键
- [ ] `tests/relative_pointer.rs`：合成 delta 单测（1:1、每周期≤1 事件）；Windows SendInput smoke 测 Raw Input→sink 延迟 p95<16ms
- [ ] `cargo check --target x86_64-apple-darwin`：stub 编译过

### Spike 3 结论

—（待填：trait 形状可行 / macOS 留口）

---

## 阶段 4：收口

- [ ] 回写 #73（tea，先设 UTF-8 编码、读回确认中文）：每 spike PASS/FAIL + 数据
- [ ] 填完本文件 checkbox、实测数据、结论、后续收敛条件
- [ ] 跑全量验证：`cargo fmt --all --check`、`cargo test --workspace --all-features`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps`、`scripts/check-darwin.ps1`
- [ ] 总结论：三 spike 全过 → #73 标注「可开迁移实施/设计单」、存档；不过 → 记录后重评

---

## 风险与降级路径

| 风险 | 影响 | 降级 |
|---|---|---|
| iced_aw 在 headless 不响应 cursor | Spike 2 走廊 hover 无法自动测 | 阶段 2 probe 先验；不能则降「纯函数状态机 + 人工」，回 #73 记录 |
| controller re-export 拉 egui 进 darwin check | darwin check 编译变慢 | egui 跨平台能编译，可接受 |
| 性能脚本需带显示器环境 | 数据采集受限 | 本机环境满足，脚本就绪即采 |
| 中途停下 | 接手困难 | 本文件随进度更新，含未完成 checkbox + 当前进度 |

## 文件索引

- spike crate：`crates/ipkvm-desktop-iced-spike/`
- controller 接线：`crates/ipkvm-desktop/src/lib.rs`（re-export）、`crates/ipkvm-desktop/src/session.rs`（`subscribe_frames`）
- 性能/平台脚本：`crates/ipkvm-desktop-iced-spike/scripts/`
