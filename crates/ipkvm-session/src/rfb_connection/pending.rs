use ipkvm_rfb::{FramebufferUpdateRequest, RfbRectangle, RfbSize};

#[derive(Default)]
pub(super) struct PendingFramebufferRequest {
    request: Option<FramebufferUpdateRequest>,
}

impl PendingFramebufferRequest {
    pub(super) fn merge(&mut self, incoming: FramebufferUpdateRequest, size: RfbSize) {
        let incoming = normalize(incoming, size);
        self.request = Some(match self.request.take() {
            None => incoming,
            Some(current) => FramebufferUpdateRequest {
                incremental: current.incremental && incoming.incremental,
                rectangle: union(current.rectangle, incoming.rectangle, size),
            },
        });
    }

    pub(super) fn get(&self) -> Option<FramebufferUpdateRequest> {
        self.request
    }

    pub(super) fn take(&mut self) -> Option<FramebufferUpdateRequest> {
        self.request.take()
    }
}

fn normalize(request: FramebufferUpdateRequest, size: RfbSize) -> FramebufferUpdateRequest {
    FramebufferUpdateRequest {
        incremental: request.incremental,
        rectangle: request
            .rectangle
            .intersection(size)
            .unwrap_or(RfbRectangle {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            }),
    }
}

fn union(left: RfbRectangle, right: RfbRectangle, size: RfbSize) -> RfbRectangle {
    if left.width == 0 || left.height == 0 {
        return right;
    }
    if right.width == 0 || right.height == 0 {
        return left;
    }

    let x = u32::from(left.x.min(right.x));
    let y = u32::from(left.y.min(right.y));
    let right_edge = (u32::from(left.x) + u32::from(left.width))
        .max(u32::from(right.x) + u32::from(right.width))
        .min(u32::from(size.width()));
    let bottom_edge = (u32::from(left.y) + u32::from(left.height))
        .max(u32::from(right.y) + u32::from(right.height))
        .min(u32::from(size.height()));

    RfbRectangle {
        x: x as u16,
        y: y as u16,
        width: (right_edge - x) as u16,
        height: (bottom_edge - y) as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> FramebufferUpdateRequest {
        FramebufferUpdateRequest {
            incremental,
            rectangle: RfbRectangle {
                x,
                y,
                width,
                height,
            },
        }
    }

    #[test]
    fn incremental_requests_merge_into_one_bounding_rectangle() {
        let size = RfbSize::new(100, 80).unwrap();
        let mut pending = PendingFramebufferRequest::default();
        pending.merge(request(true, 10, 20, 20, 10), size);
        pending.merge(request(true, 25, 5, 30, 25), size);

        assert_eq!(pending.take(), Some(request(true, 10, 5, 45, 25)));
    }

    #[test]
    fn non_incremental_request_upgrades_pending_union() {
        let size = RfbSize::new(100, 80).unwrap();
        let mut pending = PendingFramebufferRequest::default();
        pending.merge(request(true, 10, 10, 10, 10), size);
        pending.merge(request(false, 30, 20, 10, 10), size);

        assert_eq!(pending.take(), Some(request(false, 10, 10, 30, 20)));
    }

    #[test]
    fn merge_clips_without_u16_wraparound() {
        let size = RfbSize::new(u16::MAX, u16::MAX).unwrap();
        let mut pending = PendingFramebufferRequest::default();
        pending.merge(request(true, u16::MAX - 2, u16::MAX - 2, 20, 20), size);

        assert_eq!(
            pending.take(),
            Some(request(true, u16::MAX - 2, u16::MAX - 2, 2, 2))
        );
    }

    #[test]
    fn requests_outside_the_frame_stay_empty() {
        let size = RfbSize::new(100, 80).unwrap();
        let mut pending = PendingFramebufferRequest::default();
        pending.merge(request(true, 100, 80, 10, 10), size);
        pending.merge(request(true, u16::MAX, u16::MAX, 1, 1), size);

        assert_eq!(pending.take(), Some(request(true, 0, 0, 0, 0)));
        assert_eq!(pending.take(), None);
    }
}
