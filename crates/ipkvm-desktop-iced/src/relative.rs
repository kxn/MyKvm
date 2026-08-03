//! 相对鼠标：平台中立 trait + 固定间隔采样（spike 3）。
//!
//! - `RelativePointerSource`：平台差异收口点。Windows 用 Raw Input 实现
//!   （`platform::windows`）；macOS/其它平台为 stub（迁移时补实现，不堵口子）。
//! - `DeltaSampler`：固定间隔采样，任意增量累计后每个周期最多发出 1 个事件，
//!   余数保留（语义与 egui desktop `input.rs` 的 `sample_delta` 一致；
//!   迁移时统一收口到共享 crate）。

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub type DeltaReceiver = Receiver<(i16, i16)>;

/// 平台差异收口：启动捕获后通过 mpsc 流式返回原始鼠标增量。
pub trait RelativePointerSource: Send {
    /// 启动捕获并返回增量事件流；重复调用返回错误。
    fn receiver(&mut self) -> Result<DeltaReceiver, String>;
    /// 停止捕获并回收线程/窗口。
    fn stop(&mut self);
}

/// 相对鼠标源工厂：生产用平台实现，测试注入 channel。
pub trait RelativeSourceFactory: Send + Sync {
    fn create(&self) -> Result<Box<dyn RelativePointerSource>, String>;
}

/// 测试用 channel 相对源：`push` 注入增量，receiver 原样读出。
pub struct ChannelRelativeSource {
    tx: std::sync::mpsc::Sender<(i16, i16)>,
    rx: std::sync::Mutex<Option<DeltaReceiver>>,
    started: std::sync::Mutex<bool>,
}

impl ChannelRelativeSource {
    pub fn new() -> (Arc<Self>, std::sync::mpsc::Sender<(i16, i16)>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            Arc::new(Self {
                tx: tx.clone(),
                rx: std::sync::Mutex::new(Some(rx)),
                started: std::sync::Mutex::new(false),
            }),
            tx,
        )
    }

    pub fn push(&self, dx: i16, dy: i16) {
        let _ = self.tx.send((dx, dy));
    }

    fn do_receiver(&self) -> Result<DeltaReceiver, String> {
        let mut started = self.started.lock().unwrap();
        if *started {
            return Err("already started".into());
        }
        *started = true;
        self.rx
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| "receiver already taken".into())
    }

    fn do_stop(&self) {
        *self.started.lock().unwrap() = false;
    }
}

impl RelativePointerSource for ChannelRelativeSource {
    fn receiver(&mut self) -> Result<DeltaReceiver, String> {
        self.do_receiver()
    }

    fn stop(&mut self) {
        self.do_stop();
    }
}

impl RelativePointerSource for Arc<ChannelRelativeSource> {
    fn receiver(&mut self) -> Result<DeltaReceiver, String> {
        self.do_receiver()
    }

    fn stop(&mut self) {
        self.do_stop();
    }
}

/// 测试用相对源工厂：create 返回 channel 源并保存句柄，`push` 注入增量。
#[derive(Default)]
pub struct ChannelRelativeFactory {
    source: std::sync::Mutex<Option<Arc<ChannelRelativeSource>>>,
}

impl ChannelRelativeFactory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, dx: i16, dy: i16) {
        if let Some(source) = &*self.source.lock().unwrap() {
            source.push(dx, dy);
        }
    }
}

impl RelativeSourceFactory for ChannelRelativeFactory {
    fn create(&self) -> Result<Box<dyn RelativePointerSource>, String> {
        let (source, _tx) = ChannelRelativeSource::new();
        *self.source.lock().unwrap() = Some(Arc::clone(&source));
        Ok(Box::new(source))
    }
}

/// 固定间隔采样：累计增量，每间隔最多发出 1 个事件（取整数部分，余数保留）。
#[derive(Debug)]
pub struct DeltaSampler {
    interval: Duration,
    remainder_x: f32,
    remainder_y: f32,
    last_send: Option<Instant>,
}

impl DeltaSampler {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            remainder_x: 0.0,
            remainder_y: 0.0,
            last_send: None,
        }
    }

    /// 复位累计余数与采样周期：退出远程输入后丢弃历史增量，下次移动立即开新周期。
    pub fn reset(&mut self) {
        self.remainder_x = 0.0;
        self.remainder_y = 0.0;
        self.last_send = None;
    }

    /// 喂入原始增量；到采样点时返回本周期应发送的增量（最多一次，非零才发）。
    pub fn feed(&mut self, dx: f32, dy: f32, now: Instant) -> Option<(i16, i16)> {
        self.remainder_x += dx;
        self.remainder_y += dy;

        let due = match self.last_send {
            None => true,
            Some(last) => now.duration_since(last) >= self.interval,
        };
        if !due {
            return None;
        }

        let ix = self
            .remainder_x
            .trunc()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let iy = self
            .remainder_y
            .trunc()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        if ix == 0 && iy == 0 {
            return None;
        }
        self.remainder_x -= f32::from(ix);
        self.remainder_y -= f32::from(iy);
        self.last_send = Some(now);
        Some((ix, iy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn first_nonzero_delta_sends_immediately() {
        // 捕获启动后的第一次移动必须立即有事件（不吞首移动）。
        let mut s = DeltaSampler::new(Duration::from_millis(33));
        assert_eq!(s.feed(1.0, 0.0, t(0)), Some((1, 0)));
        assert_eq!(s.feed(0.0, 0.0, t(1)), None);
    }

    #[test]
    fn at_most_one_event_per_interval() {
        let mut s = DeltaSampler::new(Duration::from_millis(33));
        assert_eq!(s.feed(1.0, 0.0, t(0)), Some((1, 0)));
        // 间隔内多次喂入：不发。
        assert_eq!(s.feed(1.0, 1.0, t(10)), None);
        assert_eq!(s.feed(1.0, 0.0, t(20)), None);
        // 到间隔后：只发 1 个事件。
        assert_eq!(s.feed(0.0, 0.0, t(40)), Some((2, 1)));
        assert_eq!(s.feed(0.0, 0.0, t(41)), None);
    }

    #[test]
    fn sum_preserved_one_to_one() {
        // 任意序列喂入后，发出的增量总和 = 喂入总和（1:1，仅截断余数）。
        let mut s = DeltaSampler::new(Duration::from_millis(33));
        let mut sent_x = 0i64;
        let mut sent_y = 0i64;
        let mut fed_x = 0i64;
        let mut fed_y = 0i64;
        for i in 0..200 {
            let dx = (i % 7) as f32 - 2.5;
            let dy = (i % 5) as f32 - 1.5;
            fed_x += dx as i64;
            fed_y += dy as i64;
            if let Some((ix, iy)) = s.feed(dx, dy, t(i * 20)) {
                sent_x += ix as i64;
                sent_y += iy as i64;
            }
        }
        // 余数只差不足 1（截断误差），故总和误差 < 事件数。
        assert!((fed_x - sent_x).abs() < 200, "{fed_x} vs {sent_x}");
        assert!((fed_y - sent_y).abs() < 200, "{fed_y} vs {sent_y}");
    }

    #[test]
    fn large_deltas_keep_remainder() {
        let mut s = DeltaSampler::new(Duration::from_millis(33));
        assert_eq!(s.feed(300.6, -200.0, t(0)), Some((300, -200)));
        // 余数 0.6 / 0.0 保留：再次喂 0.4 后到间隔发出 (1, 0)。
        assert_eq!(s.feed(0.4, 0.0, t(40)), Some((1, 0)));
        assert_eq!(s.feed(0.0, 0.0, t(80)), None);
    }

    #[test]
    fn zero_remainder_does_not_consume_interval() {
        let mut s = DeltaSampler::new(Duration::from_millis(33));
        // 0.5 未到 1：不发出也不占采样点；累积 0.5 后到间隔发出 (1,0)。
        assert_eq!(s.feed(0.5, 0.0, t(0)), None);
        assert_eq!(s.feed(0.5, 0.0, t(40)), Some((1, 0)));
    }

    #[test]
    fn reset_clears_remainder_and_restarts_interval() {
        let mut s = DeltaSampler::new(Duration::from_millis(33));
        assert_eq!(s.feed(5.0, 5.0, t(0)), Some((5, 5)));
        assert_eq!(s.feed(1.0, 1.0, t(10)), None); // 间隔内累计，不发送。
        s.reset();
        // reset 后首个非零增量立即发送（last_send 复位，不继承旧周期）。
        assert_eq!(s.feed(2.0, 0.0, t(11)), Some((2, 0)));
        assert_eq!(s.feed(1.0, 0.0, t(12)), None); // 新周期内累计。
        assert_eq!(s.feed(0.0, 0.0, t(50)), Some((1, 0)));
    }
}
