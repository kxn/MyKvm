//! 真实串口命令队列：实现 [`CommandQueue`]，把 CH9329 帧写入串口设备。
//!
//! 用 `serialport` crate（跨平台：Windows COMx、Linux /dev/ttyUSBn、macOS /dev/cu.*）。
//! CH9329 出厂默认串口参数：**9600 8N1 无流控**。键鼠输出命令（0x02/0x04/0x05）在默认
//! 「无应答模式」下不需读取返回，纯单向发送即可——CH9329 收到合法帧即转成 USB-HID 输出。
//!
//! 帧间延时：CH9329 处理一帧需要时间，连续发送过快可能丢帧。9600bps 下一帧（~15 字节）
//! 传输本身约 16ms，加上芯片处理，命令间隔应 ≥ ~2ms（保守默认）。延时仅在多帧 batch 内
//! 帧间施加，batch 之间天然有上层事件间隔。
//!
//! 仅在 `serial` feature 下编译（由 `lib.rs` 的 `#[cfg(feature = "serial")] mod serial_port;` 门控）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serialport::SerialPort;

use crate::{CommandBatch, CommandQueue, CommandQueueError, QueueStats};

/// CH9329 出厂默认波特率。
pub const DEFAULT_BAUD_RATE: u32 = 9600;

/// 帧间保守延时（多帧 batch 内，每帧写完后等一会再写下一帧）。
const DEFAULT_INTER_FRAME_DELAY: Duration = Duration::from_millis(2);

/// 串口打开/写入错误。
#[derive(Debug, thiserror::Error)]
pub enum SerialCommandQueueError {
    #[error("failed to open serial port {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: serialport::Error,
    },
}

/// 把 CH9329 帧写入真实串口的 [`CommandQueue`]。
///
/// 线程安全：内部 `Arc<Mutex<SerialPort>>` 保护——Clone 时共享同一个串口句柄，
/// 这让 `Ch9329InputSink<SerialCommandQueue>` 满足 `Clone`（headless 的 RfbInputPump 需要
/// sink 可 Clone，因为文本输入服务也要一份）。
///
/// 错误处理：串口写入失败时返回 `CommandQueueError::Closed`（CH9329 命令队列错误模型
/// 目前只有 Closed 一种；真实串口断开/错误统一映射为 Closed，让上层输入链路按「串口不可用」处理）。
#[derive(Clone)]
pub struct SerialCommandQueue {
    port: Arc<Mutex<Box<dyn SerialPort>>>,
    inter_frame_delay: Duration,
    stats: Arc<Mutex<QueueStats>>,
}

impl SerialCommandQueue {
    /// 打开串口：`path`（如 `COM9` / `/dev/ttyUSB0`）、`baud`（默认 9600）、8N1 无流控。
    pub fn open(path: &str, baud: u32) -> Result<Self, SerialCommandQueueError> {
        let port = serialport::new(path, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            // 写超时：给一个合理的上限，避免设备异常时永久阻塞输入线程。
            .timeout(Duration::from_millis(500))
            .open()
            .map_err(|source| SerialCommandQueueError::Open {
                path: path.to_owned(),
                source,
            })?;
        Ok(Self {
            port: Arc::new(Mutex::new(port)),
            inter_frame_delay: DEFAULT_INTER_FRAME_DELAY,
            stats: Arc::new(Mutex::new(QueueStats::default())),
        })
    }

    /// 用默认波特率（9600）打开。
    pub fn open_default(path: &str) -> Result<Self, SerialCommandQueueError> {
        Self::open(path, DEFAULT_BAUD_RATE)
    }
}

impl CommandQueue for SerialCommandQueue {
    fn enqueue_batch(&self, batch: CommandBatch) -> Result<(), CommandQueueError> {
        let frames = batch.frames();
        let mut port = self.port.lock().map_err(|_| CommandQueueError::Closed)?;

        for (i, frame) in frames.iter().enumerate() {
            // 多帧 batch 内，非首帧前加一点延时，给 CH9329 处理上一帧的时间。
            if i > 0 && !self.inter_frame_delay.is_zero() {
                std::thread::sleep(self.inter_frame_delay);
            }
            use std::io::Write;
            if port.write_all(frame.as_bytes()).is_err() {
                // 串口写失败（设备断开等）：上层会因持续失败感知，统一映射 Closed。
                return Err(CommandQueueError::Closed);
            }
            // 立即刷新，确保字节送出（某些驱动会缓冲）。忽略 flush 错误（写已成功的话）。
            let _ = port.flush();
        }

        let mut stats = self.stats.lock().map_err(|_| CommandQueueError::Closed)?;
        stats.batches_accepted = stats.batches_accepted.saturating_add(1);
        stats.frames_accepted = stats.frames_accepted.saturating_add(frames.len() as u64);
        Ok(())
    }

    fn stats(&self) -> QueueStats {
        self.stats.lock().map(|s| *s).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_nonexistent_port_fails() {
        // 不存在的串口路径应返回 Open 错误（不 panic）。
        let result = SerialCommandQueue::open_default("COM_DOES_NOT_EXIST_999");
        assert!(result.is_err(), "opening a nonexistent port should fail");
    }

    #[test]
    fn serial_command_queue_implements_command_queue() {
        fn assert_command_queue<T: CommandQueue>() {}
        assert_command_queue::<SerialCommandQueue>();
    }

    #[test]
    fn default_baud_rate_is_9600() {
        // 回归：CH9329 出厂默认 9600，常被误记为 115200。
        assert_eq!(DEFAULT_BAUD_RATE, 9600);
    }

    #[test]
    fn default_inter_frame_delay_is_nonzero() {
        // 帧间保守延时保护 CH9329 不因连续发送过快丢帧；默认必须非零。
        assert!(!DEFAULT_INTER_FRAME_DELAY.is_zero());
    }
}
