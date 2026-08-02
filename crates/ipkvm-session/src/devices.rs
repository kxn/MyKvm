//! 设备枚举：视频采集设备与串口设备。

use thiserror::Error;

/// 视频采集设备。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoDevice {
    pub id: String,
    pub display_name: String,
}

/// 串口设备。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SerialDevice {
    pub path: String,
    pub port_type: String,
}

#[derive(Debug, Error)]
pub enum DeviceListError {
    #[error("video device enumeration failed: {0}")]
    Video(String),
    #[error("serial device enumeration failed: {0}")]
    Serial(String),
}

/// 枚举视频采集设备（复用 ipkvm-video 的 camera::list_cameras）。
pub fn list_video_devices() -> Result<Vec<VideoDevice>, DeviceListError> {
    let cameras =
        ipkvm_video::camera::list_cameras().map_err(|e| DeviceListError::Video(e.to_string()))?;
    Ok(cameras
        .iter()
        .map(|c| VideoDevice {
            id: c.id.clone(),
            display_name: c.display_name.clone(),
        })
        .collect())
}

/// 枚举串口设备（serialport::available_ports）。
///
/// 测试里无条件调用本函数，因此不做 `#[cfg(feature = "serial")]` 门控；
/// 依赖 serialport 为可选依赖时，串口枚举随 feature 联动（无 feature 时
/// 返回空列表，不 panic）。
#[cfg(feature = "serial")]
pub fn list_serial_devices() -> Result<Vec<SerialDevice>, DeviceListError> {
    let ports =
        serialport::available_ports().map_err(|e| DeviceListError::Serial(e.to_string()))?;
    Ok(ports
        .iter()
        .map(|p| SerialDevice {
            path: p.port_name.clone(),
            port_type: format!("{:?}", p.port_type),
        })
        .collect())
}

// 非 serial feature 下返回空列表（避免编译错误，测试仍通过）。
#[cfg(not(feature = "serial"))]
pub fn list_serial_devices() -> Result<Vec<SerialDevice>, DeviceListError> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_video_devices_returns_list() {
        // mock/无相机下应返回 Ok（可能为空列表），不 panic。
        let devices = list_video_devices().unwrap();
        let _ = devices;
    }

    #[test]
    fn list_serial_devices_returns_list() {
        // 无串口下应返回 Ok（可能为空列表），不 panic。
        let devices = list_serial_devices().unwrap();
        let _ = devices;
    }
}
