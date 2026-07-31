import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import readline from "node:readline";
import { spawn } from "node:child_process";
import { chromium } from "playwright-core";

const DEADLINE_MS = 10_000;

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

async function waitForConnected(page) {
  await page
    .locator('#console[data-connection-state="connected"]')
    .waitFor({ state: "attached" });
  await page.waitForFunction(() => {
    const canvas = document.querySelector("#screen canvas");
    return canvas instanceof HTMLCanvasElement &&
      canvas.width === 320 &&
      canvas.height === 180;
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
    return canvasRect.width > 0 &&
      canvasRect.height > 0 &&
      Math.abs(canvasRect.width / canvasRect.height - 16 / 9) < 0.01 &&
      canvasRect.left >= screenRect.left - epsilon &&
      canvasRect.top >= screenRect.top - epsilon &&
      canvasRect.right <= screenRect.right + epsilon &&
      canvasRect.bottom <= screenRect.bottom + epsilon &&
      document.documentElement.scrollWidth <= document.documentElement.clientWidth &&
      document.documentElement.scrollHeight <= document.documentElement.clientHeight;
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

async function run() {
  const fixturePath = process.env.IPKVM_BROWSER_FIXTURE;
  assert(fixturePath, "IPKVM_BROWSER_FIXTURE is required");
  const fixture = new FixtureProcess(fixturePath);
  let browser;
  try {
    const { url } = await fixture.ready();
    const browserPath = findBrowserExecutable();
    browser = await chromium.launch({
      executablePath: browserPath,
      headless: true,
    });
    process.stdout.write(
      `Browser: ${browserPath} (${await browser.version()})\n`,
    );
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const browserErrors = [];
    page.on("console", (message) => {
      const location = message.location();
      const source = location.url ? ` (${location.url})` : "";
      if (message.type() === "error") {
        browserErrors.push(`${message.text()}${source}`);
      }
      process.stdout.write(
        `[browser ${message.type()}] ${message.text()}${source}\n`,
      );
    });
    page.on("pageerror", (error) => {
      browserErrors.push(error.message);
    });

    const response = await page.goto(url, { waitUntil: "domcontentloaded" });
    assert(response, "root navigation did not produce a response");
    assert.equal(response.status(), 200);
    assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");
    await waitForConnected(page);
    await assertFramePixels(page);

    await assertLayout(page, { width: 1280, height: 800 });
    await assertKeyboard(page, fixture);
    await assertPointer(page, fixture, 0.73, 0.31);

    await assertLayout(page, { width: 390, height: 844 });
    await assertPointer(page, fixture, 0.27, 0.68);

    const releaseMarker = fixture.lines.mark();
    await page.getByRole("button", { name: "断开" }).click();
    await page
      .locator('#console[data-connection-state="disconnected"]')
      .waitFor({ state: "attached" });
    await fixture.lines.waitForLine(
      (line) => line === "CONTROLLER_RELEASED",
      "controller release after disconnect",
      releaseMarker,
    );
    await page.getByRole("button", { name: "重新连接" }).click();
    await waitForConnected(page);

    const missing = await page.request.get(
      `${url}/vendor/novnc/core/missing.js`,
    );
    assert.equal(missing.status(), 404);
    assert.equal(await missing.body().then((body) => body.length), 0);

    const licenses = await page.request.get(`${url}/licenses/`);
    assert.equal(licenses.status(), 200);
    assert.match(await licenses.text(), /第三方组件与许可证/);
    assert.deepEqual(browserErrors, [], "browser reported unexpected errors");

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
