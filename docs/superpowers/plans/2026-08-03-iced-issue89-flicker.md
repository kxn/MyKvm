# iced 预览/主视频画面闪烁修复记录（#89）

- **日期**：2026-08-03
- **关联**：Gitea `kxn/my_ipkvm#89`；分支 `codex/issue89-preview-flicker`

## 根因

每帧用 `Handle::from_rgba` 生成全新唯一 Id；1080p/720p RGBA 超过 2MB 阈值走
异步上传，上传完成前该层什么都不画，露出 letterbox 背景 → 闪烁。加重因素：

- `UiTick` 每 16ms 无条件触发 update → 重绘风暴放大空档；
- 预览每 100ms 用同一帧重建 Handle（`PreviewRuntime::tick` 不比较 seq）；
- `main_view` 按 `is_control_online()` 切整页（真实 CH9329 场景未验证，本次只补诊断）。

## 修复点

1. **新增 `PreloadedImage` widget**（`src/preloaded.rs`）：在 layout 阶段为
   新 Handle 阻塞预上传并持有 `Allocation`（iced 0.14 保证持有 Allocation 时
   同 Handle 立即可见）；旧帧 Allocation 保留到新帧上传成功才替换，消除空白
   窗口。主视频与连接页预览均改用该 widget。
2. **预览按 seq 去重**：`last_preview_seq` 记录最近显示帧，同 seq 不重建
   Handle（避免每 100ms 重复上传同一帧纹理）；换源/断开时统一重置。
3. **UiTick 节流**：16ms → 33ms（约 30Hz），flush_pending/drain_notices
   语义不变，减少无条件重绘。
4. **诊断日志**（`src/diag.rs`）：`IPKVM_ICED_DIAG=1` 时写
   `%TEMP%\ipkvm-iced-diag.log`，覆盖启动参数 / RefreshDevices / PreviewTick
   每 tick / FrameReady 每帧 / UiTick 聚合 / 在线状态跳变 / 连接断开时间点。

## 测试证据

- `app::tests::preview_tick_same_seq_does_not_rebuild_handle`：同 seq 帧
  Handle id 不变（先红后绿）；
- `tests/preloaded_image_pixels.rs`：tiny_skia 离屏渲染确认 `PreloadedImage`
  真实画出图像像素（先编译红后绿）；
- `diag::tests::log_path_uses_temp_dir_and_fixed_name`：诊断路径断言；
- M3 输入接线回归（flush_tick / relative / 组合键）全绿；
- `cargo fmt --all --check` 通过；`cargo test --workspace --all-features`
  全绿（47 个测试套件）。

## 待人工验证（真机）

- 真机 30s 观察：预览窗口与主视频窗口无可见闪烁；
- 采集 `%TEMP%\ipkvm-iced-diag.log` 复核：页面切换抖动、frame seq、
  preview handle 重建计数、UiTick 实际频率；
- perf 冒烟：`scripts/perf-1080p.ps1`（release 构建，10s）确认 CPU/帧间隔
  未回归（本机窗口运行，自动化无法替代）。
