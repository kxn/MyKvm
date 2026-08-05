# k3 架构与脚手架审查意见采纳清单

日期：2026-07-31

审查来源：`docs/ipkvm-arch-review.md`

目标范围：Rust 工作区脚手架、模块依赖、核心接口契约。

## 采纳并已落实

| 建议 | 处理 |
| --- | --- |
| 固定异步运行时 | 采纳。workspace 固定 tokio 作为 I/O 运行时；`ipkvm-core` 保持同步契约，不直接依赖 tokio。 |
| 修正 `InputSink` 契约 | 采纳。改为 `handle_key` / `handle_pointer` / `type_text` / `release_all`，全部返回 `Result`。 |
| 避免方法和事件枚举重复编码 | 采纳。去掉 `key_down(KeyEvent)`、`pointer_move(PointerEvent)` 这类错配签名。 |
| 明确指针坐标语义 | 采纳。`PointerEvent::AbsoluteMove` 使用帧缓冲像素坐标和帧尺寸，CH9329 0..4095 换算留在 sink 实现内。 |
| 明确滚轮方向 | 采纳。正数表示向上，负数表示向下。 |
| `Ch9329Frame::new` 返回错误 | 采纳。超长数据返回 `Ch9329Error::DataTooLong`，不再 panic。 |
| 引入统一错误类型 | 采纳。`ipkvm-core` 引入 `thiserror`，定义 `Ch9329Error`、`InputError`、`SerialError`。 |
| `VideoFrame` 改为共享所有权 | 采纳。帧数据使用 `Arc<[u8]>`，`FrameSource::latest_frame()` 返回 `Option<Arc<VideoFrame>>`。 |
| 添加帧序号 | 采纳。`VideoFrame.seq` 用于慢客户端丢帧和后续脏块检测。 |
| 添加 stride | 采纳。`VideoFrame.stride` 显式记录行宽。 |
| 明确时间戳语义 | 采纳。使用 `MonotonicTimestamp`，不表示墙钟时间。 |
| 去掉大帧相等比较 | 采纳。`VideoFrame` 不再派生 `Eq` / `PartialEq`。 |
| 补订阅接口 | 采纳。`FrameSource` 增加 `subscribe() -> FrameReceiver`。 |
| 修正 `ipkvm-rfb` 依赖方向 | 采纳。`ipkvm-rfb` 只依赖 `ipkvm-core` 和 `ipkvm-video`，不依赖 `ipkvm-session`。 |
| `http_port` 移出 RFB 配置 | 采纳。`RfbServerConfig` 只保留 RFB TCP 端口；HTTP 端口在 `HeadlessConfig` 中。 |
| 补齐 `ConsoleSessionConfig` | 采纳。增加视频格式、波特率、键盘布局、鼠标模式。 |
| mock 视频源 | 采纳。`ipkvm-video` 的 `mock` feature 提供 `MockFrameSource`。 |
| fake serial | 采纳。`ipkvm-core` 的 `mock` feature 提供 `FakeSerialWriter`。 |
| `[workspace.dependencies]` | 采纳。集中管理 `thiserror` 和 `tokio`。 |
| 最小 CI | 原采纳。后续确认没有经过维护的可用 runner，按 issue `#4` 移除无效 workflow，改由 `cargo make quick` 提供本地自动化门禁；恢复远端 CI 前需另行设计 runner 和维护责任。 |

## 部分采纳

| 建议 | 处理 |
| --- | --- |
| trait 全面 async 化 | 部分采纳。I/O 运行时固定为 tokio，但 `ipkvm-core` 的协议和输入契约保持同步；异步边界放在真实串口、HTTP、WebSocket、采集后端和 headless 组装层。 |
| 订阅接口具体使用 tokio watch | 部分采纳。当前 `ipkvm-video` 暴露 `tokio::sync::watch` 接收端，后续如果真实采集后端需要 broadcast 或自定义 fanout，可以在 video 层内部替换实现。 |

## 后续跟进

| 项目 | 阶段 |
| --- | --- |
| 真实 tokio serial 后端 | 阶段 1 |
| RFB 协议样例测试和 `DesktopSize` | 阶段 0/2 |
| 真实 HTTP/axum 接入 | 阶段 2 |
| noVNC 静态资源内嵌依赖审计 | 阶段 2 |
