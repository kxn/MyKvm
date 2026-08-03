# iced 连接页/主视图布局重构记录（#90）

- **日期**：2026-08-03
- **关联**：Gitea `kxn/my_ipkvm#90`；分支 `codex/issue90-layout-refactor`

## 背景

M2 连接页为单列垂直堆叠，与 egui 桌面版布局不同。本单按 #90 取证结论
（已从 `crates/ipkvm-desktop/src/app.rs` 提取规格）重构布局。

## 目标布局（对齐 egui）

### 连接页

- 顶部：标题 + profile 行（下拉宽 240 + 保存 + 连接设置入口）；
- 横向两栏：左栏 `set_width(380)`（视频/控制下拉 + 状态、刷新/连接按钮
  140×36、消息区）、1px 分隔线、右栏 320×180 预览（Contain 等比、无帧
  占位文字）；
- 连接参数（波特率/预览帧率/自动波特率/鼠标模式）移入连接设置模态
  （`ModalKind::Connection`），页面不再内联堆放。

### 视频页

- 整块 available 区域，letterbox 背景 + 1px 边框；
- 视频等比居中（Contain，无裁切）；无帧时显示 28px 无信号文字；
- 点击进远程输入沿用既有全局事件路径（粘性，Ctrl+Alt+K 退出）。

### 全局

- 顶菜单栏 + 底部状态栏（控制设备 | 键盘 | 指针 | 视频 | 消息）+
  中央二选一（视频页/连接页）；各设置均为 Modal。

## 测试证据

新增/更新 headless 测试（先红后绿）：

- `connection_page_view_renders_after_theme_wiring`：profile 行（保存/连接设置）、
  左栏刷新/连接、右栏占位文字；
- `video_view_shows_no_signal_when_connected_without_frame`：视频页无帧显示
  28px 无信号文字；
- `status_line_shows_five_fields`：状态栏五字段（控制/键盘/指针/视频/消息）；
- `connection_modal_contains_connection_params`：连接设置模态含波特率/帧率/
  自动波特率/鼠标模式。

新增 i18n 键：`status.pasting / remote_input / keyboard_lost / relative_mode /
pointer_outside`（en + zh-CN，同步 `translate_key` 与 `I18N_KEYS`）。

门禁：`cargo fmt --all --check` 通过；`cargo test --workspace --all-features`
全绿（47 个测试套件）；M1 缩放测试（250% DPI 三模式不裁切）回归通过。

## 待人工验证（真机/截图）

- 截图对比：连接页/视频页布局与 egui 版一致（左 380 / 分隔线 / 右 320×180
  预览、profile 行、五字段状态栏）；
- 250% DPI 下缩放/黑边无裁切；
- 真机走一遍选设备 → 预览 → 连接 → 远程输入 → 断开，确认交互无回归。
