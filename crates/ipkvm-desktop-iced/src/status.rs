//! 状态栏状态机（M1 骨架）：连接/在线/控制离线三态 + 文案。

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    ControlOffline(String),
}

pub fn derive_status(
    work_view: bool,
    control_online: bool,
    offline_reason: Option<String>,
) -> ConnectionStatus {
    if !work_view {
        return ConnectionStatus::Disconnected;
    }
    match (control_online, offline_reason) {
        (_, Some(reason)) => ConnectionStatus::ControlOffline(reason),
        (true, None) => ConnectionStatus::Connected,
        (false, None) => ConnectionStatus::Connecting,
    }
}

impl ConnectionStatus {
    pub fn label(&self, zh: bool) -> String {
        match (self, zh) {
            (Self::Disconnected, true) => "未连接".into(),
            (Self::Disconnected, false) => "Disconnected".into(),
            (Self::Connecting, true) => "连接中".into(),
            (Self::Connecting, false) => "Connecting".into(),
            (Self::Connected, true) => "已连接".into(),
            (Self::Connected, false) => "Connected".into(),
            (Self::ControlOffline(reason), true) => format!("控制设备离线：{reason}"),
            (Self::ControlOffline(reason), false) => format!("Control offline: {reason}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_when_not_connected() {
        assert_eq!(
            derive_status(false, false, Some("serial reset".into())),
            ConnectionStatus::Disconnected
        );
    }

    #[test]
    fn connected_when_online_without_reason() {
        assert_eq!(derive_status(true, true, None), ConnectionStatus::Connected);
    }

    #[test]
    fn control_offline_carries_reason() {
        assert_eq!(
            derive_status(true, false, Some("serial write failed".into())),
            ConnectionStatus::ControlOffline("serial write failed".into())
        );
    }

    #[test]
    fn work_view_without_ready_control_is_connecting() {
        assert_eq!(
            derive_status(true, false, None),
            ConnectionStatus::Connecting
        );
    }

    #[test]
    fn labels_are_nonempty_and_distinct() {
        for zh in [true, false] {
            for s in [
                ConnectionStatus::Disconnected,
                ConnectionStatus::Connecting,
                ConnectionStatus::Connected,
                ConnectionStatus::ControlOffline("x".into()),
            ] {
                assert!(!s.label(zh).is_empty());
            }
        }
        assert_ne!(
            ConnectionStatus::Connected.label(true),
            ConnectionStatus::Connected.label(false)
        );
    }
}
