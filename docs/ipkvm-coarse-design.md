# my_ipkvm 粗粒度方案与设计草案

日期：2026-07-31

## 目标

做一个机房内软件 IPKVM：一台主控机通过 USB HDMI 采集卡读取目标机控制台画面，通过 CH9329 + CH340 串口线向目标机注入 USB HID 键盘鼠标事件。

目标形态分两版：

- 桌面图形版：类似 VNC 客户端，本机窗口显示采集卡画面，窗口获得焦点时转发键盘和鼠标事件。
- 无头版：后台进程运行，对外提供 VNC/RFB 服务，并内置 noVNC 网页。普通 VNC 客户端和浏览器都能连接。

本设计不基于 Qt/PyQt 实现。`kvm-serial` 只作为协议和产品行为参考，不继承它的 PyQt 图形界面。

## 当前实施状态

截至 2026-07-31，`ipkvm-headless` 已提供库级 RFB TCP 与 WebSocket 传输层：`RfbTcpServer` 服务原生 TCP，`RfbWebSocketService` 提供可组合的 axum `/rfb` 路由。两者共用 `rfb_connection` 连接驱动器、`RfbServerEvent` 事件模型和全局单活动 `RfbConnectionGate`；生产组装必须显式创建一个连接闸门，并把它的副本分别传给两个服务。WebSocket 输入和输出承载连续 RFB 二进制字节流，不赋予 WebSocket 消息边界 RFB 语义。

`/rfb` 默认不要求子协议；请求包含 `binary` 时响应选择该子协议。单条 WebSocket 消息和帧都限制为 `RfbProtocolLimits::max_buffered_input_bytes`，现有活动连接返回 `409 Conflict`，关闭信号、事件接收端关闭、闸门中毒或客户端标识耗尽返回 `503 Service Unavailable`。锁定 noVNC 1.7.0 提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e` 的无子协议线级初始化样本验证初始与增量 `Raw` 更新；独立升级测试覆盖可选 `binary` 子协议，共享驱动测试覆盖 `DesktopSize`。

`HeadlessWebService` 已把项目自有中文控制台页面、固定 noVNC 1.7.0 资源和现有 `/rfb` 组装为单一嵌入式 HTTP 服务。真实 Chrome 自动化已经证明模拟帧像素、桌面与窄视口等比缩放、键盘 HID、缩放后的绝对坐标、按钮顺序、断开释放和重连到达 `RfbInputPump` 后的 `InputSink`。资源来源、逐文件哈希、许可证和浏览器测试依赖也进入统一本机门禁。

正式 `ipkvm-headless` 二进制已通过 CLI 组装真实视频与设备选择：`--camera <名称>` 按 id 或显示名打开 Windows Media Foundation 相机，`--list-cameras` 枚举设备后退出，`--assets <目录>` 使用 Y4M 文件伪设备（与 `--camera` 互斥），未指定视频参数时默认打开枚举到的第一台相机。`FrameSource` 暴露 `source_info` 元数据，`/api/status` 报告帧源与活动控制器状态，`/api/screenshot` 用 `jpeg-encoder` 把最新 BGRA8888 帧编码为 JPEG 快照，RFB `ClientCutText` 通过独立 `TextInputService` 转模拟键入。

上述实现仍不是可控制真实机器的完整无头产品：真实 CH9329 串口尚未实现，键鼠事件进入 `FakeCommandQueue` 后被丢弃；鉴权与 TLS 也未实现。以下设备会话和生产启动描述仍是目标形态，不能当作当前已交付能力。

当前不做完整安全子系统。鉴权、TLS、访问控制、审计、公网暴露、反向代理、VPN、会话权限等都视为后续可叠加层；但默认监听地址固定为 `127.0.0.1`，只有显式配置才允许监听其他地址。

## 总体结论

推荐从头写 Rust 项目。理由：

- CH9329 协议简单，适合做成小而稳定的核心模块。
- Rust 适合把桌面图形界面、串口、视频采集、RFB/VNC、网页服务统一到一个类型安全的工程内。
- 可以默认使用 MIT、Apache、BSD、ISC、Zlib、MPL-2.0 这类可接受依赖，避免 PyQt GPL。
- 如需 LGPL，限定在动态链接的 C/C++ 系统库或媒体库边界，避免普通 Cargo 依赖静态编入主二进制。

无头版不做 RDP 服务端。RDP 是完整桌面远程协议栈，复杂度远高于本项目需要；本项目只有 HDMI 像素流和 HID 输入，和 VNC/RFB 的远程帧缓冲模型天然匹配。

Go 可用于无头后台进程，但不推荐作为桌面主实现。Go 的网页视图类桌面框架会让键盘捕获更接近浏览器限制，和“VNC 裸窗口”目标有冲突。

## 当前范围

当前最小版本包含：

- CH9329 串口协议。
- CH9329 设备探测、信息读取和在线状态。
- 串口波特率手动选择，出厂默认值为 9600。
- USB HID 键盘鼠标映射。
- 键盘状态机：重复按下去重、失焦释放、6KRO 溢出策略。
- 鼠标绝对模式和相对模式。
- USB HDMI 采集卡视频输入。
- 视频尺寸动态变化事件。
- 桌面本地控制台窗口。
- 无头版 VNC/RFB 服务。
- RFB `DesktopSize` 伪编码。
- 内置 noVNC 网页。
- 剪贴板文本转模拟键入。
- 设备枚举和手动选择。
- 视频源元数据接口（`FrameSource::source_info`）。
- Y4M 文件伪设备（硬件到货前验证视频链路）。
- Windows Media Foundation 相机后端（`mf` 功能）。
- 门闸活动控制器状态。
- 单目标机、单采集卡、单串口会话。
- 状态接口和快照接口。

当前最小版本明确不做：

- RDP 服务端。
- Qt、PyQt、PySide。
- GPL 依赖进入主程序。
- 复用 LibVNCServer/TightVNC 代码。
- H.264/WebRTC 低带宽压缩。
- 双向剪贴板同步。
- 多目标机管理。
- 多用户并发控制仲裁。
- 音频。
- 文件传输。
- 虚拟介质。
- ATX 开关或其他硬件电源控制。
- CH9350L 支持。
- 完整安全子系统、鉴权、TLS、公网访问。

## 许可证策略

默认策略：

- Cargo 全局自动允许：MIT、Apache-2.0、Apache-2.0 WITH LLVM-exception、BSD-2-Clause、BSD-3-Clause、ISC、Zlib、Unicode-3.0。
- 可接受但按组件审查：MPL-2.0、LGPL 动态库、LGPL Rust 代码和系统 SDK；必须记录链接、修改与分发义务，不进入 Cargo 全局自动允许列表。
- 不接受：GPL 依赖进入主程序分发包，除非以后明确决定整项目 GPL。
- 不采用：PyQt 免费版，因为它是 GPL/商业版双许可，不是 LGPL。

Rust 具体规则：

- 普通 Cargo 依赖通常静态链接进最终二进制。LGPL Rust 代码不是绝对禁止，但必须按具体 crate 设计重新链接和发布义务；优先拆成动态库或独立进程。
- 固定 `cargo-deny 0.20.2`，默认拒绝未批准许可证、未知注册表和 Git 来源；Git 例外必须锁定完整提交 `rev`。
- FFmpeg/GStreamer 不进入当前最小版本；以后如使用，优先动态加载或动态链接，并在发布包内保留许可证说明、源码获取方式和替换库说明。
- noVNC 核心库是 MPL-2.0，可以作为单独审查的网页前端组件；若修改 noVNC 自身文件，修改过的文件按 MPL-2.0 保留。
- MJPEG 解码优先评估 libjpeg-turbo。其许可证属于 BSD 风格/IJG 风格，可进入白名单，但 Rust 绑定和分发方式仍需单独审计。

## 运行时决策

I/O 运行时固定使用 tokio：

- HTTP、RFB over TCP、RFB over WebSocket、串口任务、后台状态推送都按 tokio 生态实现。
- 后续 HTTP 优先评估 axum，WebSocket 优先评估 tokio-tungstenite，串口优先评估 tokio-serial。
- `ipkvm-core` 保持同步、无 tokio 依赖；它只定义协议、输入状态机、错误和串口写入队列接口。
- `ipkvm-video` 可以暴露 Tokio 的 `watch` 订阅接口，用于多个消费者读取同一最新帧。
- 异步任务边界放在 `ipkvm-headless`、真实采集后端和真实串口后端，不把 async 签名扩散到纯协议类型。

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

    core --> serial["串口写入队列"]
    core --> hid["HID 映射"]
    core --> mouse["鼠标模式和坐标映射"]
    serial --> ch9329["CH9329 协议"]

    video --> capture["采集后端"]
    video --> frames["帧源"]
    video --> resize["尺寸变化事件"]

    capture --> win["Windows Media Foundation"]
    capture --> linux["Linux V4L2"]
    capture --> mac["macOS AVFoundation"]
```

核心边界：

- `ipkvm-core` 不依赖图形界面、不依赖网页服务、不依赖 VNC、不依赖具体视频库。
- `ipkvm-video` 不知道 CH9329，只产出视频帧流和尺寸变化事件。
- `ipkvm-session` 组合视频帧源、尺寸变化事件和输入接收端。
- `ipkvm-desktop` 使用会话，负责窗口、输入事件、视频显示、DPI 换算和桌面鼠标捕获。
- `ipkvm-headless` 使用会话，负责后台进程生命周期、设备选择、HTTP/noVNC、RFB 监听、状态接口和快照接口。
- `ipkvm-rfb` 是传输无关的 RFB 协议核心，不依赖 Tokio、TCP、WebSocket 或应用生命周期。
- RFB TCP 和 WebSocket 传输位于 `ipkvm-headless`，共用 `ipkvm-rfb` 的状态机和编码逻辑。
- 所有输入事件进入同一个串口写入队列，避免来自多个入口的 CH9329 帧交错。

建议模块拆分：

```text
ipkvm-core       CH9329 帧、HID 报告、坐标换算、输入状态机、串口写入队列接口
ipkvm-video      采集设备枚举、格式选择、视频帧流、尺寸变化事件
ipkvm-session    把视频帧源、尺寸变化事件和输入接收端组合成控制台会话
ipkvm-rfb        RFB 握手、消息解码、像素转换和帧缓冲编码
ipkvm-desktop    本地图形界面
ipkvm-headless   RFB TCP/WebSocket、后台进程、HTTP、noVNC、状态接口、配置
```

## 核心接口

### `ipkvm-core`

职责：

- CH9329 帧构造：`57 AB + addr + cmd + len + data + checksum`。
- CH9329 设备探测：打开候选串口后使用读信息命令探测合法回包。
- CH9329 在线状态：读信息命令可返回芯片版本、USB 枚举状态和 Num Lock、Caps Lock、Scroll Lock 指示灯状态。
- CH9329 波特率：默认使用出厂值 9600，允许用户手动选择其他值；硬件到货前不自动改写芯片参数。
- CH9329 应答：协议层支持正常和异常应答解析；真实传输层是否逐命令等待应答，由硬件诊断结果决定。
- 键盘 HID 报告：维护当前按下的修饰键和最多 6 个普通键。
- 键盘状态机：重复 `key_down` 去重，失焦/断连/控制者切换时触发 `release_all()`。
- 6KRO 溢出策略：第 7 个普通键按下时拒绝新键，保留当前已按下键，并记录丢弃计数。
- 鼠标报告：绝对坐标、相对位移、滚轮、按钮状态。
- 鼠标模式：绝对模式和相对模式可切换。
- 坐标换算：客户端坐标 -> 视频源像素坐标 -> CH9329 的 0..4095 坐标。
- 设备状态：串口连接状态、协议统计、最后发送时间、错误状态、丢弃输入计数。

关键接口草案：

```text
SerialDevice
  list() -> Vec<SerialPortInfo>
  probe(path, baud_candidates) -> ProbeResult
  open(path, baud) -> SerialSession

CommandQueue
  enqueue_batch(frames)
  stats()

Ch9329Device
  read_info()
  set_baud_rate(baud)
  send_keyboard(report)
  release_all_keys()
  send_mouse_absolute(buttons, x, y, width, height, wheel)
  send_mouse_relative(buttons, dx, dy, wheel)

InputSink
  set_mouse_mode(mode)
  handle_key(event) -> Result
  handle_pointer(event) -> Result
  release_all() -> Result
```

`InputSink` 契约：

- `KeyEvent` 仍区分按下和释放，但只通过 `handle_key` 进入 sink，避免 `key_down(KeyEvent::Up)` 这种自相矛盾调用。
- `PointerEvent::AbsoluteMove` 使用帧缓冲像素坐标和帧尺寸，不直接传 CH9329 的 0..4095 坐标。
- CH9329 绝对坐标换算留在 sink 实现内，桌面端和 RFB 端复用同一路径。
- 滚轮方向约定为正数向上、负数向下。
- 滚轮值的语义是滚轮齿数；各平台的原始滚轮单位由入口适配层归一化。
- 所有输入方法返回 `Result`，队列关闭、不可映射按键、6KRO 溢出等错误必须能传回 UI 或 RFB 状态层。
- `Result::Ok` 只表示有序命令批次已被本进程接受，不表示 CH9329 已执行。
- 文本粘贴转模拟键入不放进物理 `InputSink`，阶段 2 由独立文本键入服务实现。

CH9329 协议与输入核心的详细设计见：

- `docs/superpowers/specs/2026-07-31-ch9329-protocol-input-core-design.md`

第一版只支持 CH9329。CH9350L 作为后续扩展，不进当前最小版本。

### `ipkvm-video`

职责：

- 枚举采集设备。
- 枚举和选择格式：分辨率、帧率、像素格式、是否 MJPEG。
- 自动格式选择：1080p 优先 MJPEG；未压缩 YUY2/NV12 只在带宽和帧率可接受时选择。
- 输出统一的视频帧流。
- 对外提供最新帧缓存和订阅接口。
- 当采集卡输出尺寸变化时发出尺寸变化事件。

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
  pixel_format: YUY2 | NV12 | Bgra8888 | MJPEG | H264 | Unknown

VideoFrame
  seq
  timestamp
  width
  height
  stride
  format
  data: Arc<[u8]>

FrameSource
  latest_frame() -> Option<Arc<VideoFrame>>
  subscribe() -> FrameReceiver

VideoEvent
  Frame(frame)
  SizeChanged(width, height)
  SignalLost
  SignalRestored(width, height)
```

视频处理分三阶段：

- 第一阶段：本地桌面显示，采集未压缩帧或 MJPEG 解码后的帧，直接渲染。
- 第二阶段：RFB 服务使用 RGBX/BGRA 帧缓冲。第一版允许整帧更新、尺寸变化通知和帧率限制。
- 第三阶段：补充 RFB 压缩编码或独立 WebRTC/H.264/VP8，不自己实现视频编码器。

帧所有权模型：

- `VideoFrame` 使用共享所有权，避免桌面渲染器和多个 RFB 客户端复制 1080p 大帧。
- `seq` 是单调递增帧序号，用于慢客户端丢帧、重复帧判断和后续脏块检测。
- `timestamp` 是单调时间戳，不表示墙钟时间。
- `stride` 必须显式记录，不能假设行宽等于 `width * bytes_per_pixel`。
- `VideoFrame` 不做整帧字节相等比较。

采集卡现实约束：

- USB 2.0 廉价采集卡在 1080p 下通常需要 MJPEG 才能达到可用帧率。
- 廉价采集卡的 EDID 通常固定且不可修改，目标机可用分辨率上限会被采集卡限制。
- 硬件到货后必须记录采集卡在 BIOS、引导加载器、操作系统阶段的分辨率变化序列。

### `ipkvm-session`

职责：

- 管理一个目标控制台会话。
- 固定本次会话的视频设备和串口设备。
- 允许本次会话的视频尺寸动态变化。
- 将输入入口发来的键鼠事件统一送入 `InputSink`。
- 将采集帧和尺寸变化事件统一发布给桌面渲染器和 RFB 服务。
- 维护当前控制者、查看者、帧率、串口统计、丢帧计数、最后输入时间。

会话配置至少包含：

- 视频设备 ID。
- 串口路径。
- 选定视频格式或格式选择策略。
- 波特率，默认 9600。
- 键盘布局，当前默认 `en_US`。
- 鼠标模式，默认绝对模式。

会话约束：

- 当前最小版本一个进程只管理一个活动会话。
- 会话启动后视频设备固定；分辨率变化不需要重连。
- 当前最小版本支持多个查看者，但只有一个控制者。
- 串口断开时视频继续运行，输入返回错误并尝试触发 `release_all()`。
- 视频断开时 RFB/图形界面保留连接，但显示合成错误帧或黑帧。

## 桌面图形版设计

推荐技术：

- 窗口和事件循环：`winit`
- 界面：`egui` 或极简自绘设置页
- 渲染：`wgpu` 或 `pixels`

交互：

- 启动显示设置页：选择视频设备、分辨率、串口、波特率、键盘布局、鼠标模式。
- 连接成功后进入控制台视图：黑底视频裸窗口，少量覆盖状态可隐藏。
- 窗口按视频比例缩放。用户可拖动大小，但内容保持比例，不变形。
- 视频尺寸变化时，窗口比例和坐标换算立即更新。
- 焦点在视频视图内时，捕获可捕获的键盘事件并发送。
- 视频视图失去焦点时无条件发送 `release_all()`。
- 主控系统会优先处理 `Alt+Tab`、`Win`、`Ctrl+Alt+Del` 等快捷键；这类组合键通过菜单或屏幕键盘发送。
- 绝对鼠标模式：只有指针在视频区域上方时，发送移动、按钮、滚轮事件。
- 相对鼠标模式：桌面端使用 `winit` 的鼠标捕获或限制能力，按相对位移发送 CH9329 相对鼠标报告。
- 相对鼠标模式使用 1:1 无加速映射；目标机侧速度/加速度导致的漂移视为硬件和操作系统现实约束。
- 点击前先发送一次位置更新，再发送按钮事件，避免在旧坐标点击。
- 提供菜单或屏幕键盘发送 `Ctrl+Alt+Del`、F 键、导航键、释放所有按键、截图。
- 坐标换算必须使用物理像素和 DPI 缩放因子，单元测试覆盖 1.25、1.5 这类非整数缩放。

桌面版第一版不做全屏，只保留后续扩展位。YUY2/NV12 到 RGB 的 GPU shader 转换作为 `wgpu` 路径优先方案，`pixels` 软渲染路径作为兜底。

## 无头版 VNC/网页设计

### 对外入口

完整无头产品计划提供两个入口：

```text
TCP 5900       标准 RFB/VNC 服务，供普通 VNC 客户端使用
HTTP 6080      设备选择页、noVNC 静态页面、RFB WebSocket 入口
```

完整无头产品的默认绑定：

- 默认监听 `127.0.0.1`。
- 只有显式设置 `--bind` 或配置文件字段时才监听其他地址。
- 这不是完整安全方案，只是避免默认暴露无鉴权控制台。

网页不再自研远控协议。它只负责：

- 枚举视频和串口设备。
- 创建或重启控制台会话。
- 嵌入 noVNC 客户端。
- 把 noVNC 连接到本进程的 RFB WebSocket 入口。
- 展示当前状态、快照、鼠标模式、控制者状态和特殊按键。

建议 HTTP 路由：

```text
GET  /                  设置页
GET  /api/devices       视频和串口设备列表
POST /api/session       启动或重启会话
GET  /api/status        当前帧率、串口统计、最后输入时间、丢帧计数、活动客户端数（已实现：帧源与控制器状态）
GET  /api/screenshot    最新帧 JPEG 快照（已实现，`jpeg-encoder` 编码质量 85）
GET  /vendor/novnc/...  noVNC 静态资源
GET  /rfb               基于 WebSocket 的 RFB
GET  /licenses/         第三方许可证与源码入口
```

noVNC 静态资源打包现状：

- 仓库保存固定的 noVNC 1.7.0 完整 npm 发布包、来源元数据和逐文件哈希清单。
- `include_dir 0.7.4` 在编译时嵌入 noVNC 与项目页面，运行时不需要 Node.js 或外部静态目录。
- 项目页面只导入 noVNC 核心库，不复制上游完整应用界面。
- `/licenses/` 提供 noVNC、MPL、pako 和 DES 许可证与嵌入源码入口。
- 资源门禁和真实浏览器闭环已经加入 `scripts/verify.ps1`。

### RFB/VNC 服务

当前最小版本实现 RFB 3.8 子集：

- `ProtocolVersion` 握手。
- `SecurityType None`。完整安全子系统不在当前范围内。
- `ClientInit` / `ServerInit`。
- `SetPixelFormat`。
- `SetEncodings`。
- `FramebufferUpdateRequest`。
- `FramebufferUpdate`。
- `DesktopSize` 伪编码。
- `KeyEvent`。
- `PointerEvent`。
- `ClientCutText` 转模拟键入。

当前最小版本的编码支持：

- 必须支持 `Raw`。
- 必须支持 `DesktopSize` 伪编码，用于目标机启动过程中分辨率变化。
- 初始实现整帧更新，限制帧率，先保证行为正确。
- 识别但不实现的伪编码必须正确忽略，不能断连；测试覆盖未知伪编码和 `EnableContinuousUpdates` 请求。
- 后续再做脏块检测，减少静态画面带宽。
- `ZRLE`、`Tight/JPEG`、H.264 扩展都不进当前最小版本。

像素格式：

- 内部统一为 `RGBX8888` 或 `BGRA8888`。
- RFB 根据客户端像素格式做必要转换。
- 第一版可只优化常见 32 位真彩格式，其他格式走慢路径或拒绝。

WebSocket 兼容：

- RFB over WebSocket 必须发送二进制 WebSocket 帧。
- 如果客户端请求 `Sec-WebSocket-Protocol: binary`，服务端可以回应该子协议以兼容旧代理和旧客户端。
- 现代 noVNC 不应强制依赖非标准 `binary` 子协议；实际行为以当前锁定的 noVNC 版本测试为准。

输入映射：

- RFB 的 `KeyEvent` 使用 keysym；当前已经实现到 USB HID 用法编号的状态化映射，并已接入单活动控制者事件泵。
- 当前映射器保证 en-US 可打印 ASCII、左右修饰键、F1-F12、方向键、导航键、系统键和小键盘常用键。
- 字符 keysym 决定最终大小写和符号；映射器会按需合成、抑制和恢复 Shift，并通过原子键盘批次提交。
- OS 自动重复产生的重复 `key_down` 会被去重；未知释放、不可映射键、6KRO 和 sink 失败具有确定结果且不提交部分状态。
- 非 ASCII 文本输入暂不保证。
- `ClientCutText` 和桌面粘贴只做“文本转模拟键入”，不是双向剪贴板同步。
- CH9329 `GetInfo` 可以读取目标机 Num Lock、Caps Lock、Scroll Lock LED 状态；文本键入前可以查询并据此选择修饰键。
- 锁定键状态仍可能在查询后被目标机或其他键盘改变，因此文本键入必须保留“状态可能竞争”的限制说明。
- RFB 的 `PointerEvent` 坐标直接对应帧缓冲像素坐标，由协议核心按厂家公式 `floor(4096 * coordinate / extent)` 映射到 CH9329 的 0..4095 合法范围。
- 当前已实现按钮 1 到 5：按钮 1、2、3 分别映射左键、中键、右键，按钮 4、5 的上升沿分别映射垂直滚轮向上、向下。
- 水平滚轮和侧键对应的按钮位 5 到 7 当前不注入 CH9329，但映射结果会显式报告被忽略的位，避免静默误判为已支持。
- 坐标超出当前帧缓冲或帧尺寸为零时拒绝整条消息，不裁剪到屏幕边缘，也不提交按钮、滚轮或 sink 状态。
- 一条 RFB 指针消息中的移动、按钮差分和滚轮通过一个原子指针批次提交；映射器和 CH9329 sink 在校验或队列失败后均保持可重试状态。
- Pointer 事件在 RFB 协议核心中固化客户端输入坐标时期，不由事件泵读取最新视频尺寸；跨越 `DesktopSize` 的半包直到解码边界清空后才切换时期。
- 不支持的 keysym 和 Shift 冲突产生可观测拒绝，事件泵继续处理后续输入和断线释放；sink 或生命周期错误则保留原事件并停止循环，调用方可以修复后重试。
- 控制者断线或事件发送端关闭时调用 `release_all()`；只有释放成功后才清空控制者和两个 mapper，失败时保留完整软件状态。
- 网页相对鼠标模式需要浏览器 Pointer Lock。若 noVNC 集成无法直接提供相对位移，则需要定制 noVNC 页面层；这项不假定 noVNC 原生完成。

会话和并发：

- 目标最小产品支持多个查看者观察同一帧缓冲，但当前已实现的 RFB TCP 服务端仍是顺序单活动连接，尚不支持并发查看者。
- 当前事件泵把这条唯一活动 RFB 连接作为控制者，并严格拒绝不符合顺序服务端契约的第二控制者事件。
- 多查看者进入实现范围后，由 `ipkvm-session` 负责跨 RFB、Web 和桌面入口的控制权仲裁；不能把当前 RFB 事件泵当成完整会话仲裁器。
- 控制者断开、事件源关闭或未来控制权切换时，必须先成功发送 `release_all()`。
- 未来每个查看客户端有自己的帧更新节流和反压处理。慢客户端只拿最新帧，旧帧直接丢弃。

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

3. 采集卡 MJPEG 透传和解码优化
   - 如果 UVC 设备已经输出 MJPEG，优先避免 CPU 重新编码。
   - 解码优先评估 libjpeg-turbo，不引入完整 FFmpeg。
   - 可另做原生网页视频流或 RFB JPEG 路径。

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
   - 视频采集用 DirectShow（2026-08-01 实测：MF 的 `MFEnumDeviceSources` 枚举不到 OBS 虚拟摄像头等 DirectShow 过滤器，OBS 官方 issue #13439 确认不计划支持 MF FrameServer；DirectShow 枚举覆盖 OBS/腾讯会议/飞书/ffmpeg 全部可见设备）。
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

已完成：

- 建立 Rust 工作区。
- 固定 tokio 作为 I/O 运行时。
- 建立 `[workspace.dependencies]` 集中管理外部依赖版本。
- 建立本地一键自动化验证脚本，覆盖文本编码、Rust 格式、全工作区测试、Clippy、Rust 文档和 Git 差异。
- 固定 `cargo-deny 0.20.2`，建立依赖许可证分级、来源限制、临时负向策略测试和当前锁定依赖图审计。
- 完成 CH9329 命令帧、类型化 HID 报告、应答解析和增量解帧，并覆盖协议金样、错误恢复和性质测试。
- 写键盘状态机测试：重复按下去重、6KRO 溢出、释放所有键。
- 写鼠标状态机测试：按钮组合、绝对坐标、相对位移拆包、模式切换和释放。
- 写坐标换算测试，覆盖厂家示例；桌面适配层后续覆盖非整数 DPI 缩放。
- 写模拟视频源，支持尺寸变化事件。
- 编写模拟命令队列和有序批次提交测试。
- `ipkvm-video` 使用 `mock` 功能开关提供测试帧源。
- `ipkvm-core` 使用 `mock` 功能开关提供模拟命令队列。
- 将会话默认串口波特率设为 CH9329 出厂值 9600。
- 完成传输无关的 RFB 3.8 `None` 握手、客户端消息增量解码、`Raw` 编码和 `DesktopSize` 伪编码，并覆盖协议金样、任意分片和性质测试。
- 未知正数 encoding 和负数 pseudo-encoding 会被保留且不使连接失败；未知客户端消息类型因无法确定长度而进入失败终态，不尝试扫描重同步。
- 完成使用模拟 BGRA8888 帧源的单活动客户端 RFB TCP 库闭环，并以真实回环 TCP 自动验证握手、Raw、DesktopSize、像素格式协商、输入事件、事件反压、关闭和断线后继续监听。
- 完成原子键盘批次：一次映射产生的多个 HID 状态变化只提交最终报告，6KRO 或队列失败时不提交部分状态。
- 完成 en-US RFB keysym 到 HID 的状态化键盘映射，覆盖可打印 ASCII、Shift 合成和抑制、左右修饰键、导航键、F1-F12、系统键、数字小键盘、别名引用和确定性错误。
- 完成原子指针批次：一条输入消息中的移动、按钮和滚轮报告按序提交，任一校验或队列错误都不提交部分状态。
- 完成 RFB `PointerEvent` 的按钮掩码、垂直滚轮边沿和绝对坐标状态映射，并覆盖真实 CH9329 sink 的边界与双层回滚。
- 完成 RFB 指针输入坐标时期：动态尺寸和跨半包输入使用客户端已公告尺寸，不读取最新视频尺寸补算。
- 完成单活动 RFB 控制者事件泵，覆盖输入拒绝、生命周期错误、原事件返还、断线和事件源关闭释放。
- 使用真实回环 TCP、模拟 BGRA 帧和 `Ch9329InputSink<FakeCommandQueue>` 自动验证键盘、指针与最终释放闭环。
- 完成双分辨率 Y4M 循环播放 mock 帧源（`ipkvm-video` 的 `mock` 功能）与 `ipkvm-demo` 演示二进制，并用独立第三方 VNC 客户端 vncdotool 1.3.0 验证 RFB TCP 画面和 `DesktopSize` 动态分辨率切换；自动化回归测试见 `crates/ipkvm-headless/tests/rfb_dynamic_resolution.rs`。
- 完成 RFB WebSocket 传输：axum `/rfb` 入口、可选 `binary` 子协议、锁定 noVNC 1.7.0 提交的线级初始化样本、TCP/WS 跨传输互斥与取消安全连接闸门。
- 完成嵌入式 HTTP 服务，固定并分发 noVNC 1.7.0 资源。
- 使用真实 Chrome 自动验证 noVNC 页面、模拟帧像素、两种视口等比缩放、键盘、绝对指针、按钮、释放和重连闭环。
- 建立 noVNC 来源、逐文件完整性、许可证与浏览器依赖锁文件的自动门禁。
- 完成正式 `ipkvm-headless` 后台进程：同时提供 RFB TCP（5900）和嵌入式 noVNC 网页 + RFB WebSocket（6080），共享单活动控制者连接闸门；硬件到货前使用 Y4M 循环播放模拟帧源和 `FakeCommandQueue`，`headless_process` 集成测试覆盖 TCP banner、WS 握手、静态资源、gate 互斥和干净关闭。
- 完成视频源能力：`FrameSource::source_info` 元数据接口（类型、显示名、来源标识）覆盖模拟帧源、文件伪设备和相机源；Y4M 文件伪设备 `FileVideoSource` 按文件名排序循环播放并支持分辨率切换；Windows Media Foundation 相机后端（`mf` 功能）提供 `list_cameras` 枚举与 `CameraSource::open` 采集循环，`camera_probe` 示例自动化执行设计文档手工验证步骤（枚举、打开、帧输出、fps）。
- 完成门闸控制器状态：连接闸门暴露活动 RFB 控制器标识与连接时间，供 `/api/status` 使用。
- 完成独立文本键入服务 `TextInputService`：RFB `ClientCutText` 文本经键盘映射器转为模拟键入序列，与物理 `InputSink` 解耦；当前假设锁定键未按下，锁定键状态源为注入点，待硬件接入后实现 GetInfo 查询。
- 完成 HTTP 状态与快照接口：`GET /api/status` 输出帧源、控制器与错误状态；`GET /api/screenshot` 输出最新帧 JPEG（质量 85）；`jpeg-encoder` 的 IJG 例外按许可证策略完整记录。headless CLI 提供 `--list-cameras`、`--camera <名称>`、`--assets <目录>`，未指定视频参数时默认打开第一台相机。

待完成：

- 使用锁定版本的第三方普通 VNC 客户端验证兼容性。
- 有明确维护的可用 runner 后，再设计并启用 Gitea Actions；在此之前以本地自动化验证结果作为 PR 验收证据。

### 阶段 1：桌面本地最小版本

- Windows 串口枚举和打开。
- CH9329 设备探测、`GetInfo` 和在线状态。
- 默认以 9600 打开；支持手动选择其他波特率。
- 使用诊断工具验证应答、锁定键 LED 和 115200 后，再决定是否增加自动切换。
- Windows 视频设备枚举。
- 自动格式选择，1080p 优先 MJPEG。
- 选择设备后进入控制台窗口。
- 显示 720p/1080p 视频。
- 视频尺寸变化时更新窗口比例和坐标换算。
- 键盘普通键、修饰键、F1-F12、方向键、导航键转发。
- 键盘重复按下去重。
- 失焦、窗口关闭、串口断开时释放所有键。
- 鼠标绝对移动、相对移动、左键、右键、中键、滚轮转发。
- 桌面端绝对/相对鼠标模式切换。
- 释放所有键、发送 `Ctrl+Alt+Del`、截图。
- 屏幕键盘覆盖层，至少支持特殊组合键和 F 键。

### 阶段 2：无头 VNC/网页最小版本

- 无头后台进程。
- 默认绑定 `127.0.0.1`。
- HTTP 设置页。
- noVNC 静态资源内嵌。
- 基于 TCP 的 RFB。
- 基于 WebSocket 的 RFB。
- `Raw` 帧缓冲更新。
- `DesktopSize` 伪编码。
- RFB `KeyEvent` 到 HID 映射。
- RFB `PointerEvent` 到 CH9329 绝对鼠标。
- RFB `ClientCutText` 转模拟键入（阶段 0 已由 `TextInputService` 完成）。
- 网页特殊按键和基础屏幕键盘。
- 单控制者策略。
- 慢客户端丢帧策略。
- `/api/status` 状态接口（阶段 0 已完成）。
- `/api/screenshot` 最新帧 JPEG 快照接口（阶段 0 已完成）。

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
- libjpeg-turbo 解码路径验证。
- Windows H.264 Media Foundation 编码器验证。
- WebRTC 验证。
- 桌面端 YUY2/NV12 到 RGB 的 GPU shader 转换。

### 阶段 5：运维化、安全叠加和辅助能力

- 配置文件。
- Windows 服务或托盘程序。
- 多设备配置档案。
- 连接健康检查。
- 发布包和许可证清单。
- Wake-on-LAN。
- 输入事件日志和审计。
- 帧录制。
- 鉴权、TLS、访问控制、审计、反向代理/VPN 部署文档。

## 风险排除表

当前阶段通过收窄范围和前置关键协议能力排除这些风险：

| 风险点 | 当前设计处理 |
| --- | --- |
| RDP 协议复杂度 | 不做 RDP 服务端，只保留未来调研可能 |
| PyQt/Qt 许可风险 | 不使用 Qt/PyQt/PySide |
| GPL 传染风险 | 不使用 LibVNCServer/TightVNC 等 GPL 代码 |
| 浏览器远控界面自研风险 | 网页版复用 noVNC |
| 自研 WebSocket 输入协议风险 | 无头版输入走 RFB `KeyEvent`/`PointerEvent` |
| BIOS 到 OS 过程分辨率变化 | 当前最小版本支持视频尺寸变化事件和 RFB `DesktopSize` |
| BIOS 绝对鼠标不兼容 | 当前最小版本支持相对鼠标模式 |
| 按键卡住 | 失焦、断连、控制者切换时发送 `release_all()` |
| OS 自动重复按键 | 键盘状态机去重重复 `key_down` |
| 6KRO 溢出 | 拒绝第 7 个普通键并记录丢弃计数 |
| 多入口串口帧交错 | 所有输入经由单一串口写入队列 |
| H.264/WebRTC 工程复杂度 | 不进当前最小版本 |
| 视频编码器许可风险 | 当前最小版本不引入 FFmpeg/GStreamer |
| RFB `Raw` 带宽风险 | 当前最小版本用于验证闭环，限制帧率；性能优化后置 |
| 键盘布局差异 | 当前最小版本只承诺 `en_US` HID 映射 |
| Caps Lock/NumLock 状态竞争 | 使用 `GetInfo` 读取 LED 状态；文本键入仍说明查询后状态可能变化 |
| 多客户端输入冲突 | 当前最小版本单控制者 |
| 设备热插拔复杂度 | 当前最小版本断开后手动重连 |
| CH9350L 差异 | 当前最小版本只支持 CH9329 |
| 安全设计复杂度 | 当前不做安全子系统，默认监听 `127.0.0.1` |

仍需硬件验证的问题：

- 廉价 HDMI 采集卡实际支持的格式、帧率、MJPEG 行为和延迟。
- DirectShow 对真实采集卡（真实 USB/HDMI 摄像头）的枚举、格式选择和帧读取行为（DirectShow 路径已实现，OBS 虚拟摄像头实测通过，真实硬件到货后验证）。
- 目标机 BIOS/UEFI/引导菜单对 CH9329 绝对鼠标的支持情况，相对模式回退是否必要。
- CH9329 成品线默认波特率、工作模式、命令应答、读信息命令和 115200 支持情况。
- 目标机从 BIOS 到 OS 全过程中采集卡输出分辨率的变化序列。
- 采集卡 EDID 锁定的分辨率上限。
- BIOS/UEFI 环境下目标机对 CH9329 模拟键盘的兼容性。

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
- noVNC 固定提交：https://github.com/novnc/noVNC/tree/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e
- noVNC 接口文档：https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/docs/API.md
- noVNC 嵌入文档：https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/docs/EMBEDDING.md
- noVNC 服务端要求：https://novnc.com/noVNC/
- WebSocket 子协议头说明：https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-WebSocket-Protocol
- libjpeg-turbo 许可证：https://github.com/libjpeg-turbo/libjpeg-turbo/blob/main/LICENSE.md
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
- 鼠标默认使用相对模式（BIOS/启动菜单把绝对 HID 固定映射到 1024×768，
  绝对模式无法覆盖全屏）；进系统后按 Ctrl+Alt+M 切到绝对模式。
- 文本转模拟键入只承诺 `en_US` 可映射字符；当前假设锁定键未按下，锁定键状态源为注入点，待硬件接入后实现 GetInfo 查询。
- CH9329 串口默认使用 9600；115200 只在硬件诊断确认后考虑自动启用。
- 串口波特率连接时自动扫描（115200→57600→38400→19200→9600，GetInfo 应答即用，
  单档 300ms 超时），可关闭；扫描失败回退手动配置值。
- 桌面指针发送限频约 30Hz 并做位置/掩码去重；按键状态变化立即发送。
- 视频新帧到达时请求重绘，空闲无输入时画面保持刷新。
- headless 自动恢复：输入泵因串口写失败退出、或视频从未出帧超过 5s 时，
  按 1s→2s→…→30s 指数退避重建会话；视频曾出帧后停滞只报告不重启
  （目标机重启场景，避免抢串口）。
- `/api/status` 暴露 `video.stalled`、`video.frame.last_frame_ns` 与
  `session.input_offline`（离线原因与时间）。
- RFB 性能优化先做脏块检测，再评估 `ZRLE` 或 `Tight/JPEG`。

## 桌面远程输入模式

- 视频面板聚焦且窗口在前台时进入远程输入模式：Tab/方向键/Esc 全部转发远端，
  不参与 egui 焦点导航（`Memory::set_focus_lock_filter`）。
- 退出远程模式：点击视频外本地 UI、按 Ctrl+Alt+K（本地拦截，不转发远端）、
  或窗口失焦（Alt+Tab 切走）。退出时执行 release_all 并复位本地输入状态。
- 桌面控制器事件提交无损：事件通道满时暂存待发送队列，通道恢复后按 FIFO
  补发，不丢按键 down/up；发送失败在状态栏可见。
- 鼠标支持绝对/相对两种模式：相对模式锁定并隐藏本地光标，按帧像素增量发送，
  不依赖目标机分辨率（BIOS/启动菜单用）；Ctrl+Alt+M 切换并通过重连应用；
  灵敏度可在高级设置调整（默认 1.0）。
- 滚轮通过相对事件通道发送，绝对与相对模式均可用。
