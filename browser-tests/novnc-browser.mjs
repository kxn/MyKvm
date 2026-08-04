import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import readline from "node:readline";
import { spawn } from "node:child_process";
import { chromium } from "playwright-core";

const DEADLINE_MS = 15_000;

const VIDEO_DEVICES = [
  { id: "fixture-camera", display_name: "Fixture Camera", kind: "video" },
];
const SERIAL_DEVICES = [
  { id: "COM7", display_name: "Fixture CH9329 (COM7)", kind: "serial" },
];
const DEFAULT_SETTINGS = {
  baud_rate: 115200,
  auto_baud: true,
  preview_fps: 30,
  mouse_mode: "absolute",
  relative_sensitivity: 1.0,
  scale_mode: "fit_window",
};

function withDeadline(promise, label, milliseconds = DEADLINE_MS) {
  const signal = AbortSignal.timeout(milliseconds);
  const expired = new Promise((_, reject) => {
    signal.addEventListener(
      "abort",
      () => reject(new Error(`${label} timed out after ${milliseconds} ms`)),
      { once: true },
    );
  });
  return Promise.race([promise, expired]);
}

class LineJournal {
  #lines = [];
  #waiters = [];

  push(line) {
    this.#lines.push(line);
    process.stdout.write(`[fixture] ${line}\n`);
    for (const waiter of [...this.#waiters]) {
      const result = waiter.check(this.#lines);
      if (result !== undefined) {
        this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
        waiter.resolve(result);
      }
    }
  }

  mark() {
    return this.#lines.length;
  }

  waitForLine(predicate, label, after = 0) {
    const check = (lines) => lines.slice(after).find(predicate);
    const existing = check(this.#lines);
    if (existing !== undefined) {
      return Promise.resolve(existing);
    }
    const pending = new Promise((resolve) => {
      this.#waiters.push({ check, resolve });
    });
    return withDeadline(pending, label);
  }

  waitForSubsequence(predicate, expected, label, after = 0) {
    const check = (lines) => {
      const relevant = lines.slice(after).filter(predicate);
      let expectedIndex = 0;
      for (const line of relevant) {
        if (line === expected[expectedIndex]) {
          expectedIndex += 1;
          if (expectedIndex === expected.length) {
            return relevant;
          }
        }
      }
      return undefined;
    };
    const existing = check(this.#lines);
    if (existing !== undefined) {
      return Promise.resolve(existing);
    }
    const pending = new Promise((resolve) => {
      this.#waiters.push({ check, resolve });
    });
    return withDeadline(pending, label);
  }
}

class FixtureProcess {
  constructor(executable) {
    assert(path.isAbsolute(executable), "fixture path must be absolute");
    assert(fs.existsSync(executable), `fixture does not exist: ${executable}`);
    this.lines = new LineJournal();
    this.stderr = [];
    this.child = spawn(executable, [], {
      shell: false,
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.exit = new Promise((resolve) => {
      this.child.once("exit", (code, signal) => resolve({ code, signal }));
    });
    readline.createInterface({ input: this.child.stdout }).on("line", (line) => {
      this.lines.push(line);
    });
    readline.createInterface({ input: this.child.stderr }).on("line", (line) => {
      this.stderr.push(line);
      process.stderr.write(`[fixture stderr] ${line}\n`);
    });
  }

  async ready() {
    const line = await this.lines.waitForLine(
      (value) => value.startsWith("READY\t"),
      "fixture ready line",
    );
    const [kind, url, width, height] = line.split("\t");
    assert.equal(kind, "READY");
    assert.equal(width, "320");
    assert.equal(height, "180");
    return { url, width: Number(width), height: Number(height) };
  }

  async stop() {
    if (this.child.exitCode === null) {
      this.child.stdin.write("STOP\n");
      await this.lines.waitForLine(
        (line) => line === "STOPPED",
        "fixture stopped line",
      );
      this.child.stdin.end();
    }
    const result = await withDeadline(this.exit, "fixture exit");
    assert.equal(
      result.code,
      0,
      `fixture failed: signal=${result.signal}, stderr=${this.stderr.join("\n")}`,
    );
  }

  async forceStop() {
    if (this.child.exitCode !== null) {
      return;
    }
    if (process.platform === "win32") {
      const killer = spawn(
        "taskkill.exe",
        ["/PID", String(this.child.pid), "/T", "/F"],
        { shell: false, windowsHide: true, stdio: "ignore" },
      );
      await withDeadline(
        new Promise((resolve) => killer.once("exit", resolve)),
        "fixture process tree cleanup",
      );
    } else {
      this.child.kill("SIGKILL");
    }
    await withDeadline(this.exit, "forced fixture exit");
  }
}

function findBrowserExecutable() {
  const candidates =
    process.platform === "win32"
      ? [
          "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
          "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
          "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
          "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
        ]
      : [
          "/usr/bin/google-chrome",
          "/usr/bin/google-chrome-stable",
          "/usr/bin/chromium",
          "/usr/bin/chromium-browser",
        ];
  const executable = candidates.find((candidate) => fs.existsSync(candidate));
  assert(executable, `no supported system browser found in ${candidates.join(", ")}`);
  return executable;
}

async function installApiMocks(context, postedSettings) {
  const storedSettings = { ...DEFAULT_SETTINGS };
  await context.route("**/api/devices", (route) => {
    return route.fulfill({
      json: { video: VIDEO_DEVICES, serial: SERIAL_DEVICES },
    });
  });
  await context.route("**/api/settings", async (route) => {
    const request = route.request();
    if (request.method() === "GET") {
      return route.fulfill({ json: { ...storedSettings } });
    }
    const posted = request.postDataJSON();
    postedSettings.push(posted);
    Object.assign(storedSettings, posted);
    return route.fulfill({ json: { ...storedSettings } });
  });
}

async function openConsole(browser, url, options = {}) {
  const {
    mockApi = true,
    permissions = ["clipboard-read", "clipboard-write"],
    beforeGoto,
    onConsoleError,
  } = options;
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    permissions,
  });
  const postedSettings = [];
  if (mockApi) {
    await installApiMocks(context, postedSettings);
  }
  if (beforeGoto) {
    await beforeGoto(context, postedSettings);
  }
  const page = await context.newPage();
  const browserErrors = [];
  page.on("console", (message) => {
    const location = message.location();
    const source = location.url ? ` (${location.url})` : "";
    if (message.type() === "error") {
      const entry = `${message.text()}${source}`;
      browserErrors.push(entry);
      onConsoleError?.(entry);
    }
    process.stdout.write(`[browser ${message.type()}] ${message.text()}${source}\n`);
  });
  page.on("pageerror", (error) => {
    browserErrors.push(error.message);
  });

  const response = await page.goto(url, { waitUntil: "domcontentloaded" });
  assert(response, "root navigation did not produce a response");
  assert.equal(response.status(), 200);
  return { context, page, postedSettings, browserErrors };
}

async function waitForConnectionView(page) {
  await page
    .locator("#connection-view:not([hidden])")
    .waitFor({ state: "attached" });
  await page.locator("#connect-button:not(:disabled)").waitFor({ state: "attached" });
}

async function waitForVideoView(page) {
  await page.locator("#video-view:not([hidden])").waitFor({ state: "attached" });
  await page.locator('#console[data-connection-state="connected"]').waitFor({
    state: "attached",
  });
  await page.waitForFunction(() => {
    const canvas = document.querySelector("#screen canvas");
    return (
      canvas instanceof HTMLCanvasElement &&
      canvas.width === 320 &&
      canvas.height === 180
    );
  });
}

async function assertFramePixels(page) {
  await page.waitForFunction(() => {
    const canvas = document.querySelector("#screen canvas");
    if (!(canvas instanceof HTMLCanvasElement)) {
      return false;
    }
    const context = canvas.getContext("2d");
    const points = [
      [80, 45, [255, 0, 0, 255]],
      [240, 45, [0, 255, 0, 255]],
      [80, 135, [0, 0, 255, 255]],
      [240, 135, [255, 255, 255, 255]],
    ];
    return points.every(([x, y, expected]) => {
      const actual = [...context.getImageData(x, y, 1, 1).data];
      return actual.every((value, index) => value === expected[index]);
    });
  });
}

async function assertLayout(page, viewport) {
  await page.setViewportSize(viewport);
  await page.waitForFunction(() => {
    const screen = document.querySelector("#screen");
    const canvas = screen?.querySelector("canvas");
    if (!(canvas instanceof HTMLCanvasElement)) {
      return false;
    }
    const screenRect = screen.getBoundingClientRect();
    const canvasRect = canvas.getBoundingClientRect();
    const epsilon = 1;
    return (
      canvasRect.width > 0 &&
      canvasRect.height > 0 &&
      Math.abs(canvasRect.width / canvasRect.height - 16 / 9) < 0.01 &&
      canvasRect.left >= screenRect.left - epsilon &&
      canvasRect.top >= screenRect.top - epsilon &&
      canvasRect.right <= screenRect.right + epsilon &&
      canvasRect.bottom <= screenRect.bottom + epsilon &&
      document.documentElement.scrollWidth <=
        document.documentElement.clientWidth &&
      document.documentElement.scrollHeight <=
        document.documentElement.clientHeight
    );
  });
}

async function assertKeyboard(page, fixture) {
  await page.locator("#screen canvas").focus();
  const marker = fixture.lines.mark();
  await page.keyboard.down("a");
  await page.keyboard.up("a");
  await fixture.lines.waitForSubsequence(
    (line) => line.startsWith("KEY\t"),
    ["KEY\tDOWN\t4", "KEY\tUP\t4"],
    "HID A key down and up",
    marker,
  );
}

async function assertKeyboardCapture(page, fixture) {
  await page.locator("#screen canvas").focus();
  const marker = fixture.lines.mark();
  await page.keyboard.down("Control");
  await page.keyboard.down("c");
  await page.keyboard.up("c");
  await page.keyboard.up("Control");
  await fixture.lines.waitForSubsequence(
    (line) => line.startsWith("KEY\t"),
    // Windows 下 noVNC 的 AltGr 探测先延迟 Control，收到 c 键时补发
    // Control down，因此远程键流为 ctrl down / c down / c up / ctrl up。
    ["KEY\tDOWN\t224", "KEY\tDOWN\t6", "KEY\tUP\t6", "KEY\tUP\t224"],
    "preventDefaulted Ctrl+C reaches the RFB key stream",
    marker,
  );
}

async function assertPointer(page, fixture, fractionX, fractionY) {
  const geometry = await page.locator("#screen canvas").evaluate(
    (canvas, fractions) => {
      const rect = canvas.getBoundingClientRect();
      const clientX = rect.left + rect.width * fractions.x;
      const clientY = rect.top + rect.height * fractions.y;
      const scaleX = rect.width / canvas.width;
      const scaleY = rect.height / canvas.height;
      return {
        clientX,
        clientY,
        expectedX: Math.min(
          canvas.width - 1,
          Math.max(0, Math.trunc((clientX - rect.left) / scaleX)),
        ),
        expectedY: Math.min(
          canvas.height - 1,
          Math.max(0, Math.trunc((clientY - rect.top) / scaleY)),
        ),
      };
    },
    { x: fractionX, y: fractionY },
  );
  const move = `POINTER\tMOVE\t${geometry.expectedX}\t${geometry.expectedY}\t320\t180`;
  const marker = fixture.lines.mark();
  await page.mouse.click(geometry.clientX, geometry.clientY);
  await fixture.lines.waitForSubsequence(
    (line) => line.startsWith("POINTER\t"),
    [
      move,
      "POINTER\tBUTTON\tLEFT\tDOWN",
      move,
      "POINTER\tBUTTON\tLEFT\tUP",
    ],
    "scaled pointer move and button sequence",
    marker,
  );
}

async function connectSession(page) {
  await page.locator("#connect-button").click();
  await waitForVideoView(page);
  await assertFramePixels(page);
}

async function assertRelativePointerMessageConstruction(page) {
  const probe = await page.evaluate(async () => {
    const { default: RFB } = await import("/vendor/novnc/core/rfb.js");
    const queue = [];
    const sock = {
      sQpush8(value) {
        queue.push(value & 0xff);
      },
      sQpush16(value) {
        queue.push((value >> 8) & 0xff, value & 0xff);
      },
      sQpush32(value) {
        queue.push(
          (value >> 24) & 0xff,
          (value >> 16) & 0xff,
          (value >> 8) & 0xff,
          value & 0xff,
        );
      },
      flush() {},
    };
    RFB.messages.relativePointerEvent(sock, 0b101, 10, -20, 1);
    return {
      bytes: queue,
      hasMessageBuilder:
        typeof RFB.messages.relativePointerEvent === "function",
      hasModeSwitch: typeof RFB.prototype.setRelativeMode === "function",
      hasSensitivitySwitch:
        typeof RFB.prototype.setRelativeSensitivity === "function",
      hasCanvasAccessor: "canvas" in RFB.prototype,
      hasScreenAccessor: "screenElement" in RFB.prototype,
    };
  });

  assert.equal(probe.hasMessageBuilder, true);
  assert.equal(probe.hasModeSwitch, true);
  assert.equal(probe.hasSensitivitySwitch, true);
  assert.equal(probe.hasCanvasAccessor, true);
  assert.equal(probe.hasScreenAccessor, true);
  assert.deepEqual(probe.bytes, [0x08, 0b101, 0, 10, 0xff, 0xec, 1]);
}

async function run() {
  const fixturePath = process.env.IPKVM_BROWSER_FIXTURE;
  assert(fixturePath, "IPKVM_BROWSER_FIXTURE is required");
  const fixture = new FixtureProcess(fixturePath);
  let browser;
  let contexts = [];
  try {
    const { url } = await fixture.ready();
    const browserPath = findBrowserExecutable();
    browser = await chromium.launch({
      executablePath: browserPath,
      headless: true,
    });
    process.stdout.write(`Browser: ${browserPath} (${await browser.version()})\n`);

    // ---- 连接页：枚举 / 选择 / 连接 ----
    const { context, page, postedSettings, browserErrors } = await openConsole(
      browser,
      url,
    );
    contexts.push(context);
    assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");
    await waitForConnectionView(page);
    assert.equal(await page.locator("#video-select option").count(), 1);
    assert.equal(await page.locator("#serial-select option").count(), 1);
    assert.match(await page.locator("#video-probe").textContent(), /就绪|Ready/);
    assert.match(await page.locator("#serial-probe").textContent(), /就绪|Ready/);
    assert.equal(await page.locator("#connect-button").isEnabled(), true);

    await connectSession(page);
    await assertKeyboard(page, fixture);
    await assertKeyboardCapture(page, fixture);
    await assertPointer(page, fixture, 0.73, 0.31);
    await assertLayout(page, { width: 1280, height: 800 });

    // ---- 特殊键菜单 ----
    const specialKeyMarker = fixture.lines.mark();
    await page.locator("#special-keys-button").click();
    await page.locator("#special-ctrl-alt-del").click();
    await fixture.lines.waitForSubsequence(
      (line) => line.startsWith("KEY\t"),
      [
        "KEY\tDOWN\t224",
        "KEY\tDOWN\t226",
        "KEY\tDOWN\t76",
        "KEY\tUP\t76",
        "KEY\tUP\t226",
        "KEY\tUP\t224",
      ],
      "special key menu sends Ctrl+Alt+Del",
      specialKeyMarker,
    );

    // ---- 设置读写 ----
    await page.locator("#open-settings").click();
    await page.locator("#settings-modal:not([hidden])").waitFor({ state: "attached" });
    assert.equal(
      await page.locator("#setting-baud-rate").inputValue(),
      "115200",
    );
    await page.locator("#setting-baud-rate").fill("9600");
    await page.locator("#settings-save").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });
    assert.equal(postedSettings.length, 1);
    assert.equal(postedSettings[0].baud_rate, 9600);
    assert.equal(postedSettings[0].preview_fps, 30);

    // ---- 设置：恢复默认值只改表单，取消还原旧值 ----
    await page.locator("#open-settings").click();
    await page.locator("#settings-modal:not([hidden])").waitFor({ state: "attached" });
    await page.locator("#setting-baud-rate").fill("12345");
    await page.locator("#settings-reset").click();
    assert.equal(
      await page.locator("#setting-baud-rate").inputValue(),
      "115200",
      "reset fills the form with defaults without applying",
    );
    await page.locator("#setting-baud-rate").fill("9600");
    await page.locator("#settings-cancel").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });
    await page.locator("#open-settings").click();
    await page.locator("#settings-modal:not([hidden])").waitFor({ state: "attached" });
    assert.equal(
      await page.locator("#setting-baud-rate").inputValue(),
      "9600",
      "cancel restores the previously applied value",
    );

    // ---- 设置：mouse_mode 与 relative_sensitivity 接入实际输入路径 ----
    await page.locator("#setting-mouse-mode").selectOption("relative");
    await page.locator("#setting-relative-sensitivity").fill("2.0");
    await page.locator("#settings-save").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });
    assert.equal(postedSettings.length, 2);
    assert.equal(postedSettings[1].mouse_mode, "relative");
    assert.equal(postedSettings[1].relative_sensitivity, 2.0);
    await page
      .locator('#relative-mode[data-state="armed"]')
      .waitFor({ state: "attached" });
    await page.locator("#open-settings").click();
    await page.locator("#settings-modal:not([hidden])").waitFor({ state: "attached" });
    await page.locator("#setting-mouse-mode").selectOption("absolute");
    await page.locator("#settings-save").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });
    assert.equal(postedSettings.length, 3);
    await page
      .locator('#relative-mode[data-state="off"]')
      .waitFor({ state: "attached" });

    // ---- 相对指针：协议层消息构造（端到端待 #141b 合入后验证）----
    await assertRelativePointerMessageConstruction(page);
    assert.equal(
      await page.locator("#relative-mode").isEnabled(),
      true,
      "relative mode button should be enabled while connected",
    );

    // ---- 截图下载 ----
    await page.locator("#screenshot-button").click();
    const downloadPromise = page.waitForEvent("download");
    await page.locator("#save-screenshot").click();
    const download = await withDeadline(downloadPromise, "screenshot download");
    assert.match(download.suggestedFilename(), /^ipkvm-.+\.jpg$/);
    const downloadPath = await download.path();
    const bytes = fs.readFileSync(downloadPath);
    assert.ok(bytes.length > 0, "screenshot file should not be empty");
    assert.deepEqual([...bytes.subarray(0, 2)], [0xff, 0xd8], "JPEG magic");

    // ---- 多标签状态同步 ----
    const second = await openConsole(browser, url);
    contexts.push(second.context);
    await second.page
      .locator("#video-view:not([hidden])")
      .waitFor({ state: "attached" });
    // 第二个标签看到 controller.active=true 时不创建 RFB 实例：显式断言
    // 忙提示、零画布与零错误（单控制器由状态机预防，不再盲试）。
    await second.page
      .locator("#video-message", { hasText: "另一处已连接" })
      .waitFor({ state: "attached" });
    assert.equal(
      await second.page.locator("#screen canvas").count(),
      0,
      "second tab must not create an RFB instance while another controller is active",
    );
    assert.deepEqual(
      second.browserErrors,
      [],
      "second tab should not observe connection errors while guarded",
    );
    const releaseMarker = fixture.lines.mark();
    await second.page.locator("#toolbar-disconnect").click();
    await waitForConnectionView(second.page);
    await waitForConnectionView(page);
    assert.match(
      await page.locator("#connection-message").textContent(),
      /停止/,
      "first tab should sync the stopped session",
    );
    await second.context.close();

    // ---- 重新连接 ----
    await fixture.lines.waitForLine(
      (line) => line === "CONTROLLER_RELEASED",
      "controller release after disconnect",
      releaseMarker,
    );
    await connectSession(page);

    // ---- 手动切回连接页（会话仍 running）：不自动重连、控制器保持空闲 ----
    const switchMarker = fixture.lines.mark();
    await page.locator("#toolbar-connect").click();
    await page
      .locator("#connection-view:not([hidden])")
      .waitFor({ state: "attached" });
    await fixture.lines.waitForLine(
      (line) => line === "CONTROLLER_RELEASED",
      "controller release after manual switch to connection page",
      switchMarker,
    );
    assert.equal(
      await page.locator("#screen canvas").count(),
      0,
      "old RFB DOM should be removed after switching to the connection page",
    );
    await page.waitForTimeout(2500);
    assert.deepEqual(
      browserErrors,
      [],
      "no RFB retry or 409/1006 while the session is running on the connection page",
    );
    // 控制器保持空闲：另一标签可接管。
    const taker = await openConsole(browser, url);
    contexts.push(taker.context);
    await taker.page
      .locator("#video-view:not([hidden])")
      .waitFor({ state: "attached" });
    await taker.page
      .locator('#console[data-connection-state="connected"]')
      .waitFor({ state: "attached", timeout: DEADLINE_MS });
    await taker.page.waitForFunction(() => {
      const canvas = document.querySelector("#screen canvas");
      return (
        canvas instanceof HTMLCanvasElement &&
        canvas.width === 320 &&
        canvas.height === 180
      );
    });
    await taker.context.close();
    // 回到视频页：连接按钮（restart）恢复本页控制。
    await page.locator("#connect-button").click();
    await waitForVideoView(page);

    // ---- 单控制器竞态：显式断言 409/1006 且失败后不再盲试 ----
    let mockStatusBusy = true;
    const race = await openConsole(browser, url, {
      beforeGoto: async (context) => {
        await context.route("**/api/status", async (route) => {
          if (!mockStatusBusy) {
            return route.continue();
          }
          const response = await route.fetch();
          const body = await response.json();
          body.controller.active = false;
          await route.fulfill({ response, json: body });
        });
      },
      onConsoleError: (entry) => {
        if (entry.includes("409")) {
          mockStatusBusy = false;
        }
      },
    });
    contexts.push(race.context);
    await race.page
      .locator("#video-message", { hasText: "远程画面连接失败" })
      .waitFor({ state: "attached", timeout: DEADLINE_MS });
    await race.page
      .locator("#video-message", { hasText: "另一处已连接" })
      .waitFor({ state: "attached", timeout: DEADLINE_MS });
    // 取消状态模拟后由真实状态接管：409 只应出现一次，不做 2s 盲试。
    await race.page.waitForTimeout(2500);
    const race409 = race.browserErrors.filter((entry) => entry.includes("409"));
    assert.ok(
      race409.length >= 1,
      "race page should observe the single-controller 409",
    );
    assert.equal(
      race409.length,
      1,
      "race page must not retry blindly after the 409",
    );
    assert.ok(
      race.browserErrors.some(
        (entry) => entry.includes("1006") || entry.includes("Failed when connecting"),
      ),
      "race page should observe the failed-connect close",
    );
    await race.context.close();

    // ---- 语言切换 ----
    await page.locator("#language-select").selectOption("en");
    assert.equal(await page.locator("html").getAttribute("lang"), "en");
    assert.equal(
      await page.locator("#toolbar-disconnect").textContent(),
      "Disconnect",
    );
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.language")),
      "en",
    );
    await page.locator("#language-select").selectOption("zh-CN");
    assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");

    // ---- 缺失 /api/settings 的降级路径 ----
    const degraded = await openConsole(browser, url, { mockApi: false });
    contexts.push(degraded.context);
    await degraded.page.locator("#open-settings").click();
    await degraded.page
      .locator("#settings-message", { hasText: "设置获取失败" })
      .waitFor({ state: "attached" });

    // ---- 静态资源与许可证 ----
    const missing = await page.request.get(`${url}/vendor/novnc/core/missing.js`);
    assert.equal(missing.status(), 404);
    assert.equal(await missing.body().then((body) => body.length), 0);

    const licenses = await page.request.get(`${url}/licenses/`);
    assert.equal(licenses.status(), 200);
    assert.match(await licenses.text(), /第三方组件与许可证/);

    assert.deepEqual(browserErrors, [], "primary page reported unexpected errors");

    await browser.close();
    browser = undefined;
    await fixture.stop();
  } catch (error) {
    if (browser) {
      await browser.close().catch(() => {});
    }
    try {
      await fixture.stop();
    } catch {
      await fixture.forceStop();
    }
    throw error;
  }
}

await run();
process.stdout.write("noVNC real browser verification passed.\n");
