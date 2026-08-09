// keydown capture：仅当画布聚焦时拦截可拦截组合，preventDefault 阻止浏览器
// 默认动作，事件继续流入 noVNC Keyboard 由其转发（保持修饰键顺序，包括
// Windows AltGr 探测延迟）。

import KeyTable from "/vendor/novnc/core/input/keysym.js";

const MODIFIER_CODES = new Map([
  ["ShiftLeft", { kind: "shift", keysym: KeyTable.XK_Shift_L }],
  ["ShiftRight", { kind: "shift", keysym: KeyTable.XK_Shift_R }],
  ["ControlLeft", { kind: "control", keysym: KeyTable.XK_Control_L }],
  ["ControlRight", { kind: "control", keysym: KeyTable.XK_Control_R }],
  ["AltLeft", { kind: "alt", keysym: KeyTable.XK_Alt_L }],
  ["AltRight", { kind: "alt", keysym: KeyTable.XK_Alt_R }],
  ["MetaLeft", { kind: "meta", keysym: KeyTable.XK_Super_L }],
  ["MetaRight", { kind: "meta", keysym: KeyTable.XK_Super_R }],
]);

const INTERCEPT = [
  { code: "KeyC", modifiers: { ctrl: true } },
  { code: "KeyV", modifiers: { ctrl: true } },
  { code: "KeyX", modifiers: { ctrl: true } },
  { code: "ArrowUp", modifiers: {} },
  { code: "ArrowDown", modifiers: {} },
  { code: "ArrowLeft", modifiers: {} },
  { code: "ArrowRight", modifiers: {} },
  { code: "F2", modifiers: {} },
  { code: "F3", modifiers: {} },
  { code: "F4", modifiers: {} },
  { code: "F6", modifiers: {} },
  { code: "F7", modifiers: {} },
  { code: "F8", modifiers: {} },
  { code: "F9", modifiers: {} },
  { code: "F10", modifiers: {} },
  { code: "F12", modifiers: {} },
];

function findRule(event) {
  const rule = INTERCEPT.find((candidate) => candidate.code === event.code);
  if (!rule) {
    return null;
  }
  const modifiers = rule.modifiers;
  if (
    (modifiers.ctrl !== undefined && event.ctrlKey !== modifiers.ctrl) ||
    (modifiers.alt !== undefined && event.altKey !== modifiers.alt) ||
    (modifiers.shift !== undefined && event.shiftKey !== modifiers.shift) ||
    (modifiers.meta !== undefined && event.metaKey !== modifiers.meta)
  ) {
    return null;
  }
  return rule;
}

export function installKeyboardInterceptor({ getRfb }) {
  const activeModifiers = new Map();

  const focusedRfb = () => {
    const rfb = getRfb();
    if (!rfb || document.activeElement !== rfb.canvas) {
      return null;
    }
    return rfb;
  };

  const eventFromCanvas = (event) => {
    const rfb = getRfb();
    return Boolean(rfb && event.target === rfb.canvas);
  };

  const shouldPrevent = (event) => {
    if (!focusedRfb()) {
      return false;
    }
    return findRule(event) !== null;
  };

  const heldModifiers = () =>
    new Set(Array.from(activeModifiers.values(), (modifier) => modifier.kind));

  const releaseTrackedModifier = (rfb, code, modifier) => {
    const keyboard = rfb?._keyboard;
    const keysym = keyboard?._keyDownList?.[code] ?? modifier.keysym;
    if (typeof keyboard?._sendKeyEvent === "function") {
      keyboard._sendKeyEvent(keysym, code, false);
    } else if (typeof rfb?.sendKey === "function") {
      rfb.sendKey(keysym, code, false);
    }
  };

  const releaseRemoteState = () => {
    const rfb = getRfb();
    const keyboard = rfb?._keyboard;
    if (typeof keyboard?._allKeysUp === "function") {
      keyboard._allKeysUp();
      activeModifiers.clear();
      return;
    }
    for (const [code, modifier] of activeModifiers) {
      releaseTrackedModifier(rfb, code, modifier);
    }
    activeModifiers.clear();
  };

  const releaseLocalState = () => {
    activeModifiers.clear();
  };

  const onKeyDown = (event) => {
    const rfb = focusedRfb();
    const modifier = MODIFIER_CODES.get(event.code);
    if (rfb && modifier) {
      activeModifiers.set(event.code, { code: event.code, ...modifier });
    }
    if (shouldPrevent(event)) {
      event.preventDefault();
    }
  };

  const onKeyUp = (event) => {
    const modifier = activeModifiers.get(event.code);
    if (modifier) {
      if (!eventFromCanvas(event)) {
        releaseTrackedModifier(getRfb(), event.code, modifier);
      }
      activeModifiers.delete(event.code);
    }
    if (shouldPrevent(event)) {
      event.preventDefault();
    }
  };

  const onWindowBlur = () => releaseLocalState();

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("blur", onWindowBlur);

  return {
    heldModifiers,
    releaseRemoteState,
    releaseLocalState,
    dispose() {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onWindowBlur);
      releaseLocalState();
    },
  };
}
