# my_ipkvm

my_ipkvm 是一个软件 IPKVM 项目：主控机通过 USB HDMI 采集卡读取目标机控制台画面，并通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

当前工程处于脚手架阶段，目标是先固定模块边界、测试入口和许可证边界。

## 当前模块

- `ipkvm-core`：CH9329 帧、HID 报告、坐标换算、输入状态机。
- `ipkvm-video`：采集设备枚举、格式选择、视频帧流。
- `ipkvm-session`：把视频帧源和输入接收端组合成一个控制台会话。
- `ipkvm-rfb`：最小 VNC/RFB 服务，后续支持 TCP 和 WebSocket 传输。
- `ipkvm-desktop`：本地图形界面入口。
- `ipkvm-headless`：无头后台进程、HTTP、noVNC 静态文件和配置入口。

## 设计文档

- `docs/ipkvm-coarse-design.md`
- `docs/references/README.md`

## 验证

```powershell
cargo fmt --all --check
cargo test --workspace
```

