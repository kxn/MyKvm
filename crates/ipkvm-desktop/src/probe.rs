use std::io::{Read, Write};
use std::time::{Duration, Instant};

use ipkvm_core::{Ch9329Command, Ch9329Decoder, Ch9329Response};

use crate::state::{ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState};

/// 自动扫描候选波特率（成品线电平未知：115200 可用则最快，失败逐档降级）。
pub const BAUD_CANDIDATES: [u32; 5] = [115200, 57600, 38400, 19200, 9600];

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("video device list failed: {0}")]
    VideoList(String),
    #[error("control device list failed: {0}")]
    ControlList(String),
}

pub trait ProbeBackend {
    fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError>;
    fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError>;
    fn probe_control(
        &mut self,
        device_id: &str,
        baud_rate: u32,
        timeout: Duration,
    ) -> ControlProbeStatus;
}

/// 刷新检测：重新枚举两类设备，并重探仍选中的串口控制设备。
///
/// 任一设备列表枚举失败时返回错误且**不替换**旧列表、不重探，避免把
/// 仍存在的选中设备误标为断开。
///
/// 视频不在这里重开：实时预览源常驻持有相机（Windows 媒体设备独占，重复
/// 打开必然失败）。刷新后视频是否需要重新探测由 app 层根据当前状态决定
/// （已打开且正常 → 跳过；未打开/出错 → 先关旧源再重开），见
/// `app::preview_refresh_action`。
pub fn refresh_detection(
    state: &mut DeviceSelectionState,
    backend: &mut dyn ProbeBackend,
    baud_rate: u32,
    timeout: Duration,
) -> Result<(), ProbeError> {
    let video = backend.list_video_devices()?;
    let control = backend.list_control_devices()?;
    state.refresh_devices(video, control);

    if let Some(device_id) = state.selected_control_id.clone() {
        state.control_status = backend.probe_control(&device_id, baud_rate, timeout);
    }
    Ok(())
}

/// 生产探测后端：视频枚举与串口枚举复用 ipkvm-session 的共享设备列表。
pub struct ProductionProbeBackend;

impl ProbeBackend for ProductionProbeBackend {
    fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
        ipkvm_session::devices::list_video_devices()
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
        ipkvm_session::devices::list_serial_devices()
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
                            return ControlProbeStatus::Ready(ControlInfo {
                                version: info.version,
                                usb_enumerated: info.usb_enumerated,
                            });
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

/// 扫描候选波特率，返回第一个 GetInfo 有合法应答的档位。
pub fn detect_baud_rate(path: &str, timeout: Duration) -> Option<u32> {
    BAUD_CANDIDATES.into_iter().find(|baud| {
        matches!(
            probe_ch9329(path, *baud, timeout),
            ControlProbeStatus::Ready(_)
        )
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::state::{
        ControlInfo, ControlProbeStatus, DeviceSelectionState, PreviewInfo, VideoProbeStatus,
    };

    #[derive(Default)]
    struct FakeBackend {
        control_calls: usize,
    }

    struct FailingListBackend;

    impl ProbeBackend for FailingListBackend {
        fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
            Err(ProbeError::VideoList("boom".into()))
        }

        fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
            Ok(Vec::new())
        }

        fn probe_control(
            &mut self,
            _device_id: &str,
            _baud_rate: u32,
            _timeout: Duration,
        ) -> ControlProbeStatus {
            ControlProbeStatus::NoResponse
        }
    }

    impl ProbeBackend for FakeBackend {
        fn list_video_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
            Ok(vec![DeviceOption {
                id: "cam0".into(),
                label: "Camera 0".into(),
            }])
        }

        fn list_control_devices(&mut self) -> Result<Vec<DeviceOption>, ProbeError> {
            Ok(vec![DeviceOption {
                id: "COM9".into(),
                label: "COM9".into(),
            }])
        }

        fn probe_control(
            &mut self,
            device_id: &str,
            _baud_rate: u32,
            _timeout: Duration,
        ) -> ControlProbeStatus {
            self.control_calls += 1;
            assert_eq!(device_id, "COM9");
            ControlProbeStatus::Ready(ControlInfo {
                version: 0x31,
                usb_enumerated: true,
            })
        }
    }

    #[test]
    fn refresh_detection_rechecks_control_but_leaves_video_status_to_preview() {
        let mut backend = FakeBackend::default();
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            // 视频状态由常驻预览源驱动，刷新不得改动它，更不得再开一次相机。
            video_status: VideoProbeStatus::Ready(PreviewInfo {
                width: 1920,
                height: 1080,
                label: "Camera 0".into(),
            }),
            ..DeviceSelectionState::default()
        };

        refresh_detection(&mut state, &mut backend, 9_600, Duration::from_millis(10)).unwrap();

        assert_eq!(backend.control_calls, 1);
        assert!(matches!(state.video_status, VideoProbeStatus::Ready(_)));
        assert!(matches!(state.control_status, ControlProbeStatus::Ready(_)));
        assert!(state.can_connect());
    }

    #[test]
    fn refresh_detection_propagates_list_errors_without_replacing_state() {
        let mut backend = FailingListBackend;
        let mut state = DeviceSelectionState {
            video_devices: vec![DeviceOption {
                id: "cam0".into(),
                label: "Camera 0".into(),
            }],
            ..DeviceSelectionState::default()
        };

        let result = refresh_detection(&mut state, &mut backend, 9_600, Duration::from_millis(10));

        assert!(result.is_err());
        assert_eq!(state.video_devices.len(), 1);
    }

    #[test]
    fn baud_candidates_prefer_115200_and_fall_back_to_9600() {
        assert_eq!(BAUD_CANDIDATES[0], 115200);
        assert_eq!(BAUD_CANDIDATES[4], 9600);
        assert_eq!(BAUD_CANDIDATES.len(), 5);
    }
}
