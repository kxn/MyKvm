# 参考资料索引

这个目录保存 my_ipkvm 设计和实现过程中会反复阅读的本地资料。

## RFB WebSocket 固定实现资料

- noVNC 兼容目标固定为 1.7.0 提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`。经常阅读的上游文件是 [WebSocket 字节队列实现](https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/core/websock.js)、[RFB 初始化与消息处理](https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/core/rfb.js) 和 [编码常量](https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/core/encodings.js)。仓库在 `third_party/novnc/1.7.0` 保存完整 npm 发布包，并以固定元数据和逐文件 SHA-256 清单验证来源与内容。
- 生产 WebSocket 路由使用 [axum 0.8.9 WebSocket 模块](https://docs.rs/axum/0.8.9/axum/extract/ws/) 与 [WebSocketUpgrade](https://docs.rs/axum/0.8.9/axum/extract/ws/struct.WebSocketUpgrade.html)。`axum` 仅启用 `http1`、`tokio`、`ws` 功能开关；锁文件版本为 0.8.9。
- 测试客户端直接使用 [tokio-tungstenite 0.29.0](https://docs.rs/tokio-tungstenite/0.29.0/tokio_tungstenite/) 和 [futures-util 0.3.33](https://docs.rs/futures-util/0.3.33/futures_util/)；两者在 `ipkvm-headless` 中声明为直接开发依赖。前者由 axum 的 `ws` 功能引入正常生产依赖树，后者是 axum 的正常依赖并且还经 tower 进入。
- `scripts/verify-web-assets.py` 离线核验固定资源、许可证和浏览器锁文件；`scripts/update-novnc.ps1` 是需要显式联网的升级入口，升级时必须重新审查固定信任值。

## 已下载资料

> **再分发声明：** 下列部分资料来自第三方标准组织或芯片厂商，本仓库仅作为本地研究用途收录，不主张对其享有版权。其中：
> - `RFC6143-rfb-protocol.txt` 来自 IETF，按 RFC 再分发条款（含 BSD 风格声明）允许随项目分发。
> - `rfbproto-community-spec.rst`、noVNC 系列 `.md`/`.txt` 随其各自许可证（MPL-2.0 等）分发，来源与许可证文件见 `third_party/`。
> - `USB-HID-Usage-Tables-1.7.pdf`、`USB-Video-Class-1.5-document-set.zip`、`uvc-1.5/` 来自 USB-IF，`CH9329-*.pdf` 来自 WCH/供应商，**这些资料无明确再分发许可**。本仓库将其与项目源码一同公开属于历史遗留；在取得书面再分发许可前，请优先通过下方"在线资料"的官方链接获取原始文档，不要从本仓库再转载这些 PDF/ZIP。如 USB-IF 或版权方提出异议，将以移除工作树文件并保留官方链接的方式处理。

- `CH9329-serial-protocol-wch-20190508.pdf`  
  CH9329 串口协议资料。用于确认串口帧格式、键盘命令、绝对鼠标命令、相对鼠标命令和校验和规则。

- `CH9329-datasheet-akizuki-mirror.pdf`  
  CH9329 数据手册镜像。用于确认芯片能力、工作模式和电气背景。

- `USB-HID-Usage-Tables-1.7.pdf`  
  USB-IF HID 用途表 1.7。用于确认键盘、小键盘和 HID 用途页定义。

- `USB-Video-Class-1.5-document-set.zip`  
  USB-IF UVC 1.5 文档集压缩包。

- `uvc-1.5/USB Video Class 1_5/`  
  已解压的 UVC 1.5 文档。最相关的文件包括：
  - `UVC 1.5 Class specification.pdf`
  - `USB_Video_Payload_MJPEG_1.5.pdf`
  - `USB_Video_Payload_H264_1.5.pdf`
  - `USB_Video_Payload_VP8_1.5.pdf`
  - `USB_Video_Payload_Uncompressed_1.5.pdf`

- `RFC6143-rfb-protocol.txt`  
  官方 RFB 3.8 RFC。用于确认 VNC/RFB 握手、帧缓冲更新消息、键盘事件、鼠标事件和基础编码。

- `rfbproto-community-spec.rst`  
  社区维护的 RFB 协议规格。用于补充 RFC 6143 未覆盖的扩展细节。

- `noVNC-README.md`  
  noVNC 概览资料。用于确认浏览器和服务端要求、支持的编码和部署注意事项。

- `noVNC-API.md`  
  noVNC 外部接口文档。用于后续决定是嵌入 noVNC 核心库，还是直接提供完整 noVNC 应用。

- `noVNC-EMBEDDING.md`  
  noVNC 嵌入和部署说明。

- `noVNC-LICENSE.txt`  
  noVNC 许可证资料。核心库是 MPL-2.0，部分应用和静态资源使用其列出的其他许可证。

## 在线资料

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
- Windows H.264 编码器：https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder
- Linux V4L2 接口：https://docs.kernel.org/userspace-api/media/v4l/v4l2.html
- GStreamer 许可证说明：https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html
- FFmpeg 法律说明：https://www.ffmpeg.org/legal.html
- WebRTC 编码格式说明：https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/WebRTC_codecs
