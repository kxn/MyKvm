# DirectShow 纯 sink filter 相机采集（OBS 虚拟摄像头）—— 设计与决策记录

日期：2026-08-02
关联模块：`crates/ipkvm-video`（`camera`、`dshow_sink`）
背景：阶段 2 把相机后端从 Media Foundation 切到 DirectShow 后，OBS 虚拟摄像头能枚举但无法持续采集——Sample Grabber 回调只触发 1 帧，自研 sink filter 在 `RenderStream` 阶段段错误崩溃。

## 1. 问题与根因

### 1.1 为什么不用 Media Foundation

Media Foundation 的 `MFEnumDeviceSources` 枚举不到 OBS 虚拟摄像头（OBS 只注册了 DirectShow 过滤器，未注册 MF 源）。改用 DirectShow 系统设备枚举器（`ICreateDevEnum` + `CLSID_VideoInputDeviceCategory`）后能枚举到。

### 1.2 为什么不用系统 Sample Grabber

实测系统 Sample Grabber（`CLSID_SampleGrabber` + `ISampleGrabberCB`）与 OBS 虚拟摄像头不兼容：
- 回调模式（`SetCallback(1)` BufferCB）只触发第 1 帧，之后不再回调；
- 缓冲模式（`SetBufferSamples(TRUE)` + `GetCurrentBuffer`）连接不上。

FFmpeg 的 `libavdevice/dshow` 不用 Sample Grabber，而是用**自研纯 sink filter**（`IBaseFilter` + `IPin` + `IMemInputPin`），在输入 pin 的 `Receive` 里拷数据。这是被验证能持续收 OBS 帧的方式。本实现完全照搬 FFmpeg 的 `libdshow_filter.c` / `libdshow_pin.c` 逻辑。

### 1.3 前一版 sink filter 在 `RenderStream` 段错误的根因

DirectShow 图管理器在 `RenderStream` 期间会回调 sink 的若干 COM 方法并接管返回的出参。windows-rs（0.61）的 vtable shim 对「返回 `Result` 的方法」在 `Err` 分支**不写出参**，调用方拿到栈上垃圾指针后 `CoTaskMemFree` / 解引用即段错误。前一版 sink filter 的确定性 bug（按嫌疑排序）：

1. **`IPin::QueryId` 返回 `Err`**：shim 不写 `PWSTR` 出参，图管理器拿到栈垃圾 `CoTaskMemFree` 崩溃。必须返回 `CoTaskMemAlloc` 分配的 UTF-16 `"In"`。
2. **`IPin::ConnectionMediaType` 返回 `Ok(())` 但不写 `AM_MEDIA_TYPE`**：把调用方栈上未初始化结构原样返回，下游读 `pbFormat`/`pUnk` 是 UB。必须深拷贝缓存的媒体类型给调用方。
3. **`IBaseFilter::QueryFilterInfo` 不写 `FILTER_INFO`**：`achName` 是 `WCHAR[128]`，垃圾内容被图管理器使用即崩。必须 `ZeroMemory` + 回填 `pGraph`。
4. **`IPin::EnumMediaTypes` 返回 `E_NOTIMPL`**：协商路径期望「空但有效」的 `IEnumMediaTypes`（`Next` 返 `S_FALSE`），而非 `E_NOTIMPL`。FFmpeg 返回 `ff_dshow_enummediatypes_Create(NULL)`。
5. **`IPin::QueryPinInfo` 的 `pFilter` 回填 `NULL`**：图管理器连接协商时调 `QueryPinInfo`，期望拿到所属 filter 的 AddRef 副本；返回 NULL 在后续处理时崩溃。必须由 camera 层在 `AddFilter` 后把 filter 引用注入 pin（`attach_filter_reference`）。

### 1.4 COM 继承链 QI（非根因，曾误判）

担心 `#[implement(IBaseFilter)]` 是否让 `QI(IPersist)`/`QI(IMediaFilter)` 成功——验证后**不是问题**：windows 0.61.3 的 `IBaseFilter_Vtbl::matches` 硬编码匹配 `IBaseFilter`+`IPersist`+`IMediaFilter`，implement 宏生成的 QI 遍历每条 chain 调 `matches`。同理 `#[implement(IPin, IMemInputPin)]` 正确。

## 2. 最终架构

```
DirectShow 图：[OBS capture filter] → [SinkFilter (自研)] 
                                        └─ SinkPin (IPin + IMemInputPin)
                                             └─ Receive(): 拷贝到 SinkFrameSlot
```

- 拓扑：capture filter 的 capture pin → sink 的输入 pin（id=`"In"`）。无输出 pin（纯 sink）。
- 连接：`ICaptureGraphBuilder2::RenderStream(NULL, NULL, device, NULL, sink)` 直连，无中间 transform filter。
- 格式协商：`ReceiveConnection` 校验 `majortype==MEDIATYPE_Video` + `formattype==FORMAT_VideoInfo`，解析 `VIDEOINFOHEADER` 的尺寸/subtype，接受 NV12/YUY2/RGB24/ARGB32，拒绝其它。
- 收帧：`IMemInputPin::Receive`（DirectShow 流线程回调）拷贝样本数据到共享槽。
- COM 线程模型：整个枚举 + 建图 + 采集在**单一 STA 线程**（`COINIT_APARTMENTTHREADED`）上，主线程不接触任何 COM 对象（STA 跨线程用 COM 对象需封送，DirectShow moniker 不支持）。

## 3. 事件驱动采集（取代轮询）

### 3.1 问题

早期实现是**轮询**：采集线程每 16ms 调 `slot.latest_frame()`，无帧时也空转。这浪费 CPU 且轮询快于出帧时需靠帧指针去重。

### 3.2 方案：Condvar 阻塞等待

`SinkFrameSlot` 内部用 `Mutex<SlotState> + Condvar`：
- `Receive`（流线程）写入帧数据后 `notify_one`；
- 采集线程在 `next_frame_into` 里 `Condvar::wait` **阻塞等待**，被唤醒后处理一帧；
- 无帧时采集线程彻底睡眠，CPU 趋近于零；
- `stop()`（drop 时调用）`notify_all` 唤醒等待者，立即返回 `Stopped` 退出，延迟亚毫秒级。

### 3.3 缓冲复用（性能关键）

实测每帧 `to_vec()` 分配（1920×1080 NV12 ~3MB，30fps）会让流线程吃满一个核。改为：
- **写入侧**（`store_frame`）：复用 slot 内的 `Vec<u8>`（`clear` + `extend_from_slice`，容量稳定后零分配）；
- **读取侧**（`next_frame_into(buf: &mut Vec<u8>)`）：调用方传入复用缓冲，零分配。

消费语义：`next_frame_into` 取走后置 `has_new=false`，下次必须等 `Receive` 写入新帧。

## 4. 性能基线（OBS 虚拟摄像头 1920×1080@30fps，release 构建）

| 配置 | 进程 CPU（单核占比） | 说明 |
|------|---------------------|------|
| 纯管线（跳过 sink 拷贝） | ~3.5% | DirectShow/OBS capture filter 固有开销 |
| 复用缓冲（当前实现） | ~36%（idle）/ ~37%（stream） | 含 raw 帧 memcpy |
| 转换 + 发布（采集线程） | ~1%（37-36） | NV12→BGRA 转换 + watch 发布 |

- release 下达到 **30fps 满帧**（debug 构建因未优化 memcpy + 转换，降到 ~10fps 且吃满核，属正常）。
- 剩余 ~33% 来自 raw 帧的 memcpy 中转（slot 拷贝）。后续可优化：在 `Receive`（流线程）就地转换并发布，省掉 raw 中转拷贝；但需评估延长 `Receive` 对管线的影响，作为独立优化项。

## 5. 默认相机选择

headless 默认（用户不指定 `--camera`）**优先 OBS 虚拟摄像头**（名字含 `"OBS"`），找不到时退回第一台。多虚拟摄像头并存时（如 ToDesk Camera + OBS Virtual Camera），枚举顺序不保证 OBS 在前，故需显式优先，避免误选。

## 6. 关键文件

- `crates/ipkvm-video/src/dshow_sink.rs`：纯 sink filter + COM 实现 + 事件驱动帧槽 + 像素转换。
- `crates/ipkvm-video/src/camera.rs`：`CameraSource`（建图、采集线程、FrameSource 实现）、设备枚举、COM 初始化守卫。
- `crates/ipkvm-video/examples/camera_probe.rs`：枚举 + 打开 + 采帧验证。
- `crates/ipkvm-video/examples/camera_perf.rs`：帧率与 CPU 性能探测。

## 7. 验证证据

- `camera_probe`：成功从 OBS 虚拟摄像头采集 1920×1080 BGRA8888，持续多帧、checksum 各异、无黑屏、不崩溃（exit 0）。
- `cargo test --workspace --all-features`：全部通过（含 sink filter COM 行为的单元测试）。
- headless `--list-cameras`：枚举到 ToDesk Camera + OBS Virtual Camera。
- headless 默认启动：日志确认 `默认使用相机：OBS Virtual Camera（1:OBS Virtual Camera）`。
