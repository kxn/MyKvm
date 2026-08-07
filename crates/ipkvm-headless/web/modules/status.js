// 状态轮询：连接页 1s、视频页 2s、失败退避 5s；由会话状态驱动视图状态机。

import { getJson } from "./api.js";

export const VIEW = Object.freeze({
  CONNECTION: "connection",
  VIDEO: "video",
});

export const REASON = Object.freeze({
  ABSENT: "absent",
  STOPPED: "stopped",
  MANUAL_STOP: "manual-stop",
  RECOVERING: "recovering",
  SWITCH: "switch",
});

export function resolveView(status) {
  const state = status?.session?.state;
  // 手动停止：切换到连接页
  if (status?.session?.manual_stop) {
    return { view: VIEW.CONNECTION, reason: REASON.MANUAL_STOP };
  }
  // 会话不存在：切换到连接页
  if (state === "absent") {
    return { view: VIEW.CONNECTION, reason: REASON.ABSENT };
  }
  // 会话运行中或停止中（恢复循环会自动重建）：保持视频页
  // 只有手动停止或会话不存在才切换到连接页
  return { view: VIEW.VIDEO, reason: null };
}

export class StatusController {
  constructor({ onStatus, onViewChange, onError }) {
    this.onStatus = onStatus;
    this.onViewChange = onViewChange;
    this.onError = onError;
    this.status = null;
    this.failures = 0;
    this.currentView = null;
    // 手动切回连接页（会话仍 running）时置 override：状态轮询不再自动切回
    // 视频页，直到用户重新连接（clearViewOverride）。
    this.viewOverride = null;
    this.timer = null;
    this.running = false;
  }

  start() {
    if (this.running) {
      return;
    }
    this.running = true;
    this.schedule(0);
  }

  stop() {
    this.running = false;
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  setViewOverride(view) {
    this.viewOverride = view;
  }

  clearViewOverride() {
    this.viewOverride = null;
  }

  schedule(delayMs) {
    if (!this.running) {
      return;
    }
    if (this.timer !== null) {
      clearTimeout(this.timer);
    }
    this.timer = setTimeout(() => {
      this.timer = null;
      this.tick().then((nextDelay) => this.schedule(nextDelay)).catch(() => {});
    }, delayMs);
  }

  async tick() {
    try {
      const status = await getJson("/api/status");
      this.failures = 0;
      this.status = status;
      this.onStatus?.(status);
      const { view, reason } = resolveView(status);
      const targetView = this.viewOverride ?? view;
      if (targetView !== this.currentView) {
        this.currentView = targetView;
        this.onViewChange?.(
          targetView,
          this.viewOverride !== null ? REASON.SWITCH : reason,
        );
      }
      return targetView === VIEW.VIDEO ? 2000 : 1000;
    } catch (error) {
      this.failures += 1;
      this.onError?.(error);
      return 5000;
    }
  }
}
