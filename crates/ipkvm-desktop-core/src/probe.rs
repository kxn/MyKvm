use std::time::Duration;

use crate::state::{ControlProbeStatus, DeviceOption, DeviceSelectionState};

/// 自动扫描候选波特率（成品线电平未知：115200 可用则最快，失败逐档降级）。
pub const BAUD_CANDIDATES: [u32; 5] = [115200, 57600, 38400, 19200, 9600];
/// 波特率探测专用超时（单档）：连接前兜底检测使用，避免每档白等过久（#97）。
/// 普通设备探测（列表枚举后的单波特率复核）仍用调用方传入的超时。
pub const BAUD_PROBE_TIMEOUT: Duration = Duration::from_millis(300);

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
        let status = backend.probe_control(&device_id, baud_rate, timeout);
        state.record_control_probe(baud_rate, status);
    }
    Ok(())
}

/// 解析连接使用的波特率（#97）。生产适配层负责提供真实检测器。
pub fn resolve_connect_baud_with(
    auto_baud: bool,
    current_baud: u32,
    control_status: &ControlProbeStatus,
    detect: impl FnOnce() -> Option<u32>,
) -> u32 {
    let verified = matches!(
        control_status,
        ControlProbeStatus::Ready(info) if info.baud == current_baud
    );
    if verified {
        return current_baud;
    }
    if auto_baud {
        detect().unwrap_or(current_baud)
    } else {
        current_baud
    }
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
                baud: _baud_rate,
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
        assert_eq!(
            state.control_status,
            ControlProbeStatus::Ready(ControlInfo {
                version: 0x31,
                usb_enumerated: true,
                baud: 9_600,
            })
        );
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

    #[test]
    fn resolve_baud_skips_detection_when_current_baud_verified() {
        // 已验证：检测器不应被调用（调用即 panic）。
        let status = ControlProbeStatus::Ready(ControlInfo {
            version: 0x31,
            usb_enumerated: true,
            baud: 115_200,
        });
        assert_eq!(
            resolve_connect_baud_with(true, 115_200, &status, || {
                panic!("已验证时必须跳过检测");
            }),
            115_200
        );
    }

    #[test]
    fn resolve_baud_detects_when_unverified_and_auto_baud_on() {
        let status = ControlProbeStatus::NoResponse;
        assert_eq!(
            resolve_connect_baud_with(true, 115_200, &status, || Some(9_600)),
            9_600
        );
        // 检测失败时保持当前值。
        assert_eq!(
            resolve_connect_baud_with(true, 115_200, &status, || None),
            115_200
        );
    }

    #[test]
    fn resolve_baud_keeps_current_when_auto_baud_off_or_baud_changed() {
        let ready_at_old = ControlProbeStatus::Ready(ControlInfo {
            version: 0x31,
            usb_enumerated: true,
            baud: 9_600,
        });
        // 改波特率后旧验证不再匹配当前值 → auto 开启时走检测。
        assert_eq!(
            resolve_connect_baud_with(true, 115_200, &ready_at_old, || Some(9_600)),
            9_600
        );
        // auto 关闭时不做检测。
        assert_eq!(
            resolve_connect_baud_with(false, 115_200, &ready_at_old, || {
                panic!("auto_baud 关闭时不得检测");
            }),
            115_200
        );
    }

    #[test]
    fn baud_probe_timeout_is_dedicated_and_short() {
        assert_eq!(BAUD_PROBE_TIMEOUT, Duration::from_millis(300));
    }
}
