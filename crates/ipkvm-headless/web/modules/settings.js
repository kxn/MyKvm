// 设置弹层：GET/POST /api/settings；缺失 API 时显示错误并回退默认值。
// 表单编辑只影响弹层；保存成功后才经 onChanged 应用到运行中的输入/画面路径。

import { errorText, getJson, postJson } from "./api.js";
import { t } from "./i18n.js";

export const SETTINGS_DEFAULTS = Object.freeze({
  baud_rate: 115200,
  auto_baud: true,
  preview_fps: 30,
  mouse_profile: "raw_absolute",
  mouse_mode: "absolute",
  relative_sensitivity: 1.0,
  scale_mode: "fit_window",
});

const MOUSE_MODES = new Set(["absolute", "relative"]);
export const MOUSE_PROFILES = new Set([
  "windows", "linux", "bios", "android", "macos", "raw_absolute", "raw_relative",
]);
const SCALE_MODES = new Set(["fit_window", "original", "follow_window"]);

export class SettingsController {
  constructor({
    modal,
    message,
    fields,
    openButton,
    cancelButton,
    saveButton,
    resetButton,
    reconnectButton,
    nav,
    isConnected,
    onReconnect,
    onChanged,
  }) {
    this.el = { modal, message, fields };
    this.reconnectButton = reconnectButton ?? null;
    this.isConnected = isConnected ?? (() => false);
    this.onReconnect = onReconnect ?? null;
    this.onChanged = onChanged;
    this.current = { ...SETTINGS_DEFAULTS };

    openButton.addEventListener("click", () => this.open());
    cancelButton.addEventListener("click", () => this.cancel());
    saveButton.addEventListener("click", () => this.save());
    resetButton.addEventListener("click", () => this.reset());
    reconnectButton?.addEventListener("click", () => {
      this.close();
      this.onReconnect?.();
    });
    nav?.addEventListener("click", (event) => {
      const item = event.target.closest(".settings-nav-item");
      if (item) {
        this.switchSection(item.dataset.section);
      }
    });
    this.el.fields.mouseProfile.addEventListener("change", () => {
      this.el.fields.mouseMode.value = modeForProfile(this.el.fields.mouseProfile.value);
    });
    this.el.fields.mouseMode.addEventListener("change", () => {
      this.el.fields.mouseProfile.value = profileForMode(this.el.fields.mouseMode.value);
    });
    this.el.modal.addEventListener("click", (event) => {
      if (event.target === this.el.modal) {
        this.cancel();
      }
    });
  }

  /// 左侧分区导航：切换右侧显示的分区，高亮当前项。
  switchSection(name) {
    for (const item of this.el.modal.querySelectorAll(".settings-nav-item")) {
      item.classList.toggle("active", item.dataset.section === name);
    }
    for (const section of this.el.modal.querySelectorAll(".settings-section")) {
      section.hidden = section.dataset.section !== name;
    }
  }

  async open() {
    this.el.modal.hidden = false;
    this.setMessage("");
    this.switchSection("general");
    if (this.reconnectButton) {
      this.reconnectButton.hidden = true;
    }
    try {
      const fetched = normalizeSettings(await getJson("/api/settings"));
      this.fill(fetched);
    } catch (error) {
      this.fill(this.current);
      this.setMessage(t("settings.loadFailed", { detail: errorText(error) }), "error");
    }
  }

  close() {
    this.el.modal.hidden = true;
    this.setMessage("");
  }

  cancel() {
    this.fill(this.current);
    this.close();
  }

  /// 应用启动时读取并应用一次服务端设置；失败回退默认值（仍会应用）。
  async loadInitial() {
    try {
      this.current = normalizeSettings(await getJson("/api/settings"));
    } catch (error) {
      this.current = { ...SETTINGS_DEFAULTS };
    }
    this.fill(this.current);
    this.onChanged?.(this.current);
  }

  fill(settings) {
    this.el.fields.baudRate.value = settings.baud_rate;
    this.el.fields.autoBaud.checked = Boolean(settings.auto_baud);
    this.el.fields.previewFps.value = settings.preview_fps;
    this.el.fields.mouseMode.value = settings.mouse_mode;
    this.el.fields.mouseProfile.value = settings.mouse_profile;
    this.el.fields.relativeSensitivity.value = settings.relative_sensitivity;
    this.el.fields.scaleMode.value = settings.scale_mode;
  }

  read() {
    return {
      baud_rate: Number(this.el.fields.baudRate.value),
      auto_baud: this.el.fields.autoBaud.checked,
      preview_fps: Number(this.el.fields.previewFps.value),
      mouse_profile: profileForForm(this.el.fields.mouseProfile.value, this.el.fields.mouseMode.value),
      mouse_mode: modeForProfile(this.el.fields.mouseProfile.value),
      relative_sensitivity: Number(this.el.fields.relativeSensitivity.value),
      scale_mode: this.el.fields.scaleMode.value,
    };
  }

  validate(settings) {
    if (!Number.isInteger(settings.baud_rate) || settings.baud_rate < 1200 || settings.baud_rate > 115200) {
      return t("settings.error.baudRate");
    }
    if (!Number.isInteger(settings.preview_fps) || settings.preview_fps < 1 || settings.preview_fps > 60) {
      return t("settings.error.previewFps");
    }
    if (!Number.isFinite(settings.relative_sensitivity) ||
        settings.relative_sensitivity < 0.1 ||
        settings.relative_sensitivity > 5.0) {
      return t("settings.error.relativeSensitivity");
    }
    if (!MOUSE_MODES.has(settings.mouse_mode)) {
      return t("settings.error.mouseMode", { mode: settings.mouse_mode });
    }
    if (!MOUSE_PROFILES.has(settings.mouse_profile)) {
      return t("settings.error.mouseProfile", { profile: settings.mouse_profile });
    }
    if (!SCALE_MODES.has(settings.scale_mode)) {
      return t("settings.error.scaleMode", { mode: settings.scale_mode });
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
      // 设备类参数（波特率）只在建立新连接时读取；已连接时改了要提示一键重连。
      const deviceChanged =
        saved.baud_rate !== this.current.baud_rate ||
        saved.auto_baud !== this.current.auto_baud;
      this.current = saved;
      this.onChanged?.(saved);
      if (deviceChanged && this.isConnected() && this.reconnectButton) {
        this.setMessage(t("settings.savedReconnect"), "ok");
        this.reconnectButton.hidden = false;
        this.switchSection("device");
      } else {
        this.setMessage(t("settings.saved"), "ok");
        this.close();
      }
    } catch (error) {
      this.setMessage(t("settings.saveFailed", { detail: errorText(error) }), "error");
    }
  }

  reset() {
    this.fill({ ...SETTINGS_DEFAULTS });
    this.setMessage("");
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
    mouse_profile: MOUSE_PROFILES.has(source.mouse_profile)
      ? source.mouse_profile
      : profileForMode(source.mouse_mode),
    mouse_mode: MOUSE_PROFILES.has(source.mouse_profile)
      ? modeForProfile(source.mouse_profile)
      : MOUSE_MODES.has(source.mouse_mode)
        ? source.mouse_mode
        : modeForProfile(source.mouse_profile),
    relative_sensitivity: Number.isFinite(Number(source.relative_sensitivity))
      ? Number(source.relative_sensitivity)
      : SETTINGS_DEFAULTS.relative_sensitivity,
    scale_mode: SCALE_MODES.has(source.scale_mode)
      ? source.scale_mode
      : SETTINGS_DEFAULTS.scale_mode,
  };
}

export function profileForMode(mode) {
  return mode === "relative" ? "raw_relative" : "raw_absolute";
}

export function modeForProfile(profile) {
  return profile === "linux" || profile === "raw_relative" ? "relative" : "absolute";
}

function profileForForm(profile, mode) {
  if (mode === "relative" && profile === "raw_absolute") {
    return "raw_relative";
  }
  if (mode === "absolute" && profile === "raw_relative") {
    return "raw_absolute";
  }
  return MOUSE_PROFILES.has(profile) ? profile : profileForMode(mode);
}
