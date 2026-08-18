// 悬浮控制条：视频页 3 秒无操作自动淡出，鼠标移至视口顶部或控制条聚焦时唤出；
// 固定开关进入文档流并常驻（状态存 localStorage）。同时承载会话菜单与更多菜单的开合。

const PIN_KEY = "my_ipkvm.controlBarPinned";
const HIDE_DELAY_MS = 3000;
const REVEAL_TOP_PX = 48;

export class ControlBar {
  constructor({
    consoleRoot,
    bar,
    videoView,
    pinButton,
    sessionMenuButton,
    sessionMenu,
    moreButton,
    moreMenu,
  }) {
    this.consoleRoot = consoleRoot;
    this.bar = bar;
    this.videoView = videoView;
    this.pinButton = pinButton;
    this.timer = null;

    this.menus = [
      { button: sessionMenuButton, menu: sessionMenu },
      { button: moreButton, menu: moreMenu },
    ];
    for (const { button, menu } of this.menus) {
      button.addEventListener("click", () => this.toggleMenu(button, menu));
    }
    document.addEventListener("click", (event) => {
      for (const { button, menu } of this.menus) {
        if (
          !menu.hidden &&
          !menu.contains(event.target) &&
          !button.contains(event.target)
        ) {
          this.closeMenu(button, menu);
        }
      }
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        this.closeMenus();
      }
    });
    // 更多菜单中的按钮/链接（状态监控、设置、许可证、GitHub）点击后收起菜单；
    // 主题/语言下拉保持展开以便连续切换。
    moreMenu.addEventListener("click", (event) => {
      if (event.target.closest("button, a")) {
        this.closeMenus();
      }
    });

    this.pinButton.addEventListener("click", () => {
      localStorage.setItem(
        PIN_KEY,
        this.consoleRoot.dataset.barPinned === "true" ? "0" : "1",
      );
      this.applyPin();
      this.poke();
    });

    // 唤出/保活：鼠标移至视口顶部、控制条内活动或聚焦。
    this.videoView.addEventListener("mousemove", (event) => {
      const rect = this.videoView.getBoundingClientRect();
      if (event.clientY - rect.top < REVEAL_TOP_PX) {
        this.poke();
      }
    });
    for (const type of ["mousemove", "pointerdown", "focusin"]) {
      this.bar.addEventListener(type, () => this.poke());
    }

    this.applyPin();
  }

  pinned() {
    return localStorage.getItem(PIN_KEY) === "1";
  }

  applyPin() {
    const pinned = this.pinned();
    this.consoleRoot.dataset.barPinned = pinned ? "true" : "false";
    this.pinButton.setAttribute("aria-pressed", pinned ? "true" : "false");
    if (pinned) {
      this.show();
    } else {
      this.scheduleHide();
    }
  }

  /// 视图切换：连接页控制条在文档流中常驻；视频页悬浮并启用自动隐藏。
  setView(view) {
    this.closeMenus();
    if (view === "video") {
      this.poke();
    } else {
      this.show();
    }
  }

  toggleMenu(button, menu) {
    if (menu.hidden) {
      this.closeMenus();
      menu.hidden = false;
      button.setAttribute("aria-expanded", "true");
    } else {
      this.closeMenu(button, menu);
    }
    this.poke();
  }

  closeMenu(button, menu) {
    menu.hidden = true;
    button.setAttribute("aria-expanded", "false");
  }

  closeMenus() {
    for (const { button, menu } of this.menus) {
      this.closeMenu(button, menu);
    }
  }

  anyMenuOpen() {
    return this.menus.some(({ menu }) => !menu.hidden);
  }

  autoHideActive() {
    return (
      this.consoleRoot.dataset.view === "video" &&
      this.consoleRoot.dataset.barPinned !== "true"
    );
  }

  poke() {
    this.show();
    this.scheduleHide();
  }

  show() {
    this.bar.classList.remove("is-hidden");
  }

  scheduleHide() {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (!this.autoHideActive()) {
      return;
    }
    this.timer = setTimeout(() => {
      this.timer = null;
      // 计时器可能在视图切换后才触发，执行时必须重新确认当前仍处于悬浮模式，
      // 否则会把连接页/固定态的控制条错误隐藏。
      if (!this.autoHideActive()) {
        return;
      }
      // 菜单展开或条内聚焦视为仍在操作，顺延一个周期。
      if (this.anyMenuOpen() || this.bar.matches(":focus-within")) {
        this.scheduleHide();
        return;
      }
      this.bar.classList.add("is-hidden");
    }, HIDE_DELAY_MS);
  }
}
