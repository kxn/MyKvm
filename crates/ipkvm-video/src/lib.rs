//! 视频采集抽象。

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Yuy2,
    Nv12,
    Rgb,
    Mjpeg,
    H264,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub pixel_format: PixelFormat,
}

impl VideoFormat {
    pub fn new(width: u32, height: u32, frames_per_second: u32, pixel_format: PixelFormat) -> Self {
        Self {
            width,
            height,
            frames_per_second,
            pixel_format,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoDeviceInfo {
    pub id: String,
    pub display_name: String,
    pub backend: String,
    pub supported_formats: Vec<VideoFormat>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoFrame {
    pub timestamp_millis: u64,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub data: Vec<u8>,
}

pub trait FrameSource {
    fn latest_frame(&self) -> Option<VideoFrame>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_format_records_dimensions_and_pixel_format() {
        let format = VideoFormat::new(1920, 1080, 60, PixelFormat::Mjpeg);

        assert_eq!(format.width, 1920);
        assert_eq!(format.height, 1080);
        assert_eq!(format.frames_per_second, 60);
        assert_eq!(format.pixel_format, PixelFormat::Mjpeg);
    }
}
