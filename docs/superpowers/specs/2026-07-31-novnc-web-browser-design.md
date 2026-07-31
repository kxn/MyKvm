# noVNC 网页与真实浏览器闭环设计

## 1. 文档状态

- 关联 issue：`#17`
- 状态：已批准，待实施
- 适用阶段：无头版网页入口与真实浏览器闭环
- 前置依赖：`#15` 已完成共享 RFB 连接驱动和 `/rfb` WebSocket 入口
- 后续阶段：真实视频采集、真实 CH9329 串口和可运行生产会话组装

## 2. 目标

本阶段把已经完成的 RFB WebSocket 服务组装为可直接在浏览器中使用和自动验收的网页入口：

1. 固定并分发 noVNC 1.7.0 的库资源，保留来源、完整性和许可证证据。
2. 使用项目自有的中文页面连接同源 `/rfb`，不复制 noVNC 完整应用界面。
3. 把页面和 noVNC 资源嵌入 `ipkvm-headless`，运行时不依赖外部静态目录或 Node.js。
4. 提供可组合的 HTTP 路由，同时承载静态资源和现有 `/rfb`。
5. 页面保持视频原始宽高比，在浏览器可视区域内自动缩放，不改变远端分辨率。
6. 页面聚焦时把键盘和位于视频区域内的鼠标事件交给 noVNC。
7. 使用真实 Chrome 或 Edge 完成页面加载、WebSocket、帧显示、键盘和鼠标输入的自动化闭环。
8. 把非 Cargo 静态资源与浏览器测试依赖纳入可重复的本机验证。

本阶段交付的是使用模拟视频源运行的完整浏览器验收入口和可供生产组装复用的 Web 服务库，
不假装真实硬件已经接入。

## 3. 不做范围

- 不接入真实 UVC 视频采集卡、CH340 串口或 CH9329 硬件。
- 不把测试模拟源接入正式 `ipkvm-headless` 二进制的默认运行路径。
- 不实现设备选择页、配置持久化或多目标管理。
- 不实现鉴权、TLS、来源校验、访问控制或公网部署。
- 不实现多个同时活动的查看者或控制权仲裁。
- 不引入 FFmpeg、GStreamer、WebRTC、H.264、JPEG 或其他视频压缩链路。
- 不修改 RFB 协议、输入映射和单活动控制者语义。
- 不分发 Chrome、Edge 或 Playwright 自带浏览器。
- 不采用 noVNC 完整 `vnc.html` 应用、图片、字体和主题资源。

## 4. 调研结论

### 4.1 noVNC 固定版本

采用 npm 包 `@novnc/novnc` 1.7.0：

- 上游提交：`63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`
- npm tarball：
  `https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz`
- npm `integrity`：
  `sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA==`
- npm `shasum`：`7f832cf07c66475a81a25708b8e5299a5c4efec5`
- tarball 大小：`155185` 字节
- tarball SHA-256：
  `32689f18d6abe96bc6530828a6bd0b9ae33bda07c083a6575ed255b5a8f2e903`
- tarball SHA-512 十六进制：
  `b9c1093b1e13d9abc844295ea1d93b6286d98f93b619ad8078b7d0ebd03fca31bd1c76dadbe3c38304a437716db6b986fbbd191b1db5b0494d168b8ad77473c8`
- Git tree：`cae4519a8bf3930266db05dc1808b93a11c24236`

该包没有运行时 npm 依赖，包含 `core/`、`vendor/`、许可证和 API 文档。`core/rfb.js`
静态导入全部解码器，压缩相关模块又导入 `vendor/pako`；只挑选当前 Raw 编码路径的少数
文件会形成容易在升级时遗漏的隐式依赖。因此仓库保存 npm 包的完整发布内容，不手工裁剪
模块图。

npm 还提供该版本的发布证明，声明包由 GitHub Actions OIDC 从官方仓库的 `v1.7.0`
发布，并指向上述提交。该 attestation 只作为人工审查的参考证据；本阶段不实现 Sigstore
签名验证，不能把“保存并校验快照哈希”描述为已验证 provenance。升级脚本保存版本元数据
和 attestation 快照，正常 Rust 构建不访问 npm、GitHub 或其他网络来源。

现有本地 `noVNC-README.md`、`noVNC-API.md` 和 `noVNC-LICENSE.txt` 与固定提交一致；
`noVNC-EMBEDDING.md` 含固定提交中不存在的 `keep_device_awake` 段落，来自较新版本。
实施时必须把该文件替换为 1.7.0 固定提交快照，并把参考资料索引中的 `master` 链接改为
固定提交，避免设计与经常阅读的本地资料发生版本分叉。

### 4.2 页面边界

noVNC 的 `vnc_lite.html` 证明库式嵌入只需要导入 `core/rfb.js` 并构造 `RFB`。本项目
使用自有 `index.html`、`app.css` 和 `app.js`：

- 只复用 noVNC 库，不复制上游完整应用。
- 页面连接当前页面同源的 `/rfb`。
- `scaleViewport = true`，使画面在容器中等比缩放。
- `resizeSession = false`，浏览器尺寸不请求远端修改分辨率。
- noVNC 负责把画面内指针坐标换算为远端 framebuffer 坐标。
- 浏览器焦点和 RFB canvas 焦点决定键盘是否进入远端。

页面首屏就是控制台，不设置营销页或说明卡片。基本布局由窄状态栏、占满剩余视口的黑色
视频区域和必要的明确命令组成。连接状态同时写入可见中文文本和稳定的 DOM 属性，供用户
与自动化测试观察。

noVNC 1.7.0 在非安全上下文会记录“需要安全上下文”的错误，但不会在构造函数中直接
拒绝连接。当前自动化基线使用浏览器认可的本机安全上下文；通过机房 IP 使用普通 HTTP
属于已知部署限制，必须在后续 TLS/部署 issue 中增加非回环地址验收，不能用本机
`localhost` 结果替代该结论。

### 4.3 许可证边界

`@novnc/novnc` 发布包中的核心 JavaScript 使用 MPL-2.0。项目不修改 vendored noVNC
文件，发布时原样附带：

- `LICENSE.txt`
- `docs/LICENSE.MPL-2.0`
- `vendor/pako/LICENSE`
- `AUTHORS`
- `README.md`
- `package.json`

项目自有 HTML、CSS 和 JavaScript 继续使用仓库许可证，不从 noVNC 示例页面复制代码。
由于不引入 noVNC 完整应用的图片、字体和主题资源，本阶段不增加 CC-BY-SA 或 OFL
分发项。若未来修改 vendored noVNC 文件或采用完整应用资源，必须新开 issue 重新审查。

MPL-2.0 是文件级许可证。把未修改 JavaScript 作为静态资源嵌入 Rust 二进制不会把
项目自有 Rust、HTML、CSS 或 JavaScript 自动改为 MPL-2.0；发布包仍必须保留对应文件的
许可证和源码获取能力。本记录是工程合规边界，不代替法律意见。

`core/crypto/des.js` 内还有必须原样保留的 BSD 风格声明。完整 vendoring、逐文件原样
分发和全文件哈希清单同时覆盖该声明，不能用顶层 MPL 文本替代它。

### 4.4 Rust 静态资源嵌入

使用 `include_dir` 0.7.4 在编译时嵌入两个只读目录：

- 项目自有网页资源。
- 固定的 noVNC npm 发布资源。

该 crate 许可证为 MIT，最低 Rust 版本低于项目工具链要求，无默认功能。相较运行时读取
磁盘目录，嵌入方式能够保证二进制与资源版本一致；相较引入前端 bundler，不产生转译输出、
源码映射和第二套构建缓存。

HTTP 层只接受预定义入口和规范化的相对资源路径。请求路径在查找前必须拒绝空段外的
`.`、`..`、反斜杠和 NUL，不把 URL 直接拼接为文件系统路径。响应根据扩展名设置明确的
MIME 类型，并对未知资源返回空 `404`。

`rust-embed` 也能实现同类能力，但其默认 debug 行为可能读取文件系统，需要额外启用
`debug-embed` 才能保证测试与 release 资源来源一致。本阶段不需要自动 ETag 或运行时
文件覆盖，`include_dir` 的“所有构建模式始终编译期嵌入”语义更简单，因此采用
`include_dir`。资源内容一致性由独立 SHA-256 清单负责，不依赖嵌入 crate 的元数据。

### 4.5 真实浏览器工具链

浏览器测试使用锁定版本的 `playwright-core`，不使用会下载浏览器的 Playwright 包装器：

- 固定版本：`playwright-core@1.62.1`。
- registry：
  `https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz`。
- npm `integrity`：
  `sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw==`。
- npm `shasum`：`120f67a19181bfd183c60fa903c0d99330b56785`。
- 许可证：Apache-2.0。
- Node.js 要求：20 或更高版本。
- 该包没有 npm 运行时依赖，锁文件允许集合只有根测试包和该精确包。
- 测试依赖固定在独立 `browser-tests/package.json` 和 `package-lock.json`。
- `npm ci --ignore-scripts` 按锁文件安装，禁止依赖安装脚本。
- 测试按顺序查找系统 Chrome 和 Edge 的已知安装位置。
- 找不到支持的系统浏览器时验证失败，不静默跳过。
- 测试打印实际浏览器路径和版本，便于复现。

系统浏览器是测试环境前置条件，不进入项目发布物。Node.js 和 npm 只用于开发验收；
生产二进制不需要它们。`npm ci` 在本机缓存未命中时会访问固定 npm registry，因此只有
vendored noVNC 资源完整性检查和正常 Rust 构建是离线的，不能宣称整个
`verify.ps1` 离线。

## 5. 已比较方案

### 5.1 方案 A：完整 vendored noVNC 库、项目自有页面、Rust 编译时嵌入

优点：

- npm 发布内容、哈希和许可证可以离线校验。
- 不依赖运行目录，单个 Rust 二进制即可提供页面。
- 页面功能和视觉边界完全由项目控制。
- 没有 bundler，也没有生产 Node.js 依赖。
- noVNC 升级是显式的 vendored 目录替换和完整性审查。

缺点：

- 仓库增加约 635 KiB 未压缩第三方资源。
- 每次修改项目页面都需要重新编译 Rust 二进制。

### 5.2 方案 B：运行时从磁盘提供 noVNC 和页面

优点：

- 页面修改无需重编译。
- Rust 不需要嵌入资源依赖。

缺点：

- 可执行文件与资源目录可能版本错配或缺失。
- 服务启动位置、打包目录和路径权限成为额外故障面。
- 自动化验收通过的资源不一定是实际部署资源。

### 5.3 方案 C：npm bundler 打包 noVNC 和项目页面

优点：

- 可以压缩、拆包并生成内容哈希文件名。

缺点：

- 引入 bundler、插件、生成目录和 npm 生产构建链。
- 当前页面只有一个库入口，压缩收益不足以抵消构建与许可证复杂度。
- bundle 会混合项目代码与 MPL 文件，修改和源码对应关系更难审计。

### 5.4 结论

采用方案 A。仓库体积增加有限，而资源版本一致性、可离线审计和生产部署简单性对无头
设备更重要。

## 6. 目录与模块

建议目录：

```text
crates/ipkvm-headless/
  src/
    web/
      assets.rs
      mod.rs
      service.rs
  web/
    app.css
    app.js
    index.html
    licenses.html
  tests/
    web_http.rs
  src/bin/
    ipkvm-browser-fixture.rs
third_party/
  novnc/
    1.7.0/
      AUTHORS
      LICENSE.txt
      README.md
      core/
      docs/
      package.json
      vendor/
    README.md
    manifest.sha256
    npm-metadata.json
    npm-attestations.json
browser-tests/
  package.json
  package-lock.json
  novnc-browser.mjs
scripts/
  update-novnc.ps1
  verify-web-assets.ps1
  verify-browser.ps1
```

### 6.1 `web::assets`

负责：

- 编译时嵌入项目页面和 noVNC 发布目录。
- 根据规范化资源键查找内容。
- 产生固定 MIME 类型。
- 区分项目资源与第三方资源。

不负责：

- HTTP 路由。
- RFB 会话。
- 动态模板或运行时文件读取。

### 6.2 `web::service`

提供 `HeadlessWebService<S>`，接收与 `RfbWebSocketService<S>` 相同的生产依赖并组装：

- `GET /` 与 `GET /index.html`
- `GET /assets/app.css`
- `GET /assets/app.js`
- `GET /vendor/novnc/...`
- `/rfb`

`/rfb` 直接合并现有 `RfbWebSocketService::router()`，不得再实现一套 WebSocket 或
RFB 状态机。`HeadlessWebService::serve(listener)` 是唯一正式启动入口，内部必须：

1. 使用 `into_make_service_with_connect_info::<SocketAddr>()`，满足现有 `/rfb`
   handler 的 `ConnectInfo` 前提。
2. 对 Axum server 使用 `with_graceful_shutdown`，监听同一个 shutdown watch。
3. 把 listener 错误作为类型化服务错误返回。

监听器仍由调用方创建和持有到调用 `serve` 为止，以便生产程序和测试使用端口 `0`。
HTTP 集成测试和浏览器夹具必须调用该入口，不能各自手写 `axum::serve`。

### 6.3 浏览器测试夹具

独立二进制 `ipkvm-browser-fixture` 只在 `browser-fixture` Cargo feature 下构建，并在
`required-features` 中声明。它使用 `ipkvm-video/mock`：

1. 创建固定尺寸、具有高对比颜色块的 BGRA 模拟帧。
2. 在 `127.0.0.1:0` 启动完整 `HeadlessWebService`。
3. 向标准输出写一行机器可解析的就绪消息。
4. 用私有记录型 `InputSink` 运行现有 `RfbInputPump`。
5. 把记录型 sink 收到的键盘、指针和释放动作逐行写到标准输出。
6. 把输入泵产生的连接、拒绝和释放 notice 逐行写到标准输出。
7. 从标准输入收到 `STOP` 后触发现有 shutdown 通道，等待服务和输入泵退出并写出停止
   消息。

测试夹具不增加生产 HTTP 测试端点，也不进入默认二进制。正式
`ipkvm-headless` 主程序继续保持硬件会话尚未组装的事实，不用模拟源伪装为生产功能。
夹具内的组装只调用正式 `HeadlessWebService`、`RfbInputPump` 和公共 trait，不创建
第二套运行时抽象；真实硬件依赖具备以后再设计生产 supervisor，避免现在根据模拟设备
过早固化错误的生命周期接口。

关闭顺序固定为：

1. 夹具收到 `STOP` 或 stdin EOF。
2. 发送 shutdown。
3. RFB 连接结束并成功投递 `Disconnected`。
4. Axum graceful shutdown 完成，router 和最后一个 `event_tx` 被释放。
5. 输入泵排空事件、调用 `release_all` 并因事件通道关闭而退出。
6. 夹具输出 `STOPPED` 后退出。

任一核心任务提前失败时，夹具触发同一关闭链并返回非零，不能留下半存活 listener 或
输入泵。

## 7. HTTP 行为

### 7.1 路由

| 请求 | 响应 |
| --- | --- |
| `GET /` | 项目 `index.html` |
| `GET /index.html` | 项目 `index.html` |
| `GET /assets/app.css` | 项目 CSS |
| `GET /assets/app.js` | 项目 JavaScript module |
| `GET /vendor/novnc/<path>` | 固定 noVNC 发布文件 |
| `GET /licenses/` | 中文第三方组件、许可证和源码位置说明 |
| `GET /rfb` 且请求升级 | 现有 RFB WebSocket |
| 未知静态路径 | 空 `404` |
| 非 GET 静态请求 | `405` |

HTML 使用 `Cache-Control: no-cache`，项目资源和固定版本第三方资源使用
`Cache-Control: public, max-age=31536000, immutable` 之前必须先具备内容版本化 URL。
本阶段项目资源 URL 没有内容哈希，因此统一使用 `no-cache`，避免旧页面与新二进制的
资源不一致。以后若增加内容哈希再调整缓存策略。

### 7.2 MIME

至少支持：

- `.html`：`text/html; charset=utf-8`
- `.css`：`text/css; charset=utf-8`
- `.js`：`text/javascript; charset=utf-8`
- `.json`：`application/json`
- `.txt`、无扩展名许可证文件：`text/plain; charset=utf-8`

未登记扩展名不根据客户端输入猜测，返回 `application/octet-stream`。响应增加
`X-Content-Type-Options: nosniff`。这不是完整安全方案，只是静态资源正确性要求。

## 8. 页面行为

### 8.1 连接

页面加载后自动构造：

```javascript
const scheme = location.protocol === "https:" ? "wss" : "ws";
const url = `${scheme}://${location.host}/rfb`;
```

随后创建 `RFB`。状态机至少包含：

- `connecting`
- `connected`
- `disconnected`
- `failed`

状态写入根元素 `data-connection-state`，并显示对应中文文本。意外断开后不进行无界自动
重试；用户使用明确的“重新连接”命令，避免故障时形成连接风暴。重新连接收到 `409` 或
其他失败时页面保持失败状态和可用命令，不隐藏错误，也不改变后端 gate 的正确释放顺序。

### 8.2 画面

视频容器占据状态栏之外的全部可视区域：

- 背景为黑色。
- `overflow: hidden`。
- noVNC `scaleViewport = true`。
- 不设置会拉伸 canvas 的自定义宽高。
- 窗口变化只触发 noVNC 本地缩放，不请求远端尺寸变化。

浏览器测试必须同时覆盖桌面和窄视口，检查 canvas 边界不超过容器、宽高比与固定模拟帧
一致、页面没有水平或垂直溢出。

### 8.3 输入

- noVNC canvas 可获得焦点。
- 连接成功后用户点击视频区域才进入正常浏览器焦点路径。
- 键盘事件由 noVNC 转换为 RFB KeyEvent。
- 鼠标只有位于视频区域时由 noVNC 转换为 RFB PointerEvent。
- 页面不启用 pointer lock，不隐藏系统光标，不捕获视频区域外的鼠标。
- 页面失焦和断开由 noVNC 的输入状态处理；后端现有输入泵负责最终 `release_all`。

浏览器层只验收确定性、跨布局稳定的普通按键和主按钮点击。组合键释放、Unicode 到
keysym、坐标裁剪和输入泵错误继续由现有 Rust 测试承担，不在浏览器脚本复制协议测试。

## 9. 自动化测试

### 9.1 Rust 单元测试

`web::assets` 覆盖：

- 根页面、项目 JavaScript 和 noVNC `core/rfb.js` 可查找。
- MIME 类型正确。
- 未知资源不存在。
- `..`、反斜杠、NUL 和非规范路径被拒绝。
- 嵌入的 noVNC `package.json` 精确声明 1.7.0。

### 9.2 Rust HTTP 集成测试

真实 `127.0.0.1:0` listener 覆盖：

- `/`、项目资源和 noVNC module 返回正确状态、内容类型和 `nosniff`。
- 未知资源 `404`，非 GET `405`。
- `/rfb` 可以完成 WebSocket 升级和 RFB banner。
- 第二个活动连接仍由已有 gate 返回 `409`。
- shutdown 能结束服务。

这些测试不启动浏览器，快速验证路由组合和 HTTP 语义。

### 9.3 静态资源完整性测试

`verify-web-assets.ps1` 离线检查：

- vendored 文件集合与 `manifest.sha256` 完全一致。
- 每个文件 SHA-256 一致。
- 不存在清单外文件或缺失文件。
- `package.json` 版本、上游提交记录和 npm tarball 完整性记录匹配设计。
- 必须分发的 noVNC 与 pako 许可证文件存在。
- 浏览器 `package-lock.json` 只包含批准的固定 registry 来源和完整性字段。

`update-novnc.ps1` 是显式升级工具，不由验证脚本调用。预期 SRI、SHA-256、大小、版本
和提交来自已经审查并写入脚本参数默认值的固定记录，脚本不能根据刚下载的元数据自行决定
期望值。更新流程：

1. 下载到唯一系统临时目录并验证固定大小、SHA-256 和 SHA-512。
2. 解包前枚举 tar 条目，拒绝绝对路径、`..`、非 `package/` 根、符号链接和硬链接。
3. 只在系统临时目录解包，并检查所有结果没有 reparse point 且规范路径仍在临时根内。
4. 校验包内 `package.json` 的版本和 `gitHead`，检查必须许可证文件。
5. 只在确认目标是仓库内 `third_party/novnc/<固定版本>` 后替换并生成 SHA-256 清单。

资源门禁自测必须在临时副本中覆盖篡改、缺失、额外文件和恶意 tar 路径/链接反例。正常
资源验证流程不访问网络。

### 9.4 真实浏览器闭环

`verify-browser.ps1`：

1. 检查 Node.js、npm 和系统 Chrome/Edge。
2. 在 `browser-tests` 执行 `npm ci --ignore-scripts`。
3. 用 Cargo JSON 输出构建并定位具有 `browser-fixture` feature 的夹具可执行文件。
4. 通过环境变量把绝对可执行文件路径传给 `novnc-browser.mjs`。
5. Node 直接 `spawn` 夹具并独占其 stdin、stdout 和 stderr。
6. Node 等待夹具就绪行，再启动浏览器。
7. Node 通过夹具 stdin 请求停止并等待 `STOPPED` 和进程退出。
8. 任一步失败都关闭浏览器并清理子进程。

PowerShell 不启动夹具，因此不会与 Node 争用进程管道。Node 是夹具和浏览器的唯一进程
所有者。Windows 异常兜底可以按 PID 终止进程树，但必须先确认 PID 等于本次 `spawn`
返回的子进程 PID；强制终止只用于测试失败清理，不是正常关闭路径。

浏览器脚本使用事件和条件等待，不使用固定 `setTimeout` 或 sleep：

- 等待根元素状态变为 `connected`。
- 等待 canvas 具有预期 framebuffer 尺寸。
- 读取 canvas 像素，验证模拟帧的多个高对比采样点，而不只检查 canvas 存在。
- 在桌面与窄视口检查 canvas 边界、宽高比和页面溢出。
- 发送普通按键按下和释放，等待记录型 `InputSink` 输出对应 HID Key 事件。
- 从实际 canvas `getBoundingClientRect()` 选择非中心、非边缘点，根据 framebuffer 尺寸、
  实际缩放和 noVNC 的取整/裁剪语义计算预期坐标。
- 在桌面和窄视口分别点击，过滤连接初始化事件后断言绝对移动、主按钮按下和释放的完整
  顺序。
- 主动断开后先等待 DOM `disconnected`，再等待夹具输出 `ControllerReleased`，然后
  触发重新连接并等待 `connected`。

所有等待都有明确截止时间。截止时间只是失败上限，不作为调度同步手段。

### 9.5 统一验证

`scripts/verify.ps1` 增加：

1. 静态资源完整性检查。
2. 静态资源门禁反例自测。
3. 浏览器依赖锁文件检查。
4. 真实浏览器闭环。

它们在 Cargo 测试前后的位置以失败诊断清晰为准，但不能静默跳过。Gitea runner 不作为
验收前提，全部证据在本机产生。

统一 UTF-8 检查必须扩展到项目自写的 `.html`、`.css`、`.js`、`.mjs` 和资源清单。
vendored 上游文件按逐文件哈希原样验证，不对第三方内容作编码重写。

## 10. 错误与进程生命周期

- 无效 Web 配置在监听前返回类型化 Rust 错误。
- 静态资源缺失属于构建或测试失败，不在运行时回退到 CDN。
- 浏览器夹具的就绪消息只有 listener 成功和 router 完成组装后才发送。
- 浏览器测试进程提前退出、输出协议损坏或停止超时都使测试失败。
- Node 脚本必须在 `finally` 中关闭浏览器。
- Node 脚本必须在 `finally` 中请求夹具正常关闭，并只在失败时终止自己直接创建的 PID。
- PowerShell 包装器不持有夹具进程，只负责传播 Node 的退出码。
- 递归清理 npm 临时目录前必须解析绝对路径并确认仍位于
  `browser-tests/node_modules` 或系统临时目录。
- HTTP 服务关闭复用现有 watch shutdown，不增加进程级强制退出作为正常路径。

## 11. 风险与控制

### 11.1 noVNC 升级漂移

风险：上游新增模块、许可证或资源，而人工只替换部分文件。

控制：完整 npm 发布包、固定 tarball 完整性、全文件 SHA-256 清单、清单外文件失败、
独立升级脚本和升级 issue。

### 11.2 MPL 文件与项目代码混合

风险：bundle 或复制示例代码后无法区分文件许可证。

控制：不 bundle、不复制示例页面，第三方目录只保留上游文件，项目页面单独存放。

### 11.3 浏览器测试假阳性

风险：只验证 WebSocket 已连接或 canvas 存在，没有证明帧和输入真正穿过 RFB。

控制：采样 canvas 真实像素，并从 `RfbInputPump` 后面的记录型 `InputSink` 确认 HID
键盘、绝对 framebuffer 坐标和按钮状态。

### 11.4 测试依赖浏览器版本

风险：系统浏览器升级后行为变化。

控制：固定 Playwright 协议客户端版本、打印浏览器版本、只使用稳定 Web 平台能力，并在
失败时保留夹具和浏览器日志。项目不为测试方便分发大型浏览器二进制。

### 11.5 模拟入口误入生产

风险：开发夹具被误认为已经支持硬件。

控制：独立 required feature、独立二进制名称、默认构建不包含入口，长期文档明确硬件
会话仍未组装。

### 11.6 页面缩放与坐标

风险：CSS 直接拉伸 canvas，导致画面比例或坐标换算错误。

控制：只使用 noVNC `scaleViewport`，不覆盖内部 canvas 尺寸；真实浏览器同时断言边界、
比例、采样像素，以及从实际 canvas 边界换算出的非中心服务端坐标。

## 12. 验收条件

1. noVNC 1.7.0 资源、来源、完整性和许可证可离线复验。
2. 项目自有中文页面从同一 HTTP 服务加载 noVNC 并连接 `/rfb`。
3. 页面在桌面与窄视口保持模拟 framebuffer 比例且没有溢出。
4. 真实浏览器看到模拟帧的正确像素。
5. 真实浏览器键盘和鼠标事件到达 Rust `RfbServerEvent`。
6. 上述输入继续经过 `RfbInputPump`，到达记录型 `InputSink` 的 HID 与绝对坐标语义。
7. HTTP 未知资源、错误方法、断开与重连路径有自动化覆盖。
8. 生产 Web service 不依赖 Node.js、运行时静态目录或测试 HTTP 端点。
9. 测试夹具不进入默认生产二进制。
10. `.\scripts\verify.ps1` 在本机完整通过，没有人工测试例外。
11. README、粗粒度设计、引用索引和本设计均用中文记录实际状态。

## 13. 自审结论

独立只读审查没有发现需要推翻方案的 P0 问题，提出的 6 个 P1 问题已经全部纳入设计：

1. Node 成为夹具和浏览器的唯一进程所有者，PowerShell 不再争用管道。
2. 正式 `serve` 入口统一注入 `ConnectInfo` 并监督 Axum graceful shutdown。
3. 固定 `playwright-core` 版本、来源、完整性和许可证，并明确 `npm ci` 可能联网。
4. noVNC 更新流程增加固定信任值、解包前路径/链接检查和资源门禁反例自测。
5. 重连测试等待输入泵 `ControllerReleased`，不破坏现有 gate 释放顺序。
6. 坐标测试从实际 canvas 边界计算，并在两种视口断言完整输入顺序。

中风险项也已处理：扩展项目网页 UTF-8 门禁、给元数据和 attestation 固定目录、增加中文
第三方许可证入口，并要求替换版本错误的本地嵌入资料。残余风险是非回环普通 HTTP 的
secure context 行为和真实硬件生命周期，它们已明确排除在本 issue 之外，必须由后续
TLS/部署和硬件会话 issue 自动化验收。

## 14. 资料

- noVNC 1.7.0 package：
  `https://raw.githubusercontent.com/novnc/noVNC/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/package.json`
- noVNC library API：
  `https://novnc.com/noVNC/docs/API.html`
- noVNC embedding guide：
  `https://github.com/novnc/noVNC/blob/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/docs/EMBEDDING.md`
- noVNC lite example：
  `https://raw.githubusercontent.com/novnc/noVNC/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/vnc_lite.html`
- noVNC license：
  `https://raw.githubusercontent.com/novnc/noVNC/63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/LICENSE.txt`
- `@novnc/novnc` npm：
  `https://www.npmjs.com/package/@novnc/novnc/v/1.7.0`
- npm attestation：
  `https://registry.npmjs.org/-/npm/v1/attestations/@novnc%2Fnovnc@1.7.0`
- MPL 2.0 FAQ：
  `https://www.mozilla.org/en-US/MPL/2.0/FAQ/`
- include_dir：
  `https://docs.rs/include_dir/0.7.4/include_dir/`
- Playwright browser type：
  `https://playwright.dev/docs/api/class-browsertype`
