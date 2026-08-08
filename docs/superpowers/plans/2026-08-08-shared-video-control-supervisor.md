# 共享视频与控制恢复状态机 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 #55：把视频链路和控制链路拆成独立恢复 runtime，并让 desktop/headless 共用共享层状态机。

**Architecture:** 在 `ipkvm-session` 新增稳定 `FrameHub`、共享状态类型和 `SessionSupervisor`。headless 和 desktop 只保留设备构造、UI/API 适配；RFB 支持视频可用但控制不可用时保持只读观看，不因 `event_tx` 缺失或关闭而结束视频连接。

**Tech Stack:** Rust 2024、Tokio、watch/mpsc channel、现有 `ipkvm-core` / `ipkvm-video` / `ipkvm-session` / `ipkvm-headless` / `ipkvm-desktop-core` / `ipkvm-desktop-iced`。

## Global Constraints

- 仓库内自写文档使用中文。
- PowerShell 处理中文前必须设置 UTF-8 编码。
- 非平凡改动围绕 GitHub issue #55 开发。
- 新增或修改核心逻辑先补失败测试，再实现，再确认测试通过。
- 控制重试耗尽后停留在视频页，显示控制设备失败提示，不返回连接页。
- 视频和控制是独立生命周期，一条链路异常不能无条件销毁另一条链路。
- web 和 desktop 使用同一套共享层恢复逻辑，不再各自维护恢复状态机。
- 不在本计划中重新设计 BIOS 绝对鼠标坐标域。

---

### Task 1: 共享稳定帧入口 `FrameHub`

**Files:**
- Create: `crates/ipkvm-session/src/frame_hub.rs`
- Modify: `crates/ipkvm-session/src/lib.rs`

**Interfaces:**
- Produces: `FrameHub::new_empty() -> Self`
- Produces: `FrameHub::set_source(&self, Arc<dyn FrameSource>) -> FrameHubForwarder`
- Produces: `FrameHub::clear(&self)`
- Produces: `FrameHub` implements `FrameSource`
- Consumes: existing `ipkvm_video::FrameSource`

- [ ] **Step 1: Write failing tests**

Add tests in `frame_hub.rs`:

```rust
#[tokio::test]
async fn subscribers_survive_source_replacement() {
    let hub = FrameHub::new_empty();
    let mut rx = hub.subscribe();

    let first = Arc::new(MockFrameSource::new());
    let forwarder = hub.set_source(first.clone());
    let task = tokio::spawn(forwarder.run());
    first.publish_frame(frame(1, 2));
    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().as_ref().unwrap().width, 2);
    task.abort();

    let second = Arc::new(MockFrameSource::new());
    let forwarder = hub.set_source(second.clone());
    let task = tokio::spawn(forwarder.run());
    second.publish_frame(frame(2, 4));
    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().as_ref().unwrap().width, 4);
    task.abort();
}

#[test]
fn clear_publishes_none_without_closing_subscription() {
    let hub = FrameHub::new_empty();
    let rx = hub.subscribe();
    hub.clear();
    assert!(rx.borrow().is_none());
    assert!(rx.has_changed().is_ok());
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-session frame_hub -- --nocapture`

Expected: compile failure because `frame_hub` module and `FrameHub` do not exist.

- [ ] **Step 3: Implement minimal `FrameHub`**

Create `frame_hub.rs` with:

```rust
pub struct FrameHub {
    sender: watch::Sender<Option<SharedVideoFrame>>,
    current: Arc<RwLock<Option<Arc<dyn FrameSource>>>>,
    generation: Arc<AtomicU64>,
}

pub struct FrameHubForwarder {
    hub: FrameHub,
    source: Arc<dyn FrameSource>,
    generation: u64,
}
```

`FrameHubForwarder::run()` subscribes to the source, forwards current/latest frames to `FrameHub.sender`, and exits when its generation is no longer current or source subscription closes.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-session frame_hub -- --nocapture`

Expected: all `frame_hub` tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-session/src/frame_hub.rs crates/ipkvm-session/src/lib.rs
git commit -m "feat(session): add stable frame hub #55"
```

### Task 2: 共享 supervisor 状态类型和退避策略

**Files:**
- Create: `crates/ipkvm-session/src/supervisor/status.rs`
- Create: `crates/ipkvm-session/src/supervisor/policy.rs`
- Create: `crates/ipkvm-session/src/supervisor/mod.rs`
- Modify: `crates/ipkvm-session/src/lib.rs`

**Interfaces:**
- Produces: `RecoveryPolicy { base_delay, max_delay, max_attempts, tick, video_start_timeout }`
- Produces: `RecoveryPolicy::next_delay(attempt: u32) -> Option<Duration>`
- Produces: `SupervisorStatus`, `SessionIntent`, `VideoRuntimeStatus`, `ControlRuntimeStatus`
- Consumes: `ActiveController`, `VideoSourceInfo`, `SourceStatsSnapshot`

- [ ] **Step 1: Write failing tests**

Add tests in `policy.rs`:

```rust
#[test]
fn next_delay_exponential_caps_and_exhausts() {
    let policy = RecoveryPolicy {
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(30),
        max_attempts: Some(3),
        tick: Duration::from_millis(500),
        video_start_timeout: Duration::from_secs(5),
    };
    assert_eq!(policy.next_delay(0), Some(Duration::from_secs(1)));
    assert_eq!(policy.next_delay(1), Some(Duration::from_secs(2)));
    assert_eq!(policy.next_delay(2), Some(Duration::from_secs(4)));
    assert_eq!(policy.next_delay(3), None);
}
```

Add tests in `status.rs`:

```rust
#[test]
fn control_failed_still_prefers_work_view() {
    let status = SupervisorStatus::for_test(
        SessionIntent::Running,
        VideoRuntimeStatus::Streaming,
        ControlRuntimeStatus::Failed {
            reason: "serial missing".into(),
            attempts: 3,
        },
    );
    assert!(status.should_show_work_view());
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-session supervisor -- --nocapture`

Expected: compile failure because supervisor module and types do not exist.

- [ ] **Step 3: Implement types**

Implement status enums with `Clone + Debug + Eq + PartialEq` where possible. `should_show_work_view()` returns false only for `SessionIntent::ManualStopped` and `SessionIntent::NoSelection`.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-session supervisor -- --nocapture`

Expected: supervisor status/policy tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-session/src/supervisor crates/ipkvm-session/src/lib.rs
git commit -m "feat(session): define supervisor status model #55"
```

### Task 3: 控制 runtime 从视频 runtime 中拆出

**Files:**
- Create: `crates/ipkvm-session/src/supervisor/control.rs`
- Modify: `crates/ipkvm-session/src/supervisor/mod.rs`
- Modify: `crates/ipkvm-session/src/session_manager.rs`

**Interfaces:**
- Produces: `ControlRuntime<S>`
- Produces: `ControlRuntime::start_with_sink(&mut self, sink: S, gate: RfbConnectionGate) -> Result<(), SessionError>`
- Produces: `ControlRuntime::stop_manual(&mut self) -> Result<(), SessionError>`
- Produces: `ControlRuntime::refresh_status(&mut self) -> ControlRuntimeStatus`
- Consumes: `SessionManager<S>` with `FrameHub` as stable frame source

- [ ] **Step 1: Write failing tests**

Add tests in `control.rs`:

```rust
#[tokio::test]
async fn input_pump_failure_does_not_clear_frame_hub() {
    let hub = Arc::new(FrameHub::new_empty());
    let mock = Arc::new(MockFrameSource::new());
    let forwarder = hub.set_source(mock.clone());
    let forward_task = tokio::spawn(forwarder.run());
    mock.publish_frame(frame(1, 2));

    let mut runtime = ControlRuntime::new(hub.clone());
    runtime
        .start_with_sink(FailingSink::fail_next_pointer(), RfbConnectionGate::new())
        .unwrap();
    let event_tx = runtime.event_publisher().borrow().clone().unwrap();
    event_tx
        .send(RfbServerEvent::Connected {
            client_id: RfbClientId::for_test(1),
            peer_addr: peer(),
            shared: true,
        })
        .await
        .unwrap();
    event_tx
        .send(RfbServerEvent::Pointer {
            client_id: RfbClientId::for_test(1),
            button_mask: 0,
            x: 1,
            y: 1,
            framebuffer_size: RfbSize::new(2, 2).unwrap(),
        })
        .await
        .unwrap();
    wait_until(|| matches!(runtime.refresh_status(), ControlRuntimeStatus::Recovering { .. })).await;
    assert!(hub.latest_frame().is_some());
    forward_task.abort();
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-session control_runtime -- --nocapture`

Expected: compile failure because `ControlRuntime` does not exist.

- [ ] **Step 3: Implement minimal control runtime**

`ControlRuntime` owns `SessionManager<S>` initialized with `FrameHub` as its frame source. It restarts/stops only the input pump and sink; it never clears `FrameHub`.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-session control_runtime -- --nocapture`

Expected: control runtime tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-session/src/supervisor/control.rs crates/ipkvm-session/src/supervisor/mod.rs crates/ipkvm-session/src/session_manager.rs
git commit -m "feat(session): split control runtime from frame source #55"
```

### Task 4: 视频 runtime 与 supervisor tick

**Files:**
- Create: `crates/ipkvm-session/src/supervisor/video.rs`
- Create: `crates/ipkvm-session/src/supervisor/runtime.rs`
- Modify: `crates/ipkvm-session/src/supervisor/mod.rs`

**Interfaces:**
- Produces: `VideoRuntime`
- Produces: `SessionSupervisor<Selection, S, F>`
- Produces: `SupervisorFactory<Selection, S>` trait with `build_video` and `build_control`
- Produces: `SessionSupervisor::start(selection)`
- Produces: `SessionSupervisor::tick()`
- Produces: `SessionSupervisor::stop_manual()`
- Consumes: `FrameHub`, `ControlRuntime`, `RecoveryPolicy`

- [ ] **Step 1: Write failing tests**

Add tests in `runtime.rs`:

```rust
#[tokio::test(start_paused = true)]
async fn control_exhaustion_keeps_video_streaming_and_work_view() {
    let factory = ScriptedFactory::new()
        .video_ok()
        .control_errors(["missing", "missing", "missing"]);
    let mut supervisor = SessionSupervisor::new(factory, RecoveryPolicy::short_for_test());
    supervisor.start(TestSelection::default()).await.unwrap();
    supervisor.tick().await;
    advance(policy_window()).await;
    supervisor.tick().await;
    advance(policy_window()).await;
    supervisor.tick().await;

    let status = supervisor.status();
    assert!(matches!(status.video, VideoRuntimeStatus::Streaming { .. }));
    assert!(matches!(status.control, ControlRuntimeStatus::Failed { .. }));
    assert!(status.should_show_work_view());
}

#[tokio::test]
async fn manual_stop_prevents_automatic_revival() {
    let factory = ScriptedFactory::new().video_ok().control_ok();
    let mut supervisor = SessionSupervisor::new(factory, RecoveryPolicy::short_for_test());
    supervisor.start(TestSelection::default()).await.unwrap();
    supervisor.stop_manual().await.unwrap();
    supervisor.tick().await;
    assert!(matches!(supervisor.status().intent, SessionIntent::ManualStopped));
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-session supervisor_runtime -- --nocapture`

Expected: compile failure because runtime types do not exist.

- [ ] **Step 3: Implement supervisor**

The supervisor owns one `FrameHub`, one `VideoRuntime`, and one `ControlRuntime`. `tick()` detects stopped control pump and video source states, advances retry timers, and publishes `SupervisorStatus`.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-session supervisor_runtime -- --nocapture`

Expected: supervisor runtime tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-session/src/supervisor/video.rs crates/ipkvm-session/src/supervisor/runtime.rs crates/ipkvm-session/src/supervisor/mod.rs
git commit -m "feat(session): add shared video control supervisor #55"
```

### Task 5: RFB 支持视频只读连接

**Files:**
- Modify: `crates/ipkvm-session/src/rfb_connection/driver.rs`
- Modify: `crates/ipkvm-session/src/rfb_connection/finalize.rs`
- Modify: `crates/ipkvm-headless/src/rfb_ws/service.rs`
- Modify: `crates/ipkvm-headless/src/rfb_tcp/server.rs`

**Interfaces:**
- Produces: `run_viewer_connection(..., event_publisher: watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>>, ...)`
- Produces: RFB input events ignored while no current control sender exists
- Consumes: existing `FrameSource` subscription and `RfbConnectionGate`

- [ ] **Step 1: Write failing tests**

Add test in RFB driver:

```rust
#[tokio::test]
async fn viewer_connection_continues_when_event_sender_is_absent() {
    let frame_source = MockFrameSource::new();
    frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 6]));
    let (_event_tx, event_rx) = watch::channel(None);
    let end = run_viewer_connection_for_test(&frame_source, event_rx, key_press_script()).await;
    assert!(!matches!(end, ConnectionEnd::Failed(RfbConnectionError::EventChannelClosed)));
}
```

Add WS service test:

```rust
#[tokio::test]
async fn websocket_upgrade_is_not_rejected_when_control_sender_is_none() {
    let response = upgrade_response_with_event_sender(None).await;
    assert_ne!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-session rfb_connection -- --nocapture` and `cargo test -p ipkvm-headless rfb_ws -- --nocapture`

Expected: tests fail because current implementation requires `event_tx`.

- [ ] **Step 3: Implement read-only control mode**

Replace the fixed `event_tx` in RFB driver with a current-sender provider. On input events, if no sender exists, ignore the input and continue serving frames. On disconnect, only send `Disconnected` if a sender exists and is current.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-session rfb_connection -- --nocapture` and `cargo test -p ipkvm-headless rfb_ws -- --nocapture`

Expected: RFB connection and WS service tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-session/src/rfb_connection crates/ipkvm-headless/src/rfb_ws/service.rs crates/ipkvm-headless/src/rfb_tcp/server.rs
git commit -m "feat(rfb): allow video-only connections during control recovery #55"
```

### Task 6: headless 接入共享 supervisor

**Files:**
- Modify: `crates/ipkvm-headless/src/web/service.rs`
- Delete or stop using: `crates/ipkvm-headless/src/web/recovery.rs`
- Modify: `crates/ipkvm-headless/src/web/mod.rs`
- Modify: `crates/ipkvm-headless/src/frame_source.rs`
- Modify: `crates/ipkvm-headless-app/src/main.rs`
- Modify tests in `crates/ipkvm-headless/tests/web_http.rs`

**Interfaces:**
- Consumes: `SessionSupervisor`
- Produces: `/api/status` fields for `video.state`, `control.state`, `session.intent`
- Produces: `/api/session create/restart/stop` calls supervisor actions

- [ ] **Step 1: Write failing tests**

Add/modify web tests:

```rust
#[tokio::test]
async fn api_status_reports_control_failed_without_absent_video() {
    let server = spawn_server_with_supervisor_state(
        VideoRuntimeStatus::Streaming,
        ControlRuntimeStatus::Failed {
            reason: "serial missing".into(),
            attempts: 3,
        },
    )
    .await;
    let body = server.get_json("/api/status").await;
    assert_eq!(body["video"]["state"], "streaming");
    assert_eq!(body["control"]["state"], "failed");
    assert_eq!(body["session"]["view"], "work");
}

#[tokio::test]
async fn manual_stop_prevents_recovery_until_create_or_restart() {
    let server = spawn_recoverable_server().await;
    server.post_session("stop").await;
    advance(Duration::from_secs(60)).await;
    let body = server.get_json("/api/status").await;
    assert_eq!(body["session"]["intent"], "manual_stopped");
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-headless web_http -- --nocapture`

Expected: tests fail because API does not expose supervisor state and uses old recovery.

- [ ] **Step 3: Implement headless adapter**

Replace `ApiState` manager/frame_source/manual_stop fields with shared `SessionSupervisor`. Keep `SessionSelection` and `SessionFactory` as headless adapter types, but make them implement supervisor factory traits.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-headless web_http -- --nocapture`

Expected: headless web tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-headless crates/ipkvm-headless-app
git commit -m "feat(headless): use shared session supervisor #55"
```

### Task 7: desktop 接入共享 supervisor

**Files:**
- Modify: `crates/ipkvm-desktop-core/src/session.rs`
- Modify: `crates/ipkvm-desktop/src/session.rs`
- Modify: `crates/ipkvm-desktop-iced/src/app.rs`
- Modify: `crates/ipkvm-desktop-iced/src/status.rs`

**Interfaces:**
- Consumes: `SessionSupervisor`
- Produces: desktop controller methods backed by supervisor snapshot
- Produces: iced main view stays on video/work page when control failed

- [ ] **Step 1: Write failing tests**

Add iced test:

```rust
#[test]
fn control_failure_keeps_main_view_in_work_state() {
    let (mut app, sink) = MockApp::new_mock();
    let _ = app.update(Message::FrameReady(make_bgra_frame(1, 16, 9)));
    sink.fail_next_pointer();
    app.controller.inject_control_failure_for_test("serial missing");
    let _ = app.update(Message::UiTick);
    assert!(app.controller.status().should_show_work_view());
    assert!(matches!(app.status(), ConnectionStatus::ControlOffline(_)));
}
```

Add desktop-core test:

```rust
#[test]
fn controller_stop_manual_is_the_only_path_to_connection_view() {
    let (mut controller, _sink) = controller_with_sink();
    controller.connect(request()).unwrap();
    controller.inject_control_failure_for_test("serial missing");
    assert!(controller.status().should_show_work_view());
    controller.stop().unwrap();
    assert!(!controller.status().should_show_work_view());
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p ipkvm-desktop-core session -- --nocapture` and `cargo test -p ipkvm-desktop-iced app::tests -- --nocapture`

Expected: tests fail because desktop still keys view off `is_control_online()` and auto-destroys session.

- [ ] **Step 3: Implement desktop adapter**

Make `DesktopSessionController` own or wrap a `SessionSupervisor`. Remove iced `sync_status()` automatic `controller.stop()` on input offline. `main_view()` uses `controller.status().should_show_work_view()`.

- [ ] **Step 4: Verify green**

Run: `cargo test -p ipkvm-desktop-core session -- --nocapture` and `cargo test -p ipkvm-desktop-iced app::tests -- --nocapture`

Expected: desktop tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ipkvm-desktop-core crates/ipkvm-desktop crates/ipkvm-desktop-iced
git commit -m "feat(desktop): keep work view during control recovery #55"
```

### Task 8: 文档、全量验证和 PR

**Files:**
- Modify: `docs/ipkvm-coarse-design.md`
- Modify: `docs/superpowers/specs/2026-08-08-shared-video-control-supervisor-design.md`
- Modify: `docs/superpowers/plans/2026-08-08-shared-video-control-supervisor.md`

**Interfaces:**
- Consumes: all earlier tasks
- Produces: updated long-term documentation and PR evidence

- [ ] **Step 1: Update docs**

Document:

- shared supervisor replaces headless private recovery.
- video/control states are independent.
- control failed/exhausted remains on work/video page.
- RFB supports video-only while control is unavailable.

- [ ] **Step 2: Run format check**

Run: `cargo fmt --all --check`

Expected: exit 0.

- [ ] **Step 3: Run workspace tests**

Run: `cargo test --workspace --all-features`

Expected: exit 0.

- [ ] **Step 4: Request code review**

Use the requesting-code-review skill with:

- DESCRIPTION: implemented #55 shared video/control supervisor
- PLAN_OR_REQUIREMENTS: this plan and issue #55
- BASE_SHA: commit before Task 1
- HEAD_SHA: current HEAD

- [ ] **Step 5: Fix review feedback**

Fix Critical and Important findings with TDD, rerun focused tests and full workspace tests.

- [ ] **Step 6: Push and create PR**

```powershell
git push -u origin codex/55-shared-video-control-supervisor
gh pr create --repo kxn/MyKvm --base main --head codex/55-shared-video-control-supervisor --title "feat(session): split video and control recovery state machines" --body-file <UTF-8 PR body>
```

PR body must include `Closes #55`, summary, design basis, tests, documentation impact, and manual verification exceptions.

## 执行记录（2026-08-08）

已按 #55 分支落地共享 supervisor 方案：

- `ipkvm-session` 新增并接入稳定 `FrameHub` 与 `SessionSupervisor`；`FrameHub` 对外 seq 保持单调，视频源替换不关闭订阅。
- RFB WS/TCP 支持 video-only 连接；控制 sender 缺失或关闭时不再断开观看路径，输入事件按只读语义忽略。
- headless 删除私有 `web/recovery.rs` 和 `SwitchableFrameSource`，HTTP API/状态轮询/前端均接入共享 supervisor。
- desktop-core/desktop/iced 接入共享 supervisor，控制失败或恢复耗尽时保持工作页；只有人工停止回连接页。
- Web 前端在控制从非 `ready` 恢复到 `ready` 后受控重连 RFB，以获取新的输入 sender。
- 初始 create/restart/connect 的 video/control 构建错误进入 supervisor 恢复态，不再触发 500 回滚或回连接页。

验证证据：

- `cargo fmt --all --check`
- `node --check crates/ipkvm-headless/web/modules/app.js`
- `cargo test -p ipkvm-session frame_hub -- --nocapture`
- `cargo test -p ipkvm-session supervisor -- --nocapture`
- `cargo test -p ipkvm-session rfb_connection -- --nocapture`
- `cargo test -p ipkvm-headless --test web_http -- --nocapture`
- `cargo test -p ipkvm-headless-app --test headless_process -- --nocapture`
- `cargo test -p ipkvm-desktop-core session -- --nocapture`
- `cargo test -p ipkvm-desktop-iced status -- --nocapture`
- `cargo check -p ipkvm-desktop-iced`
- `cargo check -p ipkvm-browser-fixture`
- `cargo test --workspace --all-features`

未覆盖的人工验证边界：

- 真实目标机 BIOS/Windows 重启期间的视频和控制链路恢复。
- CH9329 短断电、串口设备长时间缺失后的重试耗尽提示。
- 真实浏览器 noVNC 在控制恢复后的受控重连体验。
