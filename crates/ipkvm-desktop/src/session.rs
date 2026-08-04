//! 真实相机与 CH9329 串口的 production adapter。

use std::sync::Arc;

use ipkvm_core::{Ch9329InputSink, SerialCommandQueue};
use ipkvm_video::FrameSource;

pub use ipkvm_desktop_core::session::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, SessionParts,
};

/// 生产组件工厂：相机帧源 + CH9329 串口 sink + 新连接闸门。
pub fn production_parts(
    request: &ConnectRequest,
) -> Result<SessionParts<Ch9329InputSink<SerialCommandQueue>>, DesktopSessionError> {
    let frame_source: Arc<dyn FrameSource> = Arc::new(
        ipkvm_video::camera::CameraSource::open(&request.video_device_id, request.preview_fps)
            .map_err(|error| DesktopSessionError::Build(error.to_string()))?,
    );
    let queue = SerialCommandQueue::open(&request.control_device_id, request.baud_rate)
        .map_err(|error| DesktopSessionError::Build(error.to_string()))?;
    let sink = Ch9329InputSink::new(queue, 0, request.resolved_mouse_mode());
    Ok((
        frame_source,
        sink,
        ipkvm_session::rfb_connection::RfbConnectionGate::new(),
    ))
}

pub type ProductionSessionFactory =
    fn(
        &ConnectRequest,
    ) -> Result<SessionParts<Ch9329InputSink<SerialCommandQueue>>, DesktopSessionError>;

/// 生产控制器的构造入口。实际控制器类型仍由无硬件 core 提供。
pub struct ProductionDesktopSessionController;

impl ProductionDesktopSessionController {
    pub fn production()
    -> DesktopSessionController<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory> {
        DesktopSessionController::with_factory(production_parts)
    }
}
