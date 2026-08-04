// 相对指针：pointer lock 内 movement 增量经 noVNC 0x08 相对消息发送；
// 灵敏度与默认模式来自 /api/settings（保存后经 applySettings 下发）。

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
    this.selectedRelative = false;
    this.supported = relativePointerSupported();
    this.settings = null;
    this.sensitivity = 1.0;

    this.button.addEventListener("click", () => this.toggle());
    document.addEventListener("pointerlockchange", this.onPointerLockChange);
    document.addEventListener("pointerlockerror", this.onPointerLockError);
    window.addEventListener("blur", this.onWindowBlur);
    this.update();
  }

  setRfb(rfb) {
    this.rfb = rfb;
    if (rfb) {
      rfb.setRelativeSensitivity(this.sensitivity);
    } else {
      this.exit();
    }
    this.update();
  }

  applySettings(settings) {
    this.settings = settings;
    this.selectedRelative = settings?.mouse_mode === "relative";
    const sensitivity = Number(settings?.relative_sensitivity);
    this.sensitivity =
      Number.isFinite(sensitivity) && sensitivity > 0 ? sensitivity : 1.0;
    this.rfb?.setRelativeSensitivity(this.sensitivity);
    if (!this.selectedRelative && this.locked) {
      this.exit();
    }
    this.update();
  }

  async toggle(event) {
    event?.stopPropagation();
    if (this.locked) {
      this.exit();
      return;
    }
    if (!this.selectedRelative || !this.supported || !this.rfb) {
      if (this.selectedRelative && !this.supported) {
        this.message(t("video.relative.unsupported"), "error");
      }
      return;
    }
    const canvas = this.rfb.canvas;
    const showError = (error) => {
      this.message(
        t("video.relative.lockError", { detail: errorText(error) }),
        "error",
      );
    };
    try {
      const result = canvas.requestPointerLock({ unadjustedMovement: true });
      if (result && typeof result.catch === "function") {
        result.catch(() => {
          // 旧浏览器/不支持 unadjustedMovement 时退回无选项请求。
          try {
            const retry = canvas.requestPointerLock();
            if (retry && typeof retry.catch === "function") {
              retry.catch(showError);
            }
          } catch (error) {
            showError(error);
          }
        });
      }
    } catch (error) {
      showError(error);
    }
  }

  exit() {
    if (document.pointerLockElement) {
      document.exitPointerLock?.();
    }
    this.rfb?.setRelativeMode(false);
    if (this.rfb?.canvas) {
      this.rfb.canvas.style.cursor = "";
    }
    this.locked = false;
    this.update();
  }

  onPointerLockChange = () => {
    const canvas = this.rfb?.canvas;
    this.locked = canvas != null && document.pointerLockElement === canvas;
    this.rfb?.setRelativeMode(this.locked);
    if (this.rfb?.canvas) {
      this.rfb.canvas.style.cursor = this.locked ? "none" : "";
    }
    this.update();
    if (this.locked) {
      this.message(t("video.relative.on"), "ok");
    } else if (this.rfb) {
      this.message(t("video.relative.off"));
    }
  };

  onPointerLockError = () => {
    this.locked = false;
    this.rfb?.setRelativeMode(false);
    if (this.rfb?.canvas) {
      this.rfb.canvas.style.cursor = "";
    }
    this.update();
    this.message(t("video.relative.lockError", { detail: "pointerlockerror" }), "error");
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
    const armed = !this.locked && this.selectedRelative;
    if (this.locked) {
      this.button.textContent = t("video.relative.locked");
      this.button.dataset.state = "locked";
    } else if (armed) {
      this.button.textContent = t("video.relative.armed");
      this.button.dataset.state = "armed";
    } else {
      this.button.textContent = t("video.relative");
      this.button.dataset.state = "off";
    }
  }
}
