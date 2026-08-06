## 关联 issue

Refs #35（FU-1：MJPEG 透传）

## 改动摘要

D 的 sink 两轮连接已能接收 MJPEG 原始字节，但 RFB 路径（`frame_view`）拒绝非 BGRA 帧 → MJPEG 直通在 RFB 路径上不可用。本 PR 让 MJPEG 帧的原始 JPEG 字节直接作为 Tight JPEG 矩形发网络，跳过"MJPEG 解码→BGRA→再 JPEG 编码"的双重浪费。

### ipkvm-rfb
- **`encode_tight_mjpeg_passthrough`**：原始 JPEG 字节直接做 Tight JPEG 矩形 payload（encoding=7，压缩控制 0x90，Tight 变长长度，JPEG 字节流）。
- **`RfbConnectionCore::queue_mjpeg_passthrough`**：检查客户端支持 Tight(7) + 容量 + 调透传编码。

### ipkvm-session
- **driver `queue_and_write_frame`**：识别 `PixelFormat::Mjpeg` 帧分流到透传路径（不走 `frame_view` 的 BGRA 要求，不调 `queue_framebuffer_update`）。

## 测试

```
cargo fmt --all --check           # 干净
cargo test --workspace --all-features  # 42 套件，705 测试，0 失败
```

新增 `mjpeg_passthrough_preserves_jpeg_bytes_in_tight_rect`：逐字节验证 JPEG payload（SOI marker + 数据 + EOI 完整保留）。

## 后续

本 PR 是 #35 的 FU-1。后续 FU-2（CLI 开关）、FU-3（noVNC Tight 验证）、FU-4（dirty rects 生效）独立推进。
