// 装配入口：悬浮控制条、视图状态机、RFB 生命周期与各功能模块接线。

import RFB from "/vendor/novnc/core/rfb.js";
import { errorText, postJson } from "./api.js";
import { ConnectionController } from "./connection.js";
import { ControlBar } from "./controlbar.js";
import { applyLanguage, getStoredLanguage, setLanguage, t } from "./i18n.js";
import { installKeyboardInterceptor } from "./keyboard.js";
import { PointerController } from "./pointer.js";
import { PasteDialog } from "./paste.js";
import { ScreenshotController } from "./screenshot.js";
import { SETTINGS_DEFAULTS, SettingsController, modeForProfile } from "./settings.js";
import { SpecialKeysController } from "./special-keys.js";
import { REASON, StatusController, VIEW } from "./status.js";
import { StatusPanel } from "./status-panel.js";
import { getStoredTheme, initTheme, setTheme } from "./theme.js";

const SCALE_CYCLE = ["fit_window", "original", "follow_window"];
const STATUS_LINE_KEY = "my_ipkvm.statusLine";
const SCALE_I18N_KEYS = {
  fit_window: "settings.fitWindow",
  original: "settings.original",
  follow_window: "settings.followWindow",
};
const PROFILE_I18N_KEYS = {
  windows: "settings.profileWindows",
  linux: "settings.profileLinux",
  bios: "settings.profileBios",
  android: "settings.profileAndroid",
  macos: "settings.profileMacos",
  raw_absolute: "settings.profileRawAbsolute",
  raw_relative: "settings.profileRawRelative",
};

const profileLabel = (profile) =>
  PROFILE_I18N_KEYS[profile] ? t(PROFILE_I18N_KEYS[profile]) : (profile ?? "");

/// 状态行显隐偏好：默认显示，仅 "0" 视为隐藏。
function getStoredStatusLine() {
  return localStorage.getItem(STATUS_LINE_KEY) !== "0";
}

export function initApp(root) {
  const el = {
    console: root,
    connectionStatus: root.querySelector("#connection-status"),
    controlBar: root.querySelector("#control-bar"),
    sessionMenuButton: root.querySelector("#session-menu-button"),
    sessionMenu: root.querySelector("#session-menu"),
    controlTarget: root.querySelector("#control-target"),
    barReconnect: root.querySelector("#bar-reconnect"),
    barPin: root.querySelector("#bar-pin"),
    moreButton: root.querySelector("#more-button"),
    moreMenu: root.querySelector("#more-menu"),
    toolbarDisconnect: root.querySelector("#toolbar-disconnect"),
    openSettings: root.querySelector("#open-settings"),
    specialKeysButton: root.querySelector("#special-keys-button"),
    specialKeysMenu: root.querySelector("#special-keys-menu"),
    screenshotButton: root.querySelector("#screenshot-button"),
    screenshotMenu: root.querySelector("#screenshot-menu"),
    saveScreenshot: root.querySelector("#save-screenshot"),
    copyScreenshot: root.querySelector("#copy-screenshot"),
    scaleModeButton: root.querySelector("#scale-mode-button"),
    scaleModeLabel: root.querySelector("#scale-mode-label"),
    fullscreenButton: root.querySelector("#fullscreen-button"),
    sensitivitySlider: root.querySelector("#relative-sensitivity-slider"),
    sensitivityValue: root.querySelector("#sensitivity-value"),
    languageSelect: root.querySelector("#language-select"),
    themeSelect: root.querySelector("#theme-select"),
    connectionView: root.querySelector("#connection-view"),
    videoView: root.querySelector("#video-view"),
    degradedBanner: root.querySelector("#degraded-banner"),
    degradedBannerText: root.querySelector("#degraded-banner-text"),
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
    videoFps: root.querySelector("#video-fps"),
    statusProfile: root.querySelector("#status-profile"),
    videoMouseProfile: root.querySelector("#video-mouse-profile"),
    videoMessage: root.querySelector("#video-message"),
    pasteButton: root.querySelector("#paste-button"),
    relativeMode: root.querySelector("#relative-mode"),
    settingsModal: root.querySelector("#settings-modal"),
    settingsMessage: root.querySelector("#settings-message"),
    settingsNav: root.querySelector(".settings-nav"),
    settingsReconnect: root.querySelector("#settings-reconnect"),
    settingsVersion: root.querySelector("#settings-version"),
    settingsLanguage: root.querySelector("#setting-language"),
    settingsTheme: root.querySelector("#setting-theme"),
    statusLineToggle: root.querySelector("#setting-status-line"),
    statusLineBar: root.querySelector("#video-status-bar"),
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
  let lastFrameInfo = null;
  let fpsHistory = [];

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

  /// degraded 横幅：视频/输入链路异常或会话恢复中时在画面内顶部提示，
  /// 不切换视图；手动停止仍由视图状态机切回连接页。
  const updateBanner = () => {
    let text = null;
    if (!el.videoView.hidden) {
      if (rfbState === "busy") {
        text = t("video.controllerBusy");
      } else if (rfbState === "failed") {
        text = t("banner.videoRecovering");
      } else {
        const session = lastStatus?.session;
        const control = session?.control;
        if (session && session.state !== "running") {
          text = t("banner.sessionRecovering");
        } else if (control && !["ready", "idle"].includes(control.state)) {
          text = t("banner.inputOffline", {
            state: t(`video.runtime.${control.state}`),
            reason: control.reason ?? "-",
          });
        }
      }
    }
    el.degradedBanner.hidden = text === null;
    root.dataset.degraded = text === null ? "false" : "true";
    if (text !== null) {
      el.degradedBannerText.textContent = text;
    }
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
        updateBanner();
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
    updateBanner();
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
        updateBanner();
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
        updateBanner();
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
      updateBanner();
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
    updateBanner();
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
    controlBar.setView(view);
    if (view !== VIEW.VIDEO) {
      lastFrameInfo = null;
      fpsHistory = [];
    }
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
    updateBanner();
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

  /// 极简状态行：分辨率 · 帧率 · 当前目标系统；诊断数据移入状态浮层。
  const updateVideoBar = (status) => {
    const frame = status?.video?.frame;
    el.videoResolution.textContent = frame
      ? t("video.resolutionShort", { w: frame.width, h: frame.height })
      : t("video.noFrame");
    if (frame && lastFrameInfo && frame.capture_ns > lastFrameInfo.captureNs) {
      const fps =
        (frame.seq - lastFrameInfo.seq) /
        ((frame.capture_ns - lastFrameInfo.captureNs) / 1e9);
      if (Number.isFinite(fps) && fps >= 0) {
        fpsHistory.push(fps);
        if (fpsHistory.length > 3) {
          fpsHistory.shift();
        }
      }
    }
    if (frame) {
      lastFrameInfo = { seq: frame.seq, captureNs: frame.capture_ns };
    }
    el.videoFps.textContent =
      fpsHistory.length > 0
        ? t("video.fps", {
            fps: (
              fpsHistory.reduce((sum, value) => sum + value, 0) / fpsHistory.length
            ).toFixed(1),
          })
        : "";
    el.controlTarget.textContent =
      status?.video?.source?.device_name ?? "MyKvm";
    const profile = status?.session?.mouse_profile;
    el.statusProfile.textContent = profile ? profileLabel(profile) : "";
    if (profile && !profileChangePending) {
      el.videoMouseProfile.value = profile;
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

  const syncSensitivitySlider = () => {
    const value = Number(settings.relative_sensitivity);
    if (Number.isFinite(value)) {
      el.sensitivitySlider.value = String(value);
      el.sensitivityValue.textContent = value.toFixed(1);
    }
  };

  const refreshScaleButton = () => {
    el.scaleModeLabel.textContent = t(
      SCALE_I18N_KEYS[settings.scale_mode] ?? "settings.fitWindow",
    );
  };

  const applySettingsChange = (saved) => {
    settings = saved;
    applyScaleMode();
    pointer.applySettings(saved);
    connection.updateSettingsSummary(saved);
    syncSensitivitySlider();
    refreshScaleButton();
  };

  /// 控制条快捷控件（缩放循环、灵敏度滑杆）直接持久化到 /api/settings。
  const persistSettings = async (updated) => {
    try {
      const saved = await postJson("/api/settings", updated);
      applySettingsChange(saved);
    } catch (error) {
      message(t("settings.saveFailed", { detail: errorText(error) }), "error");
    }
  };

  const settingsController = new SettingsController({
    modal: el.settingsModal,
    message: el.settingsMessage,
    fields: el.settingsFields,
    openButton: el.openSettings,
    cancelButton: root.querySelector("#settings-cancel"),
    saveButton: root.querySelector("#settings-save"),
    resetButton: root.querySelector("#settings-reset"),
    reconnectButton: el.settingsReconnect,
    nav: el.settingsNav,
    isConnected: () => lastStatus?.session?.state === "running",
    onReconnect: () => el.barReconnect.click(),
    onChanged: applySettingsChange,
  });

  const pointer = new PointerController({
    button: el.relativeMode,
    message,
    getRfb: () => rfb,
  });

  const keyboard = installKeyboardInterceptor({ getRfb: () => rfb });
  el.openSettings.addEventListener("click", () => keyboard.releaseRemoteState(), true);
  // 打开设置时同步纯前端偏好控件（语言/主题/状态行）与关于区版本。
  el.openSettings.addEventListener("click", () => {
    el.settingsLanguage.value = getStoredLanguage();
    el.settingsTheme.value = getStoredTheme();
    el.statusLineToggle.checked = getStoredStatusLine();
    el.settingsVersion.textContent = lastStatus?.service?.version ?? "-";
  });

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

  // degraded 横幅点击查看诊断（状态浮层）。
  el.degradedBanner.addEventListener("click", () => statusPanel.show());

  const controlBar = new ControlBar({
    consoleRoot: root,
    bar: el.controlBar,
    videoView: el.videoView,
    pinButton: el.barPin,
    sessionMenuButton: el.sessionMenuButton,
    sessionMenu: el.sessionMenu,
    moreButton: el.moreButton,
    moreMenu: el.moreMenu,
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
      updateBanner();
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
    controlBar.closeMenus();
    try {
      await postJson("/api/session", { action: "stop" });
    } catch (error) {
      message(`${t("toolbar.disconnect")}：${errorText(error)}`, "error");
      el.toolbarDisconnect.disabled = lastStatus?.session?.state === "running";
    }
  });

  // 重新连接：仅重建本页的 RFB 客户端连接，不重启服务端会话。
  el.barReconnect.addEventListener("click", () => {
    controlBar.closeMenus();
    disconnectRfb();
    syncRfbWithStatus(lastStatus);
  });

  const pasteDialog = new PasteDialog({
    modal: root.querySelector("#paste-modal"),
    text: root.querySelector("#paste-text"),
    count: root.querySelector("#paste-count"),
    hint: root.querySelector("#paste-hint"),
    warning: root.querySelector("#paste-warning"),
    sendButton: root.querySelector("#paste-send"),
    cancelButton: root.querySelector("#paste-cancel"),
    message,
    getRfb: () => rfb,
  });
  el.pasteButton.addEventListener("click", () => pasteDialog.open());

  // 缩放模式循环：适配窗口 → 原始大小 → 窗口跟随视频。
  el.scaleModeButton.addEventListener("click", () => {
    const index = SCALE_CYCLE.indexOf(settings.scale_mode);
    const next = SCALE_CYCLE[(index + 1) % SCALE_CYCLE.length];
    settings = { ...settings, scale_mode: next };
    applyScaleMode();
    refreshScaleButton();
    void persistSettings(settings);
  });

  // 相对灵敏度滑杆：即时生效，停止操作后持久化。
  let sensitivityPersistTimer = null;
  el.sensitivitySlider.addEventListener("input", () => {
    const value = Number(el.sensitivitySlider.value);
    if (!Number.isFinite(value)) {
      return;
    }
    el.sensitivityValue.textContent = value.toFixed(1);
    settings = { ...settings, relative_sensitivity: value };
    pointer.applySettings(settings);
    if (sensitivityPersistTimer !== null) {
      clearTimeout(sensitivityPersistTimer);
    }
    sensitivityPersistTimer = setTimeout(() => {
      sensitivityPersistTimer = null;
      void persistSettings(settings);
    }, 400);
  });

  // 浏览器全屏（Fullscreen API，纯前端能力）。
  el.fullscreenButton.addEventListener("click", async () => {
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else {
        await document.documentElement.requestFullscreen();
      }
    } catch (error) {
      message(t("bar.fullscreenFailed", { detail: errorText(error) }), "error");
    }
  });
  document.addEventListener("fullscreenchange", () => {
    el.fullscreenButton.title = t(
      document.fullscreenElement ? "bar.exitFullscreen" : "bar.fullscreen",
    );
  });

  const applyUiLanguage = () => {
    applyLanguage(document);
    el.languageSelect.value = getStoredLanguage();
    el.settingsLanguage.value = getStoredLanguage();
    specialKeys.refresh();
    pointer.update();
    connection.updateSettingsSummary(settings);
    refreshScaleButton();
    if (!el.videoView.hidden) {
      updateVideoBar(lastStatus);
    } else {
      showConnectionReason(currentReason ?? REASON.ABSENT);
    }
    updateBanner();
  };

  el.languageSelect.addEventListener("change", () => {
    setLanguage(el.languageSelect.value);
    applyUiLanguage();
  });
  // 设置「常规」分区里的语言/主题与 ⋯ 菜单快捷下拉写同一份偏好，双向同步。
  el.settingsLanguage.addEventListener("change", () => {
    setLanguage(el.settingsLanguage.value);
    applyUiLanguage();
  });
  el.settingsTheme.addEventListener("change", () => {
    setTheme(el.settingsTheme.value);
    el.themeSelect.value = getStoredTheme();
  });
  el.themeSelect.addEventListener("change", () => {
    el.settingsTheme.value = getStoredTheme();
  });

  // 状态行显隐：纯前端偏好，即时生效。
  const applyStatusLine = () => {
    el.statusLineBar.hidden = !getStoredStatusLine();
  };
  el.statusLineToggle.addEventListener("change", () => {
    localStorage.setItem(STATUS_LINE_KEY, el.statusLineToggle.checked ? "1" : "0");
    applyStatusLine();
  });
  applyStatusLine();

  window.addEventListener("beforeunload", () => {
    rfb?.disconnect();
  });

  applyUiLanguage();
  initTheme(el.themeSelect);
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
