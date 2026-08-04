// 连接页：设备枚举/选择/探测状态与 create/restart 连接。

import { errorText, getJson, postJson } from "./api.js";
import { t } from "./i18n.js";

export class ConnectionController {
  constructor({ elements, getStatus, onMessage, onSettingsSummary, onConnected }) {
    this.el = elements;
    this.getStatus = getStatus;
    this.onMessage = onMessage;
    this.onSettingsSummary = onSettingsSummary;
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
      probe.dataset.state = items.length > 0 ? "ready" : "empty";
      probe.textContent =
        items.length > 0
          ? t("connection.ready", { count: items.length })
          : t("connection.empty");
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
    const session = this.getStatus()?.session;
    const hasPreviousSession =
      session != null && (session.state === "running" || session.state === "stopped");
    const fallbackToPrevious =
      hasPreviousSession &&
      this.devices.video.length === 0 &&
      this.devices.serial.length === 0;
    this.el.connectButton.disabled = !(videoOk && serialOk) && !fallbackToPrevious;
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
        mouse_profile: this.el.connectionMouseProfile.value || null,
      });
      this.onConnected?.();
    } catch (error) {
      this.onMessage(`${t("connection.connectFailed")}：${errorText(error)}`, "error");
      this.updateConnectState();
    }
  }

  updateSettingsSummary(settings) {
    if (!this.profileDirty && settings?.mouse_profile) {
      this.el.connectionMouseProfile.value = settings.mouse_profile;
    }
    const text = t("connection.settingsSummary", {
      baud: settings?.baud_rate ?? "-",
      auto: settings?.auto_baud ? t("common.on") : t("common.off"),
      fps: settings?.preview_fps ?? "-",
      profile: settings?.mouse_profile ?? "raw_absolute",
    });
    this.onSettingsSummary?.(text);
  }

  applyStatus(status) {
    if (!this.profileDirty && status?.session?.mouse_profile) {
      this.el.connectionMouseProfile.value = status.session.mouse_profile;
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
