#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewInfo {
    pub width: u32,
    pub height: u32,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlInfo {
    pub version: u8,
    pub usb_enumerated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VideoProbeStatus {
    NotSelected,
    Checking,
    Ready(PreviewInfo),
    NoSignal,
    OpenFailed(String),
    Disconnected,
}

impl Default for VideoProbeStatus {
    fn default() -> Self {
        Self::NotSelected
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlProbeStatus {
    NotSelected,
    Checking,
    Ready(ControlInfo),
    NotCh9329(String),
    NoResponse,
    OpenFailed(String),
    Disconnected,
}

impl Default for ControlProbeStatus {
    fn default() -> Self {
        Self::NotSelected
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoScaleMode {
    FitWindow,
    ActualSize,
    ResizeWindowToVideo,
}

impl Default for VideoScaleMode {
    fn default() -> Self {
        Self::FitWindow
    }
}

#[derive(Clone, Debug, Default)]
pub struct DeviceSelectionState {
    pub video_devices: Vec<DeviceOption>,
    pub control_devices: Vec<DeviceOption>,
    pub selected_video_id: Option<String>,
    pub selected_control_id: Option<String>,
    pub video_status: VideoProbeStatus,
    pub control_status: ControlProbeStatus,
}

impl DeviceSelectionState {
    pub fn can_connect(&self) -> bool {
        matches!(self.video_status, VideoProbeStatus::Ready(_))
            && matches!(self.control_status, ControlProbeStatus::Ready(_))
    }

    pub fn refresh_devices(&mut self, video: Vec<DeviceOption>, control: Vec<DeviceOption>) {
        let selected_video_missing = self
            .selected_video_id
            .as_ref()
            .is_some_and(|id| !video.iter().any(|device| device.id == *id));
        let selected_control_missing = self
            .selected_control_id
            .as_ref()
            .is_some_and(|id| !control.iter().any(|device| device.id == *id));

        self.video_devices = video;
        self.control_devices = control;

        if selected_video_missing {
            self.video_status = VideoProbeStatus::Disconnected;
        }
        if selected_control_missing {
            self.control_status = ControlProbeStatus::Disconnected;
        }
    }

    /// 正式会话期间控制设备离线（输入泵退出/事件发送失败）：标记为断开，
    /// 状态栏显示离线；用户刷新检测后可重新探测并连接。
    pub fn mark_control_offline(&mut self) {
        self.control_status = ControlProbeStatus::Disconnected;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, label: &str) -> DeviceOption {
        DeviceOption {
            id: id.to_owned(),
            label: label.to_owned(),
        }
    }

    #[test]
    fn connect_requires_video_ready_and_control_ready() {
        let mut state = DeviceSelectionState {
            video_status: VideoProbeStatus::Ready(PreviewInfo {
                width: 1920,
                height: 1080,
                label: "capture".into(),
            }),
            control_status: ControlProbeStatus::NoResponse,
            ..DeviceSelectionState::default()
        };
        assert!(!state.can_connect());

        state.control_status = ControlProbeStatus::Ready(ControlInfo {
            version: 0x31,
            usb_enumerated: true,
        });
        assert!(state.can_connect());
    }

    #[test]
    fn refresh_marks_missing_selected_devices_disconnected() {
        let mut state = DeviceSelectionState {
            video_devices: vec![option("cam0", "Camera 0")],
            control_devices: vec![option("COM9", "COM9")],
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
            }),
        };

        state.refresh_devices(Vec::new(), Vec::new());

        assert_eq!(state.video_status, VideoProbeStatus::Disconnected);
        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
        assert!(!state.can_connect());
    }

    #[test]
    fn mark_control_offline_sets_disconnected_status() {
        let mut state = DeviceSelectionState {
            control_status: ControlProbeStatus::Ready(ControlInfo {
                version: 0x31,
                usb_enumerated: true,
            }),
            ..DeviceSelectionState::default()
        };

        state.mark_control_offline();

        assert_eq!(state.control_status, ControlProbeStatus::Disconnected);
    }
}
