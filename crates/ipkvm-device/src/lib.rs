use std::cmp::Ordering;

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
            .map(|ports| {
                serial_devices_from_ports(ports)
                    .into_iter()
                    .filter(|device| serial_path_usable(&device.path))
                    .collect()
            })
            .map_err(|error| DeviceProviderError::Serial(error.to_string()))
    }
}

/// 把 serialport 枚举结果映射为设备清单：过滤不可用类型并按路径自然排序，
/// 让清单与 sysfs/注册表枚举顺序解耦，第一项即可直接作为默认选择。 #93
#[cfg(feature = "platform")]
fn serial_devices_from_ports(ports: Vec<serialport::SerialPortInfo>) -> Vec<SerialDevice> {
    let mut devices: Vec<SerialDevice> = ports
        .into_iter()
        .filter(serial_port_type_is_candidate)
        .map(map_serial_device)
        .collect();
    devices.sort_by(|left, right| compare_serial_paths(&left.path, &right.path));
    devices
}

/// 串口类型候选判定。 #93：unix 上只保留真实 USB 串口适配器（CH340/CP210x/
/// FTDI/CDC-ACM，即 CH9329 的实际接法，serialport 报 UsbPort）；板载 8250
/// UART（ttyS*，报 Unknown）通常未接线且多为 root:dialout，过滤掉。
/// 非 unix（Windows COM 口在 serialport 里多报 Unknown）无此噪音，全量保留。
#[cfg(all(feature = "platform", unix))]
fn serial_port_type_is_candidate(info: &serialport::SerialPortInfo) -> bool {
    matches!(info.port_type, serialport::SerialPortType::UsbPort(_))
}

#[cfg(all(feature = "platform", not(unix)))]
fn serial_port_type_is_candidate(_info: &serialport::SerialPortInfo) -> bool {
    true
}

/// 路径可读写判定：当前进程用户打不开的端口不进清单（如 root:dialout 660
/// 且服务用户不在 dialout 组）。access(2) 不实际打开设备，无副作用。 #93
#[cfg(all(feature = "platform", unix))]
fn serial_path_usable(path: &str) -> bool {
    std::ffi::CString::new(path)
        .map(|path| unsafe { libc::access(path.as_ptr(), libc::R_OK | libc::W_OK) == 0 })
        .unwrap_or(false)
}

#[cfg(all(feature = "platform", not(unix)))]
fn serial_path_usable(_path: &str) -> bool {
    true
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

/// 设备路径自然排序：连续 ASCII 数字段按数值比较，其余按字典序，
/// 保证 ttyUSB2 < ttyUSB10、COM9 < COM10，清单顺序与枚举顺序解耦。 #93
fn compare_serial_paths(left: &str, right: &str) -> Ordering {
    let (mut left, mut right) = (left, right);
    loop {
        match (left.chars().next(), right.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let (left_digits, left_rest) = split_ascii_digits(left);
                    let (right_digits, right_rest) = split_ascii_digits(right);
                    let left_value = left_digits.trim_start_matches('0');
                    let right_value = right_digits.trim_start_matches('0');
                    // 数值比较：先比有效位数再比字典序（等长纯数字串字典序即数值序）。
                    match left_value
                        .len()
                        .cmp(&right_value.len())
                        .then_with(|| left_value.cmp(right_value))
                    {
                        Ordering::Equal => {
                            left = left_rest;
                            right = right_rest;
                        }
                        order => return order,
                    }
                } else {
                    match left_char.cmp(&right_char) {
                        Ordering::Equal => {
                            left = &left[left_char.len_utf8()..];
                            right = &right[right_char.len_utf8()..];
                        }
                        order => return order,
                    }
                }
            }
        }
    }
}

/// 切出路径开头的连续 ASCII 数字段与其余部分。
fn split_ascii_digits(path: &str) -> (&str, &str) {
    let end = path
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(path.len());
    path.split_at(end)
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

    #[test]
    fn serial_paths_sort_naturally_with_numeric_suffixes() {
        let mut usb = vec!["/dev/ttyUSB10", "/dev/ttyUSB2", "/dev/ttyUSB0"];
        usb.sort_by(|left, right| compare_serial_paths(left, right));
        assert_eq!(
            usb,
            ["/dev/ttyUSB0", "/dev/ttyUSB2", "/dev/ttyUSB10"],
            "数字后缀应按数值排序，而非字典序"
        );

        let mut com = vec!["COM10", "COM9", "COM2"];
        com.sort_by(|left, right| compare_serial_paths(left, right));
        assert_eq!(com, ["COM2", "COM9", "COM10"]);

        assert_eq!(
            compare_serial_paths("/dev/ttyUSB0", "/dev/ttyUSB0"),
            Ordering::Equal
        );
        assert_eq!(
            compare_serial_paths("/dev/ttyUSB", "/dev/ttyUSB0"),
            Ordering::Less,
            "前缀相同者更短者在前"
        );
        assert_eq!(
            compare_serial_paths("/dev/ttyACM0", "/dev/ttyUSB0"),
            Ordering::Less,
            "非数字段按字典序"
        );
    }

    #[cfg(feature = "platform")]
    mod serial_inventory {
        use super::*;

        fn usb_port(product: &str) -> serialport::SerialPortType {
            serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x1a86,
                pid: 0x7523,
                serial_number: None,
                manufacturer: Some("QinHeng Electronics".to_owned()),
                product: Some(product.to_owned()),
            })
        }

        fn port(name: &str, port_type: serialport::SerialPortType) -> serialport::SerialPortInfo {
            serialport::SerialPortInfo {
                port_name: name.to_owned(),
                port_type,
            }
        }

        #[cfg(unix)]
        #[test]
        fn usb_adapters_are_kept_sorted_and_legacy_uarts_dropped() {
            let ports = vec![
                port("/dev/ttyS2", serialport::SerialPortType::Unknown),
                port("/dev/ttyUSB10", usb_port("USB Serial 10")),
                port("/dev/rfcomm0", serialport::SerialPortType::BluetoothPort),
                port("/dev/ttyUSB2", usb_port("USB Serial 2")),
                port("/dev/ttyUSB0", usb_port("USB Serial")),
            ];
            let devices = serial_devices_from_ports(ports);
            let paths: Vec<&str> = devices.iter().map(|d| d.path.as_str()).collect();
            assert_eq!(paths, ["/dev/ttyUSB0", "/dev/ttyUSB2", "/dev/ttyUSB10"]);
            assert_eq!(devices[0].display_name, "USB Serial");
        }

        #[cfg(not(unix))]
        #[test]
        fn com_ports_are_all_kept_and_sorted_naturally() {
            let ports = vec![
                port("COM10", serialport::SerialPortType::Unknown),
                port("COM9", serialport::SerialPortType::Unknown),
                port("COM2", serialport::SerialPortType::Unknown),
            ];
            let devices = serial_devices_from_ports(ports);
            let paths: Vec<&str> = devices.iter().map(|d| d.path.as_str()).collect();
            assert_eq!(paths, ["COM2", "COM9", "COM10"]);
        }

        #[cfg(all(unix, feature = "platform"))]
        #[test]
        fn serial_path_usable_reflects_file_permissions() {
            if unsafe { libc::geteuid() } == 0 {
                return; // root 对任何权限位都通过，本用例无意义
            }
            let path =
                std::env::temp_dir().join(format!("ipkvm-device-access-{}", std::process::id()));
            std::fs::write(&path, b"").unwrap();
            let path = path.to_str().unwrap();
            assert!(serial_path_usable(path), "属主文件默认权限应可读写");

            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
            assert!(!serial_path_usable(path), "chmod 000 后应判定不可用");

            let _ = std::fs::remove_file(path);
        }
    }
}
