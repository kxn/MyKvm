//! YUV4MPEG2 测试素材解析与 YUV420 到 BGRA 转换（mock 功能）。
//!
//! 只支持 8 位 4:2:0 平面格式：`C420`、`C420jpeg`、`C420paldv`、
//! `C420mpeg2`。真实采集后端不经过这里。

use thiserror::Error;

const MAGIC: &[u8] = b"YUV4MPEG2 ";

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Y4mError {
    #[error("missing YUV4MPEG2 magic")]
    MissingMagic,
    #[error("malformed Y4M header or frame marker")]
    MalformedHeader,
    #[error("unsupported chroma subsampling: {0}")]
    UnsupportedChroma(String),
    #[error("truncated Y4M frame: expected {expected} bytes, got {actual}")]
    TruncatedFrame { expected: usize, actual: usize },
    #[error("Y4M asset contains no frames")]
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Y4mAsset {
    width: u32,
    height: u32,
    frames: Vec<Vec<u8>>,
}

impl Y4mAsset {
    pub fn parse(bytes: &[u8]) -> Result<Self, Y4mError> {
        let header_end = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or(Y4mError::MalformedHeader)?;
        let header = &bytes[..header_end];
        if !header.starts_with(MAGIC) {
            return Err(Y4mError::MissingMagic);
        }

        let mut width: Option<u32> = None;
        let mut height: Option<u32> = None;
        let mut chroma = None;
        for field in header[MAGIC.len()..].split(|byte| *byte == b' ') {
            if field.is_empty() {
                continue;
            }
            let field = std::str::from_utf8(field).map_err(|_| Y4mError::MalformedHeader)?;
            if let Some(value) = field.strip_prefix('W') {
                width = value.parse().ok();
            } else if let Some(value) = field.strip_prefix('H') {
                height = value.parse().ok();
            } else if field.strip_prefix('C').is_some() {
                chroma = Some(field.to_string());
            }
        }

        let (Some(width), Some(height)) = (width, height) else {
            return Err(Y4mError::MalformedHeader);
        };
        if width == 0 || height == 0 {
            return Err(Y4mError::MalformedHeader);
        }

        let chroma = chroma.ok_or(Y4mError::MalformedHeader)?;
        if !matches!(
            chroma.as_str(),
            "C420" | "C420jpeg" | "C420paldv" | "C420mpeg2"
        ) {
            return Err(Y4mError::UnsupportedChroma(chroma));
        }

        let uv_width = usize::try_from(width.div_ceil(2)).map_err(|_| Y4mError::MalformedHeader)?;
        let uv_height =
            usize::try_from(height.div_ceil(2)).map_err(|_| Y4mError::MalformedHeader)?;
        let y_len = usize::try_from(width * height).map_err(|_| Y4mError::MalformedHeader)?;
        let frame_len = y_len + 2 * uv_width * uv_height;

        let mut rest = &bytes[header_end + 1..];
        let mut frames = Vec::new();
        while !rest.is_empty() {
            let marker_end = rest
                .iter()
                .position(|byte| *byte == b'\n')
                .ok_or(Y4mError::MalformedHeader)?;
            if !rest[..marker_end].starts_with(b"FRAME") {
                return Err(Y4mError::MalformedHeader);
            }
            rest = &rest[marker_end + 1..];
            if rest.len() < frame_len {
                return Err(Y4mError::TruncatedFrame {
                    expected: frame_len,
                    actual: rest.len(),
                });
            }
            frames.push(rest[..frame_len].to_vec());
            rest = &rest[frame_len..];
        }

        if frames.is_empty() {
            return Err(Y4mError::Empty);
        }

        Ok(Self {
            width,
            height,
            frames,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn frame(&self, index: usize) -> Option<&[u8]> {
        self.frames.get(index).map(Vec::as_slice)
    }

    /// 把一帧 YUV420 平面数据转换为 `BGRA8888` 字节，不包含行填充。
    pub fn frame_bgra(&self, index: usize) -> Option<Vec<u8>> {
        let mut output = Vec::new();
        self.frame_bgra_into(index, &mut output)?;
        Some(output)
    }

    /// 把一帧 YUV420 平面数据转换为 `BGRA8888`，写入 caller 传入的 `out`（复用容量，
    /// 消除每帧 Vec 分配）。见调研阶段 1.2（issue #19）。
    pub fn frame_bgra_into(&self, index: usize, out: &mut Vec<u8>) -> Option<()> {
        let frame = self.frames.get(index)?;
        let width = usize::try_from(self.width).ok()?;
        let height = usize::try_from(self.height).ok()?;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let y_len = width * height;
        out.clear();
        out.reserve(width * height * 4);

        for row in 0..height {
            for column in 0..width {
                let y = frame[row * width + column];
                let u = frame[y_len + (row / 2) * uv_width + column / 2];
                let v = frame[y_len + uv_width * uv_height + (row / 2) * uv_width + column / 2];
                let (r, g, b) = yuv_to_bgr(y, u, v);
                out.extend_from_slice(&[b, g, r, 255]);
            }
        }
        Some(())
    }
}

fn yuv_to_bgr(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let c = i32::from(y) - 16;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
    let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
    let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::Y4mError;
    use crate::y4m::Y4mAsset;

    fn yuv420_frame_bytes(width: u32, height: u32) -> Vec<u8> {
        let y_len = (width * height) as usize;
        let uv_len = (width.div_ceil(2) * height.div_ceil(2)) as usize;
        vec![0; y_len + 2 * uv_len]
    }

    fn y4m_bytes(width: u32, height: u32, chroma: &str, frame_count: usize) -> Vec<u8> {
        let mut bytes =
            format!("YUV4MPEG2 W{width} H{height} F10:1 Ip A1:1 {chroma}\n").into_bytes();
        let frame = yuv420_frame_bytes(width, height);
        for _ in 0..frame_count {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&frame);
        }
        bytes
    }

    #[test]
    fn parses_420_asset_with_multiple_frames() {
        let bytes = y4m_bytes(4, 2, "C420jpeg", 2);
        let asset = Y4mAsset::parse(&bytes).unwrap();

        assert_eq!(asset.width(), 4);
        assert_eq!(asset.height(), 2);
        assert_eq!(asset.frame_count(), 2);
        assert_eq!(asset.frame(0).unwrap().len(), 12);
        assert_eq!(asset.frame(1).unwrap().len(), 12);
        assert!(asset.frame(2).is_none());
    }

    #[test]
    fn parses_plain_420_chroma_field() {
        let bytes = y4m_bytes(2, 2, "C420", 1);
        assert!(Y4mAsset::parse(&bytes).is_ok());
    }

    #[test]
    fn rejects_unsupported_chroma_subsampling() {
        let bytes = y4m_bytes(2, 2, "C444", 1);
        assert_eq!(
            Y4mAsset::parse(&bytes),
            Err(Y4mError::UnsupportedChroma("C444".to_string()))
        );
    }

    #[test]
    fn rejects_missing_magic() {
        let bytes = b"not a y4m stream\n".to_vec();
        assert_eq!(Y4mAsset::parse(&bytes), Err(Y4mError::MissingMagic));
    }

    #[test]
    fn rejects_truncated_frame_payload() {
        let mut bytes = y4m_bytes(4, 2, "C420", 1);
        bytes.truncate(bytes.len() - 1);
        assert!(matches!(
            Y4mAsset::parse(&bytes),
            Err(Y4mError::TruncatedFrame { .. })
        ));
    }

    #[test]
    fn converts_black_and_white_yuv420_to_bgra() {
        let mut bytes = b"YUV4MPEG2 W2 H2 F10:1 Ip A1:1 C420\nFRAME\n".to_vec();
        // 黑色帧：Y=0，U=128，V=128
        bytes.extend_from_slice(&[0; 6]);
        let black_len = bytes.len();
        bytes[black_len - 2] = 128;
        bytes[black_len - 1] = 128;
        bytes.extend_from_slice(b"FRAME\n");
        // 白色帧：Y=255，U=128，V=128
        bytes.extend_from_slice(&[255; 4]);
        bytes.extend_from_slice(&[128; 2]);

        let asset = Y4mAsset::parse(&bytes).unwrap();
        let black = asset.frame_bgra(0);
        let white = asset.frame_bgra(1);

        assert_eq!(
            black.unwrap(),
            vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]
        );
        assert_eq!(white.unwrap(), vec![255u8; 16]);
    }

    /// buffer 复用：连续两次 frame_bgra_into 写入同一 buffer，第二次数据正确覆盖。
    #[test]
    fn frame_bgra_into_reuses_buffer_across_calls() {
        let mut bytes = b"YUV4MPEG2 W2 H2 F10:1 Ip A1:1 C420\nFRAME\n".to_vec();
        bytes.extend_from_slice(&[0; 6]); // 黑色帧 Y=0,U=128,V=128
        let black_len = bytes.len();
        bytes[black_len - 2] = 128;
        bytes[black_len - 1] = 128;
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend_from_slice(&[255; 4]); // 白色帧
        bytes.extend_from_slice(&[128; 2]);

        let asset = Y4mAsset::parse(&bytes).unwrap();
        let mut buf = Vec::new();

        asset.frame_bgra_into(0, &mut buf).unwrap();
        let black = buf.clone();
        assert_eq!(black.len(), 16);
        assert!(black.iter().all(|&b| b == 0 || b == 255));

        // 第二次写入同一 buffer（白色帧），数据应正确覆盖黑色帧。
        asset.frame_bgra_into(1, &mut buf).unwrap();
        assert_eq!(buf, vec![255u8; 16]);
    }
}
