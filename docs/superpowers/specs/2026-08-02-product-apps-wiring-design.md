# 产品接线设计：headless 与 desktop 两个产品 app 共享会话核心

- 关联 issue：#28（后续补独立 issue 编号，见文末「后续工作」）
- 日期：2026-08-02
- 状态：设计定稿，待实现

## 背景与目标

`ipkvm-headless` 已具备库级组件（RFB TCP/WS 传输、noVNC 网页、`RfbInputPump`、`Ch9329InputSink`、`SerialCommandQueue`、`FrameSource`）并完成真实 CH9329 串口注入。但**组装逻辑全部堆在 main.rs（450 行过程式代码）**，`HeadlessConfig` 与 CLI 参数两套配置从未统一，`ipkvm-session` 的 `ConsoleSession` 是只有 `new()/config()` 的空壳。这些组件未接成产品。

**产品最终形态是两个并列的 app**：

- **headless 版本**：自带 HTTP 服务，跑起来对外用浏览器访问。
- **纯桌面 app 版本**：跑起来直接是本地窗口应用，无鉴权（连接本机），直连本机设备。

两者都要能够**选择设备并连接**。

目标：设计把现有组件接成这两个产品的对外接口——运维/部署侧（CLI + TOML 配置）、浏览器/前端侧（HTTP 管理 API）、桌面侧（本地窗口 + 设备选择），共享同一会话核心。

## 约束与决策

- **不基于 Qt/PyQt**（沿用 coarse-design 约束）。
- 桌面版**不走回环网络**：`rfb_connection` + `rfb_input` 是传输无关的库级组件，桌面版本地窗口事件直接在内存转成 `RfbServerEvent` 喂给共享核心；回环 TCP 仅是 headless 开发期闭环测试手段，不是产品架构。
- 桌面版**无鉴权**（连本机可信环境），但**不绑定网络端口**，天然不对外暴露；安全边界假设明确记录。
- headless 版**同步加最小鉴权**（token 或非本机拒绝），接口一上线就有保护。
- 依赖单向：`headless → session`、`desktop → session`，desktop 不碰 HTTP 栈；session 是共享核心。
- 改造**分三阶段**落地，每阶段可独立验证合并。

## 总体架构

```
ipkvm-desktop        wgpu+pixels 窗口、设备选择、直连本机、无鉴权
ipkvm-headless       传输适配、noVNC 网页、/api 全量、鉴权、TOML 配置
──────────────────── 共享核心 ────────────────────
ipkvm-session        rfb_connection + rfb_input + 设备枚举
                     + SessionManager + 会话状态 + 配置模型
──────────────────── 纯协议/设备 ─────────────────
ipkvm-core / ipkvm-rfb / ipkvm-video
```

关键动机：桌面版需要输入泵和仲裁，但它们依赖 headless 的 `rfb_connection` 事件模型——desktop 不能依赖 headless（否则被迫拉进整个 HTTP 栈），所以必须把「连接驱动 + 事件模型 + 仲裁 + 输入泵」整体上移到 session。headless 只剩传输适配 + Web + 鉴权 + 配置，desktop 只剩 GUI。两个 app 通过 session 获得完全相同的设备选择和会话能力。这也落实 coarse-design「跨 RFB、Web 和桌面入口的控制权仲裁归 session」。

## 阶段 1：session 归位（行为不变，测试不破）

### 搬入 session（从 headless，机械搬迁 + import 路径调整）

- `rfb_connection` 模块：驱动、事件模型 `RfbServerEvent`、`RfbConnectionGate` 仲裁、`RfbTransportKind` 等（约 1400 行）——传输无关，desktop 也要用仲裁。
- `rfb_input` 模块：pump/mapper/keymap/text（约 2900 行）——输入泵消费 `RfbServerEvent`，与 `rfb_connection` 必须同层。

### session 新增

- `devices` 模块：视频设备枚举（封装 `list_cameras`）+ 串口枚举（`serialport::available_ports`，新增依赖，随 serial feature）。
- `SessionManager`：会话创建/重启/停止——重启 = 只重建帧源+串口+输入泵，传输层不动；供 headless 的 `/api/session` 与 desktop 的设备切换共用。
- `ConsoleSession`：从空壳变为真实组装器（帧源 + 串口 + 输入泵 + gate），`start(config) → SessionHandle`（可停止）。
- 会话状态：输入统计、最后输入时间、丢帧计数、串口统计（给 `/api/status` 扩展）。

### headless 侧

- 保留 `rfb_tcp`/`rfb_ws` 传输适配器，改用 session 的驱动与事件模型。
- `lib.rs` 重新导出 session 类型，保持对外 API 兼容（已有测试不破）。

### 依赖

session 新增 `ipkvm-rfb`、`tokio`、`serialport`（可选，随 serial feature）；headless 继续依赖 session，链不变。

### 阶段 1 验证

`cargo test --workspace --all-features` 全绿 + `verify.ps1`（含真实浏览器闭环）——headless 行为与归位前一致。

## 阶段 2：headless 产品化（TOML 配置 + 全量 API + 鉴权）

### 配置：TOML 文件 + CLI 覆盖

- 新增 `--config <路径>` 读 TOML；CLI 参数覆盖文件字段（CLI 优先级最高）。
- 默认字段与现状一致（`bind=127.0.0.1`、`tcp=5900`、`http=6080`、`fps=10`、`baud=9600`）。
- 示例：

```toml
[server]
bind = "127.0.0.1"
tcp_port = 5900
http_port = 6080

[video]
camera = "OBS Virtual Camera"    # 或 assets 目录
fps = 30

[input]
serial = "COM9"
baud = 9600

[auth]
token = "..."                    # 可选；配置了才启用鉴权
```

- 配置文件错误、字段冲突有明确报错和帮助。

### HTTP 管理 API（蓝图全量）

按 coarse-design 蓝图补全，全部走鉴权（token 或禁配时拒绝非本机）：

| 路由 | 方法 | 说明 |
|---|---|---|
| `/api/devices` | GET | 枚举视频设备 + 串口设备，供设备选择页 |
| `/api/session` | POST | 创建/重启会话（`{video, serial}`），重启=重建帧源+串口+输入泵，传输层不动 |
| `/api/status` | GET | 扩展：+ 输入统计、最后输入时间、丢帧计数、活动客户端数（现有帧源+控制器保留） |
| `/api/screenshot` | GET | 现有保留 |

### 鉴权（最小）

- 配置 `[auth] token` 则启用；未配置默认拒绝非 `127.0.0.1` 来源（防默认暴露）。
- HTTP：`Authorization: Bearer <token>` 或 cookie；RFB TCP 与 WS 同样校验（VNC 密码/WS token）。
- 统一在传输层前的一个中间件/包装点做，不散落在每个路由。

### 错误处理

- 配置解析、会话启动失败、设备枚举失败返回结构化 JSON 错误（`{error, detail}`）+ 合理 HTTP 状态码。
- 会话停止失败、鉴权失败同样结构化。

### 阶段 2 测试

- 单元：配置 merge 优先级、设备枚举格式化、鉴权中间件放行/拒绝。
- 集成：`/api/devices`、`/api/session` 创建/重启、`/api/status` 扩展字段、鉴权拒绝非本机/token 错误。
- 浏览器闭环回归保留。

## 阶段 3：desktop app（wgpu+pixels 窗口 + 共享核心）

### 形态与定位

- 本地窗口应用，无鉴权（连本机可信环境），不走回环网络——内存事件线直连共享核心。
- 从 ipkvm-session 共享核心接线，不依赖 headless 的 HTTP/传输层。

### 技术栈

- **wgpu + pixels**：GPU 渲染视频帧；当前只有 BGRA，pixels 可直接纹理显示；YUY2/NV12 后续加 shader 转换，pixels 软渲染兜底（与 coarse-design 一致）。
- **窗口**：`winit`（pixels 的宿主窗口，轻量）。
- **事件循环**：winit 事件循环，本地键盘/鼠标事件 → `RfbServerEvent`（内存）→ 共享 `RfbInputPump`。

### 接线（与 headless 共用 session 核心）

```
本地窗口事件（键盘/鼠标） ──内存──▶ RfbInputPump ──▶ Ch9329InputSink ──▶ SerialCommandQueue ──▶ 串口
FrameSource（相机）     ──latest_frame──▶ wgpu 纹理 ──▶ 窗口
```

- 设备选择：复用 session 的 `devices` 枚举（相机下拉 + 串口下拉）。
- 会话启停：`SessionManager`（与 headless 的 `/api/session` 同一套），切换设备 = 重启会话。
- 控制权：`RfbConnectionGate` 仲裁——desktop 本地事件是唯一控制者，无需多客户端仲裁，但复用同一 gate 保证后续多入口扩展（未来桌面+远程并存）。

### 无鉴权但有安全边界

- 不绑定网络端口，天然不对外暴露。
- 本地可信环境假设明确记录，不引入密码/证书复杂度。

### 视频渲染

- 当前帧 `latest_frame()` → BGRA 纹理 → 窗口；无帧时显示「无信号」占位。
- 缩放保比例；窗口 resize 处理；后续加 wgpu shader 做 YUY2/NV12 转换。

### 阶段 3 测试

- 单元：内存事件 → 共享核心 → 记录型 InputSink 的链路（与 headless 输入测试同构）。
- 设备枚举、会话切换逻辑复用 session 的测试。
- 窗口渲染本身人工验证（像素级渲染难以自动化；记录原因）。

### 与 headless 的差异

| | headless | desktop |
|---|---|---|
| 入口 | 浏览器（远程） | 本地窗口 |
| 鉴权 | token/本机限制 | 无（可信本机） |
| 输入来源 | TCP/WS 事件流 | 内存事件 |
| 传输层 | TCP/WS | 无（直连核心） |

## 后续工作（不在本设计内）

- 鉴权/TLS 完整子系统（RFB 加密、证书）仍留后续。
- 多查看者并发观察同一帧缓冲（`ipkvm-session` 负责跨入口控制权仲裁）留后续。
- 视频压缩（脏块检测、ZRLE/Tight/JPEG）留后续。
- 会话创建/重启（`/api/session`）依赖帧源热替换，实现细节后续细化；初始会话由 CLI/配置启动。
