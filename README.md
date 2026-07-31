# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程已完成 CH9329 协议与输入核心、传输无关的 RFB 3.8 协议核心、使用模拟 BGRA 帧源的单客户端 RFB TCP 库闭环、en-US 键盘和绝对指针映射，以及单活动 RFB 控制者输入事件泵。事件泵已经自动验证断线与事件源关闭时的 `release_all()`、失败回滚和真实回环 TCP 到 fake CH9329 队列的闭环。真实串口、真实视频采集、可直接运行的无头进程、WebSocket/noVNC 和桌面界面仍按阶段计划继续实现。

当前 `ipkvm-headless` 二进制仍是脚手架，尚不能控制真实机器；上述能力目前是库级闭环，尚未接到真实串口和可直接运行的后台进程。

## 当前模块

- `ipkvm-core`：CH9329 命令帧和应答解析、串口字节流增量解帧、HID 报告、6KRO 键盘状态、原子键盘和指针批次、绝对和相对鼠标状态、有序命令批次及模拟队列。
- `ipkvm-video`：采集设备枚举、格式选择、共享视频帧流。
- `ipkvm-session`：把视频帧源和输入接收端组合成一个控制台会话。
- `ipkvm-rfb`：传输无关的 RFB 3.8 None 握手、客户端消息增量解码、true-color 像素转换、Raw 更新、DesktopSize 和指针输入坐标时期。
- `ipkvm-desktop`：本地图形界面入口。
- `ipkvm-headless`：已有单客户端 RFB TCP 库接口、en-US 键盘映射器、绝对指针映射器和单控制者输入事件泵；可运行后台进程、HTTP、WebSocket/noVNC 和设备会话仍待实现。

`ipkvm-session` 当前默认按 CH9329 出厂波特率 9600 配置串口。硬件到货前不自动改写芯片参数，也不假定成品线支持 115200。

## 设计文档

- `docs/ipkvm-coarse-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-38-protocol-core-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-tcp-transport-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-keyboard-mapping-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-pointer-mapping-design.md`
- `docs/superpowers/specs/2026-07-31-rfb-input-pump-design.md`
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
