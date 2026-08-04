# Headless Relative Mouse Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline execution) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 headless Web 选中相对 profile 后，首次点击视频画布即可进入 Pointer Lock，并保持目标端相对鼠标模式，达到 desktop 的操作体验。

**Architecture:** `MouseProfile` 继续决定 headless sink 的目标端 `MouseMode`。`PointerController` 在相对 profile 下为 noVNC canvas 安装捕获阶段的 `mousedown` 用户手势处理器，调用现有 Pointer Lock 流程；输入泵在实际模式为 Relative 时丢弃 Pointer Lock 建立前的绝对过渡事件，不改变 sink 模式。Pointer Lock 建立后 noVNC 的 `movementX/Y` 仍通过现有 RFB `0x08` 消息进入相对输入路径。

**Tech Stack:** Rust 1.89+, Tokio, `ipkvm-session` RFB input pump, 原生 ES modules, vendored noVNC 1.7.0, Playwright browser verification.

## Global Constraints

- 所有仓库内自写文档使用中文；本计划文件和设计变更使用中文。
- 修改核心逻辑前必须先补充能证明问题的失败测试，并观察测试按预期失败。
- 不改变标准绝对指针路径；只有当前实际模式为 Relative 时忽略绝对过渡事件。
- Pointer Lock 只能由浏览器用户手势触发；绝对模式、失焦、Esc、断开和 Pointer Lock 失败的清理路径保持有效。
- 真实 CH9329、真实浏览器 Pointer Lock 和目标 OS 输入栈属于人工验收例外，必须在最终报告中说明。
- 完成前运行 `cargo fmt --all --check` 和 `cargo test --workspace --all-features`。
- 实现类提交使用英文 conventional commit，并包含 `#168`；PR 描述使用 `Closes #168`。

---

### Task 1: Commit The Design Record

**Files:**
- Modify: `docs/superpowers/specs/2026-08-04-headless-web-ui-design.md:182-185,267-272`
- Modify: `docs/superpowers/specs/2026-08-04-mouse-os-profile-design.md:117-124,197-201`
- Create: `docs/superpowers/plans/2026-08-04-issue168-headless-relative.md`

**Interfaces:**
- Produces the documented behavior contract for Tasks 2-5: relative profile remains target-side Relative, canvas first click requests Pointer Lock, and pre-lock absolute events are ignored.

- [x] **Step 1: Record the behavior contract**

  The design records that selecting a relative profile does not call Pointer Lock outside a user gesture, but the first video-canvas click or the existing relative-mode button does request it. It also records that an absolute transition event cannot downgrade a Relative sink.

- [x] **Step 2: Self-review the design and plan**

  Check for stale wording and placeholders:

  ```powershell
  rg -n "TBD|TODO|待补" docs\superpowers\specs\2026-08-04-headless-web-ui-design.md docs\superpowers\specs\2026-08-04-mouse-os-profile-design.md
  ```

  Expected: no unresolved placeholders; the design text explicitly mentions canvas-first capture.

- [x] **Step 3: Commit the design record**

  ```powershell
  git add docs/superpowers/specs/2026-08-04-headless-web-ui-design.md docs/superpowers/specs/2026-08-04-mouse-os-profile-design.md docs/superpowers/plans/2026-08-04-issue168-headless-relative.md
  git commit -m "docs: define headless relative capture behavior #168"
  ```

### Task 2: Guard Relative Input Against Absolute Transition Events

**Files:**
- Modify: `crates/ipkvm-session/src/rfb_input/mod.rs:35-45`
- Modify: `crates/ipkvm-session/src/rfb_input/pump.rs:480-545`
- Test: `crates/ipkvm-session/src/rfb_input/pump.rs:1580-1630`

**Interfaces:**
- Consumes: `RfbInputPump::mouse_mode`, `RfbServerEvent::Pointer`, `RfbServerEvent::PointerRelative`, and `RfbPointerOutcome`.
- Produces: `RfbPointerOutcome::IgnoredForMouseMode { mode: MouseMode }` for an absolute event received while the pump is already in Relative mode; the mapper state and sink remain unchanged.

- [x] **Step 1: Write the failing regression test**

  Change `ch9329_pointer_events_switch_sink_mode_before_dispatch` to assert that a sink created in `MouseMode::Relative` receives no CH9329 batch for the absolute transition event, still receives the later relative move, and never emits an absolute frame. Assert the first event result is `RfbPointerOutcome::IgnoredForMouseMode { mode: MouseMode::Relative }`.

- [x] **Step 2: Run the focused test and verify the expected failure**

  ```powershell
  cargo test -p ipkvm-session ch9329_pointer_events_switch_sink_mode_before_dispatch -- --exact --nocapture
  ```

  Expected: FAIL because the current pump calls `ensure_mouse_mode(Absolute)` and dispatches an absolute frame.

- [x] **Step 3: Implement the minimal mode guard**

  Add the outcome variant and, after `require_active` in `handle_pointer`, return the ignored outcome when `self.mouse_mode == Some(MouseMode::Relative)`. Do not call `ensure_mouse_mode`, `RfbPointerMapper::handle_pointer`, `release_all`, or `set_mouse_mode` on this branch. Preserve the current absolute-to-relative behavior in `handle_pointer_relative`.

- [x] **Step 4: Run the focused tests and verify they pass**

  ```powershell
  cargo test -p ipkvm-session ch9329_pointer_events_switch_sink_mode_before_dispatch -- --exact --nocapture
  cargo test -p ipkvm-session rfb_input::pump --lib
  ```

  Expected: the regression and existing pump tests pass with zero failures.

- [x] **Step 5: Commit the input guard**

  ```powershell
  git add crates/ipkvm-session/src/rfb_input/mod.rs crates/ipkvm-session/src/rfb_input/pump.rs
  git commit -m "fix: preserve relative mode across headless pointer transition #168"
  ```

### Task 3: Capture On The First Video-Canvas Gesture

**Files:**
- Modify: `crates/ipkvm-headless/web/modules/pointer.js:15-42,45-52`
- Test: `browser-tests/novnc-browser.mjs:542-585`

**Interfaces:**
- Consumes: `PointerController.setRfb`, `PointerController.toggle`, `RFB.prototype.canvas`, and the existing `selectedRelative`, `locked`, and `supported` state.
- Produces: a capture-phase `mousedown` listener attached only to the active noVNC canvas; it invokes `toggle(event)` only for a supported, selected-relative, not-yet-locked controller and removes the listener when the RFB instance changes.

- [x] **Step 1: Write the failing browser regression test**

  Add a browser assertion that constructs a `PointerController` with a real canvas, sets a fake RFB whose canvas records `requestPointerLock` calls, applies `{ mouse_mode: "relative" }`, dispatches a bubbling `mousedown` on the canvas, and asserts that the request list contains `{ unadjustedMovement: true }`. Add a parent listener and assert it is not reached, proving the first click is reserved for capture rather than an absolute noVNC pointer event.

- [x] **Step 2: Run the browser test and verify the expected failure**

  ```powershell
  npm ci --ignore-scripts --prefix browser-tests
  cargo build -p ipkvm-browser-fixture --bin ipkvm-browser-fixture
  $env:IPKVM_BROWSER_FIXTURE = (Resolve-Path target\debug\ipkvm-browser-fixture.exe).Path
  node browser-tests/novnc-browser.mjs
  ```

  Expected: FAIL at the new canvas-gesture assertion because the current controller only listens to the relative-mode button.

- [x] **Step 3: Implement the minimal canvas listener lifecycle**

  Add `onCanvasMouseDown` as an instance handler. In `setRfb`, remove it from the previous canvas, assign the new RFB, and add it to the new canvas with capture enabled. The handler must return for absolute mode, unsupported environments, missing RFB, or an already locked controller; otherwise call `toggle(event)`. Keep the existing button listener and all Pointer Lock cleanup handlers.

- [x] **Step 4: Run the focused browser verification**

  ```powershell
  node browser-tests/novnc-browser.mjs
  ```

  Expected: the new canvas capture assertion and all existing noVNC/browser assertions pass.

- [x] **Step 5: Commit the browser capture**

  ```powershell
  git add crates/ipkvm-headless/web/modules/pointer.js browser-tests/novnc-browser.mjs
  git commit -m "fix: capture headless relative mouse on canvas click #168"
  ```

### Task 4: Add End-to-End Relative Profile Coverage

**Files:**
- Modify: `crates/ipkvm-headless/tests/rfb_pointer.rs:1-180`
- Modify: `crates/ipkvm-headless/tests/rfb_input_pump.rs:1-190`
- Modify: `browser-tests/novnc-browser.mjs:350-470`

**Interfaces:**
- Consumes: the public headless RFB input pump, `FakeCommandQueue`, and browser fixture output.
- Produces: coverage proving a Relative sink ignores a pre-lock absolute event and accepts a relative `0x08` event without `input_offline` behavior.

- [x] **Step 1: Add a headless integration assertion**

  Extend the existing real TCP test helper with a `send_relative_pointer(buttons, dx, dy, wheel)` method that writes `[0x08, buttons, dx_be, dy_be, wheel]`, then assert the fake queue receives the relative CH9329 command after an initial relative-mode setup. Keep the existing absolute path test unchanged.

- [x] **Step 2: Run the focused headless integration tests**

  ```powershell
  cargo test -p ipkvm-headless --test rfb_pointer --test rfb_input_pump
  ```

  Expected: all headless pointer and input-pump integration tests pass.

- [x] **Step 3: Extend the browser scenario**

  After the existing profile-selection path, assert that selecting a relative profile leaves the profile selected, the canvas gesture requests Pointer Lock, and the existing RFB scheduler still emits the `0x08` byte layout. Keep the real fixture’s absolute transition assertion for absolute mode only.

- [x] **Step 4: Commit the end-to-end coverage**

  ```powershell
  git add crates/ipkvm-headless/tests/rfb_pointer.rs crates/ipkvm-headless/tests/rfb_input_pump.rs browser-tests/novnc-browser.mjs
  git commit -m "test: cover headless relative profile end to end #168"
  ```

### Task 5: Full Verification And Handoff

**Files:**
- Inspect: all files changed by this plan and `git status`.
- Update if needed: `HANDOFF.md` only if the current input behavior index is missing the new capture rule.

**Interfaces:**
- Consumes: commits from Tasks 1-4 and Gitea issue #168.
- Produces: verified branch state, commit evidence, and a PR-ready description containing `Closes #168`.

- [x] **Step 1: Run formatting and the required workspace test gate**

  ```powershell
  cargo fmt --all --check
  cargo test --workspace --all-features
  ```

  Expected: both commands exit 0; report any unrelated Windows process-execution failure separately instead of claiming the gate passed.

- [x] **Step 2: Run browser and static checks**

  ```powershell
  node --check browser-tests/novnc-browser.mjs
  node --check crates/ipkvm-headless/web/modules/pointer.js
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-browser.ps1
  ```

  Expected: JavaScript syntax checks and real browser verification exit 0.

- [x] **Step 3: Inspect the final diff and issue state**

  ```powershell
  git diff origin/main...HEAD --check
  git status --short --branch
  git log --oneline origin/main..HEAD
  tea issues 168 --repo kxn/my_ipkvm
  ```

  Confirm `artifacts/` is not staged, all commits include `#168`, and issue #168 remains open until the PR is merged.

- [x] **Step 4: Prepare the PR handoff**

  PR description must include:

  ```text
  Closes #168

  Summary:
  - capture relative mouse on the first video-canvas gesture;
  - preserve the target-side Relative mode across pre-lock absolute events;
  - add Rust and browser regression coverage.

  Tests:
  - cargo fmt --all --check
  - cargo test --workspace --all-features
  - scripts/verify-browser.ps1

  Documentation: updated the headless Web and mouse profile design records.
  Manual exception: real CH9329 and target OS input behavior require hardware verification.
  ```
