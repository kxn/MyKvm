# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程已完成 CH9329 协议与输入核心、传输无关的 RFB 3.8 协议核心、en-US 键盘和绝对指针映射，以及单活动 RFB 控制者输入事件泵。RFB 已有 TCP 与 WebSocket 两个库级传输层：`RfbTcpServer` 和可组合的 axum `/rfb` `RfbWebSocketService`。两者共用连接驱动器、`RfbServerEvent` 事件模型和全局 `RfbConnectionGate`；生产组装必须向两个服务显式传入同一个连接闸门。锁定到 noVNC 1.7.0 提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e` 的无子协议线级初始化样本验证了初始与增量 `Raw` 更新；独立升级测试覆盖无子协议和可选 `binary` 子协议，共享驱动测试覆盖 `DesktopSize`。

当前 `ipkvm-headless` 二进制仍是脚手架，尚不能控制真实机器；上述能力目前是库级闭环。完整网页、真实浏览器闭环、noVNC 静态资源、真实视频采集、真实串口、鉴权、TLS 和可直接运行的无头进程均尚未实现。

## 当前模块

- `ipkvm-core`：CH9329 命令帧和应答解析、串口字节流增量解帧、HID 报告、6KRO 键盘状态、原子键盘和指针批次、绝对和相对鼠标状态、有序命令批次及模拟队列。
- `ipkvm-video`：采集设备枚举、格式选择、共享视频帧流。
- `ipkvm-session`：把视频帧源和输入接收端组合成一个控制台会话。
- `ipkvm-rfb`：传输无关的 RFB 3.8 `None` 握手、客户端消息增量解码、真彩像素转换、`Raw` 更新、`DesktopSize` 和指针输入坐标时期。
- `ipkvm-desktop`：本地图形界面入口。
- `ipkvm-headless`：RFB TCP 与 WebSocket 传输层、共享连接驱动器、单控制者连接闸门、en-US 键盘映射器、绝对指针映射器和输入事件泵；完整 HTTP 页面、noVNC 静态资源、设备会话组装和可运行后台进程尚未实现。

`ipkvm-session` 当前默认按 CH9329 出厂波特率 9600 配置串口。硬件到货前不自动改写芯片参数，也不假定成品线支持 115200。

## 设计文档

- `docs/ipkvm-coarse-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-38-protocol-core-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-tcp-transport-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-keyboard-mapping-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-pointer-mapping-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-input-pump-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-websocket-transport-design.md`
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

脚本会检查文本 UTF-8 编码，用临时负向夹具验证许可证策略，再检查当前锁定依赖图的许可证和来源，随后检查 Rust 格式、全工作区测试、Clippy、Rust 文档和 Git 差异。固定工具版本、许可证分级和非 Cargo 组件边界见 `docs/dependency-license-policy.md`。
