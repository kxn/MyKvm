# my_ipkvm 桌面端 iced 迁移设计（v1 草案）

- **日期**：2026-08-03
- **关联**：Gitea `kxn/my_ipkvm#73`（调研 + spike 验收）、`docs/superpowers/plans/2026-08-03-iced-spike.md`（进度与实测数据）
- **状态**：M0–M5 已完成实现，#79 PR 待合入；本文记录迁移后的长期架构和验收口径

> 本文是长期事实来源。迁移实施期间如有偏离，必须回改本文并注明原因；实现前先开迁移实施单并引用本文。

## 1. 背景与目标

### 现状

- 迁移前桌面端 `ipkvm-desktop` 基于 egui（eframe 0.33 + wgpu）；迁移完成后该 crate 收敛为 UI 无关共享集成库，正式界面由 `ipkvm-desktop-iced` 提供。
- 会话/视频/输入核心（`ipkvm-core` / `ipkvm-session` / `ipkvm-rfb` / `ipkvm-video`）已与 UI 解耦，`DesktopSessionController` 提供平台中立接口。
- 产品要求：跨平台（至少 Windows/macOS）；现阶段「代码层面跨平台，不堵口子」，macOS 实机验证留待有机器/CI。

### 目标

1. 用 **iced** 重写桌面 UI 壳，布局与交互对齐现有 egui 桌面版（顶部菜单栏 / 主页面=连接页 / 视频区 / 底部状态栏 / 模态对话框）。
2. 复用全部核心 crate 与 `DesktopSessionController`，只替换前端渲染与事件适配层。
3. 迁移完成前旧桌面端仅在迁移窗口期与 iced 共存；完成验收后直接删除旧 UI、二进制和专属资源。
4. 单原生窗口，不做 WebView/双窗口/overlay 分层（已论证并排除）。

## 2. 本轮调研结论（2026-08-03，来源 #73）

### 框架选型结论

- **选 iced，弃 Tauri / 弃 egui / 弃双窗口**。
- 理由：单原生窗口（winit + wgpu），视频/UI/模态同层，无 WebView2、无「UI 窗口与视频窗口互抢焦点」问题；KVM 核心能力（视频帧直接进同进程渲染、键盘物理键码、相对鼠标原生采集）在 iced 上路径最短、可控性最高。
- 代价：iced 仍是 0.x、API 未冻结；默认观感需要自定义主题；菜单生态不成熟（见下）。

### 版本基线

- **iced 0.14.0**（crates.io 最新，2025-12-07），MSRV 1.88（仓库 rust-version 1.89 满足）。
- 依赖 pin：`iced = { version = "0.14", features = ["tokio", "image", "advanced"] }`（spike 已验证无版本冲突）。
- API 未冻结：0.14.0 之后 master 已有修复未发版。**已知问题必须按「使用模式」规避，不依赖未发版修复**：
  - `image::Handle` 必须存 state、view 只 clone（#3160 经验）。
  - canvas 大图缓存回归（#3173 open）：视频用 `image` 控件，不走 canvas。
  - **iced_aw 0.14.1 嵌套子菜单树状态 bug**（`Item::children()` 注释掉菜单子树 → `tree.children[1]` 越界 panic，修复仅在 0.15-dev 且依赖 iced 0.15）：**vendored 0.14.1 + 本地最小补丁**（backport 上游 main 修复到 0.14 签名），见 #95 与 `third_party/iced_aw/PATCHES.md`；spike 先复现 panic、再验证补丁后全绿。

### 跨平台约束（用户确认的 6 条规则）

1. 平台差异收口在 UI 壳下的小模块（trait 隔离），上层逻辑不感知平台；
2. 核心 crate 零平台依赖，不引入 iced/winit/windows 类型；
3. 键盘映射中，字符键使用 iced/winit 的 `modified_key` 生成字符 keysym，物理键码只作为非字符键和不可映射字符的稳定回退；macOS 特殊键映射放独立平台模块；
4. 已跨平台依赖（serialport / nokhwa / arboard / rfd）保持，不引入平台独占替代；
5. 配置路径与设备命名不写死平台假设；
6. 日常 `cargo check --target x86_64-apple-darwin` 抓 cfg 错误；实机验证后补。

### spike 验证结果汇总

| spike | 结论 | 关键证据 |
|---|---|---|
| 1 视频渲染（1080p30） | ✅ | 渲染零丢帧 2695/2695（100%）、CPU 31.7%（<40%）、内存 +11.9MB（<100MB）；平均/p95 帧间隔未达阈值，根因是 mock 推帧产能 ~22.5fps，非渲染瓶颈 |
| 2 菜单/模态 | ✅（自绘） | 4 顶层菜单、深度≥3 子菜单、i18n 切换、模态三关闭路径、hover 走廊 100 次穿越 0 误关，全部 headless 自动化（iced_test）通过 |
| 3 输入层 | ✅ | keymap 96 键、Shift/Caps Lock 字符 keysym 回归、500 键管道 0 丢失/顺序一致/不吞首键；Windows Raw Input 1:1 增量、延迟实测 0.2–0.9ms（p95 < 16ms 达标）；macOS stub 留口 |

### 本轮额外发现并修复的真实缺陷

- **flush-on-send**：`DesktopSessionController` 事件通道满时，残余事件只在下一次 `send` 时补送；突发填满通道后，最后的 key-up 可能无限期滞留 pending（目标机按键卡住）。已修复：新增 `flush_pending()`，**iced UI 必须每帧/定时调用**（egui 端此前靠每帧发事件掩盖）。
- **Raw Input 重启挂死**：窗口句柄原存全局 `OnceLock`，第二次启动的 `stop()` 把退出消息发给已销毁窗口 → `join()` 永久阻塞。已修复：HWND 随实例保存。
- **桌面端大写输入失效（#57）**：iced 端曾只用物理键码把 `KeyA..KeyZ` 固定转成小写 keysym，导致下游 RFB mapper 为了小写字符主动释放远端 Shift；Caps Lock 也因锁定键被下游忽略而无法改变大小写。已修复：字符键优先使用 `modified_key` 中已应用 Shift/Caps Lock 的单个可打印 ASCII 字符，抬起时释放按下时记录的同一 keysym。

## 3. 目标架构

### 3.1 分层

```
┌────────────────────────────────────────────────┐
│ ipkvm-desktop-iced（新 UI 壳，iced 0.14）       │
│  app/state · view/布局 · 自绘菜单/模态 · 输入适配 │
│  视频订阅 · i18n · 主题 · profile/配置 UI        │
├────────────────────────────────────────────────┤
│ DesktopSessionController（ipkvm-desktop/session）│
│  subscribe_frames() · send_key/pointer ·        │
│  flush_pending()（每帧调用）· drain_notices()   │
├────────────────────────────────────────────────┤
│ ipkvm-session / ipkvm-rfb / ipkvm-video /       │
│ ipkvm-core（不动，零平台依赖）                   │
└────────────────────────────────────────────────┘
```

- `DesktopSessionController` 中 egui 专属的 `spawn_frame_repainter(ctx: egui::Context)` 不进入 iced 端；iced 用 `subscribe_frames()` + `Subscription`。
- 视频采集后端不动：Windows DirectShow / macOS-Linux nokhwa（AVFoundation）已存在，输出统一 BGRA8888。

### 3.2 单窗口布局映射（egui → iced）

| egui 现有区域 | iced 方案 |
|---|---|
| 顶部菜单栏（文件/编辑/发送/关于 + 二级菜单） | `iced_aw` `MenuBar`/`Menu`/`Item`（vendored 补丁版，见 3.3） |
| 主页面/连接页（设备下拉、预览图、刷新、连接、保存 profile） | `Container` + `PickList`/自绘下拉 + 预览图（`image` 控件） |
| 视频区（FitWindow/ActualSize/ResizeWindowToVideo、黑边颜色） | `image` 控件 + `ContentFit`/自绘缩放（`scale.rs` 已 TDD）+ 容器背景色 |
| 底部状态栏（连接状态/键盘焦点/鼠标模式/诊断） | 固定高度 `Container` 行 |
| 设置/连接设置/保存 profile/关于模态 | 自绘模态（`ModalState` + overlay，见 3.3） |

### 3.3 菜单与模态（iced_aw vendored + 自绘模态，已验证）

- **菜单改用现成实现**（#95，用户确认）：`iced_aw 0.14.1`（MIT，`menu` feature）+ `[patch.crates-io]` 指向 `third_party/iced_aw`；本地补丁仅 backport 上游 main 的两个修复点（`open_new_menu` 先 diff 新菜单树、`Item::diff` 缺子树补空树），并把图标宏 gate 到真正使用图标的 feature。打开/关闭状态由 iced_aw 在 widget 树内管理，app 侧不再有 `MenuState`。
- 菜单项：叶子项为透明底按钮（发布 `MenuAction` 业务消息），子菜单父项为标签 + "›" 箭头，分隔线用 `rule::horizontal`；i18n 文案在 view 构建时生成。
- 已知限制：iced_aw 0.14 不支持 Esc 关闭菜单（升级 0.15 时评估）；默认浅色观感与主题化工作后置。
- 模态：`ModalState` + 遮罩 `mouse_area`（点击关闭）+ 卡片用透明 Button 命中卡片区域（卡片内点击吞掉不关闭）+ `EscClose` 包装层（Esc 关闭）；三条关闭路径（按钮/遮罩/Esc）。
- i18n：文案在 view 构建时用 `rust-i18n::t!` 生成，语言切换 = 重渲染（无宽度缓存问题）；主菜单项英文保底，语言二级菜单。
- 观感：iced_aw 默认样式（圆角面板 + 阴影 + 悬停高亮），菜单项按钮透明底 + 主题主色半透明悬停；主题化（图标、分隔线样式、暗色适配）后续单独迭代。

### 3.4 视频链路

```
frame_source.subscribe() → iced Subscription (Recipe)
  → Message::FrameReady(VideoFrame)
  → update: Handle::from_rgba（BGRA→RGBA 复用 frame.rs 转换）存入 state
  → view: Image::new(handle.clone())   // #3160 模式
```

- 帧唤醒不依赖 egui；0.14 reactive rendering 下 update 即重绘。
- 缩放模式数学复用 spike `scale.rs`（250% DPI 三模式已单测）。
- 1080p30 上传量 ~240MB/s，wgpu 实测无压力；若后续 image 控件成为瓶颈，兜底为 `shader`/custom primitive（spike 未触发，不预做）。

### 3.5 键盘

- 字符键使用 `key_event_to_keysym(Code, modified_key) -> keysym`：当 `modified_key` 是单个可打印 ASCII 字符时，直接把该字符作为 RFB/X11 keysym，保留 Shift、Caps Lock 和符号键后的字符语义。
- 非字符键、功能键、导航键、修饰键，以及暂不支持的非 ASCII/多字符输入，退回 `physical_code_to_keysym(iced::keyboard::key::Code) -> keysym`（96 键，跨平台物理键码）。
- 功能键链路端到端覆盖 F1-F20：桌面层产生 `0xffbe..0xffd1`，session 层映射为 HID usage F1-F12=`0x3a..0x45`、F13-F20=`0x68..0x6f`。
- App 记录每个物理键按下时实际发送的 keysym；抬起时释放这同一个 keysym，避免用户先松 Shift 再松字母时出现 keysym 不匹配或远端按键滞留。
- 下游 `RfbKeyboardMapper` 仍按 RFC 6143 解释字符：大小写/符号由 keysym 决定，Shift 只作为提示；特殊键（Ctrl+Alt+Del 等）走 `SpecialKey` 菜单 → 现有 `special_key_sequence` 逻辑。
- 本地组合键 Ctrl+Alt+K（退出远程捕获）、Ctrl+Alt+M（切换鼠标模式）在应用层拦截，不转发远端。
- macOS 特殊键策略（Cmd/Option 映射）放独立平台模块，迁移期实现。

### 3.6 相对鼠标

- `RelativePointerSource` trait（`start → DeltaReceiver<(i16,i16)>`，std mpsc）+ `DeltaSampler`（固定间隔采样，每周期最多 1 个事件，余数保留）。
- Windows：Raw Input（隐藏窗口 + `RIDEV_INPUTSINK` + `WM_INPUT`），已实测 1:1、p95 < 16ms。
- macOS/Linux：stub（返回「未实现」），迁移期补（winit 集成模式或 NSEvent local monitor）；不堵口子。
- 采样器与桌面 `input.rs::sample_delta` 语义一致，迁移时统一收口到共享 crate（避免双份实现漂移）。

### 3.7 剪贴板 / i18n / 主题 / 配置

- 剪贴板：arboard（已依赖，跨平台）。
- i18n：rust-i18n（现有），locales 迁移到新 crate，补齐连接页/状态栏/模态文案。
- 主题：`Theme` + 自定义 appearance（Fluent 风格参考）；黑边颜色可配置（产品需求）。
- 配置/profile：现有 `config.rs` 逻辑复用；路径改用跨平台目录库；设备显示名在 UI 层归一（如 `CH9329(COM9)`，不展示技术 ID）。
- 文件对话框：rfd（Windows 已用，macOS 放开 target feature）。

## 4. 实施阶段划分（供拆单参考）

> 顺序有依赖；每个阶段都有独立验收与回归测试。正式实施前按此拆 Gitea 单并逐单讨论。

| 阶段 | 内容 | 验收要点 |
|---|---|---|
| M0 脚手架 | 新 crate `ipkvm-desktop-iced`（或改造 desktop crate 的 UI 层），依赖 pin、workspace 接入、双端共存入口 | 空窗口可跑；`cargo test --workspace` 全绿 |
| M1 视频链路 | 帧订阅、Handle-in-state、缩放模式、黑边颜色、状态栏骨架 | spike 1 指标回归（零丢帧、CPU/内存阈值） |
| M2 菜单/模态/连接页 | 菜单改用 iced_aw（vendored 补丁，#95）、模态、连接页（设备/预览/刷新/连接）、profile 保存加载 | spike 2 全部 headless 测试移植通过 + 人工观感截图 |
| M3 输入接线 | keymap 接入、相对鼠标 trait 接 UI、`flush_pending` 定时器、特殊键、粘贴、Ctrl+Alt+K/M | spike 3 测试移植通过；真实硬件冒烟（BIOS 方向键/相对鼠标） |
| M4 主题与观感 | 菜单/模态/状态栏样式、图标、暗色适配、黑边色设置 | 截图评审（人工项） |
| M5 打包与收尾 | Windows exe/图标/资源、macOS 打包留口（stub + 文档）、全量门禁、替换发布入口、旧 UI 退役 | release 进程存活且创建顶层窗口；门禁全过；workspace 无旧 UI/spike 残留 |

### 每阶段自动化测试要求（强制）

> 教训（M0 复盘）：脚手架阶段只以「既有测试全绿」验收，新 crate 自身零新增测试——这是不合格的。**每个阶段的 PR 必须包含新增测试（先红后绿），禁止只靠既有测试不变坏当验收**；核心逻辑按 AGENTS.md TDD。

| 阶段 | 必须新增的测试 |
|---|---|
| M0（已补强） | lib/bin 拆分后可测试化；headless 渲染占位文案；窗口标题/尺寸常量断言 |
| M1 | 移植 spike 1 全部单测（scale/frames/app）+ 订阅接线测试（FrameReady→Handle 存 state、FrameClosed 停订阅）+ 状态栏状态流转测试 |
| M2 | 移植 spike 2 全部 headless 测试 + 连接页状态机测试（选设备→预览→连接→断开）+ profile 保存/加载/最近使用 + i18n 全量 |
| M3 | 移植 spike 3 全部测试 + flush_pending 定时接线测试（补送不依赖下一次输入）+ 组合键拦截测试（Ctrl+Alt+K/M 不转发远端）+ 粘贴/特殊键消息测试 |
| M4 | 主题快照测试（菜单/模态/状态栏）+ 既有交互测试全量回归 |
| M5 | 发布物冒烟脚本（启动 exe，确认进程存活和非零顶层窗口句柄）+ 删除旧 UI 后 workspace 无旧 UI/spike 残留断言 |

## 5. 风险与开放问题

| 风险/问题 | 级别 | 对策 |
|---|---|---|
| iced API 未冻结 | 中 | pin 版本；升级单独走迁移单；`iced_aw` 用 vendored 补丁版，0.15 发版后重新评估 |
| 0.14 图像已知问题 | 低 | Handle-in-state + 走 image 控件（已规避） |
| macOS 实机缺失 | 中 | 相对鼠标 stub、AVFoundation 相机未实测、notarization 未配置；代码层面留口，实机/CI 后补 |
| 键盘 macOS 特殊键 | 低 | 独立平台模块，迁移期实现 |
| 菜单观感（iced_aw 默认样式） | 低 | 默认浅色与当前默认主题一致；主题化（图标/分隔线/暗色适配）后续迭代 |
| iced_aw 未发版修复依赖 | 中 | vendored 补丁 + `PATCHES.md` 记录来源；0.15 发版后优先替换回 crates.io |
| 迁移期双端共存 | 低 | egui 端保留入口直至 iced 端达标；共享 controller 改动需两端回归 |
| `flush_pending` 依赖 | 低 | 已修复并测试；iced UI 每帧调用，回归测试覆盖 |
| 依赖许可证 | 低 | iced/iced_test/windows/iced_aw（MIT）均为宽松许可（MIT/Apache-2.0 系），按现有 deny 策略审计后引入 |

### 已确认决策（2026-08-03）

1. **迁移完成后直接删除 egui 桌面端**（`ipkvm-desktop` 的 UI 层；共享的 session/probe/config 等非 UI 逻辑先收编到合适位置再删）。
2. **macOS 打包/签名/notarization 后置**：有实际分发需求时再做；迁移期只保证代码层面留口（相对鼠标 stub、darwin 编译、AVFoundation 相机未实测）。
3. **迁移单按 M0→M5 拆分**（本设计第 4 节的粒度），按序实施。
4. **版本号暂不在 M5 重构**：当前 build.rs 继续注入 `GIT_COMMIT`，About 继续展示 short hash；后续统一版本逻辑应组合正式版本（例如 `1.0.0`）和 short hash，再单独开单实施。

## #159 边界更新（2026-08-04）

本文中“`ipkvm-desktop` 承载全部共享逻辑”的表述由 #159 细化为：纯配置、状态、探测
抽象、泛型会话控制器和帧转换位于 `ipkvm-desktop-core`；`ipkvm-desktop` 保留真实
camera/serial/clipboard production adapter，并对旧模块路径提供 re-export。iced 继续
依赖 adapter 以保留真实硬件能力，但 UI 无关单元测试和无硬件依赖门禁以 core 为准。

## #153/#154 视觉与捕获实现记录（2026-08-04）

Iced 已建立与 headless 浅色界面对齐的视觉令牌：面板最大宽度 460px、控件高度约 34px、
控件圆角 4px、面板圆角 8px；设置与连接设置采用标签/控件两列布局，窄窗口再退化为
单列。按钮、PickList、TextInput、Checkbox、菜单和弹窗使用统一浅色控件样式。

相对模式的目标端 profile 与本地捕获分离。Windows ClipCursor 只锁定视频矩形，视频矩形
尚未布局时不裁剪整个前台窗口；失焦、断开、切绝对模式和退出远程输入都会释放捕获。

## 6. 引用文档

- Gitea #73：调研结论、跨平台约束、验收标准、spike 结果
- `docs/superpowers/plans/2026-08-03-iced-spike.md`：spike 进度、实测数据、发现与修复
- `docs/dependency-license-policy.md`：新增依赖许可证审计
- `docs/superpowers/specs/2026-08-02-desktop-app-product-design.md`：现有桌面端产品设计（布局对齐基准）
