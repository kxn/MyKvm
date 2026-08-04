# Headless 与 Desktop crate 边界收敛 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` or `executing-plans` to implement this plan task-by-task. 本次按用户要求在同一工作树连续执行全部任务，不在任务之间暂停并重新询问。

**Goal:** 将 headless、browser fixture、demo、device inventory、desktop core 和 iced production adapter 拆成可验证的依赖边界，同时保持 CLI、Web、profile、RFB/input 和真实硬件行为不变。

**Architecture:** 新增无硬件 `ipkvm-device` 纯 provider 契约和 `ipkvm-desktop-core`，让 `ipkvm-session` 只负责会话生命周期；把正式 headless、demo 和 browser fixture 放到独立 package；保留 `ipkvm-desktop` 作为真实硬件 production adapter，iced 通过 core 和 adapter 组装正式应用。

**Tech Stack:** Rust 1.89、edition 2024、Cargo workspace resolver 3、Tokio 1.53、axum 0.8、iced 0.14、serialport 4.9、DirectShow/nokhwa camera backend、Gitea tea。

## Global Constraints

- 仓库自写文档使用中文，代码标识符、命令、路径和第三方名称保留原文。
- 不回滚用户或其他协作者的未提交改动；本 worktree 从 `origin/main` 独立创建。
- 每个实现边界先补会失败的测试，再实现，再运行针对性测试。
- 不修改 Web API、CLI 参数、binary 名、profile TOML、browser fixture stdout/stdin 和 session rollback 语义。
- `ipkvm-session` 默认不得依赖 `serialport` 或真实 camera backend。
- `ipkvm-headless` library 默认不得依赖 mock、serial 或真实 camera backend。
- `ipkvm-desktop-core` 不得依赖 iced、eframe/egui、serialport 或真实 camera backend。
- 实现完成前运行 `cargo fmt --all --check`、`cargo test --workspace --all-features`、clippy、doc 和结构门禁。
- 实现类 PR 使用英文 conventional commit，PR 描述使用中文并包含 `Closes #159`、测试证据、文档影响和人工验证例外。

## Task 1: Add Pure Device Inventory Contract

**Files:**
- Create: `crates/ipkvm-device/Cargo.toml`
- Create: `crates/ipkvm-device/src/lib.rs`
- Modify: `Cargo.toml`
- Test: `crates/ipkvm-device/src/lib.rs`

**Interfaces:**
- Produces `VideoDevice`, `SerialDevice`, `DeviceProviderError`, `DeviceInventoryProvider`, `FakeDeviceInventoryProvider` and feature-gated `ProductionDeviceInventoryProvider`.
- Consumes only standard library in the default feature set; production feature consumes `ipkvm-video` camera API and `serialport`.

- [x] Write tests for fake inventory success and independent video/serial errors.
- [x] Run `cargo test -p ipkvm-device`; expected initial failure because the package and types do not exist.
- [x] Add pure types and provider trait, then add `platform` implementation with the current display-name mapping rules from `ipkvm-session/src/devices.rs`.
- [x] Run `cargo test -p ipkvm-device --all-features`; expected pass.
- [x] Run `cargo tree -p ipkvm-device`; default output must not contain `serialport` or camera backend.

## Task 2: Remove Mock Leakage and Split Video Features

**Files:**
- Modify: `crates/ipkvm-video/Cargo.toml`, `crates/ipkvm-video/src/lib.rs`
- Modify: `crates/ipkvm-core/Cargo.toml`, `crates/ipkvm-core/src/lib.rs`
- Modify: `crates/ipkvm-headless/src/frame_source.rs`
- Test: `crates/ipkvm-headless/src/frame_source.rs`

**Interfaces:**
- Produces `ipkvm-video` features `camera`, `assets`, `test-support`, with `mf` and `mock` compatibility aliases.
- Produces `ipkvm-core/test-support` for `FakeCommandQueue`, with `mock` compatibility alias.
- `EmptyFrameSource` constructs its own watch channel and has no dependency on `MockFrameSource`.

- [x] Add a default-feature test/compile target for `EmptyFrameSource`, then run `cargo check -p ipkvm-headless --lib`; expected failure before the implementation because current source imports `ipkvm_video::mock`.
- [x] Move feature gates and implement the standalone empty source without changing `FrameSource` results.
- [x] Run `cargo check -p ipkvm-headless --lib` and `cargo test -p ipkvm-video --all-features`; expected pass.

## Task 3: Decouple Session from Device Enumeration

**Files:**
- Modify: `crates/ipkvm-session/Cargo.toml`, `crates/ipkvm-session/src/lib.rs`
- Delete: `crates/ipkvm-session/src/devices.rs`
- Modify: `crates/ipkvm-session/src/console_session.rs`, `session_manager.rs` only where test feature imports need updating
- Test: session default-feature compile and existing lifecycle tests

**Interfaces:**
- `ipkvm-session` retains `SessionManager`, `ConsoleSession`, RFB connection/input APIs and `FrameSource` dependency.
- Device inventory types and enumeration move to `ipkvm-device`; no session API calls `list_*_devices`.

- [x] Add a compile gate test/script that checks default `ipkvm-session` dependency tree excludes `serialport` and camera backend; run it and capture the failing baseline.
- [x] Remove devices module and the `serial` feature/optional serialport dependency; change normal video dependency to no backend feature.
- [x] Enable `ipkvm-video/test-support` only in dev-dependencies for session tests.
- [x] Run `cargo check -p ipkvm-session --lib` and targeted lifecycle tests; expected pass.

## Task 4: Inject Device Provider into Headless Web

**Files:**
- Modify: `crates/ipkvm-headless/Cargo.toml`, `src/web/service.rs`, `src/web/mod.rs`
- Modify: `crates/ipkvm-headless/src/web/recovery.rs` and all headless tests constructing `HeadlessWebService`
- Test: `crates/ipkvm-headless/src/web/service.rs`, `tests/web_http.rs`

**Interfaces:**
- `HeadlessWebService::new` receives `Arc<dyn DeviceInventoryProvider>`.
- `/api/devices` maps provider records to the existing `DeviceDto`; provider errors map to existing `503` JSON.
- `SessionFactory` and `SessionSelection` signatures remain unchanged.

- [x] Add fake-provider API tests for deterministic video/serial JSON and video/serial enumeration failure; run targeted tests and verify red before changing service code.
- [x] Store provider in `ApiState`, use it in `api_devices`, migrate all constructors, and keep fake fixture provider separate from session factory.
- [x] Run `cargo test -p ipkvm-headless --lib --test web_http`; expected pass without hardware feature.

## Task 5: Split Headless App, Demo and Browser Fixture Packages

**Files:**
- Create: `crates/ipkvm-headless-app/Cargo.toml`, `src/main.rs`
- Create: `crates/ipkvm-headless-demo/Cargo.toml`, `src/main.rs`
- Create: `crates/ipkvm-browser-fixture/Cargo.toml`, `src/main.rs`
- Modify: `Cargo.toml`, `README.md`, `scripts/verify-browser.ps1`, `scripts/verify-browser.sh`
- Move: existing headless `src/main.rs`, `src/bin/ipkvm-demo.rs`, `src/bin/ipkvm-browser-fixture.rs` and target-specific integration tests to their package owners
- Test: app process, demo build and browser fixture protocol tests

**Interfaces:**
- Binary names remain `ipkvm-headless`, `ipkvm-demo`, `ipkvm-browser-fixture`.
- `ipkvm-headless-app` enables platform provider, real serial and camera/assets features.
- `ipkvm-headless-demo` enables assets and test-support but no platform provider.
- `ipkvm-browser-fixture` enables test-support and fake provider but no serialport/camera backend.

- [x] Add package metadata and dependency-tree assertions before removing old targets; run them and verify the old graph fails the new assertions.
- [x] Move sources/tests, update `CARGO_BIN_EXE_ipkvm-headless` consumers and package-specific cargo commands while preserving stdout/stdin.
- [x] Run `cargo build -p ipkvm-headless-app --bin ipkvm-headless`, `cargo build -p ipkvm-headless-demo --bin ipkvm-demo`, `cargo build -p ipkvm-browser-fixture --bin ipkvm-browser-fixture`; expected pass.
- [x] Run `cargo tree -p ipkvm-browser-fixture --invert serialport` and camera-backend checks; expected no output.

## Task 6: Extract UI-Free Desktop Core

**Files:**
- Create: `crates/ipkvm-desktop-core/Cargo.toml`, `src/lib.rs`, `src/config.rs`, `src/frame.rs`, `src/probe.rs`, `src/render.rs`, `src/session.rs`, `src/state.rs`
- Modify: `Cargo.toml`
- Modify: `crates/ipkvm-desktop/src/lib.rs`, move production adapter code from `probe.rs` and `session.rs`
- Test: migrated core unit tests and production adapter compile tests

**Interfaces:**
- Core exports the current config/state/probe/session/frame types with the same field semantics.
- Production adapter exports `ProductionProbeBackend`, `production_parts`, `ProductionSessionFactory`, `ProductionDesktopSessionController` and compatibility re-exports.
- Core has no concrete camera/serial open function; production adapter remains the only real-hardware assembly layer.

- [x] Add a dependency gate for `ipkvm-desktop-core` and run it red against the not-yet-created package.
- [x] Move pure files and generic controller tests, then move production implementations to the adapter and preserve old re-export paths.
- [x] Run `cargo test -p ipkvm-desktop-core` and `cargo test -p ipkvm-desktop`; expected pass.

## Task 7: Rewire iced and Preserve Product Behavior

**Files:**
- Modify: `crates/ipkvm-desktop-iced/Cargo.toml`, `src/app.rs`, `src/connect.rs`, `src/profile.rs`, `src/video.rs`
- Modify: `crates/ipkvm-desktop/src/clipboard.rs` only for moved pure frame types
- Test: existing iced unit/integration tests and desktop release compile

**Interfaces:**
- iced uses `ipkvm-desktop-core` for pure types and `ipkvm-desktop` only for real production adapter/clipboard.
- No UI behavior, keyboard mapping, window behavior, profile behavior, or session lifecycle behavior changes.

- [x] Add a compile-time import/dependency test that core is the shared path; run it red before import changes.
- [x] Update dependency and import paths, keep all real hardware and Windows platform modules in the final product target.
- [x] Run `cargo test -p ipkvm-desktop-iced --all-features` and existing iced pixel/interaction tests; expected pass.

## Task 8: Add Structural Gates and Update Long-Term Docs

**Files:**
- Create: `scripts/test-crate-boundaries.ps1`, `scripts/test-crate-boundaries.sh`
- Modify: `README.md`, `HANDOFF.md`, `docs/ipkvm-coarse-design.md`, iced/headless specs, `docs/dependency-license-policy.md`
- Modify: #157 build/measurement notes where targets or commands changed
- Test: structural scripts and documentation command snippets

**Interfaces:**
- Gates inspect Cargo metadata and target-specific dependency trees rather than relying on source text alone.
- Documentation is Chinese and states that #79 egui retirement is complete, while #159 owns provider/package/core boundaries.

- [x] Add failing assertions for package names, required features, core dependency exclusions, fixture provider exclusions and no egui/spike; run the script and verify red.
- [x] Implement the gate and update all commands, then run both PowerShell and shell variants where available. PowerShell 版本通过；WSL 版本因环境没有 `cargo` 无法运行。
- [x] Run the full verification set from the design document.

## Task 9: Final Integration and Closeout

**Files:**
- Modify: only files required by verification findings, without unrelated cleanup
- Test: all commands in the design document, Windows release builds where available

- [x] Inspect `git diff`, `git status`, Cargo metadata and dependency trees for unintended files or feature leaks.
- [x] Run `cargo fmt --all --check`, `cargo test --workspace --all-features`, clippy, doc, M5 gate and crate-boundary gate; record exact evidence.
- [ ] Commit with `refactor: converge headless and desktop crate boundaries (#159)`.
- [ ] Push `codex/issue159-design`, create PR into `main` with `Closes #159`, change summary, design basis, tests, docs and hardware manual exception.
- [ ] Read PR and issue back through `tea`, confirm merge/close state; if PR merge is not performed by the repository flow, use the required closeout command only after confirming the correct issue state.
