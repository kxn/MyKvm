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
  if (state === "running") {
    return { view: VIEW.VIDEO, reason: null };
  }
  if (status?.session?.manual_stop) {
    return { view: VIEW.CONNECTION, reason: REASON.MANUAL_STOP };
  }
  if (state === "absent") {
    return { view: VIEW.CONNECTION, reason: REASON.ABSENT };
  }
  if (status?.session?.input_offline || status?.video?.stalled) {
    return { view: VIEW.CONNECTION, reason: REASON.RECOVERING };
  }
  return { view: VIEW.CONNECTION, reason: REASON.STOPPED };
}

export class StatusController {
  constructor({ onStatus, onViewChange, onError }) {
    this.onStatus = onStatus;
    this.onViewChange = onViewChange;
    this.onError = onError;
    this.status = null;
    this.failures = 0;
    this.currentView = null;
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
      if (view !== this.currentView) {
        this.currentView = view;
        this.onViewChange?.(view, reason);
      }
      return view === VIEW.VIDEO ? 2000 : 1000;
    } catch (error) {
      this.failures += 1;
      this.onError?.(error);
      return 5000;
    }
  }
}
