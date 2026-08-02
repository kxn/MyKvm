# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程已完成 CH9329 协议与输入核心和传输无关的 RFB 3.8 协议核心；连接驱动器、`RfbServerEvent` 事件模型、`RfbConnectionGate` 仲裁、en-US 键盘和绝对指针映射、输入事件泵等会话核心已在 `ipkvm-session`。RFB 已有 TCP 与 WebSocket 两个库级传输层：`RfbTcpServer` 和可组合的 axum `/rfb` `RfbWebSocketService`。两者共用 `ipkvm-session` 的连接驱动器与全局 `RfbConnectionGate`；生产组装必须向两个服务显式传入同一个连接闸门。

`ipkvm-headless` 已提供可供生产组装复用的嵌入式 Web 服务。它内置项目中文控制台页面和固定到 noVNC 1.7.0 提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e` 的完整 npm 发布资源，并通过同源 `/rfb` 建立连接。真实 Chrome 自动化已经证明模拟帧像素、桌面与窄视口等比缩放、键盘 HID、缩放后的绝对指针坐标、按键顺序、断开释放和重连全部穿过 noVNC、RFB 服务与 `RfbInputPump` 到达记录型 `InputSink`。

当前正式 `ipkvm-headless` 二进制已能作为可运行后台进程提供完整的 RFB TCP（5900）+ noVNC 网页（6080）双传输服务。视频源通过 CLI 选择：`--camera` 打开 Windows 相机（按 id 或显示名，DirectShow 后端，含 OBS 虚拟摄像头）、`--assets` 使用 Y4M 文件伪设备、未指定时默认优先打开 OBS 虚拟摄像头（找不到时退回第一台）、`--list-cameras` 只枚举设备并退出。键鼠注入通过 CLI 选择：`--serial <路径>` 打开真实 CH9329 串口（默认 9600 8N1，`--baud <速率>` 可调），未指定时键鼠事件进入模拟串口队列后被丢弃。鉴权和 TLS 尚未实现。

## 当前模块

- `ipkvm-core`：CH9329 命令帧和应答解析、串口字节流增量解帧、HID 报告、6KRO 键盘状态、原子键盘和指针批次、绝对和相对鼠标状态、有序命令批次及模拟队列；`serial` 功能下提供真实串口命令队列 `SerialCommandQueue`（9600 8N1，跨平台 COMx / ttyUSBn，帧间延时防丢帧）。
- `ipkvm-video`：采集设备枚举、格式选择、共享视频帧流与 `source_info` 元数据；`mock` 功能下提供 Y4M 循环播放帧源（可模拟不同分辨率素材顺序切换），`mf` 功能下提供 Windows DirectShow 相机后端（`list_cameras` 枚举、`CameraSource` 采集与 `camera_probe` 示例，含 OBS 虚拟摄像头）——采用自研纯 sink filter（不依赖系统的 Sample Grabber，因其与 OBS 虚拟摄像头不兼容）+ 事件驱动（Condvar 阻塞等待，无帧时零轮询），`file_source` 提供 Y4M 文件伪设备。
- `ipkvm-session`：真实会话核心——连接驱动与事件模型（`RfbConnectionGate` 仲裁）、输入泵与映射器、设备枚举（`devices`）、`ConsoleSession` 组装与 `SessionManager` 生命周期管理、会话状态统计。
- `ipkvm-rfb`：传输无关的 RFB 3.8 `None` 握手、客户端消息增量解码、真彩像素转换、`Raw` 更新、`DesktopSize` 和指针输入坐标时期。
- `ipkvm-desktop`：本地图形界面入口。
- `ipkvm-headless`：RFB TCP 与 WebSocket 传输适配层，以及内嵌中文 noVNC 页面的 HTTP 服务（含 `/api/status` 状态接口与 `/api/screenshot` JPEG 快照接口）；`demo` 功能下提供 `ipkvm-headless` 正式后台进程（`--serial`/`--baud` 真实 CH9329 串口注入）和 `ipkvm-demo` 演示二进制。鉴权和 TLS 尚未实现。

`ipkvm-session` 当前默认按 CH9329 出厂波特率 9600 配置串口。硬件到货前不自动改写芯片参数，也不假定成品线支持 115200。

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
- `.gitea/ISSUE_TEMPLATE/`
- `.gitea/PULL_REQUEST_TEMPLATE.md`

## 验证

当前不依赖 Gitea Actions runner。提交和 PR 的自动化验收在本机通过统一脚本执行：

```powershell
cargo install --locked --version 0.20.2 cargo-deny
.\scripts\verify.ps1
```

Linux/macOS 使用对应的 sh 版本：

```bash
cargo install --locked --version 0.20.2 cargo-deny
./scripts/verify.sh
```

脚本会检查文本 UTF-8 编码、noVNC 资源来源和逐文件哈希、浏览器依赖锁文件，并用临时负向夹具验证许可证与资源门禁；随后检查当前锁定依赖图、Rust 格式、全工作区测试、Clippy、Rust 文档、Git 差异和真实浏览器闭环。首次浏览器验收通过 `npm ci` 安装锁定的 `playwright-core`，需要 Node.js 20 以上版本、npm、受支持的系统 Chrome 或 Edge 和 npm registry 网络访问。

可独立运行静态资源或浏览器检查：

```powershell
.\scripts\verify-web-assets.ps1
.\scripts\verify-browser.ps1
```

固定工具版本、许可证分级和非 Cargo 组件边界见 `docs/dependency-license-policy.md`。

## 运行无头后台进程

正式 `ipkvm-headless` 二进制同时提供 RFB TCP（供标准 VNC 客户端）和嵌入式 noVNC 网页 + RFB WebSocket（供浏览器），两个入口共享同一个单活动控制者连接闸门。视频源按 CLI 参数选择：`--camera <名称>` 打开 Windows 相机（按 id 或显示名，DirectShow 后端，含 OBS 虚拟摄像头），`--assets <目录>` 使用目录内 Y4M 文件伪设备（按文件名排序循环播放），未指定任何视频参数时默认优先打开 OBS 虚拟摄像头（找不到时退回第一台，避免在多虚拟摄像头并存时误选 ToDesk 等其它设备）；`--list-cameras` 只枚举设备并退出。真实 CH9329 串口尚未接入，键鼠事件进入模拟队列后被丢弃。

```bash
./scripts/fetch-demo-assets.sh   # 首次运行下载 Y4M 素材
cargo run -p ipkvm-headless --features demo --bin ipkvm-headless \
    --assets .cache/demo-assets --tcp 5900 --http 6080 --fps 10

# Windows：使用 OBS 虚拟摄像头或其他相机（无需 --assets）
cargo run -p ipkvm-headless --features demo --bin ipkvm-headless \
    --camera "OBS Virtual Camera" --tcp 5900 --http 6080

# Windows：相机 + 真实 CH9329 串口注入（CH340 通常为 COMx，默认 9600 8N1）
cargo run -p ipkvm-headless --features demo --bin ipkvm-headless \
    --camera "OBS Virtual Camera" --tcp 5900 --http 6080 --serial COM9

# 只枚举相机设备
cargo run -p ipkvm-headless --features demo --bin ipkvm-headless --list-cameras
```

启动后用浏览器打开 `http://127.0.0.1:6080`，或用标准 VNC 客户端连接 `127.0.0.1:5900`。素材按文件名排序循环播放，切换分辨率时已连接客户端收到 `DesktopSize` 更新。`--bind` 可指定监听地址（默认 `127.0.0.1`）。`--camera` 与 `--assets` 互斥；相机未就绪时可用 `--assets` 的 Y4M 模拟帧源验证画面与键鼠链路。

## 演示：双分辨率视频 mock 源

不依赖真实采集卡，可以用真实视频文件验证 RFB 画面与动态分辨率切换：

```bash
./scripts/fetch-demo-assets.sh   # 下载并转换 640x360 与 1280x720 两个 Y4M 素材
cargo run -p ipkvm-headless --features demo --bin ipkvm-demo \
    --assets .cache/demo-assets --tcp 5900 --fps 10
```

素材按文件名排序循环播放，切换分辨率时已连接客户端会收到 `DesktopSize` 更新。用独立的 vncdotool 客户端验证：

```bash
python3 -m venv .venv && .venv/bin/pip install vncdotool
.venv/bin/python scripts/vnc-dynamic-resolution-check.py --port 5900
```

仓库内自动化测试已覆盖同一条路径：`rfb_dynamic_resolution` 通过真实 TCP 连接断言 `DesktopSize` 与切换后的 Raw 帧。
