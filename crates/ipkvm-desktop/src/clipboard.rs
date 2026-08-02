//! 剪贴板与截图保存：文本读取、图像复制到剪贴板、JPEG 文件保存。

use std::path::Path;

use thiserror::Error;

use crate::frame::RgbaFrame;

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard read failed: {0}")]
    Read(String),
    #[error("clipboard write failed: {0}")]
    Write(String),
    #[error("jpeg encode failed: {0}")]
    Jpeg(String),
    #[error("jpeg file write failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("empty frame cannot be used")]
    EmptyFrame,
    #[error("frame dimensions exceed jpeg limits")]
    DimensionsTooLarge,
}

/// 剪贴板服务：纯静态方法，直接访问系统剪贴板。
pub struct ClipboardService;

impl ClipboardService {
    /// 读取系统剪贴板文本。
    pub fn read_text() -> Result<String, ClipboardError> {
        arboard::Clipboard::new()
            .map_err(|error| ClipboardError::Read(error.to_string()))?
            .get_text()
            .map_err(|error| ClipboardError::Read(error.to_string()))
    }

    /// 把 RGBA 帧复制为系统剪贴板图像。
    pub fn copy_image(frame: &RgbaFrame) -> Result<(), ClipboardError> {
        if frame.width == 0 || frame.height == 0 {
            return Err(ClipboardError::EmptyFrame);
        }
        let image = arboard::ImageData {
            width: frame.width as usize,
            height: frame.height as usize,
            bytes: std::borrow::Cow::Borrowed(&frame.pixels),
        };
        arboard::Clipboard::new()
            .map_err(|error| ClipboardError::Write(error.to_string()))?
            .set_image(image)
            .map_err(|error| ClipboardError::Write(error.to_string()))
    }
}

/// 把 RGBA 帧编码为 JPEG 写入文件（质量 85，与 headless 截图一致）。
pub fn save_jpeg(path: &Path, frame: &RgbaFrame) -> Result<(), ClipboardError> {
    if frame.width == 0 || frame.height == 0 {
        return Err(ClipboardError::EmptyFrame);
    }
    let width = u16::try_from(frame.width).map_err(|_| ClipboardError::DimensionsTooLarge)?;
    let height = u16::try_from(frame.height).map_err(|_| ClipboardError::DimensionsTooLarge)?;
    let file = std::fs::File::create(path)?;
    let encoder = jpeg_encoder::Encoder::new(file, 85);
    encoder
        .encode(
            &frame.pixels,
            width,
            height,
            jpeg_encoder::ColorType::Rgba,
        )
        .map_err(|error| ClipboardError::Jpeg(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::frame::RgbaFrame;

    #[test]
    fn save_jpeg_writes_non_empty_file() {
        let path = std::env::temp_dir().join(format!(
            "my_ipkvm-test-{}.jpg",
            std::process::id()
        ));
        let frame = RgbaFrame {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, 255],
        };

        save_jpeg(&path, &frame).unwrap();

        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_jpeg_rejects_empty_frame() {
        let path = std::env::temp_dir().join(format!(
            "my_ipkvm-empty-{}.jpg",
            std::process::id()
        ));
        let frame = RgbaFrame {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };

        assert!(save_jpeg(&path, &frame).is_err());
        let _ = fs::remove_file(path);
    }
}
