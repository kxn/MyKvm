// keydown capture：仅当画布聚焦时拦截可拦截组合，preventDefault 阻止浏览器
// 默认动作，事件继续流入 noVNC Keyboard 由其转发（保持修饰键顺序，包括
// Windows AltGr 探测延迟）。

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
  const shouldPrevent = (event) => {
    const rfb = getRfb();
    if (!rfb || document.activeElement !== rfb.canvas) {
      return false;
    }
    return findRule(event) !== null;
  };

  const onKeyDown = (event) => {
    if (shouldPrevent(event)) {
      event.preventDefault();
    }
  };

  const onKeyUp = (event) => {
    if (shouldPrevent(event)) {
      event.preventDefault();
    }
  };

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);

  return () => {
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);
  };
}
