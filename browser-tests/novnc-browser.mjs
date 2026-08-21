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
  mouse_profile: "raw_absolute",
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

async function waitForCondition(condition, label, milliseconds = DEADLINE_MS) {
  const deadline = Date.now() + milliseconds;
  while (!condition()) {
    assert(Date.now() < deadline, `${label} timed out after ${milliseconds} ms`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}

// 视频页悬浮控制条 3 秒无操作自动淡出；与条上控件交互前先唤出。
async function revealControlBar(page) {
  await page.mouse.move(640, 10);
  await page.waitForFunction(
    () => {
      const bar = document.querySelector("#control-bar");
      return bar && !bar.classList.contains("is-hidden");
    },
    undefined,
    { timeout: DEADLINE_MS },
  );
}

async function installApiMocks(context, postedSettings, settingsGets) {
  const storedSettings = { ...DEFAULT_SETTINGS };
  const sessionPosts = [];
  await context.route("**/api/devices", (route) => {
    return route.fulfill({
      json: { video: VIDEO_DEVICES, serial: SERIAL_DEVICES },
    });
  });
  await context.route("**/api/session", (route) => {
    const request = route.request();
    if (request.method() === "POST") {
      sessionPosts.push(request.postDataJSON());
    }
    return route.continue();
  });
  await context.route("**/api/settings", async (route) => {
    const request = route.request();
    if (request.method() === "GET") {
      settingsGets.count += 1;
      return route.fulfill({ json: { ...storedSettings } });
    }
    const posted = request.postDataJSON();
    postedSettings.push(posted);
    Object.assign(storedSettings, posted);
    return route.fulfill({ json: { ...storedSettings } });
  });
  return sessionPosts;
}

async function openConsole(browser, url, options = {}) {
  const {
    mockApi = true,
    permissions = ["clipboard-read", "clipboard-write"],
    beforeGoto,
    onConsoleError,
    colorScheme,
  } = options;
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    locale: "zh-CN",
    permissions,
    ...(colorScheme ? { colorScheme } : {}),
  });
  const postedSettings = [];
  const settingsGets = { count: 0 };
  let sessionPosts = [];
  if (mockApi) {
    sessionPosts = await installApiMocks(context, postedSettings, settingsGets);
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
  return { context, page, postedSettings, settingsGets, sessionPosts, browserErrors };
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

async function waitForInitialView(page) {
  await page
    .locator('#console:not([data-connection-state="connecting"])')
    .waitFor({ state: "attached" });
  const view = await page.locator("#console").getAttribute("data-view");
  assert.ok(view === "connection" || view === "video", `unexpected view ${view}`);
  return view;
}

async function assertFramePixels(page) {
  const readSamples = () => {
    const canvas = document.querySelector("#screen canvas");
    if (!(canvas instanceof HTMLCanvasElement)) {
      return { ready: false, reason: "missing canvas" };
    }
    const context = canvas.getContext("2d");
    const points = [
      ["topLeft", 80, 45],
      ["topRight", 240, 45],
      ["bottomLeft", 80, 135],
      ["bottomRight", 240, 135],
    ];
    const samples = points.map(([name, x, y]) => {
      const actual = [...context.getImageData(x, y, 1, 1).data];
      return { name, x, y, actual };
    });
    const sample = (name) => samples.find((item) => item.name === name).actual;
    const isRed = ([r, g, b]) => r > 80 && r > g + 30 && r > b + 30;
    const isGreen = ([r, g, b]) => g > 180 && g > r + 30 && g > b + 30;
    const isBlue = ([r, g, b]) => b > 80 && b > r + 30 && b > g + 30;
    const isWhite = ([r, g, b]) => r > 180 && g > 180 && b > 180;
    const topLeft = sample("topLeft");
    const topRight = sample("topRight");
    const bottomLeft = sample("bottomLeft");
    const bottomRight = sample("bottomRight");
    return {
      ready:
        ((isRed(topLeft) && isBlue(bottomLeft)) ||
          (isBlue(topLeft) && isRed(bottomLeft))) &&
        isGreen(topRight) &&
        isWhite(bottomRight),
      width: canvas.width,
      height: canvas.height,
      samples,
    };
  };
  try {
    await page.waitForFunction(() => {
      const canvas = document.querySelector("#screen canvas");
      if (!(canvas instanceof HTMLCanvasElement)) {
        return false;
      }
      const context = canvas.getContext("2d");
      const topLeft = [...context.getImageData(80, 45, 1, 1).data];
      const topRight = [...context.getImageData(240, 45, 1, 1).data];
      const bottomLeft = [...context.getImageData(80, 135, 1, 1).data];
      const bottomRight = [...context.getImageData(240, 135, 1, 1).data];
      const isRed = ([r, g, b]) => r > 80 && r > g + 30 && r > b + 30;
      const isGreen = ([r, g, b]) => g > 180 && g > r + 30 && g > b + 30;
      const isBlue = ([r, g, b]) => b > 80 && b > r + 30 && b > g + 30;
      const isWhite = ([r, g, b]) => r > 180 && g > 180 && b > 180;
      return (
        ((isRed(topLeft) && isBlue(bottomLeft)) ||
          (isBlue(topLeft) && isRed(bottomLeft))) &&
        isGreen(topRight) &&
        isWhite(bottomRight)
      );
    });
  } catch (error) {
    const samples = await page.evaluate(readSamples);
    throw new Error(`frame pixels did not match: ${JSON.stringify(samples)}`, {
      cause: error,
    });
  }
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

async function assertKeyboardCoordinatorModules(page) {
  const special = await page.evaluate(async () => {
    const { SpecialKeysController } = await import("/assets/modules/special-keys.js");
    const button = document.createElement("button");
    const menu = document.createElement("div");
    const sent = [];
    document.body.append(button, menu);
    const controller = new SpecialKeysController({
      button,
      menu,
      message: () => {},
      getRfb: () => ({
        sendKey: (_keysym, code, down) => {
          sent.push(`${down ? "DOWN" : "UP"}:${code}`);
        },
      }),
      heldModifiers: () => new Set(["control"]),
    });
    controller.openMenu();
    menu.querySelector("#special-ctrl-alt-del").click();
    button.remove();
    menu.remove();
    return sent;
  });
  assert.deepEqual(
    special,
    ["DOWN:AltLeft", "DOWN:Delete", "UP:Delete", "UP:AltLeft"],
    "special key menu must not release a modifier held by the normal keyboard path",
  );

  const keyboard = await page.evaluate(async () => {
    const { installKeyboardInterceptor } = await import("/assets/modules/keyboard.js");
    const canvas = document.createElement("canvas");
    const button = document.createElement("button");
    canvas.tabIndex = 0;
    button.type = "button";
    document.body.append(canvas, button);
    const released = [];
    const rfb = {
      canvas,
      _keyboard: {
        _keyDownList: { ControlLeft: 0xffe3 },
        _sendKeyEvent: (keysym, code, down) => released.push({ keysym, code, down }),
      },
    };
    const capture = installKeyboardInterceptor({ getRfb: () => rfb });
    canvas.focus();
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        code: "ControlLeft",
        key: "Control",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    const heldAfterDown = Array.from(capture.heldModifiers());
    button.focus();
    button.dispatchEvent(
      new KeyboardEvent("keyup", {
        code: "ControlLeft",
        key: "Control",
        bubbles: true,
        cancelable: true,
      }),
    );
    const heldAfterUp = Array.from(capture.heldModifiers());
    capture.dispose();
    canvas.remove();
    button.remove();
    return { heldAfterDown, heldAfterUp, released };
  });
  assert.deepEqual(keyboard.heldAfterDown, ["control"]);
  assert.deepEqual(keyboard.heldAfterUp, []);
  assert.deepEqual(
    keyboard.released,
    [{ keysym: 0xffe3, code: "ControlLeft", down: false }],
    "non-canvas keyup must release the key held by noVNC's normal keyboard path",
  );

  const releaseAll = await page.evaluate(async () => {
    const { installKeyboardInterceptor } = await import("/assets/modules/keyboard.js");
    const canvas = document.createElement("canvas");
    canvas.tabIndex = 0;
    document.body.append(canvas);
    const released = [];
    const rfb = {
      canvas,
      _keyboard: {
        _keyDownList: { ControlLeft: 0xffe3, KeyA: 0x61 },
        _sendKeyEvent: (_keysym, code, down) =>
          released.push(`${down ? "DOWN" : "UP"}:${code}`),
        _allKeysUp: () => released.push("ALL"),
      },
    };
    const capture = installKeyboardInterceptor({ getRfb: () => rfb });
    canvas.focus();
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        code: "ControlLeft",
        key: "Control",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    capture.releaseRemoteState();
    const heldAfterRelease = Array.from(capture.heldModifiers());
    capture.dispose();
    canvas.remove();
    return { released, heldAfterRelease };
  });
  assert.deepEqual(
    releaseAll,
    { released: ["ALL"], heldAfterRelease: [] },
    "explicit remote keyboard release must flush noVNC's full key state, not only tracked modifiers",
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

async function openSettingsModal(page, settingsGets) {
  const before = settingsGets.count;
  // 设置入口收在 ⋯ 更多菜单里，先唤出控制条再展开菜单。
  await revealControlBar(page);
  await page.locator("#more-button").click();
  await page.locator("#open-settings").click();
  await page.locator("#settings-modal:not([hidden])").waitFor({ state: "attached" });
  await waitForCondition(
    () => settingsGets.count > before,
    "settings GET after opening modal",
  );
  await page.waitForTimeout(50);
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
    const relativePointerBytes = [...queue];
    queue.length = 0;
    RFB.messages.setMouseMode(sock, "absolute");
    RFB.messages.setMouseMode(sock, "relative");
    let invalidModeRejected = false;
    try {
      RFB.messages.setMouseMode(sock, "bogus");
    } catch {
      invalidModeRejected = true;
    }
    return {
      bytes: relativePointerBytes,
      modeBytes: queue,
      invalidModeRejected,
      hasMessageBuilder:
        typeof RFB.messages.relativePointerEvent === "function",
      hasModeMessageBuilder:
        typeof RFB.messages.setMouseMode === "function",
      hasModeSwitch: typeof RFB.prototype.setRelativeMode === "function",
      hasSensitivitySwitch:
        typeof RFB.prototype.setRelativeSensitivity === "function",
      hasCanvasAccessor: "canvas" in RFB.prototype,
      hasScreenAccessor: "screenElement" in RFB.prototype,
    };
  });

  assert.equal(probe.hasMessageBuilder, true);
  assert.equal(probe.hasModeMessageBuilder, true);
  assert.equal(probe.hasModeSwitch, true);
  assert.equal(probe.hasSensitivitySwitch, true);
  assert.equal(probe.hasCanvasAccessor, true);
  assert.equal(probe.hasScreenAccessor, true);
  assert.deepEqual(probe.bytes, [0x08, 0b101, 0, 10, 0xff, 0xec, 1]);
  assert.deepEqual(probe.modeBytes, [0x09, 0, 0, 0, 0x09, 1, 0, 0]);
  assert.equal(probe.invalidModeRejected, true);
}

async function assertRelativeScheduler(page) {
  const result = await page.evaluate(async () => {
    const { default: RFB } = await import("/vendor/novnc/core/rfb.js");
    const messages = [];
    let current = [];
    const fake = {
      _rfbConnectionState: "connected",
      _viewOnly: false,
      _relativeMode: false,
      _relativeSensitivity: 0.4,
      _relativeDeltaX: 0,
      _relativeDeltaY: 0,
      _relativeRemainderX: 0,
      _relativeRemainderY: 0,
      _relativeMoveTimer: null,
      _relativeLastMoveTime: 0,
      _ignoreNextRelativeMove: false,
      _mouseButtonMask: 0,
      _mouseMoveTimer: null,
      _accumulatedWheelDeltaX: 7,
      _accumulatedWheelDeltaY: 11,
      _canvas: {
        getBoundingClientRect() {
          return { left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100 };
        },
      },
      _flushMouseMoveTimer() {},
      _flushRelativeMove: RFB.prototype._flushRelativeMove,
      _clearWheelState: RFB.prototype._clearWheelState,
      _handleWheel: RFB.prototype._handleWheel,
      _clearRelativeMoveState() {
        if (this._relativeMoveTimer !== null) {
          clearTimeout(this._relativeMoveTimer);
          this._relativeMoveTimer = null;
        }
        this._relativeDeltaX = 0;
        this._relativeDeltaY = 0;
        this._relativeRemainderX = 0;
        this._relativeRemainderY = 0;
        this._relativeLastMoveTime = 0;
      },
      _sock: {
        sQpush8(value) { current.push(value & 0xff); },
        sQpush16(value) {
          current.push((value >> 8) & 0xff, value & 0xff);
        },
        flush() {
          messages.push(current);
          current = [];
        },
      },
    };
    RFB.prototype.setRelativeMode.call(fake, true);
    fake._ignoreNextRelativeMove = false;
    RFB.prototype._handleRelativeMouseMove.call(fake, 1, 0);
    RFB.prototype._handleRelativeMouseMove.call(fake, 1, 0);
    RFB.prototype._handleRelativeMouseMove.call(fake, 1, 0);
    RFB.prototype._handleRelativeMouseMove.call(fake, 1, 0);
    const pendingTimer = fake._relativeMoveTimer;
    RFB.prototype._flushRelativeMove.call(fake, true);
    fake._mouseButtonMask = 1;
    RFB.prototype._handleWheel.call(fake, {
      buttons: 0,
      clientX: 3,
      clientY: 4,
      deltaMode: 0,
      deltaX: 0,
      deltaY: -80,
      stopPropagation() {},
      preventDefault() {},
    });
    RFB.prototype.sendRelativePointerRelease.call(fake);
    RFB.prototype.setRelativeMode.call(fake, false);
    return {
      messages,
      pendingCleared: fake._relativeDeltaX === 0 &&
        fake._relativeDeltaY === 0 &&
        fake._relativeRemainderX === 0 &&
        fake._relativeRemainderY === 0 &&
        fake._relativeMoveTimer === null,
      wheelCleared: fake._accumulatedWheelDeltaX === 0 &&
        fake._accumulatedWheelDeltaY === 0,
      hadPendingTimer: pendingTimer !== null,
    };
  });
  assert.deepEqual(result.messages, [
    [0x09, 1, 0, 0],
    [0x08, 0, 0, 1, 0, 0, 0],
    [0x08, 1, 0, 0, 0, 0, 1],
    [0x08, 0, 0, 0, 0, 0, 0],
    [0x09, 0, 0, 0],
  ]);
  assert.equal(result.hadPendingTimer, true);
  assert.equal(result.pendingCleared, true);
  assert.equal(result.wheelCleared, true);
}

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
        _cursorImage: {
          rgbaPixels: [255, 0, 0, 255],
          hotx: 0,
          hoty: 0,
          w: 1,
          h: 1,
        },
        _showDotCursor: false,
        _shouldShowDotCursor: RFB.prototype._shouldShowDotCursor,
      };
      RFB.prototype._refreshCursor.call(fake);
      return { relativeMode, cursor: fake._canvas.style.cursor, calls };
    });
  });
  for (const mode of result) {
    assert.equal(mode.cursor, "", `mode=${mode.relativeMode} must use system cursor`);
    assert.deepEqual(
      mode.calls,
      [],
      `mode=${mode.relativeMode} must skip noVNC Cursor.change`,
    );
  }
}

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

async function assertPointerLockFallback(page) {
  const result = await page.evaluate(async () => {
    const { PointerController } = await import("/assets/modules/pointer.js");
    const button = document.createElement("button");
    const messages = [];
    const requests = [];
    let attempt = 0;
    const rfb = {
      canvas: {
        style: {},
        requestPointerLock(options) {
          requests.push(options ?? null);
          attempt += 1;
          return attempt === 1
            ? Promise.reject(new Error("unadjusted movement unsupported"))
            : Promise.resolve();
        },
      },
      setRelativeMode() {},
      setRelativeSensitivity() {},
      sendRelativePointerRelease() {},
    };
    const controller = new PointerController({
      button,
      message: (text, level) => messages.push({ text, level }),
      getRfb: () => rfb,
    });
    controller.supported = true;
    controller.setRfb(rfb);
    controller.applySettings({ mouse_mode: "relative", relative_sensitivity: 1.0 });
    await controller.toggle({ stopPropagation() {} });
    await new Promise((resolve) => setTimeout(resolve, 0));
    return { requests, messages };
  });
  assert.deepEqual(result.requests, [{ unadjustedMovement: true }, null]);
  assert.deepEqual(result.messages, []);
}

async function assertPointerReleaseOnDetach(page) {
  const result = await page.evaluate(async () => {
    const { PointerController } = await import("/assets/modules/pointer.js");
    const button = document.createElement("button");
    const canvas = document.createElement("canvas");
    let releases = 0;
    const modes = [];
    const rfb = {
      canvas,
      setRelativeMode(mode) { modes.push(mode); },
      setRelativeSensitivity() {},
      sendRelativePointerRelease() { releases += 1; },
    };
    const controller = new PointerController({
      button,
      message() {},
      getRfb: () => rfb,
    });
    controller.supported = true;
    controller.setRfb(rfb);
    controller.locked = true;
    controller.setRfb(null);
    return { releases, modes, locked: controller.locked };
  });

  assert.equal(result.releases, 1);
  assert.deepEqual(result.modes, [false]);
  assert.equal(result.locked, false);
}

async function assertPointerReleaseOnLockLossEvents(page) {
  const result = await page.evaluate(async () => {
    const { PointerController } = await import("/assets/modules/pointer.js");
    const button = document.createElement("button");
    const canvas = document.createElement("canvas");
    let releases = 0;
    let exits = 0;
    const modes = [];
    let pointerLockElement = null;
    const originalPointerLockElement = Object.getOwnPropertyDescriptor(
      document,
      "pointerLockElement",
    );
    const originalExitPointerLock = document.exitPointerLock;
    Object.defineProperty(document, "pointerLockElement", {
      configurable: true,
      get() {
        return pointerLockElement;
      },
    });
    document.exitPointerLock = () => {
      exits += 1;
      pointerLockElement = null;
    };
    const rfb = {
      canvas,
      setRelativeMode(mode) { modes.push(mode); },
      setRelativeSensitivity() {},
      sendRelativePointerRelease() { releases += 1; },
    };
    try {
      const controller = new PointerController({
        button,
        message() {},
        getRfb: () => rfb,
      });
      controller.supported = true;
      controller.setRfb(rfb);

      const snapshot = () => ({
        releases,
        modes: modes.length,
        exits,
      });
      const delta = (before) => ({
        releases: releases - before.releases,
        modes: modes.slice(before.modes),
        exits: exits - before.exits,
        locked: controller.locked,
        cursor: canvas.style.cursor,
      });

      pointerLockElement = null;
      controller.locked = true;
      let before = snapshot();
      document.dispatchEvent(new Event("pointerlockchange"));
      const pointerlockchange = delta(before);

      pointerLockElement = canvas;
      controller.locked = true;
      before = snapshot();
      document.dispatchEvent(new Event("pointerlockerror"));
      const pointerlockerror = delta(before);

      pointerLockElement = canvas;
      controller.locked = true;
      before = snapshot();
      window.dispatchEvent(new Event("blur"));
      const blur = delta(before);

      return { pointerlockchange, pointerlockerror, blur };
    } finally {
      if (originalPointerLockElement) {
        Object.defineProperty(document, "pointerLockElement", originalPointerLockElement);
      } else {
        delete document.pointerLockElement;
      }
      document.exitPointerLock = originalExitPointerLock;
    }
  });

  assert.deepEqual(result.pointerlockchange, {
    releases: 1,
    modes: [false],
    exits: 0,
    locked: false,
    cursor: "",
  });
  assert.deepEqual(result.pointerlockerror, {
    releases: 1,
    modes: [false],
    exits: 0,
    locked: false,
    cursor: "",
  });
  assert.deepEqual(result.blur, {
    releases: 1,
    modes: [false],
    exits: 1,
    locked: false,
    cursor: "",
  });
}

async function assertCanvasRelativeCapture(page) {
  const result = await page.evaluate(async () => {
    const { PointerController } = await import("/assets/modules/pointer.js");
    const host = document.createElement("div");
    const canvas = document.createElement("canvas");
    host.appendChild(canvas);
    document.body.appendChild(host);

    const requests = [];
    let parentMouseDowns = 0;
    let canvasMouseDowns = 0;
    host.addEventListener("mousedown", () => {
      parentMouseDowns += 1;
    });
    canvas.addEventListener("mousedown", () => {
      canvasMouseDowns += 1;
    });
    canvas.requestPointerLock = (options) => {
      requests.push(options ?? null);
      return Promise.resolve();
    };

    const rfb = {
      canvas,
      setRelativeMode() {},
      setRelativeSensitivity() {},
      sendRelativePointerRelease() {},
    };
    const controller = new PointerController({
      button: document.createElement("button"),
      message() {},
      getRfb: () => rfb,
    });
    controller.supported = true;
    controller.setRfb(rfb);
    controller.applySettings({
      mouse_mode: "relative",
      relative_sensitivity: 1.0,
    });

    canvas.dispatchEvent(
      new MouseEvent("mousedown", {
        bubbles: true,
        clientX: 4,
        clientY: 5,
      }),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    host.remove();
    return { requests, parentMouseDowns, canvasMouseDowns };
  });

  assert.deepEqual(result.requests, [{ unadjustedMovement: true }]);
  assert.equal(
    result.parentMouseDowns,
    0,
    "relative canvas capture must stop the absolute pointer transition",
  );
  assert.equal(
    result.canvasMouseDowns,
    0,
    "relative canvas capture must stop other canvas handlers",
  );
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

    // ---- 初始页：fixture 可能已运行会话，也可能显示连接页 ----
    const { context, page, postedSettings, settingsGets, sessionPosts, browserErrors } =
      await openConsole(browser, url);
    contexts.push(context);
    await waitForCondition(
      () => settingsGets.count >= 1,
      "initial settings GET",
    );
    assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");
    if ((await waitForInitialView(page)) === "connection") {
      assert.equal(await page.locator("#video-select option").count(), 1);
      assert.equal(await page.locator("#serial-select option").count(), 1);
      // 设备唯一时自动选中（#103）
      assert.notEqual(await page.locator("#video-select").inputValue(), "");
      assert.notEqual(await page.locator("#serial-select").inputValue(), "");
      assert.equal(await page.locator("#connect-button").isEnabled(), true);
      // 成功路径不展示探测细节（#103）
      assert.equal(await page.locator("#video-probe").isHidden(), true);
      assert.equal(await page.locator("#serial-probe").isHidden(), true);
      // 高级折叠区：坐标模式从设置同步，设备参数只读展示当前值（#103）
      assert.equal(
        await page.locator("#connection-coordinate-mode").inputValue(),
        "raw_absolute",
      );
      assert.equal(await page.locator("#connection-mouse-profile").isDisabled(), true);
      await page.locator("#connection-advanced summary").click();
      assert.equal((await page.locator("#advanced-baud").textContent()).trim(), "115200");
      assert.equal((await page.locator("#advanced-fps").textContent()).trim(), "30");
      // 坐标模式联动：跟随目标系统时解锁目标系统下拉，切回后重新禁用（#103）
      await page.locator("#connection-coordinate-mode").selectOption("follow");
      assert.equal(await page.locator("#connection-mouse-profile").isEnabled(), true);
      await page.locator("#connection-mouse-profile").selectOption("windows");
      await page.locator("#connection-coordinate-mode").selectOption("raw_absolute");
      assert.equal(await page.locator("#connection-mouse-profile").isDisabled(), true);
      await connectSession(page);
      // 坐标模式为原始坐标时覆盖目标系统选择（#103）
      assert.equal(sessionPosts.at(-1)?.mouse_profile, "raw_absolute");
    } else {
      await waitForVideoView(page);
      await assertFramePixels(page);
    }
    await assertKeyboard(page, fixture);
    await assertKeyboardCapture(page, fixture);
    await assertKeyboardCoordinatorModules(page);
    await assertPointer(page, fixture, 0.73, 0.31);
    await assertLayout(page, { width: 1280, height: 800 });
    await assertNoVncCursorRendering(page);
    await assertClipboardImageConversion(page);
    await assertPointerLockFallback(page);
    await assertPointerReleaseOnDetach(page);
    await assertPointerReleaseOnLockLossEvents(page);
    await assertCanvasRelativeCapture(page);
    assert.equal(await page.locator("#control-bar #paste-button").count(), 1);
    assert.equal(await page.locator("#video-status-bar #paste-button").count(), 0);

    // ---- 特殊键菜单 ----
    const specialKeyMarker = fixture.lines.mark();
    await revealControlBar(page);
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

    // ---- 特殊键：分类折叠默认收起，展开后可发组内按键 ----
    await page.locator("#special-keys-button").click();
    const keyGroups = page.locator("#special-keys-menu details");
    assert.equal(await keyGroups.count(), 4, "非桌面四组应收进折叠分类");
    assert.equal(
      await page.locator("#special-keys-menu details[open]").count(),
      0,
      "折叠分类默认收起",
    );
    const f5Marker = fixture.lines.mark();
    await page.locator("#special-keys-menu details", { hasText: "F5" }).locator("summary").click();
    await page.locator("#special-f5").click();
    await fixture.lines.waitForSubsequence(
      (line) => line.startsWith("KEY\t"),
      ["KEY\tDOWN\t62", "KEY\tUP\t62"],
      "collapsed group sends F5",
      f5Marker,
    );

    // ---- 特殊键：自定义组合键（Ctrl+Enter）----
    await page.locator("#special-keys-button").click();
    const customMarker = fixture.lines.mark();
    await page.locator(".special-key-modifier", { hasText: "Ctrl" }).locator("input").check();
    await page.locator("#special-custom-key").selectOption("Enter");
    await page.locator("#special-custom-send").click();
    await fixture.lines.waitForSubsequence(
      (line) => line.startsWith("KEY\t"),
      ["KEY\tDOWN\t224", "KEY\tDOWN\t40", "KEY\tUP\t40", "KEY\tUP\t224"],
      "custom combo sends Ctrl+Enter",
      customMarker,
    );

    // ---- 粘贴对话框：预填剪贴板、可编辑、发送逐键注入 ----
    await page.evaluate(() => navigator.clipboard.writeText("ab"));
    await revealControlBar(page);
    const pasteMarker = fixture.lines.mark();
    await page.locator("#paste-button").click();
    await page.locator("#paste-modal:not([hidden])").waitFor({ state: "attached" });
    assert.equal(await page.locator("#paste-text").inputValue(), "ab", "对话框应预填剪贴板");
    assert.match(await page.locator("#paste-count").textContent(), /2 字符|2 characters/);
    assert.equal(await page.locator("#paste-warning").isVisible(), false);
    await page.locator("#paste-send").click();
    await fixture.lines.waitForSubsequence(
      (line) => line.startsWith("KEY\t"),
      ["KEY\tDOWN\t4", "KEY\tUP\t4", "KEY\tDOWN\t5", "KEY\tUP\t5"],
      "paste dialog injects text as keys",
      pasteMarker,
    );
    await page.locator("#paste-modal[hidden]").waitFor({ state: "attached" });

    // ---- 粘贴对话框：超长文本警告与取消 ----
    await revealControlBar(page);
    await page.locator("#paste-button").click();
    await page.locator("#paste-modal:not([hidden])").waitFor({ state: "attached" });
    await page.locator("#paste-text").fill("x".repeat(2500));
    await page.locator("#paste-warning:not([hidden])").waitFor({ state: "attached" });
    await page.locator("#paste-cancel").click();
    await page.locator("#paste-modal[hidden]").waitFor({ state: "attached" });

    // ---- 设置读写（分区对话框）----
    await openSettingsModal(page, settingsGets);
    // 默认落在「常规」分区，设备字段不在视图中。
    await page
      .locator('.settings-section[data-section="general"]:not([hidden])')
      .waitFor({ state: "attached" });
    assert.equal(await page.locator("#setting-baud-rate").isVisible(), false);
    // 每项设置都有生效时机标注。
    assert.ok(
      (await page.locator(".settings-section .field-hint").count()) > 0,
      "each settings field should carry a timing hint",
    );
    // 切到「设备」分区读写波特率。
    await page.locator('.settings-nav-item[data-section="device"]').click();
    assert.equal(
      await page.locator("#setting-baud-rate").inputValue(),
      "115200",
    );
    await page.locator("#setting-baud-rate").fill("9600");
    await page.locator("#settings-save").click();
    // 已连接时改设备参数：弹层不关闭，提示重连后生效并给出一键重连。
    await page
      .locator("#settings-message", { hasText: "重新连接后生效" })
      .waitFor({ state: "attached" });
    await page
      .locator("#settings-reconnect:not([hidden])")
      .waitFor({ state: "attached" });
    assert.equal(postedSettings.length, 1);
    assert.equal(postedSettings[0].baud_rate, 9600);
    assert.equal(postedSettings[0].preview_fps, 30);
    // 不点重连，取消关闭（当前值已保存为 9600，cancel 还原的正是它）。
    await page.locator("#settings-cancel").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });

    // ---- 设置：恢复默认值只改表单，取消还原旧值 ----
    await openSettingsModal(page, settingsGets);
    await page.locator('.settings-nav-item[data-section="device"]').click();
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
    await openSettingsModal(page, settingsGets);
    await page.locator('.settings-nav-item[data-section="device"]').click();
    assert.equal(
      await page.locator("#setting-baud-rate").inputValue(),
      "9600",
      "cancel restores the previously applied value",
    );

    // ---- 设置：mouse_mode 与 relative_sensitivity 接入实际输入路径 ----
    await page.locator('.settings-nav-item[data-section="input"]').click();
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
    // 设置为 relative 但尚未取得 Pointer Lock 时，noVNC 仍可能先发一笔
    // absolute Pointer。正式 CH9329 sink 也必须能处理这条过渡事件，否则
    // 输入泵会退出，/api/status 的 input_offline 再把页面推回重连流程。
    const relativeTransitionMarker = fixture.lines.mark();
    const transitionPoint = await page.locator("#screen canvas").evaluate((canvas) => {
      const rect = canvas.getBoundingClientRect();
      return { x: rect.left + rect.width * 0.5, y: rect.top + rect.height * 0.5 };
    });
    await page.mouse.move(transitionPoint.x, transitionPoint.y);
    await fixture.lines.waitForLine(
      (line) => line.startsWith("POINTER\tMOVE\t"),
      "absolute pointer transition while relative mode is armed",
      relativeTransitionMarker,
    );
    await page
      .locator('#console[data-connection-state="connected"]')
      .waitFor({ state: "attached" });

    await openSettingsModal(page, settingsGets);
    await page.locator('.settings-nav-item[data-section="input"]').click();
    await page.locator("#setting-mouse-mode").selectOption("absolute");
    await page.locator("#settings-save").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });
    assert.equal(postedSettings.length, 3);
    await page
      .locator('#relative-mode[data-state="off"]')
      .waitFor({ state: "attached" });

    // ---- 设置：状态行显隐（localStorage 纯前端偏好，即时生效）----
    await openSettingsModal(page, settingsGets);
    await page.locator('.settings-nav-item[data-section="video"]').click();
    assert.equal(await page.locator("#video-status-bar").isVisible(), true);
    await page.locator("#setting-status-line").uncheck();
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.statusLine")),
      "0",
    );
    await page.locator("#video-status-bar[hidden]").waitFor({ state: "attached" });
    await page.locator("#setting-status-line").check();
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.statusLine")),
      "1",
    );
    await page
      .locator("#video-status-bar:not([hidden])")
      .waitFor({ state: "attached" });

    // ---- 设置：常规分区语言/主题与 ⋯ 菜单同步 ----
    await page.locator('.settings-nav-item[data-section="general"]').click();
    await page.locator("#setting-theme").selectOption("dark");
    assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
    assert.equal(await page.locator("#theme-select").inputValue(), "dark");
    await page.locator("#setting-theme").selectOption("system");
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.theme")),
      null,
    );
    await page.locator("#setting-language").selectOption("en");
    assert.equal(await page.locator("html").getAttribute("lang"), "en");
    assert.equal(await page.locator("#language-select").inputValue(), "en");
    await page.locator("#setting-language").selectOption("zh-CN");
    assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");

    // ---- 设置：关于分区显示版本与链接 ----
    await page.locator('.settings-nav-item[data-section="about"]').click();
    const aboutVersion = await page.locator("#settings-version").textContent();
    assert.ok(
      aboutVersion && aboutVersion !== "-" && aboutVersion.length > 0,
      "about section should show the service version",
    );
    assert.equal(
      await page.locator('.settings-section[data-section="about"] a').count(),
      2,
      "about section links to licenses and GitHub",
    );
    await page.locator("#settings-cancel").click();
    await page.locator("#settings-modal[hidden]").waitFor({ state: "attached" });

    // ---- 相对指针：协议层消息构造（端到端待 #141b 合入后验证）----
    await assertRelativePointerMessageConstruction(page);
    await assertRelativeScheduler(page);
    assert.equal(
      await page.locator("#relative-mode").isEnabled(),
      true,
      "relative mode button should be enabled while connected",
    );

    // ---- 截图下载 ----
    await revealControlBar(page);
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
    await revealControlBar(second.page);
    await second.page.locator("#session-menu-button").click();
    await second.page.locator("#toolbar-disconnect").click();
    await waitForConnectionView(second.page);
    await waitForConnectionView(page);
    assert.match(
      await page.locator("#connection-message").textContent(),
      /停止/,
      "first tab should sync the stopped session",
    );
    await second.context.close();

    // ---- 连接向导：枚举失败与未发现设备的内联错误（#103） ----
    // 会话已停止时打开新页会落在连接页，正好验证失败态展示与回退语义。
    const enumFail = await openConsole(browser, url, {
      beforeGoto: async (ctx) => {
        await ctx.route("**/api/devices", (route) =>
          route.fulfill({ status: 500, body: "boom" }),
        );
      },
    });
    contexts.push(enumFail.context);
    await enumFail.page
      .locator('#video-probe[data-state="failed"]')
      .waitFor({ state: "attached" });
    assert.match(await enumFail.page.locator("#video-probe").textContent(), /枚举失败/);
    assert.match(await enumFail.page.locator("#serial-probe").textContent(), /枚举失败/);
    // 切换为空列表：内联提示"未发现设备"；无运行中会话时连接按钮不可用。
    await enumFail.context.unroute("**/api/devices");
    await enumFail.context.route("**/api/devices", (route) =>
      route.fulfill({ json: { video: [], serial: [] } }),
    );
    await enumFail.page.locator("#refresh-video").click();
    await enumFail.page.locator("#refresh-serial").click();
    await enumFail.page
      .locator('#video-probe[data-state="empty"]')
      .waitFor({ state: "attached" });
    assert.match(await enumFail.page.locator("#video-probe").textContent(), /未发现设备/);
    assert.match(await enumFail.page.locator("#serial-probe").textContent(), /未发现设备/);
    assert.equal(await enumFail.page.locator("#connect-button").isDisabled(), true);
    await enumFail.context.close();

    // ---- 重新连接 ----
    await fixture.lines.waitForLine(
      (line) => line === "RELEASE",
      "remote input release after disconnect",
      releaseMarker,
    );
    await connectSession(page);

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
    // 语言/主题下拉收在 ⋯ 更多菜单里；先唤出控制条、按 Escape 归位菜单状态再展开。
    await revealControlBar(page);
    await page.keyboard.press("Escape");
    await page.locator("#more-button").click();
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

    // ---- 主题三态：切换即时生效、刷新保持、system 跟随 prefers-color-scheme ----
    // 主 context 未指定 colorScheme，playwright 默认 light，system 应解析为浅色。
    assert.equal(await page.locator("html").getAttribute("data-theme"), "light");
    await revealControlBar(page);
    await page.keyboard.press("Escape");
    await page.locator("#more-button").click();
    await page.locator("#theme-select").selectOption("dark");
    assert.equal(
      await page.locator("html").getAttribute("data-theme"),
      "dark",
      "manual theme switch must apply immediately",
    );
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.theme")),
      "dark",
    );
    // 手动选择优先于系统偏好：模拟系统切浅色后仍保持深色。
    await page.emulateMedia({ colorScheme: "light" });
    assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
    // 刷新后保持手动选择。
    await page.reload({ waitUntil: "domcontentloaded" });
    assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
    // 回到 system：清除手动选择并跟随 prefers-color-scheme 实时变化。
    // 刷新后菜单已复位，唤出控制条并重新展开 ⋯ 菜单。
    await revealControlBar(page);
    await page.locator("#more-button").click();
    await page.locator("#theme-select").selectOption("system");
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.theme")),
      null,
      "system choice must clear the stored override",
    );
    // emulateMedia 的 matchMedia change 事件异步派发，轮询等待而非立即断言。
    await page.emulateMedia({ colorScheme: "dark" });
    await page.waitForFunction(
      () => document.documentElement.dataset.theme === "dark",
      undefined,
      { timeout: DEADLINE_MS },
    );
    await page.emulateMedia({ colorScheme: "light" });
    await page.waitForFunction(
      () => document.documentElement.dataset.theme === "light",
      undefined,
      { timeout: DEADLINE_MS },
    );

    // ---- 系统偏好为深色的首次访问直接落深色 ----
    // 注：现代 Chromium 的 no-preference 会解析为 light，无法模拟"无偏好信号"，
    // theme.js 中"无信号落深色"仅作防御性回退，不可在真实浏览器中断言。
    const darkSystem = await openConsole(browser, url, { colorScheme: "dark" });
    contexts.push(darkSystem.context);
    assert.equal(
      await darkSystem.page.locator("html").getAttribute("data-theme"),
      "dark",
      "dark system preference must resolve to the dark theme on first visit",
    );
    await darkSystem.context.close();

    // ---- 控制条：悬浮自动隐藏 / 顶部唤出 / 固定持久化 / 缩放循环 / 全屏 ----
    // 主页此前已重连并处于视频页（主题段 reload 后自动回到视频页）。
    await waitForVideoView(page);
    // 上一节操作后 ⋯ 菜单仍展开且焦点在条内（视为仍在操作，不会自动隐藏）：
    // 先收菜单、把焦点和鼠标移到画面中央。
    await page.keyboard.press("Escape");
    await page.mouse.click(640, 400);
    // 悬浮模式：3 秒无操作自动淡出。
    await page.waitForFunction(
      () => document.querySelector("#control-bar")?.classList.contains("is-hidden"),
      undefined,
      { timeout: 6000 },
    );
    // 鼠标移至视口顶部唤出。
    await page.mouse.move(640, 10);
    await page.waitForFunction(
      () => !document.querySelector("#control-bar")?.classList.contains("is-hidden"),
      undefined,
      { timeout: DEADLINE_MS },
    );
    // 固定后不再自动隐藏，固定状态持久化。
    await page.locator("#bar-pin").click();
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.controlBarPinned")),
      "1",
    );
    await page.waitForTimeout(3400);
    assert.equal(
      await page.locator("#control-bar.is-hidden").count(),
      0,
      "pinned control bar must not auto-hide",
    );

    // 会话菜单：展开可见连接状态与断开入口，Escape 收起。
    await page.locator("#session-menu-button").click();
    await page.locator("#session-menu:not([hidden])").waitFor({ state: "attached" });
    await page.locator("#toolbar-disconnect").waitFor({ state: "visible" });
    await page.keyboard.press("Escape");
    await page.locator("#session-menu[hidden]").waitFor({ state: "attached" });

    // 缩放模式循环：适配窗口 → 原始大小，标签更新并 POST 持久化。
    assert.match(
      await page.locator("#scale-mode-label").textContent(),
      /适配窗口/,
    );
    const scalePosts = postedSettings.length;
    await page.locator("#scale-mode-button").click();
    await waitForCondition(
      () => postedSettings.length > scalePosts,
      "scale mode persist POST",
    );
    assert.match(
      await page.locator("#scale-mode-label").textContent(),
      /原始大小/,
    );

    // 浏览器全屏（Fullscreen API）。
    await page.locator("#fullscreen-button").click();
    await page.waitForFunction(() => Boolean(document.fullscreenElement), undefined, {
      timeout: DEADLINE_MS,
    });
    await page.locator("#fullscreen-button").click();
    await page.waitForFunction(() => !document.fullscreenElement, undefined, {
      timeout: DEADLINE_MS,
    });

    // 取消固定恢复悬浮。
    await page.locator("#bar-pin").click();
    assert.equal(
      await page.evaluate(() => localStorage.getItem("my_ipkvm.controlBarPinned")),
      "0",
    );

    // ---- 状态监控抽屉：⋯ 菜单打开、指标渲染、高级默认折叠、Escape 关闭 ----
    await revealControlBar(page);
    await page.keyboard.press("Escape");
    await page.locator("#more-button").click();
    await page.locator("#status-panel-button").click();
    await page.locator(".status-drawer.open").waitFor({ state: "attached" });
    // 打开即触发一次轮询，分辨率应被真实数据填充。
    await page.waitForFunction(
      () => {
        const el = document.querySelector("#sp-resolution");
        return el && el.textContent !== "-";
      },
      undefined,
      { timeout: DEADLINE_MS },
    );
    assert.equal(
      await page.locator(".status-drawer-advanced[open]").count(),
      0,
      "advanced diagnostics must stay collapsed by default",
    );
    // 抽屉展开时画面仍可交互（不遮挡整个视口）。
    assert.equal(
      await page.locator(".status-drawer").evaluate((el) => {
        const rect = el.getBoundingClientRect();
        return rect.left > 0 && rect.width < window.innerWidth;
      }),
      true,
      "drawer must dock to the right edge without covering the video",
    );
    await page.keyboard.press("Escape");
    await page.waitForFunction(
      () => !document.querySelector(".status-drawer.open"),
      undefined,
      { timeout: DEADLINE_MS },
    );

    // ---- 缺失 /api/settings 的降级路径 ----
    const degraded = await openConsole(browser, url, { mockApi: false });
    contexts.push(degraded.context);
    // 后端已提供真实 /api/settings，此处显式路由成 404 以覆盖缺失降级。
    await degraded.page.route("**/api/settings*", (route) =>
      route.fulfill({
        status: 404,
        contentType: "application/json",
        body: JSON.stringify({ error: "not found" }),
      }),
    );
    await revealControlBar(degraded.page);
    await degraded.page.locator("#more-button").click();
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
