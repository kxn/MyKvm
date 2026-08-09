# macOS 指针人工验证台账

本文记录 #82 的 macOS 目标机实机验证状态。自动化日志回放只能证明从桌面入口到
RFB mapper、CH9329 report、串口 frame 的软件链路持续携带按钮位，不能替代目标
macOS 对 HID 拖拽语义的实机接受验证。

## 当前状态

| 日期 | 操作者 | 宿主 | 目标 | 构建/提交 | 结果 |
| --- | --- | --- | --- | --- | --- |
| 2026-08-09 | 未执行 | 未记录 | macOS | #82 工作分支 | 未执行：当前自动化环境没有可操作的 macOS 目标机和真实 CH9329 链路。 |

## 待验证矩阵

| 场景 | 操作 | 预期 | 状态 | 需要保留的日志证据 |
| --- | --- | --- | --- | --- |
| Finder 标题栏拖拽 | 绝对模式按住标题栏移动 2 秒以上再释放 | 窗口持续跟随指针直到释放 | 未执行 | `desktop.iced absolute_pointer mask=0x01`、`session.rfb_pointer absolute_map incoming_mask=0x01`、`core.ch9329 pointer_report buttons=0x01`、串口 `mouse_abs buttons=0x01` |
| Finder 列表项拖拽 | 绝对模式按住文件或列表项移动到另一区域 | 项目进入拖拽状态并跟随 | 未执行 | 同上，并记录是否跨过 macOS 拖拽阈值 |
| 滚轮后继续拖拽 | 先滚轮，再立即按住标题栏拖拽 | 滚轮不改变鼠标模式，拖拽仍持续 | 未执行 | 滚轮后仍是绝对 pointer 链路，无 `PointerRelative` 混入 |
| 按住滚轮移动 | 按住中键移动并释放 | 中键状态不污染左键拖拽状态 | 未执行 | CH9329 buttons 与 RFB mask 位序一致 |
| 视频边缘坐标拖拽 | 从视频区边缘开始按住并向内/向外移动 | 出界日志可解释，入界报告不丢按钮 | 未执行 | `absolute_map_failed` 与成功 report 的时间边界 |

## 已有自动化证据

- `crates/ipkvm-desktop-iced/tests/input_log_replay.rs` 回放用户提供的长拖拽日志，验证按住期间桌面入口、RFB mapper、CH9329 report 和串口 frame 均持续携带左键按钮位。
- `absolute_drag_preserves_button_mask_across_desktop_to_sink_path` 验证桌面绝对拖拽路径不会下沉为相对事件。
- `real_ch9329_absolute_drag_carries_button_on_move_frames` 验证 RFB 绝对拖拽进入 CH9329 后，移动帧继续携带按钮位。
