use thiserror::Error;

/// 视频采集设备的稳定选择信息。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoDevice {
    pub id: String,
    pub display_name: String,
}

/// CH9329 等控制串口的选择信息。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SerialDevice {
    pub path: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum DeviceProviderError {
    #[error("video device enumeration failed: {0}")]
    Video(String),
    #[error("serial device enumeration failed: {0}")]
    Serial(String),
}

/// 只负责枚举设备，不打开独占的相机或串口句柄。
pub trait DeviceInventoryProvider: Send + Sync {
    fn list_video_devices(&self) -> Result<Vec<VideoDevice>, DeviceProviderError>;
    fn list_serial_devices(&self) -> Result<Vec<SerialDevice>, DeviceProviderError>;
}

/// 测试和 browser fixture 使用的静态 provider。
#[derive(Clone, Debug)]
pub struct StaticDeviceInventoryProvider {
    video: Result<Vec<VideoDevice>, DeviceProviderError>,
    serial: Result<Vec<SerialDevice>, DeviceProviderError>,
}

impl StaticDeviceInventoryProvider {
    pub fn new(video: Vec<VideoDevice>, serial: Vec<SerialDevice>) -> Self {
        Self {
            video: Ok(video),
            serial: Ok(serial),
        }
    }

    pub fn with_errors(video: Option<&str>, serial: Option<&str>) -> Self {
        Self {
            video: video.map_or_else(
                || Ok(Vec::new()),
                |detail| Err(DeviceProviderError::Video(detail.to_owned())),
            ),
            serial: serial.map_or_else(
                || Ok(Vec::new()),
                |detail| Err(DeviceProviderError::Serial(detail.to_owned())),
            ),
        }
    }
}

impl DeviceInventoryProvider for StaticDeviceInventoryProvider {
    fn list_video_devices(&self) -> Result<Vec<VideoDevice>, DeviceProviderError> {
        self.video.clone()
    }

    fn list_serial_devices(&self) -> Result<Vec<SerialDevice>, DeviceProviderError> {
        self.serial.clone()
    }
}

#[cfg(feature = "platform")]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProductionDeviceInventoryProvider;

#[cfg(feature = "platform")]
impl DeviceInventoryProvider for ProductionDeviceInventoryProvider {
    fn list_video_devices(&self) -> Result<Vec<VideoDevice>, DeviceProviderError> {
        ipkvm_video::camera::list_cameras()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|device| VideoDevice {
                        id: device.id,
                        display_name: device.display_name,
                    })
                    .collect()
            })
            .map_err(|error| DeviceProviderError::Video(error.to_string()))
    }

    fn list_serial_devices(&self) -> Result<Vec<SerialDevice>, DeviceProviderError> {
        serialport::available_ports()
            .map(|ports| ports.into_iter().map(map_serial_device).collect())
            .map_err(|error| DeviceProviderError::Serial(error.to_string()))
    }
}

#[cfg(feature = "platform")]
fn map_serial_device(port: serialport::SerialPortInfo) -> SerialDevice {
    let display_name = match &port.port_type {
        serialport::SerialPortType::UsbPort(info) => info
            .product
            .as_deref()
            .filter(|name| !name.is_empty())
            .or(info.manufacturer.as_deref().filter(|name| !name.is_empty()))
            .unwrap_or(&port.port_name)
            .to_owned(),
        _ => port.port_name.clone(),
    };
    SerialDevice {
        path: port.port_name,
        display_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_provider_returns_independent_video_and_serial_lists() {
        let provider = StaticDeviceInventoryProvider::new(
            vec![VideoDevice {
                id: "cam0".into(),
                display_name: "Camera 0".into(),
            }],
            vec![SerialDevice {
                path: "COM9".into(),
                display_name: "CH9329".into(),
            }],
        );

        assert_eq!(
            provider.list_video_devices().unwrap(),
            vec![VideoDevice {
                id: "cam0".into(),
                display_name: "Camera 0".into(),
            }]
        );
        assert_eq!(
            provider.list_serial_devices().unwrap(),
            vec![SerialDevice {
                path: "COM9".into(),
                display_name: "CH9329".into(),
            }]
        );
    }

    #[test]
    fn static_provider_can_report_each_enumeration_error() {
        let provider = StaticDeviceInventoryProvider::with_errors(
            Some("camera unavailable"),
            Some("serial unavailable"),
        );

        assert_eq!(
            provider.list_video_devices().unwrap_err(),
            DeviceProviderError::Video("camera unavailable".into())
        );
        assert_eq!(
            provider.list_serial_devices().unwrap_err(),
            DeviceProviderError::Serial("serial unavailable".into())
        );
    }
}
