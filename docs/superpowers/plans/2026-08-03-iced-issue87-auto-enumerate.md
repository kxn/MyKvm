# iced 连接页启动自动枚举设备修复记录（#87）

- **日期**：2026-08-03
- **关联**：Gitea `kxn/my_ipkvm#87`；分支 `codex/issue87-auto-enumerate`

## 根因

`App::production()` 的初始 Task 为 `Task::none()`，从未触发
`Message::RefreshDevices`；连接页打开时 `video_devices`/`control_devices` 为空，
iced `PickList` 空选项时点击无菜单展开（官方控件行为）。egui 版在
`DesktopApp::new()` 构造时同步执行一次 `refresh_detection(...)` 并预填
`store.last_manual()`，所以启动即可展开选择。

## 修复点

- `App::production()` 与 `App::new_mock()` 返回 `Task::done(Self::startup_message())`，
  启动后自动执行一次设备枚举（`startup_message() == Message::RefreshDevices`）；
- 新增 `prefill_last_manual()`：构造时读 `store.last_manual()` 预填连接参数与
  设备选择（枚举完成前直接预填 id，`refresh_detection` 随后复核，设备缺失置
  `Disconnected`，不阻塞启动）；
- 枚举失败沿用既有路径：`status_message` 显示失败原因，列表不被替换。

## 测试证据

新增 4 个 headless 测试（先红后绿）：

- `startup_task_auto_enumerates_devices`：启动 Task 为 `RefreshDevices` 且
  `units == 1`（`Task::none` 为 0）；消费启动消息后列表非空；
- `production_startup_task_triggers_enumeration`：生产 App 启动 Task 携带消息；
- `startup_enumeration_failure_reports_and_does_not_block`：枚举失败不阻塞启动、
  列表不替换、原因写入状态消息区；
- `startup_prefills_last_manual_snapshot`：预填连接参数与设备选择。

门禁：`cargo fmt --all --check` 通过；`cargo test --workspace --all-features`
全绿（M2 连接页状态机/菜单/模态回归未受影响）。

## 人工验证（待用户）

- 真机确认启动后连接页自动完成枚举、下拉框立即可展开选择；枚举失败时启动不
  阻塞且状态栏/消息区显示原因。
