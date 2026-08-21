// 连接页：设备枚举/选择/探测状态与 create/restart 连接。
// 向导式布局：主路径只暴露设备与目标系统；坐标模式与设备参数收进高级折叠区，
// 设备参数只读展示并引导到设置对应分区修改（#103）。

import { errorText, getJson, postJson } from "./api.js";
import { t } from "./i18n.js";

const RAW_COORDINATE_MODES = new Set(["raw_absolute", "raw_relative"]);

export class ConnectionController {
  constructor({ elements, getStatus, onMessage, onOpenSettings, onConnected }) {
    this.el = elements;
    this.getStatus = getStatus;
    this.onMessage = onMessage;
    this.onOpenSettings = onOpenSettings;
    this.onConnected = onConnected;
    this.devices = { video: [], serial: [] };
    this.profileDirty = false;

    this.el.refreshVideo.addEventListener("click", () => this.refresh("video"));
    this.el.refreshSerial.addEventListener("click", () => this.refresh("serial"));
    this.el.connectButton.addEventListener("click", () => this.connect());
    this.el.videoSelect.addEventListener("change", () => this.updateConnectState());
    this.el.serialSelect.addEventListener("change", () => this.updateConnectState());
    this.el.connectionMouseProfile.addEventListener("change", () => {
      this.profileDirty = true;
    });
    this.el.coordinateMode.addEventListener("change", () => {
      this.profileDirty = true;
      this.syncProfileControls();
    });
    this.el.advanced?.addEventListener("click", (event) => {
      const button = event.target.closest("[data-goto-section]");
      if (button) {
        this.onOpenSettings?.(button.dataset.gotoSection);
      }
    });
  }

  async refreshAll() {
    await Promise.all([this.refresh("video"), this.refresh("serial")]);
  }

  async refresh(kind) {
    const select = kind === "video" ? this.el.videoSelect : this.el.serialSelect;
    const probe = kind === "video" ? this.el.videoProbe : this.el.serialProbe;
    probe.dataset.state = "probing";
    probe.textContent = t("connection.probing");
    try {
      const data = await getJson("/api/devices");
      const items = data?.[kind] ?? [];
      this.devices[kind] = items;
      fillSelect(select, items);
      // 成功路径不展示技术细节；只有未发现设备或失败才内联提示（#103）。
      if (items.length > 0) {
        probe.dataset.state = "ready";
        probe.textContent = "";
      } else {
        probe.dataset.state = "empty";
        probe.textContent = t("connection.empty");
      }
    } catch (error) {
      probe.dataset.state = "failed";
      probe.textContent = `${t("connection.probeFailed")}：${errorText(error)}`;
    } finally {
      this.updateConnectState();
    }
  }

  updateConnectState() {
    const videoOk = this.el.videoSelect.value !== "";
    const serialOk = this.el.serialSelect.value !== "";
    // 后端契约：session.state 只有 running/absent 两值（手动停止为 absent + manual_stop）。
    const session = this.getStatus()?.session;
    const hasPreviousSession = session != null && session.state === "running";
    const fallbackToPrevious =
      hasPreviousSession &&
      this.devices.video.length === 0 &&
      this.devices.serial.length === 0;
    this.el.connectButton.disabled = !(videoOk && serialOk) && !fallbackToPrevious;
  }

  /// 当前应提交的 mouse_profile：坐标模式为原始坐标时覆盖目标系统选择。
  currentProfile() {
    const mode = this.el.coordinateMode.value;
    return mode === "follow" ? this.el.connectionMouseProfile.value : mode;
  }

  syncProfileControls() {
    this.el.connectionMouseProfile.disabled = this.el.coordinateMode.value !== "follow";
  }

  applyProfile(profile) {
    if (RAW_COORDINATE_MODES.has(profile)) {
      this.el.coordinateMode.value = profile;
    } else if (profile) {
      this.el.coordinateMode.value = "follow";
      this.el.connectionMouseProfile.value = profile;
    }
    this.syncProfileControls();
  }

  async connect() {
    const status = this.getStatus();
    const state = status?.session?.state;
    const action = state === "absent" ? "create" : "restart";
    const video = this.el.videoSelect.value || null;
    const serial = this.el.serialSelect.value || null;
    this.el.connectButton.disabled = true;
    this.onMessage(
      t(action === "create" ? "connection.message.create" : "connection.message.restart"),
    );
    try {
      await postJson("/api/session", {
        action,
        video,
        serial,
        mouse_profile: this.currentProfile() || null,
      });
      this.onConnected?.();
    } catch (error) {
      this.onMessage(`${t("connection.connectFailed")}：${errorText(error)}`, "error");
      this.updateConnectState();
    }
  }

  /// 应用服务端设置：高级折叠区展示设备参数当前值（只读），并同步 profile 到控件。
  applySettings(settings) {
    if (!this.profileDirty && settings?.mouse_profile) {
      this.applyProfile(settings.mouse_profile);
    }
    if (this.el.advancedBaud) {
      this.el.advancedBaud.textContent = settings?.baud_rate ?? "-";
    }
    if (this.el.advancedFps) {
      this.el.advancedFps.textContent = settings?.preview_fps ?? "-";
    }
    if (this.el.advancedMouseMode) {
      const mode = settings?.mouse_mode;
      this.el.advancedMouseMode.textContent = mode
        ? t(mode === "relative" ? "settings.relative" : "settings.absolute")
        : "-";
    }
  }

  applyStatus(status) {
    if (!this.profileDirty && status?.session?.mouse_profile) {
      this.applyProfile(status.session.mouse_profile);
    }
  }
}

function fillSelect(select, items) {
  select.textContent = "";
  for (const item of items) {
    const option = document.createElement("option");
    option.value = item.id;
    option.textContent = item.display_name;
    select.appendChild(option);
  }
}
