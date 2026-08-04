# Headless 与 Desktop crate 边界收敛设计

- **日期**：2026-08-04
- **关联 issue**：Gitea `kxn/my_ipkvm#159`
- **关联资料**：#79、#157、`docs/superpowers/specs/2026-08-03-iced-migration-design.md`、`docs/superpowers/specs/2026-08-04-iced-m5-packaging-design.md`
- **状态**：已确认方案，进入一次性实施

## 1. 目标与约束

本设计收敛 headless、browser fixture、demo、desktop shared core 和 iced 正式应用的 package、feature、provider 与测试边界。一次实施内部可以按依赖顺序分阶段验证，但阶段之间不作为交付停点；最终以一个连续的实现变更、完整验证、PR 和 issue 收口为准。

目标如下：

1. 正式 `ipkvm-headless` binary 不再依赖名为 `demo` 的 feature。
2. `ipkvm-headless` library 不携带真实串口、真实相机或测试 mock 的强制依赖。
3. browser fixture 独立为 package，依赖图中不出现 `serialport`、`ipkvm-video` 的 camera backend 或 `ipkvm-device` 的 platform provider。
4. 设备枚举通过显式 provider 注入，Web API 和 iced 连接页继续拥有真实设备能力，fixture 可以使用可控 fake provider。
5. `ipkvm-session` 只负责 `FrameSource` 会话生命周期、RFB 连接、输入泵和统计，不负责枚举或打开具体硬件。
6. `ipkvm-desktop-core` 不依赖 iced、eframe/egui、serialport 或真实相机 backend；它提供配置、状态、探测抽象、泛型桌面会话控制器和帧数据转换。
7. iced 正式应用继续直接连接真实 camera、CH9329、剪贴板和 Windows 平台能力；只把 UI 无关能力从 production adapter 中抽出。
8. 现有 CLI 参数、binary 名称、Web JSON/API、profile TOML 和 browser fixture stdout/stdin 契约保持不变。
9. 不修改 RFB/CH9329 协议、输入排序与限流、鼠标 profile 语义、会话回滚语义和旧连接不无缝迁移的既有约束。

## 2. 调研事实

### 2.1 #79 已完成的前置事实

主线 `8a3a213` 已包含 #79/M5 的收尾结果：

- workspace 不再包含 `ipkvm-desktop-iced-spike`；
- `ipkvm-desktop` 不再声明 `eframe`、`egui`、`wgpu`，只保留共享桌面集成逻辑；
- workspace 依赖树不再出现 `eframe`/`egui`；
- `ipkvm-desktop-iced/src/main.rs` 使用 Windows GUI subsystem 属性；
- `scripts/test-iced-m5-retirement.ps1` 在本次调研基线通过。

因此 #159 原 issue 中“当前仍保留 egui UI 和 spike member”的表述已经是历史背景，不能作为本次实施目标重复实现。#159 的 desktop 侧剩余工作是进一步把共享 core 与 production hardware adapter 分开。

### 2.2 Headless package 的实际边界

当前 `crates/ipkvm-headless/Cargo.toml` 把 library、正式 `ipkvm-headless` binary、`ipkvm-demo` 和 `ipkvm-browser-fixture` 放在同一个 package：

| target | required feature | 实际用途 |
|---|---|---|
| `ipkvm-headless` | `demo` | 正式 HTTP/RFB 后台进程 |
| `ipkvm-demo` | `demo` | Y4M 循环播放演示 |
| `ipkvm-browser-fixture` | `browser-fixture` | noVNC 浏览器自动化夹具 |

当前 `demo` 同时启用 `ipkvm-core/mock`、`ipkvm-core/serial`、`ipkvm-video/mock` 和 `ipkvm-video/mf`。正式后台进程依赖该 feature 才能编译，所以 production capability 被错误命名为 demo capability。

更严重的是 package 级 feature 和无条件依赖叠加形成泄漏：

- `ipkvm-headless` 无条件依赖 `ipkvm-session` 的 `serial` feature；
- `ipkvm-session` 无条件依赖 `ipkvm-video` 的 `mf` feature；
- `ipkvm-session/src/devices.rs` 直接调用 `ipkvm_video::camera` 和 `serialport`；
- `ipkvm-headless/src/web/service.rs` 的 `/api/devices` 直接调用 `ipkvm_session::devices`，无法注入 fixture provider；
- `ipkvm-headless/src/frame_source.rs` 的 `EmptyFrameSource` 为了得到 watch channel 复用 `MockFrameSource`；
- 因此 `cargo check -p ipkvm-headless --lib` 在没有 mock feature 时会因 `ipkvm_video::mock` 不可用而失败，现有 dev-dependency feature 只是在测试构建时掩盖了这个错误。

browser fixture 当前使用 `RecordingInputSink`、`MockFrameSource` 和独立 stdin/stdout 协议，但它仍通过 package 依赖图带入 `serialport` 和 Windows camera backend。`cargo tree -p ipkvm-headless --features browser-fixture --invert serialport` 的路径为：

```text
serialport
└── ipkvm-session
    └── ipkvm-headless
```

### 2.3 Session、video 和 input 的实际职责

`ipkvm-session` 的核心 API 已经是可复用的生命周期抽象：

- `ConsoleSession<S>` 持有 `Arc<dyn FrameSource>`、`S: InputSink`、`RfbConnectionGate` 和事件发布端；
- `SessionManager<S>` 负责 `Absent`/`Stopped`/`Running`、`create`、`replace_and_start`、`stop_and_destroy` 和旧泵释放屏障；
- `RfbInputPump` 负责控制者仲裁、键鼠映射、文本输入、release 和统计；
- desktop 与 headless 都通过 `SessionFactory`/组装函数决定具体帧源和 sink。

这部分不需要新的生命周期抽象。真正越界的是 `devices.rs`：设备枚举既不是 RFB 会话职责，也不是输入泵职责。

`ipkvm-video` 当前 feature 语义混杂：

- `mf` 实际表示 Windows DirectShow、Linux V4L2、macOS AVFoundation 的真实 camera backend；名称是历史遗留；
- `mock` 同时控制 `MockFrameSource`、Y4M 解析、file source 和 looping source；其中 Y4M/file/looping 是 demo 运行能力，不是测试专属能力。

`ipkvm-core/mock` 当前为空 feature，`FakeCommandQueue` 则被核心测试、headless demo 和 browser fixture 使用。feature 名称需要语义化，但兼容别名必须保留到迁移完成。

### 2.4 Desktop 与 iced 的实际职责

当前 `ipkvm-desktop` 已无 UI framework，但仍把以下职责放在一个 package：

- `config.rs`：profile、最近使用和 TOML 持久化；
- `state.rs`：设备选择和探测状态；
- `probe.rs`：`ProbeBackend` 抽象、刷新决策、CH9329 波特率探测和真实枚举 adapter；
- `session.rs`：泛型 `DesktopSessionController`，以及真实相机/串口 `production_parts`；
- `frame.rs`/`render.rs`：帧通道转换和指针几何；
- `clipboard.rs`：RGBA 数据、系统剪贴板和 JPEG 保存。

`ipkvm-desktop-iced` 还直接依赖 `ipkvm-core`、`ipkvm-session` 和 `ipkvm-video`，因为 UI、测试和 production wiring 同时使用它们。#79 已经证明“去掉 egui”可行，但没有形成一个可单独验证的无硬件 `desktop-core`。

## 3. 方案与决策

### 3.1 已排除的方案

只在现有 package 内把 `demo` 改名为 `runtime`，可以快速减少命名问题，但不能形成 binary 级边界；Cargo feature 仍然是 package 级的，`cargo test --all-features` 仍会把所有 target 的依赖合并。只把 `devices.rs` 复制到 headless 和 desktop 也会造成两个枚举实现漂移，因此不采用。

把 camera、serial、clipboard、每个 binary 的 session factory 全部拆成独立 package，边界会更硬，但会复制大量转发类型和生命周期接口，不符合当前稳定 API 的最小化原则。

### 3.2 采用的架构

采用“纯类型/provider + 独立 app package + desktop core”的分阶段混合方案：

```text
ipkvm-core                 协议、InputSink、CH9329 queue
ipkvm-video                FrameSource/VideoFrame；backend 按 feature
ipkvm-rfb                  RFB 协议类型与编码
ipkvm-session              RFB 连接、输入泵、SessionManager
        │
        ├── ipkvm-device  设备描述、错误、DeviceInventoryProvider
        │
        ├── ipkvm-headless       无硬件 Web/RFB library
        │       ├── ipkvm-headless-app       正式 ipkvm-headless binary
        │       ├── ipkvm-headless-demo      ipkvm-demo binary
        │       └── ipkvm-browser-fixture    浏览器夹具 binary
        │
        └── ipkvm-desktop-core   配置、状态、探测抽象、泛型 desktop controller
                └── ipkvm-desktop             production adapter
                        └── ipkvm-desktop-iced iced UI 与正式 binary
```

`ipkvm-device` 的核心部分不依赖真实 backend：

```rust
pub struct VideoDevice {
    pub id: String,
    pub display_name: String,
}

pub struct SerialDevice {
    pub path: String,
    pub display_name: String,
}

pub trait DeviceInventoryProvider: Send + Sync {
    fn list_video_devices(&self) -> Result<Vec<VideoDevice>, DeviceProviderError>;
    fn list_serial_devices(&self) -> Result<Vec<SerialDevice>, DeviceProviderError>;
}
```

真实 provider 在 `ipkvm-device` 的 `platform` feature 下实现，内部调用 `ipkvm-video::camera::list_cameras` 和 `serialport::available_ports`。fake provider 只实现上述纯 trait。

这个 provider 只负责“枚举并描述设备”，不负责打开相机或串口。相机/串口是独占资源，打开和关闭仍由各 app 的 `SessionFactory` 负责：

- headless app 保留当前 CLI + Web 选择到 `(FrameSource, HeadlessSink)` 的组装；
- desktop production adapter 保留当前 `production_parts`；
- `SessionManager::stop_and_destroy` 是重开独占设备前唯一的释放屏障；
- Web/iced 只传递不透明的设备 id/path，不持有硬件句柄。

### 3.3 Headless provider 注入

`HeadlessWebService` 的 `/api/devices` 改为使用构造时注入的 `Arc<dyn DeviceInventoryProvider>`。所有 JSON 字段保持：

```json
{
  "video": [{"id": "...", "display_name": "...", "kind": "video"}],
  "serial": [{"id": "...", "display_name": "...", "kind": "serial"}]
}
```

设备枚举失败仍返回 `503` 和现有 `{error, detail}` 结构。fixture 注入 deterministic provider，测试不访问主机设备。`SessionFactory` 的 `build`、会话切换、回滚和旧连接断开行为不变。

### 3.4 Desktop core 与 production adapter

`ipkvm-desktop-core` 迁入：

- `ConnectionSettings`、`Profile`、`ManualSnapshot`、`ProfileStore`；
- `DeviceOption`、`DeviceSelectionState`、探测状态和连接前波特率决策；
- `ProbeBackend` trait、`refresh_detection`、`preview_refresh_action`；
- `FrameSize`、BGRA 到 RGBA 的纯转换和必要的 JPEG 数据结构；
- 泛型 `DesktopSessionController<S, F>`、`ConnectRequest`、`SessionParts<S>` 和输入事件缓冲；
- 不依赖 UI 的错误、状态和生命周期测试。

当前 `ipkvm-desktop` 保留并实现：

- `ProductionProbeBackend`，通过 `ipkvm-device::ProductionDeviceInventoryProvider` 枚举设备；
- `production_parts`，打开真实 camera 和 `SerialCommandQueue`；
- 系统剪贴板的 `arboard` adapter；
- 对 `ipkvm_desktop::config`、`ipkvm_desktop::probe`、`ipkvm_desktop::session` 等已有路径的 re-export，减少迁移期 API 破坏。

`ipkvm-desktop-iced` 的正式构建可以依赖 production adapter，因为它确实需要真实硬件；但 core 单独构建和测试时不得出现 `iced`、`eframe`/`egui`、`serialport`、camera backend。#79 的无 egui 门禁继续保留。

## 4. Feature 与 package 设计

### 4.1 `ipkvm-video`

最终语义如下：

| feature | 内容 | 使用者 |
|---|---|---|
| `camera` | 真实平台 camera backend | headless app、desktop production |
| `assets` | Y4M、file source、looping source | headless demo、headless `--assets` |
| `test-support` | `MockFrameSource` | unit/integration test、browser fixture |
| `mf` | `camera` 的历史兼容别名 | 迁移兼容，不作为新文档推荐 |
| `mock` | `assets + test-support` 的历史兼容别名 | 迁移兼容，不作为新 target 依赖 |

真实 backend feature 只在 app/production package 中启用；`ipkvm-session` 默认依赖 `ipkvm-video` 的基础 FrameSource API。

### 4.2 `ipkvm-core`

`serial` 继续只控制 `SerialCommandQueue` 和 `serialport`。`FakeCommandQueue` 迁移到明确的 `test-support` feature；旧 `mock` 保留为兼容别名。核心协议和 `InputSink` 不受影响。

### 4.3 Headless targets

| package | target | 生产/测试依赖 |
|---|---|---|
| `ipkvm-headless` | library | axum、RFB/Web、session、纯 device trait；无 camera/serial/mock |
| `ipkvm-headless-app` | `ipkvm-headless` | `ipkvm-device/platform`、`ipkvm-core/serial`、`ipkvm-video/camera`；`--assets` 使用 `assets` |
| `ipkvm-headless-demo` | `ipkvm-demo` | `ipkvm-headless`、`ipkvm-video/assets`、`ipkvm-video/test-support`、fake input |
| `ipkvm-browser-fixture` | `ipkvm-browser-fixture` | `ipkvm-headless`、fake provider、`ipkvm-video/test-support`；无真实硬件 |

正式 binary 名称为 `ipkvm-headless`，demo 和 fixture 的 binary 名称及运行协议不改。Cargo package 名变化只影响开发构建命令，README 和脚本同步更新。

### 4.4 Desktop targets

| package | 责任 | backend |
|---|---|---|
| `ipkvm-desktop-core` | UI 无关状态、配置、探测抽象、会话控制、帧数据 | 无 UI/硬件 backend |
| `ipkvm-desktop` | Windows/平台真实组装与兼容 re-export | `ipkvm-device/platform`、camera、serialport、arboard |
| `ipkvm-desktop-iced` | iced widget、窗口、输入、主题、正式 binary | iced + desktop production adapter |

## 5. 兼容与生命周期边界

### 5.1 CLI、Web 和 profile

- `ipkvm-headless` 的 `--camera`、`--assets`、`--list-cameras`、`--serial`、`--baud`、`--tcp`、`--http`、`--fps`、`--token`、`--vnc-password` 等参数不变；
- `ipkvm-demo` 仍按文件名排序循环播放 Y4M，并保留 RFB TCP 和日志语义；
- browser fixture 继续输出 `READY`、`KEY`、`POINTER`、`RELEASE`、`CONTROLLER_RELEASED`、`STOPPED`，`STOP`/stdin EOF 语义不变；
- Web routes、HTTP 状态码、JSON 字段、`SessionSelection` 的 `video`/`serial` 字段不变；
- profile 文件路径、字段名、mouse mode 序列化、最近使用列表和损坏配置回退不变；
- `ipkvm-desktop` re-export 旧共享类型，iced 内部逐步改用 core 路径，避免 profile 文件和外部 Rust 调用同时迁移。

### 5.2 资源打开和失败回滚

资源生命周期必须遵守以下顺序：

```text
收到新选择
  -> stop_and_destroy 旧 SessionManager
  -> wait_stopped 完成输入 release
  -> drop 旧 FrameSource 和 InputSink
  -> SessionFactory 打开新 camera/serial
  -> create + start
  -> 发布新 frame source 和 event sender
```

新设备打开失败时，headless Web 继续按上一成功选择回滚；回滚失败则保持 `Absent` 和空帧源，并在现有错误 detail 中同时报告两次失败。desktop controller 继续在失败后清空事件出口、帧源和 pending events，之后可以再次 connect。

provider 枚举错误只影响设备列表，不销毁当前会话；生产 probe 仍保留当前 CH9329 GetInfo、超时、波特率验证和错误分类。

## 6. 一次性实施顺序

以下是同一实现工作的内部顺序，任何阶段完成后继续执行下一阶段，不创建需要重新确认的中间交付：

1. 新增 `ipkvm-device` 的纯类型、错误、trait 和真实/fake provider 契约测试。
2. 先写 `EmptyFrameSource` 无 mock 依赖的失败测试，再实现；拆分 `ipkvm-video` feature 并为历史 feature 加兼容别名。
3. 移除 `ipkvm-session::devices`、`serial` 和 `ipkvm-video/mf` 的强制依赖，补 session 默认 feature 编译门禁。
4. 为 `HeadlessWebService` 增加 provider 注入，迁移 Web 单测、recovery 测试和 browser fixture；保持现有 HTTP 契约。
5. 将正式 headless、demo、browser fixture 移到独立 package，迁移 `headless_process`、browser fixture 测试和 `scripts/verify-browser.*`；保留 binary 名和环境变量测试入口。
6. 新增 `ipkvm-desktop-core`，先迁移纯模块和泛型 controller 测试，再让 `ipkvm-desktop` 只保留 production adapter 和 re-export。
7. 修改 iced 的依赖与 import，保留真实相机/CH9329/剪贴板/平台输入能力，确认 core 单独不带硬件依赖。
8. 增加结构门禁和依赖报告：正式 binary 不再使用 `demo`，fixture 不含真实 provider，core 不含 UI/硬件 backend，workspace 不含 egui/spike。
9. 同步 README、`HANDOFF.md`、架构设计、许可证说明、headless Web 文档和 #157 的构建命令/体积口径。

## 7. 测试与门禁

### 7.1 TDD 测试顺序

每个可观察边界先添加能失败的测试，再修改实现：

- `cargo check -p ipkvm-headless --lib`：无 mock、无真实 backend 的 library 编译；
- `DeviceInventoryProvider` fake：成功列表、视频错误、串口错误；
- `/api/devices`：fake 数据 JSON、错误 `503` 和 detail；
- `EmptyFrameSource`：无帧、`VideoSourceKind::None`、subscriber 可以构造；
- `SessionManager`：旧资源 stop/release 完成后才允许重建；
- browser fixture：READY、noVNC HTTP、RFB/WS 输入记录、release、STOP 和 EOF；
- desktop core：profile round-trip、probe refresh、波特率决策、controller connect/stop/rollback、帧转换；
- feature/package 结构：target required-features、fixture 依赖排除、core 依赖排除。

### 7.2 最终验证命令

提交前运行并保存输出：

```powershell
cargo fmt --all --check
cargo check -p ipkvm-headless --lib
cargo check -p ipkvm-desktop-core --lib
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps
cargo metadata --format-version 1 --no-deps
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-iced-m5-retirement.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-crate-boundaries.ps1
```

Windows release 目标还要分别构建并测量：正式 headless、browser fixture、demo、iced desktop。#157 的 ThinLTO/codegen/panic 和 iced feature 收缩不在本设计中实现，但所有体积报告必须基于本设计确定的新 package/feature 目标重新生成。

### 7.3 人工验证例外

真实 camera 枚举/打开、真实 CH9329 GetInfo/输入、Windows iced 窗口、BIOS 键鼠和系统剪贴板仍需要 Windows 硬件人工验证，原因是 CI 没有稳定的独占采集卡和 CH9329 环境。步骤是：

1. 构建并启动正式 headless 和 iced release binary；
2. 用真实相机和 CH9329 完成枚举、连接、视频、键盘、绝对/相对鼠标、停止和重连；
3. 用 browser fixture 跑 noVNC、RFB/WS、断开释放、重连和 stdout 协议；
4. 确认 fixture 不访问真实硬件，iced core/production 依赖边界和 Web/profile 行为不回归。

人工验证结果写入 PR；不能把硬件未接入描述成自动化通过。

## 8. 风险、回滚与文档影响

主要风险是 package split 导致 Cargo binary 测试环境变量路径变化、feature alias 未覆盖旧命令、真实 provider 的 target-specific optional dependency 配置错误，以及旧 Rust re-export 路径失效。回滚策略是按 package graph 反向恢复：保留新纯 crate 和测试门禁，先恢复旧 re-export/构建命令，再逐步恢复 provider injection；不回退既有用户配置或生成物。

必须同步更新：

- `README.md`；
- `HANDOFF.md`；
- `docs/ipkvm-coarse-design.md`；
- `docs/superpowers/specs/2026-08-03-iced-migration-design.md`；
- `docs/superpowers/specs/2026-08-04-iced-m5-packaging-design.md`；
- `docs/superpowers/specs/2026-08-04-headless-web-ui-design.md`；
- `docs/dependency-license-policy.md`；
- `scripts/verify-browser.*` 和新增的 `scripts/test-crate-boundaries.*`；
- #157 的构建目标、依赖报告和体积测量说明。

## 9. 验收标准

- [x] `cargo check -p ipkvm-headless --lib` 默认通过，且不启用 mock、serial 或 camera backend。
- [x] 正式 `ipkvm-headless` binary 不再需要 `--features demo`；CLI 行为不变。
- [x] `ipkvm-demo`、`ipkvm-browser-fixture` 是独立 package，fixture 依赖树没有 `serialport`、真实 camera backend 或 platform provider。
- [x] `/api/devices` 使用注入 provider，真实 headless/iced 有真实列表，fixture 有 deterministic fake 列表，JSON/API 不变。
- [x] `ipkvm-session` 默认不依赖 `serialport` 或 `ipkvm-video/camera`，且会话生命周期测试全绿。
- [x] `ipkvm-desktop-core` 不依赖 UI framework、serialport 或 camera backend；`ipkvm-desktop` re-export 兼容并保留真实硬件组装。
- [x] `ipkvm-desktop-iced` 继续拥有真实硬件能力，依赖树不含 egui；workspace 不含 spike member。
- [x] profile、CLI、Web/RFB、fixture stdout/stdin 和现有 session rollback 行为保持。
- [x] 自动化验证命令全绿；真实硬件人工例外已记录在本节之后及 PR 描述中。
- [x] 设计、架构、构建命令、许可证和 #157 体积口径同步更新。

## 10. 实施验证记录

本设计已按方案 2 一次性实施，内部按依赖顺序分阶段验证，但没有中间交付停点。以下命令在 Windows worktree `D:\Work\my_ipkvm\.worktrees\issue159-design` 中通过：

- `cargo fmt --all --check`；
- `cargo test --workspace --all-features -j 1`；
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`；
- `cargo doc --workspace --no-deps`；该命令仅产生 Cargo 已知的同名 lib/bin 文档输出路径警告，退出码为 0；
- `cargo metadata --format-version 1 --no-deps`；
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-iced-m5-retirement.ps1`；
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-crate-boundaries.ps1`；
- `cargo build --release -p ipkvm-headless-app --bin ipkvm-headless`；
- `cargo build --release -p ipkvm-headless-demo --bin ipkvm-demo`；
- `cargo build --release -p ipkvm-browser-fixture --bin ipkvm-browser-fixture`；
- `cargo build --release -p ipkvm-desktop-iced --bin ipkvm-desktop-iced`。

`bash scripts/test-crate-boundaries.sh` 在当前 WSL 环境未能执行，原因是 WSL 中没有 `cargo`，不是脚本断言失败；对应 PowerShell 门禁已通过。真实 camera、CH9329、Windows 桌面交互和 BIOS 键鼠仍未在当前自动化环境中执行，必须由接入真实硬件的人工验收完成，不能将其标记为自动化通过。
