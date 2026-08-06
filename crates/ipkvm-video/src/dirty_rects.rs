//! Tile-based dirty rectangle 检测：对比相邻 BGRA 帧，找出变化的矩形区域。
//!
//! 用于 RFB incremental update 的差分编码（调研阶段 2.1，issue #21）。
//! 把帧按 tile_size 分成网格，逐 tile memcmp 对比，连通脏 tile 合并成矩形。
//! MJPEG 帧不做检测（有损压缩帧的微小差别会导致全屏误报）。

use crate::{Rect, VideoFrame};

/// Tile-based dirty rectangle 检测器。持有一帧 BGRA 缓存用于对比。
pub struct DirtyRectDetector {
    tile_size: u32,
    prev_frame: Option<Vec<u8>>,
    prev_width: u32,
    prev_height: u32,
}

impl DirtyRectDetector {
    pub fn new(tile_size: u32) -> Self {
        Self {
            tile_size: tile_size.max(1),
            prev_frame: None,
            prev_width: 0,
            prev_height: 0,
        }
    }

    /// 对比当前帧与上一帧，返回变化的矩形列表。
    /// - 首帧（无缓存）→ 返回单个全帧矩形。
    /// - 尺寸变化 → 返回单个全帧矩形（重置缓存）。
    /// - 完全相同 → 返回空 Vec。
    /// - 部分变化 → 返回合并后的脏矩形列表。
    /// 调用后更新内部缓存为当前帧。
    pub fn detect(&mut self, frame: &VideoFrame) -> Vec<Rect> {
        // MJPEG 帧不做检测。
        if frame.pixel_format != crate::PixelFormat::Bgra8888 {
            self.update_cache(frame);
            return vec![full_rect(frame)];
        }
        // 尺寸变化 → 全帧。
        if frame.width != self.prev_width || frame.height != self.prev_height {
            self.update_cache(frame);
            return vec![full_rect(frame)];
        }
        // 首帧 → 全帧。
        let Some(prev) = &self.prev_frame else {
            self.update_cache(frame);
            return vec![full_rect(frame)];
        };
        let cur = &*frame.data;
        let w = frame.width as usize;
        let h = frame.height as usize;
        let ts = self.tile_size as usize;
        let cols = w.div_ceil(ts);
        let rows = h.div_ceil(ts);
        // 逐 tile memcmp，标记脏 tile 网格。
        let mut dirty_grid = vec![false; cols * rows];
        let mut any_dirty = false;
        for ty in 0..rows {
            for tx in 0..cols {
                let x0 = tx * ts;
                let y0 = ty * ts;
                let tw = (x0 + ts).min(w) - x0;
                let th = (y0 + ts).min(h) - y0;
                // 对比该 tile 区域的每一行（帧可能有 stride > width*4，但我们的帧 stride = width*4）。
                let stride = frame.stride as usize;
                let mut changed = false;
                for row in 0..th {
                    let cur_off = (y0 + row) * stride + x0 * 4;
                    let prev_off = (y0 + row) * w * 4 + x0 * 4;
                    let cur_slice = &cur[cur_off..cur_off + tw * 4];
                    let prev_slice = &prev[prev_off..prev_off + tw * 4];
                    if cur_slice != prev_slice {
                        changed = true;
                        break;
                    }
                }
                if changed {
                    dirty_grid[ty * cols + tx] = true;
                    any_dirty = true;
                }
            }
        }
        self.update_cache(frame);
        if !any_dirty {
            return Vec::new();
        }
        // 合并连通脏 tile 成矩形（行扫描 + 纵向合并）。
        merge_dirty_tiles(&dirty_grid, cols, rows, ts, w as u32, h as u32)
    }

    fn update_cache(&mut self, frame: &VideoFrame) {
        self.prev_frame = Some(frame.data.to_vec());
        self.prev_width = frame.width;
        self.prev_height = frame.height;
    }
}

fn full_rect(frame: &VideoFrame) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: frame.width,
        height: frame.height,
    }
}

/// 把脏 tile 网格合并成矩形列表。
/// 算法：逐行找连续脏 tile 段（horizontal runs），相邻行的重叠段纵向合并。
fn merge_dirty_tiles(
    grid: &[bool],
    cols: usize,
    rows: usize,
    tile_size: usize,
    frame_w: u32,
    frame_h: u32,
) -> Vec<Rect> {
    // 简单实现：把每个脏 tile 直接转成 rect，然后合并相邻的。
    // 更高效的 RLE/flood-fill 留后续优化。对 KVM 场景（少量脏 tile）足够。
    let mut rects: Vec<Rect> = Vec::new();
    for ty in 0..rows {
        for tx in 0..cols {
            if !grid[ty * cols + tx] {
                continue;
            }
            let x = (tx * tile_size) as u32;
            let y = (ty * tile_size) as u32;
            let w = ((tx + 1) * tile_size).min(frame_w as usize) as u32 - x;
            let h = ((ty + 1) * tile_size).min(frame_h as usize) as u32 - y;
            rects.push(Rect {
                x,
                y,
                width: w,
                height: h,
            });
        }
    }
    // 简单合并：反复尝试合并相邻矩形直到稳定。
    loop {
        let mut merged_any = false;
        let mut i = 0;
        while i < rects.len() {
            let mut j = i + 1;
            while j < rects.len() {
                let (a, b) = (rects[i], rects[j]);
                // 水平相邻合并
                let combined = if a.y == b.y && a.height == b.height && a.x + a.width == b.x {
                    Some(Rect {
                        x: a.x,
                        y: a.y,
                        width: a.width + b.width,
                        height: a.height,
                    })
                } else if a.y == b.y && a.height == b.height && b.x + b.width == a.x {
                    Some(Rect {
                        x: b.x,
                        y: a.y,
                        width: a.width + b.width,
                        height: a.height,
                    })
                // 垂直相邻合并
                } else if a.x == b.x && a.width == b.width && a.y + a.height == b.y {
                    Some(Rect {
                        x: a.x,
                        y: a.y,
                        width: a.width,
                        height: a.height + b.height,
                    })
                } else if a.x == b.x && a.width == b.width && b.y + b.height == a.y {
                    Some(Rect {
                        x: a.x,
                        y: b.y,
                        width: a.width,
                        height: a.height + b.height,
                    })
                } else {
                    None
                };
                if let Some(c) = combined {
                    rects[i] = c;
                    rects.remove(j);
                    merged_any = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged_any {
            break;
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MonotonicTimestamp, PixelFormat};
    use std::sync::Arc;

    fn bgra_frame(seq: u64, width: u32, height: u32, fill: u8) -> VideoFrame {
        VideoFrame {
            seq,
            timestamp: MonotonicTimestamp::from_nanos(0),
            width,
            height,
            stride: width * 4,
            pixel_format: PixelFormat::Bgra8888,
            data: Arc::from(vec![fill; (width * height * 4) as usize].into_boxed_slice()),
            dirty_rects: None,
        }
    }

    fn bgra_frame_with(seq: u64, width: u32, height: u32, pixels: &[u8]) -> VideoFrame {
        VideoFrame {
            seq,
            timestamp: MonotonicTimestamp::from_nanos(0),
            width,
            height,
            stride: width * 4,
            pixel_format: PixelFormat::Bgra8888,
            data: Arc::from(pixels.to_vec().into_boxed_slice()),
            dirty_rects: None,
        }
    }

    #[test]
    fn first_frame_returns_full_rect() {
        let mut det = DirtyRectDetector::new(32);
        let frame = bgra_frame(1, 64, 64, 0);
        let rects = det.detect(&frame);
        assert_eq!(
            rects,
            vec![Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64
            }]
        );
    }

    #[test]
    fn identical_frame_returns_empty() {
        let mut det = DirtyRectDetector::new(32);
        let frame = bgra_frame(1, 64, 64, 100);
        det.detect(&frame);
        let rects = det.detect(&frame);
        assert!(
            rects.is_empty(),
            "identical frame should have no dirty rects"
        );
    }

    #[test]
    fn changed_tile_returns_its_rect() {
        let mut det = DirtyRectDetector::new(32);
        let f1 = bgra_frame(1, 64, 64, 0);
        det.detect(&f1);
        // 改变左上 tile（0..32, 0..32）的一个像素。
        let mut pixels = vec![0u8; 64 * 64 * 4];
        pixels[0] = 255;
        let f2 = bgra_frame_with(2, 64, 64, &pixels);
        let rects = det.detect(&f2);
        assert!(
            rects
                .iter()
                .any(|r| r.x == 0 && r.y == 0 && r.width == 32 && r.height == 32),
            "changed tile should produce a 32x32 rect at origin: {rects:?}"
        );
    }

    #[test]
    fn size_change_returns_full_rect() {
        let mut det = DirtyRectDetector::new(32);
        det.detect(&bgra_frame(1, 64, 64, 0));
        let rects = det.detect(&bgra_frame(2, 32, 32, 0));
        assert_eq!(
            rects,
            vec![Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 32
            }]
        );
    }

    #[test]
    fn mjpeg_frame_returns_full_rect_without_detecting() {
        let mut det = DirtyRectDetector::new(32);
        let mjpeg = VideoFrame {
            seq: 1,
            timestamp: MonotonicTimestamp::from_nanos(0),
            width: 64,
            height: 64,
            stride: 0,
            pixel_format: PixelFormat::Mjpeg,
            data: Arc::from(vec![0u8; 10].into_boxed_slice()),
            dirty_rects: None,
        };
        let rects = det.detect(&mjpeg);
        assert_eq!(
            rects,
            vec![Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64
            }]
        );
    }
}
