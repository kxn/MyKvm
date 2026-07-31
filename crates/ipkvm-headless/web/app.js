import RFB from "/vendor/novnc/core/rfb.js";

const consoleRoot = document.querySelector("#console");
const screen = document.querySelector("#screen");
const statusText = document.querySelector("#connection-status");
const reconnectButton = document.querySelector("#reconnect");
const disconnectButton = document.querySelector("#disconnect");

let session = null;
let disconnectRequested = false;

function websocketUrl() {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${location.host}/rfb`;
}

function setConnectionState(state, text) {
  consoleRoot.dataset.connectionState = state;
  statusText.textContent = text;
  reconnectButton.disabled = state === "connecting" || state === "connected";
  disconnectButton.disabled = state !== "connected";
}

function connect() {
  if (session !== null) {
    return;
  }

  disconnectRequested = false;
  setConnectionState("connecting", "正在连接");

  try {
    const next = new RFB(screen, websocketUrl(), { shared: true });
    session = next;
    next.scaleViewport = true;
    next.resizeSession = false;
    next.viewOnly = false;
    next.focusOnClick = true;

    next.addEventListener("connect", () => {
      if (session === next) {
        setConnectionState("connected", "已连接");
      }
    });

    next.addEventListener("disconnect", (event) => {
      if (session !== next) {
        return;
      }
      const requested = disconnectRequested;
      session = null;
      disconnectRequested = false;
      if (requested || event.detail.clean) {
        setConnectionState("disconnected", "已断开");
      } else {
        setConnectionState("failed", "连接失败");
      }
    });

    next.addEventListener("securityfailure", () => {
      setConnectionState("failed", "安全协商失败");
    });

    next.addEventListener("credentialsrequired", () => {
      setConnectionState("failed", "服务端要求凭据");
      next.disconnect();
    });
  } catch {
    session = null;
    setConnectionState("failed", "连接失败");
  }
}

reconnectButton.addEventListener("click", connect);

disconnectButton.addEventListener("click", () => {
  if (session === null) {
    return;
  }
  disconnectRequested = true;
  statusText.textContent = "正在断开";
  disconnectButton.disabled = true;
  session.disconnect();
});

window.addEventListener("beforeunload", () => {
  session?.disconnect();
});

connect();
