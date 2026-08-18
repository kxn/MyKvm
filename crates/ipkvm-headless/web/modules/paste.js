// 粘贴对话框：打开时尝试预填剪贴板（权限被拒则空白并提示，可手动输入），
// 可编辑确认后发送；显示字符数与超长警告。发送经 noVNC clipboardPasteFrom。

import { t } from "./i18n.js";

// 逐键注入大文本耗时，超过该阈值给出警告。
const LONG_TEXT_THRESHOLD = 2000;

export class PasteDialog {
  constructor({ modal, text, count, hint, warning, sendButton, cancelButton, message, getRfb }) {
    this.modal = modal;
    this.text = text;
    this.count = count;
    this.hint = hint;
    this.warning = warning;
    this.sendButton = sendButton;
    this.message = message;
    this.getRfb = getRfb;

    this.sendButton.addEventListener("click", () => this.send());
    cancelButton.addEventListener("click", () => this.close());
    this.text.addEventListener("input", () => this.updateMeta());
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !this.modal.hidden) {
        event.stopPropagation();
        this.close();
      }
    });
  }

  async open() {
    const rfb = this.getRfb();
    if (!rfb) {
      this.message(t("special.unsent"), "error");
      return;
    }
    this.hint.textContent = "";
    this.text.value = "";
    this.updateMeta();
    this.modal.hidden = false;
    this.text.focus();
    // 预填剪贴板：权限被拒或不可用时留空并提示，用户可手动输入。
    if (typeof navigator.clipboard?.readText === "function") {
      try {
        this.text.value = await navigator.clipboard.readText();
        this.updateMeta();
      } catch {
        this.hint.textContent = t("clipboard.prefillFailed");
      }
    } else {
      this.hint.textContent = t("clipboard.prefillFailed");
    }
  }

  updateMeta() {
    const length = this.text.value.length;
    this.count.textContent = t("clipboard.charCount", { count: length });
    this.warning.hidden = length <= LONG_TEXT_THRESHOLD;
    if (length > LONG_TEXT_THRESHOLD) {
      this.warning.textContent = t("clipboard.tooLong", { count: length });
    }
  }

  send() {
    const rfb = this.getRfb();
    if (!rfb) {
      this.message(t("special.unsent"), "error");
      this.close();
      return;
    }
    const value = this.text.value;
    if (value.length === 0) {
      this.text.focus();
      return;
    }
    rfb.clipboardPasteFrom(value);
    this.message(t("clipboard.sentOk", { count: value.length }), "ok");
    this.close();
  }

  close() {
    this.modal.hidden = true;
  }
}
