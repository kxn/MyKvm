# RFB 3.8 纯协议核心实施计划

> **供自动化执行者使用：** 必须使用 `subagent-driven-development` 或 `executing-plans` 按任务执行，并用复选框跟踪步骤。

**目标：** 在 `ipkvm-rfb` 中完成同步、传输无关、可自动化验证的 RFB 3.8 服务器协议核心。

**架构：** 协议核心只接收客户端字节、生成服务器字节和类型化事件，不持有 socket、视频源或输入接收端。客户端消息由有限缓存增量解码，连接状态机负责握手与协商状态，帧缓冲编码器把经过校验的 BGRA8888 视图事务性转换为客户端像素格式。

**技术栈：** Rust 1.89、edition 2024、`thiserror` 2、`proptest` 1、Cargo workspace、RFC 6143、RFB 社区规格。

## 全局约束

- 仓库内自写文档使用中文，代码标识符、协议字段和第三方专有名词保留原文。
- 关联 Gitea issue `#2`。
- 权威设计为 `docs/superpowers/specs/2026-07-31-rfb-38-protocol-core-design.md`。
- 只接受客户端版本 `RFB 003.008\n`，只提供 `SecurityType None`。
- 不实现 TCP、WebSocket、HTTP、noVNC、多客户端、输入映射、认证、压缩编码、帧率限制或真实视频源。
- 正常依赖只使用工作区已有的 `thiserror`；性质测试只使用工作区已有的 `proptest`。
- 所有长度、矩形和缓冲区计算使用检查运算，不依赖整数回绕。
- 所有输出操作保持事务性；失败时不得追加部分字节或提交尺寸变化。
- 所有生产代码先有能按预期失败的测试，再写最小实现。
- 每个任务形成独立提交；不得把用户在主工作区对 `AGENTS.md` 的未提交修改带入提交。
- 每个任务完成后运行 `cargo test -p ipkvm-rfb`。
- 最终运行 `cargo fmt --all --check`、`cargo test --workspace --all-features`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 和 `git diff --check`。

## 文件结构

```text
crates/ipkvm-rfb/
  Cargo.toml
  src/
    lib.rs                    公共导出和现有 TCP 端口配置
    connection.rs             配置校验、握手状态机、协商状态和外部事件
    framebuffer.rs            RFB 尺寸、矩形、BGRA 帧视图和交集
    protocol/
      mod.rs                  协议内部导出
      wire.rs                 大端整数读取和写入辅助
      pixel_format.rs         PIXEL_FORMAT 校验和像素转换
      client.rs               客户端消息增量解码
      server.rs               ServerInit 和 framebuffer 更新编码
  tests/
    protocol_transcript.rs    只使用公共 API 的完整内存转录
```

---

### 任务 1：协议基础值类型和有限帧视图

**文件：**

- 修改：`crates/ipkvm-rfb/Cargo.toml`
- 修改：`crates/ipkvm-rfb/src/lib.rs`
- 新建：`crates/ipkvm-rfb/src/framebuffer.rs`
- 新建：`crates/ipkvm-rfb/src/protocol/mod.rs`
- 新建：`crates/ipkvm-rfb/src/protocol/wire.rs`

**接口：**

- 产出：`RfbSize::new`、`RfbRectangle::intersection`、`BgraFrameView::new`。
- 产出：`RfbFramebufferError`、`RfbProtocolLimits`。
- 产出：后续协议模块使用的大端整数读写辅助。
- 后续任务依赖本任务定义的尺寸不变量、帧跨度和默认资源上限。

- [x] **步骤 1：调整 crate 依赖**

将 `crates/ipkvm-rfb/Cargo.toml` 的依赖改为：

```toml
[dependencies]
thiserror.workspace = true

[dev-dependencies]
proptest.workspace = true
```

移除本次纯协议核心尚未使用的 `ipkvm-core` 和 `ipkvm-video` 正常依赖。运行：

```powershell
cargo check -p ipkvm-rfb
```

预期：通过；该步骤只调整依赖边界，不引入协议行为。

- [x] **步骤 2：写尺寸、矩形和帧视图失败测试**

在新建的 `framebuffer.rs` 测试模块写入：

```rust
#[test]
fn size_rejects_zero_dimensions() {
    assert!(matches!(
        RfbSize::new(0, 1080),
        Err(RfbFramebufferError::ZeroSize {
            width: 0,
            height: 1080
        })
    ));
    assert!(RfbSize::new(1920, 0).is_err());
}

#[test]
fn rectangle_intersection_clips_without_u16_overflow() {
    let frame = RfbSize::new(100, 80).unwrap();
    let requested = RfbRectangle {
        x: 90,
        y: 70,
        width: u16::MAX,
        height: u16::MAX,
    };

    assert_eq!(
        requested.intersection(frame),
        Some(RfbRectangle {
            x: 90,
            y: 70,
            width: 10,
            height: 10,
        })
    );
}

#[test]
fn frame_view_accepts_padding_without_requiring_a_final_padding_tail() {
    let size = RfbSize::new(2, 2).unwrap();
    let pixels = [0_u8; 20];
    let frame = BgraFrameView::new(size, 12, &pixels).unwrap();

    assert_eq!(frame.byte_span(), 20);
    assert_eq!(frame.row(1), &[0; 8]);
}

#[test]
fn frame_view_rejects_short_stride_and_short_pixels() {
    let size = RfbSize::new(2, 2).unwrap();
    assert!(matches!(
        BgraFrameView::new(size, 7, &[0; 16]),
        Err(RfbFramebufferError::StrideTooSmall {
            minimum: 8,
            actual: 7
        })
    ));
    assert!(matches!(
        BgraFrameView::new(size, 8, &[0; 15]),
        Err(RfbFramebufferError::PixelDataTooShort {
            required: 16,
            actual: 15
        })
    ));
}

#[test]
fn frame_view_reports_span_overflow() {
    let size = RfbSize::new(2, 2).unwrap();
    assert!(matches!(
        BgraFrameView::new(size, usize::MAX, &[]),
        Err(RfbFramebufferError::SizeOverflow)
    ));
}
```

在 `lib.rs` 声明 `mod framebuffer;` 并暂时只重导出测试引用的名称。

- [x] **步骤 3：运行基础值测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb framebuffer
```

预期：编译失败，提示 `RfbSize`、`RfbRectangle`、`BgraFrameView` 或 `RfbFramebufferError` 尚未定义。

- [x] **步骤 4：实现尺寸、矩形和帧视图**

实现以下类型和签名：

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RfbSize {
    width: u16,
    height: u16,
}

impl RfbSize {
    pub fn new(width: u16, height: u16) -> Result<Self, RfbFramebufferError>;
    pub fn width(self) -> u16;
    pub fn height(self) -> u16;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfbRectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl RfbRectangle {
    pub fn intersection(self, size: RfbSize) -> Option<Self>;
}

#[derive(Clone, Copy, Debug)]
pub struct BgraFrameView<'a> {
    size: RfbSize,
    stride: usize,
    pixels: &'a [u8],
}

impl<'a> BgraFrameView<'a> {
    pub fn new(
        size: RfbSize,
        stride: usize,
        pixels: &'a [u8],
    ) -> Result<Self, RfbFramebufferError>;
    pub fn size(self) -> RfbSize;
    pub fn stride(self) -> usize;
    pub fn byte_span(self) -> usize;
    pub(crate) fn row(self, y: u16) -> &'a [u8];
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbFramebufferError {
    ZeroSize { width: u16, height: u16 },
    StrideTooSmall { minimum: usize, actual: usize },
    PixelDataTooShort { required: usize, actual: usize },
    SizeOverflow,
}
```

实现规则：

- 一行有效 BGRA 字节数为 `width * 4`。
- 帧跨度为 `(height - 1) * stride + width * 4`。
- `intersection` 用 `u32` 计算右边界和下边界，再转换回已经裁剪到 `u16` 范围的结果。
- 空交集返回 `None`；输入矩形允许零宽或零高。
- `row(y)` 只返回该行的有效像素，不包含 stride 尾部。

- [x] **步骤 5：写资源限制和 wire 辅助失败测试**

在 `lib.rs` 测试模块加入：

```rust
#[test]
fn protocol_limits_have_documented_defaults() {
    let limits = RfbProtocolLimits::default();

    assert_eq!(limits.max_desktop_name_bytes, 1024);
    assert_eq!(limits.max_encodings, 4096);
    assert_eq!(limits.max_cut_text_bytes, 1024 * 1024);
    assert_eq!(limits.max_buffered_input_bytes, 2 * 1024 * 1024);
    assert_eq!(limits.max_queued_output_bytes, 256 * 1024 * 1024);
    assert_eq!(limits.max_framebuffer_bytes, 128 * 1024 * 1024);
}
```

在 `wire.rs` 测试模块加入：

```rust
#[test]
fn reads_and_writes_rfb_big_endian_integers() {
    let bytes = [0x12, 0x34, 0x89, 0xab, 0xcd, 0xef];
    assert_eq!(read_u16(&bytes, 0), Some(0x1234));
    assert_eq!(read_u32(&bytes, 2), Some(0x89abcdef));

    let mut output = Vec::new();
    write_u16(&mut output, 0x1234);
    write_u32(&mut output, 0x89abcdef);
    write_i32(&mut output, -223);
    assert_eq!(
        output,
        [0x12, 0x34, 0x89, 0xab, 0xcd, 0xef, 0xff, 0xff, 0xff, 0x21]
    );
}
```

- [x] **步骤 6：运行限制和 wire 测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb protocol_limits_have_documented_defaults
cargo test -p ipkvm-rfb protocol::wire
```

预期：编译失败，提示 `RfbProtocolLimits` 和 wire 辅助尚未定义。

- [x] **步骤 7：实现资源限制和 wire 辅助**

在 `lib.rs` 定义：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfbProtocolLimits {
    pub max_desktop_name_bytes: usize,
    pub max_encodings: usize,
    pub max_cut_text_bytes: usize,
    pub max_buffered_input_bytes: usize,
    pub max_queued_output_bytes: usize,
    pub max_framebuffer_bytes: usize,
}

impl Default for RfbProtocolLimits {
    fn default() -> Self {
        Self {
            max_desktop_name_bytes: 1024,
            max_encodings: 4096,
            max_cut_text_bytes: 1024 * 1024,
            max_buffered_input_bytes: 2 * 1024 * 1024,
            max_queued_output_bytes: 256 * 1024 * 1024,
            max_framebuffer_bytes: 128 * 1024 * 1024,
        }
    }
}
```

在 `wire.rs` 实现：

```rust
pub(crate) fn read_u16(bytes: &[u8], offset: usize) -> Option<u16>;
pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> Option<u32>;
pub(crate) fn read_i32(bytes: &[u8], offset: usize) -> Option<i32>;
pub(crate) fn write_u16(output: &mut Vec<u8>, value: u16);
pub(crate) fn write_u32(output: &mut Vec<u8>, value: u32);
pub(crate) fn write_i32(output: &mut Vec<u8>, value: i32);
```

读取辅助对短切片返回 `None`，不 panic；写入统一使用大端字节。

- [x] **步骤 8：验证并提交任务 1**

运行：

```powershell
cargo fmt --all
cargo test -p ipkvm-rfb
git diff --check
```

预期：`ipkvm-rfb` 全部测试通过，格式检查无错误。

提交：

```powershell
git add Cargo.lock crates/ipkvm-rfb
git commit -m "feat: add RFB protocol value types (#2)"
```

---

### 任务 2：经过校验的 true-color 像素格式

**文件：**

- 新建：`crates/ipkvm-rfb/src/protocol/pixel_format.rs`
- 修改：`crates/ipkvm-rfb/src/protocol/mod.rs`
- 修改：`crates/ipkvm-rfb/src/lib.rs`

**接口：**

- 消费：任务 1 的 wire 写入辅助。
- 产出：`RfbPixelFormat::new`、`RfbPixelFormat::default_bgrx8888` 和只读 getter。
- 产出：wire 像素格式解析、序列化及单像素 BGR 转换。
- 后续客户端解码器使用 `from_wire`，服务器编码器使用 `write_bgr`。

- [x] **步骤 1：写默认格式和 wire 金样失败测试**

在 `pixel_format.rs` 测试模块写入：

```rust
#[test]
fn default_format_matches_bgrx8888_wire_layout() {
    let format = RfbPixelFormat::default_bgrx8888();
    let mut wire = Vec::new();
    format.write_wire(&mut wire);

    assert_eq!(
        wire,
        [
            32, 24, 0, 1,
            0, 255, 0, 255, 0, 255,
            16, 8, 0,
            0, 0, 0,
        ]
    );
}

#[test]
fn default_format_writes_bgr_zero_bytes() {
    let mut output = Vec::new();
    RfbPixelFormat::default_bgrx8888().write_bgr(&mut output, 0x12, 0x34, 0x56);
    assert_eq!(output, [0x12, 0x34, 0x56, 0]);
}
```

- [x] **步骤 2：运行默认格式测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb protocol::pixel_format::tests::default_format
```

预期：编译失败，提示 `RfbPixelFormat` 尚未定义。

- [x] **步骤 3：实现默认格式和像素写出骨架**

定义：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfbPixelFormat {
    bits_per_pixel: u8,
    depth: u8,
    big_endian: bool,
    red_max: u16,
    green_max: u16,
    blue_max: u16,
    red_shift: u8,
    green_shift: u8,
    blue_shift: u8,
}

impl RfbPixelFormat {
    pub fn default_bgrx8888() -> Self;
    pub fn bits_per_pixel(self) -> u8;
    pub fn depth(self) -> u8;
    pub fn big_endian(self) -> bool;
    pub fn red_max(self) -> u16;
    pub fn green_max(self) -> u16;
    pub fn blue_max(self) -> u16;
    pub fn red_shift(self) -> u8;
    pub fn green_shift(self) -> u8;
    pub fn blue_shift(self) -> u8;
    pub(crate) fn bytes_per_pixel(self) -> usize;
    pub(crate) fn write_wire(self, output: &mut Vec<u8>);
    pub(crate) fn write_bgr(self, output: &mut Vec<u8>, blue: u8, green: u8, red: u8);
}
```

`write_bgr` 先按 `(value * channel_max + 127) / 255` 缩放，再组合位域，最后按 1、2 或 4 字节和目标大小端输出。

- [x] **步骤 4：写合法格式和非法格式失败测试**

加入：

```rust
#[test]
fn writes_rgb565_little_endian() {
    let format = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
    let mut output = Vec::new();
    format.write_bgr(&mut output, 0, 0, 255);
    assert_eq!(output, [0x00, 0xf8]);
}

#[test]
fn writes_rgb332_and_scales_channels() {
    let format = RfbPixelFormat::new(8, 8, false, 7, 7, 3, 5, 2, 0).unwrap();
    let mut output = Vec::new();
    format.write_bgr(&mut output, 255, 128, 0);
    assert_eq!(output, [0x13]);
}

#[test]
fn writes_32_bit_big_endian() {
    let format = RfbPixelFormat::new(32, 24, true, 255, 255, 255, 16, 8, 0).unwrap();
    let mut output = Vec::new();
    format.write_bgr(&mut output, 0x12, 0x34, 0x56);
    assert_eq!(output, [0, 0x56, 0x34, 0x12]);
}

#[test]
fn rejects_color_map_and_invalid_masks() {
    let mut color_map_wire = RfbPixelFormat::default_bgrx8888().to_wire();
    color_map_wire[3] = 0;
    assert_eq!(
        RfbPixelFormat::from_wire(&color_map_wire),
        Err(RfbPixelFormatError::ColorMapUnsupported)
    );

    assert!(matches!(
        RfbPixelFormat::new(16, 16, false, 30, 63, 31, 11, 5, 0),
        Err(RfbPixelFormatError::InvalidChannelMax {
            channel: RfbColorChannel::Red,
            value: 30
        })
    ));
    assert!(matches!(
        RfbPixelFormat::new(16, 16, false, 31, 63, 31, 10, 5, 0),
        Err(RfbPixelFormatError::OverlappingChannels)
    ));
}
```

- [x] **步骤 5：运行格式校验测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb protocol::pixel_format
```

预期：编译失败，提示 `new`、`from_wire`、`to_wire` 或错误类型尚未定义。

- [x] **步骤 6：实现格式校验和 wire 解析**

补齐：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbColorChannel {
    Red,
    Green,
    Blue,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbPixelFormatError {
    UnsupportedBitsPerPixel(u8),
    InvalidDepth { depth: u8, bits_per_pixel: u8 },
    ColorMapUnsupported,
    InvalidChannelMax { channel: RfbColorChannel, value: u16 },
    ChannelOutOfRange { channel: RfbColorChannel },
    OverlappingChannels,
    ChannelsExceedDepth { channel_bits: u8, depth: u8 },
}

impl RfbPixelFormat {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bits_per_pixel: u8,
        depth: u8,
        big_endian: bool,
        red_max: u16,
        green_max: u16,
        blue_max: u16,
        red_shift: u8,
        green_shift: u8,
        blue_shift: u8,
    ) -> Result<Self, RfbPixelFormatError>;

    pub(crate) fn from_wire(bytes: &[u8; 16]) -> Result<Self, RfbPixelFormatError>;
    pub(crate) fn to_wire(self) -> [u8; 16];
}
```

校验顺序固定为：

1. bits-per-pixel 是 8、16、32。
2. depth 在 `1..=bits_per_pixel`。
3. true-color flag 非零。
4. 每个 max 大于零且 `max + 1` 是 2 的幂。
5. 用 `u32` 构造每个 mask，检查 `shift + channel_bits <= bits_per_pixel`。
6. 三个 mask 不重叠。
7. 三个通道位数之和不超过 depth。

wire 中布尔字段按非零为 true，padding 只读取不校验。

- [x] **步骤 7：补充全部格式边界测试并验证**

补充：

```rust
#[test]
fn rejects_unsupported_bits_depth_and_channel_ranges() {
    assert_eq!(
        RfbPixelFormat::new(24, 24, false, 255, 255, 255, 16, 8, 0),
        Err(RfbPixelFormatError::UnsupportedBitsPerPixel(24))
    );
    assert!(matches!(
        RfbPixelFormat::new(16, 0, false, 31, 63, 31, 11, 5, 0),
        Err(RfbPixelFormatError::InvalidDepth {
            depth: 0,
            bits_per_pixel: 16
        })
    ));
    assert!(matches!(
        RfbPixelFormat::new(16, 16, false, 31, 63, 31, 12, 5, 0),
        Err(RfbPixelFormatError::ChannelOutOfRange {
            channel: RfbColorChannel::Red
        })
    ));
    assert!(matches!(
        RfbPixelFormat::new(16, 8, false, 31, 63, 31, 11, 5, 0),
        Err(RfbPixelFormatError::ChannelsExceedDepth {
            channel_bits: 16,
            depth: 8
        })
    ));
}

#[test]
fn honors_endianness_and_rounds_channel_scaling() {
    let little = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
    let big = RfbPixelFormat::new(16, 16, true, 31, 63, 31, 11, 5, 0).unwrap();
    let mut little_bytes = Vec::new();
    let mut big_bytes = Vec::new();
    little.write_bgr(&mut little_bytes, 0, 0, 128);
    big.write_bgr(&mut big_bytes, 0, 0, 128);

    assert_eq!(little_bytes, [0x00, 0x80]);
    assert_eq!(big_bytes, [0x80, 0x00]);
}
```

运行：

```powershell
cargo fmt --all
cargo test -p ipkvm-rfb protocol::pixel_format
cargo test -p ipkvm-rfb
git diff --check
```

预期：全部通过。

- [x] **步骤 8：提交任务 2**

```powershell
git add crates/ipkvm-rfb
git commit -m "feat: add RFB pixel format support (#2)"
```

---

### 任务 3：客户端消息增量解码

**文件：**

- 新建：`crates/ipkvm-rfb/src/protocol/client.rs`
- 修改：`crates/ipkvm-rfb/src/protocol/mod.rs`
- 修改：`crates/ipkvm-rfb/src/lib.rs`

**接口：**

- 消费：`RfbProtocolLimits`、`RfbPixelFormat`、`RfbRectangle`、wire 读取辅助。
- 产出：crate 内部 `ClientMessage` 和 `ClientMessageDecoder`。
- 产出：公共 `FramebufferUpdateRequest` 和 `RfbProtocolError`。
- 后续连接状态机消费有序解码结果，并把内部消息转换为公共状态或事件。

- [x] **步骤 1：写固定长度消息解码失败测试**

在 `client.rs` 测试模块加入：

```rust
#[test]
fn decodes_fixed_messages_and_nonzero_booleans() {
    let bytes = [
        3, 2, 0, 1, 0, 2, 0, 3, 0, 4,
        4, 1, 0, 0, 0, 0, 0xff, 0x0d,
        5, 0b0000_0101, 0, 10, 0, 20,
        150, 7, 0, 1, 0, 2, 0, 3, 0, 4,
    ];
    let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
    let messages = decoder.push(&bytes);

    assert_eq!(
        messages,
        vec![
            Ok(ClientMessage::FramebufferUpdateRequest(
                FramebufferUpdateRequest {
                    incremental: true,
                    rectangle: RfbRectangle {
                        x: 1,
                        y: 2,
                        width: 3,
                        height: 4,
                    },
                }
            )),
            Ok(ClientMessage::Key {
                down: true,
                keysym: 0xff0d,
            }),
            Ok(ClientMessage::Pointer {
                button_mask: 5,
                x: 10,
                y: 20,
            }),
            Ok(ClientMessage::EnableContinuousUpdates {
                enable: true,
                rectangle: RfbRectangle {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
            }),
        ]
    );
}
```

- [x] **步骤 2：运行固定长度消息测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb protocol::client::tests::decodes_fixed_messages
```

预期：编译失败，提示客户端消息类型和解码器尚未定义。

- [x] **步骤 3：定义消息、错误和单消息解码骨架**

公共类型：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferUpdateRequest {
    pub incremental: bool,
    pub rectangle: RfbRectangle,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbProtocolError {
    UnsupportedVersion([u8; 12]),
    UnsupportedSecurityType(u8),
    UnsupportedClientMessageType(u8),
    TooManyEncodings { declared: usize, maximum: usize },
    CutTextTooLong { declared: usize, maximum: usize },
    InputBufferLimitExceeded { attempted: usize, maximum: usize },
    InvalidPixelFormat(RfbPixelFormatError),
    LengthOverflow,
    ConnectionFailed,
}
```

内部类型：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClientMessage {
    SetPixelFormat(RfbPixelFormat),
    SetEncodings(Vec<i32>),
    FramebufferUpdateRequest(FramebufferUpdateRequest),
    Key { down: bool, keysym: u32 },
    Pointer { button_mask: u8, x: u16, y: u16 },
    CutText(Vec<u8>),
    EnableContinuousUpdates {
        enable: bool,
        rectangle: RfbRectangle,
    },
}

pub(crate) struct ClientMessageDecoder {
    limits: RfbProtocolLimits,
    buffer: Vec<u8>,
    failed: bool,
}

impl ClientMessageDecoder {
    pub(crate) fn new(limits: RfbProtocolLimits) -> Self;
    pub(crate) fn buffered_len(&self) -> usize;
    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
    ) -> Vec<Result<ClientMessage, RfbProtocolError>>;
}
```

先实现类型 3、4、5、150 的精确固定长度解码。padding 不校验，布尔字段非零即 true。

- [x] **步骤 4：写可变长度和像素格式消息失败测试**

加入：

```rust
#[test]
fn decodes_set_pixel_format_set_encodings_and_cut_text() {
    let mut bytes = vec![0, 9, 8, 7];
    bytes.extend_from_slice(&RfbPixelFormat::default_bgrx8888().to_wire());
    bytes.extend_from_slice(&[
        2, 0, 0, 3,
        0, 0, 0, 0,
        0xff, 0xff, 0xff, 0x21,
        0xff, 0xff, 0xfe, 0xc7,
        6, 1, 2, 3, 0, 0, 0, 3, 0x41, 0x80, 0xff,
    ]);

    let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
    assert_eq!(
        decoder.push(&bytes),
        vec![
            Ok(ClientMessage::SetPixelFormat(
                RfbPixelFormat::default_bgrx8888()
            )),
            Ok(ClientMessage::SetEncodings(vec![0, -223, -313])),
            Ok(ClientMessage::CutText(vec![0x41, 0x80, 0xff])),
        ]
    );
}

#[test]
fn waits_for_complete_variable_body() {
    let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
    assert!(decoder.push(&[2, 0, 0, 1, 0, 0]).is_empty());
    assert_eq!(decoder.buffered_len(), 6);
    assert_eq!(
        decoder.push(&[0, 0]),
        vec![Ok(ClientMessage::SetEncodings(vec![0]))]
    );
}

#[test]
fn returns_complete_messages_and_keeps_an_incomplete_tail() {
    let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
    let mut bytes = vec![4, 1, 0, 0, 0, 0, 0xff, 0x0d];
    bytes.extend_from_slice(&[5, 3, 0]);

    assert_eq!(
        decoder.push(&bytes),
        vec![Ok(ClientMessage::Key {
            down: true,
            keysym: 0xff0d,
        })]
    );
    assert_eq!(decoder.buffered_len(), 3);
    assert_eq!(
        decoder.push(&[10, 0, 20]),
        vec![Ok(ClientMessage::Pointer {
            button_mask: 3,
            x: 10,
            y: 20,
        })]
    );
}
```

- [x] **步骤 5：运行可变消息测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb protocol::client::tests::decodes_set_pixel
cargo test -p ipkvm-rfb protocol::client::tests::waits_for_complete
```

预期：测试失败，因为类型 0、2、6 尚未解码。

- [x] **步骤 6：实现可变消息和增量缓存**

消息长度固定为：

| 类型 | 总长度 |
|---:|---:|
| 0 | 20 |
| 2 | `4 + count * 4` |
| 3 | 10 |
| 4 | 8 |
| 5 | 6 |
| 6 | `8 + length` |
| 150 | 10 |

实现要求：

- `push` 在扩容前检查 `buffer.len() + bytes.len()`，超限时不追加新字节。
- 类型 2 读到 4 字节头后立即检查数量上限和 `count * 4 + 4` 溢出，再决定是否等待 body。
- 类型 6 读到 8 字节头后立即检查长度上限和 `length + 8` 溢出。
- 用游标累计完整消息的已消费长度并继续解析后续消息；循环结束或遇到不完整尾部时只执行一次前缀移除，避免每条小消息都搬移剩余缓冲区。
- 未知类型立即返回 `UnsupportedClientMessageType`，设置 `failed=true`，清空缓冲区，不尝试重新同步。
- 失败后的下一次 `push` 返回单个 `ConnectionFailed`。
- `ClientCutText` 原样保留字节，不执行 UTF-8 转换。

- [x] **步骤 7：写限制和错误终态失败测试**

加入：

```rust
#[test]
fn rejects_oversized_lengths_before_waiting_for_bodies() {
    let limits = RfbProtocolLimits {
        max_encodings: 1,
        max_cut_text_bytes: 2,
        ..RfbProtocolLimits::default()
    };
    let mut encodings = ClientMessageDecoder::new(limits);
    assert_eq!(
        encodings.push(&[2, 0, 0, 2]),
        vec![Err(RfbProtocolError::TooManyEncodings {
            declared: 2,
            maximum: 1,
        })]
    );

    let mut cut_text = ClientMessageDecoder::new(limits);
    assert_eq!(
        cut_text.push(&[6, 0, 0, 0, 0, 0, 0, 3]),
        vec![Err(RfbProtocolError::CutTextTooLong {
            declared: 3,
            maximum: 2,
        })]
    );
}

#[test]
fn input_limit_is_checked_before_append() {
    let limits = RfbProtocolLimits {
        max_buffered_input_bytes: 5,
        ..RfbProtocolLimits::default()
    };
    let mut decoder = ClientMessageDecoder::new(limits);
    assert_eq!(
        decoder.push(&[5, 0, 0, 0, 0, 0]),
        vec![Err(RfbProtocolError::InputBufferLimitExceeded {
            attempted: 6,
            maximum: 5,
        })]
    );
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn unknown_type_is_fatal_and_does_not_resynchronize() {
    let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
    assert_eq!(
        decoder.push(&[99, 4, 1, 0, 0, 0, 0, 0xff, 0x0d]),
        vec![Err(RfbProtocolError::UnsupportedClientMessageType(99))]
    );
    assert_eq!(
        decoder.push(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d]),
        vec![Err(RfbProtocolError::ConnectionFailed)]
    );
}

#[test]
fn invalid_pixel_format_is_wrapped_as_a_fatal_protocol_error() {
    let mut wire = RfbPixelFormat::default_bgrx8888().to_wire();
    wire[3] = 0;
    let mut message = vec![0, 0, 0, 0];
    message.extend_from_slice(&wire);
    let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());

    assert_eq!(
        decoder.push(&message),
        vec![Err(RfbProtocolError::InvalidPixelFormat(
            RfbPixelFormatError::ColorMapUnsupported
        ))]
    );
    assert_eq!(
        decoder.push(&[]),
        vec![Err(RfbProtocolError::ConnectionFailed)]
    );
}
```

- [x] **步骤 8：实现错误终态并验证任务 3**

完成限制检查和失败终态后运行：

```powershell
cargo fmt --all
cargo test -p ipkvm-rfb protocol::client
cargo test -p ipkvm-rfb
git diff --check
```

预期：全部通过。

- [x] **步骤 9：提交任务 3**

```powershell
git add crates/ipkvm-rfb
git commit -m "feat: decode RFB client messages (#2)"
```

---

### 任务 4：RFB 3.8 握手和连接状态机

**文件：**

- 新建：`crates/ipkvm-rfb/src/protocol/server.rs`
- 新建：`crates/ipkvm-rfb/src/connection.rs`
- 修改：`crates/ipkvm-rfb/src/protocol/mod.rs`
- 修改：`crates/ipkvm-rfb/src/lib.rs`

**接口：**

- 消费：客户端增量解码器、像素格式、资源限制和 wire 写入辅助。
- 产出：`RfbConnectionCore`、`RfbConnectionConfig`、`RfbConnectionState`、`RfbEvent`。
- 产出：`RfbConfigError` 和握手服务器消息编码。
- 后续 framebuffer 更新任务在同一连接对象中读取协商状态并排队输出。

- [x] **步骤 1：写服务器握手字节金样失败测试**

在 `server.rs` 测试模块加入：

```rust
#[test]
fn server_init_matches_rfb_wire_layout() {
    let bytes = encode_server_init(
        RfbSize::new(640, 480).unwrap(),
        RfbPixelFormat::default_bgrx8888(),
        "机房 KVM",
    )
    .unwrap();

    let mut expected = vec![0x02, 0x80, 0x01, 0xe0];
    expected.extend_from_slice(&RfbPixelFormat::default_bgrx8888().to_wire());
    expected.extend_from_slice(&[0, 0, 0, 10]);
    expected.extend_from_slice("机房 KVM".as_bytes());
    assert_eq!(bytes, expected);
}
```

同时断言常量：

```rust
assert_eq!(PROTOCOL_VERSION, b"RFB 003.008\n");
assert_eq!(NONE_SECURITY_TYPES, [1, 1]);
assert_eq!(SECURITY_RESULT_OK, [0, 0, 0, 0]);
```

- [x] **步骤 2：运行服务器消息测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb protocol::server
```

预期：编译失败，提示服务器消息编码尚未定义。

- [x] **步骤 3：实现握手服务器消息编码**

在 `server.rs` 定义：

```rust
pub(crate) const PROTOCOL_VERSION: &[u8; 12] = b"RFB 003.008\n";
pub(crate) const NONE_SECURITY_TYPES: [u8; 2] = [1, 1];
pub(crate) const SECURITY_RESULT_OK: [u8; 4] = [0, 0, 0, 0];

pub(crate) fn encode_server_init(
    size: RfbSize,
    pixel_format: RfbPixelFormat,
    desktop_name: &str,
) -> Result<Vec<u8>, RfbConfigError>;
```

`ServerInit` 精确写入宽、高、16 字节像素格式、UTF-8 名称长度和名称字节。名称长度转换为 `u32` 使用检查转换。

- [x] **步骤 4：写配置校验失败测试**

在 `connection.rs` 测试模块加入：

```rust
fn config() -> RfbConnectionConfig {
    RfbConnectionConfig {
        desktop_name: "my_ipkvm".to_owned(),
        initial_size: RfbSize::new(640, 480).unwrap(),
        limits: RfbProtocolLimits::default(),
    }
}

#[test]
fn rejects_internally_inconsistent_limits() {
    let mut input = config();
    input.limits.max_buffered_input_bytes = 20;
    input.limits.max_cut_text_bytes = 20;
    assert!(matches!(
        RfbConnectionCore::new(input),
        Err(RfbConfigError::InputCapacityTooSmall { .. })
    ));

    let mut output = config();
    output.initial_size = RfbSize::new(1, 1).unwrap();
    output.limits.max_framebuffer_bytes = 1024;
    output.limits.max_queued_output_bytes = 1024;
    assert!(matches!(
        RfbConnectionCore::new(output),
        Err(RfbConfigError::OutputCapacityTooSmall { .. })
    ));

    let mut initial_frame = config();
    initial_frame.limits.max_framebuffer_bytes = 1024;
    assert!(matches!(
        RfbConnectionCore::new(initial_frame),
        Err(RfbConfigError::InitialFramebufferTooLarge {
            required: 640 * 480 * 4,
            maximum: 1024,
        })
    ));
}

#[test]
fn rejects_zero_limits_and_oversized_desktop_name() {
    let mut zero = config();
    zero.limits.max_encodings = 0;
    assert!(matches!(
        RfbConnectionCore::new(zero),
        Err(RfbConfigError::ZeroLimit("max_encodings"))
    ));

    let mut name = config();
    name.limits.max_desktop_name_bytes = 3;
    assert!(matches!(
        RfbConnectionCore::new(name),
        Err(RfbConfigError::DesktopNameTooLong { .. })
    ));
}
```

- [x] **步骤 5：运行配置测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb connection::tests::rejects_
```

预期：编译失败，提示连接配置、连接核心或配置错误尚未定义。

- [x] **步骤 6：实现配置类型和完整一致性校验**

定义：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbConnectionConfig {
    pub desktop_name: String,
    pub initial_size: RfbSize,
    pub limits: RfbProtocolLimits,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbConfigError {
    ZeroLimit(&'static str),
    FramebufferLimitTooSmall { actual: usize },
    InitialFramebufferTooLarge { required: usize, maximum: usize },
    DesktopNameTooLong { actual: usize, maximum: usize },
    InputCapacityTooSmall { actual: usize, required: usize },
    OutputCapacityTooSmall { actual: usize, required: usize },
    LimitOverflow,
}
```

连接构造时检查：

- 六个限制都大于零，framebuffer 上限至少为 4。
- 初始尺寸的 `width * height * 4` 使用检查乘法，且不超过 framebuffer 上限。
- desktop name UTF-8 字节数不超过配置和 `u32::MAX`。
- 输入缓存至少为 `max(max_cut_text + 8, max_encodings * 4 + 4, 20)`。
- 输出队列至少为 `max(max_framebuffer + 16, 12 + 2 + 4 + 24 + desktop_name.len())`。
- 所有公式使用 `checked_add`、`checked_mul`。

- [x] **步骤 7：写完整握手状态机失败测试**

先在测试模块加入只使用公共握手 API 的辅助：

```rust
fn complete(config: RfbConnectionConfig) -> RfbConnectionCore {
    let mut connection = RfbConnectionCore::new(config).unwrap();
    assert_eq!(connection.take_output(), b"RFB 003.008\n");
    assert!(connection.push_input(b"RFB 003.008\n").is_empty());
    assert_eq!(connection.take_output(), [1, 1]);
    assert!(connection.push_input(&[1]).is_empty());
    assert_eq!(connection.take_output(), [0, 0, 0, 0]);
    assert!(matches!(
        connection.push_input(&[1]).as_slice(),
        [Ok(RfbEvent::HandshakeCompleted { shared: true })]
    ));
    assert!(!connection.take_output().is_empty());
    connection
}

fn completed_connection() -> RfbConnectionCore {
    complete(config())
}
```

加入：

```rust
#[test]
fn completes_rfb_38_none_handshake_across_arbitrary_chunks() {
    let mut connection = RfbConnectionCore::new(config()).unwrap();
    assert_eq!(connection.state(), RfbConnectionState::AwaitingVersion);
    assert_eq!(connection.take_output(), b"RFB 003.008\n");

    assert!(connection.push_input(b"RFB 003.").is_empty());
    assert!(connection.push_input(b"008\n").is_empty());
    assert_eq!(connection.state(), RfbConnectionState::AwaitingSecuritySelection);
    assert_eq!(connection.take_output(), [1, 1]);

    assert!(connection.push_input(&[1]).is_empty());
    assert_eq!(connection.take_output(), [0, 0, 0, 0]);
    assert_eq!(connection.state(), RfbConnectionState::AwaitingClientInit);

    assert_eq!(
        connection.push_input(&[1]),
        vec![Ok(RfbEvent::HandshakeCompleted { shared: true })]
    );
    assert_eq!(connection.state(), RfbConnectionState::Normal);
    assert_eq!(&connection.take_output()[..4], [0x02, 0x80, 0x01, 0xe0]);
}

#[test]
fn rejects_other_versions_and_security_types() {
    let mut version = RfbConnectionCore::new(config()).unwrap();
    version.take_output();
    assert!(matches!(
        version.push_input(b"RFB 003.007\n").as_slice(),
        [Err(RfbProtocolError::UnsupportedVersion(_))]
    ));
    assert_eq!(version.state(), RfbConnectionState::Failed);

    let mut security = RfbConnectionCore::new(config()).unwrap();
    security.take_output();
    security.push_input(b"RFB 003.008\n");
    security.take_output();
    assert_eq!(
        security.push_input(&[2]),
        vec![Err(RfbProtocolError::UnsupportedSecurityType(2))]
    );
    assert_eq!(security.state(), RfbConnectionState::Failed);
}

#[test]
fn handshake_input_limit_is_checked_before_append() {
    let mut limited = config();
    limited.limits.max_encodings = 1;
    limited.limits.max_cut_text_bytes = 1;
    limited.limits.max_buffered_input_bytes = 20;
    let mut connection = RfbConnectionCore::new(limited).unwrap();
    connection.take_output();

    assert_eq!(
        connection.push_input(&[0_u8; 21]),
        vec![Err(RfbProtocolError::InputBufferLimitExceeded {
            attempted: 21,
            maximum: 20,
        })]
    );
    assert_eq!(connection.state(), RfbConnectionState::Failed);
    assert!(connection.take_output().is_empty());
}

#[test]
fn pipelined_bytes_continue_into_the_normal_decoder() {
    let mut connection = RfbConnectionCore::new(config()).unwrap();
    connection.take_output();
    let mut bytes = b"RFB 003.008\n".to_vec();
    bytes.extend_from_slice(&[1, 1]);
    bytes.extend_from_slice(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d]);

    assert_eq!(
        connection.push_input(&bytes),
        vec![
            Ok(RfbEvent::HandshakeCompleted { shared: true }),
            Ok(RfbEvent::Key {
                down: true,
                keysym: 0xff0d,
            }),
        ]
    );
    assert_eq!(connection.state(), RfbConnectionState::Normal);
}
```

- [x] **步骤 8：实现连接状态和握手**

定义：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbConnectionState {
    AwaitingVersion,
    AwaitingSecuritySelection,
    AwaitingClientInit,
    Normal,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbEvent {
    HandshakeCompleted { shared: bool },
    FramebufferUpdateRequested(FramebufferUpdateRequest),
    Key { down: bool, keysym: u32 },
    Pointer { button_mask: u8, x: u16, y: u16 },
    CutText(Vec<u8>),
    EnableContinuousUpdates {
        enable: bool,
        rectangle: RfbRectangle,
    },
}

pub struct RfbConnectionCore {
    // 字段保持私有。
}

impl RfbConnectionCore {
    pub fn new(config: RfbConnectionConfig) -> Result<Self, RfbConfigError>;
    pub fn push_input(
        &mut self,
        bytes: &[u8],
    ) -> Vec<Result<RfbEvent, RfbProtocolError>>;
    pub fn take_output(&mut self) -> Vec<u8>;
    pub fn state(&self) -> RfbConnectionState;
    pub fn pixel_format(&self) -> RfbPixelFormat;
    pub fn encoding_preferences(&self) -> &[i32];
    pub fn supports_desktop_size(&self) -> bool;
}
```

行为：

- `new` 校验配置并把版本 banner 放入输出队列。
- 握手缓冲支持每个字节边界分片，也支持客户端把后续阶段字节提前放在同一块中。
- 握手输入在扩容前检查 `当前缓冲长度 + bytes.len()`，超限时不追加输入并进入失败状态。
- 每个状态只消费自己需要的固定长度，剩余字节继续进入下一状态。
- 完成 `ClientInit` 后追加 `ServerInit`，进入 `Normal` 并产生一次 `HandshakeCompleted`。
- 致命错误是结果列表最后一项，连接立即进入 `Failed`。
- `take_output` 使用 `mem::take` 原子取走当前输出。

- [x] **步骤 9：写正常阶段状态应用和事件失败测试**

加入：

```rust
#[test]
fn applies_negotiation_messages_and_emits_input_events() {
    let mut connection = completed_connection();
    let mut messages = vec![2, 0, 0, 2];
    messages.extend_from_slice(&0_i32.to_be_bytes());
    messages.extend_from_slice(&(-223_i32).to_be_bytes());
    messages.extend_from_slice(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d]);

    assert_eq!(
        connection.push_input(&messages),
        vec![Ok(RfbEvent::Key {
            down: true,
            keysym: 0xff0d,
        })]
    );
    assert_eq!(connection.encoding_preferences(), &[0, -223]);
    assert!(connection.supports_desktop_size());

    assert!(connection.push_input(&[2, 0, 0, 0]).is_empty());
    assert!(connection.encoding_preferences().is_empty());
    assert!(!connection.supports_desktop_size());

    let mut unknown_encodings = vec![2, 0, 0, 2];
    unknown_encodings.extend_from_slice(&12_345_i32.to_be_bytes());
    unknown_encodings.extend_from_slice(&(-313_i32).to_be_bytes());
    assert!(connection.push_input(&unknown_encodings).is_empty());
    assert_eq!(connection.encoding_preferences(), &[12_345, -313]);
    assert!(!connection.supports_desktop_size());
    assert!(connection.take_output().is_empty());
}
```

再加入：

```rust
#[test]
fn applies_pixel_format_and_emits_remaining_messages_in_order() {
    let mut connection = completed_connection();
    let format = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
    let mut messages = vec![0, 9, 8, 7];
    messages.extend_from_slice(&format.to_wire());
    messages.extend_from_slice(&[3, 0, 0, 1, 0, 2, 0, 3, 0, 4]);
    messages.extend_from_slice(&[5, 3, 0, 10, 0, 20]);
    messages.extend_from_slice(&[6, 0, 0, 0, 0, 0, 0, 2, 0x41, 0xff]);
    messages.extend_from_slice(&[150, 1, 0, 5, 0, 6, 0, 7, 0, 8]);

    assert_eq!(
        connection.push_input(&messages),
        vec![
            Ok(RfbEvent::FramebufferUpdateRequested(
                FramebufferUpdateRequest {
                    incremental: false,
                    rectangle: RfbRectangle {
                        x: 1,
                        y: 2,
                        width: 3,
                        height: 4,
                    },
                }
            )),
            Ok(RfbEvent::Pointer {
                button_mask: 3,
                x: 10,
                y: 20,
            }),
            Ok(RfbEvent::CutText(vec![0x41, 0xff])),
            Ok(RfbEvent::EnableContinuousUpdates {
                enable: true,
                rectangle: RfbRectangle {
                    x: 5,
                    y: 6,
                    width: 7,
                    height: 8,
                },
            }),
        ]
    );
    assert_eq!(connection.pixel_format(), format);
}

#[test]
fn preserves_events_before_a_fatal_message_and_then_stays_failed() {
    let mut connection = completed_connection();
    assert_eq!(
        connection.push_input(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d, 99]),
        vec![
            Ok(RfbEvent::Key {
                down: true,
                keysym: 0xff0d,
            }),
            Err(RfbProtocolError::UnsupportedClientMessageType(99)),
        ]
    );
    assert_eq!(connection.state(), RfbConnectionState::Failed);
    assert_eq!(
        connection.push_input(&[4, 0, 0, 0, 0, 0, 0xff, 0x0d]),
        vec![Err(RfbProtocolError::ConnectionFailed)]
    );
}
```

- [x] **步骤 10：实现正常阶段消息应用并验证任务 4**

连接进入正常状态时，把握手缓冲中的剩余字节一次性交给 `ClientMessageDecoder`。后续输入直接交给该解码器。`SetPixelFormat` 和 `SetEncodings` 在连接内部应用，其余消息转为 `RfbEvent`。

运行：

```powershell
cargo fmt --all
cargo test -p ipkvm-rfb connection
cargo test -p ipkvm-rfb
git diff --check
```

预期：全部通过。

- [x] **步骤 11：提交任务 4**

```powershell
git add crates/ipkvm-rfb
git commit -m "feat: add RFB 3.8 handshake core (#2)"
```

---

### 任务 5：Raw 和 DesktopSize 帧缓冲更新

**文件：**

- 修改：`crates/ipkvm-rfb/src/protocol/server.rs`
- 修改：`crates/ipkvm-rfb/src/connection.rs`
- 修改：`crates/ipkvm-rfb/src/lib.rs`

**接口：**

- 消费：`BgraFrameView`、当前像素格式、encoding 偏好和已声明尺寸。
- 产出：`RfbConnectionCore::queue_framebuffer_update`。
- 产出：`FramebufferUpdateOutcome` 和 `RfbEncodeError`。
- TCP 和 WebSocket 后续只需把 `take_output` 返回的字节写到传输。

- [x] **步骤 1：加入 framebuffer 更新测试辅助**

在 `connection.rs` 的测试模块加入：

```rust
fn config_with_size(width: u16, height: u16) -> RfbConnectionConfig {
    RfbConnectionConfig {
        desktop_name: "my_ipkvm".to_owned(),
        initial_size: RfbSize::new(width, height).unwrap(),
        limits: RfbProtocolLimits::default(),
    }
}

fn completed_connection_with_size(width: u16, height: u16) -> RfbConnectionCore {
    complete(config_with_size(width, height))
}

fn set_rgb565(connection: &mut RfbConnectionCore) {
    let format = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
    let mut message = vec![0, 0, 0, 0];
    message.extend_from_slice(&format.to_wire());
    assert!(connection.push_input(&message).is_empty());
    assert_eq!(connection.pixel_format(), format);
}

fn negotiate_desktop_size(connection: &mut RfbConnectionCore) {
    let mut message = vec![2, 0, 0, 1];
    message.extend_from_slice(&(-223_i32).to_be_bytes());
    assert!(connection.push_input(&message).is_empty());
    assert!(connection.supports_desktop_size());
}

fn queue_full_frame(
    connection: &mut RfbConnectionCore,
    frame: BgraFrameView<'_>,
) -> Result<FramebufferUpdateOutcome, RfbEncodeError> {
    let size = frame.size();
    connection.queue_framebuffer_update(
        frame,
        FramebufferUpdateRequest {
            incremental: false,
            rectangle: RfbRectangle {
                x: 0,
                y: 0,
                width: size.width(),
                height: size.height(),
            },
        },
    )
}
```

这些函数只位于 `#[cfg(test)]` 模块，不进入 crate 公共 API。

- [x] **步骤 2：写默认 Raw 更新金样失败测试**

在 `connection.rs` 测试模块加入：

```rust
#[test]
fn queues_cropped_raw_update_in_default_pixel_format() {
    let mut connection = completed_connection_with_size(2, 2);
    let pixels = [
        1, 2, 3, 255, 4, 5, 6, 255,
        7, 8, 9, 255, 10, 11, 12, 255,
    ];
    let frame = BgraFrameView::new(
        RfbSize::new(2, 2).unwrap(),
        8,
        &pixels,
    )
    .unwrap();

    assert_eq!(
        connection.queue_framebuffer_update(
            frame,
            FramebufferUpdateRequest {
                incremental: true,
                rectangle: RfbRectangle {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 2,
                },
            },
        ),
        Ok(FramebufferUpdateOutcome::RawQueued {
            rectangle: RfbRectangle {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            },
        })
    );

    assert_eq!(
        connection.take_output(),
        [
            0, 0, 0, 1,
            0, 1, 0, 0, 0, 1, 0, 2, 0, 0, 0, 0,
            4, 5, 6, 0,
            10, 11, 12, 0,
        ]
    );
}
```

- [x] **步骤 3：运行 Raw 金样测试确认失败**

运行：

```powershell
cargo test -p ipkvm-rfb connection::tests::queues_cropped_raw
```

预期：编译失败，提示更新方法、结果或编码错误尚未定义。

- [x] **步骤 4：定义更新 API 和 Raw 编码**

定义：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramebufferUpdateOutcome {
    RawQueued { rectangle: RfbRectangle },
    EmptyQueued,
    ResizeAnnounced { size: RfbSize },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbEncodeError {
    FramebufferTooLarge { actual: usize, maximum: usize },
    LengthOverflow,
    OutputQueueFull { attempted: usize, maximum: usize },
    HandshakeNotComplete,
    DesktopSizeNotNegotiated {
        announced: RfbSize,
        actual: RfbSize,
    },
}

impl RfbConnectionCore {
    pub fn queue_framebuffer_update(
        &mut self,
        frame: BgraFrameView<'_>,
        request: FramebufferUpdateRequest,
    ) -> Result<FramebufferUpdateOutcome, RfbEncodeError>;
}
```

Raw 消息规则：

- 消息头为 `[0, 0, 0, 1]`。
- rectangle header 依次为交集的 x、y、宽、高和有符号 encoding `0`。
- 按行读取 BGRA；忽略 alpha 和 stride 尾部。
- 使用当前 `RfbPixelFormat` 写每个像素。
- `incremental=true` 当前仍发送完整交集。
- 空交集写入 `[0, 0, 0, 0]` 并返回 `EmptyQueued`。

- [x] **步骤 5：写 stride、像素格式和空交集测试**

加入测试：

```rust
#[test]
fn raw_update_uses_stride_and_negotiated_rgb565() {
    let mut connection = completed_connection_with_size(2, 2);
    set_rgb565(&mut connection);
    let pixels = [
        0, 0, 255, 0, 0, 255, 0, 0, 99, 99, 99, 99,
        255, 0, 0, 0, 255, 255, 255, 0,
    ];
    let frame = BgraFrameView::new(
        RfbSize::new(2, 2).unwrap(),
        12,
        &pixels,
    )
    .unwrap();

    queue_full_frame(&mut connection, frame).unwrap();
    let output = connection.take_output();
    assert_eq!(&output[16..], [0x00, 0xf8, 0xe0, 0x07, 0x1f, 0x00, 0xff, 0xff]);
}

#[test]
fn empty_intersection_queues_zero_rectangle_update() {
    let mut connection = completed_connection_with_size(2, 2);
    let pixels = [0_u8; 16];
    let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();
    let outcome = connection
        .queue_framebuffer_update(
            frame,
            FramebufferUpdateRequest {
                incremental: true,
                rectangle: RfbRectangle {
                    x: 10,
                    y: 10,
                    width: 1,
                    height: 1,
                },
            },
        )
        .unwrap();

    assert_eq!(outcome, FramebufferUpdateOutcome::EmptyQueued);
    assert_eq!(connection.take_output(), [0, 0, 0, 0]);
}
```

运行这些测试，确认在对应实现前失败；实现后重新运行并确认通过。

- [x] **步骤 6：写 DesktopSize 协商和独立更新失败测试**

加入：

```rust
#[test]
fn resize_requires_desktop_size_negotiation() {
    let mut connection = completed_connection_with_size(2, 2);
    let pixels = [0_u8; 24];
    let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();

    assert_eq!(
        queue_full_frame(&mut connection, frame),
        Err(RfbEncodeError::DesktopSizeNotNegotiated {
            announced: RfbSize::new(2, 2).unwrap(),
            actual: RfbSize::new(3, 2).unwrap(),
        })
    );
    assert!(connection.take_output().is_empty());
}

#[test]
fn negotiated_resize_is_a_standalone_desktop_size_update() {
    let mut connection = completed_connection_with_size(2, 2);
    negotiate_desktop_size(&mut connection);
    let pixels = [0_u8; 24];
    let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();

    assert_eq!(
        queue_full_frame(&mut connection, frame),
        Ok(FramebufferUpdateOutcome::ResizeAnnounced {
            size: RfbSize::new(3, 2).unwrap(),
        })
    );
    assert_eq!(
        connection.take_output(),
        [
            0, 0, 0, 1,
            0, 0, 0, 0, 0, 3, 0, 2, 0xff, 0xff, 0xff, 0x21,
        ]
    );

    assert!(matches!(
        queue_full_frame(&mut connection, frame),
        Ok(FramebufferUpdateOutcome::RawQueued { .. })
    ));
}
```

- [x] **步骤 7：实现 DesktopSize 独立更新**

尺寸变化时：

- 当前偏好不含 `-223`，返回 `DesktopSizeNotNegotiated`。
- 已协商时构造唯一 rectangle：x=0、y=0、新宽高、encoding=-223、无 body。
- 本次不附带 Raw 数据。
- 先检查完整消息能够追加到输出队列；追加成功后才更新已声明尺寸。
- 下一次请求在新尺寸下输出 Raw。
- 只有真实尺寸变化才发送 `DesktopSize`。

- [x] **步骤 8：写输出事务性和容量边界失败测试**

加入：

```rust
#[test]
fn encoding_errors_leave_output_and_announced_size_unchanged() {
    let mut config = config_with_size(2, 2);
    config.limits.max_framebuffer_bytes = 16;
    config.limits.max_queued_output_bytes = 64;
    let mut connection = complete(config);
    let original_output = connection.take_output();
    assert!(original_output.is_empty());

    let pixels = [0_u8; 24];
    let frame = BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &pixels).unwrap();
    assert_eq!(
        queue_full_frame(&mut connection, frame),
        Err(RfbEncodeError::FramebufferTooLarge {
            actual: 24,
            maximum: 16,
        })
    );
    assert!(connection.take_output().is_empty());
}

#[test]
fn update_before_handshake_does_not_consume_banner() {
    let mut connection = RfbConnectionCore::new(config_with_size(2, 2)).unwrap();
    let pixels = [0_u8; 16];
    let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();

    assert_eq!(
        queue_full_frame(&mut connection, frame),
        Err(RfbEncodeError::HandshakeNotComplete)
    );
    assert_eq!(connection.take_output(), b"RFB 003.008\n");
}

#[test]
fn output_capacity_error_preserves_the_first_queued_update() {
    let mut config = config_with_size(2, 2);
    config.limits.max_framebuffer_bytes = 16;
    config.limits.max_queued_output_bytes = 50;
    let mut connection = complete(config);
    let pixels = [0_u8; 16];
    let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();

    queue_full_frame(&mut connection, frame).unwrap();
    assert_eq!(
        queue_full_frame(&mut connection, frame),
        Err(RfbEncodeError::OutputQueueFull {
            attempted: 64,
            maximum: 50,
        })
    );
    assert_eq!(
        connection.take_output(),
        [
            0, 0, 0, 1,
            0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
        ]
    );
}

#[test]
fn failed_desktop_size_queue_does_not_commit_new_size() {
    let mut config = config_with_size(2, 2);
    config.limits.max_framebuffer_bytes = 24;
    config.limits.max_queued_output_bytes = 50;
    let mut connection = complete(config);
    negotiate_desktop_size(&mut connection);

    let old_pixels = [0_u8; 16];
    let old_frame =
        BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &old_pixels).unwrap();
    queue_full_frame(&mut connection, old_frame).unwrap();
    connection
        .queue_framebuffer_update(
            old_frame,
            FramebufferUpdateRequest {
                incremental: true,
                rectangle: RfbRectangle {
                    x: 10,
                    y: 10,
                    width: 1,
                    height: 1,
                },
            },
        )
        .unwrap();

    let new_pixels = [0_u8; 24];
    let new_frame =
        BgraFrameView::new(RfbSize::new(3, 2).unwrap(), 12, &new_pixels).unwrap();
    assert_eq!(
        queue_full_frame(&mut connection, new_frame),
        Err(RfbEncodeError::OutputQueueFull {
            attempted: 52,
            maximum: 50,
        })
    );
    assert_eq!(connection.take_output().len(), 36);
    assert!(matches!(
        queue_full_frame(&mut connection, new_frame),
        Ok(FramebufferUpdateOutcome::ResizeAnnounced { size })
            if size == RfbSize::new(3, 2).unwrap()
    ));
}
```

在 `server.rs` 加入检查运算测试：

```rust
#[test]
fn raw_and_queue_length_helpers_report_overflow() {
    assert_eq!(
        checked_raw_message_len(usize::MAX, 2, 4),
        Err(RfbEncodeError::LengthOverflow)
    );
    assert_eq!(
        checked_output_len(usize::MAX, 1),
        Err(RfbEncodeError::LengthOverflow)
    );
}
```

- [x] **步骤 9：实现先构造后提交并验证任务 5**

服务器编码辅助先在临时 `Vec<u8>` 中构造完整消息。连接在检查：

```text
current_output_len + message_len <= max_queued_output_bytes
```

后一次性 `extend_from_slice`。任何错误都不修改输出、像素格式、encoding 偏好或已声明尺寸。

实现并统一使用：

```rust
pub(crate) fn checked_raw_message_len(
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
) -> Result<usize, RfbEncodeError>;

pub(crate) fn checked_output_len(
    current: usize,
    additional: usize,
) -> Result<usize, RfbEncodeError>;
```

运行：

```powershell
cargo fmt --all
cargo test -p ipkvm-rfb connection
cargo test -p ipkvm-rfb protocol::server
cargo test -p ipkvm-rfb
git diff --check
```

预期：全部通过。

- [x] **步骤 10：提交任务 5**

```powershell
git add crates/ipkvm-rfb
git commit -m "feat: encode RFB framebuffer updates (#2)"
```

---

### 任务 6：完整转录、性质测试和长期文档收口

**文件：**

- 新建：`crates/ipkvm-rfb/tests/protocol_transcript.rs`
- 修改：`crates/ipkvm-rfb/src/protocol/client.rs`
- 修改：`crates/ipkvm-rfb/src/protocol/pixel_format.rs`
- 修改：`crates/ipkvm-rfb/src/framebuffer.rs`
- 修改：`crates/ipkvm-rfb/src/connection.rs`
- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`

**接口：**

- 只使用前五个任务的公共 API 完成端到端内存转录。
- 性质测试验证分块不变量、几何边界、输出长度和不 panic。
- 不新增 TCP、WebSocket 或测试专用生产接口。

- [x] **步骤 1：写完整公共 API 转录测试**

在 `protocol_transcript.rs` 写入一个完整流程：

```rust
#[test]
fn completes_handshake_negotiation_request_and_raw_update() {
    let size = RfbSize::new(2, 1).unwrap();
    let config = RfbConnectionConfig {
        desktop_name: "my_ipkvm".to_owned(),
        initial_size: size,
        limits: RfbProtocolLimits::default(),
    };
    let mut connection = RfbConnectionCore::new(config).unwrap();

    assert_eq!(connection.take_output(), b"RFB 003.008\n");
    assert!(connection.push_input(b"RFB 003.008\n").is_empty());
    assert_eq!(connection.take_output(), [1, 1]);
    assert!(connection.push_input(&[1]).is_empty());
    assert_eq!(connection.take_output(), [0, 0, 0, 0]);
    assert_eq!(
        connection.push_input(&[1]),
        vec![Ok(RfbEvent::HandshakeCompleted { shared: true })]
    );
    assert!(!connection.take_output().is_empty());

    let mut client_messages = vec![2, 0, 0, 2];
    client_messages.extend_from_slice(&0_i32.to_be_bytes());
    client_messages.extend_from_slice(&(-223_i32).to_be_bytes());
    client_messages.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 2, 0, 1]);
    assert_eq!(
        connection.push_input(&client_messages),
        vec![Ok(RfbEvent::FramebufferUpdateRequested(
            FramebufferUpdateRequest {
                incremental: false,
                rectangle: RfbRectangle {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
            }
        ))]
    );

    let pixels = [1, 2, 3, 255, 4, 5, 6, 255];
    let frame = BgraFrameView::new(size, 8, &pixels).unwrap();
    connection
        .queue_framebuffer_update(
            frame,
            FramebufferUpdateRequest {
                incremental: false,
                rectangle: RfbRectangle {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
            },
        )
        .unwrap();
    assert_eq!(
        connection.take_output(),
        [
            0, 0, 0, 1,
            0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0,
            1, 2, 3, 0, 4, 5, 6, 0,
        ]
    );
}
```

- [x] **步骤 2：运行转录测试并修复公共 API 差异**

运行：

```powershell
cargo test -p ipkvm-rfb --test protocol_transcript
```

预期：首次运行只允许因公共导出遗漏或行为差异失败。先确认失败与测试目标一致，再补公共导出或修复根因；不得复制内部编码逻辑到测试。

- [x] **步骤 3：写任意分块等价性质测试**

在 `client.rs` 测试模块先定义每种支持消息的代表字节：

```rust
fn representative_client_messages() -> Vec<Vec<u8>> {
    let mut pixel_format = vec![0, 9, 8, 7];
    pixel_format.extend_from_slice(&RfbPixelFormat::default_bgrx8888().to_wire());

    let mut encodings = vec![2, 0, 0, 3];
    encodings.extend_from_slice(&0_i32.to_be_bytes());
    encodings.extend_from_slice(&(-223_i32).to_be_bytes());
    encodings.extend_from_slice(&12_345_i32.to_be_bytes());

    vec![
        pixel_format,
        encodings,
        vec![3, 1, 0, 1, 0, 2, 0, 3, 0, 4],
        vec![4, 1, 0, 0, 0, 0, 0xff, 0x0d],
        vec![5, 3, 0, 10, 0, 20],
        vec![6, 0, 0, 0, 0, 0, 0, 2, 0x41, 0xff],
        vec![150, 1, 0, 5, 0, 6, 0, 7, 0, 8],
    ]
}

fn representative_client_message_stream() -> Vec<u8> {
    representative_client_messages()
        .into_iter()
        .flatten()
        .collect()
}

#[test]
fn every_message_decodes_at_every_split_boundary() {
    for bytes in representative_client_messages() {
        let mut single = ClientMessageDecoder::new(RfbProtocolLimits::default());
        let expected = single.push(&bytes);
        for split in 0..=bytes.len() {
            let mut chunked =
                ClientMessageDecoder::new(RfbProtocolLimits::default());
            let mut actual = chunked.push(&bytes[..split]);
            actual.extend(chunked.push(&bytes[split..]));
            assert_eq!(actual, expected, "split={split}, bytes={bytes:?}");
        }
    }
}
```

在 `connection.rs` 测试模块覆盖版本消息的全部分片边界：

```rust
#[test]
fn protocol_version_accepts_every_split_boundary() {
    for split in 0..=12 {
        let mut connection = RfbConnectionCore::new(config()).unwrap();
        connection.take_output();
        assert!(connection
            .push_input(&b"RFB 003.008\n"[..split])
            .is_empty());
        assert!(connection
            .push_input(&b"RFB 003.008\n"[split..])
            .is_empty());
        assert_eq!(
            connection.state(),
            RfbConnectionState::AwaitingSecuritySelection
        );
        assert_eq!(connection.take_output(), [1, 1]);
    }
}
```

再在 `client.rs` 使用 `proptest` 生成连续消息流的每段长度：

```rust
proptest! {
    #[test]
    fn arbitrary_chunking_matches_single_push(
        chunks in proptest::collection::vec(1_usize..16, 1..64)
    ) {
        let bytes = representative_client_message_stream();
        let mut single = ClientMessageDecoder::new(RfbProtocolLimits::default());
        let expected = single.push(&bytes);

        let mut chunked = ClientMessageDecoder::new(RfbProtocolLimits::default());
        let mut actual = Vec::new();
        let mut offset = 0;
        for requested in chunks {
            if offset == bytes.len() {
                break;
            }
            let end = offset.saturating_add(requested).min(bytes.len());
            actual.extend(chunked.push(&bytes[offset..end]));
            offset = end;
        }
        actual.extend(chunked.push(&bytes[offset..]));

        prop_assert_eq!(actual, expected);
    }
}
```

- [x] **步骤 4：写像素、矩形和随机输入性质测试**

在 `pixel_format.rs` 测试模块定义：

```rust
fn supported_test_formats() -> [RfbPixelFormat; 4] {
    [
        RfbPixelFormat::default_bgrx8888(),
        RfbPixelFormat::new(32, 24, true, 255, 255, 255, 16, 8, 0).unwrap(),
        RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap(),
        RfbPixelFormat::new(8, 8, false, 7, 7, 3, 5, 2, 0).unwrap(),
    ]
}
```

随后分别在对应内部测试模块加入：

```rust
proptest! {
    #[test]
    fn encoded_pixel_length_matches_bits_per_pixel(
        blue in any::<u8>(),
        green in any::<u8>(),
        red in any::<u8>(),
    ) {
        for format in supported_test_formats() {
            let mut output = Vec::new();
            format.write_bgr(&mut output, blue, green, red);
            prop_assert_eq!(output.len(), format.bytes_per_pixel());
        }
    }

    #[test]
    fn rectangle_intersection_stays_inside_frame(
        frame_width in 1_u16..=u16::MAX,
        frame_height in 1_u16..=u16::MAX,
        x in any::<u16>(),
        y in any::<u16>(),
        width in any::<u16>(),
        height in any::<u16>(),
    ) {
        let frame = RfbSize::new(frame_width, frame_height).unwrap();
        let rectangle = RfbRectangle { x, y, width, height };
        if let Some(intersection) = rectangle.intersection(frame) {
            prop_assert!((intersection.x as u32 + intersection.width as u32) <= frame_width as u32);
            prop_assert!((intersection.y as u32 + intersection.height as u32) <= frame_height as u32);
        }
    }

    #[test]
    fn random_client_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
        let _ = decoder.push(&bytes);
        prop_assert!(decoder.buffered_len() <= RfbProtocolLimits::default().max_buffered_input_bytes);
    }
}
```

内部测试辅助只能放在 `#[cfg(test)]` 模块，不进入公共 API。

- [x] **步骤 5：写输出失败事务性性质测试**

在 `connection.rs` 测试模块加入：

```rust
proptest! {
    #[test]
    fn output_capacity_failure_keeps_first_update_and_connection_state(
        pixels in prop::array::uniform16(any::<u8>())
    ) {
        let mut config = config_with_size(2, 2);
        config.limits.max_framebuffer_bytes = 16;
        config.limits.max_queued_output_bytes = 50;
        let mut connection = complete(config);
        let frame =
            BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();
        let before_state = connection.state();
        let before_format = connection.pixel_format();
        let before_encodings = connection.encoding_preferences().to_vec();

        queue_full_frame(&mut connection, frame).unwrap();
        prop_assert_eq!(
            queue_full_frame(&mut connection, frame),
            Err(RfbEncodeError::OutputQueueFull {
                attempted: 64,
                maximum: 50,
            })
        );

        let mut expected = vec![
            0, 0, 0, 1,
            0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0,
        ];
        for pixel in pixels.chunks_exact(4) {
            expected.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0]);
        }
        prop_assert_eq!(connection.take_output(), expected);
        prop_assert_eq!(connection.state(), before_state);
        prop_assert_eq!(connection.pixel_format(), before_format);
        prop_assert_eq!(
            connection.encoding_preferences(),
            before_encodings.as_slice()
        );

        prop_assert!(queue_full_frame(&mut connection, frame).is_ok());
    }
}
```

- [x] **步骤 6：运行性质测试并处理真实失败**

运行：

```powershell
cargo test -p ipkvm-rfb
```

预期：全部通过。若性质测试发现失败，先把最小反例固化为普通回归测试并确认失败，再修复生产代码根因，最后重新运行全部 `ipkvm-rfb` 测试。

- [x] **步骤 7：更新长期状态文档**

更新 `README.md`：

- 当前状态增加“RFB 3.8 纯协议核心已完成”。
- `ipkvm-rfb` 描述改为已支持 None 握手、增量客户端消息、Raw 和 DesktopSize；TCP 和 WebSocket 仍未实现。

更新 `docs/ipkvm-coarse-design.md` 阶段 0：

- 把“写 RFB 3.8 握手、Raw、DesktopSize 协议样例测试”移到已完成。
- 把“未知伪编码忽略测试”改为准确完成描述：未知 encoding 保留且不使连接失败，未知客户端消息类型因无法确定长度而致命失败。
- “用模拟帧缓冲跑通普通 VNC 客户端和 noVNC”继续留在待完成，因为本次没有 TCP 或 WebSocket。

- [x] **步骤 8：执行最终验证**

依次运行：

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

验收标准：

- 所有命令退出码为 0。
- 没有警告、失败测试或格式差异。
- `git status --short` 只包含本 issue 预期文件。
- 不需要人工测试；真实 VNC 客户端兼容性明确留给 TCP 接入 issue。

- [x] **步骤 9：提交任务 6**

```powershell
git add README.md docs/ipkvm-coarse-design.md crates/ipkvm-rfb
git commit -m "test: harden RFB protocol core (#2)"
```

- [x] **步骤 10：进行实现审查和 issue 收口**

按 `requesting-code-review` 对设计 `docs/superpowers/specs/2026-07-31-rfb-38-protocol-core-design.md`、本计划和实现 diff 做审查。修复所有严重和重要问题，并为每个行为修复先增加失败回归测试。

审查通过后：

- 在 Gitea issue `#2` 记录提交、自动化测试证据和无人工测试原因。
- 确认 issue 验收项全部满足后关闭 issue。
- 使用 `finishing-a-development-branch` 完成分支整合，不在未验证状态直接合并。
