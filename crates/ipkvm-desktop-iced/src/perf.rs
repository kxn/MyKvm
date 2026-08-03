//! 帧渲染统计（跨线程共享，perf 示例退出时读取打印 JSON）。

use std::sync::Arc;
use std::time::Instant;

/// 帧渲染统计：记录帧到达时间戳，退出时汇总（帧数/平均/p95 帧间隔）。
#[derive(Debug, Default)]
pub struct FrameStats {
    inner: std::sync::Mutex<FrameStatsInner>,
}

#[derive(Debug, Default)]
struct FrameStatsInner {
    /// 每帧渲染到达的时间戳。
    timestamps: Vec<Instant>,
}

impl FrameStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 记录一帧在指定时刻到达。
    pub fn record_at(&self, at: Instant) {
        self.inner.lock().unwrap().timestamps.push(at);
    }

    /// 计算总结统计（帧数、平均/p95 帧间隔，毫秒）。
    pub fn summary(&self) -> (u64, f64, f64) {
        let inner = self.inner.lock().unwrap();
        let n = inner.timestamps.len() as u64;
        let mut intervals: Vec<f64> = Vec::new();
        for w in inner.timestamps.windows(2) {
            intervals.push(w[1].duration_since(w[0]).as_secs_f64() * 1000.0);
        }
        let avg = if intervals.is_empty() {
            0.0
        } else {
            intervals.iter().sum::<f64>() / intervals.len() as f64
        };
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95 = if intervals.is_empty() {
            0.0
        } else {
            intervals[((intervals.len() as f64 - 1.0) * 0.95).round() as usize]
        };
        (n, avg, p95)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn summary_computes_avg_and_p95() {
        let stats = FrameStats::new();
        let t0 = std::time::Instant::now();
        stats.record_at(t0);
        stats.record_at(t0 + Duration::from_millis(10));
        stats.record_at(t0 + Duration::from_millis(40));
        let (n, avg, p95) = stats.summary();
        assert_eq!(n, 3);
        assert!((avg - 20.0).abs() < 0.01);
        assert!((p95 - 30.0).abs() < 0.01);
    }

    #[test]
    fn empty_stats_returns_zero() {
        let stats = FrameStats::new();
        let (n, avg, p95) = stats.summary();
        assert_eq!((n, avg, p95), (0, 0.0, 0.0));
    }
}
