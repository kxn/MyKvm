# my_ipkvm 粗粒度方案与设计草案

日期：2026-07-31

## 目标

做一个机房内软件 IPKVM：一台主控机通过 USB HDMI 采集卡读取目标机控制台画面，通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

目标形态分两版：

- 桌面图形版：类似 VNC 客户端，本机窗口显示采集卡画面，窗口获得焦点时转发键盘和鼠标事件。
- 无头版：后台进程运行，对外提供 VNC/RFB 服务，并内置 noVNC 网页。普通 VNC 客户端和浏览器都能连接。

本设计不基于 Qt/PyQt 实现。`kvm-serial` 只作为协议和产品行为参考，不继承它的 PyQt 图形界面。

当前设计不处理安全问题。鉴权、TLS、访问控制、审计、公网暴露、反向代理、VPN、会话权限等都视为后续可叠加层，不进入当前最小版本。

## 总体结论

推荐从头写 Rust 项目。理由：

- CH9329 协议简单，适合做成小而稳定的核心模块。
- Rust 适合把桌面图形界面、串口、视频采集、RFB/VNC、网页服务统一到一个类型安全的工程内。
- 可以默认使用 MIT、Apache、BSD、ISC、Zlib、MPL-2.0 这类可接受依赖，避免 PyQt GPL。
- 如需 LGPL，限定在动态链接的 C/C++ 系统库或媒体库边界，避免普通 Cargo 依赖静态编入主二进制。

无头版不建议做 RDP 服务端。RDP 是完整桌面远程协议栈，复杂度远高于本项目需要；我们只有 HDMI 像素流和 HID 输入，和 VNC/RFB 的远程帧缓冲模型天然匹配。

Go 可用于无头后台进程，但不推荐作为桌面主实现。Go 的网页视图类桌面框架会让键盘捕获更接近浏览器限制，和“VNC 裸窗口”目标有冲突。

## 当前范围

当前最小版本包含：

- CH9329 串口协议。
- USB HID 键盘鼠标映射。
- USB HDMI 采集卡视频输入。
- 桌面本地控制台窗口。
- 无头版 VNC/RFB 服务。
- 内置 noVNC 网页。
- 设备枚举和手动选择。
- 单目标机、单采集卡、单串口会话。

当前最小版本明确不做：

- RDP 服务端。
- Qt、PyQt、PySide。
- GPL 依赖进入主程序。
- 复用 LibVNCServer/TightVNC 代码。
- H.264/WebRTC 低带宽压缩。
- 多目标机管理。
- 多用户并发控制仲裁。
- 音频、文件传输、剪贴板同步。
- CH9350L 支持。
- 安全、鉴权、TLS、公网访问。

## 许可证策略

默认策略：

- 允许：MIT、Apache-2.0、BSD、ISC、Zlib、MPL-2.0、系统 SDK。
- 可接受但需隔离：LGPL 动态库，例如 GStreamer、LGPL 构建的 FFmpeg、平台媒体库包装层。
- 不接受：GPL 依赖进入主程序分发包，除非以后明确决定整项目 GPL。
- 不采用：PyQt 免费版，因为它是 GPL/商业版双许可，不是 LGPL。

Rust 具体规则：

- 普通 Cargo 依赖会静态链接进最终二进制，因此不把 LGPL Rust 库当普通依赖使用。
- 如果必须使用 LGPL Rust 代码，将其拆成独立 `cdylib` 或独立进程，通过 C ABI 或 IPC 调用。
- FFmpeg/GStreamer 如使用，优先动态加载或动态链接，并在发布包内保留许可证说明、源码获取方式和替换库说明。
- noVNC 核心库是 MPL-2.0，可以作为网页前端依赖；若修改 noVNC 自身文件，修改过的文件按 MPL-2.0 保留。

## 架构

```mermaid
flowchart LR
    desktop["桌面图形版"] --> session["控制台会话"]
    headless["无头后台进程"] --> session
    novnc["noVNC 网页"] --> rfbws["基于 WebSocket 的 RFB"]
    vnc["VNC 客户端"] --> rfbtcp["基于 TCP 的 RFB"]
    rfbws --> rfb["VNC/RFB 服务"]
    rfbtcp --> rfb
    rfb --> session

    session --> core["核心输入控制"]
    session --> video["视频流水线"]

    core --> serial["串口传输"]
    core --> hid["HID 映射"]
    core --> mouse["鼠标坐标映射"]
    serial --> ch9329["CH9329 协议"]

    video --> capture["采集后端"]
    video --> frames["帧源"]

    capture --> win["Windows Media Foundation"]
    capture --> linux["Linux V4L2"]
    capture --> mac["macOS AVFoundation"]
```

核心边界：

- `ipkvm-core` 不依赖图形界面、不依赖网页服务、不依赖 VNC、不依赖具体视频库。
- `ipkvm-video` 不知道 CH9329，只产出视频帧流。
- `ipkvm-session` 组合一个视频帧源和一个输入接收端。
- `ipkvm-desktop` 使用会话，负责窗口、输入事件、视频显示。
- `ipkvm-headless` 使用会话，负责后台进程生命周期、设备选择、HTTP/noVNC、RFB 监听。
- `ipkvm-rfb` 是对外协议入口，不是内部核心。

建议模块拆分：

```text
ipkvm-core       CH9329 帧、HID 报告、坐标换算、输入状态机
ipkvm-video      采集设备枚举、格式选择、视频帧流
ipkvm-session    把视频帧源和输入接收端组合成一个控制台会话
ipkvm-rfb        最小 VNC/RFB 服务，支持 TCP 和 WebSocket 传输
ipkvm-desktop    本地图形界面
ipkvm-headless   后台进程、HTTP、noVNC 静态文件、配置
```

## 核心接口

### `ipkvm-core`

职责：

- CH9329 帧构造：`57 AB + addr + cmd + len + data + checksum`。
- 键盘 HID 报告：维护当前按下的修饰键和最多 6 个普通键。
- 鼠标报告：绝对坐标、相对滚轮、按钮状态。
- 坐标换算：客户端坐标 -> 视频源像素坐标 -> CH9329 的 0..4095 坐标。
- 设备状态：串口连接状态、协议统计、最后发送时间、错误状态。

关键接口草案：

```text
SerialDevice
  list() -> Vec<SerialPortInfo>
  open(path, baud) -> SerialSession

Ch9329Device
  send_keyboard(report)
  release_all_keys()
  send_mouse_absolute(buttons, x, y, width, height, wheel)
  send_mouse_relative(buttons, dx, dy, wheel)

InputSink
  key_down(key)
  key_up(key)
  pointer_move(x, y, framebuffer_size)
  pointer_button(button, down)
  wheel(delta)
  release_all()
```

第一版只支持 CH9329。CH9350L 作为后续扩展，不进当前最小版本。

### `ipkvm-video`

职责：

- 枚举采集设备。
- 枚举和选择格式：分辨率、帧率、像素格式、是否 MJPEG。
- 输出统一的视频帧流。
- 对外提供最新帧缓存和订阅接口。

数据类型草案：

```text
VideoDeviceInfo
  id
  display_name
  backend
  supported_formats

VideoFormat
  width
  height
  fps
  pixel_format: YUY2 | NV12 | RGB | MJPEG | H264 | Unknown

VideoFrame
  timestamp
  width
  height
  format
  data
```

视频处理分三阶段：

- 第一阶段：本地桌面显示，采集未压缩帧或 MJPEG 解码后的帧，直接渲染。
- 第二阶段：RFB 服务使用 RGBX/BGRA 帧缓冲。第一版允许整帧更新和帧率限制。
- 第三阶段：补充 RFB 压缩编码或独立 WebRTC/H.264/VP8，不自己实现视频编码器。

### `ipkvm-session`

职责：

- 管理一个目标控制台会话。
- 固定本次会话的视频设备、视频格式、串口设备。
- 将输入入口发来的键鼠事件统一送入 `InputSink`。
- 将采集帧统一发布给桌面渲染器和 RFB 服务。

会话约束：

- 当前最小版本一个进程只管理一个活动会话。
- 会话启动后视频尺寸固定；采集格式变化需要重连。
- 多客户端观看可以后续支持，但当前最小版本只有一个控制者。
- 串口断开时视频继续运行，输入返回错误并尝试触发 `release_all()`。
- 视频断开时 RFB/图形界面保留连接，但显示合成错误帧或黑帧。

## 桌面图形版设计

推荐技术：

- 窗口和事件循环：`winit`
- 界面：`egui` 或极简自绘设置页
- 渲染：`wgpu` 或 `pixels`

交互：

- 启动显示设置页：选择视频设备、分辨率、串口、波特率、键盘布局。
- 连接成功后进入控制台视图：黑底视频裸窗口，少量覆盖状态可隐藏。
- 窗口按视频比例缩放。用户可拖动大小，但内容保持比例，不变形。
- 焦点在视频视图内时，捕获可捕获的键盘事件并发送。
- 鼠标不做系统级捕获；只有指针在视频区域上方时，发送移动、按钮、滚轮事件。
- 点击前先发送一次绝对移动，再发送按钮事件，避免在旧坐标点击。
- 提供菜单或快捷命令发送 `Ctrl+Alt+Del`、释放所有按键、截图。

桌面版第一版不做全屏，只保留后续扩展位。

## 无头版 VNC/网页设计

### 对外入口

无头版提供两个入口：

```text
TCP 5900       标准 RFB/VNC 服务，供普通 VNC 客户端使用
HTTP 6080      设备选择页、noVNC 静态页面、RFB WebSocket 入口
```

网页不再自研远控协议。它只负责：

- 枚举视频和串口设备。
- 创建或重启控制台会话。
- 嵌入 noVNC 客户端。
- 把 noVNC 连接到本进程的 RFB WebSocket 入口。

建议 HTTP 路由：

```text
GET  /                  设置页
GET  /api/devices       视频和串口设备列表
POST /api/session       启动或重启会话
GET  /novnc/...         noVNC 静态资源
GET  /rfb               基于 WebSocket 的 RFB
```

### RFB/VNC 服务

当前最小版本实现 RFB 3.8 子集：

- `ProtocolVersion` 握手。
- `SecurityType None`。安全不在当前范围内。
- `ClientInit` / `ServerInit`。
- `SetPixelFormat`。
- `SetEncodings`。
- `FramebufferUpdateRequest`。
- `FramebufferUpdate`。
- `KeyEvent`。
- `PointerEvent`。
- 暂时忽略 `ClientCutText`。

当前最小版本的编码支持：

- 必须支持 `Raw`。
- 初始实现整帧更新，限制帧率，先保证行为正确。
- 后续再做脏块检测，减少静态画面带宽。
- `ZRLE`、`Tight/JPEG`、H.264 扩展都不进当前最小版本。

像素格式：

- 内部统一为 `RGBX8888` 或 `BGRA8888`。
- RFB 根据客户端像素格式做必要转换。
- 第一版可只优化常见 32 位真彩格式，其他格式走慢路径或拒绝。

输入映射：

- RFB 的 `KeyEvent` 使用 keysym，需要映射到 USB HID 用法编号。
- 当前最小版本只保证 `en_US` 键盘布局、修饰键、F1-F12、方向键、导航键、小键盘常用键。
- 非 ASCII 文本输入不走剪贴板同步，后续可做“按字符模拟键入”。
- RFB 的 `PointerEvent` 坐标直接对应帧缓冲像素坐标，再映射到 CH9329 的 0..4095。
- VNC 按钮掩码映射左键、中键、右键和滚轮；额外鼠标键不进当前最小版本。

会话和并发：

- 当前最小版本支持多个查看者观察同一帧缓冲。
- 当前最小版本同一时间只允许一个控制者发送输入。
- 多个客户端同时连接时，第一个连接者获得输入控制权；其他客户端只读观看。
- 每个客户端有自己的帧更新节流和反压处理。慢客户端只拿最新帧，旧帧直接丢弃。

## 视频压缩策略

桌面图形版不需要做视频压缩，只需要采集和渲染。

无头版第一版也不直接进入视频编码。VNC/RFB 当前最小版本使用 `Raw` 编码加帧率限制，用于验证协议兼容性和控制台操作闭环。

后续优化顺序：

1. 脏块检测
   - 对帧缓冲分块计算摘要，只发送变化块。
   - 对 BIOS、终端、安装器这类静态画面收益大。

2. RFB 压缩编码
   - 优先调研 `ZRLE` 或 `Tight/JPEG`。
   - 只有确认许可证和复杂度可控后再进入实现。

3. 采集卡 MJPEG 透传
   - 如果 UVC 设备已经输出 MJPEG，可另做原生网页视频流或 RFB JPEG 路径。
   - 避免 CPU 重新编码。

4. WebRTC/H.264/VP8
   - 用于跨公网、低带宽、多客户端。
   - 不自己写视频编码器。
   - Windows 用 Media Foundation H.264 编码器。
   - macOS 用 VideoToolbox。
   - Linux 优先考虑 GStreamer 或平台硬件编码栈。

## 平台优先级

建议按工作站实际环境优先：

1. Windows 主控机最小版本
   - 当前开发环境在 Windows。
   - Media Foundation 是新代码推荐路径。
   - CH340 串口验证方便。

2. 无头 VNC/网页最小版本
   - 可先在 Windows 上使用模拟视频源验证 noVNC 和普通 VNC 客户端兼容性。
   - 硬件到货后接真实采集卡和 CH9329。

3. Linux 主控机
   - 机房服务器常见。
   - V4L2 对 UVC 采集卡最直接。
   - 适合后台进程部署。

4. macOS 主控机
   - 支持价值较低，可后置。
   - AVFoundation/VideoToolbox 路径清晰，但打包权限和签名额外麻烦。

## 阶段计划

### 阶段 0：硬件到货前

- 建立 Rust 工作区。
- 写 CH9329 协议单元测试。
- 写 HID 用法编号映射基础表。
- 写坐标换算测试。
- 写模拟视频源。
- 写模拟串口。
- 写 RFB 3.8 握手和 `Raw` 编码协议样例测试。
- 用模拟帧缓冲跑通普通 VNC 客户端和 noVNC。
- 确定依赖许可证白名单。

### 阶段 1：桌面本地最小版本

- Windows 串口枚举和打开。
- Windows 视频设备枚举。
- 选择设备后进入控制台窗口。
- 显示 720p/1080p 视频。
- 键盘普通键、修饰键、F1-F12、方向键、导航键转发。
- 鼠标绝对移动、左键、右键、中键、滚轮转发。
- 释放所有键、发送 `Ctrl+Alt+Del`、截图。

### 阶段 2：无头 VNC/网页最小版本

- 无头后台进程。
- HTTP 设置页。
- noVNC 静态页面。
- 基于 TCP 的 RFB。
- 基于 WebSocket 的 RFB。
- `Raw` 帧缓冲更新。
- RFB `KeyEvent` 到 HID 映射。
- RFB `PointerEvent` 到 CH9329 绝对鼠标。
- 单控制者策略。
- 慢客户端丢帧策略。

### 阶段 3：Linux 主控机

- Linux 串口枚举。
- Linux V4L2 采集。
- 复用阶段 2 的 RFB/noVNC 入口。
- systemd 服务草案。

### 阶段 4：性能优化

- 脏块检测。
- RFB 帧率控制和延迟指标。
- RFB `ZRLE` 或 `Tight/JPEG` 验证。
- 采集卡 MJPEG 透传验证。
- Windows H.264 Media Foundation 编码器验证。
- WebRTC 验证。

### 阶段 5：运维化和安全叠加

- 配置文件。
- Windows 服务或托盘程序。
- 多设备配置档案。
- 连接健康检查。
- 发布包和许可证清单。
- 鉴权、TLS、访问控制、审计、反向代理/VPN 部署文档。

## 风险排除表

当前阶段通过收窄范围排除这些风险：

| 风险点 | 当前设计处理 |
| --- | --- |
| RDP 协议复杂度 | 不做 RDP 服务端，只保留未来调研可能 |
| PyQt/Qt 许可风险 | 不使用 Qt/PyQt/PySide |
| GPL 传染风险 | 不使用 LibVNCServer/TightVNC 等 GPL 代码 |
| 浏览器远控界面自研风险 | 网页版复用 noVNC |
| 自研 WebSocket 输入协议风险 | 无头版输入走 RFB `KeyEvent`/`PointerEvent` |
| H.264/WebRTC 工程复杂度 | 不进当前最小版本 |
| 视频编码器许可风险 | 当前最小版本不引入 FFmpeg/GStreamer |
| RFB `Raw` 带宽风险 | 当前最小版本用于验证闭环，限制帧率；性能优化后置 |
| 键盘布局差异 | 当前最小版本只承诺 `en_US` HID 映射 |
| IME/Unicode 文本输入 | 当前最小版本不做剪贴板同步和 Unicode 粘贴 |
| 多客户端输入冲突 | 当前最小版本单控制者 |
| 设备热插拔复杂度 | 当前最小版本断开后手动重连 |
| 视频格式动态变化 | 当前最小版本会话内固定视频尺寸 |
| CH9350L 差异 | 当前最小版本只支持 CH9329 |
| 安全设计复杂度 | 当前不考虑安全，未来作为外层叠加 |

仍需硬件验证的问题：

- 廉价 HDMI 采集卡实际支持的格式、帧率、MJPEG 行为和延迟。
- Windows Media Foundation 对目标采集卡的枚举、格式选择和帧读取行为。
- CH9329 成品线默认波特率、工作模式、绝对鼠标行为。
- BIOS/UEFI 环境下目标机对 CH9329 模拟键鼠的兼容性。

## 关键参考资料

本地已下载：

- `docs/references/CH9329-serial-protocol-wch-20190508.pdf`
- `docs/references/CH9329-datasheet-akizuki-mirror.pdf`
- `docs/references/USB-HID-Usage-Tables-1.7.pdf`
- `docs/references/USB-Video-Class-1.5-document-set.zip`
- `docs/references/uvc-1.5/USB Video Class 1_5/UVC 1.5 Class specification.pdf`
- `docs/references/uvc-1.5/USB Video Class 1_5/USB_Video_Payload_MJPEG_1.5.pdf`
- `docs/references/uvc-1.5/USB Video Class 1_5/USB_Video_Payload_H264_1.5.pdf`
- `docs/references/uvc-1.5/USB Video Class 1_5/USB_Video_Payload_VP8_1.5.pdf`
- `docs/references/RFC6143-rfb-protocol.txt`
- `docs/references/rfbproto-community-spec.rst`
- `docs/references/noVNC-README.md`
- `docs/references/noVNC-API.md`
- `docs/references/noVNC-EMBEDDING.md`
- `docs/references/noVNC-LICENSE.txt`

在线资料：

- CH9329 数据手册官方页：https://www.wch-ic.com/downloads/CH9329DS1_PDF.html
- CH340/CH341 Windows 驱动：https://www.wch-ic.com/downloads/CH341SER_EXE.html
- USB HID 用途表 1.7：https://usb.org/document-library/hid-usage-tables-17
- USB 视频类 v1.5：https://www.usb.org/document-library/video-class-v15-document-set
- RFC 6143 RFB 协议：https://www.rfc-editor.org/rfc/rfc6143
- RFB 社区协议规格：https://github.com/rfbproto/rfbproto/blob/master/rfbproto.rst
- noVNC：https://github.com/novnc/noVNC
- noVNC 接口文档：https://github.com/novnc/noVNC/blob/master/docs/API.md
- noVNC 嵌入文档：https://github.com/novnc/noVNC/blob/master/docs/EMBEDDING.md
- Windows Media Foundation 采集：https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation
- Windows 视频采集设备枚举：https://learn.microsoft.com/en-us/windows/win32/medfound/enumerating-video-capture-devices
- Windows H.264 编码器：https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder
- macOS AVFoundation 采集设备选择：https://developer.apple.com/documentation/avfoundation/choosing-a-capture-device
- macOS AVCaptureVideoDataOutput：https://developer.apple.com/documentation/avfoundation/avcapturevideodataoutput
- macOS VideoToolbox：https://developer.apple.com/documentation/videotoolbox
- Linux V4L2 接口：https://docs.kernel.org/userspace-api/media/v4l/v4l2.html
- Linux V4L2 采集接口：https://www.kernel.org/doc/html/v6.4/userspace-api/media/v4l/dev-capture.html
- Linux V4L2 mmap 流式采集：https://www.kernel.org/doc/html/v4.9/media/uapi/v4l/mmap.html
- GStreamer 许可证说明：https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html
- FFmpeg 法律说明：https://www.ffmpeg.org/legal.html
- WebRTC 编码格式说明：https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/WebRTC_codecs
- Qt LGPL 义务说明：https://www.qt.io/development/open-source-lgpl-obligations
- PyQt 商业版/GPL 许可说明：https://riverbankcomputing.com/commercial/pyqt
- LibVNCServer 许可证说明：https://libvnc.github.io/
- Microsoft RDP 基础连接和图形远程协议：https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/5073f4ed-1e93-45e1-b039-6e30c385867c
- kvm-serial 参考项目：https://github.com/sjmf/kvm-serial

## 默认决策

- 第一版桌面图形界面不做全屏，只保留后续扩展位。
- 多个 VNC 客户端同时连接时，第一个连接者获得输入控制权；其他客户端只读观看。
- RFB 性能优化先做脏块检测，再评估 `ZRLE` 或 `Tight/JPEG`。
