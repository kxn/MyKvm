//! 控制台会话抽象。

pub mod console_session;
pub mod rfb_connection;
pub mod rfb_input;
pub mod serial_stats;
pub mod session_manager;

use ipkvm_core::MouseMode;
use ipkvm_video::{PixelFormat, VideoFormat};

/// 单调时钟纳秒（`std::time::Instant::now()` 的时间差）——用于「最后输入
/// 时间」这类相对时间戳。零点为进程启动，不可跨进程比较；`Instant` 不可
/// 直接序列化，此函数供 `/api/status` 计算相对时长。
pub fn now_ns() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleSessionConfig {
    video_device_id: String,
    serial_port: String,
    video_format: VideoFormat,
    baud_rate: u32,
    keyboard_layout: KeyboardLayout,
    mouse_mode: MouseMode,
}

impl ConsoleSessionConfig {
    pub fn new(
        video_device_id: impl Into<String>,
        serial_port: impl Into<String>,
        video_format: VideoFormat,
        baud_rate: u32,
        keyboard_layout: KeyboardLayout,
        mouse_mode: MouseMode,
    ) -> Self {
        Self {
            video_device_id: video_device_id.into(),
            serial_port: serial_port.into(),
            video_format,
            baud_rate,
            keyboard_layout,
            mouse_mode,
        }
    }

    pub fn default_for_devices(
        video_device_id: impl Into<String>,
        serial_port: impl Into<String>,
    ) -> Self {
        Self::new(
            video_device_id,
            serial_port,
            VideoFormat::new(1920, 1080, 60, PixelFormat::Mjpeg),
            9_600,
            KeyboardLayout::EnUs,
            MouseMode::Absolute,
        )
    }

    pub fn video_device_id(&self) -> &str {
        &self.video_device_id
    }

    pub fn serial_port(&self) -> &str {
        &self.serial_port
    }

    pub fn video_format(&self) -> VideoFormat {
        self.video_format
    }

    pub fn baud_rate(&self) -> u32 {
        self.baud_rate
    }

    pub fn keyboard_layout(&self) -> KeyboardLayout {
        self.keyboard_layout
    }

    pub fn mouse_mode(&self) -> MouseMode {
        self.mouse_mode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardLayout {
    EnUs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_config_preserves_selected_devices() {
        let format = VideoFormat::new(1280, 720, 60, PixelFormat::Mjpeg);
        let config = ConsoleSessionConfig::new(
            "video0",
            "COM3",
            format,
            115_200,
            KeyboardLayout::EnUs,
            MouseMode::Relative,
        );

        assert_eq!(config.video_device_id(), "video0");
        assert_eq!(config.serial_port(), "COM3");
        assert_eq!(config.video_format(), format);
        assert_eq!(config.baud_rate(), 115_200);
        assert_eq!(config.keyboard_layout(), KeyboardLayout::EnUs);
        assert_eq!(config.mouse_mode(), MouseMode::Relative);
    }

    #[test]
    fn session_config_defaults_to_factory_serial_baud() {
        let config = ConsoleSessionConfig::default_for_devices("video0", "COM3");

        assert_eq!(
            config.video_format(),
            VideoFormat::new(1920, 1080, 60, PixelFormat::Mjpeg)
        );
        assert_eq!(config.baud_rate(), 9_600);
        assert_eq!(config.mouse_mode(), MouseMode::Absolute);
    }
}
