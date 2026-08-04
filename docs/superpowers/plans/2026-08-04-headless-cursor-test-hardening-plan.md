# Headless 光标策略与前端回归测试实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目标：** 完成 headless 网页控制台的光标策略调整，并用真实 Chromium 回归当前未提交的剪贴板、截图、Pointer Lock 和 RFB 输入行为。

**架构：** 在 vendored noVNC 的统一光标刷新路径中忽略服务端 CursorShape，绝对模式保留浏览器系统光标，相对模式交给 Pointer Lock 隐藏本地光标。浏览器回归测试通过现有 fixture 连接真实 RFB 页面，并对 RFB 原型的光标刷新路径做最小隔离探针。

**技术栈：** Rust/Cargo、JavaScript ES modules、Chromium、Playwright Core 1.62.1、现有 `ipkvm-browser-fixture`。

## 全局约束

- 不增加运行时 npm 依赖；浏览器验证继续使用 `browser-tests/` 和系统 Chromium。
- 不引入远端光标 DOM/canvas 叠加层。
- 测试先于生产代码修改，并先运行到预期失败。
- 自写文档使用中文；提交信息使用英文 conventional commit。
- 完成前运行 `cargo fmt --all --check` 与 `cargo test --workspace --all-features`。

---

### 任务 1：增加浏览器回归断言

**文件：**

- 修改：`browser-tests/novnc-browser.mjs`
- 参考：`crates/ipkvm-headless/web/modules/clipboard.js`

**接口：**

- 消费：fixture 提供的 `/api/screenshot`、真实 `/vendor/novnc/core/rfb.js` 和网页 DOM。
- 产出：`assertNoVncCursorRendering(page)`、`assertClipboardImageConversion(page)` 两个测试辅助函数，以及主流程中的工具栏/Pointer Lock 断言。

- [ ] **步骤 1：先加入会失败的 noVNC 光标测试**

在 `run()` 连接并确认画面后调用以下辅助函数。探针调用真实 `RFB.prototype._updateCursor`，只替换其宿主对象的连接状态、canvas 和 Cursor 依赖，断言绝对与相对模式都清除自定义 CSS cursor 且不调用 `Cursor.change`：

```js
async function assertNoVncCursorRendering(page) {
  const result = await page.evaluate(async () => {
    const { default: RFB } = await import("/vendor/novnc/core/rfb.js");
    return [false, true].map((relativeMode) => {
      const calls = [];
      const fake = {
        _rfbConnectionState: "connected",
        _relativeMode: relativeMode,
        _canvas: { style: { cursor: "url(old-cursor)" } },
        _cursor: { change: () => calls.push("change") },
        _cursorImage: null,
        _showDotCursor: false,
      };
      RFB.prototype._updateCursor.call(fake, [255, 0, 0, 255], 0, 0, 1, 1);
      return { relativeMode, cursor: fake._canvas.style.cursor, calls };
    });
  });
  for (const mode of result) {
    assert.equal(mode.cursor, "", `mode=${mode.relativeMode} must use system cursor`);
    assert.deepEqual(mode.calls, [], `mode=${mode.relativeMode} must skip noVNC Cursor.change`);
  }
}
```

- [ ] **步骤 2：运行浏览器测试确认 RED**

```powershell
cargo build -p ipkvm-headless --features browser-fixture --bin ipkvm-browser-fixture
$env:IPKVM_BROWSER_FIXTURE = (Resolve-Path target\debug\ipkvm-browser-fixture.exe).Path
node browser-tests\novnc-browser.mjs
```

预期：测试在相对模式断言处失败，原因是当前工作区的 `RFB._refreshCursor()` 仍会调用 `Cursor.change`。

- [ ] **步骤 3：加入当前未提交前端功能的回归断言**

连接画面后断言工具栏只有一个 `#paste-button`，截图 JPEG 可以通过真实 `jpegToPngBlob` 转成 `image/png`，并保留已有相对消息构造断言。设置为 `relative` 后点击 `#relative-mode`，若 Chromium 支持 Pointer Lock，则等待 `data-state="locked"`，再通过 `document.exitPointerLock()` 验证退出状态恢复。

```js
async function assertClipboardImageConversion(page) {
  const result = await page.evaluate(async () => {
    const { jpegToPngBlob } = await import("/assets/modules/clipboard.js");
    const response = await fetch("/api/screenshot");
    const png = await jpegToPngBlob(await response.blob());
    return {
      type: png.type,
      signature: [...new Uint8Array(await png.arrayBuffer()).subarray(0, 8)],
    };
  });
  assert.equal(result.type, "image/png");
  assert.deepEqual(result.signature, [137, 80, 78, 71, 13, 10, 26, 10]);
}
```

### 任务 2：实现 noVNC 光标策略

**文件：** `third_party/novnc/1.7.0/core/rfb.js`

- [ ] **步骤 1：删除相对模式专用的 noVNC 光标渲染分支**

将 `_refreshCursor()` 的连接状态判断之后改为：

```js
this._canvas.style.cursor = '';
return;
```

删除 `setRelativeMode()` 中“退出相对模式时恢复本地光标”的重复局部补丁，避免模式切换重新引入 noVNC 光标语义。

- [ ] **步骤 2：运行任务 1 的浏览器测试确认 GREEN**

重新构建 fixture 并运行 `node browser-tests\novnc-browser.mjs`，预期光标探针、画面、输入、设置、截图和多标签流程全部通过。

### 任务 3：全量验证与打包

- [ ] **步骤 1：运行格式检查**

```powershell
cargo fmt --all --check
```

预期退出码为 0。

- [ ] **步骤 2：运行 Rust 工作区全量测试**

```powershell
cargo test --workspace --all-features
```

预期所有测试通过且无失败。

- [ ] **步骤 3：构建 release 包**

```powershell
cargo build -p ipkvm-headless --features demo --release
```

预期生成 `target\release\ipkvm-headless.exe`。

- [ ] **步骤 4：检查最终差异**

```powershell
git diff --check
git status --short
```

预期没有空白错误，改动仅限设计/计划记录、现有 headless 前端改动、浏览器测试和 noVNC vendored patch。

### 任务 4：提交、合并与合并后复验

- [ ] **步骤 1：提交 feature 分支改动**

```powershell
git add docs/superpowers/specs/2026-08-04-headless-cursor-test-hardening-design.md docs/superpowers/plans/2026-08-04-headless-cursor-test-hardening-plan.md browser-tests/novnc-browser.mjs crates/ipkvm-headless/web/index.html crates/ipkvm-headless/web/modules/app.js crates/ipkvm-headless/web/modules/clipboard.js crates/ipkvm-headless/web/modules/pointer.js crates/ipkvm-headless/web/modules/screenshot.js third_party/novnc/1.7.0/core/rfb.js
git commit -m "test: harden headless web console regression coverage"
```

- [ ] **步骤 2：合并到本地 `main`**

```powershell
git switch main
git merge --no-ff codex/issue147-headless-light-stop -m "merge: harden headless web console regression coverage"
```

- [ ] **步骤 3：在合并结果上重新验证**

在 `main` 上重新运行任务 3 的格式检查、Rust 全量测试、fixture 浏览器测试和 release 构建，确认合并没有改变结果。

- [ ] **步骤 4：清理已合并旧分支**

只删除已合并且未被 worktree 使用的旧本地分支；保留 `main`，`feat/rfb-websocket` 因 worktree 占用暂不删除。远端分支不在本次清理范围内。
