# Headless 光标语义与测试补强设计

## 背景

当前 headless 网页控制台使用 vendored noVNC 处理 RFB 画面和输入。noVNC
默认把服务端发送的 CursorShape 转成 `canvas.style.cursor`，这会替换浏览器
本机光标。当前工作区的未提交改动曾尝试按绝对/相对模式分别控制该行为，
但“远端光标是否由网页客户端绘制”并不是输入模式的一部分，容易造成用户
对 host 光标和 remote OS 光标的误解。

## 目标

1. headless 网页客户端不渲染 noVNC 的远端 CursorShape。
2. 绝对模式保留浏览器系统光标，继续发送绝对坐标。
3. 相对模式由 Pointer Lock 隐藏浏览器系统光标，继续发送相对输入；网页端
   不额外绘制任何远端光标。
4. 用真实浏览器测试覆盖光标策略、模式切换、相对协议消息，以及当前工作区
   已有的剪贴板和 Pointer Lock 降级行为。

## 非目标与约束

- 不新增远端光标 DOM/canvas 叠加层。
- 不修改 RFB 服务端或 remote OS 的光标策略；remote OS 是否将光标绘制到
  帧缓冲由 host/服务端自身决定。
- 不引入运行时 npm 依赖；继续使用 `browser-tests/` 中已存在的
  `playwright-core` 和系统浏览器。
- 保留当前工作区已有的粘贴按钮、截图 PNG 剪贴板、Pointer Lock 重试和
  Pointer Lock 期间避免 RFB 重连等行为。

## 方案取舍

### 方案 A：继续按模式显示 noVNC 远端光标

改动最少，但把远端光标伪装成本机光标，且绝对模式与相对模式的视觉语义
不一致。放弃。

### 方案 B：增加独立远端光标叠加层

可以同时展示本机与远端光标，但需要重构 noVNC `Cursor` 的定位、缩放、
Pointer Lock 和触摸备用路径，测试和维护成本明显增加。当前需求不需要
两个网页端光标，因此放弃。

### 方案 C：网页客户端完全忽略 CursorShape（采用）

在 noVNC 的统一光标刷新路径中清除自定义 CSS cursor 并返回，不调用
`Cursor.change`。绝对模式自然使用浏览器系统光标；相对模式由 Pointer Lock
隐藏本机光标，同时不由网页客户端绘制远端光标。方案边界清晰，改动小，
且不影响 RFB 输入协议。

## 实现与测试

- 修改 `third_party/novnc/1.7.0/core/rfb.js`：统一清除自定义 cursor，
  删除按相对模式保留 noVNC 光标的分支。
- 修改 `browser-tests/novnc-browser.mjs`：通过真实 Chromium 页面和 RFB
  光标刷新路径验证绝对/相对模式都不会写入 CSS URL cursor，并验证相对
  Pointer Lock 的进入/退出状态。
- 保留并补强现有截图复制、粘贴按钮位置、Pointer Lock 降级和 RFB 相对
  消息构造的浏览器断言。
- 验收命令：
  - `cargo fmt --all --check`
  - `cargo test --workspace --all-features`
  - 构建 `ipkvm-browser-fixture` 后运行 `browser-tests/novnc-browser.mjs`
  - `cargo build -p ipkvm-headless --features demo --release`

## 风险与说明

VNC 服务端可能通过独立 CursorShape 消息传送 remote OS 光标，而不是把光标
像素绘制进 framebuffer。禁用网页端 CursorShape 后，这类服务端的光标不会
出现在浏览器画面中；这是本设计明确选择的行为，remote OS/host 是否把光标
绘制进视频帧仍由服务端决定。
