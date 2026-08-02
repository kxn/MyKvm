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
    /// 设备管理器显示名（USB 设备取 product/manufacturer，其余取端口名）。
    pub display_name: String,
}

#[derive(Debug, Error)]
pub enum DeviceListError {
    #[error("video device enumeration failed: {0}")]
    Video(String),
    #[error("serial device enumeration failed: {0}")]
    Serial(String),
}

/// 把相机枚举结果映射为会话侧设备描述（纯函数，便于格式化断言）。
fn map_camera(camera: &ipkvm_video::camera::CameraDeviceInfo) -> VideoDevice {
    VideoDevice {
        id: camera.id.clone(),
        display_name: camera.display_name.clone(),
    }
}

/// 枚举视频采集设备（复用 ipkvm-video 的 camera::list_cameras）。
pub fn list_video_devices() -> Result<Vec<VideoDevice>, DeviceListError> {
    let cameras =
        ipkvm_video::camera::list_cameras().map_err(|e| DeviceListError::Video(e.to_string()))?;
    Ok(cameras.iter().map(map_camera).collect())
}

#[cfg(feature = "serial")]
/// 把串口枚举结果映射为会话侧设备描述（纯函数，便于格式化断言）。
fn map_serial_port(port: &serialport::SerialPortInfo) -> SerialDevice {
    SerialDevice {
        path: port.port_name.clone(),
        display_name: serial_display_name(port),
    }
}

/// 设备管理器风格的名字：USB 设备优先 product，其次 manufacturer；
/// 其他类型（蓝牙/PCI/未知）直接使用端口名。
#[cfg(feature = "serial")]
fn serial_display_name(port: &serialport::SerialPortInfo) -> String {
    match &port.port_type {
        serialport::SerialPortType::UsbPort(info) => {
            if let Some(product) = &info.product
                && !product.is_empty()
            {
                product.clone()
            } else if let Some(manufacturer) = &info.manufacturer
                && !manufacturer.is_empty()
            {
                manufacturer.clone()
            } else {
                port.port_name.clone()
            }
        }
        _ => port.port_name.clone(),
    }
}

/// 枚举串口设备（serialport::available_ports）。
///
/// 测试无条件调用本函数，因此提供无 feature 空实现（返回空列表）而非
/// 门控整个函数；依赖 serialport 为可选依赖时，串口枚举随 feature 联动
/// （无 feature 时返回空列表，不 panic）。
#[cfg(feature = "serial")]
pub fn list_serial_devices() -> Result<Vec<SerialDevice>, DeviceListError> {
    let ports =
        serialport::available_ports().map_err(|e| DeviceListError::Serial(e.to_string()))?;
    Ok(ports.iter().map(map_serial_port).collect())
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
    fn video_device_mapping_preserves_id_and_display_name() {
        let camera = ipkvm_video::camera::CameraDeviceInfo {
            id: "cam-0".into(),
            display_name: "OBS Virtual Camera".into(),
        };

        assert_eq!(
            map_camera(&camera),
            VideoDevice {
                id: "cam-0".into(),
                display_name: "OBS Virtual Camera".into(),
            }
        );
    }

    #[cfg(feature = "serial")]
    #[test]
    fn serial_device_mapping_uses_device_manager_name() {
        let port = serialport::SerialPortInfo {
            port_name: "COM9".into(),
            port_type: serialport::SerialPortType::Unknown,
        };

        assert_eq!(
            map_serial_port(&port),
            SerialDevice {
                path: "COM9".into(),
                display_name: "COM9".into(),
            }
        );
    }

    #[cfg(feature = "serial")]
    #[test]
    fn serial_device_mapping_prefers_usb_product_name() {
        let port = serialport::SerialPortInfo {
            port_name: "COM9".into(),
            port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0,
                pid: 0,
                serial_number: None,
                manufacturer: Some("WCH".into()),
                product: Some("USB-SERIAL CH340".into()),
            }),
        };

        assert_eq!(map_serial_port(&port).display_name, "USB-SERIAL CH340");
    }

    #[cfg(not(feature = "serial"))]
    #[test]
    fn serial_enumeration_without_serial_feature_returns_empty_list() {
        assert!(list_serial_devices().unwrap().is_empty());
    }

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
