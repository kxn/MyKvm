# 交接文档：my_ipkvm iced 迁移（2026-08-04）

> 本文件是**新会话（或接手 agent）的第一份必读文件**。请先读 `AGENTS.md`（仓库自动化协作者规范），再读本文，然后按第 7 节执行。
>
> 更新（2026-08-04）：**M1–M5 已全部合入（PR #83/#84/#85/#86/#161），#82 正在依据各阶段证据收口**；正式 iced 桌面端接管发布入口，旧 UI 和迁移 spike 已退役。
>
> #159 边界更新：headless app/demo/browser fixture 已拆为独立 package，`ipkvm-device`
> 提供设备 provider，`ipkvm-desktop-core` 提供无硬件桌面逻辑；实现、验证、PR 和 issue
> 收口完成前，不要把旧的 `cargo run -p ipkvm-headless --features demo` 命令当作现行入口。

## 1. 仓库与环境

- 仓库：`D:\Work\my_ipkvm`（Windows 11 + PowerShell，Rust 1.89+）。
- Gitea：`http://10.10.10.5:3000`，用户 `kxn`，仓库 `kxn/my_ipkvm`；命令行统一用 `tea`（已登录，登录名 `srpg`）。
- 写中文到 Gitea 前必须设置 UTF-8（见 AGENTS.md）：`$OutputEncoding`、`[Console]::Input/OutputEncoding`。
- 当前正式基线为 `main`；每个非平凡改动按关联 issue 创建独立分支并通过 PR 收口。
- 暂存区（stash）：`stash@{0}` = 「stale egui issue69 debug work」——旧 egui 子菜单调试的未提交改动（app.rs/menus.rs），**保留可恢复，勿删**（用户未决定去留）。
- `%TEMP%\ipkvm-stale-debug-files\`：`tmck.txt`、`tsg*.txt`（旧调试输出，共 6 个），保留可恢复。

## 2. 现状一句话

**iced 是当前正式桌面发布入口**；`ipkvm-desktop` 只保留桌面端共享集成逻辑，旧 egui UI、二进制、专属资源和 `ipkvm-desktop-iced-spike` 已删除。迁移调研（#73）与 M0–M5 已合入（PR #80/#81/#83/#84/#85/#86/#161），#79 已完成。

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
| #79 | 迁移 M5：打包收尾 + 删除 egui 桌面端 | closed（PR #161 合入） |
| #82 | 迁移各阶段自动化测试要求（横切，M1–M5 强制） | 收口中（本 PR 合入后关闭） |
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
- `b7cc330`：删除旧 egui UI、二进制、专属资源，并将 `ipkvm-desktop` 收敛为共享库。
- `3dc76f8`：新增 Windows release 进程存活/顶层窗口句柄启动冒烟。
- `e95bce6`：新增跨平台 workspace 退役门禁。
- `05f8576`（PR #161）：M5 打包收尾合入，正式 iced 发布入口接管桌面端；M5 测试矩阵包含退役门禁和 release 启动冒烟。

### 关键文件索引

- 迁移设计文档（长期事实来源，含调研结论/跨平台约束/M0–M5/测试矩阵）：`docs/superpowers/specs/2026-08-03-iced-migration-design.md`
- spike 计划与实测数据（历史记录）：`docs/superpowers/plans/2026-08-03-iced-spike.md`
- M1/M2/M3/M4 实施计划（已执行，含执行记录）：`docs/superpowers/plans/2026-08-03-iced-migration-m{1,2,3,4}.md`
- M5 设计与实施计划：`docs/superpowers/specs/2026-08-04-iced-m5-packaging-design.md`、`docs/superpowers/plans/2026-08-04-iced-m5-packaging.md`
- M5 门禁与发布冒烟：`scripts/test-iced-m5-retirement.ps1`、`scripts/test-iced-m5-retirement.sh`、`scripts/verify-desktop-release.ps1`
- M2 观感截图：`docs/superpowers/artifacts/m2-screenshots/m2-connection-page.png`
- M4 观感截图：`docs/superpowers/artifacts/m4-screenshots/m4-themed-connection-page.png`
- 正式迁移 crate：`crates/ipkvm-desktop-iced/`（M0 壳：lib.rs/main.rs）
- 迁移 spike 已在 #79 删除；历史算法已收编到正式 iced crate，相关实测数据保留在历史计划文档中。

## 6. 关键技术结论（接手前必读，避免重复踩坑）

1. **版本基线**：iced `0.14.0`（crates.io 最新），features `["tokio", "image", "advanced"]`；API 未冻结，已知问题按「使用模式」规避，不依赖未发版修复。
2. **视频渲染**：`image::Handle` 必须存 state、view 只 clone（iced #3160）；用 `image` 控件而非 canvas（#3173 回归）；1080p30 上传 ~240MB/s 实测无压力；spike 帧间隔指标受 mock 源产能限制（~22.5fps），非渲染瓶颈。
3. **iced_aw 0.14.1 不可用**：嵌套子菜单树状态 bug（`Item::children()` 注释掉菜单子树 → `tree.children[1]` 越界 panic），修复只在 0.15-dev 未发版。**菜单自绘**（spike 已验证：4 顶层/深度≥3/走廊 100 次 0 误关/模态三关闭路径，全部 headless 可测）。
4. **flush-on-send 缺陷**：`DesktopSessionController` 事件通道满时残余事件只在下一次 send 补送，突发后 key-up 可能滞留。已加 `flush_pending()`，**iced UI 必须每帧/定时调用**；迁移期间的回归测试已收编并随正式 workspace 运行。
5. **Windows Raw Input**（相对鼠标）：隐藏窗口 + `RIDEV_INPUTSINK` + `WM_INPUT`，实测增量 1:1、延迟 0.2–0.9ms（p95<16ms）；**HWND 必须随实例保存**（全局 OnceLock 会导致重启 stop 挂死）；冒烟测试必须容忍物理鼠标事件穿插（按序匹配注入序列），否则用户一用鼠标就挂。
6. **跨平台 6 条规则**（见设计文档第 2 节）：平台差异收口 trait；核心 crate 零平台依赖；键盘映射用 winit 统一物理键码；macOS 相对鼠标为 stub 留口；配置路径不写死平台假设；darwin check 受本机环境限制（无 macOS C 交叉编译器，`check-darwin.ps1` 报告 CC-MISSING 属预期）。
7. **测试纪律**（M0 教训）：每阶段 PR 必须含新增测试（先红后绿）；测试要经受真实环境干扰（如用户移动鼠标）；`#82` 与设计文档第 4 节是强制清单。

## 7. 下一步（新会话第一个任务）

1. 对照 #82 逐阶段核对新增测试和 M5 退役门禁，完成 issue 收口。
2. 保留真实相机、CH9329、BIOS 操作和 macOS 实机打包/签名/notarization 的人工例外记录。
3. 版本号统一逻辑另开单：正式版本（例如 `1.0.0`）与 short hash 组合；本次保留 `GIT_COMMIT` 注入和 About 展示。

## 8. 待用户决策/备忘

- ~~M1–M4 执行方式选择~~（2026-08-03 均因多代理不可用退化为 Inline 完成，见第 7 节）。
- stash 与 `%TEMP%` 调试文件去留（当前保留）。
- 真实硬件冒烟（BIOS 方向键/相对鼠标/特殊键/粘贴）：M3 已就绪，待用户接盒子后人工验证（成品线：CH340 + CH9329，对面可能是 ARM 或电脑，未接盒子的串口）。
- M4 视觉评审（菜单/模态/连接页/状态栏观感 vs egui）待用户确认截图。
- 相对模式光标锁定/隐藏：属于独立单据；M5 不修改。若桌面端可实现硬锁，优先硬锁；否则保持相对模式时视频外 UI 可点击的行为约束。
- 版本号统一逻辑：后续单独设计，当前 `GIT_COMMIT` 仅作 About/诊断信息。
- 250% DPI 是用户主环境，缩放/黑边相关回归必须覆盖。

## 9. issues #99–#106 批次进展（2026-08-03）

- 计划：`docs/superpowers/plans/2026-08-03-iced-issues-99-106.md`（已更新为 5 批次队列）。
- **整队已完成（2026-08-04）**：#99/#103/#100/#105/#111/#113/#112/#101/#102/#104/#106
  全部合并关闭（PR #107–#110、#123–#127），门禁全绿，main=f6ba2a2。
- 新增 issue（#101/#102/#106 拆分 #114–#122）已并入父单关闭；#87–#90 补验收关闭。
- 已知限制：#100 未连接时不置灰（vendored iced_aw 0.14 菜单项无 enabled/disabled 支持，已记录，后续如需置灰补 vendored 补丁）。
- 后续待办：
  - 真机验证整批（黑窗、居中、断开、菜单动作、鼠标模式与光标、状态栏、对话框、中英文与 Poppins 渲染）。
  - ActualSize 绘制与 scale::frame_rect 1:1 语义预存偏差（另开单）。
  - #82（横切测试要求，本 PR 收口中）。
- 子代理（subagent）本批次已验证可用；派发时勿用 deepseek-v4-pro（上游不支持）。

## 10. headless Web 控制台（2026-08-04）

- 完成：#133（桌面位序，PR #142）、#141（后端前置：设置 API/手动停止/0x08 相对指针协议，
  PR #143）、#140（前端全量：连接页/视频页/设置/特殊键/相对模式/截图/语言/多标签，PR #144），
  main=f84b00b。
- 大设计流程：先调研后开单（AGENTS.md 已加规则）；设计/调研/计划文档在
  docs/superpowers/specs/ 与 plans/。
- 默认值（#145 已定稿）：headless 未指定 --baud 时运行时默认 9600（保险优先）；
  桌面与 headless 默认鼠标模式均为绝对（BIOS 用 Ctrl+Alt+M 切相对）。
- 待人工验收：真实相机+CH9329、pointer lock 手感、Chrome/Edge 兼容矩阵；
  运行 `my_ipkvm-headless.exe` 后浏览器开 http://127.0.0.1:6080。
- 遗留：#140 复评 2 项 Minor 建议记录在台账。

## 11. #159 crate 边界与现行入口

- 正式后台：`cargo run -p ipkvm-headless-app --bin ipkvm-headless`。
- Y4M 演示：`cargo run -p ipkvm-headless-demo --bin ipkvm-demo`。
- 浏览器夹具：`cargo build -p ipkvm-browser-fixture --bin ipkvm-browser-fixture`。
- 无硬件边界检查：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-crate-boundaries.ps1`。
- 生产体积测量按 app、demo、fixture 和 iced 四个实际 binary 分别记录，不能把 library 或旧
  `demo` feature 的构建结果当作正式后台体积。

## 12. #151/#152/#153/#154/#156 联合输入与 Iced 收口

- 共享 `MouseProfile` 已进入 `ipkvm-core`；桌面 `ConnectionSettings`、headless `WebSettings`
  以及连接页/状态栏均保留 profile identity，并兼容旧 `mouse_mode` 的 Raw 迁移。
- 加载桌面连接 profile 后，匹配的控制设备会立即完成 probe，避免 `Checking` 永久阻塞连接。
- Iced/Web 相对移动统一采用累计、33 ms 调度和控制事件前 flush；输入泵的模式变化执行
  `release_all -> set_mouse_mode`。
- Desktop Windows ClipCursor 只锁定视频 screen-space 矩形；视频矩形未布局时释放裁剪，
  不把整个前台窗口误当视频区域。Web Pointer Lock 的 selected/locked 状态只在浏览器端维护。
- Web 当前会话切换接口为 `POST /api/input/mouse-profile`，只有 sink 确认实际模式后才更新
  session selection；服务端 `/api/status.session` 返回 profile 和实际模式，不返回虚构的
  本地 capture 状态。
- 本批次保留人工验证例外：Windows DPI/窗口移动下的 ClipCursor、Chrome/Edge Pointer Lock
  及降级浏览器、真实 CH9329 和 BIOS/Windows/Ubuntu/Android/macOS 目标输入栈。
- 长期事实来源：`docs/superpowers/specs/2026-08-04-mouse-os-profile-design.md`、
  `2026-08-04-input-scheduler-order-design.md`、`2026-08-04-headless-web-ui-design.md`、
  `2026-08-03-iced-migration-design.md`；联合执行计划为
  `docs/superpowers/plans/2026-08-04-issues-151-156-execution.md`。

## 13. 常用命令

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
