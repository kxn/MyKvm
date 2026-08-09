# 阶段 2 能力补齐：统一采集封装、文本键入与 HTTP API 设计

日期：2026-08-01

## 背景与目标

`ipkvm-headless` 已完成阶段 0 的 RFB TCP + noVNC 网页双传输闭环，但视频来源只有 Y4M 循环 mock 源，HTTP 服务只有静态资源和 `/rfb` 路由。本轮补齐阶段 2 的三项软件能力，并把真实视频采集接入无硬件调试路径：

1. **统一采集封装**：真实摄像头（含 OBS 虚拟摄像头，按真实摄像头接口对待）与视频文件伪设备统一为 `FrameSource`，平台后端差异收敛在采集抽象内。
2. **ClientCutText 转键入**：RFB 客户端剪切板文本转模拟键入（独立文本键入服务）。
3. **HTTP API**：`/api/status` 状态接口、`/api/screenshot` 最新帧 JPEG 快照接口。

真实 CH9329 串口、设备选择页、鉴权、TLS 不在本轮范围（硬件到货后按粗粒度设计阶段 1/2 后续处理）。

## 设计原则

- **一切视频源都是 `FrameSource`**：设备、视频文件、循环源只有实现方式之别，没有接口之别；Windows/Linux/macOS 后端差异收敛在采集抽象内，不针对平台堆代码。
- **`FrameSource` 公共接口零播放控制**：摄像头没有 loop/暂停/seek，视频文件伪设备也不暴露这些（内部自动循环是内部策略，不是公共控制）。seek/播放控制如需恢复，加在具体源实现上，不进 `FrameSource` 公共接口。
- **对外统一 BGRA8888**：各源对外始终发布 BGRA8888 帧（RFB 驱动唯一支持格式），格式转换（YUY2/MJPEG→RGB）收敛在采集后端内部，下游 RFB/截图/消费方零改动。
- **新增依赖全部符合许可证策略**：`windows`（DirectShow 绑定）、`windows-core`/`windows-interface`/`windows-implement`（手写 Sample Grabber COM 绑定）、`serde`/`serde_json`、`jpeg-encoder` 均为 MIT/Apache-2.0 全局允许范围。

## 扩展点（当前不实现，仅留位）

- 未来「压缩输出」需在帧源声明原生压缩格式集合、输出端声明接受格式集合、两端交集非空时直接透传零转码（MJPEG/H.264 透传）。`VideoFrame.pixel_format` 已是显式字段、数据为 `Arc<[u8]>`，压缩帧天然可放进帧模型——该基础已就位，本轮不动；协商接口是未来工作，不往 `FrameSource` 堆方法（避免 YAGNI）。
- 播放控制（seek 等）如需恢复，加在具体源实现上，不进 `FrameSource` 公共接口。

## 一、统一采集封装（`ipkvm-video`）

### 1a. `FrameSource` 增加元数据接口

`FrameSource` 现有 `latest_frame()` / `subscribe()` 两个方法保留（非破坏性改动），新增：

```rust
// ipkvm-video 新增
pub enum VideoSourceKind {
    Camera,       // 真实采集设备（含 OBS 虚拟摄像头）
    VideoFile,    // 视频文件伪设备
    Generated,    // 合成源（Y4M 循环、测试 mock 等）
}

pub struct VideoSourceInfo {
    pub kind: VideoSourceKind,
    pub device_name: String,   // 设备名 / 文件名 / 生成源描述
    pub is_loop: bool,         // 播放类源是否循环
}

pub trait FrameSource {        // 现有两个方法保留
    fn latest_frame(&self) -> Option<SharedVideoFrame>;
    fn subscribe(&self) -> FrameReceiver;
    fn source_info(&self) -> VideoSourceInfo;   // 新增
}
```

所有实现（`MockFrameSource`、`LoopingVideoSource`、新增 `CameraSource`、`FileVideoSource`）都实现 `source_info()`。测试用 `MockFrameSource` 的元数据为 `Generated`。

### 1b. 统一源句柄

```rust
pub enum VideoSource {          // 统一句柄：headless 从这里拿 Arc<dyn FrameSource>
    Camera(CameraSource),
    File(FileVideoSource),
    Generated(GeneratedVideoSource),
}
```

headless 只通过 `FrameSource` 方法消费，不看具体变体。

### 1c. 后端统一实现模式

每个后端是一个「发布循环」：采集任务/线程内部 `loop { 拿到一帧 → 填 VideoFrame → 写 RwLock + watch.send_replace }`，与现有 `LoopingVideoSource` 完全同构。后端差异只存在于「怎么拿到那一帧」：Windows→DirectShow、Linux→V4L2（后续）、macOS→AVFoundation（后续）、文件→解码（后续）。

> **2026-08-01 修订**：Windows 后端从 Media Foundation 改为 **DirectShow**。实测根因：OBS 虚拟摄像头是 DirectShow 过滤器，MF 的 `MFEnumDeviceSources` 枚举不到它（Win11 22H2+ 的 MF Frame Server 模式需要 `EnableFrameServerMode` 注册表键，设键后仍不行——OBS 不注册到 MF 设备分支，OBS 官方 issue #13439 已关闭不计划支持）；nokhwa（MSMF 后端）实测同样枚举不到。DirectShow 枚举（`ICreateDevEnum`）能看到 OBS（ffmpeg/腾讯会议/飞书均走此路径）。

### 1d. 摄像头来源格式策略

相机后端请求 NV12 输出（OBS 虚拟摄像头默认偏好 NV12；UVC 摄像头多为 YUY2/MJPEG，Sample Grabber 按需协商），在采集后端内部用官方 BT.601 公式转 BGRA8888。`VideoFrame.pixel_format` 对外始终 BGRA8888（匹配 RFB 驱动唯一支持格式）。

## 二、相机后端（DirectShow）与文件伪设备

### 2a. Windows 相机后端（`ipkvm-video` 新文件，feature `mf` + `cfg(windows)`）

- 枚举：`ICreateDevEnum` + `CLSID_VideoInputDeviceCategory`（DirectShow 系统设备枚举器），能看到 OBS 虚拟摄像头等 DirectShow 过滤器（MF 的 `MFEnumDeviceSources` 看不到，见 1c 修订说明）。
- 打开：moniker → `BindToObject` 出 `IBaseFilter` → Capture Graph Builder + Sample Grabber（`IGraphBuilder::Connect` 智能连接 capture pin → grabber 输入 → grabber 输出 → Null Renderer）→ `IMediaControl::Run`。
- 读帧：`ISampleGrabberCB::BufferCB` 回调模式（qedit.h 自 Win7 SDK 移除，用 `windows-interface`/`windows-implement` 手写 COM 绑定；回调在流线程触发、拷贝帧到共享槽，避免 `GetCurrentBuffer` 缓冲模式阻塞）。
- 发布循环：detached 采集线程 `回调取帧 → NV12→BGRA → 填 VideoFrame → RwLock + watch`，与 `LoopingVideoSource` 同构；`seq`/时间戳单调。`open` 用 `sync_channel` 等初始化完成即返回，不 join 采集循环。
- COM：STA（`COINIT_APARTMENTTHREADED`）+ 线程局部引用计数 `ComInit`（先释放所有 COM 对象再 `CoUninitialize`，MTA + 对象存活时反初始化会访问违例崩溃）。
- 停止：Drop/显式停止 → 置位共享停止标志 → 采集线程退出 → 图对象随线程 drop 释放。
- 跨平台：非 Windows 平台提供「不支持」stub（`CameraSourceError::UnsupportedPlatform`），保证 `verify.sh` 的 Linux/macOS 检查能编译、CLI 一致、错误清晰；V4L2/AVFoundation 后端是后续工作，加变体即可，框架不变。
- 实测验证（2026-08-01）：OBS 虚拟摄像头枚举/打开/1920×1080 BGRA 采帧/`/api/status` 帧增长/`/api/screenshot` 200 JPEG 全链路通过。

### 2b. 文件伪设备（`FileVideoSource`）

- `open(path)` 把视频文件包装成 `FrameSource`，与相机完全同一个接口。
- 本轮内嵌 Y4M 解析（已有 `Y4mAsset`）作为唯一解码路径，解码器做成可扩展点；MP4 等容器后续加后端。
- **内部自动循环播放**（播完回到开头重播，便于持续调试；这是内部策略，不暴露控制接口）。`VideoSourceInfo.is_loop` 为 `true` 如实上报。
- 保留 `LoopingVideoSource` 不动（测试/演示源，8 个集成测试依赖它），新增 `FileVideoSource` 是正式文件源，现有测试零改动。

## 三、ClientCutText → 文本键入（`TextInputService`）

当前实现位于 `ipkvm-session/src/rfb_input/text.rs`。`TextInputService` 是独立异步调度器，但不是独立输入状态机：它不持有 `InputSink`，只负责文本映射、逐字符节流和生成文本输入动作，最终由 `RfbInputPump` 串行提交到主 `InputSink`。

### 结构与数据流

- pump 收到 `RfbServerEvent::CutText` → 校验活动控制者 → 文本通过 mpsc channel 发给独立的 `TextInputService` task（不能阻塞 pump 事件循环，逐字符节流是异步慢操作）。
- 服务逐字符：`字符 → keysym → 键盘映射器 → HID usage + shift 状态`（复用现有 en-US 键盘映射器）→ 生成 `KeyBatch` 动作 → 节流间隔 → 生成 release `KeyBatch` 动作 → 节流间隔。
- pump 在同一个输入事件循环中消费文本动作；过期控制者的 key/release 动作被忽略，结果 notice 仍可上报，便于断开后看到部分键入结果。
- pump 因控制者释放、停止信号或事件源关闭退出时，如果仍有已分发但未完成的文本结果，必须先向文本服务发送取消并继续消费文本动作，直到收到对应 `TextTyped`/`TextInputFailed` 终止 notice；不能依赖固定 sleep 或事件 channel 继续存活。
- **锁定键状态源**：设计为注入点（trait），当前返回「未锁定」假设（硬件未到、GetInfo 不可靠）；未来接 `Ch9329InputSink` 的 GetInfo 查询 LED。
- **节流**：可配置参数，默认按 9600 波特留足余量（每字符约 30ms）；硬件到货后验证实际波特率再调。

### 错误处理

- 不可映射字符（非 ASCII 等）→ 跳过并计入 `chars_skipped`（不是协议失败）。
- pump 应用文本动作时遇到设备/队列错误 → 在主 sink 上 `release_all`、重置键盘/指针 mapper、丢弃剩余文本并记录 `TextInputFailed`。
- 控制者断开/释放时取消进行中的键入；释放动作由 pump 在主 sink 上执行，TextInputService 不再对 sink 克隆做第二次释放。
- 文本键入是「文本转模拟键入」，不是双向剪贴板同步；非 ASCII 文本输入暂不保证；锁定键状态可能竞争（查询后状态仍可能变化）的限制说明。

### 通知

现有 `CutTextIgnored` 换成处理结果 notice（如 `TextTyped { chars_typed, chars_skipped }` / 错误），打印日志。

## 四、HTTP API

### `/api/status`（JSON，最小有用集 + 来源元数据）

```json
{
  "service": { "name": "ipkvm-headless", "version": "0.1.0" },
  "video": {
    "source": { "kind": "camera|file|generated", "device_name": "OBS Virtual Camera", "is_loop": false },
    "frame": { "width": 1920, "height": 1080, "pixel_format": "bgra8888", "seq": 42 }
  },
  "controller": {
    "active": false,
    "client_id": null,
    "transport": null,
    "peer_addr": null,
    "connected_since_ms": null
  }
}
```

- `source` 来自 `FrameSource::source_info()`；`frame` 来自 `latest_frame()`（无帧时为 null）。
- `controller` 来自**门闸扩展**：`GateInner` 加 `watch::Sender<Option<ActiveController>>`，`acquire(transport, peer_addr)` 时写入、释放/终结时清空。TCP/WS 共享同一闸门 → 状态天然跨传输一致，且可独立单测。
- `ActiveController` 结构：`{ client_id, transport: "tcp"|"ws", peer_addr, connected_since_ms }`。

### `/api/screenshot`（`jpeg-encoder`，MIT）

- 最新 BGRA8888 帧 → JPEG（质量 85）→ `image/jpeg`，`Cache-Control: no-store`。
- 无帧 → 503；编码失败 → 500。
- 不做缓存（低频操作，YAGNI）。

### 测试

`web_http.rs` 集成测试断言 `/api/status` JSON 字段、`/api/screenshot` JPEG magic（`FFD8`）、无帧 503。

## 五、headless 接入与 CLI

- `--list-cameras` 枚举设备并退出。
- `--camera <设备名>` 按名选择相机。
- `--assets <目录>` 走文件伪设备（当前 Y4M 循环源的目录加载逻辑移入 `FileVideoSource`，目录内 `.y4m` 按文件名排序循环）。
- 无任何视频参数时默认打开枚举到的第一台摄像头。
- `ipkvm-demo` 保留现有 Y4M 循环源路径不变（CI/演示依赖）。

## 测试与验证

- 单元测试：`FrameSource` 元数据、`FileVideoSource` 循环/定格、`TextInputService` 节流/错误恢复、门闸状态 watch。
- 集成测试：`web_http.rs` 断言 status/screenshot；`headless_process.rs` 断言启动、`--list-cameras`、`--camera` 缺设备错误。
- 真实浏览器闭环（noVNC）跑 `--assets` 文件伪设备路径验证画面/缩放/键入。
- 手工验证（已 2026-08-01 完成）：OBS 虚拟摄像头 → `--camera "OBS Virtual Camera"` → noVNC 看画面 + `/api/screenshot` 截图——已实测通过。真实 USB 摄像头待硬件到货后验证（DirectShow 枚举路径相同，理论上可用）。
- 仓库门禁：`cargo fmt --all --check`、`cargo test --workspace --all-features`、`verify.ps1` 全量（许可证/资源/浏览器）。

## 后续工作（不在本轮）

- 视频文件解码支持 MP4 等容器（解码器可扩展点）。
- Linux V4L2 / macOS AVFoundation 相机后端（加 `CameraSourceError` 变体 + `VideoSource` 变体即可，框架不变）。
- 压缩输出格式协商（透传零转码，见「扩展点」）。
- 真实 CH9329 串口、设备选择页、鉴权、TLS（硬件到货后）。
