// 主题三态（跟随系统/深色/浅色）与切换；选择存 localStorage，默认跟随系统。
// 决策逻辑必须与 index.html / licenses.html 首屏内联脚本保持一致：
// system 解析为浅色仅当 prefers-color-scheme: light 成立，否则落深色。

const STORAGE_KEY = "my_ipkvm.theme";
const VALID_CHOICES = new Set(["system", "dark", "light"]);
const LIGHT_QUERY = "(prefers-color-scheme: light)";

export function getStoredTheme() {
  const stored = localStorage.getItem(STORAGE_KEY);
  return VALID_CHOICES.has(stored) ? stored : "system";
}

export function resolveTheme(choice) {
  if (choice !== "system") {
    return choice;
  }
  return window.matchMedia(LIGHT_QUERY).matches ? "light" : "dark";
}

export function applyTheme(choice) {
  document.documentElement.dataset.theme = resolveTheme(choice);
}

export function setTheme(choice) {
  if (!VALID_CHOICES.has(choice)) {
    choice = "system";
  }
  if (choice === "system") {
    localStorage.removeItem(STORAGE_KEY);
  } else {
    localStorage.setItem(STORAGE_KEY, choice);
  }
  applyTheme(choice);
}

export function initTheme(select) {
  const choice = getStoredTheme();
  applyTheme(choice);
  if (select) {
    select.value = choice;
    select.addEventListener("change", () => {
      setTheme(select.value);
    });
  }
  // system 模式下跟随系统偏好实时切换。
  window.matchMedia(LIGHT_QUERY).addEventListener("change", () => {
    if (getStoredTheme() === "system") {
      applyTheme("system");
    }
  });
}
