// 截图：/api/screenshot → 下载（<a download>）或复制（ClipboardItem）。

import { errorText } from "./api.js";
import { copyJpegToClipboard } from "./clipboard.js";
import { t } from "./i18n.js";

export class ScreenshotController {
  constructor({ menuButton, menu, saveButton, copyButton, message }) {
    this.menuButton = menuButton;
    this.menu = menu;
    this.saveButton = saveButton;
    this.copyButton = copyButton;
    this.message = message;
    this.open = false;
    this.frameAvailable = false;

    this.menuButton.addEventListener("click", () => this.toggle());
    this.saveButton.addEventListener("click", () => this.download());
    this.copyButton.addEventListener("click", () => this.copy());
    document.addEventListener("click", (event) => {
      if (this.open && !this.menu.contains(event.target) && !this.menuButton.contains(event.target)) {
        this.close();
      }
    });
  }

  toggle() {
    if (this.open) {
      this.close();
    } else {
      this.openMenu();
    }
  }

  openMenu() {
    this.open = true;
    this.menu.hidden = false;
    this.menuButton.setAttribute("aria-expanded", "true");
    this.saveButton.disabled = !this.frameAvailable;
    this.copyButton.disabled = !this.frameAvailable;
  }

  close() {
    this.open = false;
    this.menu.hidden = true;
    this.menuButton.setAttribute("aria-expanded", "false");
  }

  setFrameAvailable(available) {
    this.frameAvailable = Boolean(available);
    this.saveButton.disabled = !this.frameAvailable;
    this.copyButton.disabled = !this.frameAvailable;
  }

  async fetchScreenshot() {
    const response = await fetch("/api/screenshot");
    if (!response.ok) {
      let detail = "";
      try {
        const body = await response.json();
        detail = body?.detail ?? body?.error ?? "";
      } catch {
        // 非 JSON 错误体，保留空 detail。
      }
      throw new Error(detail || `HTTP ${response.status}`);
    }
    return response.blob();
  }

  async download() {
    this.message(t("screenshot.downloading"));
    try {
      const blob = await this.fetchScreenshot();
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `ipkvm-${new Date().toISOString().replace(/[:.]/g, "-")}.jpg`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      this.message(t("screenshot.downloaded"), "ok");
    } catch (error) {
      this.message(t("screenshot.downloadFailed", { detail: errorText(error) }), "error");
    }
  }

  async copy() {
    try {
      const blob = await this.fetchScreenshot();
      try {
        await copyJpegToClipboard(blob);
        this.message(t("screenshot.copyOk"), "ok");
      } catch (error) {
        // 剪贴板不可用时，fallback 已下载截图
        if (error.message === "clipboard unavailable, downloaded instead") {
          this.message(t("screenshot.downloadedFallback"), "ok");
        } else {
          throw error;
        }
      }
    } catch (error) {
      this.message(t("screenshot.copyFail", { detail: errorText(error) }), "error");
    }
  }
}
