# 参考资料索引

这个目录保存 my_ipkvm 设计和实现过程中会反复阅读的本地资料。

## 已下载资料

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
- noVNC：https://github.com/novnc/noVNC
- noVNC 接口文档：https://github.com/novnc/noVNC/blob/master/docs/API.md
- noVNC 嵌入文档：https://github.com/novnc/noVNC/blob/master/docs/EMBEDDING.md
- Windows Media Foundation 采集：https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation
- Windows H.264 编码器：https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder
- Linux V4L2 接口：https://docs.kernel.org/userspace-api/media/v4l/v4l2.html
- GStreamer 许可证说明：https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html
- FFmpeg 法律说明：https://www.ffmpeg.org/legal.html
- WebRTC 编码格式说明：https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/WebRTC_codecs
