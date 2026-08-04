// 相对指针：pointer lock 内 movement 增量经 noVNC 0x08 相对消息发送。

import { errorText } from "./api.js";
import { t } from "./i18n.js";

export function relativePointerSupported() {
  return "pointerLockElement" in document && navigator.maxTouchPoints === 0;
}

export class PointerController {
  constructor({ button, message, getRfb }) {
    this.button = button;
    this.message = message;
    this.getRfb = getRfb;
    this.rfb = null;
    this.locked = false;
    this.supported = relativePointerSupported();

    this.button.addEventListener("click", () => this.toggle());
    document.addEventListener("pointerlockchange", this.onPointerLockChange);
    window.addEventListener("blur", this.onWindowBlur);
    this.update();
  }

  setRfb(rfb) {
    this.rfb = rfb;
    if (!rfb) {
      this.exit();
    }
    this.update();
  }

  toggle() {
    if (this.locked) {
      this.exit();
      return;
    }
    if (!this.supported || !this.rfb) {
      return;
    }
    const canvas = this.rfb.canvas;
    try {
      const result = canvas.requestPointerLock({ unadjustedMovement: true });
      if (result && typeof result.catch === "function") {
        result.catch(() => {});
      }
    } catch (error) {
      this.message(`${t("video.relative.error")}：${errorText(error)}`, "error");
    }
  }

  exit() {
    if (document.pointerLockElement) {
      document.exitPointerLock?.();
    }
    this.rfb?.setRelativeMode(false);
    this.locked = false;
    this.update();
  }

  onPointerLockChange = () => {
    const canvas = this.rfb?.canvas;
    this.locked = canvas != null && document.pointerLockElement === canvas;
    this.rfb?.setRelativeMode(this.locked);
    this.update();
    if (this.locked) {
      this.message(t("video.relative.on"), "ok");
    } else if (this.rfb) {
      this.message(t("video.relative.off"));
    }
  };

  onWindowBlur = () => {
    this.exit();
  };

  update() {
    if (!this.supported) {
      this.button.disabled = true;
      this.button.title = t("video.relative.unsupported");
    } else {
      this.button.disabled = !this.rfb;
      this.button.title = "";
    }
    this.button.textContent = this.locked ? t("video.relative.locked") : t("video.relative");
    this.button.dataset.state = this.locked ? "locked" : "off";
  }
}
