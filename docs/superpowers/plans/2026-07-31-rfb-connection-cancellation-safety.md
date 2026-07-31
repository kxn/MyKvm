# RFB 连接取消安全实施计划

> **供自动化协作者使用：** 按任务执行测试先行、独立提交和独立审查；任何红灯都先确认
> 失败原因与本任务一致，再编写生产实现。

**目标：** 修复已激活 RFB 连接在异步取消时错误释放全局单连接闸门的问题，并补齐共享
收尾、底层 WebSocket 错误链和文档事实。

**架构：** `RfbConnectionGate` 返回可自动释放的预约；共享 owner 在连接驱动开始前把
预约同步升级为关闭失败的租约；TCP 与 WebSocket 都通过共享 `finalize_connection`
在 `Disconnected` 成功入队后同步释放租约。异常析构会毒化并关闭闸门。

**关联 issue：** `#15`

## 全局约束

- 所有说明文档使用中文。
- 不增加公开 API，不改变 RFB、TCP 或 WebSocket 线级行为。
- 不使用固定延时验证取消和反压。
- 不依赖 Gitea runner，全部测试在本机运行。
- 已激活连接的异常取消必须把闸门置为可观察的 `Poisoned`，禁止用 RAII 自动释放掩盖
  未完成的输入清理。

---

### 任务 1：用两阶段令牌和共享收尾固定取消语义

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_connection/gate.rs`
- 新建：`crates/ipkvm-headless/src/rfb_connection/finalize.rs`
- 修改：`crates/ipkvm-headless/src/rfb_connection/mod.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/server.rs`
- 修改：`crates/ipkvm-headless/src/rfb_ws/service.rs`

- [ ] 先在 `gate.rs` 写预约析构、租约显式释放、容量循环守恒和租约异常析构四组测试。
- [ ] 运行 `cargo test -p ipkvm-headless rfb_connection::gate::tests`，确认新接口尚不存在或
  旧实现错误开放闸门。
- [ ] 实现 `RfbConnectionReservation`、不可克隆的 `RfbConnectionLease`、同步
  `activate/release` 和 `Poisoned` 状态；租约异常析构必须关闭 semaphore 并唤醒等待者。
- [ ] 新建共享 owner 和收尾函数，owner 必须在第一个 `.await` 前激活预约，并返回同时
  持有 `ConnectionEnd` 与租约的不可克隆完成值。
- [ ] 先写满事件通道下的反压成功与 future 取消测试，并补充无断开原因、接收端已关闭、
  实际 owner 在 `Connected` 后被中止的测试。
- [ ] 运行共享收尾测试并确认旧的 TCP 私有收尾无法满足测试。
- [ ] 把 TCP 和 WebSocket 都切换到共享 owner 与共享收尾；删除 TCP 私有收尾实现及其
  重复测试。WebSocket 把 `Poisoned` 映射为空 `503`，TCP 返回类型化服务端错误。
- [ ] 运行：

```powershell
cargo test -p ipkvm-headless rfb_connection::
cargo test -p ipkvm-headless --test rfb_tcp
cargo test -p ipkvm-headless --test rfb_websocket
cargo test -p ipkvm-headless --test rfb_transport_exclusion
```

- [ ] 提交并独立审查取消、事件顺序、许可释放和 TCP/WS 一致性。

---

### 任务 2：保留 WebSocket 根因并修正文档事实

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_connection/transport.rs`
- 修改：`crates/ipkvm-headless/src/rfb_connection/driver.rs`
- 修改：`crates/ipkvm-headless/src/rfb_ws/transport.rs`
- 修改：`crates/ipkvm-headless/src/rfb_connection/mod.rs`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-websocket-transport-design.md`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-connection-cancellation-safety-design.md`
- 修改：`docs/references/README.md`

- [ ] 先写测试遍历 `std::error::Error::source()`，证明私有 `RfbTransportError` 保留
  底层 WebSocket 错误，且 `ConnectionEnd` 仍映射为公共
  `RfbDisconnectReason::WebSocket`。
- [ ] 把底层错误装箱保存在私有 WebSocket 错误变体，接收与发送路径都保留来源。
- [ ] 将 BGRA 错误文案改为传输无关描述并更新断言。
- [ ] 修正前置设计中的两阶段生命周期、关闭失败策略、测试职责和依赖树事实。
- [ ] 运行：

```powershell
cargo test -p ipkvm-headless
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] 提交并独立审查错误边界、文档事实和回归风险。

---

### 任务 3：整分支重新验收

- [ ] 对 issue 起点到当前 HEAD 生成完整 diff，执行整分支代码审查。
- [ ] 修复全部阻断项并重新审查，直至结论为可合并。
- [ ] 本机运行：

```powershell
.\scripts\verify.ps1
```

- [ ] 核对工作树、提交范围、中文文档、依赖锁文件和 issue 验收项。
- [ ] 推送功能分支，创建中文 PR，合并后在主分支再次运行全量验证。
