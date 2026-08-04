# Iced M5 打包与 egui 退役实施计划

> 面向自动化执行者：按任务顺序执行，每个任务完成自己的测试循环；步骤使用复选框跟踪。实现遵循先红后绿，禁止把已有测试直接当作 M5 新增验收。

目标：完成 #79，使 ipkvm-desktop-iced 成为唯一桌面发布入口，退役 egui UI 和迁移 spike，迁移 Windows 资源，并用真实进程/窗口启动冒烟替代标题 hash 验证。

架构：保留 ipkvm-desktop 作为 iced 复用的 UI 无关共享库，只删除其 egui UI、旧二进制和 UI 资源；正式 iced crate 接管 Windows exe 资源。新增 workspace 退役断言和 release exe 启动/顶层窗口句柄冒烟。

技术栈：Rust 2024、Cargo workspace、iced 0.14、PowerShell、Windows Start-Process/MainWindowHandle、embed-resource。

## 全局约束

- 仓库内新增或修改文档使用中文；代码标识符、命令和协议字段保留原文。
- 不修改主工作区 D:\Work\my_ipkvm 的未提交文件；所有实现只在 D:\Work\my_ipkvm\.worktrees\issue79-m5。
- 保留当前 GIT_COMMIT 注入和 About 诊断展示；本计划不设计正式版本号，也不使用 hash 作为启动验收。
- 不修改相对模式光标锁定/隐藏行为；该行为由独立单据跟踪。
- M5 发布冒烟不使用固定延时作为成功条件，只使用有界超时防止失败挂死。
- 最终门禁必须运行 cargo fmt --all --check、cargo test --workspace --all-features、Clippy 和 rustdoc。

---

### 任务 1：建立 M5 退役门禁并确认红灯

文件：
- 创建：scripts/test-iced-m5-retirement.ps1、scripts/test-iced-m5-retirement.sh
- 测试：脚本对当前 workspace 的 metadata、依赖树和源文件做断言

接口：
- 输入当前仓库根目录。
- 所有 M5 退役条件满足时退出码 0；任一残留返回非 0，并指出具体残留。

步骤：
- [ ] 写失败门禁脚本。PowerShell 和 sh 版本都使用 cargo metadata --format-version 1 --no-deps 断言没有 ipkvm-desktop-iced-spike；使用 cargo tree --workspace --all-features 断言依赖树没有 eframe/egui；读取 Cargo.toml 断言 ipkvm-desktop 没有 eframe/wgpu；断言旧 src/main.rs、src/app.rs 和 spike 目录不存在；断言 iced main.rs 仍含 windows_subsystem 属性，iced assets/icon.ico 和 assets/icon.rc 存在。
- [ ] 运行确认红灯：& .\scripts\test-iced-m5-retirement.ps1。预期至少报告 spike、eframe/egui 或旧 UI 文件仍存在，证明门禁没有写成永远通过。
- [ ] 运行 sh 版本确认当前也红灯：bash ./scripts/test-iced-m5-retirement.sh。
- [x] 提交：git add scripts/test-iced-m5-retirement.ps1 scripts/test-iced-m5-retirement.sh；git commit -m "test: add iced M5 retirement gate (#79)"。

### 任务 2：建立真实 release 启动冒烟并确认验证语义

文件：
- 创建：scripts/verify-desktop-release.ps1

接口：
- -ExecutablePath 可选，缺省为 target\release\ipkvm-desktop-iced.exe。
- -StartupTimeoutSeconds 可选，默认 15，仅作为失败边界。
- 成功条件是进程未退出且 MainWindowHandle 非零；不读取窗口标题、About 文本或 GIT_COMMIT。

步骤：
- [ ] 写脚本：解析 exe 路径，使用 Start-Process -PassThru 启动，循环获取进程并刷新 MainWindowHandle，句柄非零立即成功；进程提前退出或超时则失败；finally 中清理测试进程。轮询可使用 100ms 间隔，但不得把等待时长作为成功条件。
- [ ] 实现前运行：& .\scripts\verify-desktop-release.ps1 -ExecutablePath .\target\release\ipkvm-desktop-iced.exe。预期缺少 exe 时明确报告文件不存在；脚本不得出现标题/hash断言。
- [x] 提交：git add scripts/verify-desktop-release.ps1；git commit -m "test: verify iced release process startup (#79)"。

### 任务 3：把 ipkvm-desktop 收敛为共享库

文件：
- 修改：Cargo.toml、Cargo.lock、crates/ipkvm-desktop/Cargo.toml、src/lib.rs、src/session.rs、src/render.rs
- 删除：crates/ipkvm-desktop/src/app.rs、fonts.rs、input.rs、locale.rs、menus.rs、main.rs、build.rs
- 删除：crates/ipkvm-desktop/locales/ 和旧 egui 专属资源

接口：
- 保留 ipkvm_desktop 的 ConnectRequest、DesktopSessionController、DesktopSessionError、ProductionDesktopSessionController、ProductionSessionFactory、SessionParts、FrameSize re-export。
- 保留 config、probe、clipboard、frame、session、state 公共模块。
- 生产代码和 manifest 不得再引用 eframe、egui、wgpu、旧 UI rfd 或 embed-resource。

步骤：
- [ ] 从 ipkvm-desktop/Cargo.toml 删除 eframe、wgpu、rust-i18n、Windows rfd、build-dependencies 和 bin；从 src/lib.rs 删除 UI 模块、i18n 初始化、DesktopError 和 run，只保留共享模块及 re-export。
- [ ] 删除 DesktopSessionController 的 spawn_frame_repainter(Context) 和对应测试，保留 subscribe_frames；注释改为框架无关的“前端”描述。
- [ ] 将 render.rs 简化为仅包含 FrameSize { width: u32, height: u32 }，删除 VideoViewport、eframe::Rect 和 egui 几何测试；iced 自己的 scale::FrameSize 继续负责 UI 布局。
- [x] 运行 cargo test -p ipkvm-desktop --all-features --target-dir .target-issue79，预期共享库测试通过且不恢复 egui 依赖。
- [x] 提交：git add Cargo.toml Cargo.lock crates/ipkvm-desktop；git commit -m "refactor: retire egui desktop library UI (#79)"。

### 任务 4：删除迁移 spike 并迁移 Windows 资源

文件：
- 修改：Cargo.toml、Cargo.lock、crates/ipkvm-desktop-iced/Cargo.toml、build.rs、src/lib.rs
- 创建：crates/ipkvm-desktop-iced/assets/icon.rc、assets/icon.ico
- 删除：crates/ipkvm-desktop-iced-spike/

接口：
- workspace package 列表不含 ipkvm-desktop-iced-spike。
- iced build.rs 保留 GIT_COMMIT 注入，并额外编译 assets/icon.rc；非 Windows 目标使用 manifest_optional() 不报资源错误。
- iced 窗口标题为稳定产品标题，不包含 hash；About 诊断仍沿用 env!("GIT_COMMIT")，直到未来统一版本号单独改造。

步骤：
- [ ] 运行退役门禁确认实现前仍红，不放宽断言。
- [ ] 将现有 crates/ipkvm-desktop/assets/icon.ico 复制到 iced assets，新增 icon.rc，内容为 1 ICON "icon.ico"；iced build-dependencies 增加 embed-resource = "3.0.11"；build.rs 保留 git short hash 注入并追加资源编译和 rerun-if-changed。
- [ ] 从根 Cargo.toml 删除 spike workspace member，删除整个 spike 目录，运行 cargo check --workspace --all-features --target-dir .target-issue79 更新 Cargo.lock。
- [x] 运行 & .\scripts\test-iced-m5-retirement.ps1，预期全部退役断言通过。
- [x] 提交：git add Cargo.toml Cargo.lock crates/ipkvm-desktop-iced；git rm -r crates/ipkvm-desktop-iced-spike；git commit -m "feat: make iced the only desktop workspace entry (#79)"。

### 任务 5：同步长期文档和正式运行入口

文件：
- 修改：README.md、HANDOFF.md、docs/ipkvm-coarse-design.md、docs/superpowers/specs/2026-08-03-iced-migration-design.md
- 修改：scripts/verify.ps1、scripts/verify.sh
- 接入：scripts/test-iced-m5-retirement.ps1、scripts/test-iced-m5-retirement.sh

内容：
- README 桌面运行命令改为 cargo run -p ipkvm-desktop-iced，ipkvm-desktop 改称共享桌面集成库；删除旧 egui UI、Roboto 兜底和旧资源路径说明。
- 长期迁移设计的 M5 测试矩阵改为 release exe 进程存活 + 顶层窗口句柄 + workspace 无 egui/spike 残留，并明确 GIT_COMMIT 仅用于诊断。
- HANDOFF 更新正式发布入口、M5 设计/计划路径、版本号后续工作，以及 #82 在 M5 收口后关闭。
- verify.ps1 调用 PowerShell 退役门禁，verify.sh 调用 sh 退役门禁；Windows release 启动冒烟保留为独立显式命令，不强行放进 Linux/macOS 流程。

步骤：
- [ ] 先运行 rg -n -S "cargo run -p ipkvm-desktop --|标题含 GIT_COMMIT|ipkvm-desktop-iced-spike" README.md docs HANDOFF.md scripts，确认当前运行入口和 M5 验收说明不再指向 egui 或标题 hash。
- [x] 修改文档和验证入口，运行 git diff --check，确认中文 UTF-8 无 BOM；PowerShell 文件在写入外部系统前设置 UTF-8 编码。
- [ ] 提交：git add README.md HANDOFF.md docs/ipkvm-coarse-design.md docs/superpowers/specs/2026-08-03-iced-migration-design.md scripts/verify.ps1 scripts/verify.sh；git commit -m "docs: update iced M5 release entry and acceptance (#79)"。

### 任务 6：构建发布物并执行全量验证

文件：
- 不新增代码；读取任务 1–5 的脚本和 workspace。
- 生成物：.target-issue79/release/ipkvm-desktop-iced.exe，不提交。

步骤：
- [x] 运行 cargo fmt --all --check。
- [x] 运行 cargo test --workspace --all-features --target-dir .target-issue79，预期全部通过。
- [x] 运行 cargo clippy --workspace --all-targets --all-features --target-dir .target-issue79 -- -D warnings，预期无 warning。
- [x] 设置 RUSTDOCFLAGS=-D warnings 后运行 cargo doc --workspace --all-features --no-deps --target-dir .target-issue79，结束后清理该环境变量。
- [x] 构建：cargo build --release -p ipkvm-desktop-iced --bin ipkvm-desktop-iced --target-dir .target-issue79；运行 & .\scripts\verify-desktop-release.ps1 -ExecutablePath .\.target-issue79\release\ipkvm-desktop-iced.exe。预期进程存活并在超时前出现非零顶层窗口句柄，脚本没有标题/hash断言。
- [ ] 运行 PowerShell/sh 退役门禁、对应统一脚本、git diff --check 和 git status --short；确认 .target-issue79、日志和截图不进入 Git。
- [ ] 记录人工例外：真实相机 + CH9329 + BIOS 操作受硬件条件影响；macOS 实机打包/签名/notarization 后置；Windows release 启动冒烟自动完成。

### 任务 7：提交、创建 PR 并收口关联单

文件：
- 修改：本计划执行记录，在末尾新增中文执行结果。
- Gitea：PR 描述关联 #79，并说明 #82 的 M5 测试增量。

步骤：
- [ ] 复核 git status --short、git diff --stat origin/main...HEAD、git log --oneline --decorate -8；确认主工作区未提交改动未出现在本分支。
- [ ] 提交计划执行记录：git add docs/superpowers/plans/2026-08-04-iced-m5-packaging.md；git commit -m "docs: record iced M5 verification (#79)"。
- [ ] 创建 PR，标题建议 feat(iced): complete M5 packaging and retire egui (#79)。描述包含 Closes #79、改动摘要、退役门禁、workspace 测试/Clippy/doc、Windows release 进程/窗口句柄冒烟、硬件和 macOS 人工例外、版本号后续不在本单处理。
- [ ] 只有 PR #79 合入且 M5 测试矩阵验收记录完整后，才关闭 #82；不要提前关单。

## 执行结果（2026-08-04）

- 任务 1–4 已分别提交；旧 egui UI、旧二进制、专属资源和 `ipkvm-desktop-iced-spike` 已删除，正式 iced crate 接管 Windows 图标资源。
- `cargo fmt --all --check`、`cargo check --workspace --all-features --target-dir .target-issue79`、`cargo test --workspace --all-features --target-dir .target-issue79`、`cargo clippy --workspace --all-targets --all-features --target-dir .target-issue79 -- -D warnings` 和 `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps --target-dir .target-issue79` 均以退出码 0 完成。
- PowerShell 与 Git Bash 版本的 M5 退役门禁均通过。Windows release 构建以退出码 0 完成；`verify-desktop-release.ps1` 报告进程存活并创建非零窗口句柄（`hwnd=87951858`），不读取标题或 hash。
- 统一 `scripts/verify.ps1` 在既有 noVNC 资源门禁处停止：`third_party/novnc/1.7.0/core/rfb.js` 实际 SHA-256 为 `92f3108a436f93d9aa4e8ed7c05c8205d4ad11a3af60603bd44cc86ca7b58036`，清单仍为 `1084abd7b72d79304f12364c9276672e457365f09a218a32cd7d1fe25f1f448d`。该资源和清单不属于 #79，本单不修改；应另开资源完整性单处理。
- 人工例外仍为真实相机 + CH9329 + BIOS 操作，以及 macOS 实机打包/签名/notarization；相对模式光标锁定/隐藏不在本单修改。版本号统一逻辑后续单独设计，当前保留 `GIT_COMMIT` 与 About short hash。
