// 状态面板：显示连接状态、帧率、带宽、系统信息等。
// 可拖动、可关闭，半透明毛玻璃样式。

import { getJson } from "./api.js";
import { t } from "./i18n.js";

export class StatusPanel {
  constructor({ button, message }) {
    this.button = button;
    this.message = message;
    this.panel = null;
    this.visible = false;
    this.dragging = false;
    this.dragOffset = { x: 0, y: 0 };
    this.timer = null;
    this.lastStatus = null;
    this.lastSystem = null;
    this.lastEncodeBytes = 0;
    this.lastEncodeTime = 0;
    this.frameRateHistory = [];

    this.button.addEventListener("click", () => this.toggle());
  }

  toggle() {
    if (this.visible) {
      this.hide();
    } else {
      this.show();
    }
  }

  show() {
    if (this.panel) {
      this.panel.hidden = false;
      this.visible = true;
      this.startPolling();
      return;
    }

    this.panel = document.createElement("div");
    this.panel.className = "status-panel";
    this.panel.innerHTML = `
      <div class="status-panel-header">
        <span class="status-panel-title">${t("statusPanel.title")}</span>
        <button class="status-panel-close" aria-label="Close">&times;</button>
      </div>
      <div class="status-panel-body">
        <div class="status-panel-section">
          <div class="status-panel-section-title">📹 ${t("statusPanel.video")}</div>
          <div class="status-panel-row">
            <span>${t("statusPanel.fps")}</span>
            <span id="sp-fps">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.resolution")}</span>
            <span id="sp-resolution">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.format")}</span>
            <span id="sp-format">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.captured")}</span>
            <span id="sp-captured">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.dropped")}</span>
            <span id="sp-dropped">-</span>
          </div>
        </div>
        <div class="status-panel-section">
          <div class="status-panel-section-title">🖥️ ${t("statusPanel.connection")}</div>
          <div class="status-panel-row">
            <span>${t("statusPanel.client")}</span>
            <span id="sp-client">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.transport")}</span>
            <span id="sp-transport">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.connected")}</span>
            <span id="sp-connected">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.inputEvents")}</span>
            <span id="sp-input-events">-</span>
          </div>
        </div>
        <div class="status-panel-section">
          <div class="status-panel-section-title">💾 ${t("statusPanel.encoding")}</div>
          <div class="status-panel-row">
            <span>${t("statusPanel.encodeCount")}</span>
            <span id="sp-encode-count">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.encodedBytes")}</span>
            <span id="sp-encoded-bytes">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.bandwidth")}</span>
            <span id="sp-bandwidth">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.sessionDropped")}</span>
            <span id="sp-session-dropped">-</span>
          </div>
        </div>
        <div class="status-panel-section">
          <div class="status-panel-section-title">📊 ${t("statusPanel.system")}</div>
          <div class="status-panel-row">
            <span>${t("statusPanel.cpu")}</span>
            <span id="sp-cpu">-</span>
          </div>
          <div class="status-panel-row">
            <span>${t("statusPanel.memory")}</span>
            <span id="sp-memory">-</span>
          </div>
        </div>
      </div>
    `;

    document.body.appendChild(this.panel);

    // Close button
    this.panel.querySelector(".status-panel-close").addEventListener("click", () => this.hide());

    // Drag
    const header = this.panel.querySelector(".status-panel-header");
    header.addEventListener("mousedown", (e) => {
      this.dragging = true;
      this.dragOffset.x = e.clientX - this.panel.offsetLeft;
      this.dragOffset.y = e.clientY - this.panel.offsetTop;
      e.preventDefault();
    });

    document.addEventListener("mousemove", (e) => {
      if (!this.dragging) return;
      this.panel.style.left = (e.clientX - this.dragOffset.x) + "px";
      this.panel.style.top = (e.clientY - this.dragOffset.y) + "px";
      this.panel.style.right = "auto";
      this.panel.style.bottom = "auto";
    });

    document.addEventListener("mouseup", () => {
      this.dragging = false;
    });

    this.visible = true;
    this.startPolling();
  }

  hide() {
    if (this.panel) {
      this.panel.hidden = true;
    }
    this.visible = false;
    this.stopPolling();
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
    if (!this.visible || !this.panel) return;

    const el = (id) => this.panel.querySelector(`#sp-${id}`);

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
    el("client").textContent = controller?.peer_addr ?? "-";
    el("transport").textContent = controller?.transport ?? "-";
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
