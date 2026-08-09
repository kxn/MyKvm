//! 真机取证诊断日志（#89）：轻量、可开关。
//!
//! 启用：环境变量 `IPKVM_ICED_DIAG=1`；输出追加到应用同目录的 `ipkvm-iced-diag.log`。
//! 日志点：启动参数 / RefreshDevices / PreviewTick 每 tick / FrameReady 每帧 /
//! UiTick 聚合（每 60 tick 一行）/ 在线状态跳变 / 连接断开时间点。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static UI_TICK_COUNT: AtomicU64 = AtomicU64::new(0);

/// 按环境变量 `IPKVM_ICED_DIAG` 打开/关闭诊断（进程启动时调用一次）。
pub fn init() {
    let enabled = std::env::var_os("IPKVM_ICED_DIAG").is_some();
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        log("diag started");
    }
}

/// 追加一行诊断日志（未启用时零开销）。
pub fn log(message: impl AsRef<str>) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(file, "[{timestamp}] {}", message.as_ref());
    }
}

/// UiTick 聚合计数：每 60 次 tick 记一行（约 1 秒一行，避免日志爆炸）。
pub fn ui_tick() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let count = UI_TICK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_multiple_of(60) {
        log(format!("UiTick aggregate={count}"));
    }
}

/// 诊断日志文件路径：默认写到当前可执行文件所在目录，便于用户复现后找到日志。
pub fn log_path() -> PathBuf {
    log_dir().join("ipkvm-iced-diag.log")
}

pub fn default_input_log_path() -> PathBuf {
    log_dir().join("ipkvm-input-diag.log")
}

pub fn enable_input_log(path: &Path) -> std::io::Result<()> {
    ipkvm_core::diag::configure(
        ipkvm_core::diag::DiagConfig::file(path)
            .level(ipkvm_core::diag::DiagLevel::Trace)
            .categories(input_log_categories()),
    )
}

pub fn disable_input_log() {
    ipkvm_core::diag::disable();
}

fn input_log_categories() -> ipkvm_core::diag::DiagCategory {
    ipkvm_core::diag::DiagCategory::INPUT
        | ipkvm_core::diag::DiagCategory::POINTER
        | ipkvm_core::diag::DiagCategory::QUEUE
        | ipkvm_core::diag::DiagCategory::SERIAL
        | ipkvm_core::diag::DiagCategory::LIFECYCLE
}

fn log_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_uses_app_dir_and_fixed_name() {
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let path = log_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("ipkvm-iced-diag.log")
        );
        assert_eq!(path.parent(), Some(exe_dir.as_path()));

        let input_path = default_input_log_path();
        assert_eq!(
            input_path.file_name().and_then(|n| n.to_str()),
            Some("ipkvm-input-diag.log")
        );
        assert_eq!(input_path.parent(), Some(exe_dir.as_path()));
    }
}
