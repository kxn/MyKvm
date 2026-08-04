# #154 相对模式本地捕获边界实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. 本计划在 #151 profile 语义和 #156 flush 调度稳定后执行。

**Goal:** 将目标端 Relative 模式与主控端本地捕获分离，desktop 锁定实际视频矩形，Web 正确处理 Pointer Lock 与软降级。

**状态：** 实现与自动化回归已完成，纳入联合 PR 收口；真实 Windows/浏览器捕获矩阵保留为人工例外。

**Architecture:** 桌面 `CursorController` 接收明确的 screen-space 视频矩形，进入/退出由 app 的 capture state 驱动；Web `pointer.js` 保存 selected relative、locked、armed 三个状态，Pointer Lock 失败不回退目标端 profile。

## Global Constraints

- Windows ClipCursor 只裁剪实际视频区域；失焦/断开/绝对模式/退出时必定释放。
- Web Pointer Lock 必须在用户手势中请求；软降级只改变本地捕获，不清除 Relative profile。
- 不修改 CH9329 目标端模式映射或 RFB 协议。

### Task 1: Capture State Tests

**Files:** `crates/ipkvm-desktop-iced/src/platform/cursor.rs`, `src/video_area.rs`, `src/app.rs`, `crates/ipkvm-headless/web/modules/pointer.js`.

- [x] 定义可测试的 capture state transition：Inactive/Armed/Captured，覆盖 focus、Esc、disconnect、mode switch。
- [x] 增加 screen-space 视频矩形转换和窗口/DPI 更新测试；确认非视频区域不被裁剪。
- [x] 增加 Web Pointer Lock success/error/exit/blur fallback tests，确认 Relative 选择保持。

### Task 2: Desktop Region Lock

**Files:** `crates/ipkvm-desktop-iced/src/platform/cursor.rs`, `src/platform/mod.rs`, `src/app.rs`, `src/video_area.rs`.

- [x] 将 `set_clipped(bool)` 扩展为 `set_clip_rect(Option<Rect>)` 或等价接口，Windows 调用 ClipCursor 实际视频 screen rect。
- [x] 在视频区域更新、窗口移动/resize/scale、进入/退出/失焦路径同步 rect；所有 release 路径显示光标并清除 ClipCursor。
- [x] 不让目标端 Relative 模式因本地鼠标离开视频区而切回 Absolute。

### Task 3: Web Hard/Soft Capture

**Files:** `crates/ipkvm-headless/web/modules/pointer.js`, `crates/ipkvm-headless/web/modules/status.js`, `crates/ipkvm-headless/web/modules/app.js`, `crates/ipkvm-headless/web/app.css`, `crates/ipkvm-headless/web/index.html`.

- [x] selected relative 只进入 armed；用户点击视频区按钮才 requestPointerLock。
- [x] pointerlockchange/error、Esc、blur、disconnect 更新 captured 状态并恢复区域外 UI。
- [x] 不支持/失败时视频区隐藏光标、离开视频区显示并暂停相对捕获；重新进入可恢复。
- [x] 与 #156 的相对移动 timer 清理接口统一，避免残留待发 dx/dy。

### Task 4: Verify and Document

- [x] 运行 Rust、browser、结构门禁和 release 构建。
- [x] 更新 headless/iced 设计文档，记录硬锁和降级的边界。
- [x] 在 PR 记录 Windows DPI/窗口移动和 Chrome/Edge/降级浏览器的人工验证例外。
- [x] 纳入联合提交并使用 `Closes #154` 收口。
