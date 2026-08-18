// 状态监控抽屉：右侧滑出，画面保持可见可交互。
// 用户可读指标常态展示（帧率/分辨率/带宽/连接时长/CPU/内存），
// 诊断指标（采集帧/掉帧/编码字节/输入事件等）折叠进「高级」。

import { getJson } from "./api.js";
import { t } from "./i18n.js";

export class StatusPanel {
  constructor({ button, message }) {
    this.button = button;
    this.message = message;
    this.drawer = null;
    this.visible = false;
    this.timer = null;
    this.lastStatus = null;
    this.lastSystem = null;
    this.frameRateHistory = [];

    this.button.addEventListener("click", () => this.toggle());
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && this.visible) {
        this.hide();
      }
    });
  }

  toggle() {
    if (this.visible) {
      this.hide();
    } else {
      this.show();
    }
  }

  show() {
    if (!this.drawer) {
      this.build();
    }
    this.drawer.hidden = false;
    // 强制 reflow 让滑入过渡生效。
    void this.drawer.offsetWidth;
    this.drawer.classList.add("open");
    this.visible = true;
    this.startPolling();
  }

  hide() {
    if (this.drawer) {
      this.drawer.classList.remove("open");
      // 过渡结束后隐藏，避免屏幕阅读器聚焦不可见内容。
      this.drawer.hidden = true;
    }
    this.visible = false;
    this.stopPolling();
  }

  build() {
    this.drawer = document.createElement("aside");
    this.drawer.className = "status-drawer";
    this.drawer.setAttribute("aria-label", t("statusPanel.title"));
    this.drawer.innerHTML = `
      <div class="status-drawer-header">
        <span class="status-drawer-title">${t("statusPanel.title")}</span>
        <button class="status-drawer-close" aria-label="${t("statusPanel.close")}">&times;</button>
      </div>
      <div class="status-drawer-body">
        <div class="status-drawer-section">
          <div class="status-drawer-section-title">${t("statusPanel.video")}</div>
          <div class="status-drawer-row">
            <span>${t("statusPanel.fps")}</span>
            <span id="sp-fps">-</span>
          </div>
          <div class="status-drawer-row">
            <span>${t("statusPanel.resolution")}</span>
            <span id="sp-resolution">-</span>
          </div>
        </div>
        <div class="status-drawer-section">
          <div class="status-drawer-section-title">${t("statusPanel.connection")}</div>
          <div class="status-drawer-row">
            <span>${t("statusPanel.bandwidth")}</span>
            <span id="sp-bandwidth">-</span>
          </div>
          <div class="status-drawer-row">
            <span>${t("statusPanel.connected")}</span>
            <span id="sp-connected">-</span>
          </div>
        </div>
        <div class="status-drawer-section">
          <div class="status-drawer-section-title">${t("statusPanel.system")}</div>
          <div class="status-drawer-row">
            <span>${t("statusPanel.cpu")}</span>
            <span id="sp-cpu">-</span>
          </div>
          <div class="status-drawer-row">
            <span>${t("statusPanel.memory")}</span>
            <span id="sp-memory">-</span>
          </div>
        </div>
        <details class="status-drawer-advanced">
          <summary>${t("statusPanel.advanced")}</summary>
          <div class="status-drawer-section">
            <div class="status-drawer-section-title">${t("statusPanel.video")}</div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.format")}</span>
              <span id="sp-format">-</span>
            </div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.captured")}</span>
              <span id="sp-captured">-</span>
            </div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.dropped")}</span>
              <span id="sp-dropped">-</span>
            </div>
          </div>
          <div class="status-drawer-section">
            <div class="status-drawer-section-title">${t("statusPanel.connection")}</div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.inputEvents")}</span>
              <span id="sp-input-events">-</span>
            </div>
          </div>
          <div class="status-drawer-section">
            <div class="status-drawer-section-title">${t("statusPanel.encoding")}</div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.encodeCount")}</span>
              <span id="sp-encode-count">-</span>
            </div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.encodedBytes")}</span>
              <span id="sp-encoded-bytes">-</span>
            </div>
            <div class="status-drawer-row">
              <span>${t("statusPanel.sessionDropped")}</span>
              <span id="sp-session-dropped">-</span>
            </div>
          </div>
        </details>
      </div>
    `;

    document.body.appendChild(this.drawer);
    this.drawer
      .querySelector(".status-drawer-close")
      .addEventListener("click", () => this.hide());
  }

  startPolling() {
    this.stopPolling();
    this.poll();
    this.timer = setInterval(() => this.poll(), 2000);
  }

  stopPolling() {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  async poll() {
    try {
      const [status, system] = await Promise.all([
        getJson("/api/status"),
        getJson("/api/system"),
      ]);
      this.update(status, system);
    } catch (error) {
      // Ignore polling errors
    }
  }

  update(status, system) {
    if (!this.visible || !this.drawer) return;

    const el = (id) => this.drawer.querySelector(`#sp-${id}`);

    // Video
    const frame = status?.video?.frame;
    el("resolution").textContent = frame ? `${frame.width}×${frame.height}` : "-";
    el("format").textContent = frame?.pixel_format ?? "-";
    el("captured").textContent = status?.video?.source_stats?.published_frames ?? "-";

    // Calculate FPS
    const dropped = status?.video?.source_stats?.dropped_frames ?? 0;
    el("dropped").textContent = dropped;

    if (this.lastStatus && frame) {
      const lastFrame = this.lastStatus?.video?.frame;
      if (lastFrame) {
        const timeDiff = (frame.capture_ns - lastFrame.capture_ns) / 1_000_000_000;
        const frameDiff = frame.seq - lastFrame.seq;
        if (timeDiff > 0) {
          const fps = frameDiff / timeDiff;
          this.frameRateHistory.push(fps);
          if (this.frameRateHistory.length > 5) this.frameRateHistory.shift();
          const avgFps = this.frameRateHistory.reduce((a, b) => a + b, 0) / this.frameRateHistory.length;
          el("fps").textContent = `${avgFps.toFixed(1)} fps`;
        }
      }
    }

    // Connection
    const controller = status?.controller;
    if (controller?.connected_since_ms) {
      const minutes = Math.floor(controller.connected_since_ms / 60000);
      const seconds = Math.floor((controller.connected_since_ms % 60000) / 1000);
      el("connected").textContent = `${minutes}m ${seconds}s`;
    } else {
      el("connected").textContent = "-";
    }
    el("input-events").textContent = status?.session?.input_events ?? "-";

    // Encoding
    const encode = status?.session?.encode;
    el("encode-count").textContent = encode?.encode_count ?? "-";
    const encodedMB = encode ? (encode.encoded_bytes_total / 1024 / 1024).toFixed(1) : "-";
    el("encoded-bytes").textContent = encodedMB !== "-" ? `${encodedMB} MB` : "-";

    // Bandwidth calculation
    if (this.lastStatus && encode) {
      const lastEncode = this.lastStatus?.session?.encode;
      if (lastEncode) {
        const bytesDiff = encode.encoded_bytes_total - lastEncode.encoded_bytes_total;
        const timeDiff = 2; // 2 seconds between polls
        const bandwidth = bytesDiff / timeDiff;
        if (bandwidth > 1024 * 1024) {
          el("bandwidth").textContent = `${(bandwidth / 1024 / 1024).toFixed(1)} MB/s`;
        } else if (bandwidth > 1024) {
          el("bandwidth").textContent = `${(bandwidth / 1024).toFixed(1)} KB/s`;
        } else {
          el("bandwidth").textContent = `${bandwidth.toFixed(0)} B/s`;
        }
      }
    }

    // Session dropped
    el("session-dropped").textContent = status?.session?.dropped_frames ?? "-";

    // System
    if (system) {
      const cpuPercent = system.cpu_percent ?? 0;
      el("cpu").textContent = `${cpuPercent.toFixed(1)}%`;
      const memUsedMB = system.mem_used_kb ? (system.mem_used_kb / 1024).toFixed(0) : "-";
      const memTotalMB = system.mem_total_kb ? (system.mem_total_kb / 1024).toFixed(0) : "-";
      el("memory").textContent = `${memUsedMB} MB / ${memTotalMB} MB`;
    }

    this.lastStatus = status;
  }
}
