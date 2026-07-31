# RFB 输入事件泵实施计划

**目标：** 实现单活动 RFB 控制者事件泵，把 TCP 键盘和指针事件送入唯一 `InputSink`，并在断线或事件源关闭时可靠释放全部输入状态。

**架构：** `ipkvm-rfb` 固化客户端输入坐标时期；`rfb_tcp` 只传播带尺寸的事件；`rfb_input::pump` 持有 sink、两个 mapper 和当前控制者。客户端输入拒绝通过通知继续运行，sink 或生命周期错误保留原事件并停止循环。

**技术栈：** Rust 1.89、edition 2024、Tokio 1.53.1 有界 `mpsc`、thiserror 2、现有 RFB 3.8 core、`FakeCommandQueue`、真实回环 TCP。

---

## 0. 实施约束

- [x] 设计和自审先于实施。
- [x] 关联 Gitea issue `#11`。
- [x] 不修改主工作区中用户未提交的 `AGENTS.md`。
- [x] 所有新增和修改文档使用中文。
- [x] 不新增外部依赖。
- [x] 每个实现任务先观察红灯，再写最小完整实现。
- [x] 不用最新视频尺寸补算 RFB 指针坐标空间。
- [x] 不把多查看者和 session 级控制权仲裁塞进本阶段。
- [x] 不在 `Drop` 中执行无法上报错误的释放。

---

## 任务 1：给 RFB 指针事件固定输入坐标时期

**文件：**

- 修改：`crates/ipkvm-rfb/src/protocol/client.rs`
- 修改：`crates/ipkvm-rfb/src/connection.rs`

### 步骤

- [x] **步骤 1：写入初始尺寸红灯测试**

在 `connection.rs` 单元测试中完成握手后发送：

```text
PointerEvent(mask=0, x=10, y=20)
```

预期 `RfbEvent::Pointer` 同时包含初始 `RfbSize(640, 480)`。

测试必须先因枚举缺少 `framebuffer_size` 字段而编译失败。

- [x] **步骤 2：写入尺寸切换红灯测试**

1. 初始尺寸 `640x480`。
2. 协商 `DesktopSize`。
3. 成功排队 `800x600` 的尺寸更新。
4. 再发送指针事件。
5. 预期事件尺寸为 `800x600`。

- [x] **步骤 3：写入跨半包时期红灯测试**

1. 在旧尺寸下向 decoder 写入一个未完成的 6 字节指针消息前半段。
2. 成功排队新尺寸。
3. 写入指针消息剩余字节，并在同一次 push 中追加另一条完整指针消息。
4. 两条事件均使用旧尺寸。
5. 下一次单独 push 的指针使用新尺寸。

该测试固定保守时期边界，防止消息在 TCP 分片中途切换尺寸。

- [x] **步骤 4：公开 decoder 空闲查询**

把仅测试可用的 `buffered_len()` 保持私有测试辅助，同时增加生产内部方法：

```rust
pub(crate) fn is_idle(&self) -> bool
```

它只返回 `buffer.is_empty()`，不暴露缓冲内容。

- [x] **步骤 5：实现三段尺寸状态**

`RfbConnectionCore` 增加：

```rust
input_coordinate_size: RfbSize,
pending_input_size: Option<RfbSize>,
```

初始化：

- `announced_size = initial_size`
- `input_coordinate_size = initial_size`
- `pending_input_size = None`

`queue_framebuffer_update()` 成功排队 `DesktopSize` 后：

- 先提交 `announced_size`。
- decoder 空闲时立即提交 `input_coordinate_size`。
- decoder 有半包时只更新 `pending_input_size`。
- 输出排队失败时三个尺寸状态都不改变。

`push_normal_input()`：

- 本次 push 开始时复制 `input_coordinate_size`。
- 本次产生的所有 Pointer 事件使用该副本。
- 完成转换且 decoder 空闲后，再提交 `pending_input_size`。
- 协议失败时不需要为了后续输入提交新时期。

- [x] **步骤 6：补齐失败原子性测试**

扩展现有 `failed_desktop_size_queue_does_not_commit_new_size`：

- 输出容量失败后，后续指针仍携带旧尺寸。
- 再次成功排队尺寸后，后续指针才携带新尺寸。

- [x] **步骤 7：运行协议 core 验证**

```powershell
cargo test -p ipkvm-rfb
cargo clippy -p ipkvm-rfb --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

- [x] **步骤 8：提交**

```powershell
git add crates/ipkvm-rfb
git commit -m "feat: preserve RFB pointer coordinate epochs (#11)"
```

---

## 任务 2：把指针尺寸传播到 RFB TCP 事件

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_tcp/mod.rs`
- 修改：`crates/ipkvm-headless/src/rfb_tcp/connection.rs`
- 修改：`crates/ipkvm-headless/tests/rfb_tcp.rs`

### 步骤

- [x] **步骤 1：写入 TCP 事件红灯测试**

扩展现有输入顺序测试：

- Pointer 事件包含握手时公告尺寸。
- 键盘、Pointer、剪贴板的相对顺序不变。

- [x] **步骤 2：写入动态尺寸流水测试**

使用真实回环 TCP：

1. 初始帧 `640x480`。
2. 客户端协商 `DesktopSize`。
3. 视频源切换到 `800x600`。
4. 客户端在一个 write 中发送更新请求和旧尺寸 Pointer。
5. 事件中的 Pointer 尺寸仍为 `640x480`。
6. 读取服务端 `DesktopSize` 后再发 Pointer。
7. 新事件尺寸为 `800x600`。

- [x] **步骤 3：扩展事件枚举**

```rust
RfbTcpEvent::Pointer {
    client_id,
    button_mask,
    x,
    y,
    framebuffer_size: RfbSize,
}
```

`connection.rs` 只复制 `RfbEvent` 已携带的字段，不读取 `FrameSource`。

- [x] **步骤 4：修正所有模式匹配和测试构造**

使用显式字段或 `..`，但涉及坐标语义的测试必须断言尺寸，不能全部用 `..` 隐藏回归。

- [x] **步骤 5：运行 headless TCP 验证**

```powershell
cargo test -p ipkvm-headless rfb_tcp
cargo test -p ipkvm-headless --test rfb_tcp
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

- [x] **步骤 6：提交**

```powershell
git add crates/ipkvm-headless
git commit -m "feat: carry RFB pointer framebuffer size (#11)"
```

---

## 任务 3：定义事件泵公共契约

**文件：**

- 新增：`crates/ipkvm-headless/src/rfb_input/pump.rs`
- 修改：`crates/ipkvm-headless/src/rfb_input/mod.rs`

### 步骤

- [x] **步骤 1：写入公共 API 编译红灯**

测试导入：

```rust
RfbControllerReleaseReason
RfbInputError
RfbInputEventError
RfbInputEventKind
RfbInputLifecycleError
RfbInputNotice
RfbInputOperation
RfbInputPump
RfbInputRunError
RfbKeyboardRejection
```

测试必须先因类型不存在而失败。

- [x] **步骤 2：定义通知类型**

`RfbInputNotice`：

- `ControllerAcquired`
- `Keyboard`
- `KeyboardRejected`
- `Pointer`
- `CutTextIgnored`
- `ContinuousUpdatesIgnored`
- `PreHandshakeDisconnected`
- `ControllerReleased`

所有通知保存判断所需的类型化字段，不保存剪贴板正文。

- [x] **步骤 3：定义错误类型**

`RfbInputLifecycleError`：

- 已有控制者时再次连接。
- 没有控制者时收到连接级事件。
- 收到非当前控制者事件。
- 同一客户端编号的断线地址变化。

`RfbInputError`：

- 生命周期错误。
- 带客户端编号、操作类型和 `InputError` 的 sink 错误。

`RfbInputEventError`：

- 保存原始事件。
- 保存 `RfbInputError`。
- 提供 `event()`、`error()` 和 `into_parts()`。

`RfbInputRunError`：

- 当前事件失败。
- 事件源关闭时释放失败。

- [x] **步骤 4：建立最小 pump 外壳**

```rust
pub struct RfbInputPump<S> {
    sink: S,
    active: Option<ActiveController>,
    keyboard: RfbKeyboardMapper,
    pointer: RfbPointerMapper,
}
```

提供：

- `new`
- `active_client`
- 只读 `sink`

不提供 `sink_mut`、无条件 `into_sink` 或析构释放。

- [x] **步骤 5：运行公共契约测试并提交**

```powershell
cargo test -p ipkvm-headless --lib
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
git diff --check
git add crates/ipkvm-headless
git commit -m "feat: define RFB input pump contracts (#11)"
```

---

## 任务 4：按 TDD 实现逐事件控制者状态机

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_input/pump.rs`

### 步骤

- [x] **步骤 1：写入连接和合法输入红灯测试**

纯内存 `RecordingSink` 验证：

- `Connected` 获得控制权。
- 键盘事件调用键盘 mapper。
- Pointer 使用事件携带的尺寸调用指针 mapper。
- 每个事件产生准确通知。

- [x] **步骤 2：实现活动控制者校验**

内部 `require_active(client_id, event_kind)`：

- 没有活动控制者返回 `NoActiveController`。
- 编号不同返回 `WrongController`。
- 成功返回当前控制者只读信息。

`Connected` 在已有控制者时返回 `ControllerAlreadyActive`。

- [x] **步骤 3：写入可继续拒绝红灯测试**

- 不支持 keysym 返回 `KeyboardRejected::UnsupportedKeysym`。
- Shift 冲突返回 `KeyboardRejected::ConflictingShiftRequirements`。
- 后续合法键仍被处理。
- `CutText` 返回只含字节数的忽略通知。
- `ContinuousUpdates` 返回类型化忽略通知。

- [x] **步骤 4：实现键盘错误分类和非输入通知**

只把 `RfbKeyboardError::Input` 转成致命 sink 错误。其余键盘映射错误转通知。

- [x] **步骤 5：写入失败事件返还红灯测试**

- sink 拒绝键盘批次。
- `handle_event` 返回的错误保留完全相等的原事件。
- pump 仍保留当前控制者。
- 取回事件重试后成功。
- 指针 sink 失败执行同样测试。

- [x] **步骤 6：实现失败封装**

`handle_event(event)` 内部借用事件处理；失败时把取得所有权的原事件和错误一起返回。

- [x] **步骤 7：写入生命周期异常红灯测试**

- 未连接先输入。
- 活动期间重复 `Connected`。
- 其他 client id 输入。
- 其他 client id 断线。
- 相同 client id 但地址变化。

确认所有错误都不调用 sink、不改变 mapper 和控制者。

- [x] **步骤 8：实现生命周期错误**

严格遵守当前顺序 server 契约，不做隐式抢占或自动切换。

- [x] **步骤 9：运行单元验证并提交**

```powershell
cargo test -p ipkvm-headless rfb_input::pump
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git add crates/ipkvm-headless
git commit -m "feat: route RFB controller input events (#11)"
```

---

## 任务 5：实现断线释放、事件源关闭和重试

**文件：**

- 修改：`crates/ipkvm-headless/src/rfb_input/pump.rs`

### 步骤

- [x] **步骤 1：写入握手前断线红灯测试**

没有 `Connected` 的 `Disconnected`：

- 返回 `PreHandshakeDisconnected`。
- 不调用 `release_all()`。
- 保持无活动控制者。

- [x] **步骤 2：写入正常断线释放红灯测试**

1. 连接。
2. 按下键盘键和鼠标按钮。
3. 处理匹配的 `Disconnected`。
4. sink 恰好释放一次。
5. 通知包含断线原因。
6. pump 变为空闲。
7. 第二个控制者连接后，相同键和按钮会再次产生输入，证明 mapper 已重置。

- [x] **步骤 3：写入释放失败重试红灯测试**

- 第一次 `release_all` 返回错误。
- 错误保留原 `Disconnected`。
- 活动控制者和 mapper 不清空。
- 取回并重试同一事件后释放成功。
- 新控制者随后可以正常连接。

- [x] **步骤 4：实现统一释放函数**

内部所有释放入口共用一个函数：

1. 读取活动控制者副本。
2. 调用 sink。
3. 成功后清空控制者。
4. 重建两个 mapper。
5. 返回 `ControllerReleased`。

公开 `release_active()` 使用 `Explicit` 原因。

- [x] **步骤 5：写入通道关闭红灯测试**

Tokio 有界通道：

- sender 发送连接和输入后关闭。
- `run` 排空事件。
- 通道 `None` 时释放活动控制者。
- 观察回调按顺序收到通知。
- 没有活动控制者时关闭通道不调用释放。

- [x] **步骤 6：写入通道关闭释放失败测试**

- 关闭时释放失败返回 `SourceClosedRelease`。
- pump 保留活动控制者。
- 调用 `release_active()` 可以重试。

- [x] **步骤 7：实现异步运行循环**

只使用：

```rust
while let Some(event) = receiver.recv().await
```

不额外 spawn，不建立第二个队列，不吞掉错误。观察回调只接收成功通知。

- [x] **步骤 8：运行验证并提交**

```powershell
cargo test -p ipkvm-headless rfb_input::pump
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git add crates/ipkvm-headless
git commit -m "feat: release RFB input on lifecycle end (#11)"
```

---

## 任务 6：使用真实 CH9329 sink 验证输入和释放原子性

**文件：**

- 新增：`crates/ipkvm-headless/tests/rfb_input_pump.rs`

### 步骤

- [x] **步骤 1：建立真实 sink 集成测试**

组合：

```text
RfbInputPump
  -> Ch9329InputSink
     -> FakeCommandQueue
```

事件序列：

1. `Connected`
2. 键盘 `A` 按下
3. Pointer 移动并左键按下
4. `Disconnected`

断言：

- 键盘和指针命令按事件顺序入队。
- 最后一个 `release_all` 批次同时含全零键盘和鼠标释放报告。
- pump 最终无活动控制者。

- [x] **步骤 2：验证真实 sink 释放失败**

在断线前配置 `FakeCommandQueue::fail_next`：

- 第一次断线处理返回带原事件的队列错误。
- 队列没有接受释放批次。
- 重试后只增加一个完整释放批次。
- sink 和 pump 的状态都能继续服务下一个控制者。

- [x] **步骤 3：验证动态尺寸指针**

构造携带新 `RfbSize` 的 Pointer，检查 CH9329 报告坐标按该尺寸换算，不受任何模拟视频源最新尺寸影响。

- [x] **步骤 4：运行集成验证并提交**

```powershell
cargo test -p ipkvm-headless --test rfb_input_pump
cargo test -p ipkvm-headless
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git add crates/ipkvm-headless/tests/rfb_input_pump.rs
git commit -m "test: cover RFB input pump rollback (#11)"
```

---

## 任务 7：完成真实回环 TCP 输入闭环

**文件：**

- 修改：`crates/ipkvm-headless/tests/rfb_input_pump.rs`

### 步骤

- [x] **步骤 1：写入端到端红灯测试**

启动：

- `MockFrameSource` 发布合法 BGRA8888 帧。
- 临时 `TcpListener`。
- `RfbTcpServer`。
- 容量较小的事件通道。
- `RfbInputPump<Ch9329InputSink<FakeCommandQueue>>`。

- [x] **步骤 2：使用独立最小客户端驱动**

测试客户端自己写 RFB 字节，不调用 server 编码器：

1. 完成 RFB 3.8 None 握手。
2. 发送 `A` 按下。
3. 发送左键 Pointer。
4. 关闭 socket。
5. 等待 server 产生断线。
6. 发送 server shutdown。

- [x] **步骤 3：断言闭环结果**

- 事件泵观察到连接、键盘、指针、释放。
- Fake 队列最终含输入和释放批次。
- server 返回 `Ok(())`。
- sender 丢弃后事件泵正常返回。
- 没有手工调用 mapper 或 `release_all()` 绕过事件泵。

- [x] **步骤 4：运行 headless 全量验证并提交**

```powershell
cargo test -p ipkvm-headless --test rfb_input_pump
cargo test -p ipkvm-headless
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
git add crates/ipkvm-headless/tests/rfb_input_pump.rs
git commit -m "test: close RFB TCP input lifecycle loop (#11)"
```

---

## 任务 8：回写中文文档并完成验收

**文件：**

- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`
- 修改：`docs/superpowers/specs/2026-07-31-rfb-input-pump-design.md`
- 修改：`docs/superpowers/plans/2026-07-31-rfb-input-pump.md`

### 步骤

- [x] **步骤 1：核对测试清单**

```powershell
cargo test -p ipkvm-rfb -- --list
cargo test -p ipkvm-headless -- --list
```

必须能定位：

- 初始、切换和跨半包指针坐标时期。
- TCP 事件尺寸传播。
- 客户端输入拒绝后继续。
- 错误原事件返还和重试。
- 断线释放失败回滚。
- 事件源关闭释放。
- 真实 CH9329 sink。
- 真实回环 TCP 完整闭环。

- [x] **步骤 2：更新 README**

当前状态增加：

- RFB 输入事件泵。
- 单活动 RFB 控制者生命周期。
- 断线和事件源关闭 `release_all()`。
- 当前仍是库闭环，尚未接真实串口和可执行后台进程。

- [x] **步骤 3：更新粗粒度设计**

阶段 0：

- 把事件泵、单 RFB 控制者和断线释放加入已完成。
- 从待完成移除对应项目。
- 保留普通 VNC 客户端兼容性、WebSocket/noVNC 和许可证审计。

输入映射章节补充：

- 指针事件固化已公告坐标时期。
- 不支持键值不会终止事件泵。
- sink 失败保留原事件和软件状态。
- 当前控制者只覆盖顺序单连接 RFB，不覆盖未来多入口 session 仲裁。

- [x] **步骤 4：回写专项设计状态**

- 状态改为“已实施并通过本地自动化验证”。
- 所有实际步骤改为 `[x]`。
- 扫描 `TBD`、`TODO`、临时说明和中英文混杂段落。

- [x] **步骤 5：运行完整本地验证**

```powershell
.\scripts\verify.ps1
```

必须通过：

- UTF-8 无 BOM。
- Rust 格式。
- 全工作区全 feature 测试。
- Clippy `-D warnings`。
- Rust 文档 `-D warnings`。
- 工作区和暂存区 `git diff --check`。

- [x] **步骤 6：提交**

```powershell
git add README.md docs
git commit -m "docs: record RFB input pump completion (#11)"
```

- [ ] **步骤 7：最终自审**

确认：

- 没有修改用户未提交的 `AGENTS.md`。
- 没有新增外部依赖。
- RFB core 没有 Tokio、headless 或 CH9329 依赖。
- TCP 层没有读取最新视频尺寸补指针事件。
- pump 同时只有一个活动控制者。
- 断线和事件源关闭均释放。
- 释放失败不清空控制者或 mapper。
- 可继续输入拒绝不会终止循环。
- 致命错误保留原事件。
- 没有在析构中执行释放。

```powershell
git status --short
git diff --check main...HEAD
git diff --stat main...HEAD
git log --oneline main..HEAD
```
