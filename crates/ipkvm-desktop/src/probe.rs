//! 真实设备探测 adapter；纯状态机和决策位于 `ipkvm-desktop-core`。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use ipkvm_core::{Ch9329Command, Ch9329Decoder, Ch9329Response};
use ipkvm_device::{DeviceInventoryProvider, ProductionDeviceInventoryProvider};

use ipkvm_desktop_core::probe::{ProbeBackend, ProbeError};
pub use ipkvm_desktop_core::probe::{refresh_detection, resolve_connect_baud_with};
pub use ipkvm_desktop_core::state::{ControlProbeStatus, DeviceOption, VideoProbeStatus};

pub const BAUD_CANDIDATES: [u32; 5] = [115200, 57600, 38400, 19200, 9600];
pub const BAUD_PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// 生产探测后端：设备列表来自纯 provider，CH9329 探测仍在 adapter 中打开串口。
pub struct ProductionProbeBackend;

impl ProbeBackend for ProductionProbeBackend {
    fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
        ProductionDeviceInventoryProvider
            .list_video_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|device| DeviceOption {
                        id: device.id,
                        label: device.display_name,
                    })
                    .collect()
            })
            .map_err(|error| ProbeError::VideoList(error.to_string()))
    }

    fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
        ProductionDeviceInventoryProvider
            .list_serial_devices()
            .map(|devices| {
                let mut seen = std::collections::HashMap::<String, usize>::new();
                devices
                    .into_iter()
                    .map(|device| {
                        let base = if device.display_name.is_empty() {
                            device.path.clone()
                        } else {
                            device.display_name.clone()
                        };
                        let count = seen.entry(base.clone()).or_insert(0);
                        *count += 1;
                        let label = if *count > 1 {
                            format!("{base} ({})", device.path)
                        } else {
                            base
                        };
                        DeviceOption {
                            id: device.path,
                            label,
                        }
                    })
                    .collect()
            })
            .map_err(|error| ProbeError::ControlList(error.to_string()))
    }

    fn probe_control(
        &mut self,
        device_id: &str,
        baud_rate: u32,
        timeout: Duration,
    ) -> ControlProbeStatus {
        probe_ch9329(device_id, baud_rate, timeout)
    }
}

/// 串口 CH9329 探测：发 GetInfo 命令，在超时内等合法 Info 应答。
pub fn probe_ch9329(path: &str, baud_rate: u32, timeout: Duration) -> ControlProbeStatus {
    let mut port = match serialport::new(path, baud_rate)
        .timeout(Duration::from_millis(50))
        .open()
    {
        Ok(port) => port,
        Err(error) => return ControlProbeStatus::OpenFailed(error.to_string()),
    };

    let frame = match Ch9329Command::GetInfo.to_frame(0) {
        Ok(frame) => frame,
        Err(error) => return ControlProbeStatus::NotCh9329(error.to_string()),
    };
    if let Err(error) = port.write_all(frame.as_bytes()) {
        return ControlProbeStatus::OpenFailed(error.to_string());
    }

    let deadline = Instant::now() + timeout;
    let mut decoder = Ch9329Decoder::new();
    let mut buf = [0_u8; 64];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(read) => {
                for event in decoder.push(&buf[..read]) {
                    let Ok(frame) = event else {
                        continue;
                    };
                    match Ch9329Response::parse(&frame) {
                        Ok(Ch9329Response::Info(info)) => {
                            return ControlProbeStatus::Ready(
                                ipkvm_desktop_core::state::ControlInfo {
                                    version: info.version,
                                    usb_enumerated: info.usb_enumerated,
                                    baud: baud_rate,
                                },
                            );
                        }
                        Ok(_) => {
                            return ControlProbeStatus::NotCh9329(
                                "unexpected acknowledgement".into(),
                            );
                        }
                        Err(error) => {
                            return ControlProbeStatus::NotCh9329(error.to_string());
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return ControlProbeStatus::OpenFailed(error.to_string()),
        }
    }
    ControlProbeStatus::NoResponse
}

pub fn detect_baud_rate(path: &str, timeout: Duration) -> Option<u32> {
    BAUD_CANDIDATES.into_iter().find(|baud| {
        matches!(
            probe_ch9329(path, *baud, timeout),
            ControlProbeStatus::Ready(_)
        )
    })
}

pub fn resolve_connect_baud(
    auto_baud: bool,
    current_baud: u32,
    control_status: &ControlProbeStatus,
    control_id: &str,
) -> u32 {
    resolve_connect_baud_with(auto_baud, current_baud, control_status, || {
        detect_baud_rate(control_id, BAUD_PROBE_TIMEOUT)
    })
}
