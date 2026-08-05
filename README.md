# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程已完成 CH9329 协议与输入核心、传输无关的 RFB 3.8 协议核心；连接驱动器、`RfbServerEvent` 事件模型、`RfbConnectionGate` 仲裁、en-US 键盘和绝对指针映射、输入事件泵等会话核心已在 `ipkvm-session`。RFB 已有 TCP 与 WebSocket 两个库级传输层：`RfbTcpServer` 和可组合的 axum `/rfb` `RfbWebSocketService`。两者共用 `ipkvm-session` 的连接驱动器与全局 `RfbConnectionGate`；生产组装必须向两个服务显式传入同一个连接闸门。

`ipkvm-headless` 已提供可供生产组装复用的嵌入式 Web 服务。它内置项目中文控制台页面和固定到 noVNC 1.7.0 提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e` 的完整 npm 发布资源，并通过同源 `/rfb` 建立连接。真实 Chrome 自动化已经证明模拟帧像素、桌面与窄视口等比缩放、键盘 HID、缩放后的绝对指针坐标、按键顺序、断开释放和重连全部穿过 noVNC、RFB 服务与 `RfbInputPump` 到达记录型 `InputSink`。

当前正式 `ipkvm-headless` 二进制已能作为可运行后台进程提供完整的 RFB TCP（5900）+ noVNC 网页（6080）双传输服务。CLI/TOML 配置提供启动设备：`--camera` 打开 Windows 相机（按 id 或显示名，DirectShow 后端，含 OBS 虚拟摄像头）、`--assets` 使用 Y4M 文件伪设备；未指定视频参数时启动空会话，由网页连接页选择设备后再创建，`--list-cameras` 只枚举设备并退出。HTTP 管理 API 已支持运行时枚举设备和按 `video`/`serial` 重启会话；内部采用“停旧并释放旧帧源/串口、组装新帧源/串口、启动新输入泵”的会话级切换模型，不承诺旧 RFB 连接无缝迁移。键鼠注入可通过 `--serial <路径>` 打开真实 CH9329 串口（默认 9600 8N1，`--baud <速率>` 可调），未指定时键鼠事件进入模拟串口队列后被丢弃。最小鉴权已实现：`--token` 管 HTTP/WS 凭证、`--vnc-password` 管 RFB VNC 密码挑战，未配置对应凭证时默认仅本机可访问（见「运行无头后台进程」）。TLS 尚未实现。

## 当前模块

- `ipkvm-core`：CH9329 命令帧和应答解析、串口字节流增量解帧、HID 报告、6KRO 键盘状态、原子键盘和指针批次、绝对和相对鼠标状态、有序命令批次及模拟队列；`serial` 功能下提供真实串口命令队列 `SerialCommandQueue`（9600 8N1，跨平台 COMx / ttyUSBn，帧间延时防丢帧）。
- `ipkvm-video`：共享视频帧流与 `source_info` 元数据；`camera` 提供真实平台相机后端，`assets` 提供 Y4M/file/looping 素材源，`test-support` 提供 mock 帧源，`mf`/`mock` 仅为历史兼容别名。
- `ipkvm-device`：无硬件设备描述和 `DeviceInventoryProvider` 注入契约；真实 app 使用 `platform` provider，测试和 browser fixture 使用静态 provider。
- `ipkvm-session`：真实会话核心——连接驱动与事件模型（`RfbConnectionGate` 仲裁）、输入泵与映射器、`ConsoleSession` 组装与 `SessionManager` 生命周期管理、会话状态统计；不负责设备枚举或打开硬件。
- `ipkvm-rfb`：传输无关的 RFB 3.8 `None` 握手、客户端消息增量解码、真彩像素转换、`Raw` 更新、`DesktopSize` 和指针输入坐标时期。
- `ipkvm-desktop-core`：无 UI、无真实硬件的桌面配置、设备选择状态、探测抽象、会话控制器和帧转换。
- `ipkvm-desktop`：桌面 production adapter（真实相机、CH9329、设备 provider、系统剪贴板），并保留旧共享类型路径的兼容 re-export。
- `ipkvm-desktop-iced`：正式桌面图形界面和唯一桌面发布入口（设备选择、视频控制台、本地键鼠直通、特殊键/粘贴/截图、状态栏与硬件异常状态）。
- `ipkvm-headless`：无硬件 RFB TCP/WebSocket 与内嵌中文 noVNC HTTP library（含 `/api/devices`、`/api/session`、`/api/status`、`/api/screenshot`）。
- `ipkvm-headless-app`：正式 `ipkvm-headless` 后台 binary，负责真实 camera/serial 组装；`ipkvm-headless-demo` 提供 `ipkvm-demo`，`ipkvm-browser-fixture` 提供 deterministic noVNC 自动化夹具。TLS 尚未实现。

`ipkvm-session` 当前默认按 CH9329 出厂波特率 9600 配置串口。硬件到货前不自动改写芯片参数，也不假定成品线支持 115200。

## 运行桌面 app（iced）

```powershell
cargo run -p ipkvm-desktop-iced --all-features
```

启动后选择视频设备和控制设备；控制设备必须探测为合法 CH9329 后「连接」按钮才会启用。连接后：

- 点击视频区域获得焦点后可发送本地键盘/鼠标（绝对坐标按目标画面缩放换算）。
- 控制菜单可发送 Ctrl+Alt+Del、Esc、F1-F12、Insert/Delete/Home/End/PageUp/PageDown、方向键，粘贴剪贴板文本，释放所有按键，截图复制到剪贴板（Windows 还可保存 JPEG）。
- 重新选择设备或停止连接不退出 app；切换设备采用会话级停旧启新。
- 底部状态栏显示控制设备、键盘输入、鼠标坐标和视频状态（无信号/断流/分辨率）。
- 视频断流连续 2 秒显示「无信号」，app 不退出；CH9329 掉线后输入进入「控制设备离线」，刷新检测重新探测后可手动重新连接（自动恢复见 issue #37）。

## 设计文档

- `docs/ipkvm-coarse-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-38-protocol-core-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-tcp-transport-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-keyboard-mapping-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-pointer-mapping-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-input-pump-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-websocket-transport-design.md`
- `docs/superpowers/specs/2026-07-31-novnc-web-browser-design.md`
- `docs/superpowers/specs/2026-07-31-ch9329-protocol-input-core-design.md`
- `docs/superpowers/specs/2026-07-31-dependency-license-policy-design.md`
- `docs/dependency-license-policy.md`
- `docs/references/README.md`

## 开发规范

- `AGENTS.md`
- `docs/development-guidelines.md`
- `.github/ISSUE_TEMPLATE/`
- `.github/PULL_REQUEST_TEMPLATE.md`

## 验证

提交和 PR 的自动化验收在本机通过统一脚本执行（GitHub Actions 远端 CI 也调用同一组脚本）：

```powershell
cargo install --locked --version 0.20.2 cargo-deny
.\scripts\verify.ps1
```

Linux/macOS 使用对应的 sh 版本：

```bash
cargo install --locked --version 0.20.2 cargo-deny
./scripts/verify.sh
```

桌面端 M5 退役门禁也由上述统一脚本调用；Windows release 发布物启动冒烟需在构建后显式运行：

```powershell
cargo build --release -p ipkvm-desktop-iced --bin ipkvm-desktop-iced
.\scripts\verify-desktop-release.ps1
```

启动冒烟验证 release 进程持续存活并创建非零顶层窗口句柄，不读取窗口标题、About 文本或 `GIT_COMMIT`。

脚本会检查文本 UTF-8 编码、noVNC 资源来源和逐文件哈希、浏览器依赖锁文件，并用临时负向夹具验证许可证与资源门禁；随后检查当前锁定依赖图、Rust 格式、全工作区测试、Clippy、Rust 文档、Git 差异和真实浏览器闭环。首次浏览器验收通过 `npm ci` 安装锁定的 `playwright-core`，需要 Node.js 20 以上版本、npm、受支持的系统 Chrome 或 Edge 和 npm registry 网络访问。

可独立运行静态资源或浏览器检查：

```powershell
.\scripts\verify-web-assets.ps1
.\scripts\verify-browser.ps1
```

固定工具版本、许可证分级和非 Cargo 组件边界见 `docs/dependency-license-policy.md`。

## 运行无头后台进程

正式 `ipkvm-headless` 二进制同时提供 RFB TCP（供标准 VNC 客户端）和嵌入式 noVNC 网页 + RFB WebSocket（供浏览器），两个入口共享同一个单活动控制者连接闸门。视频源按 CLI 参数提供启动默认值：`--camera <名称>` 打开 Windows 相机（按 id 或显示名，DirectShow 后端，含 OBS 虚拟摄像头），`--assets <目录>` 使用目录内 Y4M 文件伪设备（按文件名排序循环播放）；未指定任何视频参数时启动空会话，网页连接页选择设备后再创建；`--list-cameras` 只枚举设备并退出。真实 CH9329 串口可通过 `--serial` 接入；未指定时键鼠事件进入模拟队列后被丢弃。

```bash
./scripts/fetch-demo-assets.sh   # 首次运行下载 Y4M 素材
cargo run -p ipkvm-headless-app --bin ipkvm-headless \
    --assets .cache/demo-assets --tcp 5900 --http 6080 --fps 10

# Windows：使用 OBS 虚拟摄像头或其他相机（无需 --assets）
cargo run -p ipkvm-headless-app --bin ipkvm-headless \
    --camera "OBS Virtual Camera" --tcp 5900 --http 6080

# Windows：相机 + 真实 CH9329 串口注入（CH340 通常为 COMx，默认 9600 8N1）
cargo run -p ipkvm-headless-app --bin ipkvm-headless \
    --camera "OBS Virtual Camera" --tcp 5900 --http 6080 --serial COM9

# 只枚举相机设备
cargo run -p ipkvm-headless-app --bin ipkvm-headless --list-cameras

# 配置与鉴权：--config 读取 TOML 文件，CLI 参数覆盖文件字段（CLI > 文件 > 默认）
cargo run -p ipkvm-headless-app --bin ipkvm-headless \
    --assets .cache/demo-assets --config config.toml --token abc12345 --vnc-password abc12345
```

启动后用浏览器打开 `http://127.0.0.1:6080`，或用标准 VNC 客户端连接 `127.0.0.1:5900`。素材按文件名排序循环播放，切换分辨率时已连接客户端收到 `DesktopSize` 更新。`--bind` 可指定监听地址（默认 `127.0.0.1`）。`--camera` 与 `--assets` 互斥；相机未就绪时可用 `--assets` 的 Y4M 模拟帧源验证画面与键鼠链路。

### HTTP 管理 API

管理 API 与页面、WebSocket 一样受 token/本机来源鉴权保护：

- `GET /api/devices`：返回视频设备和串口设备列表。
- `POST /api/session`：`{"action":"restart","video":"<设备 id>","serial":"COM9"}` 按请求设备重启会话；缺省字段沿用上一成功会话选择，初始选择来自启动配置，`serial` 为空字符串表示使用模拟队列。`create` 仅用于无会话首启，`stop` 停止当前输入泵。
- `GET /api/status`：返回服务、当前视频源、最近帧、控制连接和会话统计。
- `GET /api/screenshot`：返回当前帧源的 JPEG 快照。

运行时换设备采用会话级重启：旧输入泵先停止，旧帧源和串口 sink 被释放后再打开新设备；新会话启动成功后发布给状态、截图和新 RFB 连接。旧 RFB 连接不保证无缝迁移，客户端可断开后重连。新设备构建失败时会尝试按上一成功选择回滚启动。

### 配置：TOML 文件 + CLI 覆盖

默认值：`--bind` 默认 `127.0.0.1`、`--tcp` 默认 `5900`、`--http` 默认 `6080`、`--fps` 默认 `10`、`--baud` 默认 `9600`。`--config <路径>` 读取 TOML 配置文件，CLI 参数覆盖文件字段，优先级为 **CLI > 文件 > 默认**；配置文件错误会打印含文件路径的确定性中文报错。示例 `config.toml`（与设计文档一致）：

```toml
[server]
bind = "127.0.0.1"
tcp_port = 5900
http_port = 6080

[video]
camera = "OBS Virtual Camera"    # 或 assets 目录
fps = 30

[input]
serial = "COM9"
baud = 9600

[auth]
token = "..."                    # 可选；HTTP/WS 鉴权 token（非空，仅含字母数字与 - _ . ~）
vnc_password = "abc12345"        # 可选；RFB VNC 密码（1-8 个 ASCII 字符）
```

### 鉴权（最小）

`token` 与 `vnc_password` 独立，分别管两个入口：

- `--token` / `[auth] token`：HTTP 与 WebSocket（含 `/rfb` 升级）凭证，必须为非空、仅含 RFC 3986 无保留字符（字母数字、`- _ . ~`）的字符串。启用后所有请求（含本机）必须带 `Authorization: Bearer <token>`、cookie `ipkvm_token=<token>` 或 query 参数 `?token=<token>` 之一。浏览器首次访问 `http://host:6080/?token=xxx` 即可：页面自动把 query token 拼到 WebSocket 地址，并在放行后换得 cookie。
- `--vnc-password` / `[auth] vnc_password`：RFB TCP 入口的 VNC 密码挑战，长度 1-8 个 ASCII 字符（RFC 6143 密码上限 8 字节）。标准 VNC 客户端（含 vncdotool）用该密码连接。
- 未配置 token 时 HTTP/WS 仅放行本机来源（防默认暴露）；未配置 vnc_password 时 RFB TCP 仅允许本机连接。两个入口都支持通过 `--bind` 扩大监听范围，但鉴权凭证是独立维度。

## 演示：双分辨率视频 mock 源

不依赖真实采集卡，可以用真实视频文件验证 RFB 画面与动态分辨率切换：

```bash
./scripts/fetch-demo-assets.sh   # 下载并转换 640x360 与 1280x720 两个 Y4M 素材
cargo run -p ipkvm-headless-demo --bin ipkvm-demo \
    --assets .cache/demo-assets --tcp 5900 --fps 10
```

素材按文件名排序循环播放，切换分辨率时已连接客户端会收到 `DesktopSize` 更新。用独立的 vncdotool 客户端验证：

```bash
python3 -m venv .venv && .venv/bin/pip install vncdotool
.venv/bin/python scripts/vnc-dynamic-resolution-check.py --port 5900
```

仓库内自动化测试已覆盖同一条路径：`rfb_dynamic_resolution` 通过真实 TCP 连接断言 `DesktopSize` 与切换后的 Raw 帧。
