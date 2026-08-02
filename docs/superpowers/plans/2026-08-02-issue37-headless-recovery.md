# Issue #37 headless 视频断流与 CH9329 掉线恢复模型实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** headless 在视频断流/CH9329 掉线时保持可观测且可恢复：`/api/status` 反映断流与输入离线原因/时间；输入泵失败后按指数退避自动重建会话；视频断流恢复后分辨率变化继续通过 RFB DesktopSize 通知。

**Architecture:** 三层：(1) `SessionStats` 增加 `last_frame_ns` 与 `input_offline`，泵任务错误退出时写入原因与时间；(2) `/api/status` 暴露 `video.frame.last_frame_ns/stalled` 与 `session.input_offline`；(3) headless 新增恢复 supervisor：泵失败或视频从未出帧时按 1s→2s→…→30s 退避重建会话；视频曾出帧后停滞只报告不重启（目标机重启场景，避免抢串口）。

**Tech Stack:** Rust workspace；tokio；axum；`tea`（Gitea）。

## Global Constraints

- 仓库文档中文；提交信息英文 conventional commit。
- 围绕 Gitea issue #37；TDD。
- 提交前通过 `cargo fmt --all --check`、`cargo test --workspace --all-features`。
- 恢复重试必须退避且单例执行，避免目标机反复上下电时抢串口。
- desktop 维持第一版“不自动重连”行为；自动恢复只作用于 headless。

---

## 文件结构

- `crates/ipkvm-session/src/console_session.rs`：`InputOfflineInfo`、`SessionStats` 扩展、泵错误记录。
- `crates/ipkvm-headless/src/web/service.rs`：`FrameStatus`/`SessionStatus` 扩展与 stalled 计算。
- `crates/ipkvm-headless/src/web/recovery.rs`：`RecoveryPolicy` + 恢复循环。
- `crates/ipkvm-headless/src/web/mod.rs`：注册 `mod recovery;`。
- 测试：`console_session`（泵失败记录）、`web_http`（status 字段）、`rfb_dynamic_resolution`（断流后换分辨率恢复）。

---

### Task 1: 会话统计可观测性

**Files:**
- Modify: `crates/ipkvm-session/src/console_session.rs`

- [ ] **Step 1: 失败测试**（`console_session.rs` 测试：`pump_error_marks_session_stopped` 后追加断言）

```rust
        assert!(session.stats().input_offline.is_some());
        assert!(!session.stats().input_offline.as_ref().unwrap().reason.is_empty());
```

- [ ] **Step 2: 运行确认失败**（`cargo test -p ipkvm-session console_session::tests::pump_error_marks_session_stopped`）

- [ ] **Step 3: 实现**

```rust
/// 输入离线信息：泵因错误退出后的原因与时间。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputOfflineInfo {
    pub reason: String,
    pub since_ns: u64,
}
```

`SessionStats` 追加字段并更新文档注释；`observe_frame` 记录：

```rust
    pub fn observe_frame(&mut self) {
        if let Some(frame) = self.frame_source.latest_frame() {
            let mut stats = self.stats.lock().unwrap();
            stats.observe_frame_seq(frame.seq);
            stats.last_frame_ns = Some(crate::now_ns());
        }
    }
```

`start()` 的泵任务包装：启动前清空 `input_offline`，失败时写入：

```rust
        stats.lock().unwrap().input_offline = None;
        let task = tokio::spawn(async move {
            let result = pump.run_until_stopped(&mut event_rx, stop_rx, ...).await;
            if let Err(error) = &result {
                stats.lock().unwrap().input_offline = Some(InputOfflineInfo {
                    reason: error.to_string(),
                    since_ns: crate::now_ns(),
                });
            }
            running.store(false, Ordering::SeqCst);
            result
        });
```

`refresh_stats` 中 `observe_frame()` 顺带更新 `last_frame_ns`。

- [ ] **Step 4: 运行确认通过**（`cargo test -p ipkvm-session console_session::tests::`）

- [ ] **Step 5: 提交** `git commit -m "feat: record frame staleness and input offline reason in session stats"`

---

### Task 2: /api/status 断流与离线字段

**Files:**
- Modify: `crates/ipkvm-headless/src/web/service.rs`
- Modify: `crates/ipkvm-headless/tests/web_http.rs`

- [ ] **Step 1: 失败测试**（`web_http.rs` 的 `api_status_reports_video_and_controller` 追加）

```rust
    assert_eq!(status["video"]["stalled"], false);
    assert!(status["video"]["frame"]["last_frame_ns"].is_number());
    assert!(status["session"].get("input_offline").is_none());
```

- [ ] **Step 2: 运行确认失败**（`cargo test -p ipkvm-headless --test web_http api_status_reports_video_and_controller`）

- [ ] **Step 3: 实现**

`const VIDEO_STALL_TIMEOUT_NS: u64 = 2_000_000_000;`

`VideoStatus` 增加 `stalled: bool`；`FrameStatus` 增加 `last_frame_ns`（skip None）；`SessionStatus` 增加 `input_offline: Option<InputOfflineDto>`（skip None）；`InputOfflineDto { reason: String, since_ns: u64 }`，并 `impl From<&InputOfflineInfo>`。

`api_status` 内：从 stats 取 `last_frame_ns`/`input_offline`；`stalled = match &frame { Some(_) => last_frame_ns.map_or(true, |t| now_ns.saturating_sub(t) > VIDEO_STALL_TIMEOUT_NS), None => session_state != "absent" }`。

- [ ] **Step 4: 运行确认通过**（`cargo test -p ipkvm-headless --test web_http`）

- [ ] **Step 5: 提交** `git commit -m "feat: expose video stall and input offline in api status"`

---

### Task 3: 恢复策略与 supervisor

**Files:**
- Create: `crates/ipkvm-headless/src/web/recovery.rs`
- Modify: `crates/ipkvm-headless/src/web/mod.rs`
- Modify: `crates/ipkvm-headless/src/web/service.rs`（serve 中 spawn）

- [ ] **Step 1: 失败测试**（`recovery.rs` 内）

```rust
    #[test]
    fn next_delay_backs_off_exponentially_and_caps() {
        let policy = RecoveryPolicy::default();
        assert_eq!(policy.next_delay(0), Duration::from_secs(1));
        assert_eq!(policy.next_delay(1), Duration::from_secs(2));
        assert_eq!(policy.next_delay(4), Duration::from_secs(16));
        assert_eq!(policy.next_delay(30), policy.max_delay);
    }
```

- [ ] **Step 2: 运行确认失败**（`cargo test -p ipkvm-headless recovery::tests::`）

- [ ] **Step 3: 实现**

```rust
use std::time::Duration;

/// 自动恢复策略：指数退避 + 上限；视频只对“从未出帧”重启。
#[derive(Clone, Debug)]
pub struct RecoveryPolicy {
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub healthy_reset_after: Duration,
    pub video_start_timeout: Duration,
    pub tick: Duration,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            healthy_reset_after: Duration::from_secs(30),
            video_start_timeout: Duration::from_secs(5),
            tick: Duration::from_millis(500),
        }
    }
}

impl RecoveryPolicy {
    pub fn next_delay(&self, consecutive_failures: u32) -> Duration {
        let delay = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(consecutive_failures.min(30)));
        delay.min(self.max_delay)
    }
}
```

`run_recovery_loop`（`web/service.rs` 同模块可访问 `ApiState` 私有字段；`recovery` 作为 `web` 子模块可访问父模块私有项）：

```rust
pub async fn run_recovery_loop<I: InputSink + Clone + Send + 'static>(
    api: Arc<super::ApiState<I>>,
    mut shutdown: watch::Receiver<bool>,
    policy: RecoveryPolicy,
) {
    let mut failures: u32 = 0;
    let mut stopped_since: Option<std::time::Instant> = None;
    let mut last_attempt: Option<std::time::Instant> = None;
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(policy.tick) => {}
            _ = shutdown.changed() => return,
        }
        let now = std::time::Instant::now();
        let mut manager = api.manager.lock().await;
        manager.refresh_stats();
        let (state_name, input_offline, last_frame_ns) = match manager.session() {
            Some(session) => {
                let stats = session.stats();
                (
                    super::session_state_name(manager.state()),
                    stats.input_offline.clone(),
                    stats.last_frame_ns,
                )
            }
            None => ("absent".to_string(), None, None),
        };
        match state_name.as_str() {
            "running" => {
                stopped_since = None;
                failures = 0;
                continue;
            }
            "stopped" => {
                let stopped_at = *stopped_since.get_or_insert(now);
                let reason_present = input_offline.is_some();
                let video_never_started = last_frame_ns.is_none()
                    && now.duration_since(stopped_at) >= policy.video_start_timeout;
                if !(reason_present || video_never_started) {
                    continue;
                }
                let delay = policy.next_delay(failures);
                if last_attempt.is_some_and(|t| now.duration_since(t) < delay) {
                    continue;
                }
                // 重建会话（与 api_session restart 同路径）。
                let selection = api.selection.lock().await.clone();
                let Some(selection) = selection else {
                    continue;
                };
                let _ = manager.stop_and_destroy().await;
                api.frame_source.set_current(Arc::new(super::EmptyFrameSource::new()));
                match api.factory.build(&selection) {
                    Ok((frame_source, sink)) => {
                        match super::create_and_start_session(
                            &mut manager,
                            &frame_source,
                            sink,
                            api.gate.clone(),
                        ) {
                            Ok(()) => {
                                api.frame_source.set_current(frame_source);
                                *api.selection.lock().await = Some(selection);
                                failures = 0;
                                stopped_since = None;
                                last_attempt = Some(now);
                            }
                            Err(_) => {
                                failures = failures.saturating_add(1);
                                last_attempt = Some(now);
                            }
                        }
                    }
                    Err(_) => {
                        failures = failures.saturating_add(1);
                        last_attempt = Some(now);
                    }
                }
            }
            _ => {}
        }
    }
}
```

`web/mod.rs` 加 `mod recovery;`；`service.rs` 的 `serve()` 在构建 router 前 spawn：

```rust
        tokio::spawn(recovery::run_recovery_loop(
            Arc::clone(&self.api),
            self.shutdown.clone(),
            recovery::RecoveryPolicy::default(),
        ));
```

- [ ] **Step 4: 运行确认通过**（`cargo test -p ipkvm-headless`）

- [ ] **Step 5: 提交** `git commit -m "feat: auto-recover headless session with exponential backoff"`

---

### Task 4: 断流恢复 DesktopSize 回归测试

**Files:**
- Modify: `crates/ipkvm-headless/tests/rfb_dynamic_resolution.rs`（或新增 fixture）

- [ ] **Step 1: 失败测试**

用 `MockFrameSource` 新建小 fixture：连接客户端 → 发布 4×2 帧 → 停 300ms → 发布 2×4 帧 → 断言收到 DesktopSize (0,0,2,4)。

- [ ] **Step 2: 运行确认失败**（新增测试，先只建断言结构）

- [ ] **Step 3: 实现**：若现有 `ServerFixture` 泛型化困难，新增 `MockServerFixture`（仿照现有 fixture，source 改为 `Arc<dyn FrameSource>`）。

- [ ] **Step 4: 运行确认通过**（`cargo test -p ipkvm-headless --test rfb_dynamic_resolution`）

- [ ] **Step 5: 提交** `git commit -m "test: desktop size announced after video stall and resume"`

---

### Task 5: 文档与收口

- [ ] 更新 `docs/ipkvm-coarse-design.md`：headless 自动恢复策略（退避、视频只重启“从未出帧”）、status 字段。
- [ ] 全量验证（fmt + workspace tests）。
- [ ] 自审：退避不抢串口、视频停滞不重启、状态字段正确、desktop 行为未变。
- [ ] 推送、PR、合并、关闭 #37。

---

## Self-Review

- **覆盖**：status 断流/离线（Task 1/2）、离线原因时间（Task 1）、DesktopSize 断流恢复（Task 4）、自动恢复策略（Task 3）均有任务。
- **类型一致性**：`InputOfflineInfo` 字段在 Task 1/2 一致；`RecoveryPolicy::next_delay` 签名在 Task 3 测试与实现一致。
