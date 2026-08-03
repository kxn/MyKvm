# 交接文档：my_ipkvm iced 迁移（2026-08-03）

> 本文件是**新会话（或接手 agent）的第一份必读文件**。请先读 `AGENTS.md`（仓库自动化协作者规范），再读本文，然后按第 7 节执行。
>
> 更新（2026-08-03）：**M1–M4 已全部合入（PR #83/#84/#85/#86）**；本文件已同步，下一步是 M5（#79）。

## 1. 仓库与环境

- 仓库：`D:\Work\my_ipkvm`（Windows 11 + PowerShell，Rust 1.89+）。
- Gitea：`http://10.10.10.5:3000`，用户 `kxn`，仓库 `kxn/my_ipkvm`；命令行统一用 `tea`（已登录，登录名 `srpg`）。
- 写中文到 Gitea 前必须设置 UTF-8（见 AGENTS.md）：`$OutputEncoding`、`[Console]::Input/OutputEncoding`。
- 当前分支：`main`，与 `origin/main` 同步（HEAD `50fcf1a`），工作区干净。
- 暂存区（stash）：`stash@{0}` = 「stale egui issue69 debug work」——旧 egui 子菜单调试的未提交改动（app.rs/menus.rs），**保留可恢复，勿删**（用户未决定去留）。
- `%TEMP%\ipkvm-stale-debug-files\`：`tmck.txt`、`tsg*.txt`（旧调试输出，共 6 个），保留可恢复。
- `crates/ipkvm-desktop-iced-spike/video_1080p.stats.json`：spike 性能跑分生成的未跟踪文件，可忽略/删除。

## 2. 现状一句话

egui 桌面端仍是当前发布物；**iced 迁移已完成调研与验证（#73 关闭）、M0–M4 已全部合入（PR #80/#81/#83/#84/#85/#86）**。

## 3. 用户已确认的决策

1. 迁移框架：**iced**（弃 Tauri / 弃 egui / 弃双窗口），单原生窗口。
2. 迁移完成后**直接删除 egui 桌面端**；macOS 打包/签名/notarization **后置**。
3. 迁移单按 **M0→M5** 拆分实施；每阶段**必须新增测试**（先红后绿），禁止只靠既有测试变绿。

## 4. Gitea 单据一览

| 编号 | 内容 | 状态 |
|---|---|---|
| #73 | iced 调研 + 三个 spike 验证（结论：可行） | closed |
| #74 | 迁移 M0：脚手架（`ipkvm-desktop-iced` 壳） | closed（PR #80 合入） |
| #75 | 迁移 M1：视频链路（帧订阅/缩放/状态栏骨架） | closed（PR #83 合入） |
| #76 | 迁移 M2：自绘菜单/模态/连接页/profile UI | closed（PR #84 合入） |
| #77 | 迁移 M3：输入接线（键盘/相对鼠标/flush_pending/特殊键/粘贴） | closed（PR #85 合入） |
| #78 | 迁移 M4：主题与观感 | closed（PR #86 合入） |
| #79 | 迁移 M5：打包收尾 + 删除 egui 桌面端 | open |
| #82 | 迁移各阶段自动化测试要求（横切，M1–M5 强制） | open |
| PR #80 | M0 脚手架（含调研与 spike 验证产物） | merged |
| PR #81 | M0 测试补强 + 每阶段测试要求固化 | merged |

## 5. 已完成工作（main 提交链）

- `7a61cd0`（PR #80）：调研结论 + 迁移设计文档 + spike 验证产物（spike crate）+ `DesktopSessionController::flush_pending()` 修复，合入 main。
- `268e041`：Cargo.lock 补丁（新 crate 入 workspace）。
- `c20165b`（PR #81）：M0 测试补强（lib/bin 拆分 + headless 渲染测试）+ 设计文档「每阶段自动化测试要求」+ Raw Input 冒烟测试健壮性修复。
- `50fcf1a`：M1 实施计划文档。
- `b2bc11e`（PR #83）：M1 视频链路合入（scale/frames/video/status/app/perf + 24 项测试 + 执行记录），含 `Closes #75`。
- `bc8f902`（PR #84）：M2 菜单/模态/连接页/profile UI 合入（46 项新增测试 + 执行记录 + 观感截图），含 `Closes #76`。
- `151aa0d`（PR #85）：M3 输入接线合入（keymap/relative/platform/input/clipboard/app 接线 + 35 项新增测试 + 执行记录），含 `Closes #77`。
- `191eb1d`（PR #86）：M4 主题与观感合入（theme.rs 亮/暗 Palette、菜单/模态/状态栏/连接页样式、设置模态黑边色+暗色开关 + 8 项新增测试 + 观感截图），含 `Closes #78`。

### 关键文件索引

- 迁移设计文档（长期事实来源，含调研结论/跨平台约束/M0–M5/测试矩阵）：`docs/superpowers/specs/2026-08-03-iced-migration-design.md`
- spike 计划与实测数据：`docs/superpowers/plans/2026-08-03-iced-spike.md`
- M1/M2/M3/M4 实施计划（已执行，含执行记录）：`docs/superpowers/plans/2026-08-03-iced-migration-m{1,2,3,4}.md`
- M2 观感截图：`docs/superpowers/artifacts/m2-screenshots/m2-connection-page.png`
- M4 观感截图：`docs/superpowers/artifacts/m4-screenshots/m4-themed-connection-page.png`
- 正式迁移 crate：`crates/ipkvm-desktop-iced/`（M0 壳：lib.rs/main.rs）
- spike crate（已验证算法的事实来源，M1–M3 逐模块收编）：`crates/ipkvm-desktop-iced-spike/`

## 6. 关键技术结论（接手前必读，避免重复踩坑）

1. **版本基线**：iced `0.14.0`（crates.io 最新），features `["tokio", "image", "advanced"]`；API 未冻结，已知问题按「使用模式」规避，不依赖未发版修复。
2. **视频渲染**：`image::Handle` 必须存 state、view 只 clone（iced #3160）；用 `image` 控件而非 canvas（#3173 回归）；1080p30 上传 ~240MB/s 实测无压力；spike 帧间隔指标受 mock 源产能限制（~22.5fps），非渲染瓶颈。
3. **iced_aw 0.14.1 不可用**：嵌套子菜单树状态 bug（`Item::children()` 注释掉菜单子树 → `tree.children[1]` 越界 panic），修复只在 0.15-dev 未发版。**菜单自绘**（spike 已验证：4 顶层/深度≥3/走廊 100 次 0 误关/模态三关闭路径，全部 headless 可测）。
4. **flush-on-send 缺陷**：`DesktopSessionController` 事件通道满时残余事件只在下一次 send 补送，突发后 key-up 可能滞留。已加 `flush_pending()`，**iced UI 必须每帧/定时调用**（回归测试在 spike `tests/input_pipeline.rs`）。
5. **Windows Raw Input**（相对鼠标）：隐藏窗口 + `RIDEV_INPUTSINK` + `WM_INPUT`，实测增量 1:1、延迟 0.2–0.9ms（p95<16ms）；**HWND 必须随实例保存**（全局 OnceLock 会导致重启 stop 挂死）；冒烟测试必须容忍物理鼠标事件穿插（按序匹配注入序列），否则用户一用鼠标就挂。
6. **跨平台 6 条规则**（见设计文档第 2 节）：平台差异收口 trait；核心 crate 零平台依赖；键盘映射用 winit 统一物理键码；macOS 相对鼠标为 stub 留口；配置路径不写死平台假设；darwin check 受本机环境限制（无 macOS C 交叉编译器，`check-darwin.ps1` 报告 CC-MISSING 属预期）。
7. **测试纪律**（M0 教训）：每阶段 PR 必须含新增测试（先红后绿）；测试要经受真实环境干扰（如用户移动鼠标）；`#82` 与设计文档第 4 节是强制清单。

## 7. 下一步（新会话第一个任务）

1. 推进 **M5（#79）**：打包收尾 + 删除 egui 桌面端（窗口标题嵌入 GIT_COMMIT、Windows exe/图标/资源、macOS 打包留口 stub + 文档、全量门禁、替换发布入口、egui 端退役）。先按 superpowers 流程编写 M5 实施计划（writing-plans），再执行：
   - **Subagent-Driven（推荐）**：每任务派全新 subagent，任务间两阶段评审 → `superpowers:subagent-driven-development`。
   - **Inline**：多代理不可用时用 `superpowers:executing-plans` 批量执行 + 检查点。
2. **多代理可用性记录（2026-08-03 M1–M4 会话）**：`spawn_agent` 在 M1 Task 1 后持续返回 `unsupported call`（含最小探测），M2–M4 全程 Inline 执行；建议在 M5 前先解决 subagent 在 deepseek 上的使用问题，否则继续 Inline 并在交接记录注明。
3. 每单合入后同步 main，删除已合并分支，继续下一个。

## 8. 待用户决策/备忘

- ~~M1–M4 执行方式选择~~（2026-08-03 均因多代理不可用退化为 Inline 完成，见第 7 节）。
- stash 与 `%TEMP%` 调试文件去留（当前保留）。
- 真实硬件冒烟（BIOS 方向键/相对鼠标/特殊键/粘贴）：M3 已就绪，待用户接盒子后人工验证（成品线：CH340 + CH9329，对面可能是 ARM 或电脑，未接盒子的串口）。
- M4 视觉评审（菜单/模态/连接页/状态栏观感 vs egui）待用户确认截图。
- 相对模式光标锁定/隐藏：iced 0.14 无光标 grab API，M5 复查（若仍不可用则记录为版本限制）。
- 窗口标题嵌入 `GIT_COMMIT`（M5 验收点，历史版本验证手段）。
- 250% DPI 是用户主环境，缩放/黑边相关回归必须覆盖。

## 9. issues #99–#106 批次进展（2026-08-03）

- 计划：`docs/superpowers/plans/2026-08-03-iced-issues-99-106.md`（顺序/依赖/每单任务已定）。
- 已收口：**#99**（黑窗，PR #107）、**#103**（视频居中，PR #108）、**#100**（菜单“断开连接”，PR #109）、**#105**（菜单动作补齐，PR #110），门禁全绿，issue 已关闭并写验收评论。
- 已知限制：#100 未连接时不置灰（vendored iced_aw 0.14 菜单项无 enabled/disabled 支持，已记录，后续如需置灰补 vendored 补丁）。
- 下一步：**#102**（鼠标输入对齐：绝对坐标发送、视频区进入/退出、光标隐藏/锁定可行性验证、相对灵敏度接入）→ #104（状态栏）→ #101（对话框对齐 egui + rfd 加载 profile）→ #106（语言跟随系统 + Poppins）。
- 子代理（subagent）本批次已验证可用；派发时勿用 deepseek-v4-pro（上游不支持）。

## 10. 常用命令

```powershell
# tea 写中文前（AGENTS.md 强制）
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

tea issues list --repo kxn/my_ipkvm
tea issues 75 --repo kxn/my_ipkvm
tea pulls create --repo kxn/my_ipkvm --base main --head <branch> --title "..." --description "..."
tea pulls merge --repo kxn/my_ipkvm <PR号>

# 门禁（每单合入前必须全过）
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --all-features --no-deps

# M1 性能快速冒烟
cargo run -p ipkvm-desktop-iced --example video_1080p -- --duration 10
```
