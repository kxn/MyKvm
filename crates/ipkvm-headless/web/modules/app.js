// 装配入口：工具栏、视图状态机、RFB 生命周期与各功能模块接线。

import RFB from "/vendor/novnc/core/rfb.js";
import { errorText, postJson } from "./api.js";
import { pasteFromClipboard } from "./clipboard.js";
import { ConnectionController } from "./connection.js";
import { applyLanguage, getStoredLanguage, setLanguage, t } from "./i18n.js";
import { installKeyboardInterceptor } from "./keyboard.js";
import { PointerController } from "./pointer.js";
import { ScreenshotController } from "./screenshot.js";
import { SETTINGS_DEFAULTS, SettingsController, modeForProfile } from "./settings.js";
import { SpecialKeysController } from "./special-keys.js";
import { REASON, StatusController, VIEW } from "./status.js";
import { StatusPanel } from "./status-panel.js";

export function initApp(root) {
  const el = {
    console: root,
    connectionStatus: root.querySelector("#connection-status"),
    toolbarDisconnect: root.querySelector("#toolbar-disconnect"),
    openSettings: root.querySelector("#open-settings"),
    specialKeysButton: root.querySelector("#special-keys-button"),
    specialKeysMenu: root.querySelector("#special-keys-menu"),
    screenshotButton: root.querySelector("#screenshot-button"),
    screenshotMenu: root.querySelector("#screenshot-menu"),
    saveScreenshot: root.querySelector("#save-screenshot"),
    copyScreenshot: root.querySelector("#copy-screenshot"),
    languageSelect: root.querySelector("#language-select"),
    connectionView: root.querySelector("#connection-view"),
    videoView: root.querySelector("#video-view"),
    screen: root.querySelector("#screen"),
    connectButton: root.querySelector("#connect-button"),
    videoSelect: root.querySelector("#video-select"),
    serialSelect: root.querySelector("#serial-select"),
    connectionMouseProfile: root.querySelector("#connection-mouse-profile"),
    refreshVideo: root.querySelector("#refresh-video"),
    refreshSerial: root.querySelector("#refresh-serial"),
    videoProbe: root.querySelector("#video-probe"),
    serialProbe: root.querySelector("#serial-probe"),
    connectionHint: root.querySelector("#connection-hint"),
    connectionMessage: root.querySelector("#connection-message"),
    settingsSummary: root.querySelector("#connection-settings-summary"),
    videoResolution: root.querySelector("#video-resolution"),
    videoDevice: root.querySelector("#video-device"),
    sessionState: root.querySelector("#session-state"),
    serialStats: root.querySelector("#serial-stats"),
    inputStats: root.querySelector("#input-stats"),
    videoMouseProfile: root.querySelector("#video-mouse-profile"),
    videoMessage: root.querySelector("#video-message"),
    pasteButton: root.querySelector("#paste-button"),
    relativeMode: root.querySelector("#relative-mode"),
    settingsModal: root.querySelector("#settings-modal"),
    settingsMessage: root.querySelector("#settings-message"),
    settingsFields: {
      baudRate: root.querySelector("#setting-baud-rate"),
      autoBaud: root.querySelector("#setting-auto-baud"),
      previewFps: root.querySelector("#setting-preview-fps"),
      mouseMode: root.querySelector("#setting-mouse-mode"),
      mouseProfile: root.querySelector("#setting-mouse-profile"),
      relativeSensitivity: root.querySelector("#setting-relative-sensitivity"),
      scaleMode: root.querySelector("#setting-scale-mode"),
    },
  };

  let rfb = null;
  let rfbDom = null;
  let rfbState = "idle";
  let rfbBackoffMs = 2000;
  let rfbNextRetryAt = 0;
  let settings = { ...SETTINGS_DEFAULTS };
  let lastStatus = null;
  let currentReason = null;
  let profileChangePending = false;
  let rfbNeedsInputReconnect = false;

  const setConnectionState = (state, text) => {
    root.dataset.connectionState = state;
    el.connectionStatus.textContent = text ?? "";
  };

  let messageTimer = null;
  const message = (text, level) => {
    if (messageTimer) {
      clearTimeout(messageTimer);
      messageTimer = null;
    }
    el.videoMessage.textContent = text;
    if (level) {
      el.videoMessage.dataset.level = level;
    } else {
      delete el.videoMessage.dataset.level;
    }
    // 所有消息自动消失：ok 3秒，error 5秒，无level 3秒
    const delay = level === "error" ? 5000 : 3000;
    messageTimer = setTimeout(() => {
      el.videoMessage.textContent = "";
      delete el.videoMessage.dataset.level;
      messageTimer = null;
    }, delay);
  };

  const websocketUrl = () => {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    // token 在服务端配置校验中已限制为 RFC 3986 无保留字符（字母数字与 -_.~），
    // 因此 encodeURIComponent 对它是恒等变换；服务端按原始字节比较、不做 URL 解码。
    const token = new URLSearchParams(location.search).get("token");
    const query = token ? `?token=${encodeURIComponent(token)}` : "";
    return `${scheme}://${location.host}/rfb${query}`;
  };

  const applyScaleMode = () => {
    if (!rfb) {
      return;
    }
    const mode = settings.scale_mode ?? "fit_window";
    rfb.scaleViewport = mode === "fit_window";
    rfb.clipViewport = mode === "fit_window" || mode === "follow_window";
    rfb.resizeSession = mode === "follow_window";
  };

  const cleanupRfbDom = () => {
    if (rfbDom) {
      rfbDom.remove();
      rfbDom = null;
    }
  };

  /// 按状态驱动 RFB 生命周期：控制器被其它客户端占用时不建实例并提示；
  /// 失败后按指数退避重试；切回连接页时显式断开并停止自动重连。
  const syncRfbWithStatus = (status) => {
    if (status?.session?.state !== "running") {
      rfbNeedsInputReconnect = false;
      return;
    }
    if (pointer.locked) {
      // 相对模式（pointer lock 激活）期间不得触发任何重建/重连。
      return;
    }
    if (el.videoView.hidden) {
      // 手动切回连接页（会话仍 running）：不自动重连；保持 idle 等待用户操作。
      if (rfb || rfbState === "connecting") {
        disconnectRfb();
      } else {
        rfbState = "idle";
      }
      return;
    }
    if (rfb || rfbState === "connecting") {
      return;
    }
    if (status?.controller?.active) {
      if (rfbState !== "busy") {
        rfbState = "busy";
        cleanupRfbDom();
        message(t("video.controllerBusy"), "error");
      }
      return;
    }
    if (rfbState === "failed" && Date.now() < rfbNextRetryAt) {
      return;
    }
    if (rfbState !== "idle" && rfbState !== "failed" && rfbState !== "busy") {
      return;
    }
    rfbState = "idle";
    connectRfb();
  };

  const syncRfbInputRecovery = (status) => {
    const controlState = status?.session?.control?.state;
    if (status?.session?.state !== "running" || !controlState) {
      rfbNeedsInputReconnect = false;
      return;
    }
    if (controlState !== "ready") {
      rfbNeedsInputReconnect = true;
      return;
    }
    if (!rfbNeedsInputReconnect) {
      return;
    }
    if (rfb && pointer.locked) {
      return;
    }
    rfbNeedsInputReconnect = false;
    if (rfb && !el.videoView.hidden) {
      disconnectRfb();
    }
  };

  const connectRfb = () => {
    if (rfb || rfbState === "connecting") {
      return;
    }
    cleanupRfbDom();
    rfbState = "connecting";
    setConnectionState("connecting", t("status.connecting"));
    try {
      const next = new RFB(el.screen, websocketUrl(), { shared: true });
      rfb = next;
      rfbDom = next.screenElement ?? null;
      next.scaleViewport = true;
      next.resizeSession = false;
      next.focusOnClick = true;
      pointer.setRfb(next);
      specialKeys.updateEnabled();
      applyScaleMode();

      next.addEventListener("connect", () => {
        if (rfb !== next) {
          return;
        }
        rfbState = "connected";
        rfbBackoffMs = 2000;
        rfbNextRetryAt = 0;
        setConnectionState("connected", t("status.connected"));
        message(t("video.rfbConnected"), "ok");
        specialKeys.updateEnabled();
      });

      next.addEventListener("disconnect", (event) => {
        if (rfb !== next) {
          return;
        }
        const clean = Boolean(event?.detail?.clean);
        cleanupRfbDom();
        rfb = null;
        if (clean) {
          rfbState = "idle";
          message(t("video.rfbDisconnected"));
        } else {
          rfbState = "failed";
          rfbNextRetryAt = Date.now() + rfbBackoffMs;
          rfbBackoffMs = Math.min(rfbBackoffMs * 2, 30000);
          message(t("video.rfbFailed"), "error");
        }
        pointer.setRfb(null);
        keyboard.releaseLocalState();
        specialKeys.updateEnabled();
        if (lastStatus?.session?.state !== "running") {
          setConnectionState("disconnected", t("status.disconnected"));
        } else {
          setConnectionState("failed", t("status.connectionFailed"));
        }
      });

      next.addEventListener("securityfailure", () => {
        message(t("video.rfbFailed"), "error");
        next.disconnect();
      });

      next.addEventListener("credentialsrequired", () => {
        message(t("video.rfbFailed"), "error");
        next.disconnect();
      });
    } catch (error) {
      rfbState = "failed";
      rfbNextRetryAt = Date.now() + rfbBackoffMs;
      rfbBackoffMs = Math.min(rfbBackoffMs * 2, 30000);
      rfb = null;
      cleanupRfbDom();
      pointer.setRfb(null);
      message(`${t("video.rfbFailed")}：${errorText(error)}`, "error");
    }
  };

  const disconnectRfb = () => {
    if (rfb) {
      const current = rfb;
      rfb = null;
      rfbState = "idle";
      pointer.setRfb(null);
      specialKeys.updateEnabled();
      current.disconnect();
      keyboard.releaseLocalState();
    }
    cleanupRfbDom();
    rfbState = "idle";
    rfbNextRetryAt = 0;
    rfbBackoffMs = 2000;
  };

  const showConnectionReason = (reason) => {
    currentReason = reason;
    const hint = {
      [REASON.ABSENT]: t("connection.hint.initial"),
      [REASON.STOPPED]: t("connection.hint.stopped"),
      [REASON.MANUAL_STOP]: t("connection.hint.manualStop"),
      [REASON.RECOVERING]: t("connection.hint.recovering"),
      [REASON.SWITCH]: t("connection.hint.switch"),
    }[reason] ?? t("connection.hint.initial");
    el.connectionHint.textContent = hint;
    if (reason === REASON.MANUAL_STOP) {
      el.connectionMessage.textContent = t("status.manualStop");
      el.connectionMessage.dataset.level = "error";
    } else if (reason === REASON.RECOVERING) {
      el.connectionMessage.textContent = t("status.recovering");
      el.connectionMessage.dataset.level = "error";
    } else if (reason === REASON.STOPPED || reason === REASON.ABSENT) {
      el.connectionMessage.textContent = t("status.disconnected");
      delete el.connectionMessage.dataset.level;
    } else {
      el.connectionMessage.textContent = "";
      delete el.connectionMessage.dataset.level;
    }
  };

  const setView = (view, reason) => {
    const wasVideo = !el.videoView.hidden;
    el.connectionView.hidden = view !== VIEW.CONNECTION;
    el.videoView.hidden = view !== VIEW.VIDEO;
    root.dataset.view = view;
    if (view === VIEW.VIDEO) {
      syncRfbWithStatus(lastStatus);
      connection.updateConnectState();
    } else {
      disconnectRfb();
      pointer.exit();
      connection.updateConnectState();
      showConnectionReason(reason ?? currentReason ?? REASON.ABSENT);
      if (wasVideo && reason === REASON.MANUAL_STOP) {
        el.connectionMessage.textContent = `${t("status.manualStop")}（${t("status.synced")}）`;
      }
      // 切换到连接页时自动刷新设备列表
      connection.refreshAll();
    }
  };

  const updateToolbar = (status) => {
    const state = status?.session?.state;
    const manualStop = Boolean(status?.session?.manual_stop);
    if (state === "running") {
      setConnectionState("connected", t("status.connected"));
    } else if (state === "absent") {
      setConnectionState("unknown", t("status.unknown"));
    } else if (manualStop) {
      setConnectionState("disconnected", t("status.manualStop"));
    } else {
      setConnectionState("disconnected", t("status.disconnected"));
    }
    el.toolbarDisconnect.disabled = state !== "running";
  };

  const updateVideoBar = (status) => {
    const frame = status?.video?.frame;
    el.videoResolution.textContent = frame
      ? t("video.resolution", { w: frame.width, h: frame.height })
      : t("video.noFrame");
    el.videoDevice.textContent = t("video.device", {
      name: status?.video?.source?.device_name ?? "-",
    });
    el.sessionState.textContent = t("video.session", {
      state: status?.session?.state ?? "-",
    });
    const serial = status?.session?.serial;
    el.serialStats.textContent = serial
      ? t("video.serial", {
          batches: serial.batches_accepted ?? 0,
          frames: serial.frames_accepted ?? 0,
        })
      : "";
    const control = status?.session?.control;
    if (
      control &&
      !["ready", "idle"].includes(control.state) &&
      control.reason
    ) {
      el.inputStats.textContent = t("video.inputOffline", {
        state: t(`video.runtime.${control.state}`),
        reason: control.reason,
      });
    } else {
      el.inputStats.textContent = t("video.input", {
        events: status?.session?.input_events ?? 0,
      });
    }
    if (status?.session?.mouse_profile && !profileChangePending) {
      el.videoMouseProfile.value = status.session.mouse_profile;
    }
    el.videoMouseProfile.disabled = status?.session?.state !== "running";
  };

  const connection = new ConnectionController({
    elements: el,
    getStatus: () => lastStatus,
    onConnected: () => statusController.clearViewOverride(),
    onMessage: (text, level) => {
      el.connectionMessage.textContent = text;
      if (level) {
        el.connectionMessage.dataset.level = level;
      } else {
        delete el.connectionMessage.dataset.level;
      }
    },
    onSettingsSummary: (text) => {
      el.settingsSummary.textContent = text;
    },
  });

  const settingsController = new SettingsController({
    modal: el.settingsModal,
    message: el.settingsMessage,
    fields: el.settingsFields,
    openButton: el.openSettings,
    cancelButton: root.querySelector("#settings-cancel"),
    saveButton: root.querySelector("#settings-save"),
    resetButton: root.querySelector("#settings-reset"),
    onChanged: (saved) => {
      settings = saved;
      applyScaleMode();
      pointer.applySettings(saved);
      connection.updateSettingsSummary(saved);
    },
  });

  const pointer = new PointerController({
    button: el.relativeMode,
    message,
    getRfb: () => rfb,
  });

  const keyboard = installKeyboardInterceptor({ getRfb: () => rfb });
  el.openSettings.addEventListener("click", () => keyboard.releaseRemoteState(), true);

  const applySessionProfile = async () => {
    const profile = el.videoMouseProfile.value;
    const previous = lastStatus?.session?.mouse_profile ?? settings.mouse_profile;
    profileChangePending = true;
    el.videoMouseProfile.disabled = true;
    try {
      const result = await postJson("/api/input/mouse-profile", {
        mouse_profile: profile,
      });
      const applied = {
        ...settings,
        mouse_profile: result.mouse_profile,
        mouse_mode: result.mouse_mode ?? modeForProfile(result.mouse_profile),
      };
      pointer.applySettings(applied);
      message(t("video.profileChanged"), "ok");
    } catch (error) {
      el.videoMouseProfile.value = previous;
      message(`${t("video.profileChangeFailed")}：${errorText(error)}`, "error");
    } finally {
      profileChangePending = false;
      el.videoMouseProfile.disabled = lastStatus?.session?.state !== "running";
    }
  };

  el.videoMouseProfile.addEventListener("change", () => {
    void applySessionProfile();
  });

  const specialKeys = new SpecialKeysController({
    button: el.specialKeysButton,
    menu: el.specialKeysMenu,
    message,
    getRfb: () => rfb,
    heldModifiers: () => keyboard.heldModifiers(),
  });

  const screenshot = new ScreenshotController({
    menuButton: el.screenshotButton,
    menu: el.screenshotMenu,
    saveButton: el.saveScreenshot,
    copyButton: el.copyScreenshot,
    message,
  });

  const statusPanel = new StatusPanel({
    button: root.querySelector("#status-panel-button"),
    message,
  });

  const statusController = new StatusController({
    onStatus: (status) => {
      lastStatus = status;
      if (status?.session?.mouse_profile) {
        pointer.applySettings({
          ...settings,
          mouse_profile: status.session.mouse_profile,
          mouse_mode:
            status.session.mouse_mode ?? modeForProfile(status.session.mouse_profile),
        });
      }
      updateToolbar(status);
      updateVideoBar(status);
      connection.applyStatus(status);
      screenshot.setFrameAvailable(Boolean(status.video?.frame));
      connection.updateConnectState();
      syncRfbInputRecovery(status);
      syncRfbWithStatus(status);
    },
    onViewChange: (view, reason) => setView(view, reason),
    onError: (error) => {
      setConnectionState("failed", t("status.error"));
      message(`${t("status.error")}：${errorText(error)}`, "error");
    },
  });

  el.toolbarDisconnect.addEventListener("click", async () => {
    el.toolbarDisconnect.disabled = true;
    try {
      await postJson("/api/session", { action: "stop" });
    } catch (error) {
      message(`${t("toolbar.disconnect")}：${errorText(error)}`, "error");
      el.toolbarDisconnect.disabled = lastStatus?.session?.state === "running";
    }
  });

  el.pasteButton.addEventListener("click", async () => {
    if (!rfb) {
      return;
    }
    try {
      await pasteFromClipboard(rfb);
      message(t("clipboard.pasteOk"), "ok");
    } catch (error) {
      message(t("clipboard.pasteFail", { detail: errorText(error) }), "error");
    }
  });

  const applyUiLanguage = () => {
    applyLanguage(document);
    el.languageSelect.value = getStoredLanguage();
    specialKeys.refresh();
    pointer.update();
    connection.updateSettingsSummary(settings);
    if (!el.videoView.hidden) {
      updateVideoBar(lastStatus);
    } else {
      showConnectionReason(currentReason ?? REASON.ABSENT);
    }
  };

  el.languageSelect.addEventListener("change", () => {
    setLanguage(el.languageSelect.value);
    applyUiLanguage();
  });

  window.addEventListener("beforeunload", () => {
    rfb?.disconnect();
  });

  applyUiLanguage();
  statusController.start();
  connection.refreshAll();
  settingsController.loadInitial();
  // 初始视图：如果会话已运行，显示视频页；否则显示连接页
  const initialStatus = statusController.status;
  const initialState = initialStatus?.session?.state;
  if (initialState === "running") {
    setView(VIEW.VIDEO, null);
  } else {
    setView(VIEW.CONNECTION, REASON.ABSENT);
  }
}
