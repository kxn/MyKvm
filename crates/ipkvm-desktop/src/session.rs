//! 真实相机与 CH9329 串口的 production adapter。

use std::sync::Arc;

use ipkvm_core::{Ch9329InputSink, SerialCommandQueue};
use ipkvm_video::FrameSource;

pub use ipkvm_desktop_core::session::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, DesktopSessionFactory,
    SessionParts,
};

/// 生产组件工厂：相机帧源与 CH9329 串口 sink 分层构造，供共享 supervisor
/// 独立恢复视频与控制链路。
#[derive(Clone, Copy, Debug, Default)]
pub struct ProductionSessionFactory;

impl DesktopSessionFactory<Ch9329InputSink<SerialCommandQueue>> for ProductionSessionFactory {
    fn build(
        &mut self,
        request: &ConnectRequest,
    ) -> Result<SessionParts<Ch9329InputSink<SerialCommandQueue>>, DesktopSessionError> {
        Ok((
            self.build_video(request)?,
            self.build_control(request)?,
            self.build_gate(request)?,
        ))
    }

    fn build_video(
        &mut self,
        request: &ConnectRequest,
    ) -> Result<Arc<dyn FrameSource>, DesktopSessionError> {
        Ok(Arc::new(
            ipkvm_video::camera::CameraSource::open(&request.video_device_id, request.preview_fps)
                .map_err(|error| DesktopSessionError::Build(error.to_string()))?,
        ))
    }

    fn build_control(
        &mut self,
        request: &ConnectRequest,
    ) -> Result<Ch9329InputSink<SerialCommandQueue>, DesktopSessionError> {
        let queue = SerialCommandQueue::open(&request.control_device_id, request.baud_rate)
            .map_err(|error| DesktopSessionError::Build(error.to_string()))?;
        Ok(Ch9329InputSink::new(
            queue,
            0,
            request.resolved_mouse_mode(),
        ))
    }
}

/// 生产控制器的构造入口。实际控制器类型仍由无硬件 core 提供。
pub struct ProductionDesktopSessionController;

impl ProductionDesktopSessionController {
    pub fn production()
    -> DesktopSessionController<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory> {
        DesktopSessionController::with_factory(ProductionSessionFactory)
    }
}
