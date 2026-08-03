# my_ipkvm 桌面端 iced 迁移设计（v1 草案）

- **日期**：2026-08-03
- **关联**：Gitea `kxn/my_ipkvm#73`（调研 + spike 验收）、`docs/superpowers/plans/2026-08-03-iced-spike.md`（进度与实测数据）
- **状态**：设计草案，待评审后拆迁移实施单

> 本文是长期事实来源。迁移实施期间如有偏离，必须回改本文并注明原因；实现前先开迁移实施单并引用本文。

## 1. 背景与目标

### 现状

- 桌面端 `ipkvm-desktop` 基于 egui（eframe 0.33 + wgpu），功能可用（视频、CH9329 键鼠、profile、i18n、模态对话框等），但菜单交互（宽度缓存换行、子菜单缝隙误关）、观感与框架怪癖问题反复出现。
- 会话/视频/输入核心（`ipkvm-core` / `ipkvm-session` / `ipkvm-rfb` / `ipkvm-video`）已与 UI 解耦，`DesktopSessionController` 提供平台中立接口。
- 产品要求：跨平台（至少 Windows/macOS）；现阶段「代码层面跨平台，不堵口子」，macOS 实机验证留待有机器/CI。

### 目标

1. 用 **iced** 重写桌面 UI 壳，布局与交互对齐现有 egui 桌面版（顶部菜单栏 / 主页面=连接页 / 视频区 / 底部状态栏 / 模态对话框）。
2. 复用全部核心 crate 与 `DesktopSessionController`，只替换前端渲染与事件适配层。
3. 迁移完成前 egui 桌面端保持可编译、可测试（双端共存过渡，最终以 iced 端为发布物）。
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
  - **iced_aw 0.14.1 嵌套子菜单树状态 bug**（`Item::children()` 注释掉菜单子树 → `tree.children[1]` 越界 panic，修复仅在 0.15-dev）：**弃用 iced_aw，菜单自绘**（spike 已验证）。

### 跨平台约束（用户确认的 6 条规则）

1. 平台差异收口在 UI 壳下的小模块（trait 隔离），上层逻辑不感知平台；
2. 核心 crate 零平台依赖，不引入 iced/winit/windows 类型；
3. 键盘映射用 winit 统一物理键码，macOS 特殊键映射放独立平台模块；
4. 已跨平台依赖（serialport / nokhwa / arboard / rfd）保持，不引入平台独占替代；
5. 配置路径与设备命名不写死平台假设；
6. 日常 `cargo check --target x86_64-apple-darwin` 抓 cfg 错误；实机验证后补。

### spike 验证结果汇总

| spike | 结论 | 关键证据 |
|---|---|---|
| 1 视频渲染（1080p30） | ✅ | 渲染零丢帧 2695/2695（100%）、CPU 31.7%（<40%）、内存 +11.9MB（<100MB）；平均/p95 帧间隔未达阈值，根因是 mock 推帧产能 ~22.5fps，非渲染瓶颈 |
| 2 菜单/模态 | ✅（自绘） | 4 顶层菜单、深度≥3 子菜单、i18n 切换、模态三关闭路径、hover 走廊 100 次穿越 0 误关，全部 headless 自动化（iced_test）通过 |
| 3 输入层 | ✅ | keymap 96 键、500 键管道 0 丢失/顺序一致/不吞首键；Windows Raw Input 1:1 增量、延迟实测 0.2–0.9ms（p95 < 16ms 达标）；macOS stub 留口 |

### 本轮额外发现并修复的真实缺陷

- **flush-on-send**：`DesktopSessionController` 事件通道满时，残余事件只在下一次 `send` 时补送；突发填满通道后，最后的 key-up 可能无限期滞留 pending（目标机按键卡住）。已修复：新增 `flush_pending()`，**iced UI 必须每帧/定时调用**（egui 端此前靠每帧发事件掩盖）。
- **Raw Input 重启挂死**：窗口句柄原存全局 `OnceLock`，第二次启动的 `stop()` 把退出消息发给已销毁窗口 → `join()` 永久阻塞。已修复：HWND 随实例保存。

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
| 顶部菜单栏（文件/编辑/发送/关于 + 二级菜单） | 自绘 `MenuBar` Widget + `MenuPopup` Overlay（spike 代码为基础，见 3.3） |
| 主页面/连接页（设备下拉、预览图、刷新、连接、保存 profile） | `Container` + `PickList`/自绘下拉 + 预览图（`image` 控件） |
| 视频区（FitWindow/ActualSize/ResizeWindowToVideo、黑边颜色） | `image` 控件 + `ContentFit`/自绘缩放（`scale.rs` 已 TDD）+ 容器背景色 |
| 底部状态栏（连接状态/键盘焦点/鼠标模式/诊断） | 固定高度 `Container` 行 |
| 设置/连接设置/保存 profile/关于模态 | 自绘模态（`ModalState` + overlay，见 3.3） |

### 3.3 菜单与模态（自绘，已验证）

- **状态机在 app 侧**：`MenuState { open_root, open_path }`；widget 是纯展示 + 事件转发（点击 → 消息 → app 更新状态 → 重建 view）。headless 测试按此驱动。
- 菜单：顶层 `MenuBar`（Widget）+ 弹出 `MenuPopup`（`Overlay`，绝对定位、可嵌套）。子菜单走廊连通区域 = 父菜单 bounds ∪ 子菜单 bounds ∪ 二者间水平走廊，由子菜单持父项矩形计算，光标离开才关闭。
- 模态：`ModalState` + 遮罩 `mouse_area`（点击关闭）+ 卡片用透明 Button 命中卡片区域（卡片内点击吞掉不关闭）+ `EscClose` 包装层（Esc 关闭）；三条关闭路径（按钮/遮罩/Esc）。
- i18n：文案在 view 构建时用 `rust-i18n::t!` 生成，语言切换 = 重渲染（无宽度缓存问题）；主菜单项英文保底，语言二级菜单。
- 观感：spike 版菜单是纯文本（无样式）；迁移期需主题化：选中高亮、图标/箭头、分隔线、圆角与阴影。

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

- `physical_code_to_keysym(iced::keyboard::key::Code) -> keysym`（spike keymap.rs，96 键，跨平台物理键码）→ `controller.send_key(down, keysym)`。
- 大小写/符号由修饰键状态层处理（Shift 逻辑），特殊键（Ctrl+Alt+Del 等）走 `SpecialKey` 菜单 → 现有 `special_key_sequence` 逻辑。
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
| M2 菜单/模态/连接页 | 自绘菜单移植、模态、连接页（设备/预览/刷新/连接）、profile 保存加载 | spike 2 全部 headless 测试移植通过 + 人工观感截图 |
| M3 输入接线 | keymap 接入、相对鼠标 trait 接 UI、`flush_pending` 定时器、特殊键、粘贴、Ctrl+Alt+K/M | spike 3 测试移植通过；真实硬件冒烟（BIOS 方向键/相对鼠标） |
| M4 主题与观感 | 菜单/模态/状态栏样式、图标、暗色适配、黑边色设置 | 截图评审（人工项） |
| M5 打包与收尾 | Windows exe/图标/资源、macOS 打包留口（stub + 文档）、全量门禁、替换发布入口 | 双击可跑；门禁全过；egui 端退役决策 |

## 5. 风险与开放问题

| 风险/问题 | 级别 | 对策 |
|---|---|---|
| iced API 未冻结 | 中 | pin 版本；升级单独走迁移单；`iced_aw` 不引入 |
| 0.14 图像已知问题 | 低 | Handle-in-state + 走 image 控件（已规避） |
| macOS 实机缺失 | 中 | 相对鼠标 stub、AVFoundation 相机未实测、notarization 未配置；代码层面留口，实机/CI 后补 |
| 键盘 macOS 特殊键 | 低 | 独立平台模块，迁移期实现 |
| 菜单观感（spike 版无样式） | 中 | M4 主题化；走廊/命中逻辑不动 |
| 迁移期双端共存 | 低 | egui 端保留入口直至 iced 端达标；共享 controller 改动需两端回归 |
| `flush_pending` 依赖 | 低 | 已修复并测试；iced UI 每帧调用，回归测试覆盖 |
| 依赖许可证 | 低 | iced/iced_test/windows 均为宽松许可（MIT/Apache-2.0 系），按现有 deny 策略审计后引入 |

### 待用户决策

1. 迁移完成后是否删除 egui 桌面端（还是保留一段过渡期）。
2. macOS 打包签名/notarization 的预期（Apple Developer 账号或仅内部使用）。
3. 迁移单的拆分粒度与优先级（建议按 M0→M5）。

## 6. 引用文档

- Gitea #73：调研结论、跨平台约束、验收标准、spike 结果
- `docs/superpowers/plans/2026-08-03-iced-spike.md`：spike 进度、实测数据、发现与修复
- `docs/dependency-license-policy.md`：新增依赖许可证审计
- `docs/superpowers/specs/2026-08-02-desktop-app-product-design.md`：现有桌面端产品设计（布局对齐基准）
