# #157 当前结构下的可执行文件体积调研

## 1. 调研结论

本次调研以合并后的 `origin/main` 提交 `f0e3892` 为基线。#79、#159、#165
合并后，原 #157 调研中的部分依赖和发布入口已经失效，不能继续按旧结构实施。

当前最值得实施的两项优化仍然成立：

1. 在 workspace 统一配置 `lto = "thin"`、`codegen-units = 1`、
   `panic = "abort"`。
2. 对 `ipkvm-desktop-iced` 收缩 iced 默认 feature，Windows 发布目标保留
   `tokio`、`wgpu`、`image-without-codecs` 和 `advanced`，但必须先解决测试
   renderer 的 feature 矩阵。

两项优化不能在没有验证的情况下直接合并为一个改动。ThinLTO 对四个目标都已
成功构建；iced feature 收缩的 release exe 已能启动，但当前桌面单元测试在
关闭 iced 默认 feature 后失败，因此应拆成独立实施单。

## 2. 当前发布结构

### 2.1 真实发布入口

| 用途 | Cargo package | binary | 真实组装位置 |
| --- | --- | --- | --- |
| Windows 桌面 GUI | `ipkvm-desktop-iced` | `ipkvm-desktop-iced` | `crates/ipkvm-desktop-iced/src/main.rs` |
| 正式 headless | `ipkvm-headless-app` | `ipkvm-headless` | `crates/ipkvm-headless-app/src/main.rs` |
| 浏览器自动化 fixture | `ipkvm-browser-fixture` | `ipkvm-browser-fixture` | `crates/ipkvm-browser-fixture/src/main.rs` |
| 无硬件 demo | `ipkvm-headless-demo` | `ipkvm-demo` | `crates/ipkvm-headless-demo/src/main.rs` |

`ipkvm-headless` 当前是库，不是正式可执行文件。旧调研中使用
`cargo build -p ipkvm-headless --features demo` 的发布口径已经过时。

### 2.2 依赖边界变化

- `ipkvm-desktop-iced` 是唯一桌面发布入口。
- `ipkvm-desktop-core` 承担配置、会话控制、帧转换等纯逻辑。
- `ipkvm-desktop` 只保留真实相机、CH9329 串口、设备探测和剪贴板等生产 adapter。
- 当前桌面依赖树不包含 `eframe` 或 `egui`；#79 已经消除了旧 egui UI 的结构性体积。
- `ipkvm-session` 的 `serial` feature 现在只是历史兼容别名，不再拉入硬件依赖。
- 正式 headless 由 `ipkvm-headless-app` 显式启用 `ipkvm-core/serial`、
  `ipkvm-device/platform` 和 `ipkvm-video/camera`。
- `ipkvm-browser-fixture` 只使用 `test-support`，不启用真实串口和相机后端。
- `ipkvm-headless-demo` 只使用模拟输入和 assets，不启用真实串口和相机后端。

因此，旧调研中“headless 公共依赖无条件带入 serial/MF”和“iced 仍通过
`ipkvm-desktop` 间接带入 egui”两条结论均已失效。

## 3. 当前 release 基线

环境：Windows x86_64 MSVC，Rust 1.89，默认 Cargo release profile，提交
`f0e3892`。

| binary | 字节数 | MiB |
| --- | ---: | ---: |
| `ipkvm-desktop-iced.exe` | 17,371,136 | 16.57 |
| `ipkvm-headless.exe` | 4,366,848 | 4.16 |
| `ipkvm-browser-fixture.exe` | 3,643,392 | 3.47 |
| `ipkvm-demo.exe` | 1,129,984 | 1.08 |

实际构建命令：

```powershell
cargo build --release -p ipkvm-desktop-iced --bin ipkvm-desktop-iced
cargo build --release -p ipkvm-headless-app --bin ipkvm-headless
cargo build --release -p ipkvm-browser-fixture --bin ipkvm-browser-fixture
cargo build --release -p ipkvm-headless-demo --bin ipkvm-demo
```

PDB 不计入发布 exe 体积，但用于诊断时当前基线分别约为：桌面 7.15 MiB、
正式 headless 2.75 MiB、fixture 2.44 MiB、demo 1.82 MiB。

## 4. Release profile 对照

在隔离 target 目录通过环境变量注入：

```powershell
$env:CARGO_PROFILE_RELEASE_LTO = "thin"
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1"
$env:CARGO_PROFILE_RELEASE_PANIC = "abort"
```

四个目标均构建成功，结果如下：

| binary | 默认 release | ThinLTO 对照 | 减少 |
| --- | ---: | ---: | ---: |
| `ipkvm-desktop-iced.exe` | 16.57 MiB | 12.01 MiB | 27.5% |
| `ipkvm-headless.exe` | 4.16 MiB | 2.62 MiB | 37.0% |
| `ipkvm-browser-fixture.exe` | 3.47 MiB | 2.30 MiB | 33.9% |
| `ipkvm-demo.exe` | 1.08 MiB | 0.60 MiB | 44.2% |

ThinLTO 对照桌面 exe 已通过现有
`scripts/verify-desktop-release.ps1` 启动检查，能创建非零顶层窗口句柄。

该配置的代价是构建时间增加，尤其是 iced；需要在 CI/本机构建流程中记录时间，
但当前收益足以作为 P0 实施候选。`strip` 未在本次新基线中单独重新测量，不能
把它当作已验证收益。

## 5. iced feature 对照

当前 manifest 使用：

```toml
iced = { version = "0.14", features = ["tokio", "image", "advanced"] }
```

iced 0.14 的默认 feature 还包括 `wgpu`、`tiny-skia`、`crisp`、`web-colors`、
`thread-pool`、`linux-theme-detection`、`x11` 和 `wayland`。临时改为：

```toml
iced = { version = "0.14", default-features = false, features = [
    "tokio",
    "wgpu",
    "image-without-codecs",
    "advanced",
] }
```

当前主线对照结果：

- 默认 feature：17,371,136 B / 16.57 MiB。
- 收缩 feature：14,585,344 B / 13.91 MiB。
- 减少 2,785,792 B，约 16.0%。
- release exe 通过桌面启动冒烟。
- `cargo test -p ipkvm-desktop-iced --lib` 在临时配置下失败。

失败位置是 `crates/ipkvm-desktop-iced/src/app.rs` 的测试渲染路径：
`iced_tiny_skia::Renderer` 不满足 `iced::advanced::image::Renderer`。这说明
生产 WGPU 路径本身可以工作，但当前 `iced_tiny_skia`/`iced_test` 测试依赖的
feature 组合需要重新设计，不能直接提交 manifest 收缩。

另外，`arboard` 仍会带入它需要的图像能力，所以 `image-without-codecs` 只
移除 iced 侧的完整图片 codec，不能按“完全没有 image codec”估算收益。

## 6. 嵌入资源与剩余候选

当前资源大小：

- `crates/ipkvm-desktop-iced/assets/fonts`：638,375 B，四个 Poppins 字体。
- iced 图标：27,804 B。
- headless 项目 Web 资源：90,871 B。
- noVNC 1.7.0 目录：641,111 B。

headless 的项目 Web 和 noVNC 总计约 0.70 MiB，确实在 exe 的只读段中，但不
是 headless 体积的主要来源。当前 `include_dir!` 嵌入整个 noVNC 目录，后续可以
按 `find_asset` 白名单裁剪未服务文件，但收益低于 release profile，且必须同步
许可证页、浏览器测试和运行时资源访问测试。

字体总量约 0.61 MiB，也不是桌面 exe 的主要来源。减少字体权重会改变 UI 视觉，
不应作为当前第一优先级。

## 7. 更新后的方案排序

### P0：统一 release profile

在 workspace `Cargo.toml` 增加 release profile：

```toml
[profile.release]
lto = "thin"
codegen-units = 1
panic = "abort"
```

实施时必须：

- 构建四个当前真实发布/验证目标。
- 运行 workspace 自动化测试。
- 运行正式 headless 的 HTTP/RFB 集成测试和 browser fixture 测试。
- 运行桌面 release 启动冒烟，并保留真实相机、CH9329 的人工验证例外。
- 记录构建时间变化以及 exe/PDB 体积。

### P1：iced feature 收缩

单独建立实现单，先处理测试 renderer 的依赖特性，再合入
`default-features = false`。Windows 目标保留 WGPU；是否保留 tiny-skia、Linux
后端和跨平台 feature，应按实际发布矩阵配置，不能只按 Windows 结果删除。

验收必须包含：桌面全量测试、release 启动、视频帧渲染、截图/剪贴板、连接页
和真实 Windows GUI 人工验证。

### P2：noVNC 资源裁剪

基于运行时白名单和浏览器测试确定最小资源集，再评估是否值得做。当前预计收益
约 0.70 MiB 上限，优先级低于 profile 和 iced feature。

### 不再作为当前方案

- 不再拆分 `ipkvm-session` 的 serial feature：该依赖边界已经完成收敛。
- 不再处理 egui/eframe 体积：#79 已经从当前发布依赖树移除。
- 不把 `strip` 作为已验证的固定收益。
- 不通过删除真实相机、CH9329 或正式 headless 的 Web 资源来换取体积。

## 8. 调研范围与未覆盖项

本次仅在 Windows x86_64 MSVC、`origin/main@f0e3892` 上测量。没有重新测量：

- Linux/macOS 的 iced 后端组合。
- `strip`、PDB 生成策略或安装包压缩后的最终分发大小。
- UPX 等外部压缩工具；这会引入签名、杀软和可调试性风险，不作为默认发布方案。
- 真实硬件输入链路；#165 的 CH9329 传输逻辑不因本次体积调研改变。

正式实施前仍需要保留完整 workspace 测试和 Windows release 启动验证，不能只
以 exe 字节数作为验收。
