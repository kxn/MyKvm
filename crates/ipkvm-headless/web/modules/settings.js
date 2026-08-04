// 设置弹层：GET/POST /api/settings；缺失 API 时显示错误并回退默认值。

import { errorText, getJson, postJson } from "./api.js";
import { t } from "./i18n.js";

export const SETTINGS_DEFAULTS = Object.freeze({
  baud_rate: 115200,
  auto_baud: true,
  preview_fps: 30,
  mouse_mode: "absolute",
  relative_sensitivity: 1.0,
  scale_mode: "fit_window",
});

const MOUSE_MODES = new Set(["absolute", "relative"]);
const SCALE_MODES = new Set(["fit_window", "original", "follow_window"]);

export class SettingsController {
  constructor({ modal, message, fields, openButton, cancelButton, saveButton, resetButton, onChanged }) {
    this.el = { modal, message, fields };
    this.onChanged = onChanged;
    this.current = { ...SETTINGS_DEFAULTS };

    openButton.addEventListener("click", () => this.open());
    cancelButton.addEventListener("click", () => this.close());
    saveButton.addEventListener("click", () => this.save());
    resetButton.addEventListener("click", () => this.reset());
    this.el.modal.addEventListener("click", (event) => {
      if (event.target === this.el.modal) {
        this.close();
      }
    });
  }

  open() {
    this.el.modal.hidden = false;
    this.load();
  }

  close() {
    this.el.modal.hidden = true;
    this.el.message.textContent = "";
  }

  async load() {
    try {
      this.current = normalizeSettings(await getJson("/api/settings"));
      this.fill(this.current);
      this.setMessage("");
    } catch (error) {
      this.current = { ...SETTINGS_DEFAULTS };
      this.fill(this.current);
      this.setMessage(t("settings.loadFailed", { detail: errorText(error) }), "error");
    }
    this.onChanged?.(this.current);
  }

  fill(settings) {
    this.el.fields.baudRate.value = settings.baud_rate;
    this.el.fields.autoBaud.checked = Boolean(settings.auto_baud);
    this.el.fields.previewFps.value = settings.preview_fps;
    this.el.fields.mouseMode.value = settings.mouse_mode;
    this.el.fields.relativeSensitivity.value = settings.relative_sensitivity;
    this.el.fields.scaleMode.value = settings.scale_mode;
  }

  read() {
    return {
      baud_rate: Number(this.el.fields.baudRate.value),
      auto_baud: this.el.fields.autoBaud.checked,
      preview_fps: Number(this.el.fields.previewFps.value),
      mouse_mode: this.el.fields.mouseMode.value,
      relative_sensitivity: Number(this.el.fields.relativeSensitivity.value),
      scale_mode: this.el.fields.scaleMode.value,
    };
  }

  validate(settings) {
    if (!Number.isInteger(settings.baud_rate) || settings.baud_rate < 1200 || settings.baud_rate > 115200) {
      return "波特率必须是 1200..=115200 的整数";
    }
    if (!Number.isInteger(settings.preview_fps) || settings.preview_fps < 1 || settings.preview_fps > 60) {
      return "预览帧率必须是 1..=60 的整数";
    }
    if (!Number.isFinite(settings.relative_sensitivity) ||
        settings.relative_sensitivity < 0.1 ||
        settings.relative_sensitivity > 5.0) {
      return "相对灵敏度必须是 0.1..=5.0 的数字";
    }
    if (!MOUSE_MODES.has(settings.mouse_mode)) {
      return `未知鼠标模式：${settings.mouse_mode}`;
    }
    if (!SCALE_MODES.has(settings.scale_mode)) {
      return `未知缩放模式：${settings.scale_mode}`;
    }
    return null;
  }

  async save() {
    const settings = this.read();
    const invalid = this.validate(settings);
    if (invalid !== null) {
      this.setMessage(t("settings.invalid", { detail: invalid }), "error");
      return;
    }
    try {
      const saved = normalizeSettings(await postJson("/api/settings", settings));
      this.current = saved;
      this.setMessage(t("settings.saved"), "ok");
      this.onChanged?.(saved);
      this.close();
    } catch (error) {
      this.setMessage(t("settings.saveFailed", { detail: errorText(error) }), "error");
    }
  }

  reset() {
    this.current = { ...SETTINGS_DEFAULTS };
    this.fill(this.current);
    this.setMessage("");
    this.onChanged?.(this.current);
  }

  setMessage(text, level) {
    this.el.message.textContent = text;
    if (level) {
      this.el.message.dataset.level = level;
    } else {
      delete this.el.message.dataset.level;
    }
  }
}

function normalizeSettings(value) {
  const source = value ?? {};
  return {
    baud_rate: Number.isFinite(Number(source.baud_rate))
      ? Number(source.baud_rate)
      : SETTINGS_DEFAULTS.baud_rate,
    auto_baud: source.auto_baud === undefined ? SETTINGS_DEFAULTS.auto_baud : Boolean(source.auto_baud),
    preview_fps: Number.isFinite(Number(source.preview_fps))
      ? Number(source.preview_fps)
      : SETTINGS_DEFAULTS.preview_fps,
    mouse_mode: MOUSE_MODES.has(source.mouse_mode)
      ? source.mouse_mode
      : SETTINGS_DEFAULTS.mouse_mode,
    relative_sensitivity: Number.isFinite(Number(source.relative_sensitivity))
      ? Number(source.relative_sensitivity)
      : SETTINGS_DEFAULTS.relative_sensitivity,
    scale_mode: SCALE_MODES.has(source.scale_mode)
      ? source.scale_mode
      : SETTINGS_DEFAULTS.scale_mode,
  };
}
