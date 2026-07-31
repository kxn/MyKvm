# noVNC 网页与真实浏览器闭环实施计划

> 关联 issue：`#17`
>
> 设计文档：
> `docs/superpowers/specs/2026-07-31-novnc-web-browser-design.md`
>
> 执行要求：按任务顺序实施；每项生产行为先建立失败测试；每个任务独立提交；禁止把真实
> 硬件、鉴权或 TLS 混入本计划。

## 目标

把 noVNC 1.7.0 固定资源、项目自有中文控制台页面和现有 `/rfb` WebSocket 组装为一个
可嵌入 Rust 二进制的 HTTP 服务，并用系统 Chrome/Edge 自动证明：

```text
模拟 BGRA 帧
→ RFB Raw framebuffer
→ WebSocket
→ noVNC canvas

浏览器键鼠
→ noVNC
→ RFB 输入事件
→ RfbInputPump
→ 记录型 InputSink
```

## 任务 1：建立 noVNC 资源更新与离线完整性门禁

**文件：**

- 新建：`scripts/web-assets-tools.psm1`
- 新建：`scripts/test-web-assets.ps1`
- 新建：`scripts/verify-web-assets.ps1`
- 新建：`scripts/update-novnc.ps1`
- 新建：`third_party/novnc/README.md`
- 新建：`third_party/novnc/manifest.sha256`
- 新建：`third_party/novnc/npm-metadata.json`
- 新建：`third_party/novnc/npm-attestations.json`
- 新建：`third_party/novnc/1.7.0/**`
- 新建：`browser-tests/package.json`
- 新建：`browser-tests/package-lock.json`

### 步骤 1：写资源验证红灯

先创建模块和测试脚本。测试在唯一系统临时目录构造最小资源树，覆盖：

1. 清单和文件完全一致时通过。
2. 文件内容被篡改时失败。
3. 清单中的文件缺失时失败。
4. 出现清单外文件时失败。
5. 必需的 noVNC、MPL、pako 和 DES 声明缺失时失败。
6. 包内 `package.json` 名称、版本、许可证或运行依赖错误时失败。
7. 固定 `npm-metadata.json` 的 `gitHead`、tarball 或 integrity 错误时失败。
8. 浏览器锁文件出现未批准包、非固定版本、非 npm registry 或缺失 integrity 时失败。
9. tar 条目包含绝对路径、`..`、非 `package/` 根、符号链接或硬链接时失败。
10. 清理函数拒绝系统临时目录之外的路径。

运行：

```powershell
.\scripts\test-web-assets.ps1
```

预期：模块函数尚不存在或最小夹具不能通过，形成明确红灯。

### 步骤 2：实现纯验证工具

`web-assets-tools.psm1` 提供：

- 规范化并验证临时目录和仓库目标目录。
- 读取 `manifest.sha256` 并拒绝重复、绝对和越界路径。
- 枚举资源树并比较文件集合与 SHA-256。
- 校验 noVNC 包内元数据、固定 npm 元数据和许可证文件。
- 校验 `browser-tests/package-lock.json` 的允许包集合。
- 检查 `tar -tvf` 和 `tar -tf` 条目类型及路径。

所有写文件使用无 BOM UTF-8。递归删除前重新解析绝对路径并验证边界。

运行：

```powershell
.\scripts\test-web-assets.ps1
```

预期：所有正反例通过。

### 步骤 3：导入固定 noVNC 发布包

使用已经调研的常量：

```text
版本：1.7.0
提交：63107bd06d9e1f6136ff21aeda8cd62cbf0d433e
tarball：https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz
大小：155185
SHA-256：32689f18d6abe96bc6530828a6bd0b9ae33bda07c083a6575ed255b5a8f2e903
SHA-512：b9c1093b1e13d9abc844295ea1d93b6286d98f93b619ad8078b7d0ebd03fca31bd1c76dadbe3c38304a437716db6b986fbbd191b1db5b0494d168b8ad77473c8
```

`update-novnc.ps1` 必须先验证归档，再安全检查条目，只在唯一临时目录解包，最后替换
仓库内固定版本目录并生成全文件 SHA-256 清单。保存 npm 元数据与 attestation 快照，
明确后者只是参考证据。

运行：

```powershell
.\scripts\update-novnc.ps1
.\scripts\verify-web-assets.ps1
```

预期：66 个 npm 发布文件完整导入，清单离线验证通过。

### 步骤 4：锁定浏览器开发依赖

`browser-tests/package.json` 只声明：

```json
{
  "private": true,
  "type": "module",
  "devDependencies": {
    "playwright-core": "1.62.1"
  }
}
```

生成并提交 npm lockfile。资源验证器必须确认允许集合只有根测试包和
`playwright-core@1.62.1`，来源、integrity 和许可证记录与设计一致。

运行：

```powershell
.\scripts\verify-web-assets.ps1
```

预期：noVNC 与浏览器锁文件门禁全部通过。

### 步骤 5：提交

```powershell
git add scripts third_party/novnc browser-tests/package.json browser-tests/package-lock.json
git commit -m "build: vendor verified noVNC assets (#17)"
```

## 任务 2：实现嵌入资源和正式 HTTP 服务

**文件：**

- 修改：`Cargo.toml`
- 修改：`Cargo.lock`
- 修改：`crates/ipkvm-headless/Cargo.toml`
- 修改：`crates/ipkvm-headless/src/lib.rs`
- 新建：`crates/ipkvm-headless/src/web/mod.rs`
- 新建：`crates/ipkvm-headless/src/web/assets.rs`
- 新建：`crates/ipkvm-headless/src/web/service.rs`
- 新建：`crates/ipkvm-headless/web/README.md`
- 新建：`crates/ipkvm-headless/tests/web_http.rs`

### 步骤 1：增加锁定依赖并审计

工作区增加：

```toml
include_dir = { version = "0.7.4", default-features = false }
```

`ipkvm-headless` 使用 workspace 依赖。运行：

```powershell
cargo check -p ipkvm-headless
.\scripts\verify-licenses.ps1
```

预期：MIT 依赖和 crates.io 来源通过门禁。

### 步骤 2：写资源查找红灯

在 `web/assets.rs` 的测试中先断言：

- `/vendor/novnc/core/rfb.js` 存在。
- noVNC `package.json` 精确声明 1.7.0。
- HTML、CSS、JS、JSON、文本和未知扩展名 MIME 正确。
- `..`、反斜杠、NUL、重复分隔符和非规范路径被拒绝。
- 未知资源返回不存在。

`web/README.md` 是中文开发说明，使项目资源目录在页面实现前稳定存在。资源测试使用
测试专用静态字节覆盖 MIME，不要求提前创建产品页面；不得把不存在目录导致的
`include_dir` 宏编译错误当作红灯。

运行：

```powershell
cargo test -p ipkvm-headless web::assets
```

预期：资源 API 尚不存在或断言失败。

### 步骤 3：实现资源模块

使用两个 `include_dir!`：

- `$CARGO_MANIFEST_DIR/web`
- `$CARGO_MANIFEST_DIR/../../third_party/novnc/1.7.0`

资源 API 返回静态字节、内容类型和资源类别。路径验证不访问文件系统，不做 SPA fallback。

逐个运行步骤 2 的测试，直到全部变绿。

### 步骤 4：写 HTTP 服务红灯

`tests/web_http.rs` 使用真实 `127.0.0.1:0` listener 和模拟 frame source，覆盖：

- noVNC module。
- `Content-Type`、`Cache-Control: no-cache`、`X-Content-Type-Options: nosniff`。
- 未知资源空 `404`。
- 对已知 noVNC 静态路径的 POST 返回 `405`。
- `/rfb` 使用同一 router 完成 WebSocket 升级并读到 RFB banner。
- 活动 WebSocket 时第二个连接返回 `409`。
- shutdown 结束活动 RFB 和 Axum listener。

测试必须只调用正式 `HeadlessWebService::serve(listener)`。

运行：

```powershell
cargo test -p ipkvm-headless --test web_http
```

预期：`HeadlessWebService` 尚不存在。

### 步骤 5：实现正式服务入口

`HeadlessWebService<S>`：

- 构造时验证并持有 `RfbWebSocketService<S>` 和 shutdown receiver。
- `router()` 合并静态 GET 路由、明确的许可证路由和 `/rfb`。
- `serve(self, listener)` 消耗 `self`，保证 server 返回后 router 及其 `event_tx` 一起
  释放。
- `serve(self, listener)` 使用
  `into_make_service_with_connect_info::<SocketAddr>()`。
- `serve(self, listener)` 使用 `with_graceful_shutdown` 监听同一个 shutdown watch。
- listener 错误通过类型化 `HeadlessWebServiceError` 返回。

不得改变现有 `RfbWebSocketService` 的协议与 gate 语义。

运行：

```powershell
cargo test -p ipkvm-headless --test web_http
cargo test -p ipkvm-headless
.\scripts\verify-licenses.ps1
```

预期：全部通过。

### 步骤 6：提交

```powershell
git add Cargo.toml Cargo.lock crates/ipkvm-headless
git commit -m "feat: add embedded headless web service (#17)"
```

## 任务 3：实现浏览器夹具

**文件：**

- 修改：`crates/ipkvm-headless/Cargo.toml`
- 新建：`crates/ipkvm-headless/src/bin/ipkvm-browser-fixture.rs`
- 新建：`crates/ipkvm-headless/tests/browser_fixture.rs`

### 步骤 1：写夹具进程红灯

为 `ipkvm-browser-fixture` 定义稳定行协议：

```text
READY<TAB>http://127.0.0.1:<port><TAB><width><TAB><height>
KEY<TAB>DOWN|UP<TAB><hid-usage>
POINTER<TAB>MOVE<TAB><x><TAB><y><TAB><width><TAB><height>
POINTER<TAB>BUTTON<TAB>LEFT|MIDDLE|RIGHT<TAB>DOWN|UP
RELEASE
CONTROLLER_RELEASED
STOPPED
```

`browser_fixture.rs` 启动实际二进制，覆盖：

- feature 开启时可以构建和启动。
- READY 使用动态端口且固定 noVNC module 可访问。
- stdin `STOP` 触发 `STOPPED` 和零退出码。
- stdin EOF 走同一正常关闭路径。
- Cargo metadata 声明夹具 binary 的 `required-features` 精确包含
  `browser-fixture`。
- 默认 feature 构建只构建正式 `ipkvm-headless`，不构建夹具。

测试使用行事件和进程 exit 等待，不使用 sleep。

运行：

```powershell
cargo test -p ipkvm-headless --all-features --test browser_fixture
```

预期：夹具二进制尚不存在。

### 步骤 2：实现 feature-gated 夹具

Cargo feature：

```toml
[features]
browser-fixture = ["ipkvm-core/mock", "ipkvm-video/mock"]

[[bin]]
name = "ipkvm-browser-fixture"
path = "src/bin/ipkvm-browser-fixture.rs"
required-features = ["browser-fixture"]
```

夹具：

- 创建固定 320×180 四象限 BGRA 帧。
- 创建正式 Web service、共享 gate 和有界 RFB 事件通道。
- 使用私有 `RecordingInputSink` 运行正式 `RfbInputPump`。
- 把唯一 `event_tx` 移入 `HeadlessWebService`，不得在夹具任务中保留 sender 或 clone。
- sink 与 pump notice 输出稳定行协议并显式 flush。
- stdin 线程只接收 `STOP` 或 EOF。
- 严格按设计关闭顺序 join HTTP 和输入泵。
- 任何提前失败触发关闭并以非零退出。

逐个运行步骤 1 的测试直至通过。

### 步骤 3：运行夹具和回归测试

```powershell
cargo test -p ipkvm-headless
cargo test -p ipkvm-headless --all-features --test browser_fixture
cargo build -p ipkvm-headless --no-default-features --bin ipkvm-headless
```

预期：全部通过。

### 步骤 4：提交

```powershell
git add crates/ipkvm-headless
git commit -m "test: add headless browser fixture (#17)"
```

## 任务 4：建立真实浏览器闭环

**文件：**

- 修改：`.gitignore`
- 新建：`crates/ipkvm-headless/web/index.html`
- 新建：`crates/ipkvm-headless/web/app.css`
- 新建：`crates/ipkvm-headless/web/app.js`
- 新建：`crates/ipkvm-headless/web/licenses.html`
- 新建：`browser-tests/novnc-browser.mjs`
- 新建：`scripts/verify-browser.ps1`
- 修改：`scripts/verify.ps1`

### 步骤 1：先完成浏览器测试基础设施

`verify-browser.ps1`：

- 校验 Node >= 20 和 npm 可用。
- 运行 `npm ci --ignore-scripts --prefix browser-tests`。
- 运行 Cargo JSON 构建并解析唯一的夹具 `executable` 绝对路径。
- 通过 `cargo metadata` 验证夹具 binary 只在 `browser-fixture` feature 下可用。
- 设置 `IPKVM_BROWSER_FIXTURE` 后调用 Node。
- 不启动或持有夹具进程，只传播 Node 退出码。

`.gitignore` 增加 `/browser-tests/node_modules/`。

### 步骤 2：写真实浏览器行为红灯

`novnc-browser.mjs` 使用 Node 内建断言和 `playwright-core`：

1. 查找系统 Chrome，再查找 Edge。
2. 读取 `IPKVM_BROWSER_FIXTURE` 绝对路径并直接 `spawn`，`shell: false`。
3. 等待 READY，再启动系统浏览器并打印版本。
4. 打开控制台页，等待 `data-connection-state="connected"`。
5. 读取 4 个 canvas 采样点，断言四象限颜色。
6. 在 1280×800 和 390×844 视口断言 canvas 比例、边界和页面无溢出。
7. 聚焦 canvas，发送 `KeyA` down/up，等待 HID usage 4。
8. 按实际 canvas 矩形在两个视口点击非中心点，断言移动和按钮 down/up 顺序。
9. 断开，等待 DOM disconnected 和 `CONTROLLER_RELEASED`，再重连成功。
10. 请求不存在 module，断言 `404` 而不是 HTML fallback。
11. 请求夹具 STOP，等待 `STOPPED` 和零退出。

所有 wait helper 使用事件或谓词与明确截止时间，禁止固定 sleep。

首次运行：

```powershell
.\scripts\verify-browser.ps1
```

预期：夹具与浏览器成功启动，但根页面尚不存在，验收明确失败在页面加载或连接状态；失败
后浏览器和夹具进程都被清理。

### 步骤 3：实现最终控制台页面

页面实现：

- HTML `lang="zh-CN"`，包含紧凑状态栏、视频区域、连接命令和许可证入口。
- `app.js` 从 `/vendor/novnc/core/rfb.js` 导入。
- WebSocket URL 根据 `location.protocol` 选择 `ws` 或 `wss`，路径固定 `/rfb`。
- `scaleViewport = true`，`resizeSession = false`。
- 连接状态写入 `data-connection-state`。
- 不包含 CDN、远端脚本、自动重试定时器或固定延时。
- 剩余视口全给黑色视频容器，使用 `min-height: 0`，不直接设置内部 canvas 宽高。
- 断开和失败时命令仍可用，不做自动重试。
- `/licenses/` 用中文说明 noVNC、MPL、pako、DES 声明和仓库源码位置。

运行：

```powershell
.\scripts\verify-browser.ps1
```

预期：真实浏览器所有闭环断言通过，退出后没有夹具或浏览器子进程残留。

### 步骤 4：接入统一门禁

`verify.ps1`：

- UTF-8 检查增加 `.html`、`.css`、`.js`、`.mjs` 和 `.sha256`。
- vendored noVNC 文件由哈希门禁负责，不作编码重写。
- 在 Rust 检查之前运行资源门禁和反例自测。
- 在 Rust 检查之后运行真实浏览器闭环。

运行：

```powershell
.\scripts\verify.ps1
```

预期：资源、Cargo、Rust 和浏览器全部通过。

### 步骤 5：提交

```powershell
git add .gitignore browser-tests crates/ipkvm-headless/web scripts
git commit -m "feat: add verified noVNC browser console (#17)"
```

## 任务 5：更新固定参考资料和长期文档

**文件：**

- 修改：`docs/references/noVNC-EMBEDDING.md`
- 修改：`docs/references/README.md`
- 修改：`docs/dependency-license-policy.md`
- 修改：`README.md`
- 修改：`docs/ipkvm-coarse-design.md`
- 修改：
  `docs/superpowers/specs/2026-07-31-novnc-web-browser-design.md`

### 步骤 1：修正固定资料

- 用提交 `63107bd...` 的 `docs/EMBEDDING.md` 原样替换当前较新快照。
- 参考资料索引把 noVNC `master` 链接改为固定提交。
- 记录 vendored npm 发布包、元数据、attestation 和许可证目录。

### 步骤 2：更新长期状态

写明：

- 无头库已经有嵌入式中文 noVNC 页面与正式 `serve` 入口。
- noVNC 固定为 1.7.0，核心是 MPL-2.0，pako 是 MIT，DES 文件保留 BSD 声明。
- 浏览器输入已自动证明到 `RfbInputPump` 后的 `InputSink`。
- 正式 `ipkvm-headless` 二进制仍未接入真实视频和串口。
- Node、npm、Playwright 和系统浏览器只属于本机验收环境。
- 非回环普通 HTTP secure context 行为、TLS、鉴权和硬件会话属于后续 issue。

把设计状态更新为“已实施”，记录实施事实和任何经验证差异。

### 步骤 3：文档检查

```powershell
rg -n "TBD|TODO|FIXME|待定|稍后实现" README.md docs
git diff --check
```

预期：本次新增内容没有占位项或英文项目文档。

### 步骤 4：提交

```powershell
git add README.md docs
git commit -m "docs: record noVNC browser integration (#17)"
```

## 任务 6：最终自审、全量验证、PR 和合并

### 步骤 1：实现对设计追踪

逐条检查设计第 12 节验收条件，确认每项都有自动化证据。重点审查：

- vendored 目录与清单是否完全一致。
- 生产构建是否完全不联网且不依赖 Node。
- `HeadlessWebService::serve` 是否统一注入 `ConnectInfo` 和 graceful shutdown。
- `/rfb` 是否只复用现有实现。
- 页面是否没有改写 canvas 比例。
- 浏览器输入是否真正到达 `InputSink`。
- Node 是否唯一持有夹具管道并能在失败时清理。
- 默认生产 binary 是否不包含模拟入口。

### 步骤 2：独立代码审查

委派只读审查，按严重度报告协议回归、进程泄漏、假阳性、许可证遗漏和缺失测试。修复所有
P0/P1，必要的 P2 也在本分支解决；修复后重新运行定向测试。

### 步骤 3：全量本机验证

```powershell
.\scripts\verify.ps1
git diff --check
git status --short
```

预期：全部通过，工作树干净。

### 步骤 4：创建中文 PR

PR 描述包含：

- `Closes #17`
- 改动摘要
- noVNC 与 Playwright 固定版本、完整性和许可证
- Rust、PowerShell、真实浏览器测试证据
- 文档影响
- 没有人工验证例外

### 步骤 5：合并并复验

合并后在主工作区：

```powershell
.\scripts\verify.ps1
git status --short
```

预期：主分支全量验证通过；只保留用户原有、未由本任务修改的 `AGENTS.md` 改动。
