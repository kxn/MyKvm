# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程已完成工作区脚手架，以及 CH9329 协议编解码和键鼠输入状态核心。真实串口传输、设备探测、视频采集、桌面界面和 RFB 服务仍按阶段计划继续实现。

## 当前模块

- `ipkvm-core`：CH9329 命令帧和应答解析、串口字节流增量解帧、HID 报告、6KRO 键盘状态、绝对和相对鼠标状态、有序命令批次及 fake 队列。
- `ipkvm-video`：采集设备枚举、格式选择、共享视频帧流。
- `ipkvm-session`：把视频帧源和输入接收端组合成一个控制台会话。
- `ipkvm-rfb`：最小 VNC/RFB 服务，后续支持 TCP 和 WebSocket 传输。
- `ipkvm-desktop`：本地图形界面入口。
- `ipkvm-headless`：无头后台进程、HTTP、noVNC 静态文件和配置入口。

`ipkvm-session` 当前默认按 CH9329 出厂波特率 9600 配置串口。硬件到货前不自动改写芯片参数，也不假定成品线支持 115200。

## 设计文档

- `docs/ipkvm-coarse-design.md`
- `docs/references/README.md`

## 开发规范

- `AGENTS.md`
- `docs/development-guidelines.md`
- `.gitea/ISSUE_TEMPLATE/`
- `.gitea/PULL_REQUEST_TEMPLATE.md`

## 验证

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
```
