// 全量特殊键菜单：桌面四键 + 浏览器保留组合，经 noVNC sendKey 序列发送。
// Full special keys menu with the desktop quartet and browser-reserved combos.

import KeyTable from "/vendor/novnc/core/input/keysym.js";
import { t } from "./i18n.js";

const GROUPS = [
  {
    key: "special.desktop",
    items: [
      {
        id: "special-ctrl-alt-del",
        key: "special.ctrlAltDel",
        label: "Ctrl+Alt+Del",
        keys: [
          ["ControlLeft", KeyTable.XK_Control_L],
          ["AltLeft", KeyTable.XK_Alt_L],
          ["Delete", KeyTable.XK_Delete],
        ],
      },
      {
        id: "special-win",
        key: "special.win",
        label: "Win",
        keys: [["MetaLeft", KeyTable.XK_Super_L]],
      },
      {
        id: "special-alt-tab",
        key: "special.altTab",
        label: "Alt+Tab",
        keys: [
          ["AltLeft", KeyTable.XK_Alt_L],
          ["Tab", KeyTable.XK_Tab],
        ],
      },
      {
        id: "special-print-screen",
        key: "special.printScreen",
        label: "PrintScreen",
        keys: [["PrintScreen", KeyTable.XK_Print]],
      },
    ],
  },
  {
    key: "special.tabs",
    items: [
      {
        id: "special-ctrl-w",
        key: "special.ctrlW",
        label: "Ctrl+W",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyW", KeyTable.XK_w]],
      },
      {
        id: "special-ctrl-t",
        key: "special.ctrlT",
        label: "Ctrl+T",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyT", KeyTable.XK_t]],
      },
      {
        id: "special-ctrl-n",
        key: "special.ctrlN",
        label: "Ctrl+N",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyN", KeyTable.XK_n]],
      },
      {
        id: "special-ctrl-shift-t",
        key: "special.ctrlShiftT",
        label: "Ctrl+Shift+T",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["KeyT", KeyTable.XK_t],
        ],
      },
      {
        id: "special-ctrl-tab",
        key: "special.ctrlTab",
        label: "Ctrl+Tab",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["Tab", KeyTable.XK_Tab]],
      },
      {
        id: "special-ctrl-shift-tab",
        key: "special.ctrlShiftTab",
        label: "Ctrl+Shift+Tab",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["Tab", KeyTable.XK_Tab],
        ],
      },
      {
        id: "special-ctrl-shift-n",
        key: "special.ctrlShiftN",
        label: "Ctrl+Shift+N",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["KeyN", KeyTable.XK_n],
        ],
      },
    ],
  },
  {
    key: "special.refresh",
    items: [
      {
        id: "special-f5",
        key: "special.f5",
        label: "F5",
        keys: [["F5", KeyTable.XK_F5]],
      },
      {
        id: "special-ctrl-r",
        key: "special.ctrlR",
        label: "Ctrl+R",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyR", KeyTable.XK_r]],
      },
      {
        id: "special-ctrl-shift-r",
        key: "special.ctrlShiftR",
        label: "Ctrl+Shift+R",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["KeyR", KeyTable.XK_r],
        ],
      },
      {
        id: "special-alt-left",
        key: "special.altLeft",
        label: "Alt+←",
        keys: [
          ["AltLeft", KeyTable.XK_Alt_L],
          ["ArrowLeft", KeyTable.XK_Left],
        ],
      },
      {
        id: "special-alt-right",
        key: "special.altRight",
        label: "Alt+→",
        keys: [
          ["AltLeft", KeyTable.XK_Alt_L],
          ["ArrowRight", KeyTable.XK_Right],
        ],
      },
    ],
  },
  {
    key: "special.developer",
    items: [
      {
        id: "special-ctrl-shift-i",
        key: "special.ctrlShiftI",
        label: "Ctrl+Shift+I",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["KeyI", KeyTable.XK_i],
        ],
      },
      {
        id: "special-ctrl-shift-j",
        key: "special.ctrlShiftJ",
        label: "Ctrl+Shift+J",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["KeyJ", KeyTable.XK_j],
        ],
      },
      {
        id: "special-ctrl-shift-c",
        key: "special.ctrlShiftC",
        label: "Ctrl+Shift+C",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["KeyC", KeyTable.XK_c],
        ],
      },
      {
        id: "special-ctrl-u",
        key: "special.ctrlU",
        label: "Ctrl+U",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyU", KeyTable.XK_u]],
      },
      {
        id: "special-f11",
        key: "special.f11",
        label: "F11",
        keys: [["F11", KeyTable.XK_F11]],
      },
      {
        id: "special-f1",
        key: "special.f1",
        label: "F1",
        keys: [["F1", KeyTable.XK_F1]],
      },
    ],
  },
  {
    key: "special.other",
    items: [
      {
        id: "special-ctrl-p",
        key: "special.ctrlP",
        label: "Ctrl+P",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyP", KeyTable.XK_p]],
      },
      {
        id: "special-ctrl-s",
        key: "special.ctrlS",
        label: "Ctrl+S",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyS", KeyTable.XK_s]],
      },
      {
        id: "special-ctrl-f",
        key: "special.ctrlF",
        label: "Ctrl+F",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyF", KeyTable.XK_f]],
      },
      {
        id: "special-ctrl-h",
        key: "special.ctrlH",
        label: "Ctrl+H",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyH", KeyTable.XK_h]],
      },
      {
        id: "special-ctrl-d",
        key: "special.ctrlD",
        label: "Ctrl+D",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyD", KeyTable.XK_d]],
      },
      {
        id: "special-ctrl-o",
        key: "special.ctrlO",
        label: "Ctrl+O",
        keys: [["ControlLeft", KeyTable.XK_Control_L], ["KeyO", KeyTable.XK_o]],
      },
      {
        id: "special-ctrl-shift-delete",
        key: "special.ctrlShiftDelete",
        label: "Ctrl+Shift+Delete",
        keys: [
          ["ShiftLeft", KeyTable.XK_Shift_L],
          ["ControlLeft", KeyTable.XK_Control_L],
          ["Delete", KeyTable.XK_Delete],
        ],
      },
    ],
  },
];

export class SpecialKeysController {
  constructor({ button, menu, message, getRfb }) {
    this.button = button;
    this.menu = menu;
    this.message = message;
    this.getRfb = getRfb;
    this.open = false;

    this.button.addEventListener("click", () => this.toggle());
    document.addEventListener("click", (event) => {
      if (this.open && !this.menu.contains(event.target) && !this.button.contains(event.target)) {
        this.close();
      }
    });
    this.refresh();
  }

  toggle() {
    if (this.open) {
      this.close();
    } else {
      this.openMenu();
    }
  }

  openMenu() {
    this.open = true;
    this.menu.hidden = false;
    this.button.setAttribute("aria-expanded", "true");
    this.updateEnabled();
  }

  close() {
    this.open = false;
    this.menu.hidden = true;
    this.button.setAttribute("aria-expanded", "false");
  }

  refresh() {
    this.menu.textContent = "";
    for (const group of GROUPS) {
      const heading = document.createElement("h3");
      heading.textContent = t(group.key);
      const list = document.createElement("div");
      list.className = "special-key-group";
      for (const item of group.items) {
        const button = document.createElement("button");
        button.type = "button";
        button.id = item.id;
        button.dataset.specialKey = item.key;
        button.textContent = t(item.key);
        button.addEventListener("click", () => this.send(item));
        list.appendChild(button);
      }
      this.menu.append(heading, list);
    }
    this.updateEnabled();
  }

  updateEnabled() {
    const connected = Boolean(this.getRfb());
    for (const button of this.menu.querySelectorAll("button")) {
      button.disabled = !connected;
    }
  }

  send(item) {
    const rfb = this.getRfb();
    if (!rfb) {
      this.message(t("special.unsent"), "error");
      return;
    }
    for (const [code, keysym] of item.keys) {
      rfb.sendKey(keysym, code, true);
    }
    for (const [code, keysym] of [...item.keys].reverse()) {
      rfb.sendKey(keysym, code, false);
    }
    this.message(t("special.sent", { name: item.label }), "ok");
    this.close();
  }
}
