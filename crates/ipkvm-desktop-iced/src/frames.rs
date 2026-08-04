//! 帧订阅：把 `DesktopSessionController::subscribe_frames` 的 watch receiver
//! 转成 iced `Subscription`，由帧通知驱动重绘。
//!
//! 用自定义 `Recipe` 持有 watch receiver，在 `stream` 里 await watch 通知，
//! 每帧到达就向 iced 投递一条 [`FrameUpdate`]，iced 的 update 因此重绘
//! （view 从 state 取最新 Handle，遵循 #3160「Handle 存 state、view 只 clone」）。

use std::hash::Hash;

use futures_util::stream::BoxStream;
use iced::Subscription;
use iced_futures::subscription::{EventStream, Hasher, Recipe, from_recipe};
use ipkvm_video::{FrameReceiver, SharedVideoFrame};

/// 帧订阅的输出：要么有帧（携带最新帧），要么帧源关闭（收尾）。
#[derive(Clone, Debug)]
pub enum FrameUpdate {
    Frame(SharedVideoFrame),
    Closed,
}

/// 把一个 watch receiver 包成 iced `Subscription`。
///
/// 每个 watch 通知映射为一条 [`FrameUpdate::Frame`]；receiver 关闭后发
/// [`FrameUpdate::Closed`]。调用方在 `Closed` 后应停止返回该 subscription。
///
/// `id` 用于唯一标识该 subscription（多路帧源时区分），固定哈希避免与其它订阅混淆。
pub fn frame_subscription(id: u64, receiver: FrameReceiver) -> Subscription<FrameUpdate> {
    from_recipe(FrameRecipe { id, receiver })
}

/// 持有 watch receiver 的 iced subscription recipe。
struct FrameRecipe {
    id: u64,
    receiver: FrameReceiver,
}

impl Recipe for FrameRecipe {
    type Output = FrameUpdate;

    fn hash(&self, state: &mut Hasher) {
        // 用固定 id 标识；receiver 本身不参与哈希（无 Hash 实现）。
        "frame_recipe".hash(state);
        self.id.hash(state);
    }

    fn stream(self: Box<Self>, _input: EventStream) -> BoxStream<'static, Self::Output> {
        let mut receiver = self.receiver;
        Box::pin(async_stream::stream! {
            loop {
                match receiver.changed().await {
                    Ok(()) => {
                        // 在独立语句 clone 出帧并立刻 drop watch guard，
                        // 避免非 Send 的 Ref 跨 await 点。
                        let frame = receiver.borrow().clone();
                        if let Some(frame) = frame {
                            yield FrameUpdate::Frame(frame);
                        }
                        // 初值 None，继续等下一帧。
                    }
                    Err(_) => {
                        yield FrameUpdate::Closed;
                        break;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_video::FrameSource;
    use ipkvm_video::mock::MockFrameSource;

    /// 帧订阅逻辑可被独立验证：mock 帧源推帧后，receiver 的 changed 能读到帧。
    /// （iced Subscription 本身需要 GUI runtime，无法在纯单测里驱动；
    /// 这里验证底层 watch 链路，即 #73 关注的「watch 通知能否被 iced 消费」。）
    #[tokio::test]
    async fn watch_receiver_yields_published_frame() {
        let mock = std::sync::Arc::new(MockFrameSource::new());
        let mut receiver = mock.subscribe();

        // 模拟帧源推送。
        let frame = std::sync::Arc::new(make_frame(7));
        mock.publish_frame(frame);

        let got = next_frame(&mut receiver).await;
        assert!(got.is_some(), "watch 必须把发布的帧传给 receiver");
        assert_eq!(got.unwrap().seq, 7);
    }

    /// 从 watch receiver 取下一帧（跳过 None 初值）。
    async fn next_frame(receiver: &mut FrameReceiver) -> Option<SharedVideoFrame> {
        loop {
            if receiver.changed().await.is_err() {
                return None;
            }
            if let Some(frame) = receiver.borrow().clone() {
                return Some(frame);
            }
        }
    }

    fn make_frame(seq: u64) -> ipkvm_video::VideoFrame {
        ipkvm_video::VideoFrame::new(
            seq,
            ipkvm_video::MonotonicTimestamp::from_nanos(seq),
            2,
            2,
            8,
            ipkvm_video::PixelFormat::Bgra8888,
            std::sync::Arc::from(vec![0u8; 16].into_boxed_slice()),
        )
    }
}
