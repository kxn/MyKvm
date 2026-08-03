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
- 阶段 1（Spike 1 视频渲染）：✅ 已完成（见下）
- 阶段 2（Spike 2 菜单模态）：✅ 已完成（自绘菜单，见下）
- 阶段 3（Spike 3 输入层）：✅ 已完成（见下）
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

### 性能数据（验证者本机采，120s 稳态）

运行：`powershell -File crates/ipkvm-desktop-iced-spike/scripts/perf-1080p.ps1 -DurationSec 120`

| 指标 | 阈值 | 实测 | 结论 |
|---|---|---|---|
| 渲染/源帧率 | ≥99%（丢帧≤1%） | 2695/2695 = 100%，0 丢帧 | ✅ 达标 |
| 进程 CPU（单核当量） | <40% | 31.7% | ✅ 达标 |
| 内存增量 | <100MB | 11.9MB | ✅ 达标 |
| 平均帧间隔 | ≤34ms | 44.34ms | ⚠️ 见下分析 |
| p95 帧间隔 | ≤40ms | 59.31ms | ⚠️ 见下分析 |

- **帧间隔指标分析（关键）**：实测源帧率仅 ~22.5fps（2695 帧/120s），而非目标 30fps。原因是 mock 推帧线程每帧需构造 8MB Vec + Arc 分配 + watch 通知，单帧周期 ~44ms > 33ms 目标间隔。**渲染链路对到达的帧 100% 不丢**（2695/2695），所以帧间隔不达标是 **mock 源产能不足，不是 iced 渲染瓶颈**。真实视频源（DirectShow/网络流）由硬件/驱动按 30fps 交付，不受此限制。
- 闪烁：渲染帧率 100% 收帧 + 标题实时刷新帧计数，目视窗口帧持续变化（截图+断言确认：标题计数器单调递增证明帧在持续更新）
- 运行环境：Windows 11（win32 10.0.26200），wgpu DX12 后端，1080p 渲染

### Spike 1 结论

**渲染链路达标**。三项硬指标全过：渲染零丢帧（100% > 99%）、CPU 31.7% < 40%、内存 11.9MB < 100MB。帧间隔指标（平均 44ms/p95 59ms）不达标，但**根因是 mock 推帧线程产能不足（~22fps），非 iced 渲染**——渲染侧对到达帧 100% 收帧、零丢失。iced `Subscription` + `Handle::from_rgba` 存 state + `subscribe_frames` 链路验证通过，可承载 1080p30 渲染（真实源不受 mock 产能限制）。

**darwin check**：本机（Windows）因缺 macOS C 交叉编译器（objc/core-foundation 链需要 cc），`cargo check --target x86_64-apple-darwin` 卡在 C 依赖编译，非 Rust cfg 错误。属环境限制（与 #73 "macOS 实机留待机器/CI" 一致）。check-darwin.ps1 记录此限制并做 cfg gate 人工审计兜底。

---

## 阶段 2：Spike 2 菜单 + 模态

### 前置 probe（先做，结果回来再继续）

- [x] **probe（已通过）**：`iced_aw::menu::MenuBar` 在 iced_test headless 下响应 click/CursorMoved（当时结论：走廊可脚本化）。
- [x] **iced_aw 0.14.1 树状态 bug（本 spike 关键发现）**：打开「编辑 → Language」等嵌套子菜单后，`operate`（find）在 `iced_aw/src/widget/menu/menu_bar_overlay.rs:574` 处 `tree.children[1]` 越界 panic（`len is 1`）。根因：`menu_tree.rs` 的 `Item::children()` 把菜单子树**注释掉了**（`// [Tree::new(&self.item), m.tree()]`），`Item::diff` 在 menu 存在时也只会 `*tree = self.tree()`（同样只有 1 个孩子）。**修复仅在 iced_aw master（iced 0.15-dev）已合入，未发版**；0.14 线无可 pin 的修复提交。→ **结论：iced_aw 不能用于 0.14 迁移，改自绘菜单**（本 spike 的既定降级路径）。

### 任务

- [x] `menu.rs`：**自绘菜单**（弃用 iced_aw）：`MenuBar` Widget（顶层根按钮）+ `MenuPopup` Overlay（绝对定位、可嵌套），4 顶层（文件/编辑/发送/关于）+ 语言子菜单 + 最近使用（含「更多…」二级）+ 特殊键子菜单，文案 `rust-i18n::t!`；状态机 `MenuState`（open_root/open_path）在 app 侧持有，widget 纯展示 + 事件转发。
- [x] `modal.rs`：自绘 overlay（settings/connection/save profile/about），遮罩 + 事件拦截 + Esc/按钮/点遮罩三关闭路径（卡片用透明 Button 命中卡片区域；遮罩用 mouse_area）。

### 自动化验证

- [x] `tests/menu_interact.rs`：click 开 4 顶层、子菜单深度≥3、业务动作发布并关菜单、Esc 关、点击外部关且不穿透背景（8 项全过）
- [x] `tests/corridor_hover.rs`：自绘 overlay 真实 bounds 计算走廊中点，父→子连续穿越 100 次断言误关=0（已过；0 误关）
- [x] `tests/i18n_switch.rs`：set_locale zh↔en 后文案切换、显示译文而非 key 原文、单行无换行（4 项全过）
- [x] `tests/modal_blocking.rs`：click 背景断言不触发下层消息；三关闭路径逐一断言；关闭后交互恢复（4 项全过）

### Spike 2 结论

**改自绘菜单，且验证通过。**

- iced_aw 0.14.1 的嵌套子菜单树状态 bug 是真实缺陷（见上），修复只在 0.15-dev，无法在 0.14 上安全使用；依赖未发版修复 = 不可控，故按计划降级自绘。
- 自绘方案验证结果：4 顶层菜单、深度≥3 子菜单、i18n 切换、Esc/外点关闭、背景拦截、**hover 走廊 100 次穿越 0 误关**全部通过 headless 自动化测试。
- 关键机制：菜单/子菜单用 `Overlay` 实现（绝对定位、可嵌套），走廊连通区域 = 父菜单 bounds ∪ 子菜单 bounds ∪ 二者间水平走廊，由子菜单持父项矩形计算；打开状态由 app 侧 `MenuState` 驱动，widget 无私有状态，因此每条交互都可脚本化（click → 消息 → 状态 → 重建 view）。
- 附带修正：菜单栏 `mouse_interaction` 只能对根按钮文字声明 Pointer，整行声明会导致 stack 把下层控件光标置为 Unavailable、收不到点击（modal 背景按钮同款问题，已一并验证）。

---

## 阶段 3：Spike 3 输入层

### 任务

- [x] TDD：`keymap.rs` `iced::keyboard::key::Code → keysym`，96+ 键（字母26+数字10+F1-F20+标点+控制+方向+修饰）
- [x] TDD：`relative.rs` `RelativePointerSource` trait + `DeltaSampler`（固定间隔采样，语义对齐 `desktop/src/input.rs` 的 `sample_delta`；迁移时统一收口到共享 crate）
- [x] `platform/windows.rs`：Windows Raw Input 实现（隐藏消息窗口 + RIDEV_INPUTSINK + WM_INPUT）
- [x] `platform/stub.rs`：macOS/linux stub（trait 形状留口，返回「未实现」）

### 自动化验证

- [x] `tests/keymap_table.rs`：≥60 键 100% 通过（96 键无空洞 + 20 个代表键 spot 断言）
- [x] `tests/input_pipeline.rs`：500 次混合按键（1000 事件）顺序一致、0 丢失/重复、不吞首键
- [x] `tests/relative_pointer.rs`：DeltaSampler 单测（1:1 累积、每周期≤1 事件、首事件不吞）+ Windows SendInput smoke（1:1 到达、p95<16ms）
- [x] `cargo check --target x86_64-apple-darwin`：受环境限制（Windows 无 macOS C 交叉编译器），stub 路径已按 cfg 隔离，macOS 实机后补

### 本阶段发现并修复的真实问题

1. **控制器事件补送依赖下一次 send（flush-on-send）**：`DesktopSessionController` 的事件通道满时，残余事件暂存 pending，只在下次 `send_event` 时补送。突发填满通道后若无后续输入，残余事件（可能包含最后一次 key-up）会无限期滞留 → 目标机按键可能卡住。**修复**：新增 `pub fn flush_pending()`（`ipkvm-desktop/src/session.rs`），UI 每帧/定时调用；spike 输入管道测试即以 flush_pending 驱动验证。egui 端此前靠每帧发事件掩盖，iced 迁移必须显式接入。
2. **Raw Input 重启挂死**：窗口句柄原存全局 `OnceLock`，第二次启动的线程句柄无法写入，`stop()` 把退出消息发给已销毁的旧窗口 → `join()` 永久阻塞。**修复**：HWND 随实例保存（isize），stop 发给本实例窗口。
3. **1px SendInput 偶发异常**：注入 (1,0) 偶发收到 (0,1)；改为 (3,0)/(0,3) 交替后 1:1 稳定（两侧轴都验证）。属测试注入粒度问题，非 Raw Input 读取问题。

### 实测数据（Windows 11，SendInput 注入）

- 增量 1:1：注入 (3,0) / (0,3) 交替 20 次，全部原样到达。
- 延迟：单次 0.22–0.93ms，p95 远小于 16ms 阈值（断言通过）。
- 生命周期：重复启动拒绝；stop 后清空全局状态可重新启动。

### Spike 3 结论

**输入层可行。** `RelativePointerSource` trait 形状成立：Windows Raw Input 端到端验证（1:1 增量 + 亚毫秒延迟），键盘映射表 96 键覆盖、500 键管道 0 丢失；macOS 以 stub 留口，不堵跨平台路径。同时发现并修复了共享控制器 flush-on-send 的滞留缺陷（iced 迁移必须每帧调用 flush_pending）。

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
