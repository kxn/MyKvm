# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程已完成 CH9329 协议与输入核心、传输无关的 RFB 3.8 协议核心、en-US 键盘和绝对指针映射，以及单活动 RFB 控制者输入事件泵。RFB 已有 TCP 与 WebSocket 两个库级传输层：`RfbTcpServer` 和可组合的 axum `/rfb` `RfbWebSocketService`。两者共用连接驱动器、`RfbServerEvent` 事件模型和全局 `RfbConnectionGate`；生产组装必须向两个服务显式传入同一个连接闸门。

`ipkvm-headless` 已提供可供生产组装复用的嵌入式 Web 服务。它内置项目中文控制台页面和固定到 noVNC 1.7.0 提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e` 的完整 npm 发布资源，并通过同源 `/rfb` 建立连接。真实 Chrome 自动化已经证明模拟帧像素、桌面与窄视口等比缩放、键盘 HID、缩放后的绝对指针坐标、按键顺序、断开释放和重连全部穿过 noVNC、RFB 服务与 `RfbInputPump` 到达记录型 `InputSink`。

当前正式 `ipkvm-headless` 二进制仍是脚手架，尚不能控制真实机器。真实视频采集、真实串口、设备选择、生产进程组装、鉴权和 TLS 均尚未实现；浏览器夹具使用独立功能开关，不进入默认生产二进制。

## 当前模块

- `ipkvm-core`：CH9329 命令帧和应答解析、串口字节流增量解帧、HID 报告、6KRO 键盘状态、原子键盘和指针批次、绝对和相对鼠标状态、有序命令批次及模拟队列。
- `ipkvm-video`：采集设备枚举、格式选择、共享视频帧流；`mock` 功能下提供 Y4M 循环播放帧源，可模拟不同分辨率素材顺序切换。
- `ipkvm-session`：把视频帧源和输入接收端组合成一个控制台会话。
- `ipkvm-rfb`：传输无关的 RFB 3.8 `None` 握手、客户端消息增量解码、真彩像素转换、`Raw` 更新、`DesktopSize` 和指针输入坐标时期。
- `ipkvm-desktop`：本地图形界面入口。
- `ipkvm-headless`：RFB TCP 与 WebSocket 传输层、共享连接驱动器、单控制者连接闸门、en-US 键盘映射器、绝对指针映射器、输入事件泵，以及内嵌中文 noVNC 页面的 HTTP 服务；`demo` 功能下提供 `ipkvm-demo` 演示二进制。设备会话组装和可运行后台进程尚未实现。

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
