# Linux/macOS sh 验证脚本与双分辨率视频演示源设计

日期：2026-08-01

## 背景

仓库原有的本机验证入口是 PowerShell 脚本（`scripts/verify.ps1` 等），只适合 Windows。
本机是 Linux 环境，没有 PowerShell，导致提交前无法按仓库规范跑统一验证。
与此同时，RFB 传输层已经完成库级闭环，但缺少不依赖真实采集卡和真实串口的
可运行演示路径，也就缺少“真实第三方 VNC 客户端 + 动态分辨率”的验证手段。

## 目标

- 提供与 PowerShell 版本等价的 sh 验证脚本，覆盖文本编码、许可证策略、许可证来源、
  Rust 格式、全工作区测试、Clippy、Rust 文档和 Git 差异。
- 提供可重复生成的双分辨率视频演示素材，不引入视频解码依赖。
- 提供循环播放这些素材的 mock 帧源，素材切换时发布不同尺寸的 `VideoFrame`。
- 提供可运行的 `ipkvm-demo` 二进制：播放素材并暴露 RFB TCP 服务。
- 用独立的第三方 VNC 客户端（vncdotool）验证画面接收、输入注入和 `DesktopSize`
  动态分辨率切换，并把这些能力沉淀为自动化测试。

## 非目标

- 不实现真实视频采集、真实串口、CH9329 设备探测或真实硬件闭环。
- 不嵌入 noVNC 静态资源，不实现 HTTP 设置页和 `/api/*`。
- 不引入 FFmpeg/GStreamer 作为运行时依赖；ffmpeg 只用于素材生成阶段。
- 不修改 Windows 侧 PowerShell 脚本的行为，只保持两者覆盖范围一致。

## 设计决策

### 1. sh 验证脚本与 PowerShell 版本等价

新增：

- `scripts/verify.sh`：一键验证入口。
- `scripts/verify-licenses.sh`：检查当前锁定依赖图的许可证和来源。
- `scripts/test-license-policy.sh`：用临时负向夹具验证许可证策略。
- `scripts/license-policy-tools.sh`：固定 `cargo-deny 0.20.2` 版本契约的公共工具。

三个 sh 脚本与 `verify.ps1`、`verify-licenses.ps1`、`test-license-policy.ps1`、
`license-policy-tools.psm1` 一一对应。Windows 继续使用 PowerShell 版本，
Linux/macOS 使用 sh 版本；`verify.ps1` 的受检文本类型列表同步加入 `*.sh` 和 `*.py`。

### 2. Y4M 作为演示素材格式

演示素材使用 YUV4MPEG2（`.y4m`）：

- 它是 ffmpeg 可直接产出的原始视频流，不需要运行时解码库。
- 文件头包含明确的宽高、帧率和色度采样，便于 mock 解析。
- 只支持 8 位 4:2:0（`C420`、`C420jpeg`、`C420paldv`、`C420mpeg2`），
  其他色度采样在解析阶段确定性拒绝。

`scripts/fetch-demo-assets.sh` 从 test-videos.co.uk 下载 Big Buck Bunny 的
640×360 与 1280×720 片段，用 ffmpeg 转成 10fps、3 秒的 Y4M，输出到
`.cache/demo-assets/`（已加入 `.gitignore`，不进入版本库）。

### 3. YUV420 到 BGRA 转换

`ipkvm-video` 的 `mock` 功能下新增 `y4m::Y4mAsset`：

- 解析头部的 `W`、`H`、`C` 字段和 `FRAME` 帧标记。
- 按 BT.601 公式把 YUV420 平面数据转换成 `BGRA8888`，alpha 固定为 255。
- `frame_bgra(index)` 输出无行填充的像素字节，供 `VideoFrame` 直接使用。

### 4. 循环播放帧源

`ipkvm-video` 的 `mock` 功能下新增 `looping::LoopingVideoSource`：

- 按传入顺序循环播放多个 `Y4mAsset`，素材之间尺寸可以不同。
- 每个素材内部按帧率休眠推进，帧序号单调递增，时间戳使用启动后的单调计时。
- 发布路径复用 `FrameSource` 契约（`latest_frame()` + `subscribe()`），
  与现有 `MockFrameSource` 一致，下游 RFB 驱动无需改动。
- 空素材列表或零帧率在构造时拒绝。

### 5. 演示二进制

`ipkvm-headless` 新增 `demo` feature 和 `ipkvm-demo` 二进制：

- 命令行参数：`--assets <目录>`、`--tcp <端口>`、`--fps <帧率>`。
- 读取目录下所有 `*.y4m`，按文件名排序后交给 `LoopingVideoSource`。
- 组装 `RfbTcpServer`、`RfbConnectionGate`、事件通道和
  `RfbInputPump<Ch9329InputSink<FakeCommandQueue>>`，即“真实客户端 →
  RFB → 输入事件泵 → CH9329 帧队列”的完整库级闭环。
- 默认只监听 `127.0.0.1`，与项目安全基线一致。

### 6. 第三方客户端验证

`scripts/vnc-dynamic-resolution-check.py` 使用 vncdotool 1.3.0：

- 持续发送增量 `FramebufferUpdateRequest`。
- 记录每次观察到的桌面尺寸，出现至少两种尺寸即判定通过。
- 退出前显式停止 Twisted reactor，避免脚本挂起。

vncdotool 是独立于本仓库的第三方 RFB 客户端实现，用于验证“普通 VNC 客户端
能连、能看画面、能触发动态分辨率”，覆盖 README 中“第三方普通 VNC 客户端验证”
的待完成项。

## 数据流

```text
*.y4m 素材
  -> Y4mAsset（解析 + YUV420->BGRA）
  -> LoopingVideoSource（按序循环发布 VideoFrame）
  -> RfbTcpServer / RfbConnectionCore（Raw + DesktopSize）
  -> 第三方 VNC 客户端（vncdotool / 自动化测试客户端）

客户端输入（KeyEvent / PointerEvent）
  -> RfbInputPump
  -> Ch9329InputSink
  -> FakeCommandQueue（演示；真实串口后置）
```

## 错误处理

- Y4M 解析错误（缺失 magic、缺少尺寸、不支持色度、截断帧、空素材）全部返回
  类型化错误，不 panic、不跳过数据静默恢复。
- `LoopingVideoSource` 构造参数错误在创建时拒绝，运行期任务不产生可恢复错误。
- RFB 侧尺寸变化沿用既有协议行为：客户端未声明 `-223` 伪编码时按协议断连，
  已声明时发送 `DesktopSize` 后继续。
- vncdotool 验证脚本对连接断开和超时给出明确失败信息，返回非零退出码。

## 测试策略

### 单元测试

- `y4m`：头解析、多帧解析、纯 `C420` 字段、错误色度、缺失 magic、
  截断帧、黑白帧 BGRA 金样。
- `looping`：双素材循环与尺寸交替、帧序号单调、帧格式/stride/长度、
  空素材与零帧率拒绝。

### 集成测试

- `crates/ipkvm-headless/tests/rfb_dynamic_resolution.rs`：用内存构造的两个
  不同尺寸 Y4M 素材驱动真实 TCP `RfbTcpServer`，断言初始 Raw 帧、
  `DesktopSize`（编码 `-223`）和切换后的 Raw 帧。
- 测试素材每个分辨率包含 100 帧，避免 1000fps 播放时握手阶段尺寸竞态；
  断言相对初始尺寸，兼容任一素材先被客户端观察到的顺序。

### 命令级验证

```bash
cargo install --locked --version 0.20.2 cargo-deny
./scripts/verify.sh
```

### 人工验证例外

真实硬件（采集卡、CH9329）仍无法自动化，继续按阶段计划后置。vncdotool 验证
在无显示器环境可稳定运行，因此作为自动化脚本保留。

## 文档影响

- `README.md`：新增 sh 验证命令、演示素材与 `ipkvm-demo` 使用说明。
- `docs/development-guidelines.md`：验证命令同时列出 PowerShell 与 sh 版本。
- `docs/ipkvm-coarse-design.md`：阶段 0 完成记录补充 Y4M 演示源、`ipkvm-demo`
  与 vncdotool 验证；WebSocket 传输条目从待完成移到已完成。

## 关联 issue

本设计对应的工作需要补建 Gitea issue（开发任务模板）。当前 tea 登录
`local-gitea` 的 token 缺少 `read:issue` / `write:issue` 权限，issue 创建被
Gitea 拒绝；补建后应把 issue 编号回填到 PR 说明和本文件。

## 实施状态

实现已完成并全部验证通过，提交记录见功能分支；本文件作为事后补写的长期设计记录。
