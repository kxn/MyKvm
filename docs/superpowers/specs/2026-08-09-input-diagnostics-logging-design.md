# 输入诊断日志系统设计

关联 issue：`#73`

日期：2026-08-09

## 背景

桌面端在 macOS 目标机上复现出拖拽只能移动一小段、滚轮后移动异常、相对模式按钮状态错乱等问题。此前只能从现象倒推状态机，缺少一份能覆盖「桌面入口 -> desktop-core pending 队列 -> RFB 输入泵/mapper -> CH9329 sink -> 串口传输」的端到端事实记录。

这套日志系统的目标是复现问题后能直接拿到文件，回答以下问题：

- 桌面入口是否产生了正确的按钮 mask、坐标、滚轮和远程输入状态。
- pending 队列是否因为通道满或关闭导致输入事件滞留。
- RFB 指针 mapper 在提交前后的 `committed_mask` 与 `incoming_mask` 是否符合预期。
- CH9329 最终输出的绝对/相对 mouse report 是否携带正确按钮位和滚轮值。
- 串口队列、frame 发送、ack 和故障恢复是否连续。

## 设计原则

- 日志写文件，不依赖 console；GUI 复现时控制台可能不存在或滚屏不可保留。
- 日志是诊断能力，不改变输入链路状态机和调度语义。
- 分类可控：默认覆盖鼠标、队列、串口和生命周期；键盘详细类别默认关闭，避免不必要的敏感输入记录。
- 格式稳定：单行 `logfmt`，便于 grep、按时间排序和脚本分析。
- 配置入口分离：`ipkvm-core` 只提供 logger；desktop/headless 负责 UI、CLI、配置文件和环境变量启用。

## 架构

`ipkvm-core::diag` 提供全局轻量文件 logger：

- `DiagLevel`：`error`、`warn`、`info`、`debug`、`trace`。
- `DiagCategory`：`input`、`pointer`、`keyboard`、`queue`、`serial`、`lifecycle`、`all`。
- `DiagConfig::file(path).level(...).categories(...)`。
- `configure()`、`disable()`、`is_enabled()`、`log()`。

每行日志包含固定前缀字段：

```text
ts_ms=<unix毫秒> mono_ms=<进程单调毫秒> level=<级别> category=<类别> component=<组件> event=<事件> ...
```

后续字段由调用点追加，字段值按 logfmt 规则转义。logger 内部持有文件锁，单次 `log()` 原子写入一行并 flush，优先保证复现崩溃或退出时日志已经落盘。

## 启用方式

桌面 iced：

- 底部状态栏提供「输入日志」复选框。
- 打开后写入系统临时目录下 `ipkvm-input-diag.log`。
- 默认级别 `trace`，类别为 `input,pointer,queue,serial,lifecycle`。

headless：

- CLI：`--log-file <路径>`、`--log-level <级别>`、`--log-categories <列表>`。
- TOML：`[logging] file/level/categories`。
- 环境变量：`IPKVM_LOG_FILE`、`IPKVM_LOG_LEVEL`、`IPKVM_LOG_CATEGORIES`。
- 优先级沿用配置合并模型：CLI/文件配置进入 `Options`，环境变量只在未显式指定日志文件时作为启动便利入口。

## 调用点

桌面 iced 入口：

- `remote_input` 进入/退出、窗口失焦释放、输入日志开关。
- `cursor_moved`、`mouse_button`、绝对指针发送/抑制、绝对滚轮瞬时边沿、坐标映射失败。

desktop-core：

- `send_event` 记录事件入队。
- `flush_pending` 记录通道满、关闭、已发送数量、剩余数量和下一事件类型。

ipkvm-session：

- 输入泵生命周期、控制者断开、事件源关闭。
- keyboard 类别下记录 RFB key path；默认不启用。
- pointer 类别下记录标准绝对 pointer、相对 pointer、模式不匹配过滤、绝对/相对 mapper 展开结果。

CH9329 sink：

- `pointer_batch` 记录批次前后鼠标状态、事件数和命令数。
- `pointer_report` 记录最终绝对/相对报告的按钮位、坐标/位移和滚轮值。
- `release_all` 记录释放前键鼠状态。

串口传输：

- `enqueue_batch` 记录 accepted/full/closed。
- `frame_tx` 记录 CH9329 command、字节数和 mouse report 摘要。
- `frame_ack` 记录 ack command 与 pending 数。
- `transport_fault` 记录 timeout、协议错误、设备拒绝和 I/O 错误。

## 验收

- 不启用时不创建文件，不向 console 输出诊断日志。
- 桌面 GUI 能通过开关启用和关闭输入日志，状态栏显示文件路径或失败原因。
- headless 能通过 CLI、TOML 和环境变量启用。
- `trace` 级别下复现拖拽问题时，日志能串起入口 mask、mapper mask、pending 状态、CH9329 report buttons 和串口 frame/ack。
- 默认类别不包含 `keyboard`。

## 风险与边界

- 全局 logger 当前一次只支持一个文件目标，足够覆盖单进程 desktop/headless 复现；后续如果要做多 sink 再扩展。
- trace 级别每行 flush 会增加 I/O 开销，只建议复现问题时打开。
- 日志记录的是本进程看到和发出的事件，不证明目标 OS 已接受 HID 语义；目标 OS 兼容性仍需要结合屏幕现象判断。
