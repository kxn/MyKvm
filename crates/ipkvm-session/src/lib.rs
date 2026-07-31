//! 控制台会话抽象。

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleSessionConfig {
    video_device_id: String,
    serial_port: String,
}

impl ConsoleSessionConfig {
    pub fn new(video_device_id: impl Into<String>, serial_port: impl Into<String>) -> Self {
        Self {
            video_device_id: video_device_id.into(),
            serial_port: serial_port.into(),
        }
    }

    pub fn video_device_id(&self) -> &str {
        &self.video_device_id
    }

    pub fn serial_port(&self) -> &str {
        &self.serial_port
    }
}

#[derive(Debug)]
pub struct ConsoleSession {
    config: ConsoleSessionConfig,
}

impl ConsoleSession {
    pub fn new(config: ConsoleSessionConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ConsoleSessionConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_config_preserves_selected_devices() {
        let config = ConsoleSessionConfig::new("video0", "COM3");

        assert_eq!(config.video_device_id(), "video0");
        assert_eq!(config.serial_port(), "COM3");
    }
}
