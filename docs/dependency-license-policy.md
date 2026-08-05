# 依赖许可证策略

本文定义 my_ipkvm 对第三方依赖的准入、自动检查和发布责任。机器检查负责阻止未经批准的 Cargo 许可证与来源，但不能代替对链接方式、静态资源和最终发布包的合规审查。

## 工具

项目固定使用 `cargo-deny 0.20.2`。开发机安装命令：

```powershell
cargo install --locked --version 0.20.2 cargo-deny
```

独立运行策略自测：

```powershell
py .\scripts\test-license-policy.py
```

独立检查当前 Cargo 依赖图：

```powershell
py .\scripts\verify-licenses.py
```

独立检查非 Cargo 网页资源：

```powershell
py .\scripts\test-web-assets.py
py .\scripts\verify-web-assets.py
```

统一验收 `cargo make quick`（快速门禁）与 `cargo make full`（全量门禁，合并前）已包含以上各项。工具缺失或版本不符时必须失败，不能自动安装或跳过。

## 许可证分级

### 全局自动允许

`deny.toml` 只全局允许以下许可证：

- MIT
- Apache-2.0
- Apache-2.0 WITH LLVM-exception
- BSD-2-Clause
- BSD-3-Clause
- ISC
- Zlib
- Unicode-3.0

允许列表按实际需要保持最小。新增依赖使用 `A OR B` 时，项目可以选择其中一个已允许许可证；使用 `A AND B` 时，所有组成许可证都必须允许或获得具体包例外。

### 按组件审查

以下许可证可以接受，但不能加入 Cargo 全局自动允许列表：

- MPL-2.0。
- LGPL-2.1-only、LGPL-2.1-or-later。
- LGPL-3.0-only、LGPL-3.0-or-later。
- BSL-1.0（源码分发需保留版权与许可证声明）。
- CC0-1.0（公有领域贡献，无义务；保留按包例外以便追踪来源）。
- 需要额外归属、源码、可替换或重新链接安排的其他许可证。

Cargo 依赖只有在完成独立 issue 和中文合规记录后，才能通过 `licenses.exceptions` 按 crate 和必要的版本范围放行。记录至少包含：

1. 依赖名称和版本范围。
2. 引入目的和不可替代原因。
3. 静态链接、动态链接、独立进程或资源嵌入方式。
4. 发布时需要附带的许可证、源码、对象文件、替换说明或其他材料。
5. 必须重新审查的升级条件。

LGPL Rust 代码不是绝对禁止，但普通 Cargo 依赖通常会静态链接。只有具体方案说明如何满足适用版本的重新链接和分发义务后才能批准。优先选择动态库或独立进程边界，不能只因为上游标注 LGPL 就直接加入白名单。

当前已批准的按包例外：

- `jpeg-encoder` 0.6.x（`deny.toml` 例外 `allow = ["MIT", "Apache-2.0", "IJG"]`）：
  1. 引入目的：`ipkvm-headless` 的 `GET /api/screenshot` 把最新 BGRA8888 帧编码为 JPEG（质量 85）。备选的 `image` 等编码器依赖树大且引入大量转码路径，纯 Rust 的 `jpeg-encoder` 是最小实现。
  2. 上游声明：0.6.1 起许可证表达式为 `(MIT OR Apache-2.0) AND IJG`；0.6.0 及更早版本未声明 IJG 部分，但其 DCT 实现同样移植自 mozjpeg/IJG jpeglib（`src/fdct.rs` 带 IJG 许可头），故按 0.6.1 的如实声明批准。
  3. 链接方式：纯 Rust 静态链接进 `ipkvm-headless` 二进制（`/api/screenshot` 处理程序直接调用）。
  4. 发布义务：发布清单需附 MIT 与 Apache-2.0 许可证文本，并因 IJG 条款在产品文档中致谢使用 IJG 代码（「Conditions of distribution and use」要求 acknowledgment）。
  5. 重新审查条件：升级到 0.7.x 或更高、或上游修改许可证表达式/源码来源时重新审查（0.7 已切换 edition 2024 与 rust-version 1.87，行为差异需另行核对）。

- `serialport` 4.9.x（`deny.toml` 例外 `allow = ["MPL-2.0"]`）：
  1. 引入目的：CH9329 串口转 USB-HID 芯片的通信后端（`ipkvm-core` 的 `serial` feature，`SerialCommandQueue` 把键鼠命令写入串口）。备选的 `serial`/`mio-serial` 维护停滞或跨平台支持弱，`serialport` 是维护活跃、跨 Windows/Linux/macOS 的事实标准。
  2. 上游声明：MPL-2.0（文件级弱 copyleft，类 LGPL 的弱传染：仅修改过的源文件需开源，链接不传染）。
  3. 链接方式：纯 Rust + 平台 FFI（Linux ioctl、Windows 注册表/SetupAPI）静态链接进 `ipkvm-core`→`ipkvm-headless`；我们不修改 serialport 源文件。
  4. 发布义务：发布清单需附 MPL-2.0 许可证文本，并在产品文档/NOTICE 中声明使用了 serialport（「copyleft」按文件级，未修改文件无额外开源义务）。
  5. 重新审查条件：升级到 5.x 或上游变更许可证时重新审查。其传递依赖（`cfg-if`、`scopeguard`、`windows-sys`、`mach2`、`nix`、`io-kit-sys`）均为 MIT/Apache，已在全局允许列表内。

- `clipboard-win` 5.4.x（`deny.toml` 例外 `allow = ["BSL-1.0"]`）：
  1. 引入目的：arboard 3.x 在 Windows 上的剪贴板后端（desktop app 的文本粘贴与截图复制）。arboard 是桌面 app 计划选定的跨平台剪贴板库，Windows 实现必然依赖 clipboard-win，无同质量替代。
  2. 上游声明：BSL-1.0（Boost 软件许可证，OSI 批准的宽松许可证，无 copyleft）。
  3. 链接方式：静态链接进 `ipkvm-desktop` 二进制。
  4. 发布义务：以源码形式分发时保留 BSL-1.0 版权声明与许可证文本；二进制分发无附加义务。仓库保留 Cargo.lock 与依赖源码即可满足。
  5. 重新审查条件：升级到 6.x 或上游变更许可证时重新审查。其依赖 `error-code` 同许可证，单独批准。

- `error-code` 3.3.x（`deny.toml` 例外 `allow = ["BSL-1.0"]`）：
  1. 引入目的：`clipboard-win` 的 Windows 错误码辅助依赖。
  2. 上游声明：BSL-1.0（同上）。
  3. 链接方式：静态链接进 `ipkvm-desktop` 二进制。
  4. 发布义务：同 `clipboard-win`（源码分发保留声明，二进制无附加义务）。
  5. 重新审查条件：升级到 4.x 或上游变更许可证时重新审查。

- `hexf-parse` 0.2.x（`deny.toml` 例外 `allow = ["CC0-1.0"]`）：
  1. 引入目的：wgpu/naga 解析 WGSL 十六进制浮点字面量的固定依赖；wgpu 是 eframe/egui 桌面 app 的 GPU 渲染后端，无法避开。
  2. 上游声明：CC0-1.0（公有领域贡献 + 兜底宽松许可，无署名、许可证保留或 copyleft 义务）。
  3. 链接方式：静态链接进 `ipkvm-desktop` 二进制。
  4. 发布义务：无（不要求署名或许可证文本；按惯例在依赖清单中注明来源）。
  5. 重新审查条件：升级到 0.3.x 或上游变更许可证时重新审查。

### 默认拒绝

以下情况默认使验证失败：

- 不在全局允许列表且没有具体包例外的许可证。
- 无法确定许可证。
- 没有基于固定许可证文件哈希澄清的自定义许可证或非 SPDX 表达式。
- GPL、AGPL、SSPL、商业专有或限制用途许可证。

默认拒绝表示必须先作项目级决策，不表示对所有使用方式作法律结论。若未来决定接受强 copyleft，必须单独评估整个项目和发布方式，不能通过普通依赖更新完成。

## 依赖来源

默认只允许 crates.io 官方注册表：

- 未知注册表失败。
- 所有 Git 来源默认失败。
- workspace 和测试使用的本地路径依赖允许。

确需 Git 依赖时必须：

1. 开 issue 说明 crates.io 版本不可用的原因。
2. 审查仓库归属、许可证和维护状态。
3. 在 `deny.toml` 中精确允许仓库 URL。
4. 在 `Cargo.toml` 中使用完整提交 `rev`，不能跟踪分支或标签。
5. 上游发布合适的 crates.io 版本后移除来源例外。

## 新增和升级流程

新增或升级第三方依赖时：

1. 在对应功能 issue 中列出直接依赖、用途和候选方案。
2. 查阅 crate 元数据、上游许可证文件和必要的分发说明。
3. 先判断是否属于全局自动允许范围。
4. 条件许可证或非 crates.io 来源必须开独立审查记录。
5. 修改依赖后运行 `py .\scripts\test-license-policy.py`。
6. 运行 `py .\scripts\verify-licenses.py` 检查实际锁定依赖图。
7. 合并前运行 `cargo make full`。

禁止通过以下方式绕过审查：

- 把条件许可证加入宽泛的全局允许列表。
- 只审查直接依赖，不检查锁定后的传递依赖。
- 用 `licenses.clarify` 随意覆盖上游元数据。
- 允许整个 Git 组织或未固定的分支。
- 在 PR 中临时关闭许可证或来源检查。

如果 crate 的许可证元数据无法解析，`licenses.clarify` 必须引用实际许可证文件及其固定哈希。升级导致文件哈希变化时必须重新审查。

## 非 Cargo 组件

Cargo 门禁不完整覆盖以下内容：

- noVNC 和其他 JavaScript、HTML、CSS、字体、图标或图片。
- Qt、FFmpeg、GStreamer 和平台媒体库等动态库。
- 系统 SDK、驱动和厂商运行库。
- 外部可执行文件、固件和随包数据。

这些组件必须在各自引入 issue 中锁定版本和来源，记录许可证文件、修改范围、链接或嵌入方式以及发布义务。

当前已知原则：

- noVNC 已固定为 `@novnc/novnc` 1.7.0、上游提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`。完整 npm 发布包嵌入 Rust 二进制并原样提供，核心文件按 MPL-2.0 分发，pako 文件保留 MIT 许可证，DES 文件保留 BSD 声明；项目页面与第三方目录严格分离。修改 noVNC 自身文件或升级版本时必须重新审查并继续提供适用源码与许可证。
- `playwright-core` 1.62.1 仅用于本地真实浏览器验收，采用 Apache-2.0，不进入生产二进制或发布物。`package-lock.json` 固定版本、npm registry 来源和 integrity。
- Qt 的 LGPL 版本可以评估动态链接分发，必须保留替换动态库的能力和相应许可证材料。PyQt 免费版是 GPL/商业双许可，不采用。
- LGPL 构建的 FFmpeg 或 GStreamer 可以评估动态链接或独立进程使用，实际构建选项和附带组件必须单独核对。
- libjpeg-turbo 可以进入候选方案，但 Rust 绑定和最终分发内容仍需在引入时审查。

## 发布责任

通过 `cargo-deny` 只证明当前 Cargo 元数据和来源符合项目规则。发布前仍需：

- 根据最终锁定版本生成第三方组件清单。
- 附带要求保留的许可证和版权文本。
- 提供适用的源码获取方式、修改文件或重新链接材料。
- 核对打包目录没有引入未登记的 DLL、静态资源和工具。
- 对桌面版和无头版分别核对实际分发内容。

`cargo-deny` 依赖 crate 元数据和可识别的许可证文件，不会穷举检查每个源码文件，也不构成法律意见。

## #159 依赖边界补充

`ipkvm-device`、`ipkvm-desktop-core` 和 `ipkvm-headless` 不新增第三方许可证类型。真实
`serialport`、平台 camera backend 和 `arboard` 只由 `ipkvm-desktop` 或
`ipkvm-headless-app` 等 production package 引入；`ipkvm-browser-fixture` 和 headless
library 的依赖树不包含这些硬件依赖。发布体积和许可证清单必须分别以
`ipkvm-headless-app`、`ipkvm-headless-demo`、`ipkvm-browser-fixture`、
`ipkvm-desktop-iced` 的实际 release 依赖树为准，不能继续使用旧的
`cargo build -p ipkvm-headless --features demo` 口径。

## 自动化证明

`scripts/test-license-policy.py` 每次生成临时 Cargo 依赖图，并证明：

- MIT 和 BSD-3-Clause 依赖可以通过。
- GPL-3.0-only 依赖产生许可证拒绝。
- 未批准且未固定 `rev` 的本地 Git 依赖同时产生来源拒绝和提交规格拒绝。

夹具只使用系统临时目录和本地 `file://` Git 仓库，不访问远端。清理前会验证路径仍位于系统临时目录，避免递归删除越界。

`scripts/test-web-assets.py` 使用隔离副本证明 noVNC 文件被篡改、缺失、额外增加，固定元数据或许可证缺失，浏览器锁文件出现未批准包、浮动版本、非 npm registry 来源或缺少 integrity 时都会失败。正常验证不下载 noVNC；只有显式运行 `scripts/update-novnc.ps1` 才访问固定 tarball 来源。
