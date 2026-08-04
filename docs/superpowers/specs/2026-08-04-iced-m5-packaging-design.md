# Iced M5 打包与 egui 退役设计

## 背景

迁移 M0–M4 已将 iced 桌面端的连接页、视频链路、输入接线、菜单和主题合入主分支。当前 workspace 仍同时包含：

- `ipkvm-desktop`：egui UI、Windows 资源构建逻辑，以及被 iced 复用的会话/探测/profile/剪贴板等非 UI 逻辑；
- `ipkvm-desktop-iced`：正式 iced 桌面端；
- `ipkvm-desktop-iced-spike`：迁移期间的验证 crate，正式端已经收编其功能。

现有 M5 清单把窗口标题中的 `GIT_COMMIT` 当作发布物启动验证依据。这只是构建诊断信息，不能证明 exe 真的成功启动、创建窗口或可运行。

## 目标

1. 删除 egui 桌面 UI 和其发布入口，workspace 不再依赖 `eframe`/`egui`。
2. 保留并整理 iced 仍需要的非 UI 逻辑，不因 egui 退役而丢失连接、设备探测、profile、帧转换、剪贴板和会话控制能力。
3. 删除 `ipkvm-desktop-iced-spike` workspace 成员及其源码。
4. 将 Windows exe 图标/资源嵌入正式 iced crate，保留现有 `GIT_COMMIT` 注入和 About 诊断展示，暂不设计正式版本号体系。
5. 新增真实的 Windows release 启动冒烟：启动发布 exe，确认进程持续运行并创建顶层窗口，再结束测试进程；验证不读取窗口标题或 `GIT_COMMIT`。
6. 保留 macOS 相对输入 stub 和代码层面的编译留口，并在文档中明确 macOS 打包/签名/notarization 后置。

## 非目标

- 本单不重新设计版本号。未来版本号应统一包含产品版本（例如 `1.0.0`）和 short hash，但不在 M5 中改变现有 `GIT_COMMIT` 机制。
- 本单不修改相对模式光标锁定/隐藏的行为。该行为由独立单据跟踪；M5 只保证正式 iced 发布物包含当前实现。
- 本单不承诺真实相机、CH9329 或 BIOS 操作可以在无硬件的自动化环境完成。硬件验收作为人工例外记录。

## 方案

### 1. 保留 `ipkvm-desktop` 作为 UI 无关共享库

不新建 `ipkvm-desktop-core` crate，避免在迁移收尾阶段扩大 crate 重命名和依赖重排范围。`ipkvm-desktop` 保留 iced 使用的以下模块：

- `config`：连接设置、profile 和本地持久化；
- `probe`：视频/串口设备探测和波特率处理；
- `session`：`DesktopSessionController`、生产 session factory、帧订阅和输入接线；
- `clipboard`、`frame`：剪贴板和 BGRA/RGBA/JPEG 处理；
- `state` 以及框架无关的帧尺寸类型。

删除 egui 专属的 `app`、`fonts`、`input`、`locale`、`menus`、egui 几何绘制和旧 `main.rs`。同步移除 `eframe`、`wgpu`、`rfd`、`embed-resource` 等只服务旧 UI 的依赖和资源。`session` 中只负责 eframe 重绘的入口也删除，保留 iced 使用的 `subscribe_frames`。

### 2. 正式 iced crate 接管 Windows 资源

将现有图标资源迁移到 `crates/ipkvm-desktop-iced/assets/`，由该 crate 的 `build.rs` 编译 Windows `.rc` 资源。现有 `GIT_COMMIT` 环境变量注入继续保留，作为 About/诊断信息；它不出现在发布启动冒烟的断言中。

Windows 入口继续使用 `windows_subsystem = "windows"`，确保双击 release exe 不额外打开控制台黑窗。非 Windows 目标不执行 Windows 资源编译，并继续使用现有平台 stub。

### 3. 发布物验证

新增 PowerShell 发布冒烟脚本，输入 release exe 路径或使用默认 `target/release/ipkvm-desktop-iced.exe`。脚本执行以下步骤：

1. 解析并确认 exe 文件存在；
2. 使用 `Start-Process -PassThru` 启动 exe，不传入测试专用参数；
3. 在有界超时内轮询进程状态和 `MainWindowHandle`，要求进程未退出且顶层窗口句柄非零；
4. 通过窗口关闭或进程结束清理测试进程；
5. 失败时报告进程退出信息或句柄未出现的原因。

这验证的是“发布 exe 能启动并创建真实窗口”，不是标题内容。脚本不依赖固定延时作为成功条件，只使用超时作为失败边界。

另新增 workspace 退役门禁，自动断言：

- `cargo metadata` 不再包含 `ipkvm-desktop-iced-spike`；
- workspace 依赖树不含 `eframe`/`egui`；
- 正式 iced crate 的 Windows 资源文件和无黑窗入口存在；
- 旧 egui 二进制、egui 源码和其 manifest/build 依赖已移除。

## 测试与验收

实现遵循先红后绿：先让 workspace 退役门禁和 release 启动冒烟针对当前状态失败，再删除 egui/spike、迁移资源并使其通过。

自动化验证：

- `cargo fmt --all --check`；
- `cargo test --workspace --all-features`；
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`；
- `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps`；
- workspace 退役门禁；
- Windows release exe 启动/顶层窗口句柄冒烟。

人工例外：

- 真机相机 + CH9329 + BIOS 键鼠操作需要硬件，记录验证步骤和结果；
- macOS 实机打包、签名和 notarization 后置，本机只保留 stub/代码路径核对。

## 影响范围

- workspace/Cargo.lock：移除 egui 桌面端和 spike 相关 package/dependency；
- `crates/ipkvm-desktop`：从 egui 应用库收敛为 iced 使用的共享逻辑库；
- `crates/ipkvm-desktop-iced`：成为唯一桌面发布入口并接管图标/Windows 资源；
- `scripts/`：新增退役门禁和 Windows release 启动冒烟；
- `docs/superpowers/specs/2026-08-03-iced-migration-design.md`、`HANDOFF.md`、`README.md`：同步 M5 验收标准和发布入口；
- Gitea #79/#82：PR 描述提供测试证据，#82 在 M5 验收完成后关闭。

## #159 后续边界收敛

M5 已完成 egui/spike 退役；#159 在不改变 iced 行为的前提下继续拆出
`ipkvm-desktop-core`。本设计中原先“`ipkvm-desktop` 不新建 core”的决策被 #159
设计文档 supersede：production adapter 仍叫 `ipkvm-desktop`，纯逻辑和依赖门禁改以
`ipkvm-desktop-core` 为边界。M5 的无 egui 门禁和 Windows release 启动冒烟继续有效。
