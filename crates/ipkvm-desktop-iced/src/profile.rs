//! profile 应用与固化（M2）：把 profile 应用到连接选择、把当前选择固化为 profile。

use ipkvm_desktop::config::{ConnectionSettings, DeviceRef, Profile};

use crate::connect::{ControlProbeStatus, DeviceOption, DeviceSelectionState, VideoProbeStatus};

/// 应用 profile 后缺失设备标记。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MissingDevices {
    pub video: bool,
    pub control: bool,
}

/// 把当前选中设备固化为 DeviceRef（label 兜底用 id，复刻 egui app.rs）。
pub fn selected_device_ref(
    devices: &[DeviceOption],
    selected_id: Option<&str>,
) -> Option<DeviceRef> {
    let id = selected_id?.to_string();
    let label = devices
        .iter()
        .find(|device| device.id == id)
        .map(|device| device.label.clone())
        .unwrap_or_else(|| id.clone());
    Some(DeviceRef { id, label })
}

/// 应用 profile 到当前选择：按 id 匹配设备；匹配不到清空该侧并标记缺失。
/// 视频选中后状态置 Checking（预览由 PreviewRuntime::tick 驱动）；
/// 控制选中后状态置 Checking（调用方随后同步探测）。
pub fn apply_profile_to_selection(
    selection: &mut DeviceSelectionState,
    profile: &Profile,
) -> MissingDevices {
    let mut missing = MissingDevices::default();
    if apply_device_ref(selection, profile.video_device.clone(), true) {
        missing.video = true;
    }
    if apply_device_ref(selection, profile.control_device.clone(), false) {
        missing.control = true;
    }
    missing
}

fn apply_device_ref(
    selection: &mut DeviceSelectionState,
    device: Option<DeviceRef>,
    is_video: bool,
) -> bool {
    let Some(device) = device else {
        clear_device_selection(selection, is_video);
        return false;
    };
    let matched = if is_video {
        selection
            .video_devices
            .iter()
            .find(|candidate| candidate.id == device.id)
            .map(|candidate| candidate.id.clone())
    } else {
        selection
            .control_devices
            .iter()
            .find(|candidate| candidate.id == device.id)
            .map(|candidate| candidate.id.clone())
    };
    match matched {
        Some(id) => {
            if is_video {
                selection.selected_video_id = Some(id);
                selection.video_status = VideoProbeStatus::Checking;
            } else {
                selection.selected_control_id = Some(id);
                selection.control_status = ControlProbeStatus::Checking;
            }
            false
        }
        None => {
            clear_device_selection(selection, is_video);
            true
        }
    }
}

fn clear_device_selection(selection: &mut DeviceSelectionState, is_video: bool) {
    if is_video {
        selection.selected_video_id = None;
        selection.video_status = VideoProbeStatus::NotSelected;
    } else {
        selection.selected_control_id = None;
        selection.control_status = ControlProbeStatus::NotSelected;
    }
}

/// 把当前选择与连接参数固化为 Profile（复刻 egui do_save_profile）。
pub fn build_profile(
    name: String,
    selection: &DeviceSelectionState,
    connection: &ConnectionSettings,
) -> Profile {
    Profile {
        name,
        video_device: selected_device_ref(
            &selection.video_devices,
            selection.selected_video_id.as_deref(),
        ),
        control_device: selected_device_ref(
            &selection.control_devices,
            selection.selected_control_id.as_deref(),
        ),
        connection: connection.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::{
        ControlInfo, ControlProbeStatus, DeviceOption, DeviceSelectionState, PreviewInfo,
        VideoProbeStatus,
    };
    use ipkvm_desktop::config::{ConnectionSettings, DeviceRef, Profile};

    fn option(id: &str, label: &str) -> DeviceOption {
        DeviceOption {
            id: id.into(),
            label: label.into(),
        }
    }

    fn selection() -> DeviceSelectionState {
        DeviceSelectionState {
            video_devices: vec![option("cam0", "Camera 0")],
            control_devices: vec![option("COM9", "CH9329 (COM9)")],
            selected_video_id: Some("cam0".into()),
            selected_control_id: Some("COM9".into()),
            video_status: VideoProbeStatus::Ready(PreviewInfo {
                width: 640,
                height: 480,
                label: "Camera 0".into(),
            }),
            control_status: ControlProbeStatus::Ready(ControlInfo {
                version: 0x31,
                usb_enumerated: true,
                baud: 115200,
            }),
        }
    }

    fn profile() -> Profile {
        Profile {
            name: "办公室".into(),
            video_device: Some(DeviceRef {
                id: "cam0".into(),
                label: "Camera 0".into(),
            }),
            control_device: Some(DeviceRef {
                id: "COM9".into(),
                label: "CH9329 (COM9)".into(),
            }),
            connection: ConnectionSettings::default(),
        }
    }

    #[test]
    fn apply_profile_selects_matching_devices() {
        let mut state = DeviceSelectionState {
            video_devices: vec![option("cam0", "Camera 0")],
            control_devices: vec![option("COM9", "CH9329 (COM9)")],
            ..DeviceSelectionState::default()
        };
        let missing = apply_profile_to_selection(&mut state, &profile());
        assert_eq!(missing, MissingDevices::default());
        assert_eq!(state.selected_video_id.as_deref(), Some("cam0"));
        assert_eq!(state.selected_control_id.as_deref(), Some("COM9"));
        assert_eq!(state.video_status, VideoProbeStatus::Checking);
        assert_eq!(state.control_status, ControlProbeStatus::Checking);
    }

    #[test]
    fn apply_profile_clears_missing_devices_and_reports() {
        let mut state = DeviceSelectionState {
            video_devices: vec![option("other", "Other")],
            control_devices: vec![option("COM9", "CH9329 (COM9)")],
            selected_video_id: Some("other".into()),
            selected_control_id: Some("COM9".into()),
            ..DeviceSelectionState::default()
        };
        let missing = apply_profile_to_selection(&mut state, &profile());
        assert!(missing.video && !missing.control);
        assert_eq!(state.selected_video_id, None);
        assert_eq!(state.video_status, VideoProbeStatus::NotSelected);
        assert_eq!(state.selected_control_id.as_deref(), Some("COM9"));
    }

    #[test]
    fn selected_device_ref_falls_back_to_id_when_missing() {
        let devices = vec![option("cam0", "Camera 0")];
        assert_eq!(
            selected_device_ref(&devices, Some("cam0")),
            Some(DeviceRef {
                id: "cam0".into(),
                label: "Camera 0".into()
            })
        );
        assert_eq!(
            selected_device_ref(&devices, Some("gone")),
            Some(DeviceRef {
                id: "gone".into(),
                label: "gone".into()
            })
        );
        assert_eq!(selected_device_ref(&devices, None), None);
    }

    #[test]
    fn build_profile_captures_selection_and_connection() {
        let state = selection();
        let profile = build_profile("办公室".into(), &state, &ConnectionSettings::default());
        assert_eq!(profile.name, "办公室");
        assert_eq!(
            profile.video_device.as_ref().map(|d| d.id.as_str()),
            Some("cam0")
        );
        assert_eq!(
            profile.control_device.as_ref().map(|d| d.label.as_str()),
            Some("CH9329 (COM9)")
        );
    }
}
