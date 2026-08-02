use std::io::{Read, Write};
use std::time::{Duration, Instant};

use ipkvm_core::{Ch9329Command, Ch9329Decoder, Ch9329Response};
use ipkvm_video::FrameSource;

use crate::state::{
    ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState, PreviewInfo,
    VideoProbeStatus,
};

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
    fn preview_video(&mut self, device_id: &str, fps: u64, timeout: Duration) -> VideoProbeStatus;
    fn probe_control(
        &mut self,
        device_id: &str,
        baud_rate: u32,
        timeout: Duration,
    ) -> ControlProbeStatus;
}

/// 刷新检测：重新枚举两类设备，并对仍选中的设备重探视频预览与 CH9329。
///
/// 任一设备列表枚举失败时返回错误且**不替换**旧列表、不重探，避免把
/// 仍存在的选中设备误标为断开。
pub fn refresh_detection(
    state: &mut DeviceSelectionState,
    backend: &mut impl ProbeBackend,
    timeout: Duration,
) -> Result<(), ProbeError> {
    let video = backend.list_video_devices()?;
    let control = backend.list_control_devices()?;
    state.refresh_devices(video, control);

    if let Some(device_id) = state.selected_video_id.clone() {
        state.video_status = backend.preview_video(&device_id, state.advanced.preview_fps, timeout);
    }
    if let Some(device_id) = state.selected_control_id.clone() {
        state.control_status = backend.probe_control(&device_id, state.advanced.baud_rate, timeout);
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
                devices
                    .into_iter()
                    .map(|device| DeviceOption {
                        id: device.path.clone(),
                        label: format!("{} ({})", device.path, device.port_type),
                    })
                    .collect()
            })
            .map_err(|error| ProbeError::ControlList(error.to_string()))
    }

    fn preview_video(&mut self, device_id: &str, fps: u64, timeout: Duration) -> VideoProbeStatus {
        capture_preview(device_id, fps, timeout)
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

/// 视频预览：打开采集源等一帧；返回前 drop 源，避免预览句柄占住设备。
pub fn capture_preview(device_id: &str, fps: u64, timeout: Duration) -> VideoProbeStatus {
    let source = match ipkvm_video::camera::CameraSource::open(device_id, fps) {
        Ok(source) => source,
        Err(error) => return VideoProbeStatus::OpenFailed(error.to_string()),
    };
    let label = source.source_info().device_name;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(frame) = source.latest_frame() {
            return VideoProbeStatus::Ready(PreviewInfo {
                width: frame.width,
                height: frame.height,
                label,
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    VideoProbeStatus::NoSignal
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
        video_calls: usize,
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

        fn preview_video(
            &mut self,
            _device_id: &str,
            _fps: u64,
            _timeout: Duration,
        ) -> VideoProbeStatus {
            VideoProbeStatus::NoSignal
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

        fn preview_video(
            &mut self,
            device_id: &str,
            _fps: u64,
            _timeout: Duration,
        ) -> VideoProbeStatus {
            self.video_calls += 1;
            assert_eq!(device_id, "cam0");
            VideoProbeStatus::Ready(PreviewInfo {
                width: 1920,
                height: 1080,
                label: "Camera 0".into(),
            })
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
    fn refresh_detection_lists_devices_and_rechecks_selected_devices() {
        let mut backend = FakeBackend::default();
        let mut state = DeviceSelectionState {
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            ..DeviceSelectionState::default()
        };

        refresh_detection(&mut state, &mut backend, Duration::from_millis(10)).unwrap();

        assert_eq!(backend.video_calls, 1);
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

        let result = refresh_detection(&mut state, &mut backend, Duration::from_millis(10));

        assert!(result.is_err());
        assert_eq!(state.video_devices.len(), 1);
    }
}
